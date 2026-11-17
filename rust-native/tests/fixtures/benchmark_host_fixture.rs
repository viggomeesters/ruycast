use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    os::unix::{fs::OpenOptionsExt, net::UnixListener},
    path::PathBuf,
    process,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

static TERMINATE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_terminate(_: libc::c_int) {
    TERMINATE.store(true, Ordering::SeqCst);
}

fn mode() -> String {
    std::env::args_os()
        .next()
        .and_then(|path| PathBuf::from(path).file_name().map(|name| name.to_owned()))
        .and_then(|name| {
            name.to_str()
                .map(|name| name.rsplit('-').next().unwrap_or("ok").to_owned())
        })
        .unwrap_or_else(|| "ok".into())
}

fn event(value: &str) {
    let Some(path) = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|executable| executable.parent().map(PathBuf::from))
        .map(|root| root.join("benchmark-events.txt"))
    else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
    {
        let _ = writeln!(file, "{value}");
    }
}

fn sleep_forever() -> ! {
    loop {
        thread::sleep(Duration::from_secs(30));
    }
}

fn init_config(root: &std::path::Path) {
    if mode() == "init_hang" {
        sleep_forever();
    }
    fs::write(root.join("config.json"), br#"{"commands":[]}"#).unwrap();
}

fn respond(stream: &mut std::os::unix::net::UnixStream, reply: &Value) -> bool {
    serde_json::to_writer(&mut *stream, reply).unwrap();
    stream.write_all(b"\n").is_ok()
}

fn host() {
    unsafe {
        libc::signal(libc::SIGTERM, on_terminate as libc::sighandler_t);
    }
    let root = PathBuf::from(std::env::var_os("RUYCAST_HOME").unwrap());
    let socket = root.join("control.sock");
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let fixture_mode = mode();
    let mut last_request = Instant::now();
    event("started");
    loop {
        if TERMINATE.load(Ordering::SeqCst) && fixture_mode != "quit_ignores_terminate" {
            event("terminate");
            return;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut line = String::new();
                if BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .is_err()
                {
                    continue;
                }
                let request: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let operation = request["op"].as_str().unwrap_or("");
                let now = Instant::now();
                if fixture_mode == "clipboard_hang" && operation == "clipboard.list" {
                    event("clipboard-hang");
                    sleep_forever();
                }
                if matches!(fixture_mode.as_str(), "idle_hang" | "idle_drip")
                    && operation == "status"
                    && now.duration_since(last_request) >= Duration::from_millis(100)
                {
                    event("idle-status-hang");
                    if fixture_mode == "idle_hang" {
                        sleep_forever();
                    }
                    loop {
                        if stream.write_all(b" ").is_err() {
                            return;
                        }
                        thread::sleep(Duration::from_millis(50));
                    }
                }
                last_request = Instant::now();
                let reply = match operation {
                    "status" => {
                        json!({"ok":true,"result":{"headless":true,"configPath":root.join("config.json"),"commands":500,"clipboardCount":200}})
                    }
                    "search" => json!({"ok":true,"result":[{"id":"fixture.42"}]}),
                    "quit" => {
                        event("quit");
                        if !matches!(
                            fixture_mode.as_str(),
                            "quit_stays_alive" | "quit_ignores_terminate"
                        ) {
                            let _ = respond(&mut stream, &json!({"ok":true,"result":{}}));
                            return;
                        }
                        json!({"ok":true,"result":{}})
                    }
                    _ => json!({"ok":true,"result":[]}),
                };
                let _ = respond(&mut stream, &reply);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return,
        }
    }
}

fn cli(arguments: &[String]) {
    if mode() == "cli_hang" {
        sleep_forever();
    }
    if mode() == "cli_change" && arguments.first().is_some_and(|arg| arg == "status") {
        let path = PathBuf::from(std::env::args_os().next().unwrap());
        let parent = path.parent().unwrap();
        let counter = parent.join("mutation-count");
        let count = fs::read_to_string(&counter)
            .unwrap()
            .parse::<u32>()
            .unwrap()
            + 1;
        fs::write(counter, count.to_string()).unwrap();
        if count == 11 {
            let replacement = parent.join("mutation-replacement");
            fs::rename(replacement, path).unwrap();
        }
    }
    println!("{{\"ok\":true}}");
}

fn probe() {
    if mode() == "probe_hang" {
        sleep_forever();
    }
    println!(
        "{}",
        json!({
            "cpuNS": 0,
            "packageIdleWakeups": 0,
            "interruptWakeups": 0,
            "footprintBytes": 1_048_576,
            "diskReadBytes": 0,
            "diskWriteBytes": 0,
            "logicalWriteBytes": 0
        })
    );
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.iter().any(|value| value == "--init-config") {
        let root = PathBuf::from(std::env::var_os("RUYCAST_HOME").unwrap());
        init_config(&root);
        return;
    }
    if arguments.iter().any(|value| value == "--headless") {
        host();
        return;
    }
    if arguments
        .first()
        .is_some_and(|value| value.parse::<u32>().is_ok())
    {
        probe();
        return;
    }
    cli(&arguments);
    process::exit(0);
}
