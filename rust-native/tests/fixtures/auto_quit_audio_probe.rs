//! One owned silent-output experiment. Never enumerates processes or captures input.
use serde_json::{json, Value};
use std::{
    ffi::c_void,
    ptr, thread,
    time::{Duration, Instant},
};
type QueueRef = *mut c_void;
#[repr(C)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}
#[repr(C)]
struct Format {
    sample_rate: f64,
    format: u32,
    flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}
#[repr(C)]
struct Buffer {
    capacity: u32,
    data: *mut c_void,
    size: u32,
    user: *mut c_void,
    packet_capacity: u32,
    packets: *mut c_void,
    packet_count: u32,
}
#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}
#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioQueueNewOutput(
        format: *const Format,
        callback: extern "C" fn(*mut c_void, QueueRef, *mut Buffer),
        user: *mut c_void,
        run_loop: *const c_void,
        run_loop_mode: *const c_void,
        flags: u32,
        queue: *mut QueueRef,
    ) -> i32;
    fn AudioQueueAllocateBuffer(queue: QueueRef, size: u32, buffer: *mut *mut Buffer) -> i32;
    fn AudioQueueEnqueueBuffer(
        queue: QueueRef,
        buffer: *mut Buffer,
        count: u32,
        packets: *const c_void,
    ) -> i32;
    fn AudioQueueStart(queue: QueueRef, timestamp: *const c_void) -> i32;
    fn AudioQueueStop(queue: QueueRef, immediate: u8) -> i32;
    fn AudioQueueDispose(queue: QueueRef, immediate: u8) -> i32;
}
fn status(code: i32, operation: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("{operation}: OSStatus {code}"))
    }
}
fn property(object: u32, selector: &[u8; 4], pid: Option<&i32>) -> Result<u32, String> {
    let address = Address {
        selector: u32::from_be_bytes(*selector),
        scope: u32::from_be_bytes(*b"glob"),
        element: 0,
    };
    let mut size = 4u32;
    let mut value = 0u32;
    // All queried properties return one UInt32; only PID translation takes a qualifier.
    status(
        unsafe {
            AudioObjectGetPropertyData(
                object,
                &address,
                if pid.is_some() { 4 } else { 0 },
                pid.map_or(ptr::null(), |pid| (pid as *const i32).cast()),
                &mut size,
                (&mut value as *mut u32).cast(),
            )
        },
        "read property",
    )?;
    if size != 4 {
        return Err("Unexpected property size".into());
    }
    Ok(value)
}
fn sample() -> Result<Value, String> {
    let pid = std::process::id() as i32;
    let object = property(1, b"id2p", Some(&pid))?;
    if object == 0 {
        return Ok(json!({"known":false}));
    }
    if property(object, b"ppid", None)? != pid as u32 {
        return Err("Own process object changed".into());
    }
    let running = property(object, b"pir?", None)?;
    let input = property(object, b"piri", None)?;
    let output = property(object, b"piro", None)?;
    if [running, input, output].iter().any(|value| *value > 1) {
        return Err("Unexpected nonboolean flag".into());
    }
    if property(object, b"ppid", None)? != pid as u32 {
        return Err("Own process object changed".into());
    }
    Ok(json!({"known":true,"running":running,"input":input,"output":output}))
}
fn await_output(expected: u32) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let sample = sample()?;
        if sample["known"] == true && sample["output"] == expected {
            return Ok(sample);
        }
        if Instant::now() >= deadline {
            return Err(format!("Output flag did not become {expected}: {sample}"));
        }
        thread::sleep(Duration::from_millis(20));
    }
}
struct Queue(QueueRef);
impl Drop for Queue {
    fn drop(&mut self) {
        unsafe {
            AudioQueueStop(self.0, 1);
            AudioQueueDispose(self.0, 1);
        }
    }
}
extern "C" fn consumed(_: *mut c_void, _: QueueRef, _: *mut Buffer) {}
fn run() -> Result<Value, String> {
    let baseline = sample()?;
    let format = Format {
        sample_rate: 48_000.,
        format: u32::from_be_bytes(*b"lpcm"),
        flags: 1 | 8,
        bytes_per_packet: 8,
        frames_per_packet: 1,
        bytes_per_frame: 8,
        channels: 2,
        bits: 32,
        reserved: 0,
    };
    let mut raw = ptr::null_mut();
    status(
        unsafe {
            AudioQueueNewOutput(
                &format,
                consumed,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                0,
                &mut raw,
            )
        },
        "create output queue",
    )?;
    if raw.is_null() {
        return Err("No owned output queue".into());
    }
    let queue = Queue(raw);
    for _ in 0..3 {
        let mut buffer = ptr::null_mut();
        status(
            unsafe { AudioQueueAllocateBuffer(queue.0, 384_000, &mut buffer) },
            "allocate silence",
        )?;
        if buffer.is_null() {
            return Err("No owned output buffer".into());
        }
        // One second of initialized Float32 stereo zeros per buffer. No microphone or file reads.
        unsafe {
            if (*buffer).data.is_null() || (*buffer).capacity < 384_000 {
                return Err("Invalid owned output buffer".into());
            }
            ptr::write_bytes((*buffer).data.cast::<u8>(), 0, 384_000);
            (*buffer).size = 384_000;
        }
        status(
            unsafe { AudioQueueEnqueueBuffer(queue.0, buffer, 0, ptr::null()) },
            "enqueue silence",
        )?;
    }
    // Queue preparation may already engage output IO. Record it, do not
    // confuse AudioQueueStart acceptance with the start of device activity.
    let prepared = sample()?;
    status(
        unsafe { AudioQueueStart(queue.0, ptr::null()) },
        "start silent output",
    )?;
    let active = await_output(1)?;
    status(unsafe { AudioQueueStop(queue.0, 1) }, "stop own output")?;
    drop(queue);
    let stopped = await_output(0)?;
    if prepared["input"] != 0 || active["input"] != 0 || stopped["input"] != 0 {
        return Err("Unexpected input signal in output-only fixture".into());
    }
    Ok(
        json!({"ok":true,"schema":"ruycast.auto-quit-audio-probe.v1","ownProcessOnly":true,
        "silentFloat32OutputOnly":true,"microphoneOpened":false,"screenOrCameraCaptured":false,
        "baseline":baseline,"prepared":prepared,"active":active,"stopped":stopped,
        "scope":"One owned process and public Core Audio flags; no negative proof of media/recording safety"}),
    )
}
fn main() {
    match run() {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}
