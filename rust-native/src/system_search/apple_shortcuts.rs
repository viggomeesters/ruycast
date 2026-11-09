use super::*;
use std::{
    io::Read,
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Command as Process, Stdio},
    time::Duration,
};

pub const PREFIX: &str = "apple-shortcut.";
pub const FAILURE: &str = "Shortcut did not finish. Check Shortcuts before retrying.";
pub const TIMEOUT: &str =
    "Shortcut run timed out. Its result is unknown. Check Shortcuts before retrying.";
pub fn identifier(value: &str) -> Option<String> {
    if value.len() != 36
        || !value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}
pub fn recognizes(c: &Command) -> bool {
    let Some(id) = c.id.strip_prefix(PREFIX).and_then(identifier) else {
        return false;
    };
    c.id == format!("{PREFIX}{id}")
        && c.kind == "script"
        && c.target == "/usr/bin/shortcuts"
        && c.arguments == ["run", &id]
}
pub fn parse(data: &[u8]) -> Result<Vec<Command>, String> {
    if data.len() > 65_536 {
        return Err("Shortcut list is too large to load.".into());
    }
    let text =
        std::str::from_utf8(data).map_err(|_| "Could not read shortcuts. Refresh to retry.")?;
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for raw in text.lines() {
        let line = raw.trim_matches(|c: char| c == ' ' || c == '\t');
        if line.is_empty() {
            continue;
        }
        if line.len() < 39 || !line.ends_with(')') {
            return Err("Could not read shortcut names. Check Shortcuts, then refresh.".into());
        }
        let start = line.len() - 38;
        if !line.is_char_boundary(start)
            || line.as_bytes()[start] != b'('
            || !line[..start].ends_with(char::is_whitespace)
        {
            return Err("Could not read shortcut identifiers. Refresh to retry.".into());
        }
        let id = identifier(&line[start + 1..line.len() - 1])
            .ok_or("Could not read shortcut identifiers. Refresh to retry.")?;
        if !seen.insert(id.clone()) {
            return Err("Could not read shortcut identifiers. Refresh to retry.".into());
        }
        // Reject embedded controls before whitespace normalization (names may
        // contain spaces/emoji/parentheses, but may not create extra rows).
        let name = line[..start]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err("Could not read shortcut names. Check Shortcuts, then refresh.".into());
        }
        if out.len() >= 1000 {
            return Err("Too many shortcuts to search. The limit is 1,000.".into());
        }
        out.push(serde_json::from_value(serde_json::json!({"id":format!("{PREFIX}{id}"),"title":name,"kind":"script","target":"/usr/bin/shortcuts","arguments":["run",id],"timeout":300})).expect("fixed command"));
    }
    Ok(out)
}
/// Own only the CLI child. Closing this client does not cancel a workflow or
/// daemon, and all output remains private; callers receive generic statuses.
pub fn list(ticket: &Ticket) -> Result<Vec<Command>, String> {
    parse(&capture(
        "/usr/bin/shortcuts",
        &["list", "--show-identifiers"],
        ticket,
        65_536,
    )?)
}
pub fn run(id: &str, ticket: &Ticket) -> Result<(), String> {
    let id = identifier(id).ok_or(FAILURE)?;
    finish_run(
        capture("/usr/bin/shortcuts", &["run", &id], ticket, 65_536),
        ticket,
    )
}
// Workflow bytes are never interpreted as a script/UI response. Keep this
// boundary shared by the actual CLI route and its owned capture tests.
fn finish_run(result: Result<Vec<u8>, String>, ticket: &Ticket) -> Result<(), String> {
    result.map(|_| ()).map_err(|_| {
        if Instant::now() >= ticket.deadline {
            TIMEOUT.into()
        } else {
            FAILURE.into()
        }
    })
}
pub fn capture(
    executable: &str,
    args: &[&str],
    ticket: &Ticket,
    limit: usize,
) -> Result<Vec<u8>, String> {
    ticket.check()?;
    let mut child = Process::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|_| "Could not load shortcuts. Check Shortcuts, then refresh.")?;
    struct Owned(std::process::Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }
    let out = child.stdout.take().ok_or("Shortcut output unavailable.")?;
    let err = child.stderr.take().ok_or("Shortcut output unavailable.")?;
    let mut owned = Owned(child);
    let mut pipes: [Box<dyn ReadFd>; 2] = [Box::new(out), Box::new(err)];
    for pipe in &pipes {
        let fd = pipe.fd();
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err("Could not load shortcuts. Refresh to retry.".into());
            }
        }
    }
    let mut data = vec![];
    let mut counts = [0usize; 2];
    let mut eof = [false; 2];
    let mut buf = [0u8; 8192];
    loop {
        ticket.check()?;
        for index in 0..2 {
            if eof[index] {
                continue;
            }
            match pipes[index].read(&mut buf) {
                Ok(0) => eof[index] = true,
                Ok(n) => {
                    counts[index] = counts[index].saturating_add(n);
                    if counts[index] > limit {
                        return Err("Shortcut list is too large to load.".into());
                    }
                    if index == 0 {
                        data.extend_from_slice(&buf[..n]);
                    }
                }
                Err(e)
                    if [
                        std::io::ErrorKind::WouldBlock,
                        std::io::ErrorKind::Interrupted,
                    ]
                    .contains(&e.kind()) => {}
                Err(_) => return Err("Could not load shortcuts. Refresh to retry.".into()),
            }
        }
        if let Some(status) = owned.0.try_wait().map_err(|_| FAILURE)? {
            if eof.iter().all(|&v| v) {
                if !status.success() {
                    return Err("Could not load shortcuts. Check Shortcuts, then refresh.".into());
                }
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    ticket.check()?;
    Ok(data)
}
trait ReadFd: Read {
    fn fd(&self) -> i32;
}
impl<T: Read + AsRawFd> ReadFd for T {
    fn fd(&self) -> i32 {
        self.as_raw_fd()
    }
}

/// UUID overrides, including saved aliases/hotkeys/disabled state, precede
/// discovery and are ranked using the same command kernel as the main catalog.
pub fn merged(discovered: &[Command], saved: &[Command], query: &str) -> Vec<Command> {
    discovered
        .iter()
        .map(|c| saved.iter().find(|s| s.id == c.id).unwrap_or(c))
        .filter(|c| {
            c.enabled != Some(false)
                && super::matches(&format!("{} {}", c.title, c.aliases.join(" ")), query)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(seconds: f64) -> Ticket {
        Ticket {
            deadline: Instant::now() + Duration::from_secs_f64(seconds),
            live: Arc::new(|| true),
        }
    }

    #[test]
    fn workflow_output_and_private_failures_never_become_ui_or_public_error_data() {
        let secret = "SYNTHETIC_PRIVATE_SHORTCUT_OUTPUT";
        let payload = serde_json::json!({
            "ok": true,
            "detail": secret,
            "choices": {"items": [{"id": "one", "title": secret}]},
            "stderr": secret
        })
        .to_string();
        let t = ticket(2.);
        let captured = capture("/usr/bin/printf", &["%s", &payload], &t, 65_536).unwrap();
        assert_eq!(captured, payload.as_bytes());
        let completed: Result<(), String> = finish_run(Ok(captured), &t);
        assert_eq!(completed, Ok(()));
        assert_eq!(
            serde_json::to_value(completed).unwrap(),
            serde_json::json!({"Ok": null})
        );

        let t = ticket(2.);
        let invalid_utf8 = capture("/usr/bin/printf", &["%b", r"\0377"], &t, 65_536).unwrap();
        assert_eq!(invalid_utf8, [255]);
        assert_eq!(finish_run(Ok(invalid_utf8), &t), Ok(()));
        // A native capture has no runner JSON dictionary or decoder. Even a
        // private internal failure is replaced before the typed action escapes.
        assert_eq!(finish_run(Err(secret.into()), &t), Err(FAILURE.into()));
        for executable in ["/usr/bin/false", "/owned/missing-shortcuts-fixture"] {
            let result = capture(executable, &[], &t, 65_536);
            assert!(result.is_err());
            assert_eq!(finish_run(result, &t), Err(FAILURE.into()));
        }
    }

    #[test]
    fn elapsed_capture_deadline_reports_unknown_result_without_reinterpreting_output() {
        let t = ticket(0.05);
        let started = Instant::now();
        let result = capture("/bin/sleep", &["5"], &t, 65_536);
        assert!(Instant::now() >= t.deadline);
        assert_eq!(result, Err(CANCELLED.into()));
        assert_eq!(finish_run(result, &t), Err(TIMEOUT.into()));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(TIMEOUT.contains("result is unknown"));
        // finish_run consumes an already-finished Result, not a launch closure;
        // there is no executable/argument authority with which it could retry.
        assert_eq!(
            finish_run(Err("SYNTHETIC_PRIVATE_FAILURE".into()), &t),
            Err(TIMEOUT.into())
        );
    }
}
