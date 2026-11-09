use super::*;
use std::{
    io::{Read, Write},
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub struct Call {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub input: Vec<u8>,
    pub env: BTreeMap<String, String>,
    pub timeout: Duration,
    pub cwd: Option<PathBuf>,
}
impl Call {
    pub fn new(
        program: impl Into<PathBuf>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            input: vec![],
            env: BTreeMap::new(),
            timeout: Duration::from_secs(30),
            cwd: None,
        }
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn cwd(mut self, cwd: &Path) -> Self {
        self.cwd = Some(cwd.to_owned());
        self
    }
    pub fn env(mut self, home: &Path, profile: Option<&Path>, config: Option<&str>) -> Self {
        self.env.insert("HOME".into(), home.display().to_string());
        self.env.insert(
            "PATH".into(),
            "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin".into(),
        );
        if let Some(profile) = profile {
            self.env
                .insert("RUYCAST_HOME".into(), profile.display().to_string());
        }
        if let Some(config) = config {
            self.env.insert("RUYCAST_CONFIG_DIR".into(), config.into());
        }
        self
    }
}
pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
impl Output {
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.stdout.clone()).map_err(err)
    }
    pub fn json(&self) -> Result<Value> {
        serde_json::from_slice(&self.stdout).map_err(err)
    }
}
pub trait Runner {
    fn run(&mut self, call: Call) -> Result<Output>;
    fn secret(&mut self) -> Result<String>;
    fn running_host(&mut self, pid: i32, expected: &Path) -> Result<()>;
}
pub fn checked(r: &mut dyn Runner, call: Call) -> Result<Output> {
    let name = call
        .program
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("tool")
        .to_owned();
    let output = r.run(call)?;
    if !output.success {
        let authorization =
            String::from_utf8_lossy(&output.stderr).contains("CSSMERR_TP_NOT_TRUSTED");
        return Err(format!(
            "{name}: operation failed{}; installation state preserved or rolled back.",
            if authorization {
                " (certificate needs local code-signing authorization)"
            } else {
                ""
            }
        ));
    }
    Ok(output)
}
pub struct System;
impl Runner for System {
    fn running_host(&mut self, pid: i32, expected: &Path) -> Result<()> {
        use ruycast_native::processes::{Native, Transport};
        let before = Native
            .identity(pid)
            .ok_or("Installed host identity is unavailable.")?;
        if before.uid != unsafe { libc::getuid() } {
            return Err("Installed host belongs to another user.".into());
        }
        let path = Native
            .path(pid)
            .ok_or("Installed host executable is unavailable.")?;
        if Native.identity(pid) != Some(before) || Path::new(&path) != expected {
            return Err(
                "Readiness came from a different host executable or process identity.".into(),
            );
        }
        Ok(())
    }
    fn secret(&mut self) -> Result<String> {
        let mut data = [0u8; 32];
        if unsafe { libc::getentropy(data.as_mut_ptr().cast(), data.len()) } != 0 {
            return Err("Cannot obtain signing entropy.".into());
        }
        Ok(data.iter().map(|b| format!("{b:02x}")).collect())
    }
    fn run(&mut self, call: Call) -> Result<Output> {
        if !call.program.is_absolute() || call.input.len() > 32768 {
            return Err("Invalid installation command.".into());
        }
        let mut command = Command::new(&call.program);
        command
            .args(&call.args)
            .env_clear()
            .envs(&call.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        if let Some(cwd) = &call.cwd {
            command.current_dir(cwd);
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Could not start installation tool.".to_string())?;
        let mut input = child.stdin.take();
        let mut out = child.stdout.take().unwrap();
        let mut error = child.stderr.take().unwrap();
        struct Owned {
            child: std::process::Child,
            retired: bool,
        }
        impl Owned {
            fn retire(&mut self) {
                if !self.retired {
                    // No wait/try_wait has reaped this owned leader. Its PID
                    // still pins the group identity even after leader exit.
                    unsafe {
                        libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
                    }
                    self.retired = true;
                }
            }
            fn exited_unreaped(&self) -> Result<bool> {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        self.child.id(),
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                };
                if result != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        return Ok(false);
                    }
                    return Err(err(error));
                }
                Ok(info.si_pid == self.child.id() as libc::pid_t)
            }
        }
        impl Drop for Owned {
            fn drop(&mut self) {
                self.retire();
                let _ = self.child.wait();
            }
        }
        let mut owned = Owned {
            child,
            retired: false,
        };
        for fd in [
            input.as_ref().unwrap().as_raw_fd(),
            out.as_raw_fd(),
            error.as_raw_fd(),
        ] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err("Could not prepare bounded installation I/O.".into());
            }
        }
        let mut at = 0;
        let mut stdout = vec![];
        let mut stderr = vec![];
        let mut eof = [false; 2];
        let deadline = Instant::now() + call.timeout;
        let mut buffer = [0; 8192];
        loop {
            if Instant::now() >= deadline {
                return Err("Installation tool timed out; outcome may require rollback.".into());
            }
            if let Some(stream) = input.as_mut() {
                match stream.write(&call.input[at..]) {
                    Ok(n) => at += n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => return Err("Installation tool input failed.".into()),
                }
                if at == call.input.len() {
                    input = None;
                }
            }
            for (i, pipe, data) in [
                (0, &mut out as &mut dyn Read, &mut stdout),
                (1, &mut error as &mut dyn Read, &mut stderr),
            ] {
                if eof[i] {
                    continue;
                }
                match pipe.read(&mut buffer) {
                    Ok(0) => eof[i] = true,
                    Ok(n) => {
                        if data.len() + n > 1_048_576 {
                            return Err("Installation tool output exceeded its bound.".into());
                        }
                        data.extend_from_slice(&buffer[..n]);
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => return Err("Installation tool output failed.".into()),
                }
            }
            if eof == [true, true] && owned.exited_unreaped()? {
                owned.retire();
                let status = owned.child.wait().map_err(err)?;
                return Ok(Output {
                    success: status.success(),
                    stdout,
                    stderr,
                });
            }
            let mut readiness = [
                libc::pollfd {
                    fd: input.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                    events: libc::POLLOUT,
                    revents: 0,
                },
                libc::pollfd {
                    fd: if eof[0] { -1 } else { out.as_raw_fd() },
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: if eof[1] { -1 } else { error.as_raw_fd() },
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(2));
            let ready = unsafe {
                libc::poll(
                    readiness.as_mut_ptr(),
                    readiness.len() as libc::nfds_t,
                    wait.as_nanos().div_ceil(1_000_000) as i32,
                )
            };
            if (ready < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted)
                || readiness.iter().any(|fd| fd.revents & libc::POLLNVAL != 0)
            {
                return Err("Could not wait for bounded installation I/O.".into());
            }
        }
    }
}
pub fn plist(r: &mut dyn Runner, path: &Path) -> Result<Value> {
    fs::no_links(path)?;
    let _ = fs::bytes(path)?;
    let output = checked(
        r,
        Call::new(
            "/usr/bin/plutil",
            [
                "-convert",
                "json",
                "-o",
                "-",
                "--",
                &path.display().to_string(),
            ],
        ),
    )?;
    output.json()
}
pub fn plist_bytes(value: &Value) -> Result<Vec<u8>> {
    fn escape(s: &str) -> Result<String> {
        if s.chars().any(|c| {
            (c < ' ' && !matches!(c, '\n' | '\r' | '\t')) || c == '\u{fffe}' || c == '\u{ffff}'
        }) {
            return Err("Invalid property-list text.".into());
        }
        Ok(s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;"))
    }
    fn xml(v: &Value) -> Result<String> {
        Ok(match v {
            Value::String(s) => format!("<string>{}</string>", escape(s)?),
            Value::Bool(b) => if *b { "<true/>" } else { "<false/>" }.into(),
            Value::Number(n) if n.is_u64() => format!("<integer>{n}</integer>"),
            Value::Array(a) => format!(
                "<array>{}</array>",
                a.iter().map(xml).collect::<Result<Vec<_>>>()?.join("")
            ),
            Value::Object(m) => format!(
                "<dict>{}</dict>",
                m.iter()
                    .map(|(k, v)| Ok(format!("<key>{}</key>{}", escape(k)?, xml(v)?)))
                    .collect::<Result<Vec<_>>>()?
                    .join("")
            ),
            _ => return Err("Unsupported property-list value.".into()),
        })
    }
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">{}</plist>\n",xml(value)?).into_bytes())
}
