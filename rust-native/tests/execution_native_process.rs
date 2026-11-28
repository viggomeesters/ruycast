//! Native launcher execution on a real isolated hidden host.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::native_bridge::{
    runtime_host::{default_config, Config},
    Bridge,
};
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    thread,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires an explicitly prepared synthetic Work capsule and fixture writer"]
fn real_work_status_admission_and_recovery_fixture() {
    use sha2::{Digest, Sha256};
    use std::{io::{Read, Write}, os::{fd::AsRawFd, unix::fs::{MetadataExt, OpenOptionsExt}}, path::PathBuf, process::{Command, Stdio}};

    let capsule = PathBuf::from(std::env::var_os("VIGGO_STATUS_FIXTURE_CAPSULE").expect("prepared capsule"));
    let base = Path::new("/Users/viggomeesters/Dev/obsidian-test-vault/.tmp");
    assert_eq!(base.canonicalize().unwrap(), base);
    assert_eq!(capsule.parent(), Some(base));
    assert_eq!(capsule.canonicalize().unwrap(), capsule);
    let root = capsule.join("work");
    let note = root.join("00-inbox/20261126-status-fixture/20261126-status-fixture.md");
    let state = root.join(".work-command-state");
    let writer_source = PathBuf::from(std::env::var_os("VIGGO_STATUS_FIXTURE_WRITER").expect("fixture writer"));
    let meta = fs::symlink_metadata(&writer_source).unwrap();
    assert!(meta.is_file() && !meta.file_type().is_symlink() && meta.uid() == unsafe { libc::getuid() } && meta.nlink() == 1 && meta.len() < 128 * 1024 * 1024);
    assert_eq!(writer_source.canonicalize().unwrap(), writer_source);
    let mut source = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&writer_source).unwrap();
    let opened = source.metadata().unwrap();
    assert_eq!((opened.dev(), opened.ino()), (meta.dev(), meta.ino()));
    let mut writer_bytes = Vec::new();
    Read::by_ref(&mut source).take(128 * 1024 * 1024 + 1).read_to_end(&mut writer_bytes).unwrap();
    assert!(writer_bytes.len() <= 128 * 1024 * 1024);
    assert_eq!(format!("{:x}", Sha256::digest(&writer_bytes)), std::env::var("VIGGO_STATUS_FIXTURE_WRITER_SHA256").unwrap());
    let engine_sha = std::env::var("VIGGO_STATUS_FIXTURE_ENGINE_SHA256").unwrap();
    let formatter_sha = std::env::var("VIGGO_STATUS_FIXTURE_FORMATTER_SHA256").unwrap();
    let stage = std::env::var("VIGGO_STATUS_FIXTURE_STAGE").unwrap_or_else(|_| "journal".into());
    let mode = std::env::var("VIGGO_STATUS_FIXTURE_CASE").unwrap_or_else(|_| "host-loss".into());
    assert!(["before-grant", "journal", "postimage", "receipt"].contains(&stage.as_str()));
    assert!(["host-loss", "dismiss", "drift"].contains(&mode.as_str()));
    assert!(matches!((stage.as_str(), mode.as_str()),
        ("before-grant", "dismiss") | ("journal", "host-loss") |
        ("postimage", "host-loss") | ("postimage", "dismiss") | ("postimage", "drift") | ("receipt", "host-loss")));
    let mut host = Bridge::new(Config::current(), false);
    host.env.insert("VIGGO_STATUS_FIXTURE_CAPSULE".into(), capsule.as_os_str().to_owned());
    host.env.insert("VIGGO_STATUS_FIXTURE_ENGINE_SHA256".into(), engine_sha.clone().into());
    host.env.insert("VIGGO_STATUS_FIXTURE_FORMATTER_SHA256".into(), formatter_sha.clone().into());
    host.env.insert("VIGGO_STATUS_FIXTURE_CHECKPOINT".into(), stage.clone().into());
    let dir = host.base.join(".local/libexec");
    fs::create_dir_all(&dir).unwrap();
    let writer = dir.join("viggo-work-writer");
    fs::OpenOptions::new().write(true).create_new(true).mode(0o700).open(&writer).unwrap().write_all(&writer_bytes).unwrap();
    fs::set_permissions(&writer, fs::Permissions::from_mode(0o700)).unwrap();
    let mut identity = Command::new(&writer);
    identity.env_clear().env("PATH", "/usr/bin:/bin").env("TMPDIR", capsule.join("control")).current_dir(&capsule)
        .env("VIGGO_STATUS_FIXTURE_CAPSULE", &capsule)
        .env("VIGGO_STATUS_FIXTURE_ENGINE_SHA256", &engine_sha)
        .env("VIGGO_STATUS_FIXTURE_FORMATTER_SHA256", &formatter_sha)
        .arg("--fixture-identity-v1").stdout(Stdio::piped()).stderr(Stdio::piped());
    let identity = child_output::wait(identity.spawn().unwrap(), Duration::from_secs(10));
    assert!(identity.status.success(), "fixture identity refused");
    assert_eq!(serde_json::from_slice::<Value>(&identity.stdout).unwrap(), json!({"ok":true,"schema":"viggo.work-status-fixture.v1","capsule":capsule,"root":root,"note":"00-inbox/20261126-status-fixture/20261126-status-fixture.md"}));
    assert_eq!(root.canonicalize().unwrap(), root);
    assert_eq!(note.canonicalize().unwrap(), note);
    let meta = fs::symlink_metadata(&note).unwrap();
    assert!(meta.is_file() && !meta.file_type().is_symlink() && meta.nlink() == 1 && meta.uid() == unsafe { libc::getuid() });
    let original = fs::read(&note).unwrap();
    let task_seed = b"---\ntype: task\ncategory: general\ntask-id: task:fictional-status-fixture\nstatus: \"\xf0\x9f\x9f\xa3 backlog\"\n---\n# Fictional task\n\nOwned body.\n".as_slice();
    let note_seed = b"# Fictional note\n\nOwned body.\n".as_slice();
    let expected = match std::env::var("VIGGO_STATUS_FIXTURE_SEED").as_deref() {
        Ok("note") => note_seed,
        Ok("task") | Err(_) => task_seed,
        _ => panic!("unknown fixture seed"),
    };
    assert_eq!(Sha256::digest(&original), Sha256::digest(expected));
    assert!(!state.exists(), "use a fresh fixture");
    assert!(!capsule.join(format!("control/{stage}.ready")).exists());
    assert!(!capsule.join(format!("control/{stage}.release")).exists());
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(json!({
        "id":"task.status.todo", "title":"Fictional Work Status", "aliases":["workstatusfixture"],
        "kind":"script", "target":writer,
        "arguments":["{invocation-provenance}","task-active","todo","{uuid}",""], "timeout":60
    }));
    host.replace(&config, None);
    host.start();
    host.settled(|_| true);
    actions::show(&host, "workstatusfixture");
    actions::key(&host, 36, 0, "\r", false);
    let marker = capsule.join(format!("control/{stage}.ready"));
    let deadline = Instant::now() + Duration::from_secs(40);
    while !marker.is_file() {
        if Instant::now() >= deadline {
            let logs = host.rpc(&json!({"op":"logs","commandID":"task.status.todo"}));
            panic!("writer did not reach the {stage} barrier: {logs}");
        }
        thread::sleep(Duration::from_millis(20));
    }
    if stage == "before-grant" {
        assert_eq!(mode, "dismiss");
        actions::key(&host, 53, 0, "", false);
        host.wait(|s| s["scriptsRunning"] == 0);
        assert_eq!(Sha256::digest(fs::read(&note).unwrap()), Sha256::digest(&original));
        assert!(!state.exists(), "pre-admission dismissal must not create Work state");
        assert!(!state.join("pending.json").exists());
        assert!(!state.join("editor-request.json").exists());
        assert_eq!(fs::read_dir(capsule.join("router")).unwrap().filter(|entry| entry.as_ref().unwrap().path().extension().is_some_and(|ext| ext == "json")).count(), 0);
        let entries = host.rpc(&json!({"op":"logs","commandID":"task.status.todo"}));
        let finishes: Vec<_> = entries["result"].as_array().unwrap().iter().filter(|row| row["phase"] == "command.finish").collect();
        assert_eq!(finishes.len(), 1);
        assert_eq!(finishes[0]["success"], false);
        return;
    }
    let pending = state.join("pending.json");
    let plan: Value = serde_json::from_slice(&fs::read(&pending).unwrap()).unwrap();
    assert_eq!(plan["schema"], "viggo.work-edit.v1");
    assert_eq!(plan["path"], "00-inbox/20261126-status-fixture/20261126-status-fixture.md");
    let postimage_text = plan["updated"].as_str().expect("journal postimage");
    let scalar = |key: &str| postimage_text.lines().find_map(|line| line.strip_prefix(key))
        .map(|value| value.trim().trim_matches('"'));
    assert_eq!(scalar("type:"), Some("task"));
    assert_eq!(scalar("status:"), Some("🔴 to-do"));
    let bindings: Vec<_> = fs::read_dir(capsule.join("router")).unwrap().map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json")).collect();
    assert_eq!(bindings.len(), 1);
    let expected_id = if expected == note_seed {
        format!("task:{}", bindings[0].file_stem().unwrap().to_str().unwrap())
    } else {
        "task:fictional-status-fixture".to_owned()
    };
    assert_eq!(scalar("task-id:"), Some(expected_id.as_str()));
    assert_eq!(postimage_text.matches("Owned body.").count(), 1);
    assert_eq!(postimage_text.matches("[!activity-message|").count(), 1);
    assert!(postimage_text.contains(if expected == note_seed { "# Fictional note" } else { "# Fictional task" }));
    let postimage = postimage_text.as_bytes();
    let expected_before_stop = if stage == "journal" { Sha256::digest(&original) } else { Sha256::digest(postimage) };
    assert_eq!(Sha256::digest(fs::read(&note).unwrap()), expected_before_stop);
    if stage == "postimage" { assert!(state.join("editor-request.json").is_file()); }
    let receipt_name = plan["receipt"].as_str().unwrap();
    assert!(receipt_name.len() == 74 && receipt_name.is_ascii() && receipt_name.starts_with("edit-") && receipt_name.ends_with(".json")
        && receipt_name[5..69].bytes().all(|byte| byte.is_ascii_hexdigit()));
    let receipt = state.join(receipt_name);
    assert_eq!(receipt.is_file(), stage == "receipt");
    if mode == "dismiss" {
        assert_eq!(stage, "postimage");
        actions::key(&host, 53, 0, "", false);
        host.wait(|s| s["nativeUI"]["visible"] == false);
        fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
            .open(capsule.join(format!("control/{stage}.release"))).unwrap().write_all(b"release").unwrap();
        host.wait(|s| s["scriptsRunning"] == 0);
        assert!(receipt.is_file());
        assert!(!pending.exists());
        assert_eq!(Sha256::digest(fs::read(&note).unwrap()), Sha256::digest(postimage));
        assert_eq!(host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"], false);
        let entries = host.rpc(&json!({"op":"logs","commandID":"task.status.todo"}));
        let finishes: Vec<_> = entries["result"].as_array().unwrap().iter().filter(|row| row["phase"] == "command.finish").collect();
        assert_eq!(finishes.len(), 1);
        assert_eq!(finishes[0]["success"], true);
        return;
    }
    let pending_hash = Sha256::digest(fs::read(&pending).unwrap());
    let binding_hash = Sha256::digest(fs::read(&bindings[0]).unwrap());
    let starts_before = host.rpc(&json!({"op":"logs","commandID":"task.status.todo"}))["result"]
        .as_array().unwrap().iter().filter(|row| row["phase"] == "command.start").count();
    assert_eq!(starts_before, 1);
    let stopped = host.host.stop(false).expect("owned host must stop");
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(stopped.status.signal(), Some(libc::SIGKILL));
    let lock = fs::OpenOptions::new().read(true).write(true).open(state.join("lock")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 { break; }
        assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::WouldBlock);
        assert!(Instant::now() < deadline, "writer did not release its own lock");
        thread::sleep(Duration::from_millis(20));
    }
    drop(lock);
    host.start();
    host.settled(|s| s["scriptsRunning"] == 0);
    assert_eq!(Sha256::digest(fs::read(&note).unwrap()), expected_before_stop);
    assert!(pending.is_file(), "restarting the host must not replay the write");
    assert_eq!(Sha256::digest(fs::read(&pending).unwrap()), pending_hash);
    let after_bindings: Vec<_> = fs::read_dir(capsule.join("router")).unwrap().map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json")).collect();
    assert_eq!(after_bindings, bindings);
    assert_eq!(Sha256::digest(fs::read(&bindings[0]).unwrap()), binding_hash);
    let starts_after = host.rpc(&json!({"op":"logs","commandID":"task.status.todo"}))["result"]
        .as_array().unwrap().iter().filter(|row| row["phase"] == "command.start").count();
    assert_eq!(starts_after, starts_before, "host restarted the action");
    let drifted_hash = if mode == "drift" {
        let mut file = fs::OpenOptions::new().append(true).custom_flags(libc::O_NOFOLLOW).open(&note).unwrap();
        file.write_all(b"\nFictional concurrent edit.\n").unwrap();
        file.sync_all().unwrap();
        Some(Sha256::digest(fs::read(&note).unwrap()))
    } else { None };
    let mut recovery = Command::new(&writer);
    recovery.env_clear().env("PATH", "/usr/bin:/bin").env("TMPDIR", capsule.join("control")).current_dir(&capsule)
        .env("VIGGO_STATUS_FIXTURE_CAPSULE", &capsule)
        .env("VIGGO_STATUS_FIXTURE_ENGINE_SHA256", engine_sha)
        .env("VIGGO_STATUS_FIXTURE_FORMATTER_SHA256", formatter_sha)
        .arg("recover").arg(&root).stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = child_output::wait(recovery.spawn().unwrap(), Duration::from_secs(20));
    if let Some(drifted_hash) = drifted_hash {
        assert_eq!(output.status.code(), Some(1));
        let refusal: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(refusal["ok"], false);
        assert_eq!(refusal["error"], "Fixture editor preimage drift");
        assert_eq!(Sha256::digest(fs::read(&note).unwrap()), drifted_hash);
        assert_eq!(Sha256::digest(fs::read(&pending).unwrap()), pending_hash);
        assert!(!receipt.exists());
        return;
    }
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    if stage == "receipt" {
        assert_eq!(result["confirmed"], true, "{result}");
        assert_eq!(Sha256::digest(fs::read(&note).unwrap()), Sha256::digest(postimage));
        assert!(receipt.is_file());
    } else {
        assert_eq!(result["rolled_back"], true, "{result}");
        assert_eq!(Sha256::digest(fs::read(&note).unwrap()), Sha256::digest(&original));
        assert!(!receipt.exists());
    }
    assert!(!pending.exists());
    assert!(!state.join("editor-request.json").exists());
}

#[test]
#[ignore = "requires an explicitly prepared synthetic Work capsule and v2 capture fixture"]
fn real_work_capture_journal_recovery_fixture() {
    use sha2::{Digest, Sha256};
    use std::{io::{Read, Write}, os::{fd::AsRawFd, unix::{fs::{MetadataExt, OpenOptionsExt}, process::ExitStatusExt}}, path::PathBuf, process::{Command, Stdio}};
    #[cfg(target_os = "macos")]
    fn assert_live_writer_and_busy_journal_lock(pid: i32, host_pid: u32, writer: &Path, lock: &Path, seal: &Value) -> fs::File {
        use std::os::unix::ffi::OsStrExt;
        let process = |pid: i32| {
            assert!(pid > 1);
            let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
            assert_eq!(
                unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) },
                size,
                "fixture process {pid} must still be live",
            );
            let info = unsafe { info.assume_init() };
            assert_eq!(info.pbi_pid, pid as u32);
            assert_eq!(info.pbi_uid, unsafe { libc::getuid() });
            info
        };
        let first = process(pid);
        let helper = process(first.pbi_ppid as i32);
        assert_eq!(helper.pbi_ppid, host_pid, "fixture writer must descend from the owned host through its helper");
        let mut path = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let length = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
        assert!(length > 0);
        let end = path.iter().position(|&byte| byte == 0).unwrap();
        let expected = writer.canonicalize().unwrap();
        assert_eq!(&path[..end], expected.as_os_str().as_bytes());

        let file = fs::OpenOptions::new().read(true).write(true)
            .custom_flags(libc::O_NOFOLLOW_ANY | libc::O_NONBLOCK)
            .open(lock).unwrap();
        let metadata = file.metadata().unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.uid(), unsafe { libc::getuid() });
        assert_eq!(metadata.nlink(), 1);
        assert_eq!(seal["lock"], json!([metadata.dev(), metadata.ino()]));
        assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, -1);
        assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::WouldBlock);
        let still = process(pid);
        assert_eq!(
            (first.pbi_ppid, first.pbi_start_tvsec, first.pbi_start_tvusec),
            (still.pbi_ppid, still.pbi_start_tvsec, still.pbi_start_tvusec),
        );
        file
    }

    fn publish_release(capsule: &Path, point: &str) {
        let folder = capsule.join("control");
        let staged = folder.join(format!("capture-{point}.release-staged"));
        let release = folder.join(format!("capture-{point}.release"));
        assert!(!release.exists());
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged).unwrap();
        file.write_all(b"release").unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::rename(staged, release).unwrap();
        fs::File::open(folder).unwrap().sync_all().unwrap();
    }
    #[cfg(target_os = "macos")]
    fn reject_replaced_pending(
        mut recovery: Command,
        capsule: &Path,
        target: &Path,
        receipt_name: &str,
        original: &[u8],
    ) {
        use std::{
            ffi::OsString,
            fs::{File, Metadata, OpenOptions},
            io::{self, BufRead, BufReader},
            mem::MaybeUninit,
            net::Shutdown,
            os::{
                fd::FromRawFd,
                unix::{net::UnixStream, process::CommandExt},
            },
            process::Child,
        };

        fn identity(info: &Metadata) -> (u64, u64) {
            (info.dev(), info.ino())
        }
        fn directory(path: &Path) -> File {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW_ANY)
                .open(path)
                .unwrap();
            let info = file.metadata().unwrap();
            assert!(info.is_dir() && info.uid() == unsafe { libc::getuid() });
            assert_eq!(info.mode() & 0o7777, 0o700);
            assert_eq!(identity(&info), identity(&fs::symlink_metadata(path).unwrap()));
            file
        }
        fn roster(path: &Path) -> Vec<OsString> {
            let mut names = fs::read_dir(path)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            names.sort_unstable();
            names
        }
        #[derive(Debug, PartialEq, Eq)]
        struct Record {
            identity: (u64, u64),
            mode: u32,
            modified: (i64, i64),
            changed: (i64, i64),
            bytes: Vec<u8>,
        }
        fn record(path: &Path) -> Record {
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW_ANY | libc::O_NONBLOCK)
                .open(path)
                .unwrap();
            let info = file.metadata().unwrap();
            assert!(info.is_file() && info.uid() == unsafe { libc::getuid() });
            assert_eq!(info.mode() & 0o7077, 0);
            assert_eq!(info.nlink(), 1);
            let mut bytes = Vec::new();
            Read::by_ref(&mut file).take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes).unwrap();
            assert!(bytes.len() <= 4 * 1024 * 1024);
            assert_eq!(bytes.len() as u64, info.len());
            assert_eq!(identity(&info), identity(&fs::symlink_metadata(path).unwrap()));
            Record {
                identity: identity(&info),
                mode: info.mode(),
                modified: (info.mtime(), info.mtime_nsec()),
                changed: (info.ctime(), info.ctime_nsec()),
                bytes,
            }
        }
        struct Recovery(Option<Child>);
        impl Drop for Recovery {
            fn drop(&mut self) {
                if let Some(child) = &mut self.0 {
                    if matches!(child.try_wait(), Ok(None)) {
                        let _ = child.kill();
                    }
                    let _ = child.wait();
                }
            }
        }
        struct Stage<'a> {
            directory: &'a File,
            identity: (u64, u64),
            armed: bool,
        }
        const EVIDENCE_NAME: &std::ffi::CStr = c"original-pending.json";
        impl Drop for Stage<'_> {
            fn drop(&mut self) {
                if !self.armed {
                    return;
                }
                let fd = self.directory.as_raw_fd();
                let mut stat = MaybeUninit::<libc::stat>::uninit();
                if unsafe { libc::fstatat(fd, EVIDENCE_NAME.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } == 0 {
                    let stat = unsafe { stat.assume_init() };
                    if (stat.st_dev as u64, stat.st_ino) == self.identity {
                        unsafe { libc::unlinkat(fd, EVIDENCE_NAME.as_ptr(), 0) };
                    }
                }
            }
        }

        assert_eq!(unsafe { libc::getuid() }, unsafe { libc::geteuid() });
        let base = Path::new("/Users/viggomeesters/Dev/obsidian-test-vault/.tmp");
        assert_eq!(base.canonicalize().unwrap(), base);
        assert_eq!(capsule.parent(), Some(base));
        assert_eq!(capsule.canonicalize().unwrap(), capsule);
        let root = capsule.join("work");
        let state = root.join(".work-command-state");
        let inbox = root.join("00-inbox");
        let control = capsule.join("control");
        let pending = state.join("pending.json");
        let lock_path = state.join("lock");
        let capsule_fd = directory(capsule);
        let root_fd = directory(&root);
        let state_fd = directory(&state);
        let inbox_fd = directory(&inbox);
        let control_fd = directory(&control);
        assert_eq!(record(&capsule.join(".status-fixture")).bytes, b"synthetic-status-recovery-fixture-v1");
        assert!(target.starts_with(&inbox));
        let evidence = capsule.join(".pending-same-bytes-proof");
        let created = unsafe {
            libc::mkdirat(capsule_fd.as_raw_fd(), c".pending-same-bytes-proof".as_ptr(), 0o700)
        };
        assert_eq!(created, 0, "{}", io::Error::last_os_error());
        let evidence_fd = directory(&evidence);
        assert_eq!(evidence_fd.metadata().unwrap().dev(), state_fd.metadata().unwrap().dev());
        let preserved_path = evidence.join("original-pending.json");
        let rebind = || {
            for (path, held) in [
                (capsule, &capsule_fd),
                (root.as_path(), &root_fd),
                (state.as_path(), &state_fd),
                (inbox.as_path(), &inbox_fd),
                (control.as_path(), &control_fd),
                (evidence.as_path(), &evidence_fd),
            ] {
                assert_eq!(identity(&directory(path).metadata().unwrap()), identity(&held.metadata().unwrap()));
            }
        };
        let old = record(&pending);
        assert_eq!(old.bytes, original);
        let lock_record = record(&lock_path);
        let lock = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW_ANY).open(&lock_path).unwrap();
        assert_eq!(identity(&lock.metadata().unwrap()), lock_record.identity);
        let pending_sha256 = format!("{:x}", Sha256::digest(original));
        let seal = control.join("capture-journal.json");
        let stable_paths = [
            lock_path,
            state.join(format!("capture-admission-{receipt_name}")),
            seal.clone(),
            control.join("capture-invocation.json"),
            control.join("capture-journal.ready"),
            capsule.join(".status-binding.json"),
            capsule.join(".status-fixture"),
        ];
        let stable = stable_paths.each_ref().map(|path| record(path));
        assert_eq!(
            serde_json::from_slice::<Value>(&record(&seal).bytes).unwrap(),
            json!({"schema":"viggo.capture-fixture-journal.v1","sha256":pending_sha256,
                "state":identity(&state_fd.metadata().unwrap()),"lock":lock_record.identity,"pending":old.identity})
        );
        let rosters = || [&state, &control, &inbox].map(|path| roster(path));
        let before = rosters();
        let assert_unpublished = || {
            for path in [target.parent().unwrap().to_owned(), state.join("package"), state.join("capture.md"), state.join(receipt_name)] {
                assert_eq!(fs::symlink_metadata(path).unwrap_err().kind(), io::ErrorKind::NotFound);
            }
        };
        assert_unpublished();

        let evidence_dir_fd = evidence_fd.as_raw_fd();
        let raw = unsafe {
            libc::openat(evidence_dir_fd, EVIDENCE_NAME.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::c_int)
        };
        assert!(raw >= 0, "{}", io::Error::last_os_error());
        let mut file = unsafe { File::from_raw_fd(raw) };
        let mut staged = Stage { directory: &evidence_fd, identity: identity(&file.metadata().unwrap()), armed: true };
        file.write_all(original).unwrap();
        file.sync_all().unwrap();
        evidence_fd.sync_all().unwrap();
        capsule_fd.sync_all().unwrap();
        let staged_record = record(&preserved_path);
        assert_eq!(staged_record.identity, staged.identity);
        assert_ne!(staged_record.identity, old.identity);
        assert_eq!(staged_record.bytes, old.bytes);
        assert_eq!(staged_record.mode, old.mode);
        let evidence_roster = roster(&evidence);
        assert_eq!(evidence_roster, [OsString::from("original-pending.json")]);

        let (endpoint, mut peer) = UnixStream::pair().unwrap();
        let fd = endpoint.as_raw_fd();
        recovery.env("VIGGO_CAPTURE_FIXTURE_RECHECK_FD", fd.to_string())
            .env("VIGGO_CAPTURE_FIXTURE_RECHECK_PARENT_PID", std::process::id().to_string())
            .stdin(Stdio::null());
        // Make only the dedicated test endpoint inheritable in the owned child.
        unsafe {
            recovery.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = recovery.spawn().unwrap();
        let pid = child.id();
        let mut child = Recovery(Some(child));
        drop(endpoint);
        peer.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut ready = Vec::new();
        BufReader::new(&mut peer).take(1025).read_until(b'\n', &mut ready).unwrap();
        assert!(ready.len() <= 1024 && ready.last() == Some(&b'\n'));
        assert_eq!(
            serde_json::from_slice::<Value>(&ready).unwrap(),
            json!({"schema":"viggo.capture-fixture-recheck.v1","pid":pid,
                "capsule":identity(&capsule_fd.metadata().unwrap()),"state":identity(&state_fd.metadata().unwrap()),
                "lock":lock_record.identity,"pending":old.identity,"pending_sha256":pending_sha256})
        );
        rebind();
        assert_eq!(record(&pending), old);
        assert_eq!(rosters(), before);
        let lock_attempt = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(lock_attempt, -1, "recovery child must already own this exact lock");
        assert_eq!(io::Error::last_os_error().kind(), io::ErrorKind::WouldBlock);

        assert_eq!(record(&preserved_path), staged_record);
        let swapped = unsafe {
            libc::renameatx_np(
                evidence_dir_fd,
                EVIDENCE_NAME.as_ptr(),
                state_fd.as_raw_fd(),
                c"pending.json".as_ptr(),
                libc::RENAME_SWAP,
            )
        };
        assert_eq!(swapped, 0, "{}", io::Error::last_os_error());
        // The evidence path now holds the displaced inode. Preserve it even if verification fails.
        staged.armed = false;
        state_fd.sync_all().unwrap();
        evidence_fd.sync_all().unwrap();
        rebind();
        let replacement = record(&pending);
        assert_eq!(replacement.bytes, staged_record.bytes);
        assert_eq!(replacement.mode, staged_record.mode);
        assert_eq!(replacement.identity, staged.identity);
        let preserved = record(&preserved_path);
        assert_eq!(preserved.bytes, old.bytes);
        assert_eq!(preserved.mode, old.mode);
        assert_eq!(preserved.identity, old.identity);
        assert_eq!(rosters(), before);
        assert_eq!(roster(&evidence), evidence_roster);

        peer.write_all(b"A").unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
        let result = child_output::wait(child.0.take().unwrap(), Duration::from_secs(20));
        assert_eq!(result.status.code(), Some(1));
        assert_eq!(serde_json::from_slice::<Value>(&result.stdout).unwrap(),
            json!({"ok":false,"error":"Capture fixture journal seal changed"}));
        rebind();
        assert_eq!(record(&pending), replacement);
        assert_eq!(record(&preserved_path), preserved);
        assert_eq!(stable_paths.each_ref().map(|path| record(path)), stable);
        assert_eq!(rosters(), before);
        assert_eq!(roster(&evidence), evidence_roster);
        assert_unpublished();
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);
    }
    let capsule = PathBuf::from(std::env::var_os("VIGGO_CAPTURE_FIXTURE_CAPSULE").expect("prepared capsule"));
    let base = Path::new("/Users/viggomeesters/Dev/obsidian-test-vault/.tmp");
    assert_eq!(base.canonicalize().unwrap(), base);
    assert_eq!(capsule.parent(), Some(base));
    assert_eq!(capsule.canonicalize().unwrap(), capsule);
    let root = capsule.join("work");
    let state = root.join(".work-command-state");
    let stage = std::env::var("VIGGO_CAPTURE_FIXTURE_STAGE").unwrap();
    let kind = std::env::var("VIGGO_CAPTURE_FIXTURE_KIND").unwrap();
    let case = std::env::var("VIGGO_CAPTURE_FIXTURE_CASE").unwrap_or_else(|_| "host-loss".into());
    assert!(["before-grant", "journal"].contains(&stage.as_str()));
    assert!(["note", "task"].contains(&kind.as_str()));
    assert!(case == "host-loss" || (stage == "journal" && matches!(case.as_str(), "refuse-publication" | "pending-same-bytes")) || (stage == "before-grant" && case == "refuse-before-grant"));
    let host_loss = matches!(case.as_str(), "host-loss" | "pending-same-bytes");
    assert!(case != "pending-same-bytes" || cfg!(target_os = "macos"));
    const SIGNALS_SHA_ENV: &str = "VIGGO_CAPTURE_FIXTURE_SIGNALS_SHA256";
    let signals_sha = std::env::var_os(SIGNALS_SHA_ENV).map(|raw| {
        assert!(cfg!(target_os = "macos"), "prepared capture signals require macOS");
        let sha = raw.into_string().expect("capture signals SHA-256 must be Unicode");
        assert!(sha.len() == 64 && sha.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "capture signals SHA-256 must be 64 lowercase hex digits");
        sha
    });
    let engine = std::env::var("VIGGO_STATUS_FIXTURE_ENGINE_SHA256").unwrap();
    let formatter = std::env::var("VIGGO_STATUS_FIXTURE_FORMATTER_SHA256").unwrap();
    let source_path = PathBuf::from(std::env::var_os("VIGGO_CAPTURE_FIXTURE_WRITER").unwrap());
    let meta = fs::symlink_metadata(&source_path).unwrap();
    assert!(meta.is_file() && !meta.file_type().is_symlink() && meta.uid() == unsafe { libc::getuid() } && meta.nlink() == 1 && meta.len() < 128 * 1024 * 1024);
    assert_eq!(source_path.canonicalize().unwrap(), source_path);
    let mut source = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&source_path).unwrap();
    let opened = source.metadata().unwrap();
    assert_eq!((opened.dev(), opened.ino()), (meta.dev(), meta.ino()));
    let mut bytes = Vec::new();
    Read::by_ref(&mut source).take(128 * 1024 * 1024 + 1).read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() <= 128 * 1024 * 1024);
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), std::env::var("VIGGO_CAPTURE_FIXTURE_WRITER_SHA256").unwrap());
    let mut host = Bridge::new(Config::current(), false);
    for (key, value) in [("VIGGO_STATUS_FIXTURE_CAPSULE", capsule.as_os_str().to_owned()), ("VIGGO_STATUS_FIXTURE_ENGINE_SHA256", engine.clone().into()), ("VIGGO_STATUS_FIXTURE_FORMATTER_SHA256", formatter.clone().into()), ("VIGGO_STATUS_FIXTURE_CHECKPOINT", stage.clone().into())] { host.env.insert(key.into(), value); }
    if let Some(sha) = signals_sha.as_deref() {
        host.env.insert(SIGNALS_SHA_ENV.into(), sha.into());
    }
    let directory = host.base.join(".local/libexec");
    fs::create_dir_all(&directory).unwrap();
    let writer = directory.join("viggo-work-writer");
    fs::OpenOptions::new().write(true).create_new(true).mode(0o700).open(&writer).unwrap().write_all(&bytes).unwrap();
    fs::set_permissions(&writer, fs::Permissions::from_mode(0o700)).unwrap();
    let mut identity = Command::new(&writer);
    identity.env_clear().env("PATH", "/usr/bin:/bin").env("TMPDIR", capsule.join("control")).current_dir(&capsule)
        .env("VIGGO_STATUS_FIXTURE_CAPSULE", &capsule).env("VIGGO_STATUS_FIXTURE_ENGINE_SHA256", &engine)
        .env("VIGGO_STATUS_FIXTURE_FORMATTER_SHA256", &formatter).env("VIGGO_STATUS_FIXTURE_CHECKPOINT", &stage)
        .arg("--fixture-capture-identity-v1").stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(sha) = signals_sha.as_deref() {
        identity.env(SIGNALS_SHA_ENV, sha);
    }
    let identity = child_output::wait(identity.spawn().unwrap(), Duration::from_secs(10));
    assert!(identity.status.success());
    let mut expected_identity = json!({"ok":true,"schema":"viggo.work-capture-fixture.v1","capsule":capsule,"root":root});
    if let Some(sha) = signals_sha.as_deref() {
        let directory_identity = |path: &Path| {
            let metadata = fs::symlink_metadata(path).unwrap();
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
            assert_eq!(metadata.uid(), unsafe { libc::getuid() });
            assert_eq!(metadata.mode() & 0o7777, 0o700);
            [metadata.dev(), metadata.ino()]
        };
        expected_identity["signals"] = json!({"sha256":sha,
            "capsule_identity":directory_identity(&capsule),
            "directory_identity":directory_identity(&capsule.join("capture-signals"))});
    }
    assert_eq!(serde_json::from_slice::<Value>(&identity.stdout).unwrap(), expected_identity);
    assert!(!state.exists(), "fresh fixture required");
    let marker = capsule.join(format!("control/capture-{stage}.ready"));
    assert!(!marker.exists());
    assert!(!capsule.join(format!("control/capture-{stage}.release")).exists());
    assert!(!capsule.join(format!("control/capture-{stage}.release-staged")).exists());
    assert!(!capsule.join(format!("control/capture-{stage}.refused")).exists());
    assert!(!capsule.join("control/capture-invocation.json").exists());
    assert!(!capsule.join("control/capture-journal.json").exists());
    let id = if kind == "note" { "capture.quick-note" } else { "capture.quick-task" };
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(json!({"id":id,"title":"Fictional Capture","aliases":["capturefixture"],"kind":"script","target":writer,"arguments":["{invocation-provenance}","capture",root,kind,"{uuid}","{input}"],"inputPrompt":"Message","timeout":60}));
    host.replace(&config, None); host.start(); host.settled(|_| true);
    actions::show(&host, "capturefixture");
    actions::key(&host, 36, 0, "\r", false);
    actions::view(&host, |view| view["runInputInline"] == true);
    actions::key(&host, 36, actions::CMD, "\r", false);
    actions::view(&host, |view| view["formVisible"] == true);
    host.probe("run-input-type", json!({"text":"Fictional capture\n\nOwned body."}));
    actions::key(&host, 36, actions::CMD, "\r", false);
    let deadline = Instant::now() + Duration::from_secs(40);
    let ready_pid = loop {
        let pid = fs::OpenOptions::new().read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&marker).and_then(|file| {
                let mut bytes = Vec::new();
                file.take(33).read_to_end(&mut bytes)?;
                Ok(std::str::from_utf8(&bytes).ok().and_then(|text| text.parse::<i32>().ok())
                    .filter(|pid| *pid > 1 && bytes == pid.to_string().as_bytes()))
            }).unwrap_or(None);
        if let Some(pid) = pid { break pid; }
        if Instant::now() >= deadline {
            let logs = host.rpc(&json!({"op":"logs","commandID":id}));
            panic!("capture did not reach its barrier: {logs}");
        }
        thread::sleep(Duration::from_millis(20));
    };
    if stage == "before-grant" {
        if case == "refuse-before-grant" {
            publish_release(&capsule, "before-grant");
        } else {
            actions::key(&host, 53, 0, "", false);
        }
        host.wait(|s| s["scriptsRunning"] == 0);
        if case == "refuse-before-grant" {
            assert_eq!(fs::read(capsule.join("control/capture-before-grant.refused")).unwrap(), b"refused");
        }
        assert!(!state.exists());
        assert!(!capsule.join("control/capture-invocation.json").exists());
        return;
    }
    let pending = state.join("pending.json");
    let pending_bytes = fs::read(&pending).unwrap();
    let plan: Value = serde_json::from_slice(&pending_bytes).unwrap();
    assert_eq!(plan["schema"], "viggo.work-capture.v1");
    assert_eq!(plan["root"], json!(root));
    assert_eq!(plan["kind"], kind);
    assert_eq!(plan["input"], "Fictional capture\n\nOwned body.");
    let body = plan["body"].as_str().unwrap();
    assert_eq!(plan["post_hash"], format!("{:x}", Sha256::digest(body.as_bytes())));
    let binding: Value = serde_json::from_slice(&fs::read(capsule.join("control/capture-invocation.json")).unwrap()).unwrap();
    let capture_id = binding["id"].as_str().unwrap();
    let receipt = format!("{:x}.json", Sha256::digest(capture_id.as_bytes()));
    assert_eq!(plan["receipt"], receipt);
    assert_eq!(binding["kind"], kind);
    assert!(body.contains(&format!("type: {kind}\n")));
    assert!(body.contains(&format!("capture-id: \"{capture_id}\"")));
    assert!(body.contains("# Fictional capture"));
    assert_eq!(body.matches("Owned body.").count(), 1);
    assert_eq!(body.matches("[!activity-message|").count(), 1);
    let relative = plan["path"].as_str().unwrap();
    let pieces: Vec<_> = Path::new(relative).components().collect();
    assert!(matches!(pieces.as_slice(), [std::path::Component::Normal(a), std::path::Component::Normal(b), std::path::Component::Normal(c)] if *a == "00-inbox" && b.to_str().is_some_and(|s| s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') && c.to_str() == Some(format!("{s}.md").as_str()))));
    assert!(!root.join(relative).parent().unwrap().exists());
    assert!(!state.join("capture.md").exists());
    assert!(!state.join("package").exists());
    let pending_hash = Sha256::digest(&pending_bytes);
    let starts_before = host.rpc(&json!({"op":"logs","commandID":id}));
    let starts_before: Vec<_> = starts_before["result"].as_array().unwrap().iter().filter(|row| row["phase"] == "command.start").map(|row| row["traceID"].as_str().unwrap().to_owned()).collect();
    assert_eq!(starts_before.len(), 1);
    #[cfg(target_os = "macos")]
    let lock = {
        let seal: Value = serde_json::from_slice(&fs::read(capsule.join("control/capture-journal.json")).unwrap()).unwrap();
        assert_eq!(seal["sha256"], format!("{pending_hash:x}"));
        for (name, path) in [("state", &state), ("pending", &pending)] {
            let metadata = fs::symlink_metadata(path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            assert_eq!(seal[name], json!([metadata.dev(), metadata.ino()]));
        }
        assert_live_writer_and_busy_journal_lock(
            ready_pid,
            host.host.process.as_ref().unwrap().id(),
            &writer,
            &state.join("lock"),
            &seal,
        )
    };
    #[cfg(not(target_os = "macos"))]
    let _ = ready_pid;
    if host_loss {
        assert_eq!(host.host.stop(false).unwrap().status.signal(), Some(libc::SIGKILL));
    } else {
        publish_release(&capsule, "journal");
        host.wait(|s| s["scriptsRunning"] == 0);
        assert_eq!(fs::read(capsule.join("control/capture-journal.refused")).unwrap(), b"refused");
        assert_eq!(Sha256::digest(fs::read(&pending).unwrap()), pending_hash);
        assert!(!state.join("capture.md").exists());
        assert!(!root.join(relative).exists());
        let entries = host.rpc(&json!({"op":"logs","commandID":id}));
        let finishes: Vec<_> = entries["result"].as_array().unwrap().iter().filter(|row| row["phase"] == "command.finish").collect();
        assert_eq!(finishes.len(), 1);
        assert_eq!(finishes[0]["success"], false);
    }
    #[cfg(not(target_os = "macos"))]
    let lock = fs::OpenOptions::new().read(true).write(true).open(state.join("lock")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 { break; }
        assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::WouldBlock);
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
    drop(lock);
    if host_loss { host.start(); host.settled(|s| s["scriptsRunning"] == 0); }
    assert_eq!(Sha256::digest(fs::read(&pending).unwrap()), pending_hash);
    let starts_after = host.rpc(&json!({"op":"logs","commandID":id}));
    let starts_after: Vec<_> = starts_after["result"].as_array().unwrap().iter().filter(|row| row["phase"] == "command.start").map(|row| row["traceID"].as_str().unwrap().to_owned()).collect();
    assert_eq!(starts_after, starts_before, "host must not replay the capture");
    let mut recovery = Command::new(&writer);
    recovery.env_clear().env("PATH", "/usr/bin:/bin").env("TMPDIR", capsule.join("control")).current_dir(&capsule)
        .env("VIGGO_STATUS_FIXTURE_CAPSULE", &capsule).env("VIGGO_STATUS_FIXTURE_ENGINE_SHA256", engine)
        .env("VIGGO_STATUS_FIXTURE_FORMATTER_SHA256", formatter).arg("recover").arg(&root)
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(sha) = signals_sha.as_deref() {
        recovery.env(SIGNALS_SHA_ENV, sha);
    }
    #[cfg(target_os = "macos")]
    if case == "pending-same-bytes" {
        let mut pinned = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW_ANY | libc::O_NONBLOCK).open(&writer).unwrap();
        let info = pinned.metadata().unwrap();
        assert!(info.is_file() && info.nlink() == 1 && info.uid() == unsafe { libc::getuid() });
        assert_eq!(info.mode() & 0o7777, 0o700);
        let mut candidate = Vec::new();
        Read::by_ref(&mut pinned).take(128 * 1024 * 1024 + 1).read_to_end(&mut candidate).unwrap();
        assert_eq!(candidate, bytes, "the gated recovery must use the original pinned fixture binary");
        reject_replaced_pending(recovery, &capsule, &root.join(relative), &receipt, &pending_bytes);
        return;
    }
    let result = child_output::wait(recovery.spawn().unwrap(), Duration::from_secs(20));
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let recovered: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(recovered["rolled_back"], true);
    assert!(!pending.exists());
    assert!(!root.join(relative).exists());
    assert!(!state.join("capture.md").exists());
    assert!(!state.join("package").exists());
    assert!(!state.join(receipt).exists());
}

fn work_status_host(mode: &str) -> Bridge {
    let mut host = Bridge::new(Config::current(), false);
    fs::write(host.base.join("owned-work-writer-fixture"), b"fixture").unwrap();
    if mode == "legacy" { fs::write(host.base.join("legacy-writer"), b"fixture").unwrap(); }
    let dir = host.base.join(".local/libexec");
    fs::create_dir_all(&dir).unwrap();
    let writer = dir.join("viggo-work-writer");
    fs::copy(env!("CARGO_BIN_EXE_work-failure-fixture"), &writer).unwrap();
    fs::set_permissions(&writer, fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(json!({
        "id":"task.status.todo", "title":"Owned Work Status", "aliases":["workstatusfixture"],
        "kind":"script", "target":writer, "arguments":["task-active","todo","{uuid}",mode], "timeout":3
    }));
    host.replace(&config, None);
    host.start();
    host.settled(|_| true);
    actions::show(&host, "workstatusfixture");
    host
}
fn capture_work_host(kind: &str, mode: &str) -> Bridge {
    let mut host = Bridge::new(Config::current(), false);
    fs::write(host.base.join("owned-work-writer-fixture"), b"fixture").unwrap();
    fs::write(host.base.join("fixture-mode"), mode).unwrap();
    fs::create_dir(host.base.join("fictional-work")).unwrap();
    if mode == "legacy" { fs::write(host.base.join("legacy-writer"), b"fixture").unwrap(); }
    let dir = host.base.join(".local/libexec"); fs::create_dir_all(&dir).unwrap();
    let writer = dir.join("viggo-work-writer");
    fs::copy(env!("CARGO_BIN_EXE_work-failure-fixture"), &writer).unwrap();
    fs::set_permissions(&writer, fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = default_config();
    let id = if kind == "note" { "capture.quick-note" } else { "capture.quick-task" };
    config["commands"].as_array_mut().unwrap().push(json!({"id":id,"title":"Owned Capture","aliases":["capturefixture"],"kind":"script","target":writer,"arguments":["capture",host.base.join("fictional-work"),kind,"{uuid}","{input}"],"inputPrompt":"Message","timeout":3}));
    host.replace(&config, None); host.start(); host.settled(|_| true);
    actions::show(&host, "capturefixture");
    actions::key(&host, 36, 0, "\r", false);
    actions::view(&host, |view| view["runInputInline"] == true);
    actions::key(&host, 36, actions::CMD, "\r", false);
    actions::view(&host, |view| view["formVisible"] == true);
    host.probe("run-input-type", json!({"text":"fictional"}));
    actions::key(&host, 36, actions::CMD, "\r", false);
    host
}

#[test]
fn v2_capture_cancel_before_grant_and_finish_after_grant() {
    for kind in ["note", "task"] {
        let host = capture_work_host(kind, "before");
        wait_marker(&host.base.join("before-grant"), "ready");
        eprintln!("{kind}: before grant reached");
        actions::key(&host, 53, 0, "", false);
        host.wait(|s| s["scriptsRunning"] == 0);
        eprintln!("{kind}: before grant cancelled");
        assert!(!host.base.join("postimage").exists());
        assert!(!host.base.join("receipt").exists());
        drop(host);
        let host = capture_work_host(kind, "after");
        wait_marker(&host.base.join("postimage"), "written");
        eprintln!("{kind}: after grant reached");
        actions::key(&host, 53, 0, "", false);
        actions::key(&host, 53, 0, "", false);
        host.wait(|s| s["nativeUI"]["visible"] == false);
        eprintln!("{kind}: dismissed");
        fs::write(host.base.join("release-after"), b"go").unwrap();
        host.wait(|s| s["scriptsRunning"] == 0);
        eprintln!("{kind}: completed");
        wait_marker(&host.base.join("receipt"), "committed");
        assert_eq!(host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"], false);
    }
}
#[test]
fn v2_capture_reopen_revokes_pending_grant_and_cannot_retarget_admitted_result() {
    let host = capture_work_host("note", "before");
    wait_marker(&host.base.join("before-grant"), "ready");
    actions::show(&host, "capturefixture");
    fs::write(host.base.join("release-before"), b"go").unwrap();
    host.wait(|s| s["scriptsRunning"] == 0);
    assert!(!host.base.join("postimage").exists());
    drop(host);
    let host = capture_work_host("note", "after");
    wait_marker(&host.base.join("postimage"), "written");
    actions::show(&host, "capturefixture");
    fs::write(host.base.join("release-after"), b"go").unwrap();
    host.wait(|s| s["scriptsRunning"] == 0);
    wait_marker(&host.base.join("receipt"), "committed");
    assert_eq!(actions::probe(&host, "read")["input"], "capturefixture");
    assert_eq!(fs::read_to_string(host.base.join("capture-count")).unwrap(), "1");
}

#[test]
fn v2_capture_unknown_outcome_retires_draft_without_replay() {
    for mode in ["unknown", "incomplete"] {
    let host = capture_work_host("note", mode);
    wait_marker(&host.base.join("postimage"), "written");
    fs::write(host.base.join("release-after"), b"go").unwrap();
    host.wait(|s| s["scriptsRunning"] == 0);
    let screen = actions::view(&host, |v| v["formVisible"] == false);
    assert!(screen.to_string().contains("may already be in Work"), "missing uncertain outcome notice: {screen}");
    assert!(!screen.to_string().contains("fictional"));
    actions::key(&host, 36, actions::CMD, "\r", false);
    assert_eq!(fs::read_to_string(host.base.join("capture-count")).unwrap(), "1");
    let logs = host.rpc(&json!({"op":"logs","commandID":"capture.quick-note","errorsOnly":true}));
    assert!(!logs.to_string().contains("fictional"));
    }
}
#[test]
fn v2_capture_rejects_missing_grant_and_unsupported_writer() {
    for mode in ["legacy", "silent", "decline", "pre-error", "pre-private"] {
        let host = capture_work_host("task", mode);
        host.wait(|s| s["scriptsRunning"] == 0);
        assert!(!host.base.join("postimage").exists());
        assert!(!host.base.join("receipt").exists());
        let screen = actions::view(&host, |view| view["formVisible"] == true);
        if mode == "pre-error" { assert!(screen.to_string().contains("gecontroleerde Work-root")); }
        if mode == "pre-private" {
            assert!(!screen.to_string().contains("SECRET_PAYLOAD"));
            let logs = host.rpc(&json!({"op":"logs","commandID":"capture.quick-task","errorsOnly":true}));
            assert!(!logs.to_string().contains("SECRET_PAYLOAD"));
        }
    }
}

#[test]
fn work_status_cancel_before_admission_does_not_write() {
    let host = work_status_host("before");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&host.base.join("before-grant"), "ready");
    actions::key(&host, 53, 0, "", false);
    host.wait(|s| s["scriptsRunning"] == 0);
    assert!(!host.base.join("postimage").exists());
    assert!(!host.base.join("receipt").exists());
}

#[test]
fn work_status_finishes_after_admission_without_reopening_stale_ui() {
    let host = work_status_host("after");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&host.base.join("postimage"), "written");
    actions::key(&host, 53, 0, "", false);
    host.wait(|s| s["nativeUI"]["visible"] == false);
    actions::show(&host, "workstatusfixture");
    fs::write(host.base.join("release-after"), b"go").unwrap();
    host.wait(|s| s["scriptsRunning"] == 0);
    wait_marker(&host.base.join("receipt"), "committed");
    assert_eq!(host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"], true);
    let logs = host.rpc(&json!({"op":"logs","commandID":"task.status.todo"}));
    let entries = logs["result"].as_array().unwrap();
    assert_eq!(entries.iter().filter(|e| e["phase"] == "command.finish").count(), 1);
    assert_eq!(entries.iter().find(|e| e["phase"] == "command.finish").unwrap()["success"], true);
    let performance = host.rpc(&json!({"op":"performance"}));
    assert_eq!(performance["result"]["stages"]["script.writerCapability"]["count"], 1);
}

#[test]
fn private_status_keeps_ordinary_cancellation() {
    let host = work_status_host("private");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&host.base.join("private-started"), "ready");
    actions::key(&host, 53, 0, "", false);
    host.wait(|s| s["scriptsRunning"] == 0);
    assert!(!host.base.join("receipt").exists());
}
#[test]
fn private_status_decline_keeps_success_behavior() {
    let host = work_status_host("private-success");
    actions::key(&host, 36, 0, "\r", false);
    host.wait(|s| s["scriptsRunning"] == 0);
    let logs = host.rpc(&json!({"op":"logs","commandID":"task.status.todo"}));
    assert_eq!(logs["result"][0]["success"], true);
}
#[test]
fn older_writer_falls_back_to_existing_cancellation_without_sentinel() {
    let host = work_status_host("legacy");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&host.base.join("legacy-started"), "ready");
    actions::key(&host, 53, 0, "", false);
    host.wait(|s| s["scriptsRunning"] == 0);
}

#[test]
fn granted_work_status_is_still_stopped_by_host_shutdown_or_death() {
    for graceful in [true, false] {
        let mut host = work_status_host("after");
        actions::key(&host, 36, 0, "\r", false);
        wait_marker(&host.base.join("postimage"), "written");
        let pid: i32 = fs::read_to_string(host.base.join("writer-pid")).unwrap().parse().unwrap();
        host.host.stop(graceful);
        let deadline = Instant::now() + Duration::from_secs(5);
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(Instant::now() < deadline, "writer survived host stop");
            thread::sleep(Duration::from_millis(10));
        }
        assert_dead(pid);
        assert!(!host.base.join("receipt").exists());
    }
}
#[test]
fn granted_work_status_with_invalid_result_is_never_reported_successful() {
    let host = work_status_host("malformed");
    actions::key(&host, 36, 0, "\r", false);
    host.wait(|s| s["scriptsRunning"] == 0);
    let logs = host.rpc(&json!({"op":"logs","commandID":"task.status.todo","errorsOnly":true}));
    let entries = logs["result"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["success"], false);
    assert!(entries[0]["message"].as_str().unwrap().contains("outcome is unknown"));
    assert!(entries[0].get("applied").is_none());
}
#[test]
fn capable_writer_without_handshake_cannot_report_success() {
    let host = work_status_host("silent");
    actions::key(&host, 36, 0, "\r", false);
    host.wait(|s| s["scriptsRunning"] == 0);
    let logs = host.rpc(&json!({"op":"logs","commandID":"task.status.todo","errorsOnly":true}));
    let entry = &logs["result"][0];
    assert_eq!(entry["success"], false);
    assert!(entry["message"].as_str().unwrap().contains("outcome is unknown"));
}
#[test]
fn granted_work_status_timeout_does_not_claim_a_receipt() {
    let host = work_status_host("after");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&host.base.join("postimage"), "written");
    host.wait(|s| s["scriptsRunning"] == 0);
    assert!(!host.base.join("receipt").exists());
    let logs = host.rpc(&json!({"op":"logs","commandID":"task.status.todo","errorsOnly":true}));
    let entry = &logs["result"][0];
    assert_eq!(entry["success"], false);
    assert!(entry["message"].as_str().unwrap().contains("outcome is unknown"));
    assert!(entry.get("applied").is_none());
}

fn host(marker: &str) -> Bridge {
    configured_host(marker, false)
}
fn configured_host(marker: &str, delay: bool) -> Bridge {
    let mut host = Bridge::new(Config::current(), false);
    let mut config = default_config();
    let args = if delay {
        json!(["execution-delay", host.base.join(marker), "10000", ""])
    } else {
        json!(["execution-write", host.base.join(marker), marker])
    };
    config["commands"].as_array_mut().unwrap().push(json!({
        "id":"script.fixture", "title":"Execution Fixture", "aliases":["runfixture"],
        "kind":"script", "target":env!("CARGO_BIN_EXE_private-results-fixture"),
        "arguments":args, "timeout":3
    }));
    host.replace(&config, None);
    host.start();
    host.settled(|_| true);
    actions::show(&host, "runfixture");
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
    host
}
#[test]
fn owned_action_records_correlated_private_software_phases() {
    let secret = "owned-phase-secret";
    let host = configured_host(secret, false);
    let action = host.rpc(&json!({"op":"execute","id":"script.fixture"}));
    assert_eq!(action["ok"], true, "owned command: {action}");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !host.base.join(secret).exists() && Instant::now() < deadline { thread::sleep(Duration::from_millis(10)); }
    assert_eq!(fs::read_to_string(host.base.join(secret)).ok().as_deref(), Some(secret), "fixture logs: {}", host.rpc(&json!({"op":"logs", "commandID":"script.fixture"})));
    host.wait(|status| status["scriptsRunning"] == 0);
    let response = host.rpc(&json!({"op":"performance"}));
    let result = &response["result"];
    let runs = result["completedRuns"].as_array().unwrap();
    eprintln!("owned software runs: {runs:?}");
    let script = runs.iter().rev().find(|run| run["kind"] == "script").unwrap();
    assert_eq!(script["outcome"], "ownerProcessed");
    let points = script["pointsMS"].as_object().unwrap();
    for key in ["workerStart", "capabilityPhaseEnd", "helperSpawned", "helperReady", "dispatched", "responseReceived", "helperExited", "workerResult", "ownerHarvest", "ownerFinalizeStart"] {
        assert!(points.get(key).and_then(Value::as_f64).is_some(), "missing {key}: {script}");
    }
    assert!(!result.to_string().contains(secret));
    let reset = host.rpc(&json!({"op":"performance.reset"}));
    assert_eq!(reset["ok"], true);
    assert!(host.rpc(&json!({"op":"performance"}))["result"]["completedRuns"].as_array().unwrap().is_empty());
}
#[test]
#[ignore = "manual isolated software timing measurement"]
fn owned_script_timing_samples() {
    let mut host = Bridge::new(Config::current(), false);
    let mut config = default_config();
    for (id, args) in [
        ("fixture.quick", json!(["execution-write", host.base.join("quick"), "synthetic"])),
        ("fixture.delay", json!(["execution-delay", host.base.join("delayed"), "100", ""])),
    ] {
        config["commands"].as_array_mut().unwrap().push(json!({"id":id,"title":"Synthetic", "kind":"script", "target":env!("CARGO_BIN_EXE_private-results-fixture"), "arguments":args, "timeout":3}));
    }
    host.replace(&config, None);
    host.start();
    host.settled(|_| true);
    for id in ["fixture.quick", "fixture.delay"] {
        let mut total = Vec::new();
        let mut external = Vec::new();
        for _ in 0..20 {
            let response = host.rpc(&json!({"op":"execute","id":id}));
            assert_eq!(response["ok"], true);
            let trace = host.rpc(&json!({"op":"performance"}));
            let runs = trace["result"]["completedRuns"].as_array().unwrap();
            let run = runs.iter().rev().find(|run| run["kind"] == "script").unwrap();
            total.push(run["elapsedMS"].as_f64().unwrap());
            external.push(run["pointsMS"]["responseReceived"].as_f64().unwrap() - run["pointsMS"]["dispatched"].as_f64().unwrap());
        }
        total.sort_by(f64::total_cmp);
        external.sort_by(f64::total_cmp);
        eprintln!("synthetic={id} n=20 total_p50_ms={:.2} total_p95_ms={:.2} response_p50_ms={:.2} response_p95_ms={:.2}", total[9], total[18], external[9], external[18]);
    }
}
struct GracefulBridge(Bridge);
impl std::ops::Deref for GracefulBridge {
    type Target = Bridge;
    fn deref(&self) -> &Bridge {
        &self.0
    }
}
impl std::ops::DerefMut for GracefulBridge {
    fn deref_mut(&mut self) -> &mut Bridge {
        &mut self.0
    }
}
impl Drop for GracefulBridge {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.0.host.stop(true)));
        }
    }
}
fn running_script() -> (GracefulBridge, i32) {
    let host = GracefulBridge(configured_host("pid", true));
    actions::key(&host, 36, 0, "\r", false);
    let deadline = Instant::now() + Duration::from_secs(3);
    let pid = loop {
        if let Ok(text) = fs::read_to_string(host.base.join("pid")) {
            if let Ok(pid) = text.parse::<i32>() {
                break pid;
            }
        }
        assert!(Instant::now() < deadline, "fixture did not publish its PID");
        thread::sleep(Duration::from_millis(10));
    };
    (host, pid)
}
fn assert_dead(pid: i32) {
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
#[test]
fn window_close_cancels_running_script() {
    let (host, pid) = running_script();
    actions::probe(&host, "window-close");
    host.wait(|s| s["scriptsRunning"] == 0 && s["nativeUI"]["visible"] == false);
    assert_dead(pid);
}
#[test]
fn escape_cancels_running_script() {
    let (host, pid) = running_script();
    actions::key(&host, 53, 1 << 23, "", false);
    host.wait(|s| s["scriptsRunning"] == 0 && s["nativeUI"]["visible"] == false);
    assert_dead(pid);
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn composition_keeps_escape_until_committed() {
    let (host, pid) = running_script();
    assert_eq!(
        actions::probe(&host, "execution-ime-escape")["imeEscapeIgnored"],
        true
    );
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["scriptsRunning"],
        1
    );
    actions::key(&host, 53, 0, "", false);
    host.wait(|s| s["scriptsRunning"] == 0);
    assert_dead(pid);
}
fn wait_marker(path: &Path, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if fs::read_to_string(path).as_deref().ok() == Some(expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "fixture marker missing or incorrect"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn return_runs_script_and_records_launcher_ranking() {
    let host = host("native");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&host.base.join("native"), "native");
    host.wait(|s| s["scriptsRunning"] == 0 && s["nativeUI"]["visible"] == false);
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        1
    );
    let ranking: Value =
        serde_json::from_slice(&fs::read(host.profile.join("search-ranking.json")).unwrap())
            .unwrap();
    assert!(ranking["entries"]
        .as_object()
        .unwrap()
        .contains_key("script.fixture"));
    assert_eq!(
        host.rpc(&json!({"op":"logs"}))["result"][0]["source"],
        "launcher"
    );
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
}
#[test]
fn only_real_launcher_success_writes_recency_without_argument_text() {
    let mut host = Bridge::new(Config::current(), false);
    let sentinel = format!(
        "PRIVATE_RECENCY_ARGUMENT_{}",
        host.base.file_name().unwrap().to_str().unwrap()
    );
    let marker = host.base.join("recency-success");
    let success_id = "recency.success";
    let failure_id = "recency.failure";
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().extend([
        json!({"id":success_id,"title":"Recency Success Fixture","aliases":["recencysuccess"],
            "kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),
            "arguments":["execution-write",marker,sentinel],"timeout":3}),
        json!({"id":failure_id,"title":"Recency Failure Fixture","aliases":["recencyfailure"],
            "kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),
            "arguments":["rich-special","exit",sentinel],"timeout":3}),
    ]);
    host.replace(&config, None);
    host.start();
    host.settled(|_| true);
    let reset = host.rpc(&json!({"op":"search-ranking.reset","confirmed":true}));
    assert_eq!(reset["ok"], true);
    assert_eq!(reset["result"]["entries"], 0);
    let ranking_path = host.profile.join("search-ranking.json");
    let empty_bytes = fs::read(&ranking_path).unwrap();
    let assert_empty = || {
        assert_eq!(fs::read(&ranking_path).unwrap(), empty_bytes);
        let ranking: Value = serde_json::from_slice(&empty_bytes).unwrap();
        assert_eq!(ranking["entries"], json!({}));
        assert_eq!(host.rpc(&json!({"op":"search-ranking"}))["result"]["entries"], 0);
    };
    assert_empty();

    let cli = host.rpc_timeout(&json!({"op":"execute","id":success_id}), Duration::from_secs(6));
    assert_eq!(cli["ok"], true);
    wait_marker(&marker, &sentinel);
    assert_empty();
    fs::remove_file(&marker).unwrap();

    // Dry-run is the public CLI control route; there is no native launcher
    // dry-run interface. Its returned command preserves the same argument.
    let dry_run = host.rpc(&json!({"op":"execute","id":success_id,"dryRun":true}));
    assert_eq!(dry_run["ok"], true);
    assert_eq!(dry_run["dryRun"], true);
    assert_eq!(dry_run["result"]["arguments"][2], sentinel);
    assert!(!marker.exists());
    assert_empty();

    actions::show(&host, "recencyfailure");
    actions::key(&host, 36, 0, "\r", false);
    host.wait(|status| {
        status["scriptsRunning"] == 0
            && host.rpc(&json!({"op":"logs","commandID":failure_id}))["result"]
                .as_array().unwrap().iter().any(|entry| {
                    entry["source"] == "launcher" && entry["success"] == false
                })
    });
    assert_empty();

    actions::show(&host, "recencysuccess");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&marker, &sentinel);
    host.wait(|status| {
        status["scriptsRunning"] == 0
            && status["nativeUI"]["visible"] == false
            && host.rpc(&json!({"op":"logs","commandID":success_id}))["result"]
                .as_array().unwrap().iter().any(|entry| {
                    entry["source"] == "launcher" && entry["success"] == true
                })
    });
    let bytes = fs::read(&ranking_path).unwrap();
    let ranking: Value = serde_json::from_slice(&bytes).unwrap();
    let entries = ranking["entries"].as_object().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries.get(success_id).unwrap().as_f64().unwrap().is_finite());
    assert_eq!(host.rpc(&json!({"op":"search-ranking"}))["result"]["entries"], 1);
    assert!(!String::from_utf8(bytes).unwrap().contains(&sentinel));
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
}
#[test]
fn ranked_command_is_filtered_after_disable_and_removal() {
    let mut host = Bridge::new(Config::current(), false);
    let suffix = host
        .base
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .replace('-', "");
    let command_id = format!("recency.filter.{suffix}");
    let alias = format!("rankedfilter{suffix}");
    let marker = host.base.join("recency-filter-executed");
    let sentinel = format!("PRIVATE_RECENCY_FILTER_ARGUMENT_{suffix}");
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(json!({
        "id":command_id,
        "title":"Recency Filter Fixture",
        "aliases":[alias],
        "kind":"script",
        "target":env!("CARGO_BIN_EXE_private-results-fixture"),
        "arguments":["execution-write",marker,sentinel],
        "timeout":3
    }));
    host.replace(&config, None);
    host.start();
    host.settled(|_| true);
    let reset = host.rpc(&json!({"op":"search-ranking.reset","confirmed":true}));
    assert_eq!(reset["ok"], true);
    assert_eq!(reset["result"]["entries"], 0);

    let search_has_command = |expected: bool| {
        let result = host.rpc(&json!({"op":"search","mode":"commands","query":alias}));
        assert_eq!(result["ok"], true, "{result}");
        let found = result["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == command_id);
        assert_eq!(found, expected, "{result}");
    };

    actions::show(&host, &alias);
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&marker, &sentinel);
    host.wait(|status| {
        status["scriptsRunning"] == 0 && status["nativeUI"]["visible"] == false
    });
    search_has_command(true);
    let ranking_path = host.profile.join("search-ranking.json");
    let ranking_bytes = fs::read(&ranking_path).unwrap();
    let ranking: Value = serde_json::from_slice(&ranking_bytes).unwrap();
    assert_eq!(ranking["entries"].as_object().unwrap().len(), 1);
    assert!(ranking["entries"].get(&command_id).is_some());

    let mut disabled = config.clone();
    disabled["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|command| command["id"] == command_id)
        .unwrap()["enabled"] = json!(false);
    host.replace(&disabled, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    search_has_command(false);
    assert_eq!(fs::read(&ranking_path).unwrap(), ranking_bytes);

    let mut removed = disabled;
    removed["commands"]
        .as_array_mut()
        .unwrap()
        .retain(|command| command["id"] != command_id);
    host.replace(&removed, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    search_has_command(false);
    assert_eq!(fs::read(&ranking_path).unwrap(), ranking_bytes);
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
}
#[test]
fn actions_open_runs_script() {
    let host = host("actions");
    actions::action(&host, "Run");
    wait_marker(&host.base.join("actions"), "actions");
    host.wait(|s| s["scriptsRunning"] == 0);
}
#[test]
fn click_runs_selected_script() {
    let host = host("clicked");
    host.probe("click", json!({"row":0}));
    wait_marker(&host.base.join("clicked"), "clicked");
    host.wait(|s| s["scriptsRunning"] == 0);
}
#[test]
fn query_change_keeps_new_results_visible_after_completion() {
    let host = GracefulBridge(host("native"));
    let ready = host.base.join("ready");
    let done = host.base.join("done");
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["id"] == "script.fixture")
        .unwrap();
    command["arguments"] = json!(["execution-query", ready, done]);
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    actions::show(&host, "runfixture");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&ready, "ready");
    assert_eq!(host.rpc(&json!({"op":"show","query":"finder"}))["ok"], true);
    host.settled(|s| s["nativeUI"]["selectedId"] == "app.finder");
    wait_marker(&done, "done");
    host.wait(|s| s["scriptsRunning"] == 0);
    let status = host.rpc(&json!({"op":"status"}));
    assert_eq!(status["result"]["nativeUI"]["visible"], true);
    assert_eq!(status["result"]["nativeUI"]["selectedId"], "app.finder");
    assert_eq!(actions::probe(&host, "read")["input"], "finder");
}
#[test]
fn stale_completion_is_rejected_after_newer_local_intent() {
    let host = host("native");
    actions::probe(&host, "reject-stale-execution-view");
    host.settled(|_| true);
    assert_eq!(actions::probe(&host, "read")["actionsVisible"], true);
}

#[test]
fn pending_local_intents_survive_ready_completion() {
    use std::io::{BufRead, Write};
    for mode in ["query", "menu", "editor", "selection"] {
        let mut bridge = Bridge::new(Config::current(), false);
        let ready = bridge.base.join("ready");
        let gate = bridge.base.join("gate");
        let marker = bridge.base.join("marker");
        let mut config = default_config();
        let commands = config["commands"].as_array_mut().unwrap();
        commands.push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","gated",ready,gate,marker,"completed"],"timeout":3}));
        commands.push(json!({"id":"script.other","title":"Execution Fixture Other","aliases":["otherfixture"],"kind":"script","target":"/usr/bin/true","arguments":[],"timeout":3}));
        bridge.replace(&config, None);
        bridge.start();
        let host = GracefulBridge(bridge);
        host.settled(|_| true);
        actions::show(
            &host,
            if mode == "selection" {
                "fixture"
            } else {
                "runfixture"
            },
        );
        assert_eq!(
            host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["selectedId"],
            "script.fixture"
        );
        actions::key(&host, 36, 0, "\r", false);
        wait_marker(&ready, "ready");
        host.probe("schedule-execution-intent", json!({"mode":mode}));
        let mut delay = host.connect(Duration::from_secs(5));
        delay
            .write_all(b"{\"op\":\"native.delay\",\"executionBarrier\":true}\n")
            .unwrap();
        let started = host.profile.join("owner-delay-started");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !started.exists() {
            assert!(Instant::now() < deadline, "owner delay did not start");
            thread::sleep(Duration::from_millis(10));
        }
        fs::write(&gate, "go").unwrap();
        let fired = host.profile.join("execution-intent-fired");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !fired.exists() {
            assert!(Instant::now() < deadline, "scheduled intent did not fire");
            thread::sleep(Duration::from_millis(10));
        }
        let mut line = String::new();
        std::io::BufReader::new(delay).read_line(&mut line).unwrap();
        let evidence: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(evidence["result"]["newIntentDuringDelay"], true);
        assert!(evidence["result"]["executionReady"].as_u64().unwrap() > 0);
        host.wait(|s| s["scriptsRunning"] == 0);
        let view = actions::probe(&host, "read");
        let status = host.rpc(&json!({"op":"status"}));
        assert_eq!(status["result"]["nativeUI"]["visible"], true);
        match mode {
            "query" => assert_eq!(view["input"], "finder"),
            "menu" => assert_eq!(view["actionsVisible"], true),
            "editor" => assert_eq!(view["formVisible"], true),
            "selection" => assert_eq!(status["result"]["nativeUI"]["selectedId"], "script.other"),
            _ => unreachable!(),
        }
    }
}

fn pending_cli(
    mode: Option<&str>,
) -> (
    GracefulBridge,
    std::path::PathBuf,
    std::os::unix::net::UnixStream,
    std::os::unix::net::UnixStream,
) {
    use std::io::Write;
    let mut bridge = Bridge::new(Config::current(), false);
    let ready = bridge.base.join("ready");
    let gate = bridge.base.join("gate");
    let marker = bridge.base.join("marker");
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","gated",ready,gate,marker,"completed"],"timeout":3}));
    bridge.replace(&config, None);
    bridge.start();
    let host = GracefulBridge(bridge);
    host.settled(|_| true);
    let mut execution = host.connect(Duration::from_secs(6));
    execution
        .write_all(b"{\"op\":\"execute\",\"id\":\"runfixture\"}\n")
        .unwrap();
    wait_marker(&ready, "ready");
    if let Some(mode) = mode {
        host.probe("schedule-execution-intent", json!({"mode":mode}));
    }
    let mut delay = host.connect(Duration::from_secs(5));
    delay
        .write_all(b"{\"op\":\"native.delay\",\"executionBarrier\":true}\n")
        .unwrap();
    wait_exists(&host.profile.join("owner-delay-started"));
    (host, gate, delay, execution)
}
fn wait_exists(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "fixture did not publish {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn response(stream: std::os::unix::net::UnixStream) -> Value {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut line)
        .unwrap();
    serde_json::from_str(&line).unwrap()
}
#[test]
fn cli_completion_delivered_and_recorded_before_native_stop() {
    let (mut host, gate, delay, execution) = pending_cli(Some("stop"));
    fs::write(gate, "go").unwrap();
    wait_exists(&host.profile.join("execution-intent-fired"));
    let evidence = response(delay);
    assert_eq!(evidence["result"]["nativeStopDuringDelay"], true);
    assert!(evidence["result"]["executionReady"].as_u64().unwrap() > 0);
    assert_eq!(response(execution)["ok"], true);
    let output = host.wait_exit(Duration::from_secs(5), false);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let logs: Value =
        serde_json::from_slice(&fs::read(host.profile.join("command-log.json")).unwrap()).unwrap();
    assert_eq!(
        logs["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["command.start", "command.finish"]
    );
    let usage: Value =
        serde_json::from_slice(&fs::read(host.profile.join("usage.json")).unwrap()).unwrap();
    assert_eq!(usage["script.fixture"], 1);
}
#[test]
fn external_replacement_cannot_receive_completed_command_usage() {
    let (host, gate, delay, execution) = pending_cli(None);
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    command["target"] = json!("/usr/bin/true");
    command["arguments"] = json!([]);
    host.replace(&config, None);
    fs::write(gate, "go").unwrap();
    assert!(
        response(delay)["result"]["executionReady"]
            .as_u64()
            .unwrap()
            > 0
    );
    let result = response(execution);
    assert_eq!(result["ok"], true);
    assert!(result.get("usageWarning").is_some());
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn ready_cli_completion_survives_queued_rpc_quit() {
    use std::io::Write;
    let (mut host, gate, delay, execution) = pending_cli(None);
    fs::write(gate, "go").unwrap();
    let mut quit = host.connect(Duration::from_secs(5));
    quit.write_all(b"{\"op\":\"quit\"}\n").unwrap();
    assert!(
        response(delay)["result"]["executionReady"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(response(execution)["ok"], true);
    assert_eq!(response(quit)["ok"], true);
    let output = host.wait_exit(Duration::from_secs(5), false);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let usage: Value =
        serde_json::from_slice(&fs::read(host.profile.join("usage.json")).unwrap()).unwrap();
    assert_eq!(usage["script.fixture"], 1);
}

#[test]
fn config_replacement_and_invalid_config_clear_running_label() {
    use std::io::Write;
    for invalid in [false, true] {
        let mut bridge = Bridge::new(Config::current(), false);
        let ready = bridge.base.join("ready");
        let gate = bridge.base.join("gate");
        let marker = bridge.base.join("marker");
        let mut config = default_config();
        config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","gated",ready,gate,marker,"completed"],"timeout":3}));
        config["commands"].as_array_mut().unwrap().push(json!({"id":"script.other","title":"Execution Fixture Other","aliases":["otherfixture"],"kind":"script","target":"/usr/bin/true","arguments":[],"timeout":3}));
        bridge.replace(&config, None);
        bridge.start();
        let host = GracefulBridge(bridge);
        host.settled(|_| true);
        actions::show(&host, "runfixture");
        assert_eq!(
            host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["selectedId"],
            "script.fixture"
        );
        actions::key(&host, 36, 0, "\r", false);
        wait_marker(&ready, "ready");
        let mut delay = host.connect(Duration::from_secs(5));
        delay
            .write_all(b"{\"op\":\"native.delay\",\"executionBarrier\":true}\n")
            .unwrap();
        wait_exists(&host.profile.join("owner-delay-started"));
        if invalid {
            host.replace_bytes(b"{invalid", None);
        } else {
            let command = config["commands"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|c| c["id"] == "script.fixture")
                .unwrap();
            command["target"] = json!("/usr/bin/true");
            command["arguments"] = json!([]);
            host.replace(&config, None);
        }
        fs::write(&gate, "go").unwrap();
        assert!(
            response(delay)["result"]["executionReady"]
                .as_u64()
                .unwrap()
                > 0
        );
        host.wait(|s| s["scriptsRunning"] == 0);
        let view = actions::probe(&host, "read");
        assert!(!view["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "Running…"));
        if invalid {
            assert!(view["screenText"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t.as_str().unwrap().starts_with("Config:")));
        }
        assert_eq!(
            host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
            true
        );
        assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
    }
}

#[test]
fn running_and_failure_copy_in_both_themes() {
    for theme in ["light", "dark"] {
        let mut bridge = Bridge::new(Config::current(), false);
        let gate = bridge.base.join("gate");
        let ready = bridge.base.join("ready");
        let marker = bridge.base.join("unused");
        let mut config = default_config();
        config["theme"] = json!(theme);
        config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","gated-fail",ready,gate,marker,""],"timeout":3}));
        bridge.replace(&config, None);
        bridge.start();
        let host = GracefulBridge(bridge);
        host.settled(|_| true);
        actions::show(&host, "runfixture");
        actions::key(&host, 36, 0, "\r", false);
        wait_marker(&ready, "ready");
        for state in ["running", "failure"] {
            if state == "failure" {
                fs::write(&gate, "go").unwrap();
                host.wait(|s| s["scriptsRunning"] == 0);
            }
            let view = host.probe("screenshot", json!({}));
            let texts = view["screenText"].as_array().unwrap();
            if state == "running" {
                assert!(texts.iter().any(|text| text == "Running…"));
            } else {
                assert!(texts
                    .iter()
                    .any(|text| text.as_str().unwrap().contains("status 7")));
            }
            if let Some(evidence) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
                let evidence = std::path::PathBuf::from(evidence);
                let folder = evidence.parent().unwrap().join("execution");
                fs::create_dir_all(&folder).unwrap();
                fs::copy(
                    host.profile.join("native-probe.png"),
                    folder.join(format!("{state}-{theme}.png")),
                )
                .unwrap();
                fs::write(
                    folder.join(format!("{state}-{theme}.json")),
                    serde_json::to_vec_pretty(&view).unwrap(),
                )
                .unwrap();
            }
            assert_eq!(view["windowShown"], false);
        }
    }
}
