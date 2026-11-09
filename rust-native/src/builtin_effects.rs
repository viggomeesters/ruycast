//! Reuse the actual configuration writer over the owner-only control socket.
//! This is a bounded client, not a second configuration mutation authority.
use crate::system_search::Ticket;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};
pub fn reload(root: &Path, ticket: &Ticket) -> Result<(), String> {
    ticket.check()?;
    let mut stream = UnixStream::connect(root.join("control.sock"))
        .map_err(|_| "Could not reach the configuration owner.")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(50)))
        .map_err(|_| "Could not prepare configuration reload.")?;
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .map_err(|_| "Could not prepare configuration reload.")?;
    stream.write_all(b"{\"op\":\"reload\"}\n").map_err(|_| {
        "Configuration reload result is unknown. Check configuration before retrying."
    })?;
    let mut bytes = vec![];
    let mut chunk = [0u8; 4096];
    loop {
        ticket.check().map_err(|_| {
            "Configuration reload result is unknown. Check configuration before retrying."
        })?;
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(
                    "Configuration reload result is unknown. Check configuration before retrying."
                        .into(),
                )
            }
            Ok(n) => {
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.len() > 1_048_576 {
                    return Err("Configuration reload response is too large.".into());
                }
                if let Some(end) = bytes.iter().position(|b| *b == b'\n') {
                    let response: serde_json::Value = serde_json::from_slice(&bytes[..end])
                        .map_err(|_| "Configuration reload returned an invalid response.")?;
                    return if response["ok"] == true {
                        Ok(())
                    } else {
                        Err(response["error"]
                            .as_str()
                            .unwrap_or("Configuration could not be reloaded.")
                            .into())
                    };
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => {
                return Err(
                    "Configuration reload result is unknown. Check configuration before retrying."
                        .into(),
                )
            }
        }
    }
}
pub fn validate(
    command: &rustcast_core::Command,
    values: &serde_json::Value,
) -> Result<(), String> {
    if !values.is_null()
        && values
            .as_object()
            .is_none_or(|m| m.values().any(|v| !v.is_string()))
    {
        return Err("Input values must be strings.".into());
    }
    if let Some(prompt) = &command.input_prompt {
        let empty = objc2::rc::autoreleasepool(|_| {
            objc2_foundation::NSString::from_str(values["input"].as_str().unwrap_or(""))
                .stringByTrimmingCharactersInSet(
                    &objc2_foundation::NSCharacterSet::whitespaceAndNewlineCharacterSet(),
                )
                .is_empty()
        });
        if empty {
            return Err(format!("Enter {prompt}."));
        }
    }
    crate::execution::budget(command, values["input"].as_str())
}
