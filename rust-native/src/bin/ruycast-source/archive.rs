//! Small canonical USTAR writer; compression is the unchanged system gzip.
use super::*;
pub fn tar(files: &BTreeMap<String, Vec<u8>>, modes: &BTreeMap<String, u32>) -> Result<Vec<u8>> {
    let mut members = BTreeMap::<String, (u8, u32, &[u8])>::new();
    members.insert("ruycast/".into(), (b'5', 0o755, &[]));
    for (path, bytes) in files {
        let mode = *modes.get(path).ok_or("Missing archive mode")?;
        let path = format!("ruycast/{path}");
        for (at, _) in path.match_indices('/') {
            members
                .entry(path[..=at].to_owned())
                .or_insert((b'5', 0o755, &[]));
        }
        members.insert(path, (b'0', mode, bytes.as_slice()));
    }
    let mut out = vec![];
    for (path, (kind, mode, bytes)) in members {
        let (prefix, name) = if path.len() <= 100 {
            ("", path.as_str())
        } else {
            path.match_indices('/')
                .rev()
                .find_map(|(at, _)| {
                    (at <= 155 && path.len() - at - 1 <= 100)
                        .then_some((&path[..at], &path[at + 1..]))
                })
                .ok_or("USTAR path is too long")?
        };
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(format!("{mode:07o}\0").as_bytes());
        h[108..116].copy_from_slice(b"0000000\0");
        h[116..124].copy_from_slice(b"0000000\0");
        h[124..136].copy_from_slice(format!("{:011o}\0", bytes.len()).as_bytes());
        h[136..148].copy_from_slice(b"00000000000\0");
        h[148..156].fill(b' ');
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        let sum = h.iter().map(|b| u64::from(*b)).sum::<u64>();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend(h);
        out.extend(bytes);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.extend([0; 1024]);
    Ok(out)
}
pub fn gzip(input: &Path, decompress: bool, limit: usize) -> Result<Vec<u8>> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        sync::mpsc,
        time::{Duration, Instant},
    };
    let file = regular(input)?.0;
    let child = Command::new("/usr/bin/gzip")
        .args(if decompress {
            vec!["-d", "-c"]
        } else {
            vec!["-n", "-c"]
        })
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(file)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(err)?;
    struct Reaper(std::process::Child);
    impl Drop for Reaper {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }
    let mut child = Reaper(child);
    let stdout = child.0.stdout.take().ok_or("gzip output unavailable")?;
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        let result = stdout
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let until = Instant::now() + Duration::from_secs(30);
    let result = loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(result) => {
                break result.map_err(err).and_then(|b| {
                    if b.len() > limit {
                        Err("gzip output exceeds the bounded artifact size".into())
                    } else {
                        Ok(b)
                    }
                })
            }
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < until => continue,
            _ => break Err("gzip stopped or timed out".into()),
        }
    };
    if result.is_err() {
        let _ = child.0.kill();
    }
    let status = loop {
        match child.0.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(2)),
            Ok(None) => {
                let _ = child.0.kill();
                let _ = child.0.wait();
                break Err("gzip exit timed out".to_string());
            }
            Err(e) => break Err(err(e)),
        }
    };
    let _ = reader.join();
    let result = result?;
    if !status?.success() {
        return Err("gzip failed".into());
    }
    Ok(result)
}
