//! Own-process protocol/fault harness, excluded from production builds.
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{self, Read},
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    helper: PathBuf,
    command: rustcast_core::Command,
    home: PathBuf,
    cancel_ms: Option<u64>,
    panic_ms: Option<u64>,
    deadline_ms: Option<u64>,
}
fn main() {
    let mut data = String::new();
    io::stdin()
        .take(8 * 1024 * 1024 + 8192)
        .read_to_string(&mut data)
        .unwrap();
    let input: Input = serde_json::from_str(&data).unwrap();
    let start = Instant::now();
    let result = std::panic::catch_unwind(|| {
        ruycast_native::supervision::probe(
            &input.helper,
            &input.command,
            &input.home,
            input
                .deadline_ms
                .map(|ms| start + Duration::from_millis(ms)),
            || {
                assert!(
                    input
                        .panic_ms
                        .is_none_or(|ms| start.elapsed() < Duration::from_millis(ms)),
                    "own supervisor callback panic"
                );
                input
                    .cancel_ms
                    .is_some_and(|ms| start.elapsed() >= Duration::from_millis(ms))
            },
        )
    });
    let value = match result {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => json!({"error":error}),
        Err(_) => json!({"panicCaught":true}),
    };
    let mut status = 0;
    let child = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
    let clean = child == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD);
    println!(
        "{}",
        json!({"result":value,"noChildren":clean,"elapsedMs":start.elapsed().as_millis()})
    );
    assert!(clean, "supervisor left an owned helper");
}
