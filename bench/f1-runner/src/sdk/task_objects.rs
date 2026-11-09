//! Task/Pipe/Handle business adapter in the existing workspace receiver arena.
//! These fields grant no object, class, thread or memory authority. The parent
//! factory owns the actual typed construction/cleanup and receiver identity.
use super::*;
use crate::sdk::task_model::{Failure, Operation, Task};
use crate::sdk::task_model::{Handle as FileHandle, Stream};
use std::os::fd::AsRawFd;
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
use crate::sdk::process_router::ReferenceTaskRoute;

pub(super) struct Fields {
    pub(super) task: Arc<Task>,
    pub(super) environment: Mutex<Option<BTreeMap<String, String>>>,
    arguments_present: AtomicBool,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    shortcuts_route: Mutex<Option<ReferenceTaskRoute>>,
    streams: Mutex<[Option<Arc<super::super::factory::TaskChildReference>>; 3]>,
}
impl Fields {
    pub(super) fn new(task: Arc<Task>) -> Self {
        Self {
            task,
            environment: Mutex::new(None),
            arguments_present: AtomicBool::new(false),
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            shortcuts_route: Mutex::new(None),
            streams: Mutex::new([None, None, None]),
        }
    }
    pub(super) fn stream(
        &self,
        index: usize,
    ) -> Result<Option<Arc<super::super::factory::TaskChildReference>>> {
        self.streams
            .lock()
            .map_err(|_| "Task stream properties poisoned")?
            .get(index)
            .cloned()
            .ok_or_else(|| "Task stream index invalid".into())
    }
}

fn boundary<T: Copy>(fallback: T, body: impl FnOnce() -> Result<T>) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => fallback,
        Err(payload) => {
            std::mem::forget(payload);
            if let Some(installed) = INSTALLED.get() {
                installed
                    .model
                    .faulted
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            fallback
        }
    }
}

fn fresh_fields(receiver: *mut AnyObject, fields: &Arc<Fields>) -> Result<()> {
    if !Arc::ptr_eq(fields, &super::task_fields(receiver)?) {
        return Err("Task receiver changed during property decoding".into());
    }
    Ok(())
}
fn returned_value(value: foundation::FoundationValue) -> Result<*mut AnyObject> {
    let pointer = new_foundation_value(value)?;
    unsafe { autorelease_foundation(pointer) }
}
fn path_value(fields: &Fields) -> Result<String> {
    fields
        .task
        .config
        .lock()
        .map_err(|_| "Task config poisoned")?
        .executable
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "Task executable is not valid Unicode".into())
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_url(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_url_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn set_url(receiver: *mut AnyObject, selector: Sel, value: *mut AnyObject) {
    unsafe { set_url_checked(receiver, selector, value, 0) };
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn reference_shortcuts_path(
    value: *mut AnyObject, return_pc: usize,
) -> Result<(String, ReferenceTaskRoute, impl Fn() -> Result<()>)> {
    use std::ffi::{c_char, CString};
    let (route, expected, check) = crate::sdk::process_router::verify_reference_shortcuts_setter(return_pc, None)?;
    // The verified Swift caller owns this bridge result until we return.
    let object = unsafe { &*value };
    let class = object.class();
    if class.name() != c"NSURL" {
        return Err("Reference Shortcuts bridge did not supply NSURL".into());
    }
    let class = crate::sdk::metadata::registered_class(class as *const AnyClass as usize)?;
    let selector = c"fileSystemRepresentation";
    let method = crate::sdk::metadata::method(class, selector, false)?
        .ok_or("Reference Shortcuts NSURL path method absent")?;
    let code = method.code();
    if method.owner() != class.address() || method.encoding() != b"r*16@0:8"
        || code.image() != b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation"
        || code.offset() != 0x38f54
        || code.uuid() != [0x91, 0xda, 0xce, 0x39, 0xfa, 0x28, 0x31, 0x91,
            0x81, 0x8d, 0x1f, 0xcc, 0x6a, 0x0e, 0x61, 0x5a]
    {
        return Err("Reference Shortcuts NSURL path method differs".into());
    }
    let read: unsafe extern "C-unwind" fn(&AnyObject, Sel) -> *const c_char =
        unsafe { std::mem::transmute(method.implementation()) };
    let path = unsafe { read(object, Sel::register(selector)) };
    let expected = CString::new(expected.as_os_str().as_encoded_bytes()).map_err(|_| "Reference runner path contains NUL")?;
    if path.is_null() || unsafe { libc::strncmp(path, expected.as_ptr(), expected.as_bytes_with_nul().len()) } != 0 {
        return Err("Reference Shortcuts executable path differs".into());
    }
    check()?;
    Ok((expected.into_string().map_err(|_| "Reference runner path is not UTF-8")?, route, check))
}

unsafe extern "C" fn set_url_checked(
    receiver: *mut AnyObject, _: Sel, value: *mut AnyObject, return_pc: usize,
) {
    boundary((), || {
        let fields = super::task_fields(receiver)?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let mut reference_check = None;
        let path = if value.is_null() {
            String::new()
        } else {
            match foundation_url(value) {
                Ok(url) if url.is_file() => {
                    String::from_utf16(url.path().ok_or("Task file URL has no path")?.utf16())
                        .map_err(|_| "Task URL path is not valid Unicode")?
                }
                Ok(_) => return Err("Task executable URL is not a file URL".into()),
                Err(error) => {
                    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                    {
                        let _ = error;
                        let (path, route, check) = reference_shortcuts_path(value, return_pc)?;
                        reference_check = Some((route, check));
                        path
                    }
                    #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
                    {
                        let _ = return_pc;
                        return Err(error);
                    }
                }
            }
        };
        if path.contains('\0') {
            return Err("Task executable contains NUL".into());
        }
        fresh_fields(receiver, &fields)?;
        let mut config = fields.task.config.lock().map_err(|_| "Task config poisoned")?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        {
            let mut marker = fields.shortcuts_route.lock().map_err(|_| "Shortcuts Task route poisoned")?;
            if let Some((_, check)) = &reference_check {
                check()?;
            }
            *marker = reference_check.as_ref().map(|(route, _)| *route);
            config.monitor_locale_verified = false;
        }
        config.executable = path.into();
        Ok(())
    })
}
unsafe extern "C" fn url(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let fields = super::task_fields(receiver)?;
        let path = path_value(fields.as_ref())?;
        if path.is_empty() {
            return Ok(std::ptr::null_mut());
        }
        let text = foundation::Text::from_utf8(path.as_bytes())?;
        let result = returned_value(foundation::FoundationValue::Url(Arc::new(
            foundation::Url::file_path(&text, false)?,
        )))?;
        fresh_fields(receiver, &fields)?;
        Ok(result)
    })
}
unsafe extern "C" fn set_path(receiver: *mut AnyObject, _: Sel, value: *mut AnyObject) {
    boundary((), || {
        let fields = super::task_fields(receiver)?;
        let path = if value.is_null() {
            String::new()
        } else {
            get_string(value)?
        };
        if path.contains('\0') {
            return Err("Task executable contains NUL".into());
        }
        fresh_fields(receiver, &fields)?;
        let mut config = fields.task.config.lock().map_err(|_| "Task config poisoned")?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        {
            *fields.shortcuts_route.lock().map_err(|_| "Shortcuts Task route poisoned")? = None;
            config.monitor_locale_verified = false;
        }
        config.executable = path.into();
        Ok(())
    })
}
unsafe extern "C" fn path(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let fields = super::task_fields(receiver)?;
        let path = path_value(fields.as_ref())?;
        if path.is_empty() {
            return Ok(std::ptr::null_mut());
        }
        let result = returned_value(foundation::FoundationValue::String(Arc::new(
            foundation::Text::from_utf8(path.as_bytes())?,
        )))?;
        fresh_fields(receiver, &fields)?;
        Ok(result)
    })
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_arguments(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_arguments_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn set_arguments(receiver: *mut AnyObject, selector: Sel, value: *mut AnyObject) {
    unsafe { set_arguments_checked(receiver, selector, value, 0) };
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
/// Read only the bounded text of a compiler-borrowed Foundation string.
/// # Safety
/// The verified original caller must keep a live Objective-C argument
/// owned for this read, including tagged-pointer representations.
pub(super) unsafe fn reference_string(value: *mut AnyObject) -> Result<String> {
    use objc2_foundation::NSRange;
    let object = unsafe { value.as_ref() }.ok_or("Reference Foundation string absent")?;
    let class = object.class();
    let (offsets, image, uuid, encodings) = match class.name().to_bytes() {
        name @ (b"Swift.__StringStorage" | b"Swift.__SharedStringStorage") => (
            if name == b"Swift.__StringStorage" { [0x27dec, 0x27e04] } else { [0xca38, 0x33fd1c] },
            b"/usr/lib/swift/libswiftCore.dylib".as_slice(),
            [0x83, 0x79, 0x4f, 0xb3, 0xde, 0x9b, 0x3d, 0x23,
                0xab, 0x5e, 0x2c, 0x1d, 0x5d, 0x30, 0xf1, 0x34],
            [b"q16@0:8".as_slice(), b"v40@0:8^S16{?=qq}24".as_slice()],
        ),
        name => (
            match name {
                b"NSTaggedPointerString" => [0xfb18, 0xfc90],
                b"__NSCFString" | b"__NSCFConstantString" => [0x72f4, 0x15dcc],
                b"NSIndirectTaggedPointerString" => [0x1603b8, 0x1602e0],
                _ => return Err("Reference Foundation string class differs".into()),
            },
            b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation".as_slice(),
            [0x9b, 0x67, 0x27, 0x62, 0x7b, 0x1f, 0x30, 0xbc,
                0x96, 0xde, 0xf1, 0x76, 0xb3, 0x72, 0xd6, 0x6d],
            [b"Q16@0:8".as_slice(), b"v40@0:8^S16{_NSRange=QQ}24".as_slice()],
        ),
    };
    let class = crate::sdk::metadata::registered_class(class as *const AnyClass as usize)?;
    let select = |selector, index: usize| -> Result<usize> {
        let method = crate::sdk::metadata::method(class, selector, false)?
            .ok_or("Reference Foundation string method absent")?;
        let code = method.code();
        if method.encoding() != encodings[index] || code.offset() != offsets[index]
            || code.image() != image || code.uuid() != uuid
        {
            return Err("Reference Foundation string method differs".into());
        }
        Ok(method.implementation())
    };
    // The signed Swift and unsigned CF encodings use identical arm64
    // registers. Negative lengths fail the upper bound before any read.
    let length: unsafe extern "C-unwind" fn(&AnyObject, Sel) -> usize =
        unsafe { std::mem::transmute(select(c"length", 0)?) };
    let read: unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut u16, NSRange) =
        unsafe { std::mem::transmute(select(c"getCharacters:range:", 1)?) };
    let count = unsafe { length(object, Sel::register(c"length")) };
    if count > 32 {
        return Err("Reference Foundation string length differs".into());
    }
    let mut units = [0u16; 32];
    unsafe { read(object, Sel::register(c"getCharacters:range:"), units.as_mut_ptr(), NSRange::new(0, count)) };
    String::from_utf16(&units[..count]).map_err(|_| "Reference Foundation string is invalid UTF-16".into())
}

unsafe extern "C" fn set_arguments_checked(
    receiver: *mut AnyObject, _: Sel, value: *mut AnyObject, return_pc: usize,
) {
    boundary((), || {
        let fields = super::task_fields(receiver)?;
        let values = if value.is_null() {
            Ok(vec![])
        } else {
            foundation_array_elements(value)
        };
        let values = match values {
            Ok(values) => values,
            Err(error) => {
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                {
                    let marker = *fields.shortcuts_route.lock().map_err(|_| "Shortcuts Task route poisoned")?;
                    if let Some(marker) = marker {
                        let (route, expected, check) = crate::sdk::process_router::verify_reference_shortcuts_setter(return_pc, Some(marker))?;
                        if route != marker {
                            return Err("Reference Shortcuts Task setter route differs".into());
                        }
                        let file_search;
                        let arguments: &[&str] = match route {
                            ReferenceTaskRoute::Discovery => &["list", "--show-identifiers"],
                            ReferenceTaskRoute::Script => &["__run-script"],
                            ReferenceTaskRoute::ProcessTable => &crate::sdk::process_router::REFERENCE_PS_ARGS,
                            ReferenceTaskRoute::FileSearch => {
                                file_search = crate::sdk::process_router::reference_file_search_args();
                                &[file_search[0].as_str(), file_search[1].as_str()]
                            }
                        };
                        for (index, argument) in arguments.iter().enumerate() {
                            let item = unsafe { super::reference_swift_array_element(value, arguments.len() as isize, index as isize) }?;
                            if unsafe { reference_string(item)? } != *argument {
                                return Err("Reference Shortcuts argument bytes differ".into());
                            }
                        }
                        fresh_fields(receiver, &fields)?;
                        let mut config = fields.task.config.lock().map_err(|_| "Task config poisoned")?;
                        let mut marker = fields.shortcuts_route.lock().map_err(|_| "Shortcuts Task route poisoned")?;
                        check()?;
                        if config.executable != expected || *marker != Some(route) {
                            return Err("Reference Shortcuts Task URL admission changed".into());
                        }
                        *marker = None;
                        config.arguments = arguments.iter().map(|argument| (*argument).to_owned()).collect();
                        config.monitor_locale_verified = false;
                        fields.arguments_present.store(true, Ordering::Release);
                        return Ok(());
                    }
                }
                let _ = return_pc;
                return Err(error);
            }
        };
        if values.len() > 64 {
            return Err("Task arguments exceed registered route count".into());
        }
        let mut arguments = Vec::with_capacity(values.len());
        for value in values {
            let text = get_string(value.as_ptr()?)?;
            // Same bounds as Registry::register, not a second argv policy.
            if text.len() > 8192 || text.contains('\0') {
                return Err("Task argument exceeds registered route bounds".into());
            }
            arguments.push(text);
        }
        fresh_fields(receiver, &fields)?;
        let mut config = fields.task.config.lock().map_err(|_| "Task config poisoned")?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        {
            *fields.shortcuts_route.lock().map_err(|_| "Shortcuts Task route poisoned")? = None;
            config.monitor_locale_verified = false;
        }
        config.arguments = arguments;
        fields
            .arguments_present
            .store(!value.is_null(), Ordering::Release);
        Ok(())
    })
}
unsafe extern "C" fn arguments(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let fields = super::task_fields(receiver)?;
        if !fields.arguments_present.load(Ordering::Acquire) {
            return Ok(std::ptr::null_mut());
        }
        let arguments = fields
            .task
            .config
            .lock()
            .map_err(|_| "Task config poisoned")?
            .arguments
            .clone();
        let mut values = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let pointer = new_foundation_value(foundation::FoundationValue::String(Arc::new(
                foundation::Text::from_utf8(argument.as_bytes())?,
            )))?;
            values.push(unsafe { foundation_created_reference(pointer) }?);
        }
        let pointer = new_foundation_array(values)?;
        let result = unsafe { autorelease_foundation(pointer) }?;
        fresh_fields(receiver, &fields)?;
        Ok(result)
    })
}
fn set_stream(receiver: *mut AnyObject, value: *mut AnyObject, index: usize) -> Result<()> {
    let fields = super::task_fields(receiver)?;
    let (stream, reference) = if value.is_null() {
        (Stream::Null, None)
    } else {
        let (stream, reference) = super::task_stream_reference(value)?;
        (stream, Some(reference))
    };
    fresh_fields(receiver, &fields)?;
    let previous = {
        let mut references = fields.streams.lock().map_err(|_| "Task streams poisoned")?;
        let reference_slot = references
            .get_mut(index)
            .ok_or("Task stream index invalid")?;
        let mut config = fields
            .task
            .config
            .lock()
            .map_err(|_| "Task config poisoned")?;
        let stream_slot = match index {
            0 => &mut config.input,
            1 => &mut config.output,
            2 => &mut config.error,
            _ => return Err("Task stream index invalid".into()),
        };
        (
            std::mem::replace(stream_slot, stream),
            std::mem::replace(reference_slot, reference),
        )
    };
    drop(previous);
    Ok(())
}
fn stream(receiver: *mut AnyObject, index: usize) -> Result<*mut AnyObject> {
    let fields = super::task_fields(receiver)?;
    let result = match fields.stream(index)? {
        None => Ok(std::ptr::null_mut()),
        Some(reference) => reference.autoreleased_pointer(),
    }?;
    fresh_fields(receiver, &fields)?;
    Ok(result)
}
unsafe extern "C" fn set_input(p: *mut AnyObject, _: Sel, value: *mut AnyObject) {
    boundary((), || set_stream(p, value, 0))
}
unsafe extern "C" fn set_output(p: *mut AnyObject, _: Sel, value: *mut AnyObject) {
    boundary((), || set_stream(p, value, 1))
}
unsafe extern "C" fn set_error(p: *mut AnyObject, _: Sel, value: *mut AnyObject) {
    boundary((), || set_stream(p, value, 2))
}
unsafe extern "C" fn input(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || stream(p, 0))
}
unsafe extern "C" fn output(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || stream(p, 1))
}
unsafe extern "C" fn error(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || stream(p, 2))
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn set_environment(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym set_environment_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn set_environment(receiver: *mut AnyObject, selector: Sel, value: *mut AnyObject) {
    unsafe { set_environment_checked(receiver, selector, value, 0) }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn is_monitor_process(config: &crate::sdk::task_model::Config) -> bool {
    config.executable == std::path::Path::new("/bin/ps")
        && config.arguments.iter().map(String::as_str).eq(crate::sdk::process_router::REFERENCE_PS_ARGS)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe fn verify_monitor_locale(value: *mut AnyObject) -> Result<impl Fn() -> Result<()>> {
    use crate::sdk::metadata;
    use objc2_core_foundation::{CFRetained, CFString};
    let object = unsafe { value.as_ref() }.ok_or("Reference monitor environment absent")?;
    let class = object.class();
    if class.name() != c"_TtGCs26_SwiftDeferredNSDictionarySSSS_$" {
        return Err("Reference monitor environment dictionary class differs".into());
    }
    let handle = metadata::registered_class(class as *const AnyClass as usize)?;
    let method = metadata::method(handle, c"objectForKey:", false)?
        .ok_or("Reference monitor environment lookup absent")?;
    let code = method.code();
    if method.owner() != handle.address() || method.encoding() != b"@24@0:8@16"
        || code.image() != b"/usr/lib/swift/libswiftCore.dylib" || code.offset() != 0x1eb978
        || code.uuid() != [0x83, 0x79, 0x4f, 0xb3, 0xde, 0x9b, 0x3d, 0x23,
            0xab, 0x5e, 0x2c, 0x1d, 0x5d, 0x30, 0xf1, 0x34]
    {
        return Err("Reference monitor environment lookup differs".into());
    }
    let key = CFString::from_str("LC_ALL");
    let lookup: unsafe extern "C-unwind" fn(&AnyObject, Sel, *const AnyObject) -> *mut AnyObject =
        unsafe { std::mem::transmute(method.implementation()) };
    let locale = unsafe { lookup(object, Sel::register(c"objectForKey:"),
        CFRetained::as_ptr(&key).as_ptr().cast()) };
    if unsafe { reference_string(locale)? } != "C" {
        return Err("Reference monitor locale differs".into());
    }
    let current = move || metadata::verify_current_method(handle, c"objectForKey:", false, &method);
    current()?;
    Ok(current)
}

unsafe extern "C" fn set_environment_checked(receiver: *mut AnyObject, selector: Sel, value: *mut AnyObject, caller: usize) {
    boundary((), || {
        let fields = super::task_fields(receiver)?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        {
            let monitor = {
                let config = fields.task.config.lock().map_err(|_| "Task config poisoned")?;
                is_monitor_process(&config)
            };
            if monitor {
                if selector.name() != c"setEnvironment:" {
                    return Err("Reference monitor environment selector differs".into());
                }
                let current = crate::sdk::process_router::verify_reference_monitor_environment(caller)?;
                let lookup_current = unsafe { verify_monitor_locale(value)? };
                fresh_fields(receiver, &fields)?;
                lookup_current()?;
                let mut config = fields.task.config.lock().map_err(|_| "Task config poisoned")?;
                let mut environment = fields.environment.lock().map_err(|_| "Task environment poisoned")?;
                current()?;
                if !is_monitor_process(&config) || environment.is_some() {
                    return Err("Reference monitor environment changed or replayed".into());
                }
                *environment = Some(BTreeMap::from([("LC_ALL".into(), "C".into())]));
                config.monitor_locale_verified = true;
                return Ok(());
            }
        }
        let _ = (selector, caller);
        let decoded = if value.is_null() {
            None
        } else {
            Some(foundation_string_dictionary(value)?)
        };
        // Decoding verifies each actual Foundation child and can invoke the
        // existing source guards. Recheck the receiver before pure publication.
        if !Arc::ptr_eq(&fields, &super::task_fields(receiver)?) {
            return Err("Task environment receiver changed during decoding".into());
        }
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let mut config = fields.task.config.lock().map_err(|_| "Task config poisoned")?;
        let mut environment = fields
            .environment
            .lock()
            .map_err(|_| "Task environment poisoned")?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        { config.monitor_locale_verified = false; }
        *environment = decoded;
        Ok(())
    })
}
unsafe extern "C" fn environment(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let fields = super::task_fields(receiver)?;
        let values = fields
            .environment
            .lock()
            .map_err(|_| "Task environment poisoned")?
            .clone();
        let result = match values {
            None => Ok(std::ptr::null_mut()),
            Some(values) => {
                let pointer = new_foundation_string_dictionary(&values)?;
                // Consume precisely this fresh common dictionary factory +1.
                unsafe { autorelease_foundation(pointer) }
            }
        }?;
        fresh_fields(receiver, &fields)?;
        Ok(result)
    })
}

unsafe extern "C" fn init_task(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || unsafe { super::init_task(p) })
}
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn init_pipe(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || unsafe { super::init_pipe(p, false) })
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn init_pipe(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym init_pipe_checked);
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn init_pipe_checked(p: *mut AnyObject, _: Sel, caller: usize) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let parent_reader = crate::workspace::callbacks::reference_shortcuts_pipe_reader(caller)?;
        unsafe { super::init_pipe(p, parent_reader) }
    })
}
pub(super) fn handle_initializer() -> unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject {
    init_handle
}

unsafe extern "C" fn init_handle(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || unsafe { super::init_handle(p) })
}
fn returned_task(pointer: *mut AnyObject) -> Result<*mut AnyObject> {
    let pending = unsafe { super::super::factory::OwnedReceiver::from_owned(pointer) }?;
    let result = unsafe { super::super::factory::autorelease_owned(pointer) }?;
    pending.into_raw();
    Ok(result)
}
unsafe extern "C" fn pipe_factory(class: *mut AnyClass, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        returned_task(super::new_task_pipe(class)?)
    })
}
unsafe extern "C" fn null_factory(class: *mut AnyClass, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        returned_task(super::new_task_null_handle(class)?)
    })
}
unsafe extern "C" fn pipe_read(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || super::task_pipe_handle(p, false))
}
unsafe extern "C" fn pipe_write(p: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || super::task_pipe_handle(p, true))
}
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn descriptor(p: *mut AnyObject, _: Sel) -> i32 {
    boundary(-1, || Ok(super::handle(p)?.loan()?.as_raw_fd()))
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn descriptor(_: *mut AnyObject, _: Sel) -> i32 {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym descriptor_checked);
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn descriptor_checked(p: *mut AnyObject, _: Sel, caller: usize) -> i32 {
    boundary(-1, || {
        let loan = super::handle(p)?.loan()?;
        let fd = loan.as_raw_fd();
        crate::workspace::callbacks::hold_reference_shortcuts_descriptor(caller, loan)?;
        Ok(fd)
    })
}
/// Business result used by both the selected nonthrowing API and the pending
/// NSError API. No original NSError factory or substitute error object is used.
fn read_owned(p: *mut AnyObject, limit: Option<usize>) -> Result<Vec<u8>> {
    let handle = super::handle(p)?;
    match limit {
        None => {
            let context = crate::sdk::process_abi::installed_context()?;
            handle.verify_registry(&context.registry)?;
            handle.read_to_end(context.life.as_ref())
        }
        Some(limit) => read_handle(&handle, limit).map_err(Failure::description),
    }
}
fn read_handle(handle: &FileHandle, limit: usize) -> std::result::Result<Vec<u8>, Failure> {
    let context = crate::sdk::process_abi::installed_context()
        .map_err(|e| Failure::policy(Operation::Read, "registry.context", e))?;
    handle
        .verify_registry(&context.registry)
        .map_err(|e| Failure::policy(Operation::Read, "handle.issuer", e))?;
    handle.read_failure(limit, context.life.as_ref())
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn read_end(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    core::arch::naked_asm!("mov x2, x30", "b {checked}", checked = sym read_end_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn read_end(p: *mut AnyObject, selector: Sel) -> *mut AnyObject {
    unsafe { read_end_checked(p, selector, 0) }
}

unsafe extern "C" fn read_end_checked(p: *mut AnyObject, _: Sel, _caller: usize) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let monitor_bridge = crate::sdk::system_providers::monitor_timeout::qualify_pipe_read_bridge(_caller, p as usize)?;
        let bytes = read_owned(p, None)?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        {
            use crate::sdk::system_providers::monitor_timeout;
            monitor_timeout::observe_assembly_stage("Task pipe read completed");
            if monitor_bridge {
                use objc2_core_foundation::{CFData, CFRetained};
                monitor_timeout::qualify_pipe_read_bridge(_caller, p as usize)?;
                // Preserve the original Swift Data bridge for the private
                // pipe output. CFData copies the already owned bytes.
                let value = CFRetained::into_raw(CFData::from_bytes(&bytes));
                let value = unsafe { Retained::from_raw(value.as_ptr().cast::<AnyObject>()) }
                    .ok_or("Reference monitor pipe data allocation failed")?;
                return Ok(Retained::autorelease_return(value));
            }
        }
        returned_value(foundation::FoundationValue::Data(Arc::new(
            foundation::Data::new(bytes),
        )))
    })
}
unsafe extern "C" fn read_length(p: *mut AnyObject, _: Sel, length: usize) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let bytes = read_owned(p, Some(length))?;
        returned_value(foundation::FoundationValue::Data(Arc::new(
            foundation::Data::new(bytes),
        )))
    })
}
fn write_owned(p: *mut AnyObject, value: *mut AnyObject) -> std::result::Result<(), Failure> {
    let policy = |error| Failure::policy(Operation::Write, "handle.arguments", error);
    let handle = super::handle(p).map_err(policy)?;
    let bytes = foundation_bytes(value).map_err(policy)?;
    write_handle(p, &handle, &bytes, || Ok(()))
}
fn write_handle(
    p: *mut AnyObject,
    handle: &FileHandle,
    bytes: &[u8],
    check: impl Fn() -> Result<()>,
) -> std::result::Result<(), Failure> {
    let policy = |error| Failure::policy(Operation::Write, "handle.arguments", error);
    let current = super::handle(p).map_err(policy)?;
    if !handle.same_handle(&current) {
        return Err(Failure::policy(
            Operation::Write,
            "handle.identity",
            "Handle receiver changed during data decoding",
        ));
    }
    let context = crate::sdk::process_abi::installed_context().map_err(policy)?;
    handle.verify_registry(&context.registry).map_err(policy)?;
    current.verify_registry(&context.registry).map_err(policy)?;
    // The actual qualified receiver is rechecked after decoding; original data
    // is never cast or messaged, and no foreign FD can be adopted by Handle.
    handle.write_checked(bytes, context.life.as_ref(), check)
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) use reference_write::spec as reference_write_spec;
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) use reference_write::{bridge as bridge_swift_data, SwiftData};

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
mod reference_write {
    use super::*;
    use std::ffi::c_void;

    type Object = *mut c_void;
    const SYMBOL: &CStr =
        c"$sSo12NSFileHandleC10FoundationE5write10contentsOfyx_tKAC12DataProtocolRzlF";

    #[repr(C)]
    pub(crate) struct SwiftData(pub(crate) [usize; 2]);

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        #[link_name = "$sSo12NSFileHandleC10FoundationE5write10contentsOfyx_tKAC12DataProtocolRzlF"]
        static ORIGINAL: u8;
        #[link_name = "$s10Foundation4DataVN"]
        static DATA_METADATA: u8;
        #[link_name = "$s10Foundation4DataV19_bridgeToObjectiveCSo6NSDataCyF"]
        pub(crate) fn bridge(data: SwiftData) -> Object;
        #[link_name = "$s10Foundation22_convertNSErrorToErrorys0E0_pSo0C0CSgF"]
        fn swift_error(error: Object) -> Object;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDataGetLength(data: *const c_void) -> isize;
        fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
        fn CFErrorCreate(allocator: Object, domain: Object, code: isize, info: Object) -> Object;
        static kCFErrorDomainPOSIX: Object;
        fn CFRelease(object: *const c_void);
    }

    pub(crate) fn spec() -> crate::sdk::ImportSpec {
        crate::sdk::ImportSpec {
            symbol: SYMBOL,
            original: (&raw const ORIGINAL) as usize,
            replacement: entry as *const () as usize,
        }
    }

    #[unsafe(naked)]
    unsafe extern "C" fn entry() {
        core::arch::naked_asm!(
            "mov x3, x20",
            "mov x4, x30",
            "stp x29, x30, [sp, #-16]!",
            "mov x29, sp",
            "bl {write}",
            "mov x21, x0",
            "ldp x29, x30, [sp], #16",
            "ret",
            write = sym write,
        );
    }

    extern "C" fn write(
        data: *const SwiftData,
        metadata: usize,
        witness: usize,
        receiver: *mut AnyObject,
        caller: usize,
    ) -> Object {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let policy = |error| Failure::policy(Operation::Write, "handle.swift_data", error);
            let check = crate::sdk::process_router::verify_reference_script_write(caller, witness)
                .map_err(policy)?;
            if metadata != (&raw const DATA_METADATA) as usize
                || data.is_null()
                || !(data as usize).is_multiple_of(std::mem::align_of::<SwiftData>())
            {
                return Err(policy("Reference script Data ABI differs".into()));
            }
            let handle = super::super::handle(receiver).map_err(policy)?;
            check().map_err(policy)?;
            // The admitted Main caller owns this indirect Data value until
            // return. Borrow its two words without taking Swift ownership.
            let bridged = unsafe { bridge(SwiftData((*data).0)) };
            if bridged.is_null() {
                return Err(policy("Reference script Data bridge returned nil".into()));
            }
            struct Bridged(Object);
            impl Drop for Bridged {
                fn drop(&mut self) {
                    unsafe { CFRelease(self.0) };
                }
            }
            let bridged = Bridged(bridged);
            let length = unsafe { CFDataGetLength(bridged.0) };
            if !(1..=16 * 1024).contains(&length) {
                return Err(policy("Reference script Data length differs".into()));
            }
            let bytes = unsafe { CFDataGetBytePtr(bridged.0) };
            if bytes.is_null() {
                return Err(policy("Reference script Data bytes absent".into()));
            }
            let bytes = unsafe { std::slice::from_raw_parts(bytes, length as usize) };
            check().map_err(policy)?;
            write_handle(receiver, &handle, bytes, check)
        }));
        let errno = match result {
            Ok(Ok(())) => return std::ptr::null_mut(),
            Ok(Err(failure)) => match failure.kind {
                crate::sdk::task_model::FailureKind::Posix(errno) => errno,
                _ => libc::EPERM,
            },
            Err(payload) => {
                std::mem::forget(payload);
                unsafe { libc::_exit(78) };
            }
        };
        unsafe {
            let nil = std::ptr::null_mut();
            let error = CFErrorCreate(nil, kCFErrorDomainPOSIX, errno as isize, nil);
            if error.is_null() {
                libc::_exit(78);
            }
            let result = swift_error(error);
            CFRelease(error);
            if result.is_null() {
                libc::_exit(78);
            }
            result
        }
    }
}

unsafe extern "C" fn write_data(p: *mut AnyObject, _: Sel, value: *mut AnyObject) {
    boundary((), || write_owned(p, value).map_err(Failure::description))
}
fn close_owned(p: *mut AnyObject) -> Result<()> {
    // Exact cleanup identity/FD issuer survives business retirement. No new
    // read, write, launch or process-authority callback is admitted here.
    super::cleanup_task_handle(p)?.close()
}
unsafe extern "C" fn close_file(p: *mut AnyObject, _: Sel) {
    boundary((), || close_owned(p))
}
fn clear_error(error: *mut *mut AnyObject) -> Result<()> {
    if !error.is_null() {
        if !(error as usize).is_multiple_of(std::mem::align_of::<*mut AnyObject>()) {
            return Err("unaligned Task error output".into());
        }
        // Typed source callers provide a writable NSError* output slot.
        unsafe { error.write(std::ptr::null_mut()) };
    }
    Ok(())
}
fn write_error(error: *mut *mut AnyObject, failure: &Failure) -> Result<()> {
    if !error.is_null() {
        let pointer = returned_task(super::new_task_error(failure)?)?;
        unsafe { error.write(pointer) };
    }
    Ok(())
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn launch_error(_: *mut AnyObject, _: Sel, _: *mut *mut AnyObject) -> Bool {
    core::arch::naked_asm!("mov x3, x30", "b {checked}", checked = sym launch_error_checked);
}

#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn launch_error(p: *mut AnyObject, selector: Sel, error: *mut *mut AnyObject) -> Bool {
    unsafe { launch_error_checked(p, selector, error, 0) }
}

unsafe extern "C" fn launch_error_checked(
    p: *mut AnyObject, selector: Sel, error: *mut *mut AnyObject, caller: usize,
) -> Bool {
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    use crate::sdk::system_providers::monitor_timeout::observe_assembly_stage;
    boundary(Bool::NO, || {
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        observe_assembly_stage("Task launch entry");
        if selector.name() != c"launchAndReturnError:" {
            return Err("Task launch selector differs".into());
        }
        let fields = super::task_fields(p)?;
        clear_error(error)?;
        let context = crate::sdk::process_abi::installed_context()?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let monitor = match crate::sdk::system_providers::monitor_timeout::begin_task_timeout_launch(
            caller, p as usize, fields.task.is_reference_file_search()?,
        ) {
            Ok(admission) => admission,
            Err(detail) => {
                observe_assembly_stage("Task launch caller admission refused");
                write_error(error, &Failure::policy(Operation::Launch, "monitor.admission", detail))?;
                return Ok(Bool::NO);
            }
        };
        let _ = caller;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let timeout_route = monitor.as_ref().map(|admission| admission.route());
        #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
        let timeout_route = None;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if monitor.is_some() {
            fresh_fields(p, &fields)?;
        }
        match fields.task.launch_failure(&context.registry, timeout_route) {
            Ok(()) => {
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                observe_assembly_stage("Task launch committed");
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                if let Some(admission) = monitor {
                    admission.commit();
                }
                Ok(Bool::YES)
            }
            Err(description) => {
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                observe_assembly_stage(description.assembly_launch_stage());
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                drop(monitor);
                write_error(error, &description)?;
                Ok(Bool::NO)
            }
        }
    })
}
unsafe extern "C" fn read_error(
    p: *mut AnyObject,
    _: Sel,
    length: usize,
    error: *mut *mut AnyObject,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let handle = super::handle(p)?;
        clear_error(error)?;
        match read_handle(&handle, length) {
            Ok(bytes) => returned_value(foundation::FoundationValue::Data(Arc::new(
                foundation::Data::new(bytes),
            ))),
            Err(description) => {
                write_error(error, &description)?;
                Ok(std::ptr::null_mut())
            }
        }
    })
}
unsafe extern "C" fn write_error_imp(
    p: *mut AnyObject,
    _: Sel,
    value: *mut AnyObject,
    error: *mut *mut AnyObject,
) -> Bool {
    boundary(Bool::NO, || {
        super::handle(p)?;
        clear_error(error)?;
        match write_owned(p, value) {
            Ok(()) => Ok(Bool::YES),
            Err(description) => {
                write_error(error, &description)?;
                Ok(Bool::NO)
            }
        }
    })
}
unsafe extern "C" fn close_error(p: *mut AnyObject, _: Sel, error: *mut *mut AnyObject) -> Bool {
    boundary(Bool::NO, || {
        let handle = super::cleanup_task_handle(p)?;
        clear_error(error)?;
        match handle.close_failure() {
            Ok(()) => Ok(Bool::YES),
            Err(description) => {
                write_error(error, &description)?;
                Ok(Bool::NO)
            }
        }
    })
}

unsafe extern "C" fn running(receiver: *mut AnyObject, _: Sel) -> Bool {
    boundary(Bool::NO, || {
        Ok(Bool::new(super::task(receiver)?.running()?))
    })
}
unsafe extern "C" fn terminate(receiver: *mut AnyObject, _: Sel) {
    boundary((), || super::task(receiver)?.terminate())
}
unsafe extern "C" fn pid(receiver: *mut AnyObject, _: Sel) -> i32 {
    boundary(-1, || super::task_fields(receiver)?.task.pid())
}
pub(super) fn wait_implementation() -> unsafe extern "C" fn(*mut AnyObject, Sel) {
    wait
}

unsafe extern "C" fn wait(receiver: *mut AnyObject, _: Sel) {
    boundary((), || super::task_fields(receiver)?.task.wait().map(|_| ()))
}
unsafe extern "C" fn status(receiver: *mut AnyObject, _: Sel) -> i32 {
    let status = boundary(-1, || Ok(super::task_fields(receiver)?.task.status()?.0));
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    crate::sdk::system_providers::monitor_timeout::observe_assembly_stage(match status {
        0 => "Task exit status zero",
        -1 => "Task exit status unavailable",
        _ => "Task exit status nonzero",
    });
    status
}
unsafe extern "C" fn reason(receiver: *mut AnyObject, _: Sel) -> isize {
    boundary(0, || {
        isize::try_from(super::task_fields(receiver)?.task.status()?.1)
            .map_err(|_| "Task termination reason exceeds NSInteger".into())
    })
}

macro_rules! entries {
    ($emit:ident) => {
        $emit! {
            (c"NSTask",c"init",false,init_task,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSTask",c"setExecutableURL:",false,set_url,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
            (c"NSTask",c"executableURL",false,url,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSTask",c"setLaunchPath:",false,set_path,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
            (c"NSTask",c"launchPath",false,path,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSTask",c"setArguments:",false,set_arguments,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
            (c"NSTask",c"arguments",false,arguments,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSTask",c"setEnvironment:",false,set_environment,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
            (c"NSTask",c"environment",false,environment,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSTask",c"setStandardInput:",false,set_input,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
            (c"NSTask",c"standardInput",false,input,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSTask",c"setStandardOutput:",false,set_output,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
            (c"NSTask",c"standardOutput",false,output,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSTask",c"setStandardError:",false,set_error,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
            (c"NSTask",c"standardError",false,error,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSTask",c"isRunning",false,running,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
            (c"NSTask",c"launchAndReturnError:",false,launch_error,unsafe extern "C" fn(*mut AnyObject,Sel,*mut *mut AnyObject)->Bool),
            (c"NSTask",c"terminate",false,terminate,unsafe extern "C" fn(*mut AnyObject,Sel)),
            (c"NSTask",c"processIdentifier",false,pid,unsafe extern "C" fn(*mut AnyObject,Sel)->i32),
            (c"NSTask",c"waitUntilExit",false,wait,unsafe extern "C" fn(*mut AnyObject,Sel)),
            (c"NSTask",c"terminationStatus",false,status,unsafe extern "C" fn(*mut AnyObject,Sel)->i32),
            (c"NSTask",c"terminationReason",false,reason,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
            (c"NSPipe",c"init",false,init_pipe,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSPipe",c"pipe",true,pipe_factory,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
            (c"NSPipe",c"fileHandleForReading",false,pipe_read,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSPipe",c"fileHandleForWriting",false,pipe_write,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSFileHandle",c"init",false,init_handle,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSFileHandle",c"fileHandleWithNullDevice",true,null_factory,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
            (c"NSFileHandle",c"fileDescriptor",false,descriptor,unsafe extern "C" fn(*mut AnyObject,Sel)->i32),
            (c"NSFileHandle",c"readDataToEndOfFile",false,read_end,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
            (c"NSFileHandle",c"readDataOfLength:",false,read_length,unsafe extern "C" fn(*mut AnyObject,Sel,usize)->*mut AnyObject),
            (c"NSFileHandle",c"readDataUpToLength:error:",false,read_error,unsafe extern "C" fn(*mut AnyObject,Sel,usize,*mut *mut AnyObject)->*mut AnyObject),
            (c"NSFileHandle",c"writeData:",false,write_data,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)),
            (c"NSFileHandle",c"writeData:error:",false,write_error_imp,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,*mut *mut AnyObject)->Bool),
            (c"NSFileHandle",c"closeFile",false,close_file,unsafe extern "C" fn(*mut AnyObject,Sel)),
            (c"NSFileHandle",c"closeAndReturnError:",false,close_error,unsafe extern "C" fn(*mut AnyObject,Sel,*mut *mut AnyObject)->Bool),
        }
    };
}
macro_rules! exports {
    ($(($class:expr,$selector:expr,$class_method:expr,$function:ident,$ty:ty)),*$(,)?) => {
        pub(super) fn replacements() -> Vec<(&'static CStr,&'static CStr,bool,usize)> {
            vec![$(($class,$selector,$class_method,$function as *const () as usize)),*]
        }
        pub(super) fn signatures() -> Vec<(&'static CStr,&'static CStr,bool,Vec<objc2::encode::Encoding>)> {
            vec![$(($class,$selector,$class_method,signature::<$ty>())),*]
        }
    };
}
fn signature<F: objc2::runtime::MethodImplementation>() -> Vec<objc2::encode::Encoding> {
    use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
    let mut result = vec![F::Return::ENCODING_RETURN, Encoding::Object, Sel::ENCODING];
    result.extend_from_slice(F::Arguments::ENCODINGS);
    result
}
entries!(exports);
