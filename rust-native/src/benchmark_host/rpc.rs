//! Absolute-deadline newline JSON over an owned Unix stream.
use serde_json::Value;
use std::{
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, net::UnixStream},
    },
    path::Path,
    time::{Duration, Instant},
};
const RESPONSE_LIMIT: usize = 1024 * 1024;

fn remaining(deadline: Instant) -> io::Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(io::Error::new(io::ErrorKind::TimedOut, "RPC deadline"))
    } else {
        Ok(left)
    }
}
fn poll(stream: &UnixStream, events: i16, deadline: Instant) -> io::Result<()> {
    loop {
        let left = remaining(deadline)?;
        let millis = left.as_millis().saturating_add(1).min(i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd: stream.as_raw_fd(),
            events,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut descriptor, 1, millis) };
        remaining(deadline)?;
        if result > 0 {
            if descriptor.revents & libc::POLLNVAL != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid owned socket",
                ));
            }
            return Ok(());
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

pub fn connect(socket: &Path, deadline: Instant) -> io::Result<UnixStream> {
    remaining(deadline)?;
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let name = socket.as_os_str().as_bytes();
    if name.contains(&0) || name.len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid owned Unix socket path",
        ));
    }
    address.sun_family = libc::AF_UNIX as _;
    address.sun_len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + name.len() + 1) as _;
    for (target, byte) in address.sun_path.iter_mut().zip(name) {
        *target = *byte as _;
    }
    let descriptor = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(descriptor) };
    stream.set_nonblocking(true)?;
    let enabled: i32 = 1;
    if unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        || unsafe {
            libc::setsockopt(
                descriptor,
                libc::SOL_SOCKET,
                libc::SO_NOSIGPIPE,
                (&enabled as *const i32).cast(),
                std::mem::size_of_val(&enabled) as _,
            )
        } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe {
        libc::connect(
            descriptor,
            (&address as *const libc::sockaddr_un).cast(),
            address.sun_len as _,
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS)
            && error.kind() != io::ErrorKind::WouldBlock
        {
            return Err(error);
        }
        poll(&stream, libc::POLLOUT, deadline)?;
        if let Some(error) = stream.take_error()? {
            return Err(error);
        }
        stream.peer_addr()?;
    }
    remaining(deadline)?;
    Ok(stream)
}

/// One absolute deadline spans connect, complete write, and the entire reply.
pub fn request(socket: &Path, value: &Value, deadline: Instant) -> io::Result<Value> {
    let mut stream = connect(socket, deadline)?;
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let mut request = bytes.as_slice();
    while !request.is_empty() {
        remaining(deadline)?;
        match stream.write(request) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "RPC request")),
            Ok(count) => request = &request[count..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                poll(&stream, libc::POLLOUT, deadline)?
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    let mut response = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        remaining(deadline)?;
        match stream.read(&mut buffer) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete RPC reply",
                ))
            }
            Ok(count) => {
                let newline = buffer[..count].iter().position(|byte| *byte == b'\n');
                let used = newline.unwrap_or(count);
                if response.len() + used > RESPONSE_LIMIT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "RPC reply exceeds 1 MiB",
                    ));
                }
                response.extend_from_slice(&buffer[..used]);
                if newline.is_some() {
                    let parsed = serde_json::from_slice(&response)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                    remaining(deadline)?;
                    return Ok(parsed);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                poll(&stream, libc::POLLIN, deadline)?
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}
