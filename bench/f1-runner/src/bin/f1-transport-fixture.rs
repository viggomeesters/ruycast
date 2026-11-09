//! Owned transport fake only. No product or SDK family is loaded here.
#[path = "../guardian.rs"]
pub mod guardian;
use std::{
    fs,
    io::{self, BufRead, Read, Write},
    os::fd::FromRawFd,
    path::Path,
    time::Duration,
};

fn grant_snapshot(
    guard: &guardian::Guard,
    name: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let fd: i32 = std::env::var(format!("F1_TEST_{name}_FD"))?.parse()?;
    let expected: serde_json::Value =
        serde_json::from_str(&std::env::var(format!("F1_TEST_{name}_PIN"))?)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    guard.before_effect()?;
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if fd <= 2 || flags < 0 || unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err("own grant missing".into());
    }
    let stat = unsafe { stat.assume_init() };
    let actual = serde_json::json!({"device":stat.st_dev as u64,"inode":stat.st_ino,"kind":u32::from(stat.st_mode&libc::S_IFMT),"access":flags&libc::O_ACCMODE});
    if actual != expected
        || stat.st_mode & libc::S_IFMT != libc::S_IFIFO
        || flags & libc::O_ACCMODE != libc::O_RDONLY
    {
        return Err("own inherited grant differs".into());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut file = unsafe { fs::File::from_raw_fd(fd) };
    let mut bytes = vec![];
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        guard.before_effect()?;
        if std::time::Instant::now() >= deadline {
            return Err("own grant EOF deadline".into());
        }
        let mut buffer = [0u8; 1024];
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&buffer[..n]);
                if bytes.len() > 16388 {
                    return Err("own grant exceeds framing bound".into());
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2))
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => (),
            Err(e) => return Err(e.into()),
        }
    }
    if bytes.len() < 4 {
        return Err("own grant incomplete".into());
    }
    let length = u32::from_ne_bytes(bytes[..4].try_into()?) as usize;
    if length == 0 || length > 16384 || bytes.len() != length + 4 {
        return Err("own grant framing/EOF differs".into());
    }
    Ok(serde_json::from_slice(&bytes[4..])?)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1).ok_or("mode missing")?;
    let guard = guardian::Guard::bootstrap()?;
    guard.before_effect()?;
    let home = std::env::var("HOME")?;
    let home = Path::new(&home);
    if mode == "descendant" {
        fs::write(home.join("descendant.pid"), std::process::id().to_string())?;
        loop {
            guard.before_effect()?;
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    if mode == "read-grants" {
        let sdk = grant_snapshot(&guard, "SDK")?;
        let native = if std::env::var_os("F1_TEST_NATIVE_FD").is_some() {
            Some(grant_snapshot(&guard, "NATIVE")?)
        } else {
            None
        };
        let blocked_absent = if let Ok(value) = std::env::var("F1_TEST_BLOCKED_FD") {
            let fd: i32 = value.parse()?;
            let expected: serde_json::Value =
                serde_json::from_str(&std::env::var("F1_TEST_BLOCKED_PIN")?)?;
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
                true
            } else {
                let stat = unsafe { stat.assume_init() };
                serde_json::json!({"device":stat.st_dev as u64,"inode":stat.st_ino,"kind":u32::from(stat.st_mode&libc::S_IFMT),"access":flags&libc::O_ACCMODE})
                    != expected
            }
        } else {
            true
        };
        if !blocked_absent {
            return Err("unlisted private grant was inherited".into());
        }
        println!(
            "{}",
            serde_json::json!({"phase":"owned-fake-ready","sdk":sdk,"native":native,"blocked_absent":blocked_absent})
        );
        io::stdout().flush()?;
        return Ok(());
    }
    let _descendant = if matches!(mode.as_str(), "escaped" | "partial" | "drop" | "panic-drop") {
        Some(guardian::Endpoint::start(
            &std::env::current_exe()?,
            &["descendant".into()],
            &[("HOME".into(), home.display().to_string())],
        )?)
    } else {
        None
    };
    println!("{{\"phase\":\"owned-fake-ready\"}}");
    io::stdout().flush()?;
    if mode == "eof" {
        return Ok(());
    }
    if matches!(mode.as_str(), "no-read" | "escaped" | "drop" | "panic-drop") {
        loop {
            guard.before_effect()?;
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    guard.before_effect()?;
    if mode == "malformed" {
        println!("invalid-json");
    } else if mode == "partial" {
        print!("{{\"unterminated\":");
        io::stdout().flush()?;
        loop {
            guard.before_effect()?;
            std::thread::sleep(Duration::from_millis(5));
        }
    } else if mode == "echo" {
        let request: serde_json::Value = serde_json::from_str(&line)?;
        println!(
            "{}",
            serde_json::json!({"reply":request,"cwd":std::env::current_dir()?,"environment_cleared":std::env::var_os("F1_FORBIDDEN_SELFTEST").is_none(),"guard_alive":guard.is_alive()})
        );
    } else {
        return Err("unknown own fixture mode".into());
    }
    io::stdout().flush()?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("owned transport fixture: {error}");
        std::process::exit(2);
    }
}
