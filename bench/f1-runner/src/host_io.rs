//! Producer-only owned Host transport. Source-include this neutral module;
//! never link the SDK rlib into the producer (__interpose is real there).
//! Preparation is not runnable authority and is valid only while this holder
//! survives. Each persistent Worker gets its own one-shot cap/artifact set.
use serde::Serialize;
use std::{
    io,
    os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
};
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Fingerprint {
    pub device: u64,
    pub inode: u64,
    pub kind: u32,
    pub access: i32,
}
fn fingerprint(fd: &impl AsRawFd) -> io::Result<Fingerprint> {
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fstat(fd.as_raw_fd(), info.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    Ok(Fingerprint {
        device: info.st_dev as u64,
        inode: info.st_ino,
        kind: u32::from(info.st_mode & libc::S_IFMT),
        access: flags & libc::O_ACCMODE,
    })
}
fn owner(fd: &impl AsRawFd) -> io::Result<u32> {
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd.as_raw_fd(), info.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { info.assume_init() }.st_uid)
}
struct ReadCaps {
    input: OwnedFd,
    output: OwnedFd,
    error: OwnedFd,
    sdk: crate::guardian::OwnedReadGrant,
    native: crate::guardian::OwnedReadGrant,
    input_writer: OwnedFd,
    output_reader: OwnedFd,
}
/// Fresh actual factories only. No raw-FD adoption or public arbitrary-fstat
/// constructor can manufacture this held producer preparation.
pub struct HostIo {
    caps: ReadCaps,
    sdk_writer: OwnedFd,
    native_writer: OwnedFd,
}
/// Both grant writers have been consumed and closed. Pipe reads are one-shot:
/// this holder cannot refill/rearm them or produce a second fresh child session.
pub struct PublishedHostIo {
    caps: ReadCaps,
}
/// Only the parent's live command/response ends survive successful spawn.
/// This carries no grant writer or child-side duplicate that could hide EOF.
pub struct ParentIo {
    input: OwnedFd,
    output: OwnedFd,
}
impl ParentIo {
    pub fn into_fds(self) -> (OwnedFd, OwnedFd) {
        (self.input, self.output)
    }
}
impl HostIo {
    pub fn new() -> io::Result<Self> {
        let (input, input_writer) = crate::guardian::pipe()?;
        if unsafe { libc::fcntl(input_writer.as_raw_fd(), libc::F_SETNOSIGPIPE, 1) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let (output_reader, output) = crate::guardian::pipe()?;
        let (sdk, sdk_writer) = crate::guardian::private_grant_pipe()?;
        let (native, native_writer) = crate::guardian::private_grant_pipe()?;
        let raw = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let error = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Self {
            caps: ReadCaps {
                input,
                output,
                error,
                sdk,
                native,
                input_writer,
                output_reader,
            },
            sdk_writer,
            native_writer,
        })
    }
    pub fn native_compile_pin(&self, nonce_sha256: &str) -> io::Result<serde_json::Value> {
        if nonce_sha256.len() != 64
            || !nonce_sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "nonce SHA256 must be exact lowercase hex",
            ));
        }
        Ok(
            serde_json::json!({"fd":self.caps.native.as_raw_fd(),"fingerprint":fingerprint(&self.caps.native)?,"nonce_sha256":nonce_sha256}),
        )
    }
    pub fn native_owner_uid(&self) -> io::Result<u32> {
        owner(&self.caps.native)
    }
    pub fn sdk_owner_uid(&self) -> io::Result<u32> {
        owner(&self.caps.sdk)
    }
    pub fn sdk_pins(&self) -> io::Result<(Fingerprint, [Fingerprint; 3])> {
        Ok((
            fingerprint(&self.caps.sdk)?,
            [
                fingerprint(&self.caps.input)?,
                fingerprint(&self.caps.output)?,
                fingerprint(&self.caps.error)?,
            ],
        ))
    }
    pub fn native_pin(&self) -> io::Result<(i32, Fingerprint)> {
        Ok((
            self.caps.native.as_raw_fd(),
            fingerprint(&self.caps.native)?,
        ))
    }
    pub fn stdio(&self) -> [BorrowedFd<'_>; 3] {
        [
            self.caps.input.as_fd(),
            self.caps.output.as_fd(),
            self.caps.error.as_fd(),
        ]
    }
    pub fn sdk_grant(&self) -> BorrowedFd<'_> {
        self.caps.sdk.as_fd()
    }
    pub fn native_grant(&self) -> BorrowedFd<'_> {
        self.caps.native.as_fd()
    }
    /// Publish BOTH complete immutable snapshots before exec. A physically full
    /// pipe refuses without blocking; callers must not spawn after this error.
    /// JSON is owned trusted-producer data, not a substitute for receiver pins.
    pub fn publish(
        self,
        sdk: &serde_json::Value,
        native: &serde_json::Value,
    ) -> io::Result<PublishedHostIo> {
        let sdk = serde_json::to_vec(sdk)?;
        let native = serde_json::to_vec(native)?;
        let Self {
            caps,
            sdk_writer,
            native_writer,
        } = self;
        write_snapshot(sdk_writer, &sdk, "SDK")?;
        write_snapshot(native_writer, &native, "Native")?;
        Ok(PublishedHostIo { caps })
    }
}
impl PublishedHostIo {
    /// Existing stdio-only Guardian route, not a full Host-grant launch. The
    /// owner itself supplies this exact stdio and consumes all child ends only
    /// after successful kernel admission. Failure drops the one-shot holder.
    /// Extra SDK/Native readcap inheritance remains a separately reviewed path.
    pub fn spawn_stdio(
        self,
        program: &std::path::Path,
        args: &[String],
        env: &[(String, String)],
        cwd: &std::path::Path,
    ) -> io::Result<(crate::guardian::Endpoint, ParentIo)> {
        let descriptors = self.stdio().map(|fd| fd.as_raw_fd());
        let endpoint = crate::guardian::Endpoint::start_with_io(
            program,
            args,
            env,
            &crate::guardian::SpawnIo { cwd, descriptors },
        )?;
        Ok((endpoint, self.into_parent()))
    }
    /// Data role: inherit exactly this owner's SDK read grant, never Native's.
    pub fn spawn_with_sdk_grant(
        self,
        program: &std::path::Path,
        args: &[String],
        env: &[(String, String)],
        cwd: &std::path::Path,
    ) -> io::Result<(crate::guardian::Endpoint, ParentIo)> {
        self.spawn_with_private_grants(program, args, env, cwd, false)
    }
    /// Full Host role transport only; its source/fourteen-scope admission remains
    /// independent. Both grants are this exact one-shot owner's private pipes.
    pub fn spawn_with_host_grants(
        self,
        program: &std::path::Path,
        args: &[String],
        env: &[(String, String)],
        cwd: &std::path::Path,
    ) -> io::Result<(crate::guardian::Endpoint, ParentIo)> {
        self.spawn_with_private_grants(program, args, env, cwd, true)
    }
    fn spawn_with_private_grants(
        self,
        program: &std::path::Path,
        args: &[String],
        env: &[(String, String)],
        cwd: &std::path::Path,
        include_native: bool,
    ) -> io::Result<(crate::guardian::Endpoint, ParentIo)> {
        let descriptors = self.stdio().map(|fd| fd.as_raw_fd());
        let mut grants = vec![&self.caps.sdk];
        if include_native {
            grants.push(&self.caps.native);
        }
        let endpoint = crate::guardian::Endpoint::start_with_read_grants(
            program,
            args,
            env,
            &crate::guardian::SpawnIo { cwd, descriptors },
            &grants,
        )?;
        Ok((endpoint, self.into_parent()))
    }
    fn into_parent(self) -> ParentIo {
        let ReadCaps {
            input,
            output,
            error,
            sdk,
            native,
            input_writer,
            output_reader,
        } = self.caps;
        drop((input, output, error, sdk, native));
        ParentIo {
            input: input_writer,
            output: output_reader,
        }
    }
    pub fn stdio(&self) -> [BorrowedFd<'_>; 3] {
        [
            self.caps.input.as_fd(),
            self.caps.output.as_fd(),
            self.caps.error.as_fd(),
        ]
    }
    pub fn sdk_grant(&self) -> BorrowedFd<'_> {
        self.caps.sdk.as_fd()
    }
    pub fn native_grant(&self) -> BorrowedFd<'_> {
        self.caps.native.as_fd()
    }
    pub fn parent_input(&self) -> BorrowedFd<'_> {
        self.caps.input_writer.as_fd()
    }
    pub fn parent_output(&self) -> BorrowedFd<'_> {
        self.caps.output_reader.as_fd()
    }
}
fn snapshot_error(grant: &'static str, phase: &'static str, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("{grant} grant {phase}: os_error={:?}", error.raw_os_error()),
    )
}
fn write_snapshot(writer: OwnedFd, bytes: &[u8], grant: &'static str) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > 16384 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Host grant JSON exceeds16KiB bound",
        ));
    }
    let flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(snapshot_error(grant, "F_GETFL", io::Error::last_os_error()));
    }
    if unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
        return Err(snapshot_error(grant, "F_SETFL", io::Error::last_os_error()));
    }
    let mut framed = (bytes.len() as u32).to_ne_bytes().to_vec();
    framed.extend_from_slice(bytes);
    let mut at = 0;
    while at < framed.len() {
        let n = unsafe {
            libc::write(
                writer.as_raw_fd(),
                framed[at..].as_ptr().cast(),
                framed.len() - at,
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(snapshot_error(grant, "write", e));
        }
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("{grant} grant write: zero progress"),
            ));
        }
        at += n as usize;
    }
    drop(writer); // EOF is part of the protocol, not an optional timing hint.
    Ok(())
}

#[cfg(test)]
mod snapshot_error_tests {
    use super::*;
    #[test]
    fn context_preserves_error_class_without_frame_content() {
        let error = snapshot_error("SDK", "write", io::Error::from_raw_os_error(libc::EAGAIN));
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            error.to_string(),
            format!("SDK grant write: os_error=Some({})", libc::EAGAIN)
        );
        let other = snapshot_error(
            "Native",
            "F_SETFL",
            io::Error::from_raw_os_error(libc::EBADF),
        );
        assert_eq!(
            other.to_string(),
            format!("Native grant F_SETFL: os_error=Some({})", libc::EBADF)
        );
    }
}
