//! Owned fake executable and headless owner harness. Never shipped.
use std::{io::Write, path::PathBuf};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--init-config" | "--headless") if args.len() == 1 => {
            if let Err(error) = benchmark_host_adapter(&args[0]) {
                eprintln!("Owned benchmark adapter refused: {error}");
                std::process::exit(1);
            }
        }
        Some("host") => {
            let home = PathBuf::from(&args[1]);
            let paths = ruycast_native::profile::ProfilePaths::new(home.join("p"), home, None);
            match ruycast_native::runtime::run_fixture(paths) {
                Ok(result) => println!("{result}"),
                Err(error) => {
                    eprintln!("Owned host failed: {error}");
                    std::process::exit(1)
                }
            }
        }
        Some("fake") => {
            let mode = &args[1];
            let mut marker = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&args[2])
                .unwrap();
            marker.write_all(b"run\n").unwrap();
            drop(marker);
            if mode == "wait" {
                std::thread::sleep(std::time::Duration::from_secs(4));
            }
            if mode == "date-detail" {
                print!("{}", serde_json::json!({"ok":true,"detail":args.last().unwrap()}));
                std::io::stdout().flush().unwrap();
                return;
            }
            if mode == "cycle" {
                let value = args.last().unwrap();
                let output = if value.is_empty() {
                    serde_json::json!({"ok":true,"detail":"Owned cycle report","choices":{"title":"Owned","placeholder":"Find","items":[{"id":"one","title":"One","subtitle":"","value":"literal Ω\n$(no-shell)"}]}})
                } else {
                    serde_json::json!({"ok":true,"received":value})
                };
                print!("{output}");
            } else if mode == "echo" {
                print!("{}", args.last().unwrap());
            } else {
                print!("{}", args[3]);
            }
            std::io::stdout().flush().unwrap();
            if mode == "exit7" || mode == "exit23" {
                eprintln!("SYNTHETIC_PRIVATE_SECRET_9817");
                std::process::exit(if mode == "exit7" { 7 } else { 23 })
            }
        }
        _ => panic!("Expected owned host/fake mode"),
    }
}

// Only the unchanged benchmark driver's two legacy host arguments are bridged.
// This lives in the existing test-tools fixture binary, never the product host.
fn benchmark_host_adapter(operation: &str) -> Result<(), String> {
    use std::{fs, os::unix::{ffi::OsStrExt, fs::{DirBuilderExt, FileTypeExt, MetadataExt}, process::CommandExt}, path::Path, process::Command};
    let profile = PathBuf::from(std::env::var_os("RUYCAST_HOME").ok_or("missing explicit benchmark profile")?);
    let parent = PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").ok_or("missing owned benchmark parent")?);
    benchmark_private_directory(&parent)?;
    benchmark_private_directory(&profile)?;
    if profile.parent() != Some(parent.as_path())
        || !profile.file_name().is_some_and(|name| name.to_string_lossy().starts_with("rc-h-"))
        || profile.join("control.sock").as_os_str().as_bytes().len() > 104 {
        return Err("benchmark profile is not a short direct owned child".into());
    }
    benchmark_marker(&parent.join(".ruycast-test-fixture"), false)?;
    // Explicit paths and independently supplied current artifact pins only.
    let cli = benchmark_pinned_tool("RUYCAST_CLI_PARITY_BINARY")?;
    let host = benchmark_pinned_tool("RUYCAST_CLI_PARITY_NATIVE")?;
    benchmark_marker(&profile.join(".ruycast-test-fixture"), true)?;
    let applications = profile.join("Applications");
    match fs::DirBuilder::new().mode(0o700).create(&applications) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.to_string()),
    }
    benchmark_private_directory(&applications)?;
    if fs::read_dir(&applications).map_err(|error| error.to_string())?.next().is_some() {
        return Err("benchmark discovery fixture is not empty".into());
    }
    match fs::symlink_metadata(profile.join("control.sock")) {
        Ok(metadata) if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } =>
            return Err("benchmark control endpoint is not an own socket".into()),
        Ok(_) => (), // The existing runtime alone owns stale/live socket handling.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.to_string()),
    }
    let (binary, arguments): (&Path, &[&str]) = match operation {
        "--init-config" => (&cli, &["config", "init"]),
        "--headless" => (&host, &["--headless-test"]),
        _ => return Err("unsupported benchmark adapter argument".into()),
    };
    let mut command = Command::new(binary);
    command.args(arguments).env_clear()
        .env("PATH", "/usr/bin:/bin").env("HOME", &profile)
        .env("RUYCAST_HOME", &profile).env("RUSTCAST_HOME", &profile)
        .env("TMPDIR", &profile).env("RUYCAST_APP_BINARY", &host)
        .env("RUSTCAST_APP_BINARY", &host)
        .env("RUYCAST_NATIVE_TEST_APP_ROOTS", serde_json::to_string(&[&applications]).map_err(|error| error.to_string())?);
    // exec keeps the driver's exact child PID and existing graceful/forced
    // cleanup/deadline semantics; there is no adapter supervisor or fallback.
    Err(command.exec().to_string())
}
fn benchmark_private_directory(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !path.is_absolute() || path.canonicalize().map_err(|error| error.to_string())? != path
        || !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0 {
        return Err("benchmark directory must be canonical, owned and private".into());
    }
    Ok(())
}
fn benchmark_marker(path: &std::path::Path, create: bool) -> Result<(), String> {
    use std::{io::{Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt}};
    const MARKER: &[u8] = b"own-disposable-fixture-v1";
    if create {
        match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(path) {
            Ok(mut file) => file.write_all(MARKER).map_err(|error| error.to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.to_string()),
        }
    }
    let mut file = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(path).map_err(|error| error.to_string())?;
    let before = file.metadata().map_err(|error| error.to_string())?;
    if !before.is_file() || before.uid() != unsafe { libc::geteuid() } || before.mode() & 0o077 != 0 || before.len() != MARKER.len() as u64 {
        return Err("benchmark marker is not its exact owned private file".into());
    }
    let mut bytes = Vec::new();
    (&mut file).take(MARKER.len() as u64 + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    let after = file.metadata().map_err(|error| error.to_string())?;
    let current = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if bytes != MARKER || benchmark_file_identity(&before) != benchmark_file_identity(&after)
        || benchmark_file_identity(&before) != benchmark_file_identity(&current) {
        return Err("benchmark marker changed during qualification".into());
    }
    Ok(())
}
fn benchmark_file_identity(metadata: &std::fs::Metadata) -> (u64,u64,u32,u32,u64,u64,i64,i64,i64,i64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(),metadata.ino(),metadata.uid(),metadata.mode(),metadata.nlink(),metadata.len(),metadata.mtime(),metadata.mtime_nsec(),metadata.ctime(),metadata.ctime_nsec())
}
fn benchmark_pinned_tool(key: &str) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    use std::{io::Read, os::unix::fs::{MetadataExt, OpenOptionsExt}};
    let path = PathBuf::from(std::env::var_os(key).ok_or("missing explicit benchmark binary")?);
    let expected = std::env::var(format!("{key}_SHA256")).map_err(|_| "missing current benchmark artifact pin")?;
    if !path.is_absolute() || path.canonicalize().map_err(|error| error.to_string())? != path
        || expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("benchmark binary path or SHA is invalid".into());
    }
    let mut file = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(&path).map_err(|error| error.to_string())?;
    let before = file.metadata().map_err(|error| error.to_string())?;
    if !before.is_file() || before.uid() != unsafe { libc::geteuid() } || before.mode() & 0o022 != 0
        || before.mode() & 0o111 == 0 || before.len() > 128 * 1024 * 1024 {
        return Err("benchmark binary is not an owned bounded executable".into());
    }
    let mut digest = Sha256::new();let mut buffer = [0u8;65536];let mut total = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {break;}
        total = total.checked_add(count as u64).ok_or("benchmark binary length overflow")?;
        if total > 128 * 1024 * 1024 {return Err("benchmark binary grew beyond bound".into());}
        digest.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(|error| error.to_string())?;
    let current = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if format!("{:x}",digest.finalize()) != expected.to_ascii_lowercase()
        || benchmark_file_identity(&before) != benchmark_file_identity(&after)
        || benchmark_file_identity(&before) != benchmark_file_identity(&current) {
        return Err("benchmark artifact differs from its explicit pin".into());
    }
    Ok(path)
}
