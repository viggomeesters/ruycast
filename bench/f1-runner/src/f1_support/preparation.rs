//! Owned F1 build preparation and explicit guarded synthetic assembly. No installed-app launch.
use super::{json, sha, write_new, Binding, Path, PathBuf, Result, Value};
use std::{collections::BTreeMap, fs, process::Command, time::Instant};
const REFERENCE_CAPTURE_SECONDS: u64 = 1200;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub root: PathBuf,
    pub files: BTreeMap<PathBuf, String>,
}
fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, String>) -> Result<()> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let name = entry.file_name();
        if [
            ".git",
            ".go",
            ".build",
            ".dist",
            "target",
            "node_modules",
            "__pycache__",
            ".DS_Store",
        ]
        .iter()
        .any(|n| name == *n)
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("inventory refuses symlink input".into());
        }
        if metadata.is_dir() {
            walk(root, &path, files)?;
        } else if metadata.is_file() {
            files.insert(
                path.strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_path_buf(),
                sha(&fs::read(&path).map_err(|e| e.to_string())?),
            );
        } else {
            return Err("nonregular input in source inventory".into());
        }
    }
    Ok(())
}
pub(super) fn checkout_root() -> Result<PathBuf> {
    let root = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .map_err(|e| e.to_string())?;
    for member in ["bench/f1-runner/Cargo.toml", "rust-native/Cargo.toml", "rust/Cargo.toml"] {
        let path = root.join(member);
        if fs::canonicalize(&path).map_err(|e| e.to_string())? != path
            || !fs::symlink_metadata(path).map_err(|e| e.to_string())?.is_file()
        {
            return Err("source checkout layout differs".into());
        }
    }
    Ok(root)
}
pub fn inventory(root: &Path) -> Result<Inventory> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let checkout = checkout_root()?;
    if root != checkout {
        return Err("preparation only inventories the compiled checkout".into());
    }
    let mut files = BTreeMap::new();
    walk(&root, &root, &mut files)?;
    Ok(Inventory { root, files })
}
pub fn verify(binding: &Binding) -> Result<()> {
    binding.verify()?;
    let expected: Inventory = super::read(&binding.path)?;
    let actual = inventory(&expected.root)?;
    if actual.files != expected.files {
        return Err("source inventory additions/removals/bytes differ from frozen review".into());
    }
    Ok(())
}
fn file_binding(path: &Path) -> Result<Value> {
    Ok(json!({"path":path,"sha256":sha(&fs::read(path).map_err(|e|e.to_string())?)}))
}
const ORIGINAL_WORKLOADS_PATH: &str =
    "bench/reference/0.8.26/Sources/RuycastApp/PerformanceChecks.swift";
const ORIGINAL_WORKLOADS_SHA256: &str =
    "a0534165a6df3d6afda628abaefdc8f740b4b0a54a969f814af76e8860382950";
fn original_workloads_binding(measurement: &Path) -> Value {
    json!({
        "path": measurement.join(ORIGINAL_WORKLOADS_PATH),
        "sha256": ORIGINAL_WORKLOADS_SHA256,
    })
}
fn reference_capture_alive(deadline:Instant)->Result<()> {
    if Instant::now()>=deadline {Err("whole Reference capture absolute deadline".into())} else {Ok(())}
}
fn compiler_tool_deadline(now:Instant,whole:Option<Instant>)->Instant {
    let own=now+std::time::Duration::from_secs(900);
    whole.map_or(own,|end|end.min(own))
}

fn reference_guard_arguments(source: &Path, object: &Path, full_host: bool) -> Vec<std::ffi::OsString> {
    let mut arguments = vec![
        "--edition=2021".into(),
        "--crate-type=lib".into(),
        "--emit=obj".into(),
        "-O".into(),
        "-Cpanic=abort".into(),
        source.as_os_str().to_owned(),
        "-o".into(),
        object.as_os_str().to_owned(),
    ];
    if full_host {
        arguments.extend(["--cfg".into(), "feature=\"full-reference-host\"".into(), "-Cdebuginfo=2".into()]);
    }
    arguments
}
fn compile_guard(source: &Path, object: &Path, tmp: &Path, out: &Path, full_host: bool, compiler: Option<&Binding>,deadline:Option<Instant>) -> Result<()> {
    let arguments = reference_guard_arguments(source, object, full_host);
    let source_before = file_binding(source)?;
    if full_host && compiler.is_none() { return Err("full Reference guard needs the explicit before-bound Rust compiler".into()); }
    if let Some(compiler) = compiler { compiler.verify()?; }
    let mut command = Command::new(compiler.map_or_else(||Path::new("rustc"),|binding|binding.path.as_path()));
    command.args(&arguments).env("TMPDIR",tmp);
    let output = if full_host { native_tool_until(&mut command,&checkout_root()?,deadline.ok_or("whole Reference guard deadline absent")?)? }
        else {command.output().map_err(|e|e.to_string())?};
    if let Some(compiler) = compiler { compiler.verify()?; }
    if file_binding(source)? != source_before { return Err("Reference preentry source changed across compiler invocation".into()); }
    fs::write(out.join("preentry-rustc.stdout"), &output.stdout).map_err(|e| e.to_string())?;
    fs::write(out.join("preentry-rustc.stderr"), &output.stderr).map_err(|e| e.to_string())?;
    write_new(
        &out.join("preentry-object.json"),
        &json!({"source":source_before,"compiler":compiler,"arguments":arguments.iter().map(|a|a.to_string_lossy()).collect::<Vec<_>>(),"exit_code":output.status.code(),"object":if output.status.success(){file_binding(object)?}else{Value::Null},"scope":"admission constructor only, not complete SDK-selection evidence"}),
    )?;
    if !output.status.success() {
        return Err("Rust preentry constructor compilation failed".into());
    }
    Ok(())
}

pub fn preentry_test(out: &Path) -> Result<()> {
    fs::create_dir(out).map_err(|e| e.to_string())?;
    let tmp = out.join("compiler-tmp");
    fs::create_dir(&tmp).map_err(|e| e.to_string())?;
    let guard = Path::new(env!("CARGO_MANIFEST_DIR")).join("../reference/ReferencePreentry.rs");
    let object = out.join("ReferencePreentry.o");
    compile_guard(&guard, &object, &tmp, out, false,None,None)?;
    let source = out.join("fake_main.rs");
    fs::write(&source,r#"
#[cfg(not(no_ready))]
#[no_mangle]pub extern "C" fn ruycast_f1_preentry_ready_v1()->u64 {
 if std::env::var("FAKE_READY").ok().as_deref()==Some("yes"){0xF100_0026_0000_3FFF}else{0}
}
fn main(){std::fs::write(std::env::var_os("FAKE_MARKER").unwrap(),b"mock-effect-after-admission").unwrap();}
"#).map_err(|e|e.to_string())?;
    let mut cases = vec![];
    for no_ready in [false, true] {
        let executable = out.join(if no_ready {
            "fake_without_ready"
        } else {
            "fake_with_ready"
        });
        let mut compiler = Command::new("rustc");
        compiler
            .arg("--edition=2021")
            .arg(&source)
            .arg("-C")
            .arg(format!("link-arg={}", object.display()))
            .arg("-C")
            .arg("link-arg=-Wl,-export_dynamic")
            .arg("-o")
            .arg(&executable)
            .env("TMPDIR", &tmp);
        if no_ready {
            compiler.arg("--cfg").arg("no_ready");
        }
        let output = compiler.output().map_err(|e| e.to_string())?;
        fs::write(out.join(format!("fake-{no_ready}.stderr")), &output.stderr)
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("own Rust fake-main compilation failed".into());
        }
        for (name, env_ok, ready, expected) in if no_ready {
            vec![("missing-symbol", true, true, 87)]
        } else {
            vec![
                ("missing-env", false, true, 86),
                ("incomplete-ready", true, false, 88),
                ("complete-owned-fake-admission", true, true, 0),
            ]
        } {
            let marker = out.join(format!("{name}.marker"));
            let mut child = Command::new(&executable);
            child
                .env_clear()
                .env("HOME", out)
                .env("RUYCAST_HOME", out)
                .env("RUSTCAST_HOME", out)
                .env("FAKE_MARKER", &marker)
                .env("FAKE_READY", if ready { "yes" } else { "no" });
            if env_ok {
                child
                    .env("RUYCAST_F1_AUTHORITY_SHA256", "a".repeat(64))
                    .env("RUYCAST_F1_GUARD_SHA256", "b".repeat(64));
            }
            let output = child.output().map_err(|e| e.to_string())?;
            if output.status.code() != Some(expected) || marker.exists() != (expected == 0) {
                return Err("constructor failed to prevent fake effect before admission".into());
            }
            cases.push(json!({"name":name,"exit_code":output.status.code(),"marker_exists":marker.exists(),"executable":file_binding(&executable)?}));
        }
    }
    write_new(
        &out.join("result.json"),
        &json!({"scope":"own pure Rust fake-main only; no native/reference app/SDK object executed; magic tests admission only, never complete SDK selection","cases":cases,"guard_source":file_binding(&guard)?,"guard_object":file_binding(&object)?}),
    )
}

/// Private compile capability. Its backing is retained in this process through
/// compilation. An exported receipt can never recreate this pipe or authorize a
/// later process: EOF framing makes each cap a single-child/session resource.
struct NativeCompileCap {
    io: crate::host_io::HostIo,
    nonce: String,
    pin: Value,
}
impl NativeCompileCap {
    fn new() -> Result<Self> {
        use std::io::Read;
        let io = crate::host_io::HostIo::new().map_err(|e| e.to_string())?;
        let mut random = [0u8; 32];
        fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut random))
            .map_err(|e| e.to_string())?;
        let nonce = sha(&random);
        let pin = io
            .native_compile_pin(&sha(nonce.as_bytes()))
            .map_err(|e| e.to_string())?;
        Ok(Self { io, nonce, pin })
    }
    fn verify(&self) -> Result<()> {
        let actual = self
            .io
            .native_compile_pin(&sha(self.nonce.as_bytes()))
            .map_err(|e| e.to_string())?;
        if actual != self.pin {
            return Err("typed HostIo Native cap changed during build".into());
        }
        Ok(())
    }
}

fn host_compiler_target(root: &Path) -> Result<String> {
    let result = Command::new("rustc")
        .arg("-vV")
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err("cannot query current Rust host compiler".into());
    }
    let text = String::from_utf8(result.stdout).map_err(|e| e.to_string())?;
    let target = text
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("current compiler has no host target")?;
    if target.is_empty()
        || !target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("invalid compiler host target".into());
    }
    Ok(target.into())
}
fn space_for_build(path: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err("cannot check remaining build space".into());
    }
    let info = unsafe { info.assume_init() };
    if (info.f_bavail as u64).saturating_mul(info.f_frsize) < 15 * 1024 * 1024 * 1024 {
        return Err("build refused below the existing 15GiB free-space floor".into());
    }
    Ok(())
}
/// Concrete Native stage, without a loaded SDK. This command intentionally
/// cannot hand an expired compile cap to a later collect invocation. The paired
/// runner must retain its typed HostIo owner and build both images in that same
/// process before publishing the two final one-shot grant frames.
pub fn native_owned_host(root: &Path, out: &Path) -> Result<()> {
    let deadline=Instant::now()+std::time::Duration::from_secs(REFERENCE_CAPTURE_SECONDS);
    let cap = NativeCompileCap::new()?;
    native_stage_with_deadline(root,out,&cap,Some(deadline))
}
fn native_capture_alive(whole:Option<Instant>)->Result<()> {
    if let Some(deadline)=whole {reference_capture_alive(deadline)?;}
    Ok(())
}
// Compiler/tool processes only. Neither a product nor a generated SDK is run.
// Keep the owned leader unreaped until both bounded streams reach EOF, so
// timeout cleanup cannot retarget a reused process/group identity.
fn native_tool(command: &mut Command, root: &Path) -> Result<std::process::Output> {
    native_tool_with_deadline(command,root,None)
}
fn native_tool_until(command: &mut Command,root:&Path,deadline:Instant)->Result<std::process::Output> {
    native_tool_with_deadline(command,root,Some(deadline))
}
fn native_tool_with_deadline(command: &mut Command, root: &Path,whole:Option<Instant>) -> Result<std::process::Output> {
    native_tool_capture(command,root,whole,None)
}
// A file sink is used only by the bounded complete-disassembly collector.
// Both existing pipe budgets and the process/deadline cleanup remain shared.
fn native_tool_capture(command:&mut Command,root:&Path,whole:Option<Instant>,stdout_file:Option<&fs::File>)->Result<std::process::Output> {
    use std::{
        io::Read,
        os::{fd::AsRawFd, unix::process::CommandExt},
        process::Stdio,
    };
    if let Some(deadline)=whole { reference_capture_alive(deadline)?; }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("DYLD_") {
            command.env_remove(key);
        }
    }
    command
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(file)=stdout_file {
        command.stdout(Stdio::from(file.try_clone().map_err(|e|e.to_string())?));
    } else {command.stdout(Stdio::piped());}
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    let result = (|| {
        let mut stdout = child.stdout.take();
        if stdout_file.is_none()&&stdout.is_none(){return Err("compiler stdout absent".into());}
        let mut stderr = child.stderr.take().ok_or("compiler stderr absent")?;
        for fd in stdout.as_ref().map(AsRawFd::as_raw_fd).into_iter().chain(std::iter::once(stderr.as_raw_fd())) {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        let deadline = compiler_tool_deadline(Instant::now(),whole);
        let mut buffers = [Vec::new(), Vec::new()];
        let mut eof = [false, false];
        let mut empty=std::io::empty();
        let stdout:&mut dyn Read=match stdout.as_mut(){Some(pipe)=>pipe,None=>&mut empty};
        let mut last_space = Instant::now();
        while !eof.iter().all(|v| *v) {
            if Instant::now() >= deadline {
                return Err("compiler/tool absolute deadline".into());
            }
            if last_space.elapsed() >= std::time::Duration::from_secs(1) {
                space_for_build(root)?;
                last_space = Instant::now();
            }
            for (index, stream) in [&mut *stdout, &mut stderr as &mut dyn Read]
                .into_iter()
                .enumerate()
            {
                if eof[index] {
                    continue;
                }
                let mut chunk = [0_u8; 8192];
                match stream.read(&mut chunk) {
                    Ok(0) => eof[index] = true,
                    Ok(n) => {
                        if buffers[index].len() + n > 32 * 1024 * 1024 {
                            return Err("compiler/tool output exceeds32MiB per stream".into());
                        }
                        buffers[index].extend_from_slice(&chunk[..n]);
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // EOF alone is not exit; retain the same absolute deadline for wait.
        loop {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            if info.si_pid == pid as i32 {
                break;
            }
            if Instant::now() >= deadline {
                return Err("compiler/tool exit deadline".into());
            }
            space_for_build(root)?;
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok((
            std::mem::take(&mut buffers[0]),
            std::mem::take(&mut buffers[1]),
        ))
    })();
    if result.is_err() {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let (stdout, stderr) = result?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}
fn native_private_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
// Cargo's quoted display is parsed as data, never executed as shell text.
fn native_shell_words(line: &str) -> Result<Vec<String>> {
    if line.len() > 1024 * 1024 {
        return Err("compiler command display bound".into());
    }
    let mut words = vec![];
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut active = false;
    for c in line.chars() {
        if escaped {
            if quote == Some('"') && !matches!(c, '$' | '`' | '"' | '\\' | '\n') {
                word.push('\\');
            }
            if c != '\n' {
                word.push(c);
            }
            escaped = false;
            active = true;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            active = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None
            } else {
                word.push(c)
            }
            active = true;
            continue;
        }
        if matches!(c, '\'' | '"') {
            quote = Some(c);
            active = true
        } else if c.is_whitespace() {
            if active {
                words.push(std::mem::take(&mut word));
                active = false
            }
        } else {
            word.push(c);
            active = true
        }
    }
    if quote.is_some() || escaped {
        return Err("compiler command quote/escape truncated".into());
    }
    if active {
        words.push(word)
    }
    if words.len() > 32768 {
        return Err("compiler argument count bound".into());
    }
    Ok(words)
}
fn native_rustc_commands(stderr: &str) -> Result<Vec<Vec<String>>> {
    let mut commands = vec![];
    for line in stderr.lines() {
        let Some(body) = line
            .trim()
            .strip_prefix("Running `")
            .and_then(|s| s.strip_suffix('`'))
        else {
            continue;
        };
        let words = native_shell_words(body)?;
        let start = words.iter().position(|s| !s.contains('='));
        let Some(start) = start else { continue };
        if Path::new(&words[start]).file_name() != Some(std::ffi::OsStr::new("rustc")) {
            continue;
        }
        if !Path::new(&words[start]).is_absolute() {
            return Err("actual compiler path must be absolute".into());
        }
        // Never persist the preceding environment assignments, even for Cargo.
        commands.push(words[start..].to_vec());
    }
    if commands.is_empty() || commands.len() > 1024 {
        return Err("actual Cargo rustc invocation roster absent/bound".into());
    }
    Ok(commands)
}
fn native_uuid(bytes: &[u8]) -> Result<[u8; 16]> {
    let u32at = |at: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or("MachO UUID truncated")?
                .try_into()
                .map_err(|_| "UUID word")?,
        ))
    };
    if u32at(0)? != 0xfeedfacf {
        return Err("UUID image is not thin64 MachO".into());
    }
    let end = 32usize
        .checked_add(u32at(20)? as usize)
        .ok_or("UUID command overflow")?;
    if end > bytes.len() {
        return Err("UUID commands truncated".into());
    }
    let mut at = 32usize;
    let mut uuid = None;
    for _ in 0..u32at(16)? {
        let size = u32at(at + 4)? as usize;
        if size < 8 || at.checked_add(size).is_none_or(|p| p > end) {
            return Err("UUID load command bound".into());
        }
        if u32at(at)? == 0x1b {
            if size != 24 || uuid.is_some() {
                return Err("UUID duplicate/size".into());
            }
            uuid = Some(
                bytes[at + 8..at + 24]
                    .try_into()
                    .map_err(|_| "UUID bytes")?,
            )
        }
        at += size;
    }
    if at != end {
        return Err("UUID command extent differs".into());
    }
    uuid.ok_or("required MachO UUID absent".into())
}
fn native_objc_inputs(package: &Path) -> Result<BTreeMap<PathBuf, String>> {
    let mut files = BTreeMap::new();
    // objc2 includes example files as documentation via include_str!. They
    // are real rustc dep-info inputs even when no examples are executed.
    walk(package, package, &mut files)?;
    files.insert(
        PathBuf::from("Cargo.toml"),
        sha(&fs::read(package.join("Cargo.toml")).map_err(|e| e.to_string())?),
    );
    files.insert(
        PathBuf::from("build.rs"),
        sha(&fs::read(package.join("build.rs")).map_err(|e| e.to_string())?),
    );
    Ok(files
        .into_iter()
        .map(|(p, h)| (package.join(p), h))
        .collect())
}
// Compiler inputs have a separate fixed artifact bound from the narrower
// 2MiB runtime declaration reader. This function issues no runtime witness.
fn compiler_input_bytes(path:&Path,owner:u32,whole:Option<Instant>)->Result<(Binding,Vec<u8>)> {
    let (cap,bytes)=compiler_input_cap(path,owner,whole)?;
    Ok((cap.binding,bytes))
}
#[derive(serde::Serialize)]
struct CompilerInputCap { binding:Binding,#[serde(skip)] file:fs::File,identity:NativeTraceIdentity,owner:u32 }
fn compiler_input_cap(path:&Path,owner:u32,whole:Option<Instant>)->Result<(CompilerInputCap,Vec<u8>)> {
    use std::{io::Read,os::{fd::AsRawFd,unix::fs::{MetadataExt,OpenOptionsExt}}};
    native_capture_alive(whole)?;
    if !path.is_absolute() || fs::canonicalize(path).map_err(|e|e.to_string())?!=path {
        return Err("compiler input path alias".into());
    }
    let file=fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC).open(path).map_err(|e|e.to_string())?;
    let before=file.metadata().map_err(|e|e.to_string())?;
    let identity=|m:&fs::Metadata|(m.dev(),m.ino(),m.uid(),m.gid(),m.mode(),m.nlink(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
    let access=unsafe{libc::fcntl(file.as_raw_fd(),libc::F_GETFL)};
    let flags=unsafe{libc::fcntl(file.as_raw_fd(),libc::F_GETFD)};
    if !before.is_file() || before.uid()!=owner || before.nlink()==0 || before.mode()&0o022!=0
        || before.len()>512*1024*1024 || access<0 || flags<0
        || access&libc::O_ACCMODE!=libc::O_RDONLY || flags&libc::FD_CLOEXEC==0 {
        return Err("compiler input readcap ownership/access/bound".into());
    }
    let mut bytes=Vec::new();
    (&file).take(512*1024*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    let after=file.metadata().map_err(|e|e.to_string())?;
    let named=fs::symlink_metadata(path).map_err(|e|e.to_string())?;
    if bytes.len()>512*1024*1024 || bytes.len() as u64!=before.len()
        || identity(&before)!=identity(&after) || identity(&after)!=identity(&named) {
        return Err("compiler input identity changed".into());
    }
    let binding=Binding{path:path.to_owned(),sha256:sha(&bytes)};
    native_capture_alive(whole)?;
    Ok((CompilerInputCap{binding,file,identity:identity(&after),owner},bytes))
}
fn verify_compiler_input_cap(cap:&CompilerInputCap,whole:Option<Instant>)->Result<()> {
    use std::os::unix::fs::MetadataExt;
    let identity=|m:&fs::Metadata|(m.dev(),m.ino(),m.uid(),m.gid(),m.mode(),m.nlink(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
    native_capture_alive(whole)?;
    let held=cap.file.metadata().map_err(|e|e.to_string())?;
    let named=fs::symlink_metadata(&cap.binding.path).map_err(|e|e.to_string())?;
    let (actual,_)=compiler_input_cap(&cap.binding.path,cap.owner,whole)?;
    if identity(&held)!=cap.identity || identity(&named)!=cap.identity
        ||actual.identity!=cap.identity ||actual.binding.sha256!=cap.binding.sha256 {
        return Err("held compiler executable identity/bytes changed".into());
    }
    native_capture_alive(whole)
}
struct NativePackages {
    objc:(PathBuf,BTreeMap<PathBuf,String>),
    source_before:BTreeMap<PathBuf,String>,
    roots:BTreeMap<String,(PathBuf,u32)>,
}
type NativePackageBefore=(BTreeMap<PathBuf,String>,BTreeMap<String,(PathBuf,u32)>);
fn native_package_before(root:&Path,packages:&[Value],whole:Option<Instant>)->Result<NativePackageBefore> {
    use std::os::unix::fs::MetadataExt;
    fn collect(dir:&Path,owner:u32,files:&mut BTreeMap<PathBuf,String>,whole:Option<Instant>,visited:&mut usize,depth:usize)->Result<()> {
        native_capture_alive(whole)?;
        if depth>64{return Err("compiler package directory depth bound".into());}
        for entry in fs::read_dir(dir).map_err(|e|e.to_string())? {
            native_capture_alive(whole)?;
            if *visited>=100000{return Err("compiler package traversal entry bound".into());}
            *visited+=1;
            let entry=entry.map_err(|e|e.to_string())?;let path=entry.path();let name=entry.file_name();
            if [".git",".go",".build",".dist","target","node_modules","__pycache__",".DS_Store"]
                .iter().any(|ignored|name==*ignored){continue;}
            let metadata=fs::symlink_metadata(&path).map_err(|e|e.to_string())?;
            if metadata.file_type().is_symlink(){return Err("compiler package source symlink".into());}
            if metadata.is_dir(){
                if metadata.uid()!=owner || metadata.mode()&0o022!=0 {return Err("compiler package directory owner/mode".into());}
                collect(&path,owner,files,whole,visited,depth+1)?;
            }else if metadata.is_file(){
                if files.len()>=100000{return Err("compiler package source roster bound".into());}
                let (binding,_)=compiler_input_bytes(&path,owner,whole)?;
                if files.insert(binding.path,binding.sha256.clone()).is_some_and(|old|old!=binding.sha256) {
                    return Err("compiler package source aliases differ".into());
                }
            }else{return Err("compiler package source special file".into());}
        }
        Ok(())
    }
    if packages.is_empty() || packages.len()>1024{return Err("compiler package metadata bound".into());}
    let mut files=BTreeMap::new();let mut roots=BTreeMap::new();let mut visited=0;
    for package in packages {
        native_capture_alive(whole)?;
        let manifest=Path::new(package["manifest_path"].as_str().ok_or("compiler package manifest absent")?);
        if fs::canonicalize(manifest).map_err(|e|e.to_string())?!=manifest{return Err("compiler package manifest alias".into());}
        let directory=manifest.parent().ok_or("compiler package root absent")?;
        if !directory.starts_with(root) && !package["source"].as_str()
            .is_some_and(|source|source.starts_with("registry+")||source.starts_with("git+")) {
            return Err("unregistered external compiler package root".into());
        }
        let metadata=fs::symlink_metadata(directory).map_err(|e|e.to_string())?;
        if !metadata.is_dir() || ![0,unsafe{libc::getuid()}].contains(&metadata.uid())
            || metadata.mode()&0o022!=0 {return Err("compiler package root owner/mode".into());}
        let id=package["id"].as_str().ok_or("compiler package id absent")?;
        if roots.insert(id.to_owned(),(directory.to_owned(),metadata.uid())).is_some(){return Err("compiler package id duplicate".into());}
        collect(directory,metadata.uid(),&mut files,whole,&mut visited,0)?;
    }
    native_capture_alive(whole)?;
    Ok((files,roots))
}
fn native_objc_package(
    root: &Path,
    out: &Path,
    cargo: &Path,
    whole:Option<Instant>,
) -> Result<NativePackages> {
    native_capture_alive(whole)?;
    let mut tool = Command::new(cargo);
    tool.current_dir(root)
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(root.join("rust-native/Cargo.toml"));
    let result = native_tool_with_deadline(&mut tool,root,whole)?;
    if !result.status.success() {
        return Err("locked Cargo metadata failed before compile".into());
    }
    let value: Value =
        serde_json::from_slice(&result.stdout).map_err(|_| "invalid locked Cargo metadata")?;
    let (package_source_before,package_roots)=native_package_before(root,
        value["packages"].as_array().ok_or("Cargo package roster absent")?,whole)?;
    let packages = value["packages"]
        .as_array()
        .ok_or("Cargo package roster absent")?
        .iter()
        .filter(|p| p["name"] == "objc2")
        .collect::<Vec<_>>();
    if packages.len() != 1 {
        return Err("exact one locked objc2 package required".into());
    }
    let package = packages[0];
    let manifest = Path::new(
        package["manifest_path"]
            .as_str()
            .ok_or("objc2 manifest absent")?,
    );
    if fs::canonicalize(manifest).map_err(|e| e.to_string())? != manifest {
        return Err("objc2 manifest path must be canonical".into());
    }
    let directory = manifest
        .parent()
        .ok_or("objc2 package parent absent")?
        .to_owned();
    let inputs = native_objc_inputs(&directory)?;
    native_capture_alive(whole)?;
    for leaf in [
        "src/macros/define_class.rs",
        "src/__macro_helpers/defined_ivars.rs",
    ] {
        if !inputs.contains_key(&directory.join(leaf)) {
            return Err("required objc2 macro inputs missing".into());
        }
    }
    native_private_bytes(
        &out.join("objc2-source-before.json"),
        &serde_json::to_vec_pretty(&json!({"package":package,"inputs":inputs}))
            .map_err(|e| e.to_string())?,
    )?;
    Ok(NativePackages{objc:(directory,inputs),source_before:package_source_before,roots:package_roots})
}
fn native_argument<'a>(args: &'a [String], key: &str) -> Result<&'a str> {
    let matches = args
        .windows(2)
        .filter(|p| p[0] == key)
        .map(|p| p[1].as_str())
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!("actual rustc {key} missing/duplicate"));
    }
    Ok(matches[0])
}
fn native_toolchain(root: &Path, out: &Path,whole:Option<Instant>) -> Result<(PathBuf, Value)> {
    native_capture_alive(whole)?;
    let mut paths = BTreeMap::new();
    let mut records = BTreeMap::new();
    for name in ["rustc", "cargo"] {
        let mut command = Command::new("rustup");
        command.current_dir(root).args(["which", name]);
        let result = native_tool_with_deadline(&mut command,root,whole)?;
        if !result.status.success() || !result.stderr.is_empty() {
            return Err("exact compiler/toolchain resolution failed".into());
        }
        let text = std::str::from_utf8(&result.stdout)
            .map_err(|_| "toolchain path UTF8")?
            .trim();
        let path = fs::canonicalize(text).map_err(|e| e.to_string())?;
        if !path.is_absolute() || path.file_name() != Some(std::ffi::OsStr::new(name)) {
            return Err("resolved toolchain executable identity differs".into());
        }
        let binding = file_binding(&path)?;
        let mut command = Command::new(&path);
        command.arg(if name == "rustc" { "-vV" } else { "-V" });
        let version = native_tool_with_deadline(&mut command,root,whole)?;
        if !version.status.success() || !version.stderr.is_empty() {
            return Err("toolchain version capture failed".into());
        }
        records.insert(name,json!({"binary":binding,"version":std::str::from_utf8(&version.stdout).map_err(|_|"toolchain version UTF8")?}));
        paths.insert(name, path);
    }
    let value = json!(records);
    native_private_bytes(
        &out.join("native-toolchain.json"),
        &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )?;
    Ok((paths.remove("cargo").ok_or("cargo path missing")?, value))
}
fn native_cargo_summary(stdout: &[u8], stderr: &[u8]) -> Result<Value> {
    let mut records = vec![];
    for line in stdout.split(|b| *b == b'\n').filter(|v| !v.is_empty()) {
        let item: Value = serde_json::from_slice(line)
            .map_err(|_| "Cargo stdout was not declared JSON protocol")?;
        match item["reason"].as_str(){
            Some("compiler-artifact")=>records.push(json!({"reason":"compiler-artifact","package_id":item["package_id"],"target":item["target"]["name"],"filenames":item["filenames"],"fresh":item["fresh"]})),
            Some("compiler-message")=>records.push(json!({"reason":"compiler-message","level":item["message"]["level"],"code":item["message"]["code"]["code"]})),
            Some("build-finished")=>records.push(json!({"reason":"build-finished","success":item["success"]})),
            Some("build-script-executed")=>(), // Its env values are never persisted.
            _=>return Err("unexpected Cargo JSON reason".into()),
        }
    }
    // Cargo-vv stderr contains environment assignments. Only the separately
    // parsed rustc argv is retained; never write its raw command-display bytes.
    Ok(
        json!({"records":records,"stdout_bytes":stdout.len(),"stderr_bytes":stderr.len(),"environment_values_retained":false}),
    )
}
/// Declaration inputs are compiler inputs, not runtime-discovered authority.
/// This capture is consumed only by the same source-owned SDK build below.
struct NSStringCompileInputs {
    package: PathBuf,
    package_identity: Value,
    features: std::collections::BTreeSet<String>,
    source_before: BTreeMap<PathBuf, String>,
    sdk_root: PathBuf,
    header: Binding,
    owner: u32,
}
fn declaration_file(path: &Path, owner: u32) -> Result<Binding> {
    declaration_bytes(path,owner).map(|(binding,_)|binding)
}
fn declaration_bytes(path: &Path, owner: u32) -> Result<(Binding,Vec<u8>)> {
    use std::{io::Read, os::{fd::AsRawFd, unix::fs::{MetadataExt, OpenOptionsExt}}};
    if !path.is_absolute() || fs::canonicalize(path).map_err(|_| "declaration canonical path")? != path {
        return Err("declaration input path alias".into());
    }
    let file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path).map_err(|_| "declaration readcap open")?;
    let before = file.metadata().map_err(|_| "declaration fstat")?;
    let identity = |m: &fs::Metadata| (m.dev(),m.ino(),m.uid(),m.gid(),m.mode(),m.nlink(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
    let access=unsafe { libc::fcntl(file.as_raw_fd(),libc::F_GETFL) };
    let descriptor_flags=unsafe { libc::fcntl(file.as_raw_fd(),libc::F_GETFD) };
    if !before.is_file() || before.nlink() != 1 || before.uid() != owner
        || before.mode() & 0o022 != 0 || before.len() > 2 * 1024 * 1024
        || access<0 || descriptor_flags<0 || access & libc::O_ACCMODE != libc::O_RDONLY
        || descriptor_flags & libc::FD_CLOEXEC == 0
    { return Err("declaration readcap ownership/access".into()); }
    let mut bytes = Vec::new();
    (&file).take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| "declaration bounded read")?;
    let after = file.metadata().map_err(|_| "declaration refstat")?;
    let leaf = fs::symlink_metadata(path).map_err(|_| "declaration restat")?;
    if bytes.len() > 2 * 1024 * 1024 || identity(&before) != identity(&after) || identity(&after) != identity(&leaf) {
        return Err("declaration input identity changed".into());
    }
    Ok((Binding { path: path.to_owned(), sha256: sha(&bytes) },bytes))
}
fn nsstring_compile_inputs(root: &Path, cargo: &Path, owner: u32) -> Result<NSStringCompileInputs> {
    nsstring_compile_inputs_with_deadline(root,cargo,owner,None)
}
fn nsstring_compile_inputs_with_deadline(root:&Path,cargo:&Path,owner:u32,whole:Option<Instant>)->Result<NSStringCompileInputs> {
    native_capture_alive(whole)?;
    let mut metadata = Command::new(cargo);
    metadata.args(["metadata","--locked","--offline","--format-version","1","--manifest-path"])
        .arg(root.join("bench/f1-runner/Cargo.toml")).current_dir(root);
    let output = native_tool_with_deadline(&mut metadata,root,whole)?;
    if !output.status.success() { return Err("declaration locked Cargo metadata failed".into()); }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| "declaration metadata JSON")?;
    let packages = value["packages"].as_array().ok_or("declaration package roster")?
        .iter().filter(|p| p["name"] == "objc2-foundation" && p["version"] == "0.3.2").collect::<Vec<_>>();
    if packages.len() != 1 { return Err("exact locked Foundation package required".into()); }
    let manifest = Path::new(packages[0]["manifest_path"].as_str().ok_or("Foundation manifest")?);
    let nodes = value["resolve"]["nodes"].as_array().ok_or("locked Foundation resolution")?
        .iter().filter(|node| node["id"] == packages[0]["id"]).collect::<Vec<_>>();
    if nodes.len() != 1 { return Err("exact locked Foundation feature node required".into()); }
    let features = nodes[0]["features"].as_array().ok_or("resolved Foundation features")?
        .iter().map(|v| v.as_str().map(str::to_owned).ok_or("Foundation feature name"))
        .collect::<std::result::Result<std::collections::BTreeSet<_>,_>>()?;
    if !features.contains("NSString") { return Err("resolved Foundation NSString feature absent".into()); }
    if fs::canonicalize(manifest).map_err(|_| "Foundation manifest canonicalization")? != manifest {
        return Err("Foundation package path alias".into());
    }
    let package = manifest.parent().ok_or("Foundation package root")?.to_owned();
    let mut relative = BTreeMap::new();
    walk(&package, &package.join("src"), &mut relative)?;
    let mut source_before = relative.into_iter().map(|(p,h)|(package.join(p),h)).collect::<BTreeMap<_,_>>();
    source_before.insert(manifest.to_owned(), declaration_file(manifest, owner)?.sha256);
    let mut xcrun = Command::new("/usr/bin/xcrun");
    xcrun.args(["--sdk","macosx","--show-sdk-path"]).current_dir(root);
    let sdk = native_tool_with_deadline(&mut xcrun,root,whole)?;
    if !sdk.status.success() { return Err("selected SDK resolution failed".into()); }
    let selected = std::str::from_utf8(&sdk.stdout).map_err(|_| "selected SDK UTF8")?.trim();
    if selected.is_empty() || selected.lines().count() != 1 { return Err("selected SDK path count".into()); }
    let sdk_root = fs::canonicalize(selected).map_err(|_| "selected SDK canonicalization")?;
    let header_path = fs::canonicalize(sdk_root.join("System/Library/Frameworks/Foundation.framework/Headers/NSString.h")).map_err(|_| "selected SDK NSString header")?;
    if !header_path.starts_with(&sdk_root) { return Err("selected SDK header escaped SDK".into()); }
    let header = declaration_file(&header_path,0)?;
    native_capture_alive(whole)?;
    Ok(NSStringCompileInputs { package, package_identity: json!({"id":packages[0]["id"],"name":packages[0]["name"],"version":packages[0]["version"],"source":packages[0]["source"],"manifest":file_binding(manifest)?}), features, source_before, sdk_root, header, owner })
}
fn nsstring_compiler_role(commands: &[Vec<String>]) -> Result<&[String]> {
    let roles = commands.iter().filter(|args| args.windows(2).any(|a| a[0] == "--crate-name" && a[1] == "objc2_foundation")).collect::<Vec<_>>();
    if roles.len() != 1 { return Err("exact same-build Foundation compiler required; cached artifact has no declaration provenance".into()); }
    let args = roles[0];
    if !matches!(native_argument(args,"--crate-type")?, "lib" | "rlib")
        || !args.windows(2).any(|a|a[0] == "--cfg" && a[1] == "feature=\"NSString\"")
    { return Err("Foundation compiler role/NSString feature differs".into()); }
    native_requires_link(args)?;
    Ok(args)
}
fn nsstring_compile_pins(root: &Path, out: &Path, before: &NSStringCompileInputs,
    commands: &[Vec<String>], selected_rlib: &Path, compiler: &Path, emit_receipt: bool) -> Result<String> {
    let args = nsstring_compiler_role(commands)?;
    let features = args.windows(2).filter(|a|a[0]=="--cfg")
        .filter_map(|a|a[1].strip_prefix("feature=\"").and_then(|v|v.strip_suffix('"')))
        .map(str::to_owned).collect::<std::collections::BTreeSet<_>>();
    if features != before.features { return Err("actual Foundation features differ from locked resolution".into()); }
    for (path, expected) in &before.source_before {
        if declaration_file(path, before.owner)?.sha256 != *expected {
            return Err("Foundation package source changed across compile".into());
        }
    }
    if fs::canonicalize(&args[0]).map_err(|_| "Foundation compiler path")? != compiler {
        return Err("Foundation actual compiler differs from selected toolchain".into());
    }
    let sources = args.iter().filter(|s| !s.contains('=') && Path::new(s).extension() == Some(std::ffi::OsStr::new("rs"))).collect::<Vec<_>>();
    if sources.len() != 1 { return Err("Foundation compiler source count".into()); }
    let source = Path::new(sources[0]);
    let source = fs::canonicalize(if source.is_absolute() { source.to_owned() } else { before.package.join(source) }).map_err(|_| "Foundation compiler source")?;
    if source != before.package.join("src/lib.rs") { return Err("Foundation actual compiler source differs".into()); }
    let output = Path::new(native_argument(args,"--out-dir")?);
    if !output.is_absolute() || !output.starts_with(root.join("rust-native/target")) { return Err("Foundation compiler output escaped owned target".into()); }
    let suffix = args.windows(2).filter(|a|a[0]=="-C").filter_map(|a|a[1].strip_prefix("extra-filename=")).collect::<Vec<_>>();
    if suffix.len()!=1 || suffix[0].contains(['/', '\\']) { return Err("Foundation output suffix".into()); }
    let emitted = output.join(format!("libobjc2_foundation{}.rlib",suffix[0]));
    if fs::canonicalize(&emitted).map_err(|_| "Foundation emitted rlib")? != selected_rlib {
        return Err("SDK selected extern is not same-build Foundation output".into());
    }
    let dep = output.join(format!("objc2_foundation{}.d",suffix[0]));
    let mut actual = BTreeMap::new();
    for path in dependency_paths(&fs::read_to_string(&dep).map_err(|_| "Foundation actual dep-info")?)? {
        let path = fs::canonicalize(if path.is_absolute() {path} else {before.package.join(path)}).map_err(|_| "Foundation dep-info input")?;
        let pin = declaration_file(&path,before.owner)?;
        if before.source_before.get(&path) != Some(&pin.sha256) { return Err("Foundation dep-info input absent/changed from precompile source".into()); }
        actual.insert(path,pin.sha256);
    }
    let declaration = before.package.join("src/generated/NSString.rs");
    if !actual.contains_key(&source) || !actual.contains_key(&declaration) { return Err("Foundation dep-info does not contain actual NSString implementation source".into()); }
    let header = declaration_file(&before.header.path,0)?;
    if header.sha256 != before.header.sha256 { return Err("selected SDK header changed across compile".into()); }
    let cargo = declaration_file(&declaration,before.owner)?;
    let pins = serde_json::to_string(&[(&header.path,&header.sha256,0_u32),(&cargo.path,&cargo.sha256,before.owner)]).map_err(|_| "declaration pin literal")?;
    if emit_receipt {
        native_private_bytes(&out.join("NSString-compiler-inputs.json"),&serde_json::to_vec_pretty(&json!({"schema":"ruycast.f1.nsstring-compiler-inputs.v1","compiler":file_binding(Path::new(&args[0]))?,"arguments":args,"package":before.package_identity,"features":features,"dep_info":file_binding(&dep)?,"source_before":before.source_before,"actual_inputs":actual,"selected_extern":file_binding(selected_rlib)?,"sdk_root":before.sdk_root,"foreign_inputs":[header,cargo],"compiled_literal":pins,"runtime_authority":false})).map_err(|_| "declaration receipt serialization")?)?;
    }
    Ok(pins)
}
fn native_compile_role(
    crate_name: &str,
    crate_type: &str,
    source: &Path,
    native: &Path,
    objc: &Path,
) -> Result<Option<&'static str>> {
    let role = match (crate_name, crate_type) {
        ("ruycast_native", "bin") => Some(("native-bin", native.join("src/main.rs"))),
        ("ruycast_native", "lib") => Some(("native-lib", native.join("src/lib.rs"))),
        ("objc2", "lib") => Some(("objc2-lib", objc.join("src/lib.rs"))),
        ("ruycast_native" | "objc2", _) => {
            return Err("required compiler role has wrong crate-type".into())
        }
        _ => None,
    };
    let Some((name, expected)) = role else {
        return Ok(None);
    };
    if source != expected {
        return Err("compiler role actual source differs".into());
    }
    Ok(Some(name))
}
fn native_required_roles(roles: &BTreeMap<&str, usize>) -> Result<()> {
    if roles.len() != 3
        || ["native-bin", "native-lib", "objc2-lib"]
            .iter()
            .any(|name| roles.get(name) != Some(&1))
    {
        return Err("exact same-build Native bin/lib/objc2 roles required; cached artifact has no inherited authority".into());
    }
    Ok(())
}
fn native_requires_link(args: &[String]) -> Result<()> {
    let mut emits = args
        .iter()
        .filter_map(|s| s.strip_prefix("--emit="))
        .collect::<Vec<_>>();
    emits.extend(
        args.windows(2)
            .filter(|p| p[0] == "--emit")
            .map(|p| p[1].as_str()),
    );
    if emits.len() != 1
        || !emits[0].split(',').any(|v| v == "link")
        || !emits[0].split(',').any(|v| v == "dep-info")
    {
        return Err(
            "same-build compiler must actually emit link and dep-info, not reuse old output".into(),
        );
    }
    Ok(())
}
// Cargo's pipelined LIB consumer may select the .rmeta emitted by the
// same invocation whose .rlib the BIN links. Join only that exact producer;
// changing a suffix or merely swapping extensions is not sufficient.
fn native_extern_link_path(root:&Path,commands:&[Vec<String>],name:&str,selected:&Path)->Result<PathBuf> {
    if selected.extension()!=Some(std::ffi::OsStr::new("rmeta")){return Ok(selected.into());}
    let mut matches=Vec::new();
    for args in commands {
        if native_argument(args,"--crate-name")?!=name {continue;}
        if !matches!(native_argument(args,"--crate-type")?,"lib"|"rlib"){continue;}
        native_requires_link(args)?;
        if !args.iter().filter_map(|arg|arg.strip_prefix("--emit="))
            .any(|emit|emit.split(',').any(|part|part=="metadata")) {
            return Err("selected metadata was not emitted by actual linking compiler".into());
        }
        let output=Path::new(native_argument(args,"--out-dir")?);
        if !output.is_absolute() || !output.starts_with(root.join("rust-native/target")) {
            return Err("metadata producer output leaves existing target".into());
        }
        let suffix=args.windows(2).filter(|pair|pair[0]=="-C")
            .filter_map(|pair|pair[1].strip_prefix("extra-filename=")).collect::<Vec<_>>();
        if suffix.len()!=1 || suffix[0].is_empty() || suffix[0].contains(['/', '\\']) {
            return Err("metadata producer output suffix differs".into());
        }
        if selected==output.join(format!("lib{name}{}.rmeta",suffix[0])) {
            matches.push(output.join(format!("lib{name}{}.rlib",suffix[0])));
        }
    }
    if matches.len()!=1{return Err("selected metadata lacks unique same-build linking producer".into());}
    Ok(matches.remove(0))
}
// Produced only by the actual argv/DI/output joins below, never deserialized
// from a persisted compiler receipt. Data and callback capture share this
// compiler boundary; typed callback/DWARF qualification stays downstream.
struct NativeRoleContext {
    inputs: BTreeMap<PathBuf,Value>,
    externs: BTreeMap<String,Value>,
    compilers: BTreeMap<PathBuf,Value>,
    depfiles: Vec<Value>,
    roles: BTreeMap<&'static str,usize>,
    objc_output: Option<Value>,
    objc_depinfo: Option<Value>,
    objc_consumers: BTreeMap<&'static str,Value>,
}
fn native_role_context(root:&Path,commands:&[Vec<String>],
    objc:&(PathBuf,BTreeMap<PathBuf,String>),owned:&Inventory,whole:Option<Instant>) -> Result<NativeRoleContext> {
    native_capture_alive(whole)?;
    let mut inputs = BTreeMap::new();
    let mut externs = BTreeMap::new();
    let mut compilers = BTreeMap::new();
    let mut depfiles = vec![];
    let mut roles = BTreeMap::new();
    let mut objc_output = None;
    let mut objc_depinfo = None;
    let mut objc_consumers = BTreeMap::new();
    for args in commands {
        native_capture_alive(whole)?;
        let compiler = fs::canonicalize(&args[0]).map_err(|e| e.to_string())?;
        compilers.insert(compiler.clone(), file_binding(&compiler)?);
        let name = native_argument(args, "--crate-name")?;
        if !matches!(name, "ruycast_native" | "objc2") {
            continue;
        }
        native_requires_link(args)?;
        let cwd = if name == "objc2" {
            objc.0.clone()
        } else {
            root.join("rust-native")
        };
        let sources = args
            .iter()
            .skip(1)
            .filter(|s| {
                !s.contains('=') && Path::new(s).extension() == Some(std::ffi::OsStr::new("rs"))
            })
            .collect::<Vec<_>>();
        if sources.len() != 1 {
            return Err("actual role source argument missing/duplicate".into());
        }
        let source = Path::new(sources[0]);
        let source = fs::canonicalize(if source.is_absolute() {
            source.to_owned()
        } else {
            cwd.join(source)
        })
        .map_err(|e| e.to_string())?;
        let role = native_compile_role(
            name,
            native_argument(args, "--crate-type")?,
            &source,
            &root.join("rust-native"),
            &objc.0,
        )?
        .ok_or("required compiler role missing")?;
        *roles.entry(role).or_insert(0) += 1;
        let output = Path::new(native_argument(args, "--out-dir")?);
        if !output.is_absolute() || !output.starts_with(root.join("rust-native/target")) {
            return Err("Native rustc output escaped existing target".into());
        }
        let extra = args
            .windows(2)
            .filter(|p| p[0] == "-C")
            .filter_map(|p| p[1].strip_prefix("extra-filename="))
            .collect::<Vec<_>>();
        if extra.len() != 1 || extra[0].contains('/') || extra[0].contains('\\') {
            return Err("Native compiler output suffix missing/invalid".into());
        }
        let dep = output.join(format!("{name}{}.d", extra[0]));
        let body = fs::read_to_string(&dep).map_err(|e| e.to_string())?;
        let mut role_inputs = BTreeMap::new();
        for source in dependency_paths(&body)? {
            let source = if source.is_absolute() {
                source
            } else {
                cwd.join(source)
            };
            let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
            if role == "objc2-lib" {
                let binding = file_binding(&source)?;
                if objc.1.get(&source).map(String::as_str) != binding["sha256"].as_str() {
                    return Err("actual objc2 dep-info source absent/different from before-build macro/source closure".into());
                }
                role_inputs.insert(source.clone(), binding);
                continue;
            }
            if !source.starts_with(root) {
                return Err("Native dep-info source outside owned inventory".into());
            }
            let binding = file_binding(&source)?;
            let relative = source
                .strip_prefix(root)
                .map_err(|_| "Native source root differs")?;
            if owned.files.get(relative).map(String::as_str) != binding["sha256"].as_str() {
                return Err(
                    "actual Native dependency absent/different in before-build owned inventory"
                        .into(),
                );
            }
            inputs.insert(source.clone(), binding);
        }
        if role == "objc2-lib" {
            for leaf in [
                "src/lib.rs",
                "src/macros/define_class.rs",
                "src/__macro_helpers/defined_ivars.rs",
            ] {
                if !role_inputs.contains_key(&objc.0.join(leaf)) {
                    return Err("actual objc2 dep-info missing mandatory macro input".into());
                }
            }
            let artifact = output.join(format!("libobjc2{}.rlib", extra[0]));
            let artifact = fs::canonicalize(artifact).map_err(|e| e.to_string())?;
            if objc_output.replace(file_binding(&artifact)?).is_some() {
                return Err("duplicate same-build objc2 output".into());
            }
            objc_depinfo = Some(
                json!({"command":args,"source":file_binding(&source)?,"dep_info":file_binding(&dep)?,"actual_inputs":role_inputs}),
            );
            depfiles.push(file_binding(&dep)?);
            continue;
        }
        depfiles.push(file_binding(&dep)?);
        let selected = args
            .windows(2)
            .filter(|p| p[0] == "--extern" && p[1].starts_with("objc2="))
            .collect::<Vec<_>>();
        if selected.len() != 1 {
            return Err(
                "each Native compiler role must select exactly one actual objc2 extern".into(),
            );
        }
        let path = fs::canonicalize(
            selected[0][1]
                .strip_prefix("objc2=")
                .ok_or("objc2 extern path missing")?,
        )
        .map_err(|e| e.to_string())?;
        let selected_binding=file_binding(&path)?;
        if inputs.insert(path.clone(),selected_binding.clone()).is_some_and(|old|old!=selected_binding) {
            return Err("Native selected objc2 metadata changed across consumers".into());
        }
        let linked=native_extern_link_path(root,commands,"objc2",&path)?;
        objc_consumers.insert(role,json!({"selected":selected_binding,"linked_output":file_binding(&linked)?}));
        for pair in args.windows(2).filter(|p| p[0] == "--extern") {
            let (name, path) = pair[1]
                .split_once('=')
                .ok_or("actual Native extern lacks file")?;
            let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
            let selected_binding=file_binding(&path)?;
            if inputs.insert(path.clone(),selected_binding.clone()).is_some_and(|old|old!=selected_binding) {
                return Err("Native selected extern bytes changed across consumers".into());
            }
            let linked=native_extern_link_path(root,commands,name,&path)?;
            let binding = file_binding(&linked)?;
            if let Some(old) = externs.insert(name.to_owned(), binding.clone()) {
                if old != binding {
                    return Err("Native extern changed across invocations".into());
                }
            }
        }
    }
    native_required_roles(&roles)?;
    if externs.get("objc2") != objc_output.as_ref()
        || objc_consumers.len() != 2
        || objc_consumers
            .values()
            .any(|v| Some(&v["linked_output"]) != objc_output.as_ref())
    {
        return Err(
            "actual Native objc2 extern does not match exact same-build objc2 output".into(),
        );
    }
    if native_objc_inputs(&objc.0)? != objc.1 {
        return Err("objc2 source closure changed across Native build".into());
    }
    native_capture_alive(whole)?;
    Ok(NativeRoleContext{inputs,externs,compilers,depfiles,roles,objc_output,objc_depinfo,objc_consumers})
}

fn native_capture_closure(
    root: &Path,
    out: &Path,
    artifact: &Path,
    commands: &[Vec<String>],
    objc: &(PathBuf, BTreeMap<PathBuf, String>),
    role_context: NativeRoleContext,
    whole:Option<Instant>,
) -> Result<Value> {
    use std::os::unix::fs::PermissionsExt;
    native_capture_alive(whole)?;
    let NativeRoleContext{inputs,externs,compilers,depfiles,roles,objc_output,objc_depinfo,objc_consumers}=role_context;
    native_private_bytes(
        &out.join("native-rustc-arguments.json"),
        &serde_json::to_vec_pretty(commands).map_err(|e| e.to_string())?,
    )?;
    let dsym = out.join("native-host.dSYM");
    let mut tool = Command::new("/usr/bin/xcrun");
    tool.args(["dsymutil"]).arg(artifact).arg("-o").arg(&dsym);
    let result = native_tool_with_deadline(&mut tool,root,whole)?;
    native_private_bytes(&out.join("native-dsymutil.stdout"), &result.stdout)?;
    native_private_bytes(&out.join("native-dsymutil.stderr"), &result.stderr)?;
    if !result.status.success() || !result.stderr.is_empty() {
        return Err(
            "mandatory dsymutil failed/warned; complete same-build objects not proven".into(),
        );
    }
    let dwarf = dsym.join("Contents/Resources/DWARF/native-host");
    for directory in [
        &dsym,
        &dsym.join("Contents"),
        &dsym.join("Contents/Resources"),
        &dsym.join("Contents/Resources/DWARF"),
    ] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    fs::set_permissions(&dwarf, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    let uuid = native_uuid(&fs::read(artifact).map_err(|e| e.to_string())?)?;
    if native_uuid(&fs::read(&dwarf).map_err(|e| e.to_string())?)? != uuid {
        return Err("retained Native and dSYM UUID differ".into());
    }
    let mut tool = Command::new("/usr/bin/xcrun");
    tool.args(["dsymutil", "--dump-debug-map"]).arg(artifact);
    let result = native_tool_with_deadline(&mut tool,root,whole)?;
    native_private_bytes(&out.join("native-debug-map.txt"), &result.stdout)?;
    native_private_bytes(&out.join("native-debug-map.stderr"), &result.stderr)?;
    if !result.status.success() || !result.stderr.is_empty() {
        return Err("same-build debug map failed/warned; no compiler authority".into());
    }
    let map = String::from_utf8(result.stdout).map_err(|_| "debug map UTF8 differs")?;
    let object_dir = out.join("compiler-objects");
    fs::create_dir(&object_dir).map_err(|e| e.to_string())?;
    fs::set_permissions(&object_dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    let mut objects = BTreeMap::new();
    // Preserve the complete owned object set before any typed dump can refuse.
    // This is raw compiler data, never a substitute for the checks below.
    let mut raw_objects = BTreeMap::new();
    let map_objects=super::callbacks::debug_map(&map)?.into_iter()
        .map(|entry|entry.object).collect::<std::collections::BTreeSet<_>>();
    for name in &map_objects {
        native_capture_alive(whole)?;
        if name.contains('\'') || name.contains('\0') {
            return Err("unsupported compiler object path escape".into());
        }
        let path = Path::new(name);
        if path.extension() != Some(std::ffi::OsStr::new("o")) {
            continue;
        }
        if !path.is_absolute()
            || !path.starts_with(root.join("rust-native/target"))
            || fs::canonicalize(path).map_err(|e| e.to_string())? != path
        {
            return Err("compiler object not exact owned target input".into());
        }
        space_for_build(root)?;
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let hash = sha(&bytes);
        let retained = object_dir.join(format!("{hash}.o"));
        if !retained.exists() {
            native_private_bytes(&retained, &bytes)?;
        }
        let original=file_binding(path)?;
        if original["sha256"]!=hash {return Err("compiler object changed during raw retention".into());}
        raw_objects.insert(name.to_owned(),json!({"original":original,"retained":file_binding(&retained)?}));
    }
    if raw_objects.is_empty() {return Err("actual same-build compiler object roster absent".into());}
    let mut retained_externs=BTreeMap::new();
    let artifact_dir=out.join("compiler-artifacts");
    fs::create_dir(&artifact_dir).map_err(|e|e.to_string())?;
    fs::set_permissions(&artifact_dir,fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
    for (name,binding) in &externs {
        native_capture_alive(whole)?;
        space_for_build(root)?;
        let path=Path::new(binding["path"].as_str().ok_or("compiler extern path absent")?);
        let bytes=fs::read(path).map_err(|e|e.to_string())?;
        if sha(&bytes)!=binding["sha256"] {return Err("compiler extern changed before raw retention".into());}
        let suffix=path.extension().and_then(|v|v.to_str()).ok_or("compiler extern suffix absent")?;
        let retained=artifact_dir.join(format!("{}.{}",sha(&bytes),suffix));
        if !retained.exists() {native_private_bytes(&retained,&bytes)?;}
        if file_binding(path)?!=*binding {return Err("compiler extern changed during raw retention".into());}
        retained_externs.insert(name.clone(),json!({"original":binding,"retained":file_binding(&retained)?}));
    }
    native_capture_alive(whole)?;
    native_private_bytes(&out.join("native-raw-compiler-cohort.json"),&serde_json::to_vec_pretty(&json!({
        "scope":"raw compiler checkpoint before object DWARF and callback parsing","authority":false,
        "native":file_binding(artifact)?,"uuid":uuid,"dsym":file_binding(&dwarf)?,
        "compiler_arguments":file_binding(&out.join("native-rustc-arguments.json"))?,
        "cargo_result":file_binding(&out.join("native-build-result.json"))?,
        "cargo_artifacts":file_binding(&out.join("native-compiler.stdout"))?,
        "source_before":file_binding(&out.join("native-raw-source-before.json"))?,
        "debug_map":file_binding(&out.join("native-debug-map.txt"))?,
        "objects":raw_objects,"externs":retained_externs,"typed_object_dump_complete":false
    })).map_err(|e|e.to_string())?)?;
    for (name,raw) in &raw_objects {
        native_capture_alive(whole)?;
        let path=Path::new(name);
        let retained=Path::new(raw["retained"]["path"].as_str().ok_or("raw retained object path absent")?);
        let hash=raw["retained"]["sha256"].as_str().ok_or("raw retained object hash absent")?;
        let mut tool = Command::new("/usr/bin/xcrun");
        tool.args(["dwarfdump", "--debug-info"]).arg(retained);
        let result = native_tool_with_deadline(&mut tool,root,whole)?;
        if !result.status.success()
            || !result.stderr.is_empty()
            || !result
                .stdout
                .windows(19)
                .any(|s| s == b"DW_TAG_compile_unit")
        {
            return Err("required compiler object typed DWARF absent".into());
        }
        let capture = object_dir.join(format!("{hash}.dwarf.txt"));
        if !capture.exists() {
            native_private_bytes(&capture, &result.stdout)?;
        }
        if sha(&fs::read(path).map_err(|e| e.to_string())?) != hash {
            return Err("compiler object changed while retaining typed DWARF".into());
        }
        objects.insert(name.to_owned(),json!({"original":file_binding(path)?,"retained":file_binding(retained)?,"dwarf":file_binding(&capture)?}));
    }
    if objects.is_empty() {
        return Err("actual same-build compiler object roster absent".into());
    }
    let mut tool = Command::new("/usr/bin/nm");
    tool.arg("-a").arg(artifact);
    let result = native_tool_with_deadline(&mut tool,root,whole)?;
    if !result.status.success() || !result.stderr.is_empty() {
        return Err("retained Native symbols missing".into());
    }
    native_private_bytes(&out.join("native-symbols.txt"), &result.stdout)?;
    let input_closure=out.join("native-input-build-closure.json");
    native_private_bytes(&input_closure,&serde_json::to_vec_pretty(&json!({
        "native":file_binding(artifact)?,"native_uuid":uuid,
        "compiler_arguments":file_binding(&out.join("native-rustc-arguments.json"))?,
        "cargo_command":file_binding(&out.join("native-build-command.json"))?,
        "toolchain":file_binding(&out.join("native-toolchain.json"))?,
        "foundation_actual_compile":file_binding(&out.join("native-foundation-compiler/NSString-compiler-inputs.json"))?,
        "compilers":compilers,"compiler_roles":roles,"actual_source_inputs":inputs,
        "externs":externs,"dep_info":depfiles,"objc2_actual_compile":objc_depinfo,
        "objc2_source_inputs":objc.1,"objc2_same_build_output":objc_output,
        "objc2_native_consumers":objc_consumers,"objects":objects,
        "debug_map":file_binding(&out.join("native-debug-map.txt"))?,
        "symbols":file_binding(&out.join("native-symbols.txt"))?
    })).map_err(|e|e.to_string())?)?;
    let input_callbacks = native_input_callback_pins(root, out, artifact, &input_closure, &objects, &inputs,
        (&map,std::str::from_utf8(&result.stdout).map_err(|_| "Native symbols UTF8 differs")?,whole))?;
    native_capture_alive(whole)?;
    let receipt = json!({"schema":"ruycast.f1.native-compiler-capture.v1","native":file_binding(artifact)?,"native_uuid":uuid,"dsym":file_binding(&dwarf)?,"compiler_arguments":file_binding(&out.join("native-rustc-arguments.json"))?,"cargo_command":file_binding(&out.join("native-build-command.json"))?,"toolchain":file_binding(&out.join("native-toolchain.json"))?,"compilers":compilers,"compiler_roles":roles,"objc2_same_build_output":objc_output,"objc2_native_consumers":objc_consumers,"objc2_actual_compile":objc_depinfo,"dep_info":depfiles,"actual_source_inputs":inputs,"externs":externs,"objc2_source_inputs":objc.1,"objects":objects,"debug_map":file_binding(&out.join("native-debug-map.txt"))?,"symbols":file_binding(&out.join("native-symbols.txt"))?,"input_callbacks":input_callbacks,"unstripped":true,"callback_authority":false,"source_routes_qualified":false});
    native_private_bytes(
        &out.join("native-compiler-capture.json"),
        &serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
    )?;
    native_capture_alive(whole)?;
    file_binding(&out.join("native-compiler-capture.json"))
}

/// Fixed Input5 producer data, never an admission witness. Every object and
/// source here belongs to the just-completed, checked compiler capture above.
fn native_input_callback_pins(
    root: &Path, out: &Path, artifact: &Path, compiler_closure: &Path,
    objects: &BTreeMap<String, Value>, inputs: &BTreeMap<PathBuf, Value>,
    tools: (&str, &str,Option<Instant>),
) -> Result<Value> {
    let (debug_map,symbols,whole)=tools;
    native_capture_alive(whole)?;
    for member in ["main.rs","owned_host_entry.rs","carbon_hotkeys.rs","hid_service.rs","keyword_capture.rs","native_keyboard.rs"] {
        let source=root.join("rust-native/src").join(member);
        if inputs.get(&source)!=Some(&file_binding(&source)?) {
            return Err("Input typed getter/capsule source absent or changed in actual DI".into());
        }
    }
    let mut retained = Vec::new();
    for (original, object) in objects {
        let object_path = Path::new(object["retained"]["path"].as_str().ok_or("Input object path absent")?);
        let dwarf_path = Path::new(object["dwarf"]["path"].as_str().ok_or("Input DWARF path absent")?);
        if file_binding(object_path)? != object["retained"] || file_binding(dwarf_path)? != object["dwarf"] {
            return Err("Input compiler object/DWARF changed after capture".into());
        }
        retained.push((original.clone(), fs::read_to_string(dwarf_path).map_err(|e|e.to_string())?,
            fs::read(object_path).map_err(|e|e.to_string())?));
    }
    let captures = retained.iter().map(|(path,dwarf,bytes)| super::callbacks::ObjectCapture {
        original_path:path, dwarf, bytes,
    }).collect::<Vec<_>>();
    let image = fs::read(artifact).map_err(|e|e.to_string())?;
    let proof = super::callbacks::input_five_linked_code(&captures,debug_map,symbols,&image)?;
    let (base,_,_) = host_image_preflight(&image,2,0)?;
    let capsule = image_symbol(symbols,"_RUYCAST_NATIVE_INPUT_CALLBACKS_V1",0)?;
    input_capsule_section(&image,capsule)?;
    let bytes = readonly_file_range(&image,capsule,56)?;
    if bytes[..8] != 0xF1000026494E5054_u64.to_le_bytes()
        || bytes[8..12] != 1_u32.to_le_bytes() || bytes[12..16] != 5_u32.to_le_bytes() {
        return Err("Input MAIN capsule header differs".into());
    }
    let mut tool=Command::new("/usr/bin/dyld_info");
    tool.arg("-fixups").arg(artifact);
    let result=native_tool_with_deadline(&mut tool,root,whole)?;
    native_private_bytes(&out.join("native-input-fixups.txt"),&result.stdout)?;
    native_private_bytes(&out.join("native-input-fixups.stderr"),&result.stderr)?;
    if !result.status.success() || !result.stderr.is_empty() {
        return Err("Input MAIN static fixup capture failed/warned".into());
    }
    let fixup_text=std::str::from_utf8(&result.stdout).map_err(|_|"Input fixups UTF8 differs")?;
    let fixups=static_rebases(fixup_text)?;
    let capsule_end=capsule.checked_add(56).ok_or("Input capsule range overflow")?;
    for line in fixup_text.lines() {
        let fields=line.split_whitespace().collect::<Vec<_>>();
        let Some(slot)=fields.get(2).and_then(|value|value.strip_prefix("0x"))
            .and_then(|value|u64::from_str_radix(value,16).ok()) else {continue};
        if (capsule..capsule_end).contains(&slot) && fields.get(3)!=Some(&"rebase") {
            return Err("Input capsule contains a non-rebase fixup".into());
        }
    }
    if fixups.range(capsule..capsule_end).map(|(slot,_)|*slot).collect::<Vec<_>>()
        !=(0..5).map(|index|capsule+16+index*8).collect::<Vec<_>>() {
        return Err("Input capsule contains missing or extra rebase slots".into());
    }
    let bodies=proof["bodies"].as_array().filter(|v|v.len()==5).ok_or("Input compiler body roster differs")?;
    let mut pins=Vec::new();
    for (index,(body,id)) in bodies.iter().zip(["carbon_pressed","hid_input","hid_removed","hid_filter","keyword_filter"]).enumerate() {
        let linked=&body["linked"];
        let address=linked["image_address"].as_u64().ok_or("Input linked body address absent")?;
        let size=linked["function_size"].as_u64().filter(|v|*v>0&&*v<=65536).ok_or("Input body byte bound differs")?;
        let offset=address.checked_sub(base).filter(|v|*v>0).ok_or("Input body precedes MAIN")?;
        require_rx_offset(&image,base,offset)?;
        require_rx_offset(&image,base,offset.checked_add(size-1).ok_or("Input body range overflow")?)?;
        let slot=capsule.checked_add(16+index as u64*8).ok_or("Input capsule slot overflow")?;
        if fixups.get(&slot)!=Some(&address) { return Err("Input capsule fixup is not its exact typed C body".into()); }
        let relative=body["source_member"].as_str().ok_or("Input fixed source member absent")?;
        let path=root.join(relative);
        let source=inputs.get(&path).ok_or("Input body source absent from actual same-build DI")?;
        if file_binding(&path)?!=*source || linked["declaration_file"].as_str()!=path.to_str() {
            return Err("Input body declaration/source bytes differ from compiler closure".into());
        }
        pins.push(json!({"id":id,"offset":offset,"bytes":size,"body_sha256":sha(readonly_file_range(&image,address,size as usize)?),"source":source}));
    }
    // A callback capsule is not the typed userdata publisher. Require its real
    // same-image factory/caller/retirement graph from the same retained objects;
    // unresolved imported/indirect/capture targets remain a hard refusal.
    let mut publisher_route=super::callbacks::input_publisher_linked_code(&captures,debug_map,symbols,&image)?;
    if publisher_route["uuid"]!=json!(native_uuid(&image)?)
        || publisher_route["factories"].as_array().is_none_or(|v|v.len()!=3) {
        return Err("Input publisher factory/image roster differs".into());
    }
    let bodies=publisher_route["bodies"].as_array_mut().filter(|v|!v.is_empty())
        .ok_or("Input publisher typed transitive bodies absent")?;
    for body in bodies {
        native_capture_alive(whole)?;
        let linked=&body["linked"];
        let address=linked["image_address"].as_u64().ok_or("Input publisher body address absent")?;
        let size=linked["function_size"].as_u64().filter(|v|*v>0&&*v<=65536)
            .ok_or("Input publisher body byte bound differs")?;
        let offset=address.checked_sub(base).filter(|v|*v>0).ok_or("Input publisher body precedes MAIN")?;
        require_rx_offset(&image,base,offset)?;
        require_rx_offset(&image,base,offset.checked_add(size-1).ok_or("Input publisher body range overflow")?)?;
        let source=PathBuf::from(linked["declaration_file"].as_str().ok_or("Input publisher source declaration absent")?);
        let source_pin=inputs.get(&source).ok_or("Input publisher source is outside actual same-build DI")?;
        if file_binding(&source)?!=*source_pin { return Err("Input publisher source bytes changed after capture".into()); }
        let object=objects.get(linked["object_path"].as_str().ok_or("Input publisher object origin absent")?)
            .ok_or("Input publisher original object absent from compiler closure")?;
        body["body_sha256"]=json!(sha(readonly_file_range(&image,address,size as usize)?));
        body["source"]=source_pin.clone();
        body["object"]=object["retained"].clone();
        body["dwarf"]=object["dwarf"].clone();
    }
    publisher_route["artifact"]=file_binding(artifact)?;
    publisher_route["compiler_closure"]=file_binding(compiler_closure)?;
    publisher_route["debug_map"]=file_binding(&out.join("native-debug-map.txt"))?;
    publisher_route["symbols"]=file_binding(&out.join("native-symbols.txt"))?;
    native_capture_alive(whole)?;
    let evidence=out.join("native-input-compiler-evidence.json");
    native_private_bytes(&evidence,&serde_json::to_vec_pretty(&json!({
        "schema":"ruycast.f1.native-input-compiler.v1","artifact":file_binding(artifact)?,
        "compiler_closure":file_binding(compiler_closure)?,
        "body_proof":proof,"publisher_route":publisher_route,"capsule_offset":capsule.checked_sub(base).ok_or("Input capsule precedes MAIN")?,
        "fixups":file_binding(&out.join("native-input-fixups.txt"))?,"callbacks":pins,
        "runtime_admission":false
    })).map_err(|e|e.to_string())?)?;
    native_capture_alive(whole)?;
    Ok(json!({"main_uuid":native_uuid(&image)?,"capsule_offset":capsule-base,"callbacks":pins,"evidence":file_binding(&evidence)?}))
}

fn input_capsule_section(image:&[u8], address:u64)->Result<()> {
    let word=|at:usize|->Result<u32>{Ok(u32::from_le_bytes(image.get(at..at+4).ok_or("Input section word leaves image")?.try_into().map_err(|_|"Input section word")?))};
    let wide=|at:usize|->Result<u64>{Ok(u64::from_le_bytes(image.get(at..at+8).ok_or("Input section address leaves image")?.try_into().map_err(|_|"Input section address")?))};
    let mut at=32_usize; let mut found=0;
    for _ in 0..word(16)? {
        let size=word(at+4)? as usize;
        if size<8 || at.checked_add(size).is_none_or(|end|end>image.len()) { return Err("Input segment command leaves image".into()); }
        if word(at)?==0x19 {
            let count=word(at+64)? as usize;
            if count.checked_mul(80).and_then(|n|n.checked_add(72)).is_none_or(|n|n>size) { return Err("Input sections exceed segment".into()); }
            for i in 0..count {
                let section=at+72+i*80;
                let name=&image[section..section+16];
                if name.split(|b|*b==0).next()==Some(b"__ruycast_input") {
                    if image[at+8..at+24].split(|b|*b==0).next()!=Some(b"__DATA_CONST")
                        || image[section+16..section+32].split(|b|*b==0).next()!=Some(b"__DATA_CONST")
                        || wide(section+32)?!=address || wide(section+40)?!=56 || !address.is_multiple_of(8) {
                        return Err("Input capsule section/size/alignment differs".into());
                    }
                    found+=1;
                }
            }
        }
        at+=size;
    }
    if found!=1 { return Err("Input capsule section absent or ambiguous".into()); }
    Ok(())
}
#[cfg(test)]
mod native_compiler_capture_tests {
    use super::*;
    #[test]
    fn original_workloads_provenance_is_pinned_without_reading_the_historical_file() {
        let measurement = std::env::temp_dir().join(format!(
            "ruycast-missing-workload-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = measurement.join(ORIGINAL_WORKLOADS_PATH);
        assert!(!source.exists());
        assert_eq!(
            original_workloads_binding(&measurement),
            json!({
                "path": source,
                "sha256": "a0534165a6df3d6afda628abaefdc8f740b4b0a54a969f814af76e8860382950"
            })
        );
    }
    #[test]
    #[ignore="explicit actual Native compiler capture required; no image execution"]
    fn actual_objc_doc_inputs_are_in_package_inventory() {
        let capture=PathBuf::from(std::env::var_os("RUYCAST_F1_NATIVE_CAPTURE_DIR").expect("actual capture required"));
        let root=checkout_root().unwrap();
        assert!(capture.starts_with(root.join("rust-native/target")));
        assert_eq!(fs::canonicalize(&capture).unwrap(),capture);
        let before:Value=super::super::read(&capture.join("objc2-source-before.json")).unwrap();
        let package=Path::new(before["package"]["manifest_path"].as_str().unwrap()).parent().unwrap();
        let commands:Vec<Vec<String>>=super::super::read(&capture.join("native-raw-rustc-arguments.json")).unwrap();
        let roles=commands.iter().filter(|args|native_argument(args,"--crate-name").unwrap()=="objc2").collect::<Vec<_>>();
        assert_eq!(roles.len(),1);
        let args=roles[0];
        let suffix=args.windows(2).filter(|pair|pair[0]=="-C")
            .filter_map(|pair|pair[1].strip_prefix("extra-filename=")).collect::<Vec<_>>();
        assert_eq!(suffix.len(),1);
        let dep=Path::new(native_argument(args,"--out-dir").unwrap()).join(format!("objc2{}.d",suffix[0]));
        let paths=dependency_paths(&fs::read_to_string(dep).unwrap()).unwrap();
        let inventory=native_objc_inputs(package).unwrap();
        let mut examples=0;
        for path in paths {
            let path=fs::canonicalize(if path.is_absolute(){path}else{package.join(path)}).unwrap();
            examples+=usize::from(path.starts_with(package.join("examples")));
            assert_eq!(inventory.get(&path),Some(&sha(&fs::read(&path).unwrap())),"{}",path.display());
        }
        assert!(examples>0,"actual doc-included examples must be exercised");
        // Preflight the ENTIRE observed extern graph before paying for
        // another fresh compiler cohort, including pipelining and dual-output
        // rustcast_core. Actual Cargo artifacts must confirm the selected file.
        let summary:Value=super::super::read(&capture.join("native-compiler.stdout")).unwrap();
        let records=summary["records"].as_array().unwrap();
        let mut metadata=0;
        let mut dual_output=0;
        for command in &commands {
            let outputs=native_emitted_libraries(command).unwrap();
            dual_output+=usize::from(outputs.len()==2);
            for pair in command.windows(2).filter(|pair|pair[0]=="--extern") {
                let Some((alias,path))=pair[1].split_once('=') else {assert_eq!(pair[1],"proc_macro");continue;};
                let path=Path::new(path);
                metadata+=usize::from(path.extension()==Some(std::ffi::OsStr::new("rmeta")));
                let linked=native_extern_link_path(&root,&commands,alias,path).unwrap();
                assert_eq!(commands.iter().filter(|producer|native_emitted_libraries(producer).unwrap().contains(&linked)).count(),1,"{}",path.display());
                let artifacts=records.iter().filter(|row|row["reason"]=="compiler-artifact" && row["fresh"]==false
                    && row["filenames"].as_array().is_some_and(|files|files.iter().any(|file|file.as_str()==path.to_str()))).collect::<Vec<_>>();
                assert_eq!(artifacts.len(),1,"{}",path.display());
                assert!(fs::metadata(path).unwrap().len()>0);
            }
        }
        assert!(metadata>0 && dual_output>0);
        let owned_before:Inventory=super::super::read(&capture.join("native-raw-source-before.json")).unwrap();
        assert_eq!(owned_before.root,root);
        let mut shared_defaults=0;
        for command in &commands {
            let name=native_argument(command,"--crate-name").unwrap();
            let suffix=command.windows(2).filter(|pair|pair[0]=="-C")
                .filter_map(|pair|pair[1].strip_prefix("extra-filename=")).collect::<Vec<_>>();
            assert_eq!(suffix.len(),1);
            let dep=Path::new(native_argument(command,"--out-dir").unwrap()).join(format!("{name}{}.d",suffix[0]));
            for path in dependency_paths(&fs::read_to_string(dep).unwrap()).unwrap() {
                let path=fs::canonicalize(if path.is_absolute(){path}else{root.join("rust-native").join(path)}).unwrap();
                if path.starts_with(&root) && !path.starts_with(root.join("rust-native/target")) {
                    let relative=path.strip_prefix(&root).unwrap();
                    assert_eq!(owned_before.files.get(relative),Some(&sha(&fs::read(&path).unwrap())),"{}",path.display());
                    shared_defaults+=usize::from(relative==Path::new("config/default.json"));
                }
            }
        }
        assert!(shared_defaults>0,"real cross-crate include of defaults must be covered");
        // Coverage diagnostic over genuine compiler data; not a new before
        // observation for that old build or any executable capability.
    }
    #[test]
    fn metadata_selection_requires_exact_actual_linking_producer() {
        let root=Path::new("/owned");
        let producer=["/compiler/rustc","--crate-name","objc2_foundation","--crate-type","lib",
            "--emit=dep-info,metadata,link","-C","extra-filename=-same",
            "--out-dir","/owned/rust-native/target/target/release/deps"].map(str::to_owned).to_vec();
        let selected=Path::new("/owned/rust-native/target/target/release/deps/libobjc2_foundation-same.rmeta");
        let commands=vec![producer.clone()];
        assert_eq!(native_extern_link_path(root,&commands,"objc2_foundation",selected).unwrap(),selected.with_extension("rlib"));
        assert!(native_extern_link_path(root,&commands,"objc2_foundation",&selected.with_file_name("libobjc2_foundation-other.rmeta")).is_err());
        assert!(native_extern_link_path(root,&[producer.clone(),producer.clone()],"objc2_foundation",selected).is_err());
        let mut no_metadata=producer.clone();no_metadata[5]="--emit=dep-info,link".into();
        assert!(native_extern_link_path(root,&[no_metadata],"objc2_foundation",selected).is_err());
        let mut no_link=producer;no_link[5]="--emit=dep-info,metadata".into();
        assert!(native_extern_link_path(root,&[no_link],"objc2_foundation",selected).is_err());
        assert!(native_extern_link_path(root,&[],"objc2_foundation",selected).is_err());
        let mut dual=commands[0].clone();
        dual.extend(["--crate-type".into(),"staticlib".into()]);
        dual[4]="rlib".into();
        let outputs=native_emitted_libraries(&dual).unwrap();
        assert_eq!(outputs.len(),2);
        assert!(outputs.contains(&selected.with_extension("rlib")));
        assert!(outputs.contains(&selected.with_extension("a")));
        let mut duplicate=dual.clone();duplicate[4]="staticlib".into();
        assert!(native_emitted_libraries(&duplicate).is_err());
        dual[4]="dylib".into();
        assert!(native_emitted_libraries(&dual).is_err());
    }
    #[test]
    fn native_whole_budget_refuses_before_inventory_and_preserves_unbudgeted_policy() {
        use std::os::unix::fs::MetadataExt;
        let parent=PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("explicit owned fixture parent"));
        let metadata=fs::symlink_metadata(&parent).unwrap();
        assert!(metadata.is_dir()&&metadata.uid()==unsafe{libc::geteuid()}&&metadata.mode()&0o777==0o700);
        assert_eq!(fs::canonicalize(&parent).unwrap(),parent);
        let root=parent.join(format!("native-budget-absent-{}",std::process::id()));
        assert!(fs::symlink_metadata(&root).is_err());
        let out=root.join("rust-native/target/never-created");
        let cap=NativeCompileCap::new().unwrap();
        let expired=Instant::now()-std::time::Duration::from_millis(1);
        let error=native_stage_with_deadline(&root,&out,&cap,Some(expired)).unwrap_err();
        assert!(error.contains("whole Reference capture absolute deadline"));
        assert!(fs::symlink_metadata(&root).is_err());
        // The unchanged two-image route still reaches its old source-path
        // validation, rather than inheriting this command's whole budget.
        let error=native_stage(&root,&out,&cap).unwrap_err();
        assert!(!error.contains("deadline"));
        assert!(fs::symlink_metadata(&root).is_err());
    }
    #[test]
    fn reference_plain_actor_files_preserve_pinned_history_and_refuse_source_mutation() {
        use std::os::unix::fs::{MetadataExt,PermissionsExt};
        let parent=PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("explicit owned fixture parent"));
        let owner=unsafe{libc::geteuid()};
        let metadata=fs::symlink_metadata(&parent).unwrap();
        assert!(metadata.is_dir()&&metadata.uid()==owner&&metadata.mode()&0o777==0o700);
        assert_eq!(fs::canonicalize(&parent).unwrap(),parent);
        let root=parent.join(format!("reference-actor-inputs-{}",std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(root.join("assets")).unwrap();
        let icon=fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/app-icon.png")).unwrap();
        fs::write(root.join("assets/app-icon.png"),&icon).unwrap();
        let history=serde_json::to_vec(&vec![json!({"text":"owned test history"});200]).unwrap();
        let history_path=root.join("clipboard-200.json");
        fs::write(&history_path,&history).unwrap();
        let manifest=root.join("fixture.json");
        fs::write(&manifest,serde_json::to_vec(&json!({"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":1730000000_u64,
            "files":{"clipboard-200.json":file_binding(&history_path).unwrap()}})).unwrap()).unwrap();
        let fixture:Binding=serde_json::from_value(file_binding(&manifest).unwrap()).unwrap();
        let make_roots=|suffix:&str|{
            let base=root.join(suffix);fs::create_dir(&base).unwrap();
            ["home","cache","temp","applications"].map(|name|{
                let path=base.join(name);fs::create_dir(&path).unwrap();
                fs::set_permissions(&path,fs::Permissions::from_mode(0o700)).unwrap();path
            })
        };
        let roots=make_roots("first");
        let (files,origins)=reference_actor_environment(&root,&roots,&fixture,owner,1730000000).unwrap();
        assert_eq!(files.len(),4);
        assert_eq!(origins[0].sha256,sha(&history));
        assert_eq!(origins[1].sha256,sha(&icon));
        assert_eq!(fs::read(&files["history"].path).unwrap(),history);
        assert_eq!(fs::read(&files["icon"].path).unwrap(),icon);
        for binding in files.values() {
            assert_eq!(fs::metadata(&binding.path).unwrap().mode()&0o777,0o400);
            assert_eq!(file_binding(&binding.path).unwrap()["sha256"],binding.sha256);
        }
        assert_eq!(fs::metadata(&files["executable"].path).unwrap().mode()&0o111,0);
        assert_eq!(fs::read_dir(&roots[3]).unwrap().count(),1);
        let roots2=make_roots("refused");
        assert!(reference_actor_environment(&root,&roots2,&fixture,owner,1730000001).is_err());
        fs::write(&history_path,b"[]").unwrap();
        assert!(reference_actor_environment(&root,&roots2,&fixture,owner,1730000000).is_err());
        assert_eq!(fs::read_dir(&roots2[0]).unwrap().count(),0);
        assert_eq!(fs::read_dir(&roots2[3]).unwrap().count(),0);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fixture_epoch_comes_from_the_bounded_manifest_not_an_environment_default() {
        use std::os::unix::fs::{MetadataExt,PermissionsExt};
        let parent=PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("explicit owned fixture parent"));
        let owner=unsafe{libc::geteuid()};
        let meta=fs::symlink_metadata(&parent).unwrap();
        assert!(meta.is_dir()&&meta.uid()==owner&&meta.mode()&0o777==0o700);
        assert_eq!(fs::canonicalize(&parent).unwrap(),parent);
        let root=parent.join(format!("fixture-epoch-{}",std::process::id()));
        fs::create_dir(&root).unwrap();fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).unwrap();
        let path=root.join("fixture.json");
        let valid=br#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":1730000000,"other_fixture_fields":true}"#;
        fs::write(&path,valid).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();
        let (binding,epoch)=fixture_epoch_binding(&path,owner).unwrap();
        assert_eq!(epoch,1730000000);assert_eq!(binding.sha256,sha(valid));
        assert!(fixture_epoch_binding(&path,owner^1).is_err());
        for bad in [r#"{"schema":"other","epoch_unix_seconds":1730000000}"#,
            r#"{"schema":"ruycast.f1.fixture.v1"}"#,r#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":-1}"#,
            r#"{"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":1.5}"#] {
            fs::write(&path,bad).unwrap();assert!(fixture_epoch_binding(&path,owner).is_err());
        }
        fs::remove_file(&path).unwrap();
        let c=std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe{libc::mkfifo(c.as_ptr(),0o600)},0);
        assert!(fixture_epoch_binding(&path,owner).is_err()); // no writer, no blocking open
        fs::remove_file(&path).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]
    fn data_declaration_literal_requires_closed_role_and_exact_private_uid_tuple() {
        assert_eq!(data_declaration_pins(DataCase::AppleEvent,&[],501,None).unwrap(),None);
        assert!(data_declaration_pins(DataCase::AppleEvent,&[],501,Some("[]")).is_err());
        let inputs=[Binding{path:"/not-opened/NSString.h".into(),sha256:"a".repeat(64)},
            Binding{path:"/not-opened/NSString.rs".into(),sha256:"b".repeat(64)}];
        assert!(data_declaration_pins(DataCase::AppleEvent,&inputs,501,None).is_err());
        assert!(data_declaration_pins(DataCase::Foundation,&inputs[..1],501,None).is_err());
        assert!(data_declaration_pins(DataCase::Foundation,&inputs,501,None).is_err());
        assert!(data_declaration_pins(DataCase::Foundation,&inputs,501,Some("invalid")).is_err());
        let wrong=serde_json::to_string(&[(&inputs[0].path,&inputs[0].sha256,0_u32),
            (&inputs[1].path,&inputs[1].sha256,502_u32)]).unwrap();
        assert_eq!(data_declaration_pins(DataCase::Foundation,&inputs,501,Some(&wrong)).unwrap_err(),
            "Foundation compiled declaration literal differs from private pins");
    }
    #[test]
    fn canonical_data_recipes_never_promote_private_lab_records() {
        use std::os::unix::fs::{MetadataExt,PermissionsExt};
        let parent=PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("explicit owned fixture parent"));
        let metadata=fs::symlink_metadata(&parent).unwrap();
        assert!(metadata.is_dir()&&metadata.uid()==unsafe{libc::getuid()}&&metadata.mode()&0o777==0o700);
        assert_eq!(fs::canonicalize(&parent).unwrap(),parent);
        let root=parent.join(format!("canonical-data-recipe-{}",std::process::id()));
        let out=root.join("rust-native/target/owned-output");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("rust-native")).unwrap();
        fs::create_dir(root.join("rust-native/target")).unwrap();
        fs::create_dir(&out).unwrap();
        fs::set_permissions(&out,fs::Permissions::from_mode(0o700)).unwrap();
        let cap=NativeCompileCap::new().unwrap();
        for case in [DataCase::AppleEvent, DataCase::Foundation] {
            assert_eq!(source_owned_data_recipe(&root,&out,case,&cap).err().unwrap(),
                "canonical Data source-owned compiler recipe is not yet qualified");
        }
        fs::set_permissions(&out,fs::Permissions::from_mode(0o755)).unwrap();
        assert!(source_owned_data_recipe(&root,&out,DataCase::Foundation,&cap).err().unwrap().contains("same compilecap owner"));
        fs::remove_dir(&out).unwrap();
        fs::remove_dir(root.join("rust-native/target")).unwrap();
        fs::remove_dir(root.join("rust-native")).unwrap();
        fs::remove_dir(&root).unwrap();
    }
    #[test]
    fn canonical_recipe_paths_refuse_laboratory_traversal_and_source_outputs() {
        assert!(recipe_relative_path(Path::new("bench/f1-runner/src/sdk/metadata.rs"),false).is_ok());
        for path in ["", "/outside/source.rs", "../source.rs", "./source.rs", "a/./b.rs", "a//b.rs", "a/b.rs/", ".go/input.json", ".build/source.rs", "rust-native/target/output.rs"] {
            assert!(recipe_relative_path(Path::new(path),false).is_err(),"{path}");
        }
        assert!(recipe_relative_path(Path::new("rust-native/target/release/deps/libx.rlib"),true).is_ok());
        assert!(recipe_relative_path(Path::new("bench/old.rlib"),true).is_err());
    }
    #[test]
    fn canonical_data_argv_uses_closed_aliases_and_source_entry() {
        let root=Path::new("/owned/checkout");
        let names=["block2","libc","objc2","objc2_core_foundation","objc2_foundation","ruycast_native","serde","serde_json","sha2"];
        let dependencies=names.iter().map(|name|(root.join(format!("rust-native/target/release/deps/lib{name}-fixed.rlib")),"already-qualified-elsewhere".into())).collect::<BTreeMap<_,String>>();
        let args=data_compiler_arguments(root,DataCase::Foundation,&dependencies,Path::new("/qualified/toolchain/bin/rustc"),Path::new("/owned/out"),Path::new("/owned/out.d")).unwrap();
        assert_eq!(args.iter().filter(|s|*s=="--extern").count(),9);
        assert_eq!(args[0],"/qualified/toolchain/bin/rustc");
        assert!(!args.iter().any(|s|s=="+1.92.0"));
        for compiler in ["rustc","/qualified/bin/not-rustc","/qualified/../bin/rustc","/qualified//bin/rustc"] {
            assert!(data_compiler_arguments(root,DataCase::Foundation,&dependencies,Path::new(compiler),Path::new("/owned/out"),Path::new("/owned/out.d")).is_err());
        }
        assert!(args.contains(&root.join(DataCase::Foundation.entry()).to_str().unwrap().to_owned()));
        assert!(args.contains(&"lto=thin".into()));
        assert!(args.contains(&"debug-assertions=yes".into()));
        let mut missing=dependencies.clone();missing.remove(&root.join("rust-native/target/release/deps/libsha2-fixed.rlib"));
        assert!(data_compiler_arguments(root,DataCase::Foundation,&missing,Path::new("/qualified/toolchain/bin/rustc"),Path::new("/owned/out"),Path::new("/owned/out.d")).is_err());
        let mut duplicate=missing;duplicate.insert(root.join("rust-native/target/release/deps/libserde-second.rlib"),"not-authority".into());
        assert!(data_compiler_arguments(root,DataCase::Foundation,&duplicate,Path::new("/qualified/toolchain/bin/rustc"),Path::new("/owned/out"),Path::new("/owned/out.d")).is_err());
    }
    #[test]
    fn declaration_readcap_refuses_alias_hardlink_mode_owner_and_size() {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink};
        let parent = PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
            .expect("explicit owned fixture parent required"));
        assert_eq!(fs::canonicalize(&parent).unwrap(),parent);
        let metadata = fs::symlink_metadata(&parent).unwrap();
        let owner = unsafe { libc::geteuid() };
        assert!(metadata.is_dir() && metadata.uid()==owner && metadata.mode()&0o777==0o700);
        let root = parent.join(format!("foreign-readcap-{}",std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.join("NSString.rs");
        fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&path).unwrap();
        fs::write(&path,b"owned declaration fixture").unwrap();
        assert_eq!(declaration_file(&path,owner).unwrap().sha256,sha(b"owned declaration fixture"));
        assert!(declaration_file(&path,owner^1).is_err());
        let alias=root.join("alias");symlink(&path,&alias).unwrap();
        assert!(declaration_file(&alias,owner).is_err());fs::remove_file(alias).unwrap();
        let linked=root.join("linked");fs::hard_link(&path,&linked).unwrap();
        assert!(declaration_file(&path,owner).is_err());fs::remove_file(linked).unwrap();
        fs::set_permissions(&path,fs::Permissions::from_mode(0o660)).unwrap();
        assert!(declaration_file(&path,owner).is_err());
        fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();
        let fifo = root.join("declaration-fifo");
        let c_fifo = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_fifo.as_ptr(),0o600) },0);
        // No writer exists: a blocking RO-open would hang BEFORE regular-file
        // validation. NONBLOCK must reach that validation and refuse instead.
        assert!(declaration_file(&fifo,owner).is_err());
        fs::remove_file(fifo).unwrap();
        fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(2*1024*1024+1).unwrap();
        assert!(declaration_file(&path,owner).is_err());
        let tool=root.join("rustc");
        fs::write(&tool,b"read-only compiler binding canary; never executed").unwrap();
        let compiler=Binding{path:tool.clone(),sha256:sha(&fs::read(&tool).unwrap())};
        verify_data_compiler(&compiler).unwrap();
        fs::write(&tool,b"changed bytes").unwrap();
        assert!(verify_data_compiler(&compiler).is_err());
        fs::remove_file(&tool).unwrap();
        let c_tool=std::ffi::CString::new(tool.to_str().unwrap()).unwrap();
        assert_eq!(unsafe{libc::mkfifo(c_tool.as_ptr(),0o600)},0);
        assert!(verify_data_compiler(&compiler).is_err()); // no writer, no blocking open
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn foundation_declaration_requires_one_actual_linking_nsstring_compile() {
        let role = vec!["/owned/rustc", "--crate-name", "objc2_foundation", "--crate-type", "lib",
            "--cfg", "feature=\"NSString\"", "--emit=dep-info,metadata,link"]
            .into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert!(nsstring_compiler_role(&[]).is_err()); // cached output alone
        assert_eq!(nsstring_compiler_role(std::slice::from_ref(&role)).unwrap(), role);
        assert!(nsstring_compiler_role(&[role.clone(),role.clone()]).is_err());
        let mut wrong = role.clone(); wrong[4] = "bin".into();
        assert!(nsstring_compiler_role(&[wrong]).is_err());
        let mut wrong = role.clone(); wrong[6] = "feature=\"NSObject\"".into();
        assert!(nsstring_compiler_role(&[wrong]).is_err());
        let mut wrong = role; wrong[7] = "--emit=dep-info,metadata".into();
        assert!(nsstring_compiler_role(&[wrong]).is_err());
    }
    #[test]
    fn exact_native_roles_and_fresh_objc_output_are_required() {
        let native = Path::new("/owned/native");
        let objc = Path::new("/owned/objc2");
        assert_eq!(
            native_compile_role(
                "ruycast_native",
                "bin",
                &native.join("src/main.rs"),
                native,
                objc
            )
            .unwrap(),
            Some("native-bin")
        );
        assert_eq!(
            native_compile_role(
                "ruycast_native",
                "lib",
                &native.join("src/lib.rs"),
                native,
                objc
            )
            .unwrap(),
            Some("native-lib")
        );
        assert!(native_compile_role(
            "ruycast_native",
            "lib",
            &native.join("src/main.rs"),
            native,
            objc
        )
        .is_err());
        assert!(
            native_compile_role("objc2", "lib", &native.join("src/lib.rs"), native, objc).is_err()
        );
        assert!(
            native_required_roles(&BTreeMap::from([("native-lib", 2), ("objc2-lib", 1)])).is_err()
        );
        assert!(
            native_required_roles(&BTreeMap::from([("native-lib", 1), ("native-bin", 1)])).is_err()
        );
        assert!(native_required_roles(&BTreeMap::from([
            ("native-lib", 1),
            ("native-bin", 1),
            ("objc2-lib", 1)
        ]))
        .is_ok());
        assert!(native_requires_link(&["--emit=dep-info,metadata".into()]).is_err());
        assert!(native_requires_link(&["--emit=dep-info,metadata,link".into()]).is_ok());
    }
    #[test]
    fn cargo_argv_is_data_and_discards_environment_values() {
        let text="     Running `PRIVATE_TEST_MARKER='do not retain' /owned/rustc --crate-name ruycast_native --cfg 'feature=\"test-tools\"' '/owned/source path.rs'`\n";
        let commands = native_rustc_commands(text).unwrap();
        assert_eq!(
            commands[0],
            vec![
                "/owned/rustc",
                "--crate-name",
                "ruycast_native",
                "--cfg",
                "feature=\"test-tools\"",
                "/owned/source path.rs"
            ]
        );
        assert!(!serde_json::to_string(&commands)
            .unwrap()
            .contains("do not retain"));
        assert!(native_shell_words("/owned/rustc 'truncated").is_err());
        assert_eq!(
            native_shell_words(r#""a\qb" "a\\b" 'a\b'"#).unwrap(),
            vec![r"a\qb", r"a\b", r"a\b"]
        );
        assert!(native_rustc_commands("Running `/owned/wrapper /owned/rustc a.rs`").is_err());
        assert!(native_argument(
            &[
                "--out-dir".into(),
                "/a".into(),
                "--out-dir".into(),
                "/b".into()
            ],
            "--out-dir"
        )
        .is_err());
    }
    #[test]
    fn cargo_receipt_never_persists_build_script_env_or_raw_display() {
        let output=b"{\"reason\":\"build-script-executed\",\"env\":[[\"TEST\",\"synthetic-private-value\"]]}\n{\"reason\":\"build-finished\",\"success\":true}\n";
        let value =
            native_cargo_summary(output, b"PRIVATE_TEST_MARKER=synthetic-private-value").unwrap();
        assert!(!value.to_string().contains("synthetic-private-value"));
        assert_eq!(value["records"].as_array().unwrap().len(), 1);
        assert!(native_cargo_summary(b"not-json", b"").is_err());
        assert!(native_cargo_summary(b"{\"reason\":\"new-unknown-reason\"}", b"").is_err());
    }
    #[test]
    fn retained_uuid_requires_exactly_one_complete_identity() {
        let mut bytes = vec![0_u8; 56];
        for (at, value) in [(0, 0xfeedfacf_u32), (16, 1), (20, 24), (32, 0x1b), (36, 24)] {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[40..56].copy_from_slice(&[7; 16]);
        assert_eq!(native_uuid(&bytes).unwrap(), [7; 16]);
        let mut duplicate = bytes.clone();
        duplicate.extend_from_slice(&bytes[32..]);
        duplicate[16..20].copy_from_slice(&2_u32.to_le_bytes());
        duplicate[20..24].copy_from_slice(&48_u32.to_le_bytes());
        assert!(native_uuid(&duplicate).is_err());
        assert!(native_uuid(&bytes[..55]).is_err());
        bytes[32..36].copy_from_slice(&0_u32.to_le_bytes());
        assert!(native_uuid(&bytes).is_err());
    }
}

/// Static data from an already retained compiler cohort. This route never
/// rebuilds, reconstructs missing Cargo JSON, or publishes an admission grant.
pub fn native_retained_data(root:&Path,manifest:&Path,pinned:&str,main_sha:&str,out:&Path)->Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let deadline=Instant::now()+std::time::Duration::from_secs(REFERENCE_CAPTURE_SECONDS);
    let root=fs::canonicalize(root).map_err(|e|e.to_string())?;
    let owner=unsafe{libc::getuid()};
    let (manifest_binding,bytes)=declaration_bytes(manifest,owner)?;
    if manifest_binding.sha256!=pinned {return Err("retained Native manifest hash differs".into());}
    let rows:Vec<Value>=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
    if rows.is_empty() || rows.len()>4096 {return Err("retained Native roster bound differs".into());}
    let artifacts=manifest.parent().ok_or("retained manifest parent absent")?.join("artifacts");
    let checked=|path:&Path,hash:&str,size:u64|->Result<Vec<u8>> {
        use std::{io::Read,os::{fd::AsRawFd,unix::fs::{MetadataExt,OpenOptionsExt}}};
        reference_capture_alive(deadline)?;
        if size>512*1024*1024 {return Err("retained input size exceeds bounded artifact read".into());}
        let file=fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)
            .open(path).map_err(|e|e.to_string())?;
        let before=file.metadata().map_err(|e|e.to_string())?;
        let identity=|m:&fs::Metadata|(m.dev(),m.ino(),m.uid(),m.mode(),m.nlink(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
        if !before.is_file() || before.uid()!=owner || before.nlink()!=1 || before.len()!=size
            || unsafe{libc::fcntl(file.as_raw_fd(),libc::F_GETFL)}&libc::O_ACCMODE!=libc::O_RDONLY {
            return Err("retained artifact readcap differs".into());
        }
        let mut bytes=Vec::new();
        (&file).take(size+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        if bytes.len() as u64!=size || (!hash.is_empty() && sha(&bytes)!=hash)
            || identity(&before)!=identity(&file.metadata().map_err(|e|e.to_string())?)
            || identity(&before)!=identity(&fs::symlink_metadata(path).map_err(|e|e.to_string())?) {
            return Err("retained artifact changed across bounded read".into());
        }
        reference_capture_alive(deadline)?;
        Ok(bytes)
    };
    let mut inputs=BTreeMap::new();
    let mut main=None;
    for row in &rows {
        reference_capture_alive(deadline)?;
        let original=PathBuf::from(row["original"].as_str().ok_or("retained original absent")?);
        let retained=PathBuf::from(row["retained"].as_str().ok_or("retained file absent")?);
        let hash=row["sha256"].as_str().ok_or("retained hash absent")?;
        if !original.starts_with(root.join("rust-native/target")) || retained!=artifacts.join(hash)
            || fs::canonicalize(&original).map_err(|e|e.to_string())?!=original
            || fs::canonicalize(&retained).map_err(|e|e.to_string())?!=retained
            || inputs.insert(original.clone(),(retained.clone(),hash.to_owned(),row["bytes"].as_u64().ok_or("retained size absent")?)).is_some() {
            return Err("retained Native original/copy binding differs; no restoration permitted".into());
        }
        let size=row["bytes"].as_u64().ok_or("retained size absent")?;
        checked(&original,hash,size)?;
        let retained_bytes=checked(&retained,hash,size)?;
        if original.file_name().is_some_and(|name|name=="ruycast-native")
            && (hash!=main_sha || main.replace((retained,retained_bytes)).is_some()) {
            return Err("retained Native Main selection differs".into());
        }
    }
    let (main,main_bytes)=main.ok_or("retained Native Main absent")?;
    let main_binding=json!({"path":main,"sha256":main_sha});
    let parent=out.parent().ok_or("static output parent absent")?;
    if !out.is_absolute() || !out.starts_with(root.join("rust-native/target"))
        || fs::canonicalize(parent).map_err(|e|e.to_string())?!=parent
        || out.components().any(|part|!matches!(part,std::path::Component::RootDir|std::path::Component::Normal(_))) {
        return Err("static output must be exclusive inside the existing owned target".into());
    }
    space_for_build(&root)?;
    fs::create_dir(out).map_err(|e|e.to_string())?;
    fs::set_permissions(out,fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
    native_private_bytes(&out.join("retained-inputs.json"),&bytes)?;
    let run=|label:&str,command:&mut Command|->Result<Vec<u8>> {
        reference_capture_alive(deadline)?;
        space_for_build(&root)?;
        native_private_bytes(&out.join(format!("{label}.command.json")),&serde_json::to_vec_pretty(&json!({
            "program":command.get_program().to_string_lossy(),"arguments":command.get_args().map(|v|v.to_string_lossy()).collect::<Vec<_>>(),
            "launcher":file_binding(&fs::canonicalize(command.get_program()).map_err(|e|e.to_string())?)?,"selected_tool_qualified":false,"runtime_execution":false
        })).map_err(|e|e.to_string())?)?;
        let result=native_tool_until(command,&root,deadline)?;
        native_private_bytes(&out.join(format!("{label}.stdout")),&result.stdout)?;
        native_private_bytes(&out.join(format!("{label}.stderr")),&result.stderr)?;
        use std::os::unix::process::ExitStatusExt;
        native_private_bytes(&out.join(format!("{label}.result.json")),&serde_json::to_vec_pretty(&json!({"exit":result.status.code(),"signal":result.status.signal()})).map_err(|e|e.to_string())?)?;
        if !result.status.success() || !result.stderr.is_empty() {return Err("retained Native static tool failed/warned".into());}
        Ok(result.stdout)
    };
    let mut command=Command::new("/usr/bin/xcrun");
    command.args(["dsymutil","--dump-debug-map"]).arg(&main);
    let map=String::from_utf8(run("debug-map",&mut command)?).map_err(|_|"Native debug map UTF8 differs")?;
    let names=super::callbacks::debug_map(&map)?.into_iter().map(|entry|entry.object).collect::<std::collections::BTreeSet<_>>();
    let mut objects=BTreeMap::new();
    for name in names {
        let path=PathBuf::from(&name);
        if path.extension()!=Some(std::ffi::OsStr::new("o")) {continue;}
        let (retained,hash,_)=inputs.get(&path).ok_or("debug-map object absent from actual retained cohort")?;
        objects.insert(name,json!({"original":{"path":path,"sha256":hash},"retained":{"path":retained,"sha256":hash}}));
    }
    if objects.is_empty() {return Err("retained Native owned object set absent".into());}
    // Checkpoint the genuine map/object association before any link or dump.
    native_private_bytes(&out.join("raw-objects.json"),&serde_json::to_vec_pretty(&objects).map_err(|e|e.to_string())?)?;
    let dsym=out.join("native-retained.dSYM");
    let mut command=Command::new("/usr/bin/xcrun");
    command.arg("dsymutil").arg(&main).arg("-o").arg(&dsym);
    run("dsymutil",&mut command)?;
    let dwarf=dsym.join("Contents/Resources/DWARF").join(main.file_name().ok_or("retained Main basename absent")?);
    let uuid=native_uuid(&main_bytes)?;
    // A newly produced dSYM has no prior digest; the same bounded readcap
    // supplies its bytes/identity before deriving a DATA-only hash and UUID.
    let dwarf_size=fs::symlink_metadata(&dwarf).map_err(|e|e.to_string())?.len();
    let dwarf_bytes=checked(&dwarf,"",dwarf_size)?;
    if native_uuid(&dwarf_bytes)?!=uuid {return Err("retained Native dSYM UUID differs".into());}
    native_private_bytes(&out.join("raw-dsym.json"),&serde_json::to_vec_pretty(&json!({"main":main_binding,"dsym":{"path":dwarf,"sha256":sha(&dwarf_bytes)},"uuid":uuid,"authority":false})).map_err(|e|e.to_string())?)?;
    for row in objects.values_mut() {
        reference_capture_alive(deadline)?;
        let path=Path::new(row["retained"]["path"].as_str().ok_or("retained object path absent")?);
        let hash=row["retained"]["sha256"].as_str().ok_or("retained object hash absent")?;
        let mut command=Command::new("/usr/bin/xcrun");
        command.args(["dwarfdump","--debug-info"]).arg(path);
        let label=format!("object-{hash}");
        let dump=run(&label,&mut command)?;
        if !dump.windows(19).any(|s|s==b"DW_TAG_compile_unit") {return Err("retained object typed DWARF absent".into());}
        row["dwarf"]=json!({"path":out.join(format!("{label}.stdout")),"sha256":sha(&dump)});
    }
    for (original,(retained,hash,size)) in &inputs {
        checked(original,hash,*size)?;
        checked(retained,hash,*size)?;
    }
    reference_capture_alive(deadline)?;
    native_private_bytes(&out.join("static-data.json"),&serde_json::to_vec_pretty(&json!({
        "scope":"retained compiler data postprocessing only","main":main_binding,"uuid":uuid,"objects":objects,
        "manifest":{"path":manifest_binding.path,"sha256":manifest_binding.sha256},
        "cargo_fresh_json":"unknown_not_retained","original_capture_status":"FAILED","authority":false,"runtime_execution":false
    })).map_err(|e|e.to_string())?)?;
    reference_capture_alive(deadline)
}

fn native_stage(root:&Path,out:&Path,cap:&NativeCompileCap)->Result<()> {
    native_stage_with_deadline(root,out,cap,None)
}
// Select aliases only from the actual checked Native BIN invocation. This
// preserves compiler provenance; it does not qualify their transitive sources.
fn native_data_dependencies(root:&Path,commands:&[Vec<String>],roles:&NativeRoleContext)->Result<BTreeMap<String,Binding>> {
    const NAMES:[&str;9]=["block2","libc","objc2","objc2_core_foundation","objc2_foundation","ruycast_native","serde","serde_json","sha2"];
    let mut bin=None;
    let mut lib=None;
    for args in commands {
        if native_argument(args,"--crate-name")?!="ruycast_native" {continue;}
        match native_argument(args,"--crate-type")? {
            "bin"=>{if bin.replace(args).is_some(){return Err("duplicate Data Native BIN compiler".into());}},
            "lib"=>{if lib.replace(args).is_some(){return Err("duplicate Data Native LIB compiler".into());}},
            _=>return Err("Data Native compiler role differs".into()),
        }
    }
    let bin=bin.ok_or("actual Data Native BIN compiler absent")?;
    let lib=lib.ok_or("actual Data Native LIB compiler absent")?;
    let mut selected=BTreeMap::new();
    for pair in bin.windows(2).filter(|pair|pair[0]=="--extern") {
        let (name,path)=pair[1].split_once('=').ok_or("Data BIN extern argument lacks path")?;
        if !NAMES.contains(&name) {continue;}
        let path=PathBuf::from(path);
        if !path.is_absolute() || !path.starts_with(root.join("rust-native/target"))
            || fs::canonicalize(&path).map_err(|e|e.to_string())?!=path {
            return Err("Data BIN extern path leaves exact existing target".into());
        }
        let actual=file_binding(&path)?;
        if roles.externs.get(name)!=Some(&actual) {return Err("Data BIN extern differs from checked compiler roles".into());}
        let binding=Binding{path,sha256:actual["sha256"].as_str().ok_or("Data BIN extern digest absent")?.to_owned()};
        if selected.insert(name.to_owned(),binding).is_some(){return Err("duplicate Data BIN extern selection".into());}
    }
    if selected.len()!=NAMES.len(){return Err("actual Native BIN lacks exact nine Data dependencies".into());}
    let suffix=lib.windows(2).filter(|pair|pair[0]=="-C")
        .filter_map(|pair|pair[1].strip_prefix("extra-filename=")).collect::<Vec<_>>();
    if suffix.len()!=1 || suffix[0].contains(['/', '\\']) {return Err("Data Native LIB emitted suffix differs".into());}
    let emitted=Path::new(native_argument(lib,"--out-dir")?).join(format!("libruycast_native{}.rlib",suffix[0]));
    if selected["ruycast_native"].path!=emitted {
        return Err("Data BIN selected Native extern is not actual Native LIB output".into());
    }
    Ok(selected)
}

type NativeTraceIdentity=(u64,u64,u32,u32,u32,u64,u64,i64,i64,i64,i64);
#[derive(serde::Serialize)]
struct NativeTraceFile { binding:Binding,identity:NativeTraceIdentity }
#[derive(serde::Serialize)]
struct NativeTraceBefore {
    directories:BTreeMap<PathBuf,std::collections::BTreeSet<PathBuf>>,
    files:BTreeMap<PathBuf,Option<NativeTraceFile>>,
}
fn native_trace_file(path:&Path,owner:u32,whole:Option<Instant>)->Result<Option<(NativeTraceFile,Vec<u8>)>> {
    use std::os::unix::fs::MetadataExt;
    native_capture_alive(whole)?;
    let before=match fs::symlink_metadata(path) {
        Ok(value)=>value,
        Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(error)=>return Err(error.to_string()),
    };
    let identity=|m:&fs::Metadata|(m.dev(),m.ino(),m.uid(),m.gid(),m.mode(),m.nlink(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
    let (binding,bytes)=declaration_bytes(path,owner)?;
    let after=fs::symlink_metadata(path).map_err(|e|e.to_string())?;
    if identity(&before)!=identity(&after){return Err("Cargo run trace changed during observation".into());}
    native_capture_alive(whole)?;
    Ok(Some((NativeTraceFile{binding,identity:identity(&after)},bytes)))
}
fn native_trace_before(target:&Path,triple:&str,owner:u32,whole:Option<Instant>)->Result<NativeTraceBefore> {
    use std::os::unix::fs::MetadataExt;
    let mut directories=BTreeMap::new();let mut files=BTreeMap::new();let mut entries=0;
    for base in [target.join("release/build"),target.join(triple).join("release/build")] {
        native_capture_alive(whole)?;
        let mut prefix=target.to_owned();
        for part in base.strip_prefix(target).map_err(|_|"Cargo trace target prefix")?.components() {
            prefix.push(part);
            match fs::symlink_metadata(&prefix) {
                Ok(metadata)=>if !metadata.is_dir() || metadata.uid()!=owner || metadata.mode()&0o022!=0
                    || fs::canonicalize(&prefix).map_err(|e|e.to_string())?!=prefix {return Err("Cargo trace parent owner/alias".into());},
                Err(error)if error.kind()==std::io::ErrorKind::NotFound=>break,
                Err(error)=>return Err(error.to_string()),
            }
        }
        let mut present=std::collections::BTreeSet::new();
        let iterator=match fs::read_dir(&base) {
            Ok(value)=>Some(value),
            Err(error)if error.kind()==std::io::ErrorKind::NotFound=>None,
            Err(error)=>return Err(error.to_string()),
        };
        if let Some(iterator)=iterator {
            for entry in iterator {
                native_capture_alive(whole)?;entries+=1;if entries>100000{return Err("Cargo trace directory bound".into());}
                let path=entry.map_err(|e|e.to_string())?.path();let metadata=fs::symlink_metadata(&path).map_err(|e|e.to_string())?;
                if !metadata.is_dir() || metadata.uid()!=owner || metadata.mode()&0o022!=0
                    || fs::canonicalize(&path).map_err(|e|e.to_string())?!=path {return Err("Cargo trace run-directory owner/alias".into());}
                for name in ["invoked.timestamp","root-output"] {
                    let file=path.join(name);files.insert(file.clone(),native_trace_file(&file,owner,whole)?.map(|(file,_)|file));
                }
                present.insert(path);
            }
        }
        directories.insert(base,present);
    }
    native_capture_alive(whole)?;
    Ok(NativeTraceBefore{directories,files})
}
fn native_dirty_trace(before:&NativeTraceBefore,output:&Path,owner:u32,whole:Option<Instant>)
    ->Result<Option<(NativeTraceFile,NativeTraceFile)>> {
    use std::os::unix::{ffi::OsStrExt,fs::MetadataExt};
    if output.file_name()!=Some(std::ffi::OsStr::new("out")){return Err("Cargo OUT_DIR layout differs from bound Cargo version".into());}
    let run=output.parent().ok_or("Cargo run-directory absent")?;
    let base=run.parent().ok_or("Cargo build-directory absent")?;
    let dirs=before.directories.get(base).ok_or("Cargo OUT_DIR not in observed target build roots")?;
    let metadata=fs::symlink_metadata(run).map_err(|e|e.to_string())?;
    if !metadata.is_dir() || metadata.uid()!=owner || metadata.mode()&0o022!=0
        || fs::canonicalize(run).map_err(|e|e.to_string())?!=run {return Err("Cargo produced run-directory owner/alias".into());}
    let mut after=Vec::new();
    for name in ["invoked.timestamp","root-output"] {
        let path=run.join(name);
        let old=if dirs.contains(run){before.files.get(&path).ok_or("Cargo existing trace before-state absent")?.as_ref()}else{None};
        let Some((current,bytes))=native_trace_file(&path,owner,whole)? else{return Ok(None);};
        if old.is_some_and(|old|old.identity==current.identity){return Ok(None);}
        after.push((current,bytes));
    }
    if after[0].1!=b"This file has an mtime of when this was started."
        || after[1].1!=output.as_os_str().as_bytes() {
        return Err("Cargo dirty trace content/JSON OUT_DIR differs".into());
    }
    native_capture_alive(whole)?;
    let root=after.pop().ok_or("Cargo root trace absent")?;let marker=after.pop().ok_or("Cargo invocation trace absent")?;
    Ok(Some((marker.0,root.0)))
}
#[derive(serde::Serialize)]
struct NativeProducedRoot {
    package_id:String,
    out_dir:PathBuf,
    producer:Binding,
    compiler_arguments:Vec<String>,
    invocation:NativeTraceFile,
    root_output:NativeTraceFile,
}
// Cargo may report cached build-script metadata too. Only a same-call Running
// executable joined to an actual fresh script compilation is a produced root.
fn native_produced_roots(root:&Path,streams:(&[u8],&[u8]),commands:&[Vec<String>],
    packages:&BTreeMap<String,(PathBuf,u32)>,trace_before:&NativeTraceBefore,owner:u32,whole:Option<Instant>)
    ->Result<BTreeMap<PathBuf,NativeProducedRoot>> {
    let (stdout,stderr)=streams;
    let mut running=std::collections::BTreeSet::new();
    for line in std::str::from_utf8(stderr).map_err(|_|"Cargo Running display UTF8")?.lines() {
        let Some(body)=line.trim().strip_prefix("Running `").and_then(|s|s.strip_suffix('`'))else{continue;};
        let words=native_shell_words(body)?;
        let Some(start)=words.iter().position(|word|!word.contains('='))else{continue;};
        let program=&words[start];
        if Path::new(program).file_name()==Some(std::ffi::OsStr::new("build-script-build")) {
            let path=PathBuf::from(program);
            if !path.is_absolute() || !path.starts_with(root.join("rust-native/target"))
                || fs::canonicalize(&path).map_err(|e|e.to_string())?!=path {
                return Err("actual build-script executable escaped target".into());
            }
            if !running.insert(path){return Err("actual build-script invocation ambiguous".into());}
        }
    }
    let records=stdout.split(|b|*b==b'\n').filter(|line|!line.is_empty())
        .map(|line|serde_json::from_slice::<Value>(line).map_err(|_|"Cargo produced-node JSON".to_owned()))
        .collect::<Result<Vec<_>>>()?;
    if records.len()>8192{return Err("Cargo produced-node record bound".into());}
    let mut roots=BTreeMap::new();
    for value in &records {
        native_capture_alive(whole)?;
        if value["reason"]!="build-script-executed"{continue;}
        let id=value["package_id"].as_str().ok_or("build-script package id absent")?;
        let (package,_)=packages.get(id).ok_or("build-script package absent from genuine before metadata")?;
        let output=PathBuf::from(value["out_dir"].as_str().ok_or("build-script OUT_DIR absent")?);
        if !output.is_absolute() || !output.starts_with(root.join("rust-native/target"))
            || fs::canonicalize(&output).map_err(|e|e.to_string())?!=output {
            return Err("actual build-script OUT_DIR escaped target".into());
        }
        let mut matched=None;
        for args in commands {
            if native_argument(args,"--crate-name")?!="build_script_build"{continue;}
            let sources=args.iter().filter(|arg|!arg.contains('=')&&Path::new(arg).extension()==Some(std::ffi::OsStr::new("rs"))).collect::<Vec<_>>();
            if sources.len()!=1{return Err("build-script source argument ambiguous".into());}
            // External relative argv has no recorded package cwd: never guess.
            let source=Path::new(sources[0]);
            if !source.is_absolute(){continue;}
            let source=fs::canonicalize(source).map_err(|e|e.to_string())?;
            if !source.starts_with(package){continue;}
            native_requires_link(args)?;
            let suffix=args.windows(2).filter(|p|p[0]=="-C")
                .filter_map(|p|p[1].strip_prefix("extra-filename=")).collect::<Vec<_>>();
            if suffix.len()!=1 || suffix[0].contains(['/', '\\']){return Err("build-script emitted suffix differs".into());}
            let dir=Path::new(native_argument(args,"--out-dir")?);
            let executable=dir.join("build-script-build");
            if !running.contains(&executable){continue;}
            let emitted=dir.join(format!("build_script_build{}",suffix[0]));
            let artifacts=records.iter().filter(|record|record["reason"]=="compiler-artifact"
                && record["package_id"]==id && record["fresh"]==false
                && record["target"]["kind"].as_array().is_some_and(|kinds|kinds.len()==1&&kinds[0]=="custom-build")
                && record["target"]["src_path"].as_str()==source.to_str()
                && record["filenames"].as_array().is_some_and(|files|files.iter()
                    .any(|file|file.as_str()==executable.to_str()||file.as_str()==emitted.to_str())))
                .collect::<Vec<_>>();
            if artifacts.is_empty(){continue;}
            if artifacts.len()!=1{return Err("actual build-script package/source artifact ambiguous".into());}
            let producer=compiler_input_bytes(&executable,owner,whole)?.0;
            if compiler_input_bytes(&emitted,owner,whole)?.0.sha256!=producer.sha256 {
                return Err("executed build-script bytes differ from actual compiler output".into());
            }
            let Some((invocation,root_output))=native_dirty_trace(trace_before,&output,owner,whole)? else{continue;};
            if matched.replace(NativeProducedRoot{package_id:id.to_owned(),out_dir:output.clone(),producer,
                compiler_arguments:args.clone(),invocation,root_output}).is_some(){return Err("produced OUT_DIR compiler join ambiguous".into());}
        }
        // A cached/unknown producer is deliberately not qualified as produced.
        if let Some(produced)=matched {
            if roots.insert(output,produced).is_some(){return Err("duplicate produced OUT_DIR".into());}
        }
    }
    native_capture_alive(whole)?;
    Ok(roots)
}

#[derive(serde::Serialize)]
struct NativeArtifact {
    package_id:String,
    source:PathBuf,
    filenames:Vec<PathBuf>,
    fresh:bool,
}
fn native_artifacts(stdout:&[u8],whole:Option<Instant>)->Result<Vec<NativeArtifact>> {
    let mut artifacts=Vec::new();
    for line in stdout.split(|b|*b==b'\n').filter(|line|!line.is_empty()) {
        native_capture_alive(whole)?;
        let value:Value=serde_json::from_slice(line).map_err(|_|"Cargo actual artifact JSON")?;
        if value["reason"]!="compiler-artifact"{continue;}
        let source=PathBuf::from(value["target"]["src_path"].as_str().ok_or("Cargo artifact source absent")?);
        if !source.is_absolute()||fs::canonicalize(&source).map_err(|e|e.to_string())?!=source {
            return Err("Cargo artifact source is not canonical".into());
        }
        let filenames=value["filenames"].as_array().ok_or("Cargo artifact outputs absent")?.iter()
            .map(|path|path.as_str().map(PathBuf::from).ok_or_else(||"Cargo artifact output not path".to_owned())).collect::<Result<Vec<_>>>()?;
        if filenames.is_empty()||filenames.len()>1024||filenames.iter().any(|p|!p.is_absolute()) {
            return Err("Cargo artifact output roster bound/path".into());
        }
        artifacts.push(NativeArtifact{package_id:value["package_id"].as_str().ok_or("Cargo artifact package absent")?.to_owned(),
            source,filenames,fresh:value["fresh"].as_bool().ok_or("Cargo artifact freshness absent")?});
        if artifacts.len()>8192{return Err("Cargo artifact record bound".into());}
    }
    native_capture_alive(whole)?;
    Ok(artifacts)
}
#[derive(serde::Serialize)]
struct NativeGeneratedInput {
    binding:Binding,
    package_id:String,
    producer_root:PathBuf,
    consumer_arguments:Vec<String>,
}
#[derive(serde::Serialize)]
struct NativeCompilerInput { binding:Binding,owner:u32,origin:&'static str }
#[derive(serde::Serialize)]
struct NativeCompilerSources {
    inputs:BTreeMap<PathBuf,NativeCompilerInput>,
    generated:Vec<NativeGeneratedInput>,
}
fn native_emitted_libraries(args:&[String])->Result<Vec<PathBuf>> {
    let kinds=args.windows(2).filter(|pair|pair[0]=="--crate-type")
        .flat_map(|pair|pair[1].split(',')).collect::<Vec<_>>();
    let unique=kinds.iter().copied().collect::<std::collections::BTreeSet<_>>();
    if kinds.is_empty() || unique.len()!=kinds.len()
        || (kinds.len()>1 && unique!=std::collections::BTreeSet::from(["rlib","staticlib"])) {
        return Err("transitive compiler crate-type roster unsupported/duplicate".into());
    }
    let suffix=args.windows(2).filter(|pair|pair[0]=="-C")
        .filter_map(|pair|pair[1].strip_prefix("extra-filename=")).collect::<Vec<_>>();
    if suffix.len()!=1 || suffix[0].contains(['/', '\\']){return Err("transitive emitted library suffix differs".into());}
    let mut outputs=Vec::new();
    for kind in kinds {
        let extension=match kind {"lib"|"rlib"=>"rlib","proc-macro"|"dylib"|"cdylib"=>"dylib","staticlib"=>"a","bin"=>continue,
            _=>return Err("transitive compiler emitted library kind unsupported".into())};
        outputs.push(Path::new(native_argument(args,"--out-dir")?).join(format!("lib{}{}.{}",native_argument(args,"--crate-name")?,suffix[0],extension)));
    }
    Ok(outputs)
}
// This consumes the actual per-command .d files. Generated files are separate
// produced-node observations; a package prefix never invents a before hash.
fn native_compiler_sources(root:&Path,compiled:&NativeCompiled<'_>,whole:Option<Instant>)
    ->Result<NativeCompilerSources> {
    for (path,expected) in &compiled.role_context.inputs {
        native_capture_alive(whole)?;
        if file_binding(path)?!=*expected{return Err("Native selected source/metadata changed before transitive join".into());}
    }
    let owner=compiled.cap.io.sdk_owner_uid().map_err(|e|e.to_string())?;
    let mut inputs=BTreeMap::new();let mut generated=Vec::new();
    for args in &compiled.commands {
        native_capture_alive(whole)?;
        native_requires_link(args)?;
        let name=native_argument(args,"--crate-name")?;
        let source_args=args.iter().filter(|arg|!arg.contains('=')&&Path::new(arg).extension()==Some(std::ffi::OsStr::new("rs"))).collect::<Vec<_>>();
        if source_args.len()!=1{return Err("transitive compiler source argument absent/ambiguous".into());}
        let declared=Path::new(source_args[0]);
        let source=if declared.is_absolute(){declared.to_owned()}else if name=="ruycast_native" {
            root.join("rust-native").join(declared)
        }else{return Err("external transitive compiler cwd is not captured".into());};
        let source=fs::canonicalize(source).map_err(|e|e.to_string())?;
        let artifacts=compiled.artifacts.iter().filter(|artifact|!artifact.fresh&&artifact.source==source).collect::<Vec<_>>();
        if artifacts.len()!=1{return Err("transitive compiler source/package artifact absent or ambiguous".into());}
        let package_id=&artifacts[0].package_id;
        let (package,package_owner)=compiled.package_roots.get(package_id)
            .ok_or("transitive compiler package absent from genuine before metadata")?;
        if !source.starts_with(package){return Err("transitive compiler source leaves exact artifact package".into());}
        let expected=compiled.package_source_before.get(&source).ok_or("transitive compiler entry absent from before-package")?;
        let entry=compiler_input_bytes(&source,*package_owner,whole)?.0;
        if entry.sha256!=*expected{return Err("transitive compiler entry changed from before-package".into());}
        let suffix=args.windows(2).filter(|pair|pair[0]=="-C")
            .filter_map(|pair|pair[1].strip_prefix("extra-filename=")).collect::<Vec<_>>();
        if suffix.len()!=1 || suffix[0].contains(['/', '\\']){return Err("transitive compiler emitted suffix differs".into());}
        let dir=Path::new(native_argument(args,"--out-dir")?);
        if !dir.is_absolute()||!dir.starts_with(root.join("rust-native/target")) {return Err("transitive compiler output escaped target".into());}
        let dep=dir.join(format!("{name}{}.d",suffix[0]));
        let (dep_binding,bytes)=compiler_input_bytes(&dep,owner,whole)?;
        inputs.insert(dep.clone(),NativeCompilerInput{binding:dep_binding,owner,origin:"actual compiler dep-info"});
        for pair in args.windows(2).filter(|pair|pair[0]=="--extern") {
            let Some((alias,path))=pair[1].split_once('=')else{
                if pair[1]=="proc_macro"{continue;}
                return Err("unpathed transitive extern has no fixed sysroot role".into());
            };
            let path=PathBuf::from(path);
            if !path.is_absolute()||!path.starts_with(root.join("rust-native/target"))
                ||fs::canonicalize(&path).map_err(|e|e.to_string())?!=path{return Err("transitive extern escaped exact existing target".into());}
            let binding=compiler_input_bytes(&path,owner,whole)?.0;
            let linked=native_extern_link_path(root,&compiled.commands,alias,&path)?;
            let mut producers=Vec::new();
            for producer in &compiled.commands {
                if !native_emitted_libraries(producer)?.contains(&linked){continue;}
                let sources=producer.iter().filter(|arg|!arg.contains('=')&&Path::new(arg).extension()==Some(std::ffi::OsStr::new("rs"))).collect::<Vec<_>>();
                if sources.len()!=1{return Err("transitive extern producer source ambiguous".into());}
                let source=Path::new(sources[0]);
                let source=fs::canonicalize(if source.is_absolute(){source.to_owned()}else{
                    if native_argument(producer,"--crate-name")?!="ruycast_native"{return Err("transitive extern producer cwd unknown".into());}
                    root.join("rust-native").join(source)
                }).map_err(|e|e.to_string())?;
                let artifact=compiled.artifacts.iter().filter(|artifact|!artifact.fresh&&artifact.source==source).collect::<Vec<_>>();
                if artifact.len()!=1{return Err("transitive extern lacks unique actual fresh artifact".into());}
                let aliases=artifact[0].filenames.iter().filter(|file|file.extension()==path.extension()).collect::<Vec<_>>();
                if aliases.is_empty(){return Err("transitive actual artifact library output absent".into());}
                for alias in aliases {
                    if !alias.starts_with(root.join("rust-native/target"))
                        ||compiler_input_bytes(alias,owner,whole)?.0.sha256!=binding.sha256 {
                        return Err("transitive extern differs from actual emitted artifact bytes".into());
                    }
                }
                producers.push(producer);
            }
            if producers.len()!=1{return Err("transitive extern/proc-macro lacks unique same-build compiler producer".into());}
            if inputs.insert(path,NativeCompilerInput{binding:binding.clone(),owner,origin:"actual emitted extern/proc-macro"})
                .is_some_and(|old:NativeCompilerInput|old.binding.sha256!=binding.sha256||old.owner!=owner) {
                return Err("transitive extern changed across consumers".into());
            }
        }
        for path in dependency_paths(std::str::from_utf8(&bytes).map_err(|_|"transitive compiler DI UTF8")?)? {
            native_capture_alive(whole)?;
            let path=fs::canonicalize(if path.is_absolute(){path}else{package.join(path)}).map_err(|e|e.to_string())?;
            let (binding,read_owner,origin)=if let Some(expected)=compiled.package_source_before.get(&path) {
                let expected_owner=compiled.package_roots.values().filter(|(dir,_)|path.starts_with(dir))
                    .max_by_key(|(dir,_)|dir.components().count()).map(|(_,owner)|*owner)
                    .ok_or("transitive input original package owner absent")?;
                let binding=compiler_input_bytes(&path,expected_owner,whole)?.0;
                if binding.sha256!=*expected{return Err("actual compiler DI source changed from genuine before-package".into());}
                (binding,expected_owner,"genuine package-before")
            }else if let Some(expected)=path.strip_prefix(root).ok().and_then(|relative|compiled.before.files.get(relative)) {
                // include_str!/include_bytes! may read a shared repository
                // input outside the crate directory, e.g. config/default.json.
                // Only an exact genuine precompile inventory member qualifies.
                let binding=compiler_input_bytes(&path,owner,whole)?.0;
                if binding.sha256!=*expected{return Err("actual shared repository DI source changed from genuine before".into());}
                (binding,owner,"genuine owned repository-before")
            }else{
                let producers=compiled.produced_roots.values().filter(|produced|produced.package_id==*package_id
                    && path.starts_with(&produced.out_dir)).collect::<Vec<_>>();
                if producers.len()!=1{return Err(format!("actual generated DI input has no unique same-build producer: {} (package {}, matches {}, observed roots {:?})",path.display(),package_id,producers.len(),compiled.produced_roots.keys().collect::<Vec<_>>()));}
                let binding=compiler_input_bytes(&path,owner,whole)?.0;
                generated.push(NativeGeneratedInput{binding:binding.clone(),package_id:package_id.clone(),
                    producer_root:producers[0].out_dir.clone(),consumer_arguments:args.clone()});
                (binding,owner,"same-call produced OUT_DIR")
            };
            if inputs.insert(path,NativeCompilerInput{binding:binding.clone(),owner:read_owner,origin})
                .is_some_and(|old:NativeCompilerInput|old.binding.sha256!=binding.sha256||old.owner!=read_owner) {
                return Err("actual compiler DI aliases changed bytes".into());
            }
            if inputs.len()>100000 || generated.len()>100000{return Err("transitive compiler input bound".into());}
        }
    }
    native_capture_alive(whole)?;
    Ok(NativeCompilerSources{inputs,generated})
}

struct NativeCompiled<'a> {
    cap: &'a NativeCompileCap,
    compiler: Binding,
    cargo: Binding,
    data_dependencies: BTreeMap<String,Binding>,
    package_source_before: BTreeMap<PathBuf,String>,
    package_roots: BTreeMap<String,(PathBuf,u32)>,
    produced_roots: BTreeMap<PathBuf,NativeProducedRoot>,
    artifacts: Vec<NativeArtifact>,
    rustc_inputs:DataRustcInputs,
    executables:Vec<CompilerInputCap>,
    linker:DataLinkerInputs,
    before: Inventory,
    artifact: PathBuf,
    commands: Vec<Vec<String>>,
    objc: (PathBuf,BTreeMap<PathBuf,String>),
    role_context: NativeRoleContext,
    target: String,
    began: Instant,
    foundation_pins: String,
    foreign_inputs: Vec<Binding>,
}
fn native_stage_with_deadline(root:&Path,out:&Path,cap:&NativeCompileCap,whole:Option<Instant>)->Result<()> {
    let compiled=native_compile_with_deadline(root,out,cap,whole)?;
    let compiler_sources=native_compiler_sources(root,&compiled,whole)?;
    let NativeCompiled{cap,compiler,cargo,data_dependencies,package_source_before,package_roots,produced_roots,artifacts,rustc_inputs,executables,linker,before,artifact,commands,objc,role_context,target,began,foundation_pins,foreign_inputs}=compiled;
    let root=&before.root;
    let native_inputs=|value:&mut Inventory| {
        value.files.retain(|path,_|path.starts_with("rust-native")||path.starts_with("rust")
            ||path.starts_with("config")||path.starts_with("assets")||path.starts_with(".cargo")
            ||path==Path::new("rust-toolchain.toml"));
    };
    let compiler_capture = native_capture_closure(root,out,&artifact,&commands,&objc,role_context,whole)?;
    let mut final_source = inventory(root)?;
    native_inputs(&mut final_source);
    native_capture_alive(whole)?;
    if final_source.files != before.files || native_objc_inputs(&objc.0)? != objc.1 {
        return Err("source changed during retained Native diagnostic capture".into());
    }
    cap.verify()?;
    verify_data_compiler(&compiler)?;
    cargo.verify()?;
    for dependency in data_dependencies.values(){dependency.verify()?;}
    verify_data_rustc_inputs(&rustc_inputs,whole)?;
    for executable in &executables{verify_compiler_input_cap(executable,whole)?;}
    verify_data_linker_inputs(&linker,whole)?;
    for input in compiler_sources.inputs.values() {
        native_capture_alive(whole)?;
        if compiler_input_bytes(&input.binding.path,input.owner,whole)?.0.sha256!=input.binding.sha256 {
            return Err("transitive compiler input changed during diagnostic capture".into());
        }
    }
    for produced in produced_roots.values() {
        if compiler_input_bytes(&produced.producer.path,produced.invocation.identity.2,whole)?.0.sha256!=produced.producer.sha256 {
            return Err("actual build-script executable changed after capture".into());
        }
        for expected in [&produced.invocation,&produced.root_output] {
            let Some((actual,_))=native_trace_file(&expected.binding.path,expected.identity.2,whole)? else {
                return Err("actual produced trace disappeared after capture".into());
            };
            if actual.identity!=expected.identity || actual.binding.sha256!=expected.binding.sha256 {
                return Err("actual produced post-Cargo trace changed during diagnostic capture".into());
            }
        }
    }
    for (path,expected) in &package_source_before {
        native_capture_alive(whole)?;
        let owner=package_roots.values().filter(|(directory,_)|path.starts_with(directory))
            .max_by_key(|(directory,_)|directory.components().count()).map(|(_,owner)|*owner)
            .ok_or("compiler source has no original package owner")?;
        if compiler_input_bytes(path,owner,whole)?.0.sha256!=*expected {
            return Err("compiler package source changed during Native capture".into());
        }
    }
    let owner=cap.io.sdk_owner_uid().map_err(|e|e.to_string())?;
    if data_declaration_pins(DataCase::Foundation,&foreign_inputs,owner,Some(&foundation_pins))?
        .as_deref()!=Some(foundation_pins.as_str()) {
        return Err("Native Foundation declarations changed after diagnostic capture".into());
    }
    let mut tool = Command::new("/usr/bin/dyld_info");
    tool.arg("-exports").arg(&artifact);
    let exports = native_tool_with_deadline(&mut tool,root,whole)?;
    native_private_bytes(&out.join("native-exports.txt"), &exports.stdout)?;
    native_private_bytes(&out.join("native-exports.stderr"), &exports.stderr)?;
    if !exports.status.success() {
        return Err("Native static export inspection failed".into());
    }
    let text = String::from_utf8(exports.stdout).map_err(|e| e.to_string())?;
    let symbols: Vec<_> = text
        .lines()
        .filter(|line| line.split_whitespace().last() == Some("_ruycast_owned_host_bind_v1"))
        .collect();
    if symbols.len() != 1 {
        return Err("Native must export exactly one owned Host sink".into());
    }
    let offset = symbols[0]
        .split_whitespace()
        .find_map(|v| {
            v.strip_prefix("0x")
                .and_then(|v| usize::from_str_radix(v, 16).ok())
        })
        .ok_or("Native export offset missing")?;
    native_private_bytes(
        &out.join("source-before.json"),
        &serde_json::to_vec_pretty(&before).map_err(|e| e.to_string())?,
    )?;
    native_capture_alive(whole)?;
    native_private_bytes(
        &out.join("receipt.json"),
        &serde_json::to_vec_pretty(&json!({"scope":"Native compile/hash/export preparation only; no endpoint or SDK execution","artifact":file_binding(&artifact)?,"compiler_capture":compiler_capture,"foundation_declaration_pins":foundation_pins,"foundation_foreign_inputs":foreign_inputs,"data_dependencies":data_dependencies,"package_source_before":package_source_before,"package_roots":package_roots,"produced_roots":produced_roots,"transitive_compiler_sources":compiler_sources,"actual_artifacts":artifacts,"rustc_inputs":rustc_inputs,"source_manifest_files":before.files.len(),"source_manifest_sha256":sha(&fs::read(out.join("source-before.json")).map_err(|e|e.to_string())?),"compile_pin":cap.pin,"native_sink_image_offset":offset,"compiler_host_target":target,"optimized":true,"wall_ns":began.elapsed().as_nanos(),"whole_budget_seconds":whole.map(|_|REFERENCE_CAPTURE_SECONDS),"native_cap_held_through_build":true,"cap_lifetime":"ends when this preparation command returns; not reusable or runnable authority","sdk_built":false,"final_grants_published":false,"source_routes_qualified":false,"paired_session_model":"one fresh cap set per arm for the entire paired Worker session; never resend into an EOF-consumed FIFO"})).map_err(|e|e.to_string())?,
    )?;
    native_capture_alive(whole)
}

fn native_compile_with_deadline<'a>(root:&Path,out:&Path,cap:&'a NativeCompileCap,whole:Option<Instant>)->Result<NativeCompiled<'a>> {
    native_capture_alive(whole)?;
    use std::os::unix::fs::PermissionsExt;
    let native_inputs = |value: &mut Inventory| {
        value.files.retain(|path, _| {
            path.starts_with("rust-native")
                || path.starts_with("rust")
                || path.starts_with("config")
                || path.starts_with("assets")
                || path.starts_with(".cargo")
                || path == Path::new("rust-toolchain.toml")
        });
    };
    let mut before = inventory(root)?;
    native_inputs(&mut before);
    native_capture_alive(whole)?;
    let root = &before.root;
    if !out.is_absolute() || !out.starts_with(root.join("rust-native/target")) {
        return Err(
            "Native preparation output must be inside the existing task Native target".into(),
        );
    }
    let parent = out.parent().ok_or("Native output parent missing")?;
    if fs::canonicalize(parent).map_err(|e| e.to_string())? != parent
        || out.components().any(|part| {
            !matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err("Native output parent must be exact, without links or traversal".into());
    }
    space_for_build(root)?;
    fs::create_dir(out).map_err(|e| e.to_string())?;
    fs::set_permissions(out, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    native_private_bytes(&out.join("native-raw-source-before.json"),&serde_json::to_vec_pretty(&before).map_err(|e|e.to_string())?)?;
    let pin = serde_json::to_string(&cap.pin).map_err(|e| e.to_string())?;
    let target_dir = root.join("rust-native/target");
    let (cargo, toolchain) = native_toolchain(root,out,whole)?;
    let hosts = toolchain["rustc"]["version"]
        .as_str()
        .ok_or("compiler version absent")?
        .lines()
        .filter_map(|s| s.strip_prefix("host: "))
        .collect::<Vec<_>>();
    if hosts.len() != 1
        || hosts[0].is_empty()
        || !hosts[0]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("captured compiler host target invalid/ambiguous".into());
    }
    let target = hosts[0].to_owned();
    let NativePackages{objc,source_before:package_source_before,roots:package_roots}=
        native_objc_package(root,out,&cargo,whole)?;
    let foundation_before=nsstring_compile_inputs_with_deadline(root,&cargo,cap.io.sdk_owner_uid().map_err(|e|e.to_string())?,whole)?;
    native_private_bytes(&out.join("native-raw-foundation-before.json"),&serde_json::to_vec_pretty(&json!({
        "package":foundation_before.package_identity,"features":foundation_before.features,
        "source_before":foundation_before.source_before,"sdk_root":foundation_before.sdk_root,
        "header":{"path":foundation_before.header.path,"sha256":foundation_before.header.sha256},"owner":foundation_before.owner
    })).map_err(|e|e.to_string())?)?;
    let compiler = Path::new(
        toolchain["rustc"]["binary"]["path"]
            .as_str()
            .ok_or("captured rustc path absent")?,
    );
    if !toolchain["rustc"]["version"].as_str().is_some_and(|version|version.starts_with("rustc 1.92.0 ")) {
        return Err("Data/compiler source policy requires the bound rustc 1.92.0 version".into());
    }
    if !toolchain["cargo"]["version"].as_str().is_some_and(|version|version.starts_with("cargo 1.92.0 ")) {
        return Err("Cargo run-trace layout requires the bound 1.92.0 version".into());
    }
    let trace_before=native_trace_before(&target_dir,&target,
        cap.io.sdk_owner_uid().map_err(|e|e.to_string())?,whole)?;
    let compiler_binding=Binding{path:compiler.to_owned(),sha256:toolchain["rustc"]["binary"]["sha256"]
        .as_str().ok_or("captured compiler digest absent")?.to_owned()};
    use std::os::unix::fs::MetadataExt;
    let mut executables=Vec::new();
    for (path,expected) in [(compiler,&toolchain["rustc"]["binary"]), (cargo.as_path(),&toolchain["cargo"]["binary"])] {
        let owner=fs::symlink_metadata(path).map_err(|e|e.to_string())?.uid();
        if ![0,cap.io.sdk_owner_uid().map_err(|e|e.to_string())?].contains(&owner){return Err("compiler executable original owner differs".into());}
        let held=compiler_input_cap(path,owner,whole)?.0;
        if held.binding.sha256!=expected["sha256"] {return Err("compiler executable differs from before selection".into());}
        executables.push(held);
    }
    let rustc_inputs=data_rustc_inputs(root,out,&compiler_binding,&target,
        cap.io.sdk_owner_uid().map_err(|e|e.to_string())?,whole)?;
    let linker=data_linker_inputs(root,out,&foundation_before.sdk_root,whole)?;
    let rustflags=linker.rustflags();
    let mut command = Command::new(&cargo);
    command
        .current_dir(root)
        .args([
            "rustc",
            "-v",
            "--message-format=json",
            "--locked",
            "--offline",
            "--release",
            "--features",
            "test-tools",
            "--bin",
            "ruycast-native",
            "--manifest-path",
        ])
        .arg(root.join("rust-native/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--target")
        .arg(&target)
        .args(["--", "-C", "save-temps=yes"])
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_PROFILE_RELEASE_DEBUG", "2")
        .env("CARGO_PROFILE_RELEASE_STRIP", "none")
        .env_remove("RUSTFLAGS")
        .env("CARGO_ENCODED_RUSTFLAGS",rustflags.join("\u{1f}"))
        .env("DEVELOPER_DIR",&linker.developer)
        .env("SDKROOT",&linker.sdk)
        .env("RUSTC", compiler)
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env_remove("UNSAFE_OBJC2_ALLOW_CLASS_OVERRIDE")
        .env("RUYCAST_F1_NATIVE_GRANT_PIN", &pin);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("DYLD_") {
            command.env_remove(key);
        }
    }
    let cargo_arguments = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|v| {
            v.to_str()
                .map(str::to_owned)
                .ok_or_else(|| "Native compiler argv UTF8 differs".to_string())
        })
        .collect::<Result<Vec<_>>>()?;
    native_private_bytes(&out.join("native-build-command.json"),&serde_json::to_vec_pretty(&json!({"arguments":cargo_arguments,"working_directory":root,"producer":file_binding(&std::env::current_exe().map_err(|e|e.to_string())?)?,"cargo_trace_before":&trace_before,"executable_readcaps":&executables,"linker_inputs":&linker,"declared_controls":{"debug_info":2,"strip":"none","save_temps":true,"incremental":false,"encoded_rustflags":rustflags,"ambient_rustflags_removed":true,"whole_budget_seconds":whole.map(|_|REFERENCE_CAPTURE_SECONDS),"per_tool_seconds_max":900,"rustc_wrapper_disabled":true,"rustc_workspace_wrapper_disabled":true,"objc2_class_override_disabled":true},"ambient_environment_values_recorded":false})).map_err(|e|e.to_string())?)?;
    let began = Instant::now();
    let result = native_tool_with_deadline(&mut command,root,whole)?;
    {
        use std::os::unix::process::ExitStatusExt;
        native_private_bytes(&out.join("native-build-result.json"),&serde_json::to_vec_pretty(&json!({
            "exit_code":result.status.code(),"signal":result.status.signal(),"success":result.status.success(),
            "stdout_bytes":result.stdout.len(),"stderr_bytes":result.stderr.len(),"runtime_execution":false
        })).map_err(|e|e.to_string())?)?;
    }
    let parsed_commands=std::str::from_utf8(&result.stderr).map_err(|_|"Cargo command display UTF8".to_owned())
        .and_then(native_rustc_commands);
    if let Ok(commands)=&parsed_commands {
        native_private_bytes(&out.join("native-raw-rustc-arguments.json"),&serde_json::to_vec_pretty(commands).map_err(|e|e.to_string())?)?;
    }
    let summary = native_cargo_summary(&result.stdout, &result.stderr)?;
    native_private_bytes(
        &out.join("native-compiler.stdout"),
        &serde_json::to_vec_pretty(&summary).map_err(|e| e.to_string())?,
    )?;
    native_private_bytes(
        &out.join("native-compiler.stderr"),
        b"Cargo-vv environment display omitted; exact rustc argv captured separately.\n",
    )?;
    if !result.status.success() {
        return Err("Native compile failed; diagnostics retained, no SDK loaded".into());
    }
    let commands = parsed_commands?;
    for args in &commands {
        let path = fs::canonicalize(&args[0]).map_err(|e| e.to_string())?;
        if path != compiler || file_binding(&path)? != toolchain["rustc"]["binary"] {
            return Err("actual rustc invocation differs from before-build compiler pin".into());
        }
    }
    let mut native_foundation=None;
    let mut native_consumers=0;
    for args in &commands {
        if native_argument(args,"--crate-name")? != "ruycast_native" { continue; }
        let selected=args.windows(2).filter(|v|v[0]=="--extern")
            .filter_map(|v|v[1].strip_prefix("objc2_foundation=")).collect::<Vec<_>>();
        if selected.len()!=1 { return Err("Native Input5 Foundation extern absent or ambiguous".into()); }
        let selected=fs::canonicalize(selected[0]).map_err(|e|e.to_string())?;
        let path=native_extern_link_path(root,&commands,"objc2_foundation",&selected)?;
        if native_foundation.as_ref().is_some_and(|previous|previous!=&path) {
            return Err("Native Input5 bin/lib select different Foundation outputs".into());
        }
        native_foundation=Some(path); native_consumers+=1;
    }
    if native_consumers!=2 { return Err("Native Input5 requires both actual bin/lib Foundation consumers".into()); }
    let foundation_receipt=out.join("native-foundation-compiler");
    fs::create_dir(&foundation_receipt).map_err(|e|e.to_string())?;
    fs::set_permissions(&foundation_receipt,fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
    let foundation_pins=nsstring_compile_pins(root,&foundation_receipt,&foundation_before,&commands,
        native_foundation.as_deref().ok_or("Native Input5 Foundation output absent")?,compiler,true)?;
    let foreign_inputs=vec![foundation_before.header.clone(),
        declaration_file(&foundation_before.package.join("src/generated/NSString.rs"),foundation_before.owner)?];
    if file_binding(&cargo)? != toolchain["cargo"]["binary"] {
        return Err("Cargo executable changed while compiling Native".into());
    }
    space_for_build(root)?;
    cap.verify()?;
    let mut after = inventory(root)?;
    native_inputs(&mut after);
    native_capture_alive(whole)?;
    if after.files != before.files {
        return Err("source changed while compiling Native Host".into());
    }
    let artifact = out.join("native-host");
    fs::copy(
        target_dir.join(&target).join("release/ruycast-native"),
        &artifact,
    )
    .map_err(|e| e.to_string())?;
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o500)).map_err(|e| e.to_string())?;
    let bytes = fs::read(&artifact).map_err(|e| e.to_string())?;
    if bytes.len() < 32
        || bytes[..4] != [0xcf, 0xfa, 0xed, 0xfe]
        || u32::from_le_bytes(bytes[12..16].try_into().map_err(|_| "Mach-O type")?) != 2
        || !bytes.windows(pin.len()).any(|part| part == pin.as_bytes())
    {
        return Err("compiled Native image/type/private-pin binding missing".into());
    }
    host_image_preflight(&bytes, 2, 0)?;
    let role_context=native_role_context(root,&commands,&objc,&before,whole)?;
    let data_dependencies=native_data_dependencies(root,&commands,&role_context)?;
    let artifacts=native_artifacts(&result.stdout,whole)?;
    let produced_roots=native_produced_roots(root,(&result.stdout,&result.stderr),&commands,&package_roots,&trace_before,
        cap.io.sdk_owner_uid().map_err(|e|e.to_string())?,whole)?;
    // Preserve the actual joined nodes before a later transitive DI failure;
    // never persist Cargo's surrounding environment assignments for diagnosis.
    native_private_bytes(&out.join("native-produced-roots.json"),&serde_json::to_vec_pretty(&produced_roots).map_err(|e|e.to_string())?)?;
    native_capture_alive(whole)?;
    cap.verify()?;
    // Copy only the expected before-build bindings from this live toolchain
    // result, after every actual compiler argv and Cargo hash has matched it.
    let compiler=compiler_binding;
    let cargo=Binding{path:cargo,sha256:toolchain["cargo"]["binary"]["sha256"]
        .as_str().ok_or("captured Cargo digest absent")?.to_owned()};
    for executable in &executables{verify_compiler_input_cap(executable,whole)?;}
    verify_data_linker_arguments(&commands,&linker)?;
    verify_data_linker_inputs(&linker,whole)?;
    Ok(NativeCompiled{cap,compiler,cargo,data_dependencies,package_source_before,package_roots,produced_roots,artifacts,rustc_inputs,executables,linker,before,artifact,commands,objc,role_context,target,began,foundation_pins,foreign_inputs})
}

fn rust_path(path: &Path) -> Result<String> {
    serde_json::to_string(path.to_str().ok_or("Host source path must be UTF8")?)
        .map_err(|e| e.to_string())
}
/// Pure data preparation for both arms, never a product/SDK launch. The single
/// fixture manifest is what Authority.fixture and NativeGrant.fixture_sha256
/// must bind; it is not an independent collection authority.
pub fn complete_fixture(out:&Path)->Result<()> {
    let checkout=checkout_root()?;
    let measurement=checkout.as_path();
    let target=measurement.join("rust-native/target");
    if out.exists() || out.parent()!=Some(target.as_path()) || fs::canonicalize(&target).map_err(|e|e.to_string())?!=target || out.components().any(|part|matches!(part,std::path::Component::ParentDir|std::path::Component::CurDir)) {return Err("new fixture must be a direct child of existing measurement target".into())}
    space_for_build(measurement)?;
    let epoch=i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e|e.to_string())?.as_secs()).map_err(|_|"fixture epoch overflow")?;
    write_complete_fixture(measurement,out,epoch)
}
fn write_complete_fixture(measurement:&Path,out:&Path,epoch:i64)->Result<()> {
    use std::{io::Write,os::unix::fs::{OpenOptionsExt,PermissionsExt}};
    use ruycast_native::native_f1::fixture;
    fs::create_dir(out).map_err(|e|e.to_string())?;
    fs::set_permissions(out,fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
    let defaults=measurement.join("config/default.json");
    let base:Value=serde_json::from_slice(&fs::read(&defaults).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let mut files=BTreeMap::new();
    let mut put=|name:&str,bytes:&[u8]|->Result<()>{
        let path=out.join(name);
        let mut file=fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_CLOEXEC|libc::O_NOFOLLOW).open(&path).map_err(|e|e.to_string())?;
        file.write_all(bytes).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?;
        fs::set_permissions(&path,fs::Permissions::from_mode(0o400)).map_err(|e|e.to_string())?;
        files.insert(name.to_owned(),file_binding(&path)?);Ok(())
    };
    for size in ruycast_native::native_f1::SIZES {put(&format!("config-catalog-{size}.json"),&serde_json::to_vec_pretty(&fixture::configuration(&base,size)?).map_err(|e|e.to_string())?)?;}
    let mut unicode=base.clone();unicode["commands"]=json!(fixture::unicode_catalog());
    put("config-unicode.json",&serde_json::to_vec_pretty(&unicode).map_err(|e|e.to_string())?)?;
    put("clipboard-200.json",&serde_json::to_vec_pretty(&fixture::history(epoch)?).map_err(|e|e.to_string())?)?;
    put("search-ranking-512.json",&serde_json::to_vec_pretty(&fixture::recency(epoch)).map_err(|e|e.to_string())?)?;
    put("process-commands-1000.json",&serde_json::to_vec_pretty(&fixture::process_commands()).map_err(|e|e.to_string())?)?;
    put("image-1200x800.rgba",&fixture::image_rgba())?;
    let source=measurement.join("rust-native/src/native_f1.rs");
    let manifest=json!({"schema":"ruycast.f1.fixture.v1","epoch_unix_seconds":epoch,"fixture_source":file_binding(&source)?,"original_workloads":original_workloads_binding(measurement),"default_configuration":file_binding(&defaults)?,"files":files,
        "catalog_sizes":[500,1000,5000,10000],"catalog_cached_query":"m","recency_search_query":"pro das","recency_entries":512,
        "clipboard_count":200,"clipboard_text_definition":"Fixture i: + native clipboard Ω repeated400; preserve originalUTF8bytes","clipboard_settings":{"enabled":true,"limit":200,"maxTextBytes":100000,"retentionDays":30,"excludedApps":[]},
        "process_count":1000,"process_cached_query":"","image":{"width":1200,"height":800,"channels":4,"format":"RGBA8","seed":42,"lcg_multiplier":1664525,"lcg_increment":1013904223},
        "not_qualified":["full14SDK","all157productionhandlers","cold_fresh_launches","independent_collection_authority"],"source_routes_qualified":false});
    write_new(&out.join("fixture.json"),&manifest)?;
    fs::set_permissions(out.join("fixture.json"),fs::Permissions::from_mode(0o400)).map_err(|e|e.to_string())?;
    Ok(())
}

fn fingerprint_source(value: crate::host_io::Fingerprint) -> String {
    format!(
        "sdk::core_preentry::Fingerprint{{device:{},inode:{},kind:{},access:{}}}",
        value.device, value.inode, value.kind, value.access
    )
}

/// Read the actual source-owned workload manifest through one bounded readcap.
/// An epoch is never a free environment/grant scalar or inferred from a clock.
fn fixture_epoch_binding(path:&Path,owner:u32)->Result<(Binding,u64)> {
    use std::{io::Read,os::unix::fs::{MetadataExt,OpenOptionsExt}};
    let pinned=declaration_file(path,owner)?;
    let file=fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC|libc::O_NONBLOCK)
        .open(path).map_err(|e|e.to_string())?;
    let before=file.metadata().map_err(|e|e.to_string())?;
    let identity=|v:&fs::Metadata|(v.dev(),v.ino(),v.uid(),v.mode(),v.nlink(),v.len(),v.mtime(),v.mtime_nsec(),v.ctime(),v.ctime_nsec());
    if !before.is_file() || before.uid()!=owner || before.nlink()!=1 || before.mode()&0o022!=0 {
        return Err("fixture manifest readcap ownership/type differs".into());
    }
    let mut bytes=Vec::new();
    (&file).take(2*1024*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    if bytes.len()>2*1024*1024 || sha(&bytes)!=pinned.sha256
        || identity(&before)!=identity(&file.metadata().map_err(|e|e.to_string())?)
        || identity(&before)!=identity(&fs::symlink_metadata(path).map_err(|e|e.to_string())?) {
        return Err("fixture manifest changed across epoch read".into());
    }
    #[derive(serde::Deserialize)]
    struct Epoch {schema:String,epoch_unix_seconds:u64}
    let manifest:Epoch=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
    if manifest.schema!="ruycast.f1.fixture.v1" {return Err("fixture epoch schema differs".into());}
    Ok((pinned,manifest.epoch_unix_seconds))
}

/// Plain deterministic SDK environment, separate from original F1 workloads.
fn reference_actor_environment(root:&Path,roots:&[PathBuf;4],fixture:&Binding,owner:u32,epoch:u64)->Result<(BTreeMap<&'static str,Binding>,[Binding;2])> {
    use std::os::unix::fs::PermissionsExt;
    let (manifest_pin,bytes)=declaration_bytes(&fixture.path,owner)?;
    if manifest_pin.sha256!=fixture.sha256 {return Err("Reference actor fixture manifest changed".into());}
    let manifest:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
    if manifest["schema"]!="ruycast.f1.fixture.v1" || manifest["epoch_unix_seconds"].as_u64()!=Some(epoch) {
        return Err("Reference actor fixture schema/epoch differs".into());
    }
    let history_pin:Binding=serde_json::from_value(manifest["files"]["clipboard-200.json"].clone()).map_err(|e|e.to_string())?;
    if history_pin.path!=fixture.path.with_file_name("clipboard-200.json") || !history_pin.path.starts_with(root) {
        return Err("Reference actor history is not the exact owned fixture member".into());
    }
    let (actual,history)=declaration_bytes(&history_pin.path,owner)?;
    if actual.sha256!=history_pin.sha256
        || serde_json::from_slice::<Value>(&history).map_err(|e|e.to_string())?.as_array().is_none_or(|v|v.len()!=200) {
        return Err("Reference actor history bytes/count differ".into());
    }
    let (icon_pin,icon)=declaration_bytes(&root.join("assets/app-icon.png"),owner)?;
    if !icon.starts_with(b"\x89PNG\r\n\x1a\n") {return Err("Reference actor icon is not actual PNG data".into());}
    let bundle=roots[3].join("Fixture.app");
    let contents=bundle.join("Contents");
    let metadata=roots[0].join("actor-inputs");
    for directory in [&bundle,&contents,&contents.join("MacOS"),&contents.join("Resources"),&metadata] {
        fs::create_dir(directory).map_err(|e|e.to_string())?;
        fs::set_permissions(directory,fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
    }
    let info=b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>org.ruycast.f1.fixture</string><key>CFBundleName</key><string>Fixture</string><key>CFBundleDisplayName</key><string>Fixture</string><key>CFBundleExecutable</key><string>Fixture</string><key>CFBundleIconFile</key><string>Fixture.png</string></dict></plist>";
    let mut files=BTreeMap::new();
    for (name,path,data) in [
        ("info",contents.join("Info.plist"),info.as_slice()),
        ("executable",contents.join("MacOS/Fixture"),b"owned fixture metadata\n".as_slice()),
        ("icon",contents.join("Resources/Fixture.png"),icon.as_slice()),
        ("history",metadata.join("clipboard-200.json"),history.as_slice()),
    ] {
        native_private_bytes(&path,data)?;
        fs::set_permissions(&path,fs::Permissions::from_mode(0o400)).map_err(|e|e.to_string())?;
        files.insert(name,declaration_file(&path,owner)?);
    }
    Ok((files,[actual,icon_pin]))
}

/// Actual two-image construction; deliberately never loads either image. The
/// one-shot pipes are held across both builds and EOF publication. Returning
/// ends that authority: these receipts are not a later runnable Host session.
pub fn owned_host_images(root: &Path, fixture_manifest: &Path, out: &Path) -> Result<()> {
    let cap = NativeCompileCap::new()?;
    let fixture_owner=cap.io.native_owner_uid().map_err(|e|e.to_string())?;
    let (workload_fixture,_fixture_epoch)=fixture_epoch_binding(fixture_manifest,fixture_owner)?;
    native_stage(root, out, &cap)?;
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let endpoint = out.join("native-host");
    let endpoint_sha=sha(&fs::read(&endpoint).map_err(|e|e.to_string())?);
    let sdk_artifact = out.join("libRuycastOwnedHost.dylib");
    let initializer = out.join("owned-host-initializer.rs");
    let manifest_path = out.join("host-source-manifest.json");
    let fixture = out.join("owned-fixture");
    for dir in ["home", "cache", "temp", "applications", "profile"] {
        fs::create_dir_all(fixture.join(dir)).map_err(|e| e.to_string())?;
    }
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    fs::set_permissions(&fixture, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let identity = fs::metadata(fixture.join("home")).map_err(|e| e.to_string())?;
    let sdk_nonce = {
        use std::io::Read;
        let mut bytes = [0u8; 32];
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|e| e.to_string())?;
        sha(&bytes)
    };
    let (grant, stdio) = cap.io.sdk_pins().map_err(|e| e.to_string())?;
    let (native_fd, native_pin) = cap.io.native_pin().map_err(|e| e.to_string())?;
    let sdk_owner = cap.io.sdk_owner_uid().map_err(|e| e.to_string())?;
    let native_owner = cap.io.native_owner_uid().map_err(|e| e.to_string())?;
    if native_owner!=fixture_owner {return Err("Native fixture owner changed during preparation".into());}
    let mut source = String::from(
        "// Source-owned constructor. Source routes remain closed; never load this build.\n",
    );
    for (name, path) in [
        ("sdk", "sdk/mod.rs"),
        ("workspace", "sdk/workspace.rs"),
        ("guardian", "guardian.rs"),
        ("guardian_alive", "guardian_alive.rs"),
    ] {
        source.push_str(&format!(
            "#[path={}] pub mod {name};\n",
            rust_path(&root.join("bench/f1-runner/src").join(path))?
        ));
    }
    source.push_str(&format!(r#"
fn make_pins() -> sdk::core_preentry::HostPins<'static> {{
    sdk::core_preentry::HostPins {{ root:std::path::Path::new({root}), endpoint:std::path::Path::new({endpoint}),
        endpoint_sha256:{endpoint_sha}, sdk_artifact:std::path::Path::new({sdk}), nonce_sha256:{nonce_sha},
        owner_uid:{sdk_owner}, grant:{grant}, stdio:[{stdin},{stdout},{stderr}],
        source_manifest:std::path::Path::new({manifest}), initializer_source:std::path::Path::new({initializer}),
        native_grant_fd:{native_fd}, native_grant:{native_pin} }}
}}
// Existing concrete bootstrap route only. The complete fourteen-scope assembler
// is still missing and Registry/source gates must refuse this candidate.
fn finish_host(admission:sdk::core_preentry::HostAdmission) -> Result<(),String> {{
    use sdk::owned::OwnedRoot;
    let home=OwnedRoot::open(std::path::Path::new({home}))?;
    let account=sdk::bootstrap::Account::fixture({uid},{gid},"f1-owned","Owned Fixture",home.path())?;
    let context=sdk::bootstrap::Bootstrap::new(home,
        OwnedRoot::open(std::path::Path::new({cache}))?,OwnedRoot::open(std::path::Path::new({temp}))?,
        OwnedRoot::open(std::path::Path::new({apps}))?,account,admission.life())?;
    let module=admission.initialize_bootstrap(context)?;
    let mut registry=sdk::Registry::new(admission.life());
    registry.register(module)?;
    sdk::publish_registry(registry)?;
    admission.push_profile(std::path::Path::new({profile}),None)
}}
full_owned_host_initializer!(make_pins,finish_host);
// Retain these exact typed source functions for static pin/code inspection.
// Neither anchor invokes a function or adds another initializer.
#[used]
#[link_section="__DATA_CONST,__const"]
static HOST_PIN_MAKER: fn()->sdk::core_preentry::HostPins<'static> = make_pins;
#[used]
#[link_section="__DATA_CONST,__const"]
static HOST_FINISH: fn(sdk::core_preentry::HostAdmission)->Result<(),String> = finish_host;
"#,
        root=rust_path(&root)?,endpoint=rust_path(&endpoint)?,endpoint_sha=serde_json::to_string(&endpoint_sha).map_err(|e|e.to_string())?,
        sdk=rust_path(&sdk_artifact)?,nonce_sha=serde_json::to_string(&sha(sdk_nonce.as_bytes())).map_err(|e|e.to_string())?,
        grant=fingerprint_source(grant),stdin=fingerprint_source(stdio[0]),stdout=fingerprint_source(stdio[1]),stderr=fingerprint_source(stdio[2]),
        manifest=rust_path(&manifest_path)?,initializer=rust_path(&initializer)?,native_pin=fingerprint_source(native_pin),
        home=rust_path(&fixture.join("home"))?,cache=rust_path(&fixture.join("cache"))?,temp=rust_path(&fixture.join("temp"))?,apps=rust_path(&fixture.join("applications"))?,profile=rust_path(&fixture.join("profile"))?,uid=identity.uid(),gid=identity.gid(),
    ));
    fs::write(&initializer, source).map_err(|e| e.to_string())?;
    let before = inventory(&root)?;
    let mut sources = before
        .files
        .iter()
        .filter(|(path, _)| {
            [
                "rust-native",
                "rust",
                "bench/f1-runner",
                ".cargo",
                "config",
                "assets",
            ]
            .iter()
            .any(|p| path.starts_with(p))
                || path.as_path() == Path::new("rust-toolchain.toml")
        })
        .map(|(path, hash)| json!({"path":root.join(path),"sha256":hash}))
        .collect::<Vec<_>>();
    sources.push(file_binding(&initializer)?);
    sources.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    write_new(&manifest_path, &json!({"sources":sources}))?;
    let mut source_set = Vec::new();
    for input in &sources {
        source_set.extend_from_slice(input["path"].as_str().ok_or("source path")?.as_bytes());
        source_set.push(0);
        source_set.extend_from_slice(input["sha256"].as_str().ok_or("source hash")?.as_bytes());
        source_set.push(b'\n');
    }
    let target = host_compiler_target(&root)?;
    let target_dir = root.join("rust-native/target");
    let toolchain: Value = super::read(&out.join("native-toolchain.json"))?;
    let cargo = Path::new(toolchain["cargo"]["binary"]["path"].as_str().ok_or("selected Cargo path")?);
    let rustc = Path::new(toolchain["rustc"]["binary"]["path"].as_str().ok_or("selected rustc path")?);
    if file_binding(cargo)? != toolchain["cargo"]["binary"] || file_binding(rustc)? != toolchain["rustc"]["binary"] {
        return Err("selected toolchain changed before SDK dependencies".into());
    }
    let declarations = nsstring_compile_inputs(&root, cargo, sdk_owner)?;
    let mut dependency_compiler = Command::new(cargo);
    dependency_compiler
        .current_dir(&root)
        .args([
            "build",
            "-vv",
            "--locked",
            "--offline",
            "--release",
            "--lib",
            "--message-format=json",
            "--manifest-path",
        ])
        .arg(root.join("bench/f1-runner/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--target")
        .arg(&target)
        .env("CARGO_INCREMENTAL", "0")
        .env("SDKROOT", &declarations.sdk_root)
        .env("RUSTC", rustc)
        .env_remove("RUYCAST_F1_NSSTRING_DECLARATION_PINS")
        .env_remove("UNSAFE_OBJC2_ALLOW_CLASS_OVERRIDE")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("DYLD_") { dependency_compiler.env_remove(key); }
    }
    let output = native_tool(&mut dependency_compiler, &root)?;
    native_private_bytes(&out.join("sdk-dependencies.stdout"), &serde_json::to_vec_pretty(&native_cargo_summary(&output.stdout,&output.stderr)?).map_err(|_| "SDK dependency summary")?)?;
    native_private_bytes(&out.join("sdk-dependencies.stderr"), b"Cargo-vv environment display omitted; exact rustc argv captured separately.\n")?;
    if !output.status.success() {
        return Err("SDK dependency build failed; no SDK loaded".into());
    }
    let names = [
        "ruycast_native",
        "serde",
        "serde_json",
        "sha2",
        "libc",
        "objc2",
        "block2",
        "objc2_core_foundation",
        "objc2_foundation",
    ];
    let mut dependencies = BTreeMap::new();
    for line in output.stdout.split(|b| *b == b'\n') {
        let Ok(row) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if row["reason"] != "compiler-artifact" {
            continue;
        }
        let Some(name) = row["target"]["name"].as_str() else {
            continue;
        };
        if !names.contains(&name) {
            continue;
        }
        if let Some(files) = row["filenames"].as_array() {
            for path in files.iter().filter_map(Value::as_str) {
                if path.ends_with(".rlib")
                    && Path::new(path).starts_with(target_dir.join(&target).join("release/deps"))
                {
                    dependencies.insert(name.to_owned(), PathBuf::from(path));
                }
            }
        }
    }
    if dependencies.len() != names.len() {
        return Err("SDK actual dependency roster incomplete".into());
    }
    let commands = native_rustc_commands(std::str::from_utf8(&output.stderr).map_err(|_| "SDK compiler display UTF8")?)?;
    let foundation_rlib = dependencies.get("objc2_foundation").ok_or("selected Foundation extern missing")?;
    let declaration_pins = nsstring_compile_pins(&root,out,&declarations,&commands,foundation_rlib,rustc,true)?;
    let foundation_before = file_binding(foundation_rlib)?;
    let mut compiler = Command::new(rustc);
    compiler
        .current_dir(&root)
        .arg(&initializer)
        .args([
            "--crate-name",
            "ruycast_owned_host",
            "--crate-type",
            "cdylib",
            "--edition=2021",
            "--target",
        ])
        .arg(&target)
        .args([
            "-C",
            "opt-level=3",
            "-C",
            "panic=unwind",
            "-Dwarnings",
            "-L",
        ])
        .arg(format!(
            "dependency={}",
            target_dir.join(&target).join("release/deps").display()
        ))
        .arg("-L")
        .arg(format!("dependency={}",target_dir.join("release/deps").display()))
        .arg("-o")
        .arg(&sdk_artifact)
        .env("CARGO_INCREMENTAL", "0")
        .env("SDKROOT", &declarations.sdk_root)
        .env("RUYCAST_F1_NSSTRING_DECLARATION_PINS", &declaration_pins)
        .env("CARGO_PKG_VERSION",env!("CARGO_PKG_VERSION"));
    for (name, path) in &dependencies {
        compiler
            .arg("--extern")
            .arg(format!("{name}={}", path.display()));
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("DYLD_") {
            compiler.env_remove(key);
        }
    }
    let output = compiler.output().map_err(|e| e.to_string())?;
    fs::write(out.join("sdk-compiler.stdout"), &output.stdout).map_err(|e| e.to_string())?;
    fs::write(out.join("sdk-compiler.stderr"), &output.stderr).map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("source-owned SDK constructor compilation failed; no loading".into());
    }
    if file_binding(foundation_rlib)? != foundation_before
        || file_binding(cargo)? != toolchain["cargo"]["binary"]
        || file_binding(rustc)? != toolchain["rustc"]["binary"]
        || nsstring_compile_pins(&root,out,&declarations,&commands,foundation_rlib,rustc,false)? != declaration_pins {
        return Err("SDK Foundation compiler inputs changed across SDK compile".into());
    }
    cap.verify()?;
    if cap.io.sdk_pins().map_err(|e|e.to_string())? != (grant,stdio)
        || cap.io.sdk_owner_uid().map_err(|e|e.to_string())? != sdk_owner
        || cap.io.native_owner_uid().map_err(|e|e.to_string())? != native_owner {
        return Err("Host SDK/stdio/owner capabilities changed during builds".into());
    }
    space_for_build(&root)?;
    if inventory(&root)?.files != before.files {
        return Err("SDK sources changed during construction".into());
    }
    fs::set_permissions(&sdk_artifact, fs::Permissions::from_mode(0o500)).map_err(|e|e.to_string())?;
    if sha(&fs::read(&endpoint).map_err(|e|e.to_string())?)!=endpoint_sha {
        return Err("Native artifact changed during SDK build".into());
    }
    let image = fs::read(&sdk_artifact).map_err(|e| e.to_string())?;
    let (base, initializers, initializer_slot) = host_image_preflight(&image, 6, 1)?;
    let output = Command::new("/usr/bin/nm")
        .args(["-a", "-n"])
        .arg(&sdk_artifact)
        .output()
        .map_err(|e| e.to_string())?;
    fs::write(out.join("sdk-symbols.txt"), &output.stdout).map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("SDK static symbol inspection failed".into());
    }
    let symbols = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let sdk_nonce_sha=sha(sdk_nonce.as_bytes());
    let endpoint_pin_present=image.windows(64).any(|part|part==endpoint_sha.as_bytes());
    let nonce_pin_present=image.windows(64).any(|part|part==sdk_nonce_sha.as_bytes());
    write_new(&out.join("sdk-static-inspection.json"),&json!({
        "scope":"static build proof only; no SDK loading", "sdk":file_binding(&sdk_artifact)?,
        "initializer_source":file_binding(&initializer)?,"source_manifest":file_binding(&manifest_path)?,
        "symbols":file_binding(&out.join("sdk-symbols.txt"))?,"image_base":base,
        "initializer_count":initializers,"source_inventory_unchanged":true,"final_grants_published":false,
        "compiled_endpoint_hash_present":endpoint_pin_present,
        "compiled_sdk_nonce_hash_present":nonce_pin_present
    }))?;
    let mut roster = Vec::new();
    let mut function_offsets = BTreeMap::new();
    for (name, needle) in [
        ("bind", "10profile_io4bind"),
        ("request", "10profile_io7request"),
        ("release_handle", "10profile_io14release_handle"),
        ("return_buffer", "10profile_io13return_buffer"),
        ("getter", "_rcf1_profile_io_v1"),
        ("ready", "_ruycast_f1_preentry_ready_v1"),
    ] {
        let offset=image_symbol(&symbols,needle,base)?;
        require_rx_offset(&image,base,offset)?;
        function_offsets.insert(name,offset);
        roster.push(json!({"name":name,"image_offset":offset}));
    }
    let api_offset = image_symbol(&symbols, "_rcf1_profile_io_api_v1", base)?;
    let initializer_offset = image_symbol(&symbols, "ruycast_full_owned_host_initializer", base)?;
    require_rx_offset(&image, base, initializer_offset)?;
    let mut fixup_tool=Command::new("/usr/bin/dyld_info");
    fixup_tool.arg("-fixups").arg(&sdk_artifact);
    for (key,_) in std::env::vars_os(){if key.to_string_lossy().starts_with("DYLD_"){fixup_tool.env_remove(key);}}
    let fixups=fixup_tool.output().map_err(|e|e.to_string())?;
    fs::write(out.join("sdk-fixups.txt"),&fixups.stdout).map_err(|e|e.to_string())?;
    fs::write(out.join("sdk-fixups.stderr"),&fixups.stderr).map_err(|e|e.to_string())?;
    if !fixups.status.success() || fixups.stdout.len()>16*1024*1024 {
        return Err("bounded static SDK fixup inspection failed".into());
    }
    let fixups=String::from_utf8(fixups.stdout).map_err(|e|e.to_string())?;
    let address=|offset:u64|base.checked_add(offset).ok_or_else(||"SDK address overflow".to_string());
    let callback=|name|function_offsets.get(name).copied().ok_or("SDK callback missing").and_then(|offset|base.checked_add(offset).ok_or("SDK callback address overflow"));
    verify_static_pointers(&image,initializer_slot.ok_or("SDK initializer slot absent")?,
        address(api_offset)?,address(initializer_offset)?,
        [callback("bind")?,callback("request")?,callback("release_handle")?,callback("return_buffer")?],&fixups)?;

    if !endpoint_pin_present || !nonce_pin_present {
        return Err("optimized SDK Host compile pins absent; no final grants".into());
    }
    let native_capture:Value=serde_json::from_slice(&fs::read(out.join("native-compiler-capture.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let input_callbacks=native_capture.get("input_callbacks").filter(|v|v.is_object()).ok_or("same-build Native Input5 compiler pins absent")?;
    let sdk_grant = json!({"role":"full-owned-host-v1","owner_uid":sdk_owner,"endpoint_sha256":endpoint_sha,"sdk_artifact_sha256":sha(&image),"source_set_sha256":sha(&source_set),"nonce":sdk_nonce,"stdio":stdio,"source_manifest":file_binding(&manifest_path)?,"initializer_source":file_binding(&initializer)?,"input_callbacks":input_callbacks});
    if fixture_epoch_binding(&workload_fixture.path,native_owner)?.0.sha256!=workload_fixture.sha256 {
        return Err("Native workload fixture changed before grant publication".into());
    }
    let native_grant = json!({"role":"full-owned-host-v1","owner_uid":native_owner,"nonce":cap.nonce,"endpoint":file_binding(&endpoint)?,"sdk":file_binding(&sdk_artifact)?,"source_manifest":file_binding(&manifest_path)?,"source_set_sha256":sha(&source_set),"fixture":workload_fixture,"fixture_sha256":workload_fixture.sha256,"api_header":file_binding(&root.join("rust-native/src/profile_io_abi.rs"))?,"api_offset":api_offset,"initializer_offset":initializer_offset,"function_roster":roster,"profile":fixture.join("profile"),"home":fixture.join("home"),"config_override":null});
    write_new(&out.join("final-sdk-grant.json"), &sdk_grant)?;
    write_new(&out.join("final-native-grant.json"), &native_grant)?;
    for path in [&initializer,&manifest_path,&out.join("final-sdk-grant.json"),&out.join("final-native-grant.json")] {
        fs::set_permissions(path,fs::Permissions::from_mode(0o400)).map_err(|e|e.to_string())?;
    }
    let _published = cap
        .io
        .publish(&sdk_grant, &native_grant)
        .map_err(|e| e.to_string())?;
    write_new(
        &out.join("host-images-receipt.json"),
        &json!({"scope":"two-image static construction only; never loaded or executed","native":file_binding(&endpoint)?,"sdk":file_binding(&sdk_artifact)?,"initializer":file_binding(&initializer)?,"source_manifest":file_binding(&manifest_path)?,"dependencies":dependencies,"initializer_count":initializers,"final_grants_published":true,"cap_lifetime":"held through both builds and final EOF publication; expires on preparation return","source_routes_qualified":false,"complete_fourteen_scope_assembler":false,"driver_protocol_wired":false,"runtime_readcap_inheritance_qualified":false}),
    )
}
fn image_symbol(symbols: &str, needle: &str, base: u64) -> Result<u64> {
    let matches = symbols
        .lines()
        .filter(|line| {
            let mut columns = line.split_whitespace();
            let (Some(_), Some(kind), Some(name), None) =
                (columns.next(), columns.next(), columns.next(), columns.next())
            else {
                return false; // nm -a also emits multi-column debug records.
            };
            kind.len() == 1 && kind.as_bytes()[0].is_ascii_alphabetic()
                && !name.starts_with(".Lfunc_end_")
                && if needle.starts_with('_') { name == needle } else { name.contains(needle) }
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "actual SDK symbol missing/ambiguous: {needle}; no synthetic offsets or final grant"
        ));
    }
    u64::from_str_radix(
        matches[0].split_whitespace().next().ok_or("symbol value")?,
        16,
    )
    .map_err(|e| e.to_string())?
    .checked_sub(base)
    .ok_or_else(|| "SDK symbol precedes image base".into())
}

#[cfg(test)]
#[test]
fn image_symbol_ignores_debug_records_and_refuses_two_real_definitions() {
    let one = "0000000100000100 - 01 0000 FUN _callback\n0000000100000100 T _callback\n";
    assert_eq!(image_symbol(one, "_callback", 0x100000000).unwrap(), 0x100);
    let two = format!("{one}0000000100000200 t _callback\n");
    assert!(image_symbol(&two, "_callback", 0x100000000).is_err());
    let guard = "__ZN21ruycast_reference_sdk3sdk17reference_runtime15reference_guard17hd0ab3debe5cb6983E";
    let naked = format!("0000000000075668 t {guard}\n0000000000075674 t .Lfunc_end_{guard}\n");
    let needle = "reference_runtime15reference_guard17h";
    assert_eq!(image_symbol(&naked, needle, 0).unwrap(), 0x75668);
    assert!(image_symbol(&format!("0000000000075674 t .Lfunc_end_{guard}\n"), needle, 0).is_err());
    assert!(image_symbol(&format!("{naked}0000000000075680 t {guard}\n"), needle, 0).is_err());
}
fn host_image_preflight(
    bytes: &[u8],
    kind: u32,
    expected_initializers: usize,
) -> Result<(u64, usize, Option<u64>)> {
    host_image_preflight_inner(bytes,kind,expected_initializers,None)
}
// The already existing Reference child policy admits one exact 4-byte
// S_INIT_FUNC_OFFSETS entry. Native/SDK callers retain their original parser
// policy through the wrapper above; no alternate initialization is admitted.
fn host_image_preflight_inner(bytes:&[u8],kind:u32,expected_initializers:usize,reference:Option<(u64,u64)>)
    ->Result<(u64,usize,Option<u64>)> {
    if reference.is_some() && (kind!=2 || expected_initializers!=1){return Err("Reference initializer policy only belongs to Main".into());}
    let u32_at = |at: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or("truncated Mach-O")?
                .try_into()
                .map_err(|_| "Mach-O u32")?,
        ))
    };
    let u64_at = |at: usize| -> Result<u64> {
        Ok(u64::from_le_bytes(
            bytes
                .get(at..at + 8)
                .ok_or("truncated Mach-O")?
                .try_into()
                .map_err(|_| "Mach-O u64")?,
        ))
    };
    if u32_at(0)? != 0xfeedfacf || u32_at(12)? != kind {
        return Err("Host image Mach-O kind differs".into());
    }
    let end = 32usize
        .checked_add(u32_at(20)? as usize)
        .ok_or("Mach-O command overflow")?;
    if end > bytes.len() {
        return Err("Mach-O commands exceed image".into());
    }
    let mut at = 32;
    let mut base = None;
    let mut initializers = 0;
    let mut initializer_slot = None;
    let mut reference_tables=0;
    for _ in 0..u32_at(16)? {
        let command = u32_at(at)?;
        let size = u32_at(at + 4)? as usize;
        let next = at.checked_add(size).ok_or("Mach-O command overflow")?;
        if size < 8 || next > end {
            return Err("invalid Mach-O load command".into());
        }
        if command == 0x11 || command == 0x1a || reference.is_some() && command==0x27 {
            return Err("LC_ROUTINES initializer is forbidden".into());
        }
        if command == 0x19 {
            if size < 72 {
                return Err("short SDK segment".into());
            }
            if bytes.get(at + 8..at + 14) == Some(b"__TEXT") {
                base = Some(u64_at(at + 24)?);
            }
            let count = u32_at(at + 64)? as usize;
            if count
                .checked_mul(80)
                .and_then(|n| n.checked_add(72))
                .is_none_or(|n| n > size)
            {
                return Err("SDK sections exceed segment".into());
            }
            for i in 0..count {
                let section = at + 72 + i * 80;
                let name = bytes
                    .get(section..section + 16)
                    .ok_or("truncated section name")?;
                let name = &name[..name.iter().position(|b| *b == 0).unwrap_or(16)];
                if matches!(name, b"__objc_nlclslist" | b"__objc_nlcatlist")
                    && u64_at(section + 40)? != 0
                {
                    return Err("Nonlazy ObjC +load sections are forbidden".into());
                }
                let flags=u32_at(section + 64)? & 0xff;
                let length=u64_at(section+40)?;
                if let Some((initializer,table))=reference {
                    if length!=0 && (matches!(flags,9|0xa|0xd|0x15)||name==b"__interpose") {
                        return Err("Reference unexpected initialization/termination section".into());
                    }
                    if flags==0x16 && length!=0 {
                        let address=u64_at(section+32)?;
                        let vm=u64_at(at+24)?;
                        let vm_end=vm.checked_add(u64_at(at+32)?).ok_or("Reference initializer segment overflow")?;
                        if length!=4 || address<vm || address.checked_add(4).is_none_or(|end|
                            end>vm_end) {
                            return Err("Reference initializer offset section range differs".into());
                        }
                        let encoded=readonly_file_range(bytes,address,4)?;
                        if u32::from_le_bytes(encoded.try_into().map_err(|_|"Reference initializer offset width")?) as u64!=initializer {
                            return Err("Reference sole initializer offset differs".into());
                        }
                        initializers+=1;
                    }
                    if name==b"__ruycast_ref" && length!=0 {
                        let segment=bytes.get(section+16..section+32).ok_or("Reference table segment name truncated")?;
                        if length!=216 || u64_at(section+32)?!=table
                            || segment.split(|byte|*byte==0).next()!=Some(b"__DATA_CONST") {
                            return Err("Reference exact static table section differs".into());
                        }
                        readonly_file_range(bytes,table,216)?;
                        reference_tables+=1;
                    }
                    continue;
                }
                if matches!(flags,0x15|0x16) && u64_at(section+40)?!=0 {
                    return Err("Alternate initializer sections are forbidden".into());
                }
                if flags == 9 {
                    let length = u64_at(section + 40)?;
                    if length != 0 {
                        let address=u64_at(section+32)?;
                        initializer_slot=Some(address);
                        let vm=u64_at(at+24)?;
                        let vm_end=vm.checked_add(u64_at(at+32)?).ok_or("initializer segment overflow")?;
                        let file_offset=u32_at(section+48)? as usize;
                        let protection=u32_at(at+60)?;
                        let readonly=protection&1!=0 && (protection&2==0 || u32_at(at+68)?&0x10!=0);
                        if length!=8 || address%8!=0 || address<vm
                            || address.checked_add(8).is_none_or(|end|end>vm_end)
                            || file_offset.checked_add(8).is_none_or(|end|end>bytes.len()) || !readonly {
                            return Err("SDK initializer slot is not one bounded read-only pointer".into());
                        }
                    }
                    initializers += usize::try_from(length / 8).map_err(|e| e.to_string())?;
                }
            }
        }
        at = next;
    }
    if at != end || initializers != expected_initializers || reference.is_some() && reference_tables!=1 {
        return Err("Host image initializer roster differs".into());
    }
    Ok((base.ok_or("SDK text segment absent")?, initializers, initializer_slot))
}

fn require_rx_offset(bytes: &[u8], base: u64, offset: u64) -> Result<()> {
    let address = base
        .checked_add(offset)
        .ok_or("initializer address overflow")?;
    let u32_at = |at: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or("truncated Mach-O")?
                .try_into()
                .map_err(|_| "Mach-O u32")?,
        ))
    };
    let u64_at = |at: usize| -> Result<u64> {
        Ok(u64::from_le_bytes(
            bytes
                .get(at..at + 8)
                .ok_or("truncated Mach-O")?
                .try_into()
                .map_err(|_| "Mach-O u64")?,
        ))
    };
    let mut at = 32;
    for _ in 0..u32_at(16)? {
        if u32_at(at)? == 0x19 {
            let start = u64_at(at + 24)?;
            let end = start
                .checked_add(u64_at(at + 32)?)
                .ok_or("segment overflow")?;
            if (start..end).contains(&address) && u32_at(at + 60)? & 7 == 5 {
                return Ok(());
            }
        }
        at += u32_at(at + 4)? as usize;
    }
    Err("actual SDK initializer is outside RX image code".into())
}

#[cfg(test)]
mod host_image_tests {
    use super::*;
    fn image(kind: u32, command: u32, nonlazy: bool, initializer: bool) -> Vec<u8> {
        let size = if command == 0x19 { 152 } else { 8 };
        let mut image = vec![0u8; 32 + size];
        let put = |bytes: &mut [u8], at: usize, value: u32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes())
        };
        put(&mut image, 0, 0xfeedfacf);
        put(&mut image, 12, kind);
        put(&mut image, 16, 1);
        put(&mut image, 20, size as u32);
        put(&mut image, 32, command);
        put(&mut image, 36, size as u32);
        if command == 0x19 {
            image[40..46].copy_from_slice(b"__TEXT");
            image[64..72].copy_from_slice(&4096u64.to_le_bytes());
            put(&mut image, 92, 5);
            put(&mut image, 96, 1);
            if nonlazy {
                image[104..120].copy_from_slice(b"__objc_nlclslist");
            }
            image[144..152].copy_from_slice(&8u64.to_le_bytes());
            if initializer {
                put(&mut image, 168, 9);
            }
        }
        image
    }
    #[test]
    fn static_preflight_rejects_initializer_commands_and_nonlazy_objc() {
        for command in [0x11, 0x1a] {
            assert!(host_image_preflight(&image(2, command, false, false), 2, 0).is_err());
        }
        assert!(host_image_preflight(&image(2, 0x19, true, false), 2, 0).is_err());
        assert!(host_image_preflight(&image(2, 0x19, false, true), 2, 0).is_err());
        for flags in [0x15u32,0x16] {
            let mut alternate=image(6,0x19,false,true);
            alternate[168..172].copy_from_slice(&flags.to_le_bytes());
            assert!(host_image_preflight(&alternate,6,1).is_err());
        }
        assert_eq!(
            host_image_preflight(&image(6, 0x19, false, true), 6, 1).unwrap(),
            (0, 1, Some(0))
        );
        require_rx_offset(&image(6, 0x19, false, true), 0, 128).unwrap();
        assert!(require_rx_offset(&image(6, 0x19, false, true), 0, 4096).is_err());
    }
    #[test]
    fn static_preflight_refuses_missing_or_ambiguous_symbols() {
        assert_eq!(
            image_symbol("00000010 T _ready\n", "_ready", 0).unwrap(),
            16
        );
        assert!(image_symbol("00000010 T _ready\n00000020 T _ready\n", "_ready", 0).is_err());
        assert!(image_symbol("00000010 T _other\n", "_ready", 0).is_err());
    }
}

fn static_rebases(text: &str) -> Result<BTreeMap<u64, u64>> {
    if text.len() > 16 * 1024 * 1024 {
        return Err("static fixup output exceeds bound".into());
    }
    let hex = |value: &str| {
        value
            .strip_prefix("0x")
            .ok_or("fixup is not hexadecimal")
            .and_then(|value| {
                u64::from_str_radix(value, 16).map_err(|_| "invalid fixup hexadecimal")
            })
    };
    let mut result = BTreeMap::new();
    for line in text.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.get(3) != Some(&"rebase") {
            continue;
        }
        if fields.len() != 5 {
            return Err("unsupported static rebase format".into());
        }
        if result.insert(hex(fields[2])?, hex(fields[4])?).is_some() {
            return Err("duplicate static rebase slot".into());
        }
    }
    Ok(result)
}
fn readonly_file_range(bytes: &[u8], address: u64, length: usize) -> Result<&[u8]> {
    let word = |at: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or("truncated SDK image")?
                .try_into()
                .map_err(|_| "SDK word")?,
        ))
    };
    let wide = |at: usize| -> Result<u64> {
        Ok(u64::from_le_bytes(
            bytes
                .get(at..at + 8)
                .ok_or("truncated SDK image")?
                .try_into()
                .map_err(|_| "SDK wide")?,
        ))
    };
    let end = address
        .checked_add(length as u64)
        .ok_or("SDK readonly range overflow")?;
    let mut at = 32usize;
    for _ in 0..word(16)? {
        let command = word(at)?;
        let size = word(at + 4)? as usize;
        if size < 8 {
            return Err("invalid SDK load command".into());
        }
        if command == 0x19 {
            if size < 72 {
                return Err("short SDK segment".into());
            }
            let vm = wide(at + 24)?;
            let vm_end = vm
                .checked_add(wide(at + 32)?)
                .ok_or("SDK segment overflow")?;
            if address >= vm && end <= vm_end {
                let protection = word(at + 60)?;
                if protection & 1 == 0 || (protection & 2 != 0 && word(at + 68)? & 0x10 == 0) {
                    return Err("SDK storage is not read-only after fixups".into());
                }
                let delta = address - vm;
                let file_size = wide(at + 48)?;
                if delta
                    .checked_add(length as u64)
                    .is_none_or(|end| end > file_size)
                {
                    return Err("SDK storage is not file-backed".into());
                }
                let file = wide(at + 40)?
                    .checked_add(delta)
                    .ok_or("SDK file offset overflow")?;
                let file = usize::try_from(file).map_err(|e| e.to_string())?;
                return bytes
                    .get(file..file.checked_add(length).ok_or("SDK file range overflow")?)
                    .ok_or_else(|| "SDK storage exceeds file".into());
            }
        }
        at = at.checked_add(size).ok_or("SDK command overflow")?;
    }
    Err("SDK storage outside image segments".into())
}
fn verify_static_pointers(
    image: &[u8],
    initializer_slot: u64,
    api_address: u64,
    initializer: u64,
    callbacks: [u64; 4],
    fixups: &str,
) -> Result<()> {
    use ruycast_native::profile_io_abi::{Api, VERSION};
    if !initializer_slot.is_multiple_of(8) || !api_address.is_multiple_of(std::mem::align_of::<Api>() as u64) {
        return Err("unaligned SDK static storage".into());
    }
    readonly_file_range(image, initializer_slot, 8)?;
    let table = readonly_file_range(image, api_address, std::mem::size_of::<Api>())?;
    if table[..4] != VERSION.to_le_bytes()
        || table[4..8] != (std::mem::size_of::<Api>() as u32).to_le_bytes()
        || table[8..24].iter().any(|byte| *byte != 0)
    {
        return Err("actual static SDK API header differs".into());
    }
    let rebases = static_rebases(fixups)?;
    if rebases.get(&initializer_slot) != Some(&initializer) {
        return Err("actual SDK initializer fixup differs from selected RX constructor".into());
    }
    let fields = [
        std::mem::offset_of!(Api, bind),
        std::mem::offset_of!(Api, request),
        std::mem::offset_of!(Api, release_handle),
        std::mem::offset_of!(Api, return_buffer),
    ];
    for (field, callback) in fields.into_iter().zip(callbacks) {
        let slot = api_address
            .checked_add(field as u64)
            .ok_or("SDK API slot overflow")?;
        if rebases.get(&slot) != Some(&callback) {
            return Err("actual SDK API callback fixup differs from pinned RX roster".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod static_pointer_tests {
    use super::*;
    #[test]
    fn input_capsule_requires_its_exact_single_const_section() {
        let mut image=vec![0_u8;512];
        let word=|b:&mut[u8],at:usize,v:u32|b[at..at+4].copy_from_slice(&v.to_le_bytes());
        let wide=|b:&mut[u8],at:usize,v:u64|b[at..at+8].copy_from_slice(&v.to_le_bytes());
        word(&mut image,16,1); word(&mut image,32,0x19); word(&mut image,36,152);
        image[40..52].copy_from_slice(b"__DATA_CONST"); word(&mut image,96,1);
        image[104..119].copy_from_slice(b"__ruycast_input");
        image[120..132].copy_from_slice(b"__DATA_CONST");
        wide(&mut image,136,0x1200); wide(&mut image,144,56);
        input_capsule_section(&image,0x1200).unwrap();
        let mut wrong=image.clone(); wrong[120]=b'X';
        assert!(input_capsule_section(&wrong,0x1200).is_err());
        let mut wrong=image.clone(); wide(&mut wrong,144,64);
        assert!(input_capsule_section(&wrong,0x1200).is_err());
        assert!(input_capsule_section(&image,0x1204).is_err());
        let mut wrong=image.clone(); word(&mut wrong,96,2); word(&mut wrong,36,232);
        wrong[184..264].copy_from_slice(&image[104..184]);
        assert!(input_capsule_section(&wrong,0x1200).is_err());
    }
    fn image() -> Vec<u8> {
        let mut image = vec![0u8; 256];
        let put = |bytes: &mut [u8], at: usize, value: u32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes())
        };
        put(&mut image, 16, 1);
        put(&mut image, 32, 0x19);
        put(&mut image, 36, 72);
        image[64..72].copy_from_slice(&256u64.to_le_bytes());
        image[80..88].copy_from_slice(&256u64.to_le_bytes());
        put(&mut image, 92, 3);
        put(&mut image, 100, 0x10);
        put(&mut image, 128, ruycast_native::profile_io_abi::VERSION);
        put(
            &mut image,
            132,
            std::mem::size_of::<ruycast_native::profile_io_abi::Api>() as u32,
        );
        image[232..240].fill(0xff); // Encoded words are never interpreted as pointers.
        image
    }
    const FIXUPS:&str="__DATA_CONST __mod_init_func 0xE8 rebase 0x1000\n__DATA_CONST __const 0x98 rebase 0x2000\n__DATA_CONST __const 0xA0 rebase 0x3000\n__DATA_CONST __const 0xA8 rebase 0x4000\n__DATA_CONST __const 0xB0 rebase 0x5000\n";
    fn check(image: &[u8], fixups: &str) -> Result<()> {
        verify_static_pointers(
            image,
            232,
            128,
            0x1000,
            [0x2000, 0x3000, 0x4000, 0x5000],
            fixups,
        )
    }
    #[test]
    fn actual_decoded_pointer_values_are_required() {
        check(&image(), FIXUPS).unwrap();
        assert!(check(&image(), &FIXUPS.replace("rebase 0x1000", "rebase 0x9999")).is_err());
        assert!(check(&image(), &FIXUPS.replace("rebase 0x2000", "rebase 0x9999")).is_err());
        assert!(check(
            &image(),
            &FIXUPS.lines().skip(1).collect::<Vec<_>>().join("\n")
        )
        .is_err());
        assert!(check(
            &image(),
            &(FIXUPS.to_owned() + "__DATA_CONST __const 0x98 rebase 0x2000\n")
        )
        .is_err());
        assert!(check(&image(), &FIXUPS.replace("rebase 0x2000", "rebase unknown")).is_err());
    }
    #[test]
    fn actual_api_header_and_readonly_file_storage_are_required() {
        let mut writable = image();
        writable[100..104].fill(0);
        assert!(check(&writable, FIXUPS).is_err());
        let mut reserved = image();
        reserved[136] = 1;
        assert!(check(&reserved, FIXUPS).is_err());
        let mut size = image();
        size[132] = 0;
        assert!(check(&size, FIXUPS).is_err());
        assert!(readonly_file_range(&image(), 252, 8).is_err());
    }
}

/// One source-owned data-family builder. The reviewed immutable input/argument
/// records below are provenance, not caller-selected packets or runtime pins.
pub fn apple_event_data(out: &Path, execute: bool) -> Result<()> {
    data_family_inner(out, execute, false, DataCase::AppleEvent)
}
pub fn diagnose_apple_event_data(out: &Path) -> Result<()> {
    data_family_inner(out, true, true, DataCase::AppleEvent)
}
pub fn foundation_data(out:&Path,execute:bool)->Result<()> {
    data_family_inner(out,execute,false,DataCase::Foundation)
}
pub fn diagnose_foundation_data(out: &Path) -> Result<()> {
    data_family_inner(out, true, true, DataCase::Foundation)
}
#[derive(Clone,Copy,Eq,PartialEq)]
enum DataCase { AppleEvent, Foundation }
struct DataRecipe<'a> {
    inputs: Binding,
    dependencies: Binding,
    compiler: Binding,
    foreign_inputs: Vec<Binding>,
    // Produced by nsstring_compile_pins from the qualified same-build receipt;
    // never a runtime environment value or a source-file observation alone.
    declaration_pins: Option<String>,
    compiled:NativeCompiled<'a>,
    compiler_sources:NativeCompilerSources,
    search:DataSearchInputs,
    whole:Option<Instant>,
}
impl DataRecipe<'_> {
    fn verify(&self)->Result<()> {
        verify_data_source_context(&self.compiled,&self.compiler_sources,self.whole)?;
        verify_data_search_inputs(&self.search,self.whole)?;
        verify_data_rustc_inputs(&self.compiled.rustc_inputs,self.whole)
    }
    fn arguments(&self,root:&Path,case:DataCase,dependencies:&BTreeMap<PathBuf,String>,endpoint:&Path,dep:&Path)->Result<Vec<String>> {
        let mut arguments=data_compiler_arguments(root,case,dependencies,&self.compiler.path,endpoint,dep)?;
        let mut index=0;
        while index<arguments.len(){if arguments[index]=="-L"{arguments.drain(index..index+2);}else{index+=1;}}
        arguments.extend(self.search.arguments.clone());
        arguments.extend(self.compiled.linker.rustflags().into_iter().skip(2));
        Ok(arguments)
    }
}
fn data_compiler_path(compiler: &Path) -> Result<&str> {
    let normalized = compiler.components().collect::<PathBuf>();
    if !compiler.is_absolute() || compiler.file_name()!=Some(std::ffi::OsStr::new("rustc"))
        || normalized.as_os_str()!=compiler.as_os_str()
        || compiler.components().any(|part|matches!(part,std::path::Component::ParentDir|std::path::Component::CurDir)) {
        return Err("Data compiler must be the exact absolute qualified rustc path".into());
    }
    compiler.to_str().ok_or("Data compiler path UTF8".into())
}
fn verify_data_compiler(compiler: &Binding) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    data_compiler_path(&compiler.path)?;
    if fs::canonicalize(&compiler.path).map_err(|_| "Data compiler canonical path")?!=compiler.path {
        return Err("Data compiler path alias".into());
    }
    let owner=fs::symlink_metadata(&compiler.path).map_err(|_| "Data compiler metadata")?.uid();
    let actual=declaration_file(&compiler.path,owner)?;
    if actual.sha256!=compiler.sha256 {return Err("Data compiler byte binding changed".into());}
    Ok(())
}
fn data_declaration_pins(case: DataCase, inputs: &[Binding], owner: u32,
    supplied: Option<&str>) -> Result<Option<String>> {
    if case == DataCase::AppleEvent {
        if !inputs.is_empty() || supplied.is_some() {
            return Err("AppleEvent recipe must not contain NSString declaration pins".into());
        }
        return Ok(None);
    }
    if inputs.len() != 2 { return Err("Foundation declaration pin count differs".into()); }
    let supplied: Vec<(PathBuf,String,u32)> = serde_json::from_str(
        supplied.ok_or("Foundation same-build declaration literal missing")?)
        .map_err(|_| "Foundation declaration literal schema")?;
    let expected = inputs.iter().enumerate().map(|(i,input)|
        (input.path.clone(),input.sha256.clone(),if i == 0 { 0 } else { owner })
    ).collect::<Vec<_>>();
    if supplied != expected { return Err("Foundation compiled declaration literal differs from private pins".into()); }
    for (input,(_,_,uid)) in inputs.iter().zip(&expected) {
        let actual = declaration_file(&input.path,*uid)?;
        if actual.path != input.path || actual.sha256 != input.sha256 {
            return Err("Foundation declaration differs from compiler binding".into());
        }
    }
    serde_json::to_string(&expected).map(Some).map_err(|_| "Foundation declaration literal serialization".into())
}
#[derive(serde::Serialize)]
struct DataSearchInputs {
    owner:u32,
    arguments:Vec<String>,
    directories:BTreeMap<PathBuf,std::collections::BTreeSet<PathBuf>>,
    inputs:Vec<(Binding,u32)>,
}
fn data_search_paths(directory:&Path,owner:u32,whole:Option<Instant>)->Result<std::collections::BTreeSet<PathBuf>> {
    use std::os::unix::fs::MetadataExt;
    native_capture_alive(whole)?;
    let metadata=fs::symlink_metadata(directory).map_err(|e|e.to_string())?;
    if !metadata.is_dir()||metadata.uid()!=owner||metadata.mode()&0o022!=0
        ||fs::canonicalize(directory).map_err(|e|e.to_string())?!=directory {
        return Err("Data compiler dependency search directory owner/alias".into());
    }
    let mut paths=std::collections::BTreeSet::new();let mut entries=0;
    for entry in fs::read_dir(directory).map_err(|e|e.to_string())? {
        native_capture_alive(whole)?;entries+=1;
        if entries>100000{return Err("Data compiler search directory entry bound".into());}
        let path=entry.map_err(|e|e.to_string())?.path();
        if !path.extension().and_then(|ext|ext.to_str()).is_some_and(|ext|matches!(ext,"rlib"|"rmeta"|"dylib"|"so"|"a")){continue;}
        if paths.len()>=4096{return Err("Data compiler library candidate bound".into());}
        // Candidate aliases are inputs too; refuse rather than silently resolve.
        if fs::canonicalize(&path).map_err(|e|e.to_string())?!=path{return Err("Data compiler search candidate alias".into());}
        paths.insert(path);
    }
    native_capture_alive(whole)?;Ok(paths)
}
fn data_search_inputs(root:&Path,compiled:&NativeCompiled<'_>,whole:Option<Instant>)->Result<DataSearchInputs> {
    let owner=compiled.cap.io.sdk_owner_uid().map_err(|e|e.to_string())?;
    let bins=compiled.commands.iter().filter(|args|args.windows(2).any(|p|p[0]=="--crate-name"&&p[1]=="ruycast_native")
        &&args.windows(2).any(|p|p[0]=="--crate-type"&&p[1]=="bin")).collect::<Vec<_>>();
    if bins.len()!=1{return Err("Data actual BIN search arguments absent/ambiguous".into());}
    let mut parents=std::collections::BTreeSet::new();let mut arguments=Vec::new();
    for pair in bins[0].windows(2).filter(|p|p[0]=="-L") {
        let (kind,path)=pair[1].split_once('=').ok_or("Data actual search kind absent")?;
        if kind!="dependency"{return Err("Data actual non-dependency search kind not yet qualified".into());}
        let path=PathBuf::from(path);
        if !parents.insert(path){return Err("Data actual search directory duplicate".into());}
        arguments.extend(["-L".to_owned(),pair[1].clone()]);
    }
    for dependency in compiled.data_dependencies.values() {
        if !dependency.path.parent().is_some_and(|parent|parents.contains(parent)) {
            return Err("Data direct extern parent absent from actual BIN search path".into());
        }
    }
    let mut directories=BTreeMap::new();let mut inputs=Vec::new();
    for directory in parents {
        if !directory.starts_with(root.join("rust-native/target")){return Err("Data compiler search leaves exact existing target".into());}
        let paths=data_search_paths(&directory,owner,whole)?;
        for path in &paths {inputs.push((compiler_input_bytes(path,owner,whole)?.0,owner));}
        directories.insert(directory,paths);
    }
    if directories.is_empty(){return Err("Data compiler actual dependency search context absent".into());}
    Ok(DataSearchInputs{owner,arguments,directories,inputs})
}
fn verify_data_search_inputs(inputs:&DataSearchInputs,whole:Option<Instant>)->Result<()> {
    for (binding,owner) in &inputs.inputs {
        if compiler_input_bytes(&binding.path,*owner,whole)?.0.sha256!=binding.sha256 {
            return Err("Data compiler library/proc-macro search candidate changed".into());
        }
    }
    for (directory,paths) in &inputs.directories {
        if data_search_paths(directory,inputs.owner,whole)?!=*paths{return Err("Data compiler search candidate set changed".into());}
    }
    native_capture_alive(whole)
}

#[derive(serde::Serialize)]
struct DataLinkerInputs {
    developer:PathBuf,
    sdk:PathBuf,
    clang:PathBuf,
    ld:PathBuf,
    executables:Vec<CompilerInputCap>,
    lto:CompilerInputCap,
}
impl DataLinkerInputs {
    fn rustflags(&self)->Vec<String> {
        vec!["-C".into(),"debuginfo=2".into(),"-C".into(),format!("linker={}",self.clang.display()),
            "-C".into(),format!("link-arg=--ld-path={}",self.ld.display()),
            "-C".into(),format!("link-arg=-Wl,-lto_library,{}",self.lto.binding.path.display())]
    }
}
fn verify_data_linker_inputs(inputs:&DataLinkerInputs,whole:Option<Instant>)->Result<()> {
    for executable in &inputs.executables{verify_compiler_input_cap(executable,whole)?;}
    verify_compiler_input_cap(&inputs.lto,whole)
}
fn verify_data_linker_arguments(commands:&[Vec<String>],inputs:&DataLinkerInputs)->Result<()> {
    let wanted=[format!("linker={}",inputs.clang.display()),format!("link-arg=--ld-path={}",inputs.ld.display()),
        format!("link-arg=-Wl,-lto_library,{}",inputs.lto.binding.path.display())];
    let mut roles=0;
    for args in commands {
        if !args.windows(2).any(|p|p[0]=="--crate-name"&&matches!(p[1].as_str(),"ruycast_native"|"objc2"|"objc2_foundation")){continue;}
        let mut codegen=Vec::new();let mut index=0;
        while index<args.len() {
            let arg=&args[index];
            if arg=="-C"||arg=="--codegen" {
                index+=1;codegen.push(args.get(index).ok_or("actual codegen value absent")?.as_str());
            }else if let Some(value)=arg.strip_prefix("-C").or_else(||arg.strip_prefix("--codegen=")) {
                codegen.push(value);
            }
            index+=1;
        }
        if codegen.iter().filter(|value|value.starts_with("linker=")).copied().collect::<Vec<_>>() != [wanted[0].as_str()] {
            return Err("actual compiler role has absent/conflicting clang selection".into());
        }
        let mut ld_paths=Vec::new();let mut lto_paths=Vec::new();
        for value in codegen {
            if value.starts_with("linker-flavor="){return Err("actual compiler changes the qualified linker flavor".into());}
            if let Some(argument)=value.strip_prefix("link-arg=") {
                if argument.contains("isysroot")||argument.contains("sysroot")||argument.contains("syslibroot") {
                    return Err("actual compiler custom SDK override".into());
                }
                if argument.starts_with("--ld-path="){ld_paths.push(value);}
                else if argument.starts_with("-Wl,-lto_library,"){lto_paths.push(value);}
                else if argument.contains("ld-path")||argument.contains("fuse-ld") {
                    return Err("actual compiler alternate linker selection".into());
                }else if argument.contains("lto_library") {
                    return Err("actual compiler alternate LTO library selection".into());
                }
            }else if value.starts_with("link-args=")&&(value.contains("ld-path")||value.contains("fuse-ld")||value.contains("lto_library")
                ||value.contains("isysroot")||value.contains("sysroot")||value.contains("syslibroot")) {
                return Err("actual compiler grouped alternate linker selection".into());
            }
        }
        if ld_paths != [wanted[1].as_str()] {
            return Err("actual compiler role has absent/conflicting ld selection".into());
        }
        if lto_paths != [wanted[2].as_str()] {return Err("actual compiler role has absent/conflicting LTO library selection".into());}
        roles+=1;
    }
    if roles!=4{return Err("qualified linker requires the existing four compiler roles".into());}
    Ok(())
}
fn data_linker_inputs(root:&Path,out:&Path,sdk:&Path,whole:Option<Instant>)->Result<DataLinkerInputs> {
    use std::os::unix::fs::MetadataExt;
    let mut executables=Vec::new();
    for path in ["/usr/bin/xcode-select","/usr/bin/xcrun"] {
        executables.push(compiler_input_cap(Path::new(path),0,whole)?.0);
    }
    let run=|name:&str,command:&mut Command|->Result<std::process::Output> {
        native_capture_alive(whole)?;
        for held in &executables{verify_compiler_input_cap(held,whole)?;}
        let args=std::iter::once(command.get_program()).chain(command.get_args())
            .map(|arg|arg.to_str().map(str::to_owned).ok_or("linker query argument UTF8"))
            .collect::<std::result::Result<Vec<_>,_>>()?;
        native_private_bytes(&out.join(format!("data-linker-{name}-command.json")),
            &serde_json::to_vec_pretty(&json!({"arguments":args,"scope":"tool selection/driver query only; no compilation or linker execution"})).map_err(|e|e.to_string())?)?;
        let result=native_tool_with_deadline(command,root,whole)?;
        {use std::os::unix::process::ExitStatusExt;
            native_private_bytes(&out.join(format!("data-linker-{name}-result.json")),
                &serde_json::to_vec_pretty(&json!({"exit":result.status.code(),"signal":result.status.signal(),
                    "stdout_bytes":result.stdout.len(),"stderr_bytes":result.stderr.len()})).map_err(|e|e.to_string())?)?;
        }
        native_private_bytes(&out.join(format!("data-linker-{name}.stdout")),&result.stdout)?;
        native_private_bytes(&out.join(format!("data-linker-{name}.stderr")),&result.stderr)?;
        if !result.status.success(){return Err("selected linker query refused".into());}
        Ok(result)
    };
    let path_output=|output:std::process::Output|->Result<PathBuf> {
        if !output.stderr.is_empty(){return Err("selected linker path query diagnostics".into());}
        let text=std::str::from_utf8(&output.stdout).map_err(|_|"selected linker path UTF8")?;
        let value=text.trim();let path=PathBuf::from(value);
        if value.lines().count()!=1||!path.is_absolute()||path.components().any(|part|matches!(part,std::path::Component::CurDir|std::path::Component::ParentDir)) {
            return Err("selected linker path is not exact absolute".into());
        }
        // Keep the selector's actual text in stdout; hold the resolved real
        // input, including the normal MacOSX.sdk version-selection symlink.
        fs::canonicalize(&path).map_err(|e|e.to_string())
    };
    let mut select=Command::new("/usr/bin/xcode-select");
    select.args(["-p"]).current_dir(root).env_clear().env("PATH","/usr/bin:/bin");
    let developer=path_output(run("developer",&mut select)?)?;
    let meta=fs::symlink_metadata(&developer).map_err(|e|e.to_string())?;
    if !meta.is_dir()||meta.uid()!=0||meta.mode()&0o022!=0{return Err("selected developer directory ownership".into());}
    let mut sdk_query=Command::new("/usr/bin/xcrun");
    sdk_query.args(["--sdk","macosx","--show-sdk-path"]).current_dir(root).env_clear()
        .env("PATH","/usr/bin:/bin").env("DEVELOPER_DIR",&developer);
    if path_output(run("sdk",&mut sdk_query)?)?!=sdk{return Err("selected linker SDK differs from actual Foundation declaration SDK".into());}
    let sdk_meta=fs::symlink_metadata(sdk).map_err(|e|e.to_string())?;
    if !sdk_meta.is_dir()||sdk_meta.uid()!=0||sdk_meta.mode()&0o022!=0{return Err("selected linker SDK ownership".into());}
    let mut selected=Vec::new();
    for tool in ["clang","ld"] {
        let mut command=Command::new("/usr/bin/xcrun");
        command.args(["--find",tool]).current_dir(root).env_clear().env("PATH","/usr/bin:/bin").env("DEVELOPER_DIR",&developer);
        selected.push(path_output(run(tool,&mut command)?)?);
    }
    let clang=selected.remove(0);let ld=selected.remove(0);
    if !clang.starts_with(&developer)||!ld.starts_with(&developer)||clang.parent()!=ld.parent() {
        return Err("selected clang/ld leave the same developer toolchain".into());
    }
    // The query has not executed the selected tools yet. Hold their real bytes
    // and LTO library before asking the driver to display its linker command.
    let clang_cap=compiler_input_cap(&clang,0,whole)?.0;
    let ld_cap=compiler_input_cap(&ld,0,whole)?.0;
    let lto=compiler_input_cap(&clang.parent().and_then(Path::parent).ok_or("clang install absent")?.join("lib/libLTO.dylib"),0,whole)?.0;
    if lto.binding.path.to_str().ok_or("LTO path UTF8")?.contains(','){return Err("LTO path cannot be a grouped linker argument".into());}
    let mut command=Command::new(&clang);
    command.args(["-###","-x","c","/dev/null","-o"]).arg(out.join("data-driver-query-never-created"))
        .arg(format!("--ld-path={}",ld.display())).current_dir(root).env_clear().env("PATH","/usr/bin:/bin").env("DEVELOPER_DIR",&developer);
    command.arg(format!("-Wl,-lto_library,{}",lto.binding.path.display())).arg("-isysroot").arg(sdk).env("SDKROOT",sdk);
    let output=run("driver",&mut command)?;
    let lines=std::str::from_utf8(&output.stderr).map_err(|_|"clang driver display UTF8")?;
    let mut matches=0;
    for line in lines.lines().filter(|line|line.trim_start().starts_with('"')) {
        let words=native_shell_words(line.trim())?;
        if words.first().is_some_and(|word|Path::new(word)==ld) {
            for (option,expected) in [("-lto_library",lto.binding.path.as_path()),("-syslibroot",sdk)] {
                let mut count=0;
                for (index,word) in words.iter().enumerate() {
                    if word==option {
                        if words.get(index+1).is_none_or(|value|Path::new(value)!=expected) {
                            return Err("displayed linker has a conflicting LTO/SDK selection".into());
                        }
                        count+=1;
                    }
                }
                if count==0{return Err("displayed linker does not select pinned LTO/SDK".into());}
            }
            matches+=1;
        }
    }
    if matches!=1||out.join("data-driver-query-never-created").exists(){return Err("clang did not display exactly the selected nonexecuted linker".into());}
    executables.extend([clang_cap,ld_cap]);
    let inputs=DataLinkerInputs{developer,sdk:sdk.to_owned(),clang,ld,executables,lto};
    verify_data_linker_inputs(&inputs,whole)?;Ok(inputs)
}

#[derive(serde::Serialize)]
struct DataRustcInputs { install:PathBuf,driver:DataSearchInputs,sysroot:DataSearchInputs }
fn data_rustc_inputs(root:&Path,out:&Path,compiler:&Binding,target:&str,cap_owner:u32,whole:Option<Instant>)->Result<DataRustcInputs> {
    use std::os::unix::fs::MetadataExt;
    verify_data_compiler(compiler)?;
    let owner=fs::symlink_metadata(&compiler.path).map_err(|e|e.to_string())?.uid();
    if ![0,cap_owner].contains(&owner) {
        return Err("Data selected compiler install owner differs".into());
    }
    let install=compiler.path.parent().and_then(Path::parent).ok_or("Data compiler install root absent")?.to_owned();
    let capture=|directory:PathBuf|->Result<DataSearchInputs> {
        if !directory.starts_with(&install){return Err("Data compiler libraries leave selected install".into());}
        let paths=data_search_paths(&directory,owner,whole)?;
        let mut inputs=Vec::new();
        for path in &paths {inputs.push((compiler_input_bytes(path,owner,whole)?.0,owner));}
        if inputs.is_empty(){return Err("Data compiler library input set absent".into());}
        Ok(DataSearchInputs{owner,arguments:Vec::new(),directories:BTreeMap::from([(directory,paths)]),inputs})
    };
    // Bind the installed driver before the actual semantic sysroot query.
    let driver=capture(install.join("lib"))?;
    if !driver.inputs.iter().any(|(binding,_)|binding.path.file_name().and_then(|n|n.to_str())
        .is_some_and(|name|name.starts_with("librustc_driver-")&&name.ends_with(".dylib"))) {
        return Err("Data actual selected rustc driver library absent".into());
    }
    let mut command=Command::new(&compiler.path);
    command.args(["--print","sysroot"]).current_dir(root).env_clear().env("PATH","/usr/bin:/bin");
    native_private_bytes(&out.join("data-rustc-sysroot-command.json"),&serde_json::to_vec_pretty(&json!({
        "compiler":compiler,"arguments":["--print","sysroot"],"driver_before":&driver,
        "scope":"semantic query after toolchain-selection probes, before Native Cargo compilation"})).map_err(|e|e.to_string())?)?;
    let result=native_tool_with_deadline(&mut command,root,whole)?;
    {use std::os::unix::process::ExitStatusExt;
        native_private_bytes(&out.join("data-rustc-sysroot-result.json"),&serde_json::to_vec_pretty(&json!({
            "exit":result.status.code(),"signal":result.status.signal(),"stdout_bytes":result.stdout.len(),"stderr_bytes":result.stderr.len()})).map_err(|e|e.to_string())?)?;
    }
    native_private_bytes(&out.join("data-rustc-sysroot.stdout"),&result.stdout)?;
    native_private_bytes(&out.join("data-rustc-sysroot.stderr"),&result.stderr)?;
    if !result.status.success()||!result.stderr.is_empty(){return Err("Data actual rustc sysroot query failed".into());}
    let value=std::str::from_utf8(&result.stdout).map_err(|_|"Data sysroot UTF8")?.trim();
    if value.lines().count()!=1||Path::new(value)!=install||fs::canonicalize(value).map_err(|e|e.to_string())?!=install {
        return Err("Data actual rustc sysroot differs from selected install".into());
    }
    verify_data_compiler(compiler)?;verify_data_search_inputs(&driver,whole)?;
    let sysroot=capture(install.join("lib/rustlib").join(target).join("lib"))?;
    if !sysroot.inputs.iter().any(|(binding,_)|binding.path.file_name().and_then(|n|n.to_str())
        .is_some_and(|name|name.starts_with("libproc_macro-")&&name.ends_with(".rlib"))) {
        return Err("Data fixed builtin proc_macro sysroot input absent".into());
    }
    native_capture_alive(whole)?;
    Ok(DataRustcInputs{install,driver,sysroot})
}
fn verify_data_rustc_inputs(inputs:&DataRustcInputs,whole:Option<Instant>)->Result<()> {
    verify_data_search_inputs(&inputs.driver,whole)?;
    verify_data_search_inputs(&inputs.sysroot,whole)
}

fn verify_data_source_context(compiled:&NativeCompiled<'_>,sources:&NativeCompilerSources,whole:Option<Instant>)->Result<()> {
    verify_data_linker_inputs(&compiled.linker,whole)?;
    native_capture_alive(whole)?;compiled.cap.verify()?;
    verify_data_compiler(&compiled.compiler)?;compiled.cargo.verify()?;
    for executable in &compiled.executables{verify_compiler_input_cap(executable,whole)?;}
    let owner=compiled.cap.io.sdk_owner_uid().map_err(|e|e.to_string())?;
    if data_declaration_pins(DataCase::Foundation,&compiled.foreign_inputs,owner,Some(&compiled.foundation_pins))?
        .as_deref()!=Some(compiled.foundation_pins.as_str()) {
        return Err("Data common Foundation foreign/literal context changed".into());
    }
    for dependency in compiled.data_dependencies.values(){dependency.verify()?;}
    for (path,expected) in &compiled.package_source_before {
        let owner=compiled.package_roots.values().filter(|(dir,_)|path.starts_with(dir))
            .max_by_key(|(dir,_)|dir.components().count()).map(|(_,owner)|*owner)
            .ok_or("Data compiler source original package owner absent")?;
        if compiler_input_bytes(path,owner,whole)?.0.sha256!=*expected {
            return Err("Data compiler package source changed".into());
        }
    }
    for input in sources.inputs.values() {
        if compiler_input_bytes(&input.binding.path,input.owner,whole)?.0.sha256!=input.binding.sha256 {
            return Err("Data transitive compiler input changed".into());
        }
    }
    for produced in compiled.produced_roots.values() {
        if compiler_input_bytes(&produced.producer.path,produced.invocation.identity.2,whole)?.0.sha256!=produced.producer.sha256 {
            return Err("Data generated-input producer executable changed".into());
        }
        for expected in [&produced.invocation,&produced.root_output] {
            let Some((actual,_))=native_trace_file(&expected.binding.path,expected.identity.2,whole)?else {
                return Err("Data compiler produced trace disappeared".into());
            };
            if actual.identity!=expected.identity||actual.binding.sha256!=expected.binding.sha256 {
                return Err("Data compiler produced post-state changed".into());
            }
        }
    }
    native_capture_alive(whole)
}
// Compile the complete real SDK entry with its existing panic-only pins.
// This produces compiler DATA and never an executable or admission witness.
fn canonical_data_source_probe(root:&Path,out:&Path,case:DataCase,compiled:&NativeCompiled<'_>,
    before:&Inventory,search:&DataSearchInputs,whole:Option<Instant>)->Result<Binding> {
    use std::os::unix::fs::PermissionsExt;
    native_capture_alive(whole)?;compiled.cap.verify()?;
    let owner=compiled.cap.io.sdk_owner_uid().map_err(|e|e.to_string())?;
    verify_data_compiler(&compiled.compiler)?;compiled.cargo.verify()?;
    let dependencies=compiled.data_dependencies.values().map(|binding|(binding.path.clone(),binding.sha256.clone()))
        .collect::<BTreeMap<_,_>>();
    if dependencies.len()!=9{return Err("Data probe exact nine actual compiler dependencies absent".into());}
    for dependency in compiled.data_dependencies.values(){dependency.verify()?;}
    let pins=root.join("bench/f1-runner/inputs/compile-only-pins.rs");
    let (template,_)=declaration_bytes(&pins,owner)?;
    if before.files.get(Path::new("bench/f1-runner/inputs/compile-only-pins.rs"))!=Some(&template.sha256) {
        return Err("Data probe panictemplate differs from canonical before".into());
    }
    let artifact=out.join("data-source-probe.rmeta");let dep=out.join("data-source-probe.d");
    let mut arguments=data_compiler_arguments(root,case,&dependencies,&compiled.compiler.path,&artifact,&dep)?;
    // Remove materialized parent-only paths and use the exact actual BIN -L
    // order, including its host proc-macro dependency directory.
    let mut index=0;
    while index<arguments.len(){if arguments[index]=="-L"{arguments.drain(index..index+2);}else{index+=1;}}
    arguments.extend(search.arguments.clone());
    arguments.extend(compiled.linker.rustflags().into_iter().skip(2));
    let emit=arguments.iter_mut().filter(|arg|arg.starts_with("--emit=")).collect::<Vec<_>>();
    if emit.len()!=1{return Err("Data probe compiler emit role differs".into());}
    *emit.into_iter().next().ok_or("Data probe compiler emit absent")?=
        format!("--emit=metadata,dep-info={}",dep.to_str().ok_or("Data probe dep-info UTF8")?);
    let mut command=Command::new(&arguments[0]);
    command.args(&arguments[1..]).current_dir(root).env_clear()
        .env("PATH","/usr/bin:/bin").env("CARGO_INCREMENTAL","0")
        .env("DEVELOPER_DIR",&compiled.linker.developer).env("SDKROOT",&compiled.linker.sdk)
        .env("CARGO_MANIFEST_DIR",root.join("bench/f1-runner"))
        .env("CARGO_PKG_NAME",env!("CARGO_PKG_NAME")).env("CARGO_PKG_VERSION",env!("CARGO_PKG_VERSION"))
        .env("F1_DATA_PINS_SOURCE",&pins);
    if case==DataCase::Foundation {
        let literal=data_declaration_pins(case,&compiled.foreign_inputs,owner,Some(&compiled.foundation_pins))?
            .ok_or("Data Foundation probe literal absent")?;
        command.env("RUYCAST_F1_NSSTRING_DECLARATION_PINS",literal);
    }
    native_private_bytes(&out.join("data-source-probe-command.json"),&serde_json::to_vec_pretty(&json!({
        "arguments":arguments,"compiler":compiled.compiler,"panictemplate":template,"dependency_search_before":search,"rustc_inputs_before":compiled.rustc_inputs,
        "scope":"full canonical entry/SDK compile-only metadata; no execution/authority"})).map_err(|e|e.to_string())?)?;
    let result=native_tool_with_deadline(&mut command,root,whole)?;
    native_private_bytes(&out.join("data-source-probe.stdout"),&result.stdout)?;
    native_private_bytes(&out.join("data-source-probe.stderr"),&result.stderr)?;
    {use std::os::unix::process::ExitStatusExt;
        native_private_bytes(&out.join("data-source-probe-result.json"),&serde_json::to_vec_pretty(&json!({
            "exit":result.status.code(),"signal":result.status.signal(),"stdout_bytes":result.stdout.len(),
            "stderr_bytes":result.stderr.len(),"authority":false,"executable":false})).map_err(|e|e.to_string())?)?;
    }
    if !result.status.success(){return Err("canonical Data full SDK metadata probe failed; no endpoint executed".into());}
    let (_,dep_bytes)=compiler_input_bytes(&dep,owner,whole)?;
    let paths=dependency_paths(std::str::from_utf8(&dep_bytes).map_err(|_|"Data probe DI UTF8")?)?;
    let mut rows=BTreeMap::new();
    for path in paths {
        native_capture_alive(whole)?;
        let path=fs::canonicalize(if path.is_absolute(){path}else{root.join(path)}).map_err(|e|e.to_string())?;
        let relative=path.strip_prefix(root).map_err(|_|"canonical Data probe source leaves checkout")?;
        recipe_relative_path(relative,false)?;
        let actual=declaration_file(&path,owner)?;
        if before.files.get(relative)!=Some(&actual.sha256){return Err("actual Data SDK DI input absent/changed from genuine canonical before".into());}
        if rows.insert(relative.to_owned(),actual.sha256).is_some(){return Err("canonical Data probe DI source duplicate".into());}
    }
    for required in [Path::new(case.entry()),Path::new("bench/f1-runner/inputs/compile-only-pins.rs"),
        Path::new("bench/f1-runner/src/sdk/mod.rs"),Path::new("bench/f1-runner/src/sdk/data_family_preentry.rs")] {
        if !rows.contains_key(required){return Err("canonical Data complete SDK/entry/panictemplate DI incomplete".into());}
    }
    verify_data_compiler(&compiled.compiler)?;compiled.cargo.verify()?;compiled.cap.verify()?;
    for dependency in compiled.data_dependencies.values(){dependency.verify()?;}
    let actual=rows.into_iter().map(|(path,sha256)|Binding{path,sha256}).collect::<Vec<_>>();
    let bytes=serde_json::to_vec_pretty(&actual).map_err(|e|e.to_string())?;
    let record=out.join("canonical-data-actual-source-inputs.json");native_private_bytes(&record,&bytes)?;
    fs::set_permissions(&artifact,fs::Permissions::from_mode(0o600)).map_err(|e|e.to_string())?;
    native_capture_alive(whole)?;
    Ok(Binding{path:record,sha256:sha(&bytes)})
}

fn source_owned_data_recipe<'a>(root:&Path,out:&Path,case:DataCase,cap:&'a NativeCompileCap) -> Result<DataRecipe<'a>> {
    use std::os::unix::fs::MetadataExt;
    cap.verify()?;
    let owner=cap.io.sdk_owner_uid().map_err(|e|e.to_string())?;
    let root_metadata=fs::symlink_metadata(root).map_err(|_|"Data recipe root metadata")?;
    let output_metadata=fs::symlink_metadata(out).map_err(|_|"Data recipe output metadata")?;
    if !root_metadata.is_dir() || !output_metadata.is_dir()
        || root_metadata.uid()!=owner || output_metadata.uid()!=owner || output_metadata.mode()&0o777!=0o700
        || fs::canonicalize(root).map_err(|_|"Data recipe root canonical")?!=root
        || fs::canonicalize(out).map_err(|_|"Data recipe output canonical")?!=out
        || !out.starts_with(root.join("rust-native/target")) {
        return Err("Data recipe requires the already validated owned output and same compilecap owner".into());
    }
    // The private laboratory's historical path/hash packets are deliberately
    // NOT public recipes. Restore each role only after its actual canonical
    // source closure, same-build dependencies and foreign inputs are reviewed.
    // These declarations were derived from the actual canonical caller DI,
    // not the private laboratory's older 41-row closure. They are deliberately
    // insufficient to qualify dependencies, foreign pins or runtime admission.
    let rows: Vec<Binding> = serde_json::from_slice(case.source_declaration())
        .map_err(|_| "canonical Data source declaration schema")?;
    let mut paths=std::collections::BTreeSet::new();
    for row in rows {
        recipe_relative_path(&row.path,false)?;
        if row.sha256.len()!=64 || !row.sha256.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))
            || !paths.insert(row.path) { return Err("canonical Data declaration duplicate/digest".into()); }
    }
    if !paths.contains(Path::new(case.entry())) || !paths.contains(Path::new("bench/f1-runner/inputs/compile-only-pins.rs")) {
        return Err("canonical Data declaration entry/pins template missing".into());
    }
    if !root.join(case.entry()).is_file()
        ||!root.join("bench/f1-runner/inputs/compile-only-pins.rs").is_file() {
        return Err("canonical Data source-owned compiler recipe is not yet qualified".into());
    }
    let canonical_before=inventory(root)?;
    data_compiler_policy(root,case)?;
    let whole=Some(Instant::now()+std::time::Duration::from_secs(REFERENCE_CAPTURE_SECONDS));
    let compiled=native_compile_with_deadline(root,&out.join("same-build-native"),cap,whole)?;
    let compiler_sources=native_compiler_sources(root,&compiled,whole)?;
    let search_inputs=data_search_inputs(root,&compiled,whole)?;
    let rustc_inputs=&compiled.rustc_inputs;
    verify_data_source_context(&compiled,&compiler_sources,whole)?;
    verify_data_search_inputs(&search_inputs,whole)?;
    verify_data_rustc_inputs(rustc_inputs,whole)?;
    let actual_sdk_sources=canonical_data_source_probe(root,out,case,&compiled,&canonical_before,&search_inputs,whole)?;
    verify_data_source_context(&compiled,&compiler_sources,whole)?;
    verify_data_search_inputs(&search_inputs,whole)?;
    verify_data_rustc_inputs(rustc_inputs,whole)?;
    let mut lines=String::new();
    for dependency in compiled.data_dependencies.values() {
        let relative=dependency.path.strip_prefix(root).map_err(|_|"Data dependency leaves source target")?;
        recipe_relative_path(relative,true)?;
        lines.push_str(&format!("{}  {}\n",dependency.sha256,relative.to_str().ok_or("Data dependency path UTF8")?));
    }
    let record=out.join("canonical-data-actual-dependencies.txt");
    native_private_bytes(&record,lines.as_bytes())?;
    let compiler=compiled.compiler.clone();
    let foreign_inputs=if case==DataCase::Foundation{compiled.foreign_inputs.clone()}else{Vec::new()};
    let declaration_pins=data_declaration_pins(case,&foreign_inputs,owner,
        (case==DataCase::Foundation).then_some(compiled.foundation_pins.as_str()))?;
    let recipe=DataRecipe{inputs:actual_sdk_sources,
        dependencies:Binding{path:record,sha256:sha(lines.as_bytes())},compiler,foreign_inputs,declaration_pins,
        compiled,compiler_sources,search:search_inputs,whole};
    recipe.verify()?;
    native_capture_alive(whole)?;
    Ok(recipe)
}
impl DataCase {
    fn family(self)->&'static str { match self { Self::AppleEvent=>"AppleEventDescriptor",Self::Foundation=>"FoundationData" } }
    fn role(self)->&'static str { match self { Self::AppleEvent=>"apple-event-descriptor-data-v1",Self::Foundation=>"foundation-data-v1" } }
    fn argv(self)->&'static str { match self { Self::AppleEvent=>"__f1-appleevent-data",Self::Foundation=>"__f1-foundation-data" } }
    fn stem(self)->&'static str { match self { Self::AppleEvent=>"apple-event-data",Self::Foundation=>"foundation-data" } }
    fn entry(self)->&'static str { match self { Self::AppleEvent=>"bench/f1-runner/examples/apple_event_data.rs",Self::Foundation=>"bench/f1-runner/examples/foundation_data.rs" } }
    fn source_declaration(self)->&'static [u8] { match self {
        Self::AppleEvent=>include_bytes!("../../inputs/apple-event-source-roster.json"),
        Self::Foundation=>include_bytes!("../../inputs/foundation-source-roster.json"),
    } }
}
fn recipe_relative_path(path: &Path, artifact: bool) -> Result<()> {
    let normalized: PathBuf = path.components().collect();
    if path.as_os_str().is_empty() || path.is_absolute() || normalized.as_os_str()!=path.as_os_str()
        || path.components().any(|part| !matches!(part,std::path::Component::Normal(_)))
        || path.components().any(|part| [".git",".go",".build"].iter().any(|name| part.as_os_str()==*name))
        || (!artifact && path.components().any(|part|part.as_os_str()=="target"))
        || (artifact && !path.starts_with("rust-native/target")) {
        return Err("compiler recipe requires exact source-relative paths, not laboratory paths or traversal".into());
    }
    Ok(())
}
fn source_recipe_binding(root: &Path, relative: Binding) -> Result<Binding> {
    use std::os::unix::fs::MetadataExt;
    recipe_relative_path(&relative.path,false)?;
    let path = root.join(relative.path);
    let owner = fs::symlink_metadata(root).map_err(|_| "source checkout owner")?.uid();
    let observed = declaration_file(&path,owner)?;
    if observed.sha256 != relative.sha256 { return Err("source-owned recipe input bytes differ".into()); }
    Ok(observed)
}
fn data_compiler_policy(root: &Path, case: DataCase) -> Result<Binding> {
    let (name, bytes): (&str,&[u8]) = match case {
        DataCase::AppleEvent=>("apple-event-compiler-policy.json",include_bytes!("../../inputs/apple-event-compiler-policy.json")),
        DataCase::Foundation=>("foundation-compiler-policy.json",include_bytes!("../../inputs/foundation-compiler-policy.json")),
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "embedded Data compiler policy")?;
    if value != json!({"schema":"ruycast.f1.data-compiler-policy.v1","role":case.role(),"toolchain":"1.92.0","edition":"2021","test_no_main":true,"optimized":true,"debug_assertions":true,"lto":"thin","codegen_units":1}) {
        return Err("source-owned Data compiler policy differs".into());
    }
    source_recipe_binding(root,Binding{path:Path::new("bench/f1-runner/inputs").join(name),sha256:sha(bytes)})
}
fn data_compiler_arguments(root: &Path, case: DataCase, dependencies: &BTreeMap<PathBuf,String>,
    compiler: &Path, endpoint: &Path, dep_info: &Path) -> Result<Vec<String>> {
    // Materialization ONLY, after the same-build source/dependency receipt has
    // been qualified. Filenames route aliases; they never qualify an rlib.
    let names=["block2","libc","objc2","objc2_core_foundation","objc2_foundation","ruycast_native","serde","serde_json","sha2"];
    if dependencies.len()!=names.len() { return Err("exact nine Data externs required".into()); }
    let mut selected=BTreeMap::new();
    for path in dependencies.keys() {
        if !path.starts_with(root.join("rust-native/target")) || path.components().any(|part|matches!(part,std::path::Component::ParentDir|std::path::Component::CurDir)) {
            return Err("Data extern materialization escaped owned target".into());
        }
        let file=path.file_name().and_then(|p|p.to_str()).ok_or("Data extern filename UTF8")?;
        let name=names.iter().filter(|name|file==format!("lib{name}.rlib") || (file.starts_with(&format!("lib{name}-")) && file.ends_with(".rlib"))).collect::<Vec<_>>();
        if name.len()!=1 || selected.insert(*name[0],path).is_some() { return Err("Data extern alias missing/duplicate".into()); }
    }
    let mut args=vec![data_compiler_path(compiler)?.into(),"--edition=2021".into(),"--test".into(),"-Dwarnings".into(),
        root.join(case.entry()).to_str().ok_or("Data entry UTF8")?.into(),"-O".into(),"-C".into(),"debug-assertions=yes".into(),"-C".into(),"lto=thin".into(),"-C".into(),"codegen-units=1".into()];
    let mut directories=std::collections::BTreeSet::new();
    for (name,path) in selected {
        args.extend(["--extern".into(),format!("{name}={}",path.to_str().ok_or("Data extern path UTF8")?)]);
        directories.insert(path.parent().ok_or("Data extern parent")?);
    }
    for directory in directories { args.extend(["-L".into(),format!("dependency={}",directory.to_str().ok_or("Data extern directory UTF8")?)]); }
    args.extend(["-o".into(),endpoint.to_str().ok_or("Data output UTF8")?.into(),format!("--emit=link,dep-info={}",dep_info.to_str().ok_or("Data dep-info UTF8")?)]);
    Ok(args)
}
fn data_family_inner(out: &Path, execute: bool, diagnose: bool, case:DataCase) -> Result<()> {
    use std::os::{fd::AsRawFd, unix::fs::PermissionsExt};
    let repository = checkout_root()?;
    let measurement = repository.clone();
    let source_root = repository.clone();
    if !out.is_absolute()
        || !out.starts_with(measurement.join("rust-native/target"))
        || out.components().any(|p| {
            !matches!(
                p,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
        || fs::canonicalize(out.parent().ok_or("data output parent missing")?)
            .map_err(|e| e.to_string())?
            != out.parent().unwrap()
    {
        return Err("data output must be new within exact existing measurement target".into());
    }
    space_for_build(&measurement)?;
    fs::create_dir(out).map_err(|e| e.to_string())?;
    fs::set_permissions(out, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    // One holder owns the dependency capture, foreign pins and final child.
    let cap=NativeCompileCap::new()?;
    let recipe=source_owned_data_recipe(&repository,out,case,&cap)?;
    recipe.verify()?;
    let owner=cap.io.sdk_owner_uid().map_err(|e|e.to_string())?;
    let compiler_binding = recipe.compiler.clone();
    verify_data_compiler(&compiler_binding)?;
    let reviewed_inputs = recipe.inputs.clone();
    let reviewed_args = data_compiler_policy(&repository,case)?;
    let (actual_inputs,input_bytes)=compiler_input_bytes(&reviewed_inputs.path,owner,recipe.whole)?;
    if actual_inputs.sha256!=reviewed_inputs.sha256{return Err("Data actual source record changed".into());}
    if compiler_input_bytes(&reviewed_args.path,owner,recipe.whole)?.0.sha256!=reviewed_args.sha256 {
        return Err("Data compiler policy changed".into());
    }
    let reviewed_dependencies = recipe.dependencies.clone();
    let (actual_dependencies,dependency_bytes)=compiler_input_bytes(&reviewed_dependencies.path,owner,recipe.whole)?;
    if actual_dependencies.sha256!=reviewed_dependencies.sha256{return Err("Data actual dependency record changed".into());}
    let mut expected_dependencies = BTreeMap::new();
    for line in std::str::from_utf8(&dependency_bytes).map_err(|_|"Data dependency record UTF8")?.lines()
    {
        if line.is_empty() {
            continue;
        }
        let (hash, path) = line
            .split_once("  ")
            .ok_or("reviewed dependency record format")?;
        if Path::new(path)
            .extension()
            .is_some_and(|extension| extension == "rlib")
        {
            let relative = Path::new(path);
            recipe_relative_path(relative,true)?;
            let path = repository.join(relative);
            if fs::canonicalize(&path).map_err(|e|e.to_string())? != path { return Err("Data extern path alias".into()); }
            if expected_dependencies
                .insert(path, hash.to_owned())
                .is_some()
            {
                return Err("duplicate reviewed rlib".into());
            }
        }
    }
    if expected_dependencies.len() != 9 {
        return Err("reviewed rlib roster incomplete".into());
    }
    let input_rows: Vec<Binding> = serde_json::from_slice::<Vec<Binding>>(&input_bytes).map_err(|_|"Data source record schema")?.into_iter()
        .map(|row|source_recipe_binding(&source_root,row)).collect::<Result<_>>()?;
    let main = fs::canonicalize(
        source_root.join(case.entry()),
    )
    .map_err(|e| e.to_string())?;
    let abi = fs::canonicalize(
        source_root.join(match case {DataCase::AppleEvent=>"bench/f1-runner/src/sdk/workspace_apple_events.rs",DataCase::Foundation=>"bench/f1-runner/src/sdk/workspace_foundation.rs"}),
    )
    .map_err(|e| e.to_string())?;
    let placeholder = repository.join("bench/f1-runner/inputs/compile-only-pins.rs");
    let fixture = out.join("owned-fixture");
    fs::create_dir(&fixture).map_err(|e| e.to_string())?;
    fs::set_permissions(&fixture, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let pins_path = out.join("data-pins.rs");
    let endpoint = out.join(case.stem());
    let dep_info = out.join(format!("{}.d",case.stem()));
    let manifest = out.join("source-manifest.json");
    let mut sources = BTreeMap::new();
    let mut placeholder_found = false;
    for input in &input_rows {
        if compiler_input_bytes(&input.path,owner,recipe.whole)?.0.sha256!=input.sha256 {
            return Err("Data reviewed source input changed".into());
        }
        if input.path == placeholder {
            placeholder_found = true;
            continue;
        }
        let path = fs::canonicalize(&input.path).map_err(|e| e.to_string())?;
        if !path.starts_with(&source_root) {
            return Err("reviewed data source leaves retained task source root".into());
        }
        if sources
            .insert(path, input.sha256.clone())
            .is_some_and(|old| old != input.sha256)
        {
            return Err("canonical source aliases differ in bytes".into());
        }
    }
    if !placeholder_found || !sources.contains_key(&main) || !sources.contains_key(&abi) {
        return Err("reviewed data inputs incomplete".into());
    }
    let mut source_paths = sources.keys().cloned().collect::<Vec<_>>();
    source_paths.push(pins_path.clone());
    source_paths.sort_by(|a, b| {
        a.as_os_str()
            .as_encoded_bytes()
            .cmp(b.as_os_str().as_encoded_bytes())
    });
    let (grant, stdio) = cap.io.sdk_pins().map_err(|e| e.to_string())?;
    let grant_fd = cap.io.sdk_grant().as_raw_fd();
    let fingerprint = |fp: crate::host_io::Fingerprint| {
        format!(
            "sdk::data_family_preentry::Fingerprint{{device:{},inode:{},kind:{},access:{}}}",
            fp.device, fp.inode, fp.kind, fp.access
        )
    };
    let paths = source_paths
        .iter()
        .map(|p| rust_path(p).map(|p| format!("std::path::Path::new({p})")))
        .collect::<Result<Vec<_>>>()?
        .join(",\n");
    // These exact foreign declaration inputs are separate from rustc's actual
    // source closure. A runtime observation cannot mint their compiled pins.
    let foreign_inputs = recipe.foreign_inputs.clone();
    if foreign_inputs.len() != if case == DataCase::Foundation { 2 } else { 0 } {
        return Err("Data recipe foreign declaration count differs".into());
    }
    let declaration_pins = data_declaration_pins(case,&foreign_inputs,owner,recipe.declaration_pins.as_deref())?;
    let foreign_literals=foreign_inputs.iter().map(|input|Ok(format!("sdk::data_family_preentry::ForeignInputPin{{path:std::path::Path::new({}),sha256:{}}}",rust_path(&input.path)?,serde_json::to_string(&input.sha256).map_err(|e|e.to_string())?))).collect::<Result<Vec<_>>>()?.join(",");
    let pins = format!(
        r#"// Actual trusted producer literals. No self-artifact hash or runtime pin file.
static DATA_SOURCE_PATHS: std::sync::LazyLock<[&'static std::path::Path;{count}]> = std::sync::LazyLock::new(||[{paths}]);
static DATA_FOREIGN_INPUTS: std::sync::LazyLock<[sdk::data_family_preentry::ForeignInputPin<'static>;{foreign_count}]> = std::sync::LazyLock::new(||[{foreign_literals}]);
fn make_data_pins()->sdk::data_family_preentry::Pins<'static>{{
    sdk::data_family_preentry::Pins{{family:sdk::data_family_preentry::DataFamily::{family},foreign_inputs:&*DATA_FOREIGN_INPUTS,
        source_root:std::path::Path::new({source_root}),fixture_root:std::path::Path::new({fixture}),
        endpoint:std::path::Path::new({endpoint}),source_manifest:std::path::Path::new({manifest}),
        abi_leaf:std::path::Path::new({abi}),entry_source:std::path::Path::new({main}),
        source_paths:&*DATA_SOURCE_PATHS,nonce_sha256:{nonce},owner_uid:{owner},grant_fd:{grant_fd},
        grant:{grant},stdio:[{stdin},{stdout},{stderr}]}}
}}
"#,
        count = source_paths.len(),
        family=case.family(),
        foreign_count=foreign_inputs.len(),
        source_root = rust_path(&source_root)?,
        fixture = rust_path(&fixture)?,
        endpoint = rust_path(&endpoint)?,
        manifest = rust_path(&manifest)?,
        abi = rust_path(&abi)?,
        main = rust_path(&main)?,
        nonce = serde_json::to_string(&sha(cap.nonce.as_bytes())).map_err(|e| e.to_string())?,
        grant = fingerprint(grant),
        stdin = fingerprint(stdio[0]),
        stdout = fingerprint(stdio[1]),
        stderr = fingerprint(stdio[2])
    );
    fs::write(&pins_path, pins).map_err(|e| e.to_string())?;
    sources.insert(
        pins_path.clone(),
        compiler_input_bytes(&pins_path,owner,recipe.whole)?.0.sha256,
    );
    let mut dependencies = BTreeMap::new();
    for path in expected_dependencies.keys() {
            let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
            if !path.starts_with(measurement.join("rust-native/target")) { return Err("Data dependency leaves existing owned target".into()); }
            dependencies.insert(
                path.clone(),
                compiler_input_bytes(&path,owner,recipe.whole)?.0.sha256,
            );
    }
    if dependencies != expected_dependencies {
        return Err(
            "actual rlib bytes differ from independently reviewed dependency freeze".into(),
        );
    }
    let arguments = recipe.arguments(&repository,case,&dependencies,&endpoint,&dep_info)?;
    recipe.verify()?;
    verify_data_compiler(&compiler_binding)?;
    let mut compiler = Command::new(&arguments[0]);
    compiler
        .args(&arguments[1..])
        .current_dir(&measurement)
        .env_clear()
        .env("PATH","/usr/bin:/bin")
        .env("DEVELOPER_DIR",&recipe.compiled.linker.developer)
        .env("SDKROOT",&recipe.compiled.linker.sdk)
        .env("CARGO_MANIFEST_DIR",repository.join("bench/f1-runner"))
        .env("F1_DATA_PINS_SOURCE", &pins_path)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_PKG_NAME", env!("CARGO_PKG_NAME"))
        .env("CARGO_PKG_VERSION", env!("CARGO_PKG_VERSION"));
    compiler.env_remove("RUYCAST_F1_NSSTRING_DECLARATION_PINS");
    if let Some(literal) = &declaration_pins {
        compiler.env("RUYCAST_F1_NSSTRING_DECLARATION_PINS",literal);
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("DYLD_") {
            compiler.env_remove(key);
        }
    }
    native_private_bytes(&out.join("data-child-compiler-command.json"),&serde_json::to_vec_pretty(&json!({
        "arguments":arguments,"compiler":compiler_binding,"linker_inputs":&recipe.compiled.linker,
        "search_inputs":&recipe.search,"same_compilecap":true,"scope":"compile only; endpoint not yet admitted"})).map_err(|e|e.to_string())?)?;
    let compiled = native_tool_with_deadline(&mut compiler,&measurement,recipe.whole)?;
    {use std::os::unix::process::ExitStatusExt;
        native_private_bytes(&out.join("data-child-compiler-result.json"),&serde_json::to_vec_pretty(&json!({
            "exit":compiled.status.code(),"signal":compiled.status.signal(),"stdout_bytes":compiled.stdout.len(),
            "stderr_bytes":compiled.stderr.len()})).map_err(|e|e.to_string())?)?;
    }
    fs::write(out.join("compiler.stdout"), &compiled.stdout).map_err(|e| e.to_string())?;
    fs::write(out.join("compiler.stderr"), &compiled.stderr).map_err(|e| e.to_string())?;
    if !compiled.status.success() {
        return Err("actual data source/pins compile failed; no endpoint executed".into());
    }
    recipe.verify()?;
    fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o500)).map_err(|e| e.to_string())?;
    let (_,dep_bytes)=compiler_input_bytes(&dep_info,owner,recipe.whole)?;
    let actual_paths =
        dependency_paths(std::str::from_utf8(&dep_bytes).map_err(|_|"Data actual dep-info UTF8")?)?
            .into_iter()
            .map(|p| fs::canonicalize(p).map_err(|e| e.to_string()))
            .collect::<Result<std::collections::BTreeSet<_>>>()?;
    if actual_paths != sources.keys().cloned().collect() {
        return Err(
            "actual data rustc dependency set differs from generated exact source roster".into(),
        );
    }
    for (path, hash) in sources.iter().chain(dependencies.iter()) {
        if compiler_input_bytes(path,owner,recipe.whole)?.0.sha256 != *hash {
            return Err("data compiler input changed during build".into());
        }
    }
    verify_data_compiler(&compiler_binding)?;
    if data_declaration_pins(case,&foreign_inputs,owner,recipe.declaration_pins.as_deref())? != declaration_pins {
        return Err("Data declaration compiler pins changed during compile".into());
    }
    cap.verify()?;
    if cap.io.sdk_pins().map_err(|e| e.to_string())? != (grant, stdio)
        || cap.io.sdk_owner_uid().map_err(|e| e.to_string())? != owner
    {
        return Err("data private capabilities changed during compile".into());
    }
    space_for_build(&measurement)?;
    let (_,image) = compiler_input_bytes(&endpoint,owner,recipe.whole)?;
    let mut symbols_tool = Command::new("/usr/bin/nm");
    symbols_tool.args(["-a", "-n"]).arg(&endpoint);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("DYLD_") {
            symbols_tool.env_remove(key);
        }
    }
    let symbols = native_tool_with_deadline(&mut symbols_tool,&measurement,recipe.whole)?;
    fs::write(out.join("symbols.txt"), &symbols.stdout).map_err(|e| e.to_string())?;
    fs::write(out.join("symbols.stderr"), &symbols.stderr).map_err(|e| e.to_string())?;
    if !symbols.status.success() || symbols.stdout.len() > 16 * 1024 * 1024 {
        return Err("bounded data code-entry symbol inspection failed".into());
    }
    let symbols = String::from_utf8(symbols.stdout).map_err(|e| e.to_string())?;
    let main_address = image_symbol(&symbols, "_main", 0)?;
    data_image_preflight(&image, main_address)?;
    recipe.verify()?;
    let rows = source_paths
        .iter()
        .map(|path| json!({"path":path,"sha256":sources[path]}))
        .collect::<Vec<_>>();
    write_new(&manifest, &json!({"sources":rows}))?;
    let mut source_set = Vec::new();
    for path in &source_paths {
        let hash = &sources[path];
        source_set.extend_from_slice(path.as_os_str().as_encoded_bytes());
        source_set.push(0);
        source_set.extend_from_slice(hash.as_bytes());
        source_set.push(b'\n');
    }
    let final_grant = json!({"role":case.role(),"family":case.family(),"foreign_inputs":foreign_inputs,"owner_uid":owner,"nonce":cap.nonce,"endpoint_sha256":sha(&image),"source_set_sha256":sha(&source_set),"source_manifest":compiler_input_bytes(&manifest,owner,recipe.whole)?.0,"abi_leaf":compiler_input_bytes(&abi,owner,recipe.whole)?.0,"stdio":stdio});
    write_new(&out.join("final-data-grant.json"), &final_grant)?;
    recipe.verify()?;
    let published = cap
        .io
        .publish(&final_grant, &Value::Null)
        .map_err(|e| e.to_string())?;
    for path in [&pins_path, &manifest, &out.join("final-data-grant.json")] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o400)).map_err(|e| e.to_string())?;
    }
    let mut child_evidence=None;
    let mut sample_evidence = None;
    let status = if execute {
        let deadline = Instant::now() + std::time::Duration::from_secs(15);
        let (mut child, parent) = published
            .spawn_with_sdk_grant(
                &endpoint,
                &[case.argv().into()],
                &[
                    ("F1_DATA_GRANT_FD".into(), grant_fd.to_string()),
                    ("__CF_USER_TEXT_ENCODING".into(), data_cf_context(owner)),
                ],
                &fixture,
            )
            .map_err(|e| e.to_string())?;
        let (input, output) = parent.into_fds();
        drop(input);
        if diagnose {
            sample_evidence=Some(data_diagnostics::observe_owned_child(&child.process,deadline,out));
        }
        let wait_budget=if diagnose {deadline.saturating_duration_since(Instant::now())} else {std::time::Duration::from_secs(15)};
        let evidence=finish_data_child(&mut child,output,out,wait_budget);
        let status=evidence["original_wait_status"].as_i64().map(|value|value as i32);
        child_evidence=Some(evidence);
        status
    } else {
        drop(published);
        None
    };
    let raw_stdout=child_evidence.as_ref().and_then(|evidence|evidence.get("raw_stdout")).cloned();
    let unexpected_stdout=status==Some(0) && child_evidence.as_ref().and_then(|evidence|evidence["stdout_bytes"].as_u64()).is_some_and(|count|count!=0);
    write_new(
        &out.join("receipt.json"),
        &json!({"scope":"owned closed data family only; no SDK/Host/public14 admission","family":case.family(),"endpoint":file_binding(&endpoint)?,"pins":file_binding(&pins_path)?,"source_manifest":file_binding(&manifest)?,"reviewed_inputs":reviewed_inputs,"reviewed_arguments":reviewed_args,"reviewed_dependencies":reviewed_dependencies,"compiler":compiler_binding,"compiler_arguments":arguments,"dependencies":dependencies,"source_paths":source_paths,"foreign_inputs":foreign_inputs,"private_cap_held_across_compile":true,"grant_published_with_eof":true,"native_cap_inherited":false,"executed":execute,"exit_status":status,"sampling_diagnostic":sample_evidence,"raw_stdout":raw_stdout,"child_evidence":child_evidence,"prepare_cap_lifetime":"expires on prepare-only return; new child requires fresh cap/source compile","source_routes_qualified":false}),
    )?;
    if let Some(evidence)=&child_evidence {
        if let Some(error)=evidence["original_wait_error"].as_str(){return Err(error.to_owned());}
        for key in ["cleanup_error","stdout_capture_error"]{
            if let Some(error)=evidence[key].as_str(){return Err(error.to_owned());}
        }
        if evidence["stdout_complete"]!=true{return Err("data stdout drain incomplete; bounded evidence retained".into());}
    }
    if sample_evidence.as_ref().is_some_and(|sample| sample["failure"].is_string()) {
        return Err("data sampling diagnostic failed; private evidence retained".into());
    }
    if unexpected_stdout{return Err("successful data endpoint produced unexpected stdout; raw evidence retained".into());}
    if status.is_some_and(|status| status != 0) {
        return Err("owned data family endpoint failed; bounded receipt retained".into());
    }
    Ok(())
}
// Data-only process context, not a capability or a copied user preference.
// The UID is the same private grantcap fstat owner already compiled into Pins.
fn data_cf_context(pinned_owner_uid: u32) -> String {
    format!("0x{pinned_owner_uid:X}:0x0:0x0")
}
fn dependency_paths(text: &str) -> Result<Vec<PathBuf>> {
    let joined = text.replace("\\\n", " ");
    let line = joined
        .lines()
        .find(|line| !line.is_empty())
        .ok_or("missing actual dep-info")?;
    let (_, paths) = line
        .split_once(": ")
        .ok_or("invalid actual dep-info target")?;
    let mut result = vec![];
    let mut current = String::new();
    let mut escaped = false;
    for ch in paths.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                result.push(PathBuf::from(std::mem::take(&mut current)));
            }
        } else {
            current.push(ch);
        }
    }
    if escaped {
        return Err("truncated dep-info escape".into());
    }
    if !current.is_empty() {
        result.push(PathBuf::from(current));
    }
    if result.is_empty() {
        return Err("empty actual data dependencies".into());
    }
    Ok(result)
}
fn data_image_preflight(bytes: &[u8], main_address: u64) -> Result<()> {
    let (base, _, _) = host_image_preflight(bytes, 2, 0)?;
    require_rx_offset(
        bytes,
        base,
        main_address
            .checked_sub(base)
            .ok_or("data main precedes image")?,
    )?;
    let word = |at: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or("truncated data image")?
                .try_into()
                .map_err(|_| "data word")?,
        ))
    };
    let wide = |at: usize| -> Result<u64> {
        Ok(u64::from_le_bytes(
            bytes
                .get(at..at + 8)
                .ok_or("truncated data image")?
                .try_into()
                .map_err(|_| "data wide")?,
        ))
    };
    let mut at = 32;
    let mut entry_file = None;
    let mut main_file = None;
    for _ in 0..word(16)? {
        let command = word(at)?;
        let size = word(at + 4)? as usize;
        if command == 0x27 {
            return Err("data image contains loader environment command".into());
        }
        if matches!(command, 0x4 | 0x5) {
            return Err("data image contains alternate thread entry".into());
        }
        if command == 0x80000028 {
            if size < 24 || entry_file.is_some() {
                return Err("data image requires exactly one LC_MAIN".into());
            }
            entry_file = Some(wide(at + 8)?);
        }
        if command == 0x19 {
            if bytes.get(at + 8..at + 15) == Some(b"__TEXT\0") {
                let vm = wide(at + 24)?;
                let delta = main_address
                    .checked_sub(vm)
                    .ok_or("data main precedes __TEXT")?;
                if delta >= wide(at + 48)? {
                    return Err("data main is not file-backed __TEXT code".into());
                }
                if main_file
                    .replace(
                        wide(at + 40)?
                            .checked_add(delta)
                            .ok_or("data main file offset overflow")?,
                    )
                    .is_some()
                {
                    return Err("ambiguous data __TEXT main".into());
                }
            }
            for index in 0..word(at + 64)? as usize {
                let section = at + 72 + index * 80;
                let name = bytes
                    .get(section..section + 16)
                    .ok_or("data section bounds")?;
                if name.starts_with(b"__interpose\0") && wide(section + 40)? != 0 {
                    return Err("data image must not contain interposition".into());
                }
            }
        }
        if matches!(
            command,
            0xc | 0x80000018 | 0x8000001f | 0x80000023 | 0x20 | 0xe
        ) {
            let start = at
                .checked_add(word(at + 8)? as usize)
                .ok_or("data dylib name overflow")?;
            let end = at.checked_add(size).ok_or("data command overflow")?;
            let path = bytes.get(start..end).ok_or("data dylib name bounds")?;
            let length = path
                .iter()
                .position(|b| *b == 0)
                .ok_or("unterminated data dylib name")?;
            let path = &path[..length];
            let path_text = std::str::from_utf8(path).map_err(|_| "nonUTF8 data dylib name")?;
            if Path::new(path_text)
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
                || (!path.starts_with(b"/System/Library/") && !path.starts_with(b"/usr/lib/"))
            {
                return Err("data image may link only reviewed OS images".into());
            }
        }
        at += size;
    }
    if entry_file.is_none() || main_file.is_none() || entry_file != main_file {
        return Err("LC_MAIN does not select the reviewed _main code".into());
    }
    Ok(())
}

#[cfg(test)]
mod data_builder_tests {
    use super::*;
    #[test]
    fn cf_context_is_exactly_derived_from_pinned_cap_owner() {
        assert_eq!(data_cf_context(0), "0x0:0x0:0x0");
        assert_eq!(data_cf_context(501), "0x1F5:0x0:0x0");
        assert_eq!(data_cf_context(u32::MAX), "0xFFFFFFFF:0x0:0x0");
    }
    #[test]
    fn real_dependency_roster_is_not_truncated_by_make_escapes() {
        assert_eq!(
            dependency_paths("out: /owned/a\\ b.rs \\\n/owned/c.rs\n\n/owned/a\\ b.rs:\n").unwrap(),
            vec![PathBuf::from("/owned/a b.rs"), PathBuf::from("/owned/c.rs")]
        );
        assert!(dependency_paths("out: ").is_err());
        assert!(dependency_paths("out: /owned/a\\").is_err());
    }
    fn image(interpose: bool, dylib: &str) -> Vec<u8> {
        let command_size = (24 + dylib.len() + 1 + 7) & !7;
        let mut image = vec![0u8; 32 + 152 + command_size + 24];
        let put = |b: &mut [u8], at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut image, 0, 0xfeedfacf);
        put(&mut image, 12, 2);
        put(&mut image, 16, 3);
        put(&mut image, 20, (152 + command_size + 24) as u32);
        put(&mut image, 32, 0x19);
        put(&mut image, 36, 152);
        image[40..46].copy_from_slice(b"__TEXT");
        put(&mut image, 96, 1);
        image[64..72].copy_from_slice(&4096u64.to_le_bytes());
        let length = image.len() as u64;
        image[80..88].copy_from_slice(&length.to_le_bytes());
        put(&mut image, 92, 5);
        if interpose {
            image[104..115].copy_from_slice(b"__interpose");
            image[144..152].copy_from_slice(&16u64.to_le_bytes());
        }
        put(&mut image, 184, 0xc);
        put(&mut image, 188, command_size as u32);
        put(&mut image, 192, 24);
        image[208..208 + dylib.len()].copy_from_slice(dylib.as_bytes());
        let entry = 184 + command_size;
        put(&mut image, entry, 0x80000028);
        put(&mut image, entry + 4, 24);
        image[entry + 8..entry + 16].copy_from_slice(&128u64.to_le_bytes());
        image
    }
    #[test]
    fn data_preexec_forbids_interposition_and_non_os_dependencies() {
        data_image_preflight(&image(false, "/usr/lib/libSystem.B.dylib"), 128).unwrap();
        assert!(data_image_preflight(&image(true, "/usr/lib/libSystem.B.dylib"), 128).is_err());
        assert!(data_image_preflight(&image(false, "@rpath/libRuycastSDK.dylib"), 128).is_err());
        assert!(data_image_preflight(&image(false, "/owned/private.dylib"), 128).is_err());
        assert!(
            data_image_preflight(&image(false, "/usr/lib/../../owned/private.dylib"), 128).is_err()
        );
        let mut loader_env = image(false, "/usr/lib/libSystem.B.dylib");
        loader_env[184..188].copy_from_slice(&0x27u32.to_le_bytes());
        assert!(data_image_preflight(&loader_env, 128).is_err());
    }
    #[test]
    fn actual_main_entry_is_required_without_thread_alternative() {
        let original = image(false, "/usr/lib/libSystem.B.dylib");
        data_image_preflight(&original, 128).unwrap();
        let entry = original.len() - 24;
        let mut wrong = original.clone();
        wrong[entry + 8..entry + 16].copy_from_slice(&129u64.to_le_bytes());
        assert!(data_image_preflight(&wrong, 128).is_err());
        for command in [0x4u32, 0x5] {
            let mut thread = original.clone();
            thread[entry..entry + 4].copy_from_slice(&command.to_le_bytes());
            assert!(data_image_preflight(&thread, 128).is_err());
        }
        let mut missing = original;
        missing[entry..entry + 4].copy_from_slice(&0x2u32.to_le_bytes());
        assert!(data_image_preflight(&missing, 128).is_err());
    }
}

fn drain_data_stdout(
    output: std::os::fd::OwnedFd,
    bytes: &mut Vec<u8>,
    deadline: Instant,
) -> std::io::Result<bool> {
    use std::{io::Read, os::fd::AsRawFd};
    let flags = unsafe { libc::fcntl(output.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(output.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let mut output = fs::File::from(output);
    loop {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "owned stdout drain deadline",
            ));
        }
        let remaining = 4097usize.saturating_sub(bytes.len());
        if remaining == 0 {
            return Ok(false);
        }
        let mut chunk = [0u8; 1024];
        let length = remaining.min(chunk.len());
        match output.read(&mut chunk[..length]) {
            Ok(0) => return Ok(true),
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(2))
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => (),
            Err(error) => return Err(error),
        }
    }
}
fn finish_data_child(
    child: &mut crate::guardian::Endpoint,
    output: std::os::fd::OwnedFd,
    out: &Path,
    budget: std::time::Duration,
) -> Value {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let (original_status, wait_error, wait_error_kind) = match child.process.wait(budget) {
        Ok(status) => (Some(status), None, None),
        Err(error) => (
            None,
            Some(error.to_string()),
            Some(format!("{:?}", error.kind())),
        ),
    };
    let (mut cleanup_status, mut cleanup_error) = (None, None);
    if wait_error.is_some() {
        child.cancel();
        match child.process.terminate() {
            Ok(status) => cleanup_status = Some(status),
            Err(error) => cleanup_error = Some(error.to_string()),
        }
    }
    let mut bytes = Vec::new();
    let (complete, mut capture_error) = match drain_data_stdout(
        output,
        &mut bytes,
        Instant::now() + std::time::Duration::from_secs(2),
    ) {
        Ok(complete) => (complete, None),
        Err(error) => (false, Some(error.to_string())),
    };
    let raw_path = out.join("child.stdout");
    let raw = (|| -> Result<Value> {
        let mut raw = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&raw_path)
            .map_err(|e| e.to_string())?;
        raw.write_all(&bytes)
            .and_then(|_| raw.sync_all())
            .map_err(|e| e.to_string())?;
        drop(raw);
        file_binding(&raw_path)
    })();
    let raw = match raw {
        Ok(binding) => Some(binding),
        Err(error) => {
            capture_error = Some(match capture_error {
                Some(before) => format!("{before}; {error}"),
                None => error,
            });
            None
        }
    };
    json!({"original_wait_status":original_status,"original_wait_error":wait_error,"original_wait_error_kind":wait_error_kind,
        "post_cleanup_wait_status":cleanup_status,"cleanup_error":cleanup_error,"raw_stdout":raw,"stdout_bytes":bytes.len(),
        "stdout_complete":complete,"stdout_capture_error":capture_error,"stdout_limit_bytes":4097,"stdout_drain_deadline_ms":2000})
}

#[cfg(test)]
mod timeout_evidence_tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, time::Duration};
    #[test]
    #[ignore = "requires explicitly pinned neutral fixture and an own vault/.tmp parent"]
    fn neutral_timeout_retains_original_error_and_bounded_output() {
        let binary = PathBuf::from(
            std::env::var_os("RUYCAST_F1_OWNED_TRANSPORT_FIXTURE").expect("explicit safe fixture"),
        );
        assert_eq!(
            sha(&fs::read(&binary).unwrap()),
            "d9408a245b59e6176cd8c55e496e0b55f6204a9fccaf0f5b793c1047dede2b40"
        );
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("explicit own temporary parent"),
        );
        let out = parent.join(format!("timeout-evidence-{}", std::process::id()));
        fs::create_dir(&out).unwrap();
        fs::set_permissions(&out, fs::Permissions::from_mode(0o700)).unwrap();
        let published = crate::host_io::HostIo::new()
            .unwrap()
            .publish(&Value::Null, &Value::Null)
            .unwrap();
        let (mut child, parent) = published
            .spawn_stdio(
                &binary,
                &["no-read".into()],
                &[("HOME".into(), out.display().to_string())],
                &out,
            )
            .unwrap();
        let pid = child.process.id();
        let (input, output) = parent.into_fds();
        drop(input);
        let evidence = finish_data_child(&mut child, output, &out, Duration::from_millis(100));
        write_new(&out.join("receipt.json"),&json!({"scope":"neutral owned timeout only, no Data/SDK","success":false,"child_evidence":evidence})).unwrap();
        assert!(evidence["original_wait_status"].is_null());
        assert_eq!(evidence["original_wait_error_kind"], "TimedOut");
        assert_eq!(evidence["original_wait_error"], "own process did not exit");
        assert!(evidence["post_cleanup_wait_status"].is_i64());
        assert!(evidence["cleanup_error"].is_null());
        assert_eq!(evidence["stdout_complete"], true);
        assert!(evidence["stdout_capture_error"].is_null());
        let bytes = fs::read(out.join("child.stdout")).unwrap();
        assert_eq!(bytes, b"{\"phase\":\"owned-fake-ready\"}\n");
        assert!(bytes.len() <= 4097);
        assert_eq!(
            fs::metadata(out.join("child.stdout"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        let (read, writer) = crate::guardian::pipe().unwrap();
        let mut partial = Vec::new();
        let error = drain_data_stdout(
            read,
            &mut partial,
            Instant::now() + Duration::from_millis(50),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        drop(writer);
        // The explicit operator preserves this small receipt/raw pair, then
        // removes only its own temporary subtree after content-bound review.
        println!("neutral timeout evidence retained at {}", out.display());
    }
}

mod data_diagnostics {
    use super::*;
    use std::{
        io::{Read, Write},
        os::{
            fd::AsRawFd,
            unix::{
                fs::OpenOptionsExt,
                process::CommandExt,
            },
        },
        process::{Child, Stdio},
        time::Duration,
    };
    // Test-only and deliberately accepts an owned Process borrow, never a PID.
    pub(super) fn observe_owned_child(target:&crate::guardian::Process, deadline:Instant,out:&Path)->Value {
        let observation=(||->Result<Value>{
            loop {
                let mut info:libc::siginfo_t=unsafe{std::mem::zeroed()};
                if unsafe{libc::waitid(libc::P_PID,target.id() as u32,&mut info,libc::WEXITED|libc::WNOHANG|libc::WNOWAIT)}!=0{return Err(std::io::Error::last_os_error().to_string());}
                if info.si_pid!=0{return Ok(json!({"sampled":false,"reason":"owned child exited before sampling; remains unreaped","failure":null}));}
                let now=Instant::now();
                if now>=deadline{return Err("data collection deadline before sample".into());}
                if now>=deadline- Duration::from_secs(12) {
                    let sample_deadline=sample_window(now,deadline).ok_or("insufficient remaining sample budget")?;
                    let directory=out.join("sample-diagnostic");
                    fs::create_dir(&directory).map_err(|e|e.to_string())?;
                    let mut sample=sample_owned_stack(target,sample_deadline,&directory)?;
                    if sample["failure"].is_null() && sample["sampler_original_exit_code"]!=0 {
                        sample["failure"]=json!("sampler exited unsuccessfully");
                    }
                    if sample["target_exited"]==true && sample["failure"].is_null() {
                        sample["failure"]=json!("owned target exited during sampling");
                    }
                    return Ok(sample);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        })();
        observation.unwrap_or_else(|error|json!({"sampled":false,"failure":error}))
    }
    fn sample_window(now:Instant,deadline:Instant)->Option<Instant>{
        (deadline.saturating_duration_since(now)>=Duration::from_secs(4)).then(||deadline-Duration::from_secs(2))
    }
    fn sample_owned_stack(
        target: &crate::guardian::Process,
        deadline: Instant,
        out: &Path,
    ) -> Result<Value> {
        const LIMIT: usize = 1024 * 1024;
        if Instant::now() >= deadline {
            return Err("sample absolute deadline before launch".into());
        }
        let pid = target.id();
        let mut target_info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::waitid(
                libc::P_PID,
                pid as u32,
                &mut target_info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if target_info.si_pid != 0 {
            return Err("owned sample target already exited; remains unreaped".into());
        }
        let log = |name| {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(out.join(name))
                .map_err(|e| e.to_string())
        };
        let mut stdout_log = log("sample.stdout")?;
        let mut stderr_log = log("sample.stderr")?;
        let mut command = Command::new("/usr/bin/sample");
        command
            .args([
                pid.to_string(),
                "1".into(),
                "10".into(),
                "-mayDie".into(),
                "-file".into(),
                "/dev/fd/1".into(),
            ])
            .env_clear()
            .env("HOME", out)
            .env("TMPDIR", out)
            .current_dir(out)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        struct OwnedSampler {
            child: Child,
            retired: bool,
        }
        impl OwnedSampler {
            fn retire(&mut self) {
                if !self.retired {
                    unsafe {
                        libc::kill(-(self.child.id() as i32), libc::SIGKILL);
                    };
                    self.retired = true;
                }
            }
        }
        impl Drop for OwnedSampler {
            fn drop(&mut self) {
                self.retire();
                let _ = self.child.wait();
            }
        }
        let mut sampler = OwnedSampler {
            child: command.spawn().map_err(|e| e.to_string())?,
            retired: false,
        };
        let sampler_pid = sampler.child.id();
        let mut stdout = sampler.child.stdout.take().ok_or("sample stdout absent")?;
        let mut stderr = sampler.child.stderr.take().ok_or("sample stderr absent")?;
        for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        fn drain(
            reader: &mut dyn Read,
            log: &mut fs::File,
            bytes: &mut Vec<u8>,
            total: &mut usize,
            deadline: Instant,
        ) -> Result<bool> {
            loop {
                if Instant::now() >= deadline {
                    return Err("sample absolute deadline during drain".into());
                }
                let mut chunk = [0u8; 8192];
                match reader.read(&mut chunk) {
                    Ok(0) => return Ok(true),
                    Ok(count) => {
                        *total = total.checked_add(count).ok_or("sample output overflow")?;
                        if *total > LIMIT {
                            return Err("sample output exceeds1MiB bound".into());
                        }
                        log.write_all(&chunk[..count]).map_err(|e| e.to_string())?;
                        bytes.extend_from_slice(&chunk[..count]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        return Ok(false)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => (),
                    Err(error) => return Err(error.to_string()),
                }
            }
        }
        let (mut out_bytes, mut err_bytes, mut total) = (Vec::new(), Vec::new(), 0usize);
        let (mut out_eof, mut err_eof, mut status) = (false, false, None);
        let mut failure = None;
        loop {
            if Instant::now() >= deadline {
                failure = Some("sample absolute deadline".to_owned());
                break;
            }
            if !out_eof {
                match drain(
                    &mut stdout,
                    &mut stdout_log,
                    &mut out_bytes,
                    &mut total,
                    deadline,
                ) {
                    Ok(eof) => out_eof = eof,
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            if !err_eof {
                match drain(
                    &mut stderr,
                    &mut stderr_log,
                    &mut err_bytes,
                    &mut total,
                    deadline,
                ) {
                    Ok(eof) => err_eof = eof,
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            if status.is_none() {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                if unsafe {
                    libc::waitid(
                        libc::P_PID,
                        sampler_pid,
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                } != 0
                {
                    failure = Some(std::io::Error::last_os_error().to_string());
                    break;
                }
                if info.si_pid == sampler_pid as i32 {
                    sampler.retire();
                    status = Some(sampler.child.wait().map_err(|e| e.to_string())?);
                }
            }
            if status.is_some() && out_eof && err_eof {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let original_status = status.as_ref().and_then(std::process::ExitStatus::code);
        let mut cleanup_status = None;
        if status.is_none() {
            sampler.retire();
            cleanup_status = Some(sampler.child.wait().map_err(|e| e.to_string())?);
        }
        stdout_log.sync_all().map_err(|e| e.to_string())?;
        stderr_log.sync_all().map_err(|e| e.to_string())?;
        // No target wait/reap occurs anywhere in this function, including errors.
        let mut target_after: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::waitid(
                libc::P_PID,
                pid as u32,
                &mut target_after,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } != 0
        {
            return Err("sample target ownership no longer held".into());
        }
        Ok(
            json!({"scope":"owned target thread-stack diagnostic only; not a timing or family-acceptance result","target_pid":pid,"sampler_pid":sampler_pid,
            "sampler_original_exit_code":original_status,"sampler_cleanup_status":cleanup_status.map(|s|s.to_string()),
            "failure":failure,"stdout":file_binding(&out.join("sample.stdout"))?,"stderr":file_binding(&out.join("sample.stderr"))?,
            "stdout_bytes":out_bytes.len(),"stderr_bytes":err_bytes.len(),"stdout_eof":out_eof,"stderr_eof":err_eof,
            "target_still_unreaped":true,"target_exited":target_after.si_pid==pid,"limit_bytes":LIMIT}),
        )
    }
    #[cfg(test)]
    #[test]
    fn diagnostic_window_preserves_absolute_deadline() {
        let now=Instant::now();let deadline=now+Duration::from_secs(15);
        assert_eq!(sample_window(now+Duration::from_secs(3),deadline),Some(deadline-Duration::from_secs(2)));
        assert_eq!(sample_window(now+Duration::from_secs(9),deadline),Some(deadline-Duration::from_secs(2)));
        assert!(sample_window(now+Duration::from_secs(12),deadline).is_none());
        assert!(sample_window(deadline,deadline).is_none());
        assert!(sample_window(deadline+Duration::from_secs(1),deadline).is_none());
    }
    #[cfg(test)]
    #[test]
    #[ignore = "explicit neutral owned fixture; invokes system sample twice, never Data"]
    fn neutral_owned_sample_fd_deadline_and_canary() {
        use std::os::unix::fs::PermissionsExt;
        let binary = PathBuf::from(
            std::env::var_os("RUYCAST_F1_OWNED_TRANSPORT_FIXTURE")
                .expect("explicit neutral fixture"),
        );
        assert_eq!(
            sha(&fs::read(&binary).unwrap()),
            "d9408a245b59e6176cd8c55e496e0b55f6204a9fccaf0f5b793c1047dede2b40"
        );
        assert_eq!(
            sha(&fs::read("/usr/bin/sample").unwrap()),
            "cb256ff7588e0767aa9f72b1c2471aba71896ccc7e5662134a65aaa2dd03485a"
        );
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("explicit own temporary parent"),
        );
        let out = parent.join(format!("sample-qualification-{}", std::process::id()));
        fs::create_dir(&out).unwrap();
        fs::set_permissions(&out, fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["normal", "deadline", "expired"] {
            fs::create_dir(out.join(name)).unwrap();
        }
        let canary = "RUYCAST_NEUTRAL_ENV_CANARY_71C26B_PRIVATE_TEST_ONLY";
        let published = crate::host_io::HostIo::new()
            .unwrap()
            .publish(&Value::Null, &Value::Null)
            .unwrap();
        let absolute = Instant::now() + Duration::from_secs(15);
        let (mut child, parent_io) = published
            .spawn_stdio(
                &binary,
                &["no-read".into()],
                &[
                    ("HOME".into(), out.display().to_string()),
                    ("RUYCAST_NEUTRAL_CANARY".into(), canary.into()),
                ],
                &out,
            )
            .unwrap();
        let target_pid = child.process.id();
        let (input, output) = parent_io.into_fds();
        // Parent keeps both its own input and the unreaped Process throughout both samples.
        let normal = sample_owned_stack(&child.process, absolute, &out.join("normal")).unwrap();
        write_new(&out.join("normal.json"), &normal).unwrap();
        let bounded = sample_owned_stack(
            &child.process,
            Instant::now() + Duration::from_millis(50),
            &out.join("deadline"),
        )
        .unwrap();
        write_new(&out.join("deadline.json"), &bounded).unwrap();
        let expired = sample_owned_stack(
            &child.process,
            Instant::now() - Duration::from_millis(1),
            &out.join("expired"),
        )
        .unwrap_err();
        drop(input);
        child.cancel();
        let cleanup = child.process.terminate().unwrap();
        let cleanup_evidence =
            finish_data_child(&mut child, output, &out, Duration::from_millis(10));
        write_new(&out.join("receipt.json"),&json!({"scope":"neutral sampling only; no Data","normal":normal,"deadline":bounded,"expired":expired,"target_pid":target_pid,"target_cleanup_status":cleanup,"cleanup":cleanup_evidence,"canary_absence_scope":"one synthetic environment value only; report can contain process paths, UID and binary-image metadata"})).unwrap();
        assert!(normal["failure"].is_null(), "{normal}");
        assert_eq!(normal["sampler_original_exit_code"], 0);
        assert_eq!(normal["stdout_eof"], true);
        assert_eq!(normal["stderr_eof"], true);
        assert_eq!(normal["target_still_unreaped"], true);
        assert_eq!(normal["target_exited"], false);
        assert!(bounded["failure"]
            .as_str()
            .unwrap()
            .contains("absolute deadline"));
        assert!(bounded["sampler_original_exit_code"].is_null());
        assert!(bounded["sampler_cleanup_status"].is_string());
        assert_eq!(bounded["target_still_unreaped"], true);
        assert_eq!(expired, "sample absolute deadline before launch");
        assert_eq!(fs::read_dir(out.join("expired")).unwrap().count(), 0);
        for phase in ["normal", "deadline"] {
            let mut total = 0;
            for name in ["sample.stdout", "sample.stderr"] {
                let path = out.join(phase).join(name);
                let bytes = fs::read(&path).unwrap();
                total += bytes.len();
                assert!(!bytes
                    .windows(canary.len())
                    .any(|part| part == canary.as_bytes()));
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            assert!(total <= 1024 * 1024);
            assert_eq!(
                fs::read_dir(out.join(phase)).unwrap().count(),
                2,
                "no default report in explicit private TMPDIR"
            );
        }
        let report = fs::read_to_string(out.join("normal/sample.stdout")).unwrap();
        assert!(
            report.contains("Call graph:") || report.contains("Call graph"),
            "FD1 must contain actual stack report"
        );
        for result in [&normal, &bounded] {
            let pid = result["sampler_pid"].as_u64().unwrap() as i32;
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
        assert_eq!(unsafe { libc::kill(target_pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        println!("neutral sample evidence retained at {}", out.display());
    }
}

#[cfg(test)]
mod notification_runtime_tests {
    use super::*;
    fn artifacts(output:&std::process::Output)->Result<Vec<Value>> {
        output.stdout.split(|byte|*byte==b'\n').filter(|line|!line.is_empty())
            .map(|line|serde_json::from_slice::<Value>(line).map_err(|_|"Cargo artifact JSON".into()))
            .collect()
    }
    #[test]
    #[ignore="explicit isolated compiler/payload contract; needs owned fixture parent and a fresh Foundation compiler invocation"]
    fn owned_notification_uses_authentic_compiler_inputs() {
        run().unwrap();
    }
    fn run()->Result<()> {
        use std::os::unix::fs::{MetadataExt,PermissionsExt};
        let root=fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .map_err(|e|e.to_string())?;
        let parent=PathBuf::from(std::env::var_os("RUYCAST_F1_WORKSPACE_FIXTURE_PARENT")
            .ok_or("explicit owned fixture parent required")?);
        if !parent.is_absolute()||fs::canonicalize(&parent).map_err(|e|e.to_string())?!=parent {
            return Err("Notification fixture parent must be canonical absolute path".into());
        }
        let owner=unsafe{libc::getuid()};let meta=fs::symlink_metadata(&parent).map_err(|e|e.to_string())?;
        if !meta.is_dir()||meta.uid()!=owner||meta.mode()&0o022!=0 {
            return Err("Notification fixture parent ownership/access differs".into());
        }
        let out=parent.join(format!("notification-compiler-{}",std::process::id()));
        fs::create_dir(&out).map_err(|e|e.to_string())?;
        fs::set_permissions(&out,fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
        let (cargo,tools)=native_toolchain(&root,&out,None)?;
        let rustc=Path::new(tools["rustc"]["binary"]["path"].as_str().ok_or("selected rustc absent")?);
        let before=nsstring_compile_inputs(&root,&cargo,owner)?;
        let target=host_compiler_target(&root)?;
        let target_dir=root.join("rust-native/target/native-tools");
        let sources_before=inventory(&root)?;
        let configure=|command:&mut Command| {
            command.current_dir(&root).args(["--locked","--offline","--manifest-path"])
                .arg(root.join("bench/f1-runner/Cargo.toml"))
                .args(["--target",&target,"--target-dir"]).arg(&target_dir)
                .args(["--lib","-vv","--message-format=json"])
                .env("CARGO_INCREMENTAL","0").env("RUSTC",rustc).env("SDKROOT",&before.sdk_root)
                .env("RUSTC_WRAPPER","").env("RUSTC_WORKSPACE_WRAPPER","")
                .env_remove("RUSTFLAGS").env_remove("CARGO_ENCODED_RUSTFLAGS")
                .env_remove("RUYCAST_F1_NSSTRING_DECLARATION_PINS");
        };
        let mut dependency=Command::new(&cargo);dependency.arg("build");configure(&mut dependency);
        let built=native_tool(&mut dependency,&root)?;
        native_private_bytes(&out.join("dependencies.json"),&serde_json::to_vec_pretty(
            &native_cargo_summary(&built.stdout,&built.stderr)?).map_err(|e|e.to_string())?)?;
        if !built.status.success(){return Err("Notification dependency build failed".into());}
        let records=artifacts(&built)?;
        let foundation=records.iter().filter(|v|v["reason"]=="compiler-artifact"&&v["target"]["name"]=="objc2_foundation")
            .flat_map(|v|v["filenames"].as_array().into_iter().flatten())
            .filter_map(Value::as_str).filter(|p|p.ends_with(".rlib")).collect::<Vec<_>>();
        if foundation.len()!=1 {return Err("Notification exact Foundation artifact absent".into());}
        let foundation=fs::canonicalize(foundation[0]).map_err(|e|e.to_string())?;
        let foundation_binding=file_binding(&foundation)?;
        let commands=native_rustc_commands(std::str::from_utf8(&built.stderr).map_err(|_|"compiler display UTF8")?)?;
        let literal=nsstring_compile_pins(&root,&out,&before,&commands,&foundation,rustc,true)?;
        let mut tests=Command::new(&cargo);tests.args(["test","--no-run"]);configure(&mut tests);
        tests.env("RUYCAST_F1_NSSTRING_DECLARATION_PINS",&literal);
        let compiled=native_tool(&mut tests,&root)?;
        native_private_bytes(&out.join("tests-compile.json"),&serde_json::to_vec_pretty(
            &native_cargo_summary(&compiled.stdout,&compiled.stderr)?).map_err(|e|e.to_string())?)?;
        if !compiled.status.success(){return Err("Notification test compilation failed".into());}
        let calls=native_rustc_commands(std::str::from_utf8(&compiled.stderr).map_err(|_|"test compiler display UTF8")?)?;
        let sdk=calls.iter().filter(|args|args.windows(2).any(|w|w[0]=="--crate-name"&&w[1]=="ruycast_f1_sdk")
            &&args.iter().any(|arg|arg=="--test")).collect::<Vec<_>>();
        if sdk.len()!=1||!sdk[0].windows(2).any(|w|w[0]=="--extern"
            &&w[1]==format!("objc2_foundation={}",foundation.display())) {
            return Err("Notification test compiler does not consume the actual Foundation output".into());
        }
        let records=artifacts(&compiled)?;
        let executables=records.iter().filter(|v|v["reason"]=="compiler-artifact"
            &&v["target"]["name"]=="ruycast_f1_sdk"&&v["profile"]["test"]==true)
            .filter_map(|v|v["executable"].as_str()).collect::<Vec<_>>();
        if executables.len()!=1 {return Err("Notification exact test executable absent".into());}
        let executable=fs::canonicalize(executables[0]).map_err(|e|e.to_string())?;
        if !executable.starts_with(&target_dir)||file_binding(&foundation)?!=foundation_binding
            ||nsstring_compile_pins(&root,&out,&before,&commands,&foundation,rustc,false)?!=literal
            ||inventory(&root)?.files!=sources_before.files {
            return Err("Notification compiler/source/artifact bindings changed before execution".into());
        }
        let executable_binding=file_binding(&executable)?;
        native_private_bytes(&out.join("test-compiler.json"),&serde_json::to_vec_pretty(&json!({
            "compiler":tools["rustc"],"arguments":sdk[0],"executable":executable_binding,
            "foundation":foundation_binding,"source_before":sources_before,"runtime_authority":false
        })).map_err(|e|e.to_string())?)?;
        let mut probe=Command::new(&executable);
        probe.args(["--ignored","--exact","workspace::delegate::tests::owned_notification_payload_dispatch_and_retirement","--nocapture"])
            .env("RUYCAST_F1_WORKSPACE_FIXTURE_PARENT",&parent);
        let executed=native_tool(&mut probe,&root)?;
        native_private_bytes(&out.join("execution.stdout"),&executed.stdout)?;
        native_private_bytes(&out.join("execution.stderr"),&executed.stderr)?;
        if !executed.status.success()||!String::from_utf8_lossy(&executed.stdout).contains("1 passed; 0 failed; 0 ignored;") {
            return Err(format!("Owned Notification execution failed; primary output at {}",out.display()));
        }
        if file_binding(&executable)?!=executable_binding||file_binding(&foundation)?!=foundation_binding
            ||nsstring_compile_pins(&root,&out,&before,&commands,&foundation,rustc,false)?!=literal
            ||inventory(&root)?.files!=sources_before.files {
            return Err("Notification execution changed its source/artifact bindings".into());
        }
        println!("owned Notification probe passed; primary compiler/execution evidence at {}",out.display());
        Ok(())
    }
}

#[cfg(test)]
mod reference_guard_argument_tests {
    use super::*;
    #[test]
    fn whole_capture_deadline_preserves_tool_ceiling_and_refuses_spawn_after_expiry() {
        let now=Instant::now();
        assert_eq!(compiler_tool_deadline(now,None),now+std::time::Duration::from_secs(900));
        assert_eq!(compiler_tool_deadline(now,Some(now+std::time::Duration::from_secs(1200))),
            now+std::time::Duration::from_secs(900));
        assert_eq!(compiler_tool_deadline(now,Some(now+std::time::Duration::from_secs(3))),
            now+std::time::Duration::from_secs(3));
        let mut command=Command::new("/never-spawned/whole-budget-test");
        let error=native_tool_until(&mut command,Path::new("/never-read"),
            now-std::time::Duration::from_secs(1)).unwrap_err();
        assert!(error.contains("whole Reference capture absolute deadline"));
        assert_eq!(REFERENCE_CAPTURE_SECONDS,1200);
    }
    fn legacy(source: &Path, object: &Path) -> Vec<std::ffi::OsString> {
        vec!["--edition=2021".into(), "--crate-type=lib".into(), "--emit=obj".into(),
            "-O".into(), "-Cpanic=abort".into(), source.as_os_str().to_owned(),
            "-o".into(), object.as_os_str().to_owned()]
    }
    #[test]
    fn partial_bridge_preserves_all_previous_arguments() {
        let source=Path::new("reference/ReferencePreentry.rs");
        let object=Path::new("owned-output/ReferencePreentry.o");
        assert_eq!(reference_guard_arguments(source,object,false),legacy(source,object));
    }
    #[test]
    fn full_host_adds_exact_reference_cfg_and_object_debug_information() {
        let source=Path::new("reference/ReferencePreentry.rs");
        let object=Path::new("owned-output/ReferencePreentry.o");
        let mut expected=legacy(source,object);
        expected.extend(["--cfg".into(),"feature=\"full-reference-host\"".into(),"-Cdebuginfo=2".into()]);
        assert_eq!(reference_guard_arguments(source,object,true),expected);
    }
    #[test]
    fn fake_main_preentry_keeps_full_host_symbols_disabled() {
        let args=reference_guard_arguments(Path::new("fake/ReferencePreentry.rs"),
            Path::new("fake/ReferencePreentry.o"),false);
        assert_eq!(args,legacy(Path::new("fake/ReferencePreentry.rs"),Path::new("fake/ReferencePreentry.o")));
        assert!(!args.iter().any(|arg|arg=="--cfg" || arg.to_string_lossy().contains("full-reference-host")));
    }

}
