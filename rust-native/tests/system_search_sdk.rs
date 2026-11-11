//! Owned process integration: install the complete SDK substitute before the
//! Rust provider can perform any application discovery or native message.
#![cfg(feature = "test-tools")]
use sha2::{Digest, Sha256};
use serde_json::{json,Value};
use std::{io::{BufRead,BufReader,Read,Write},os::unix::net::UnixStream,process::{Child,Stdio},time::{Duration,Instant}};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[test]
fn optional_main_safari_deadline_keeps_real_cli_local_results_available() {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixStream,
        time::{Duration, Instant},
    };
    let native = root();
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| native.join("target"));
    let base = parent.join(format!("os-rpc-{}", std::process::id()));
    fs::create_dir(&base).unwrap();
    let profile = base.join("p");
    fs::create_dir(&profile).unwrap();
    let applications = base.join("Applications");
    fs::create_dir(&applications).unwrap();
    fs::write(
        base.join(".ruycast-test-fixture"),
        b"own-disposable-fixture-v1",
    )
    .unwrap();
    fs::copy(
        native.parent().unwrap().join("config/default.json"),
        profile.join("config.json"),
    )
    .unwrap();
    let socket = profile.join("control.sock");
    fn rpc(socket: &Path, value: serde_json::Value) -> Result<serde_json::Value, String> {
        let mut stream = UnixStream::connect(socket).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        serde_json::to_writer(&mut stream, &value).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&line).map_err(|e| e.to_string())
    }
    let binary = env!("CARGO_BIN_EXE_ruycast-native");
    let child = Command::new(binary)
        .arg("--headless-test")
        .env_clear()
        .env("HOME", &base)
        .env("RUYCAST_HOME", &profile)
        .env("RUYCAST_APP_BINARY", binary)
        .env(
            "RUYCAST_NATIVE_TEST_APP_ROOTS",
            serde_json::to_string(&vec![&applications]).unwrap(),
        )
        .env("RUYCAST_NATIVE_TEST_SYSTEM_SEARCH", "owned")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    struct Owned {
        child: std::process::Child,
        socket: PathBuf,
        base: PathBuf,
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = rpc(&self.socket, serde_json::json!({"op":"quit"}));
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if self.child.try_wait().ok().flatten().is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = fs::remove_dir_all(&self.base);
        }
    }
    let mut owned = Owned {
        child,
        socket: socket.clone(),
        base,
    };
    let ready = Instant::now() + Duration::from_secs(6);
    loop {
        if rpc(&socket, serde_json::json!({"op":"status"})).is_ok() {
            break;
        }
        assert!(
            owned.child.try_wait().unwrap().is_none(),
            "Owned headless host failed"
        );
        assert!(
            Instant::now() < ready,
            "Own socket did not become ready; use the documented short fixture parent"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let cli = native.parent().unwrap().join(".build/rust/release/ruycast");
    let invoke = |arguments: &[&str]| {
        let result = Command::new(&cli)
            .args(arguments)
            .env_clear()
            .env("HOME", &owned.base)
            .env("RUYCAST_HOME", &profile)
            .env("RUYCAST_APP_BINARY", binary)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()
    };
    let windows = invoke(&["search", "", "--mode", "windows"]);
    let first = &windows["result"][0];
    assert_eq!(first["window"]["application"], "Owned Editor");
    assert_eq!(invoke(&["run", first["id"].as_str().unwrap()])["ok"], true);
    assert_eq!(
        rpc(&socket, serde_json::json!({"op":"system.fixture"})).unwrap()["result"]["effects"],
        serde_json::json!(["activate", "AXRaise"])
    );
    assert_eq!(
        rpc(
            &socket,
            serde_json::json!({"op":"system.fixture","action":"clear-effects"})
        )
        .unwrap()["ok"],
        true
    );
    assert_eq!(
        rpc(
            &socket,
            serde_json::json!({"op":"system.fixture","action":"delay","milliseconds":1500})
        )
        .unwrap()["ok"],
        true
    );
    let start = Instant::now();
    let response = rpc(
        &socket,
        serde_json::json!({"op":"search","mode":"commands","query":"finder"}),
    )
    .unwrap();
    assert_eq!(response["ok"], true, "{response}");
    assert!(response["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["id"] == "app.finder"));
    assert!(start.elapsed() < Duration::from_millis(1400));
    assert_eq!(
        rpc(&socket, serde_json::json!({"op":"system.fixture"})).unwrap()["result"]["effects"],
        serde_json::json!([])
    );
}
#[test]
fn direct_sdk_transport_is_fully_owned() {
    let root = root();
    let out = root.join("target/system-search-sdk");
    fs::create_dir_all(&out).unwrap();
    let fixture = out.join("fixture");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(
        fixture.join(".ruycast-test-fixture"),
        b"own-disposable-fixture-v1",
    )
    .unwrap();
    let shim = out.join("owned-sdk.dylib");
    let executable = Path::new(env!("CARGO_BIN_EXE_system-search-probe"));
    let artifact = std::env::var_os("RUYCAST_TEST_SYSTEM_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            executable
                .parent()
                .unwrap()
                .join("examples/libsystem_fixture.dylib")
        });
    assert!(
        artifact.is_absolute() && artifact.is_file(),
        "Build the same-profile system-fixture Rust example first"
    );
    fs::copy(&artifact, &shim).unwrap();
    assert_eq!(
        Sha256::digest(fs::read(&artifact).unwrap()),
        Sha256::digest(fs::read(&shim).unwrap())
    );
    let sources = [
        "rust-native/tests/fixtures/system_nodes.rs",
        "rust-native/tests/fixtures/system_effects.rs",
        "rust-native/tests/fixtures/system_apps.rs",
        "rust-native/tests/fixtures/system_events.rs",
        "rust-native/tests/fixtures/system_fixture.rs",
        "rust-native/Cargo.toml",
        "rust-native/Cargo.lock",
        "rust/Cargo.toml",
        "rust-toolchain.toml",
    ]
    .into_iter()
    .map(|name| {
        (
            name,
            format!(
                "{:x}",
                Sha256::digest(fs::read(root.parent().unwrap().join(name)).unwrap())
            ),
        )
    })
    .collect::<BTreeMap<_, _>>();
    fs::write(out.join("rust-sdk-bindings.json"), serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":{shim.strip_prefix(root.parent().unwrap()).unwrap().to_str().unwrap():format!("{:x}",Sha256::digest(fs::read(&shim).unwrap()))}})).unwrap()).unwrap();
    let probe = Command::new(env!("CARGO_BIN_EXE_system-search-probe"))
        .env("DYLD_INSERT_LIBRARIES", &shim)
        .env("RUYCAST_SYSTEM_SDK_FIXTURE_HOME", &fixture)
        .output()
        .unwrap();
    assert!(
        probe.status.success(),
        "{}{}",
        String::from_utf8_lossy(&probe.stdout),
        String::from_utf8_lossy(&probe.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["noExternalQueries"], true);
    assert_eq!(result["realDesktopEffects"], false);
    fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}
#[test]
fn direct_sdk_transport_refuses_without_owned_provider() {
    let denied = Command::new(env!("CARGO_BIN_EXE_system-search-probe"))
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("RUYCAST_SYSTEM_SDK_FIXTURE_HOME")
        .output()
        .unwrap();
    assert!(!denied.status.success());
    let denied: serde_json::Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert!(denied["error"]
        .as_str()
        .unwrap()
        .contains("external discovery disabled"));
}
#[test]
fn optional_handoff_manifest_and_actual_native_text_are_content_bound() {
    let native = root();
    let workspace = native.parent().unwrap();
    let Some(baseline) = std::env::var_os("RUYCAST_SYSTEM_SEARCH_BASELINE").map(PathBuf::from)
    else {
        return;
    };
    let baseline_bytes = fs::read(&baseline).unwrap();
    let before: BTreeMap<String, String> = serde_json::from_slice(&baseline_bytes).unwrap();
    let mut files = BTreeSet::new();
    fn scan(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let e = entry.unwrap();
            let p = e.path();
            let name = e.file_name();
            if ["target", ".build", ".git", ".go", "__pycache__"]
                .iter()
                .any(|n| name == *n)
            {
                continue;
            }
            let ty = e.file_type().unwrap();
            if ty.is_dir() {
                scan(root, &p, out);
            } else if ty.is_file() {
                out.insert(p.strip_prefix(root).unwrap().to_str().unwrap().into());
            }
        }
    }
    scan(workspace, workspace, &mut files);
    files.extend(before.keys().cloned());
    let mut manifest = vec![];
    for file in files {
        if file == "WORKSPACE.md" {
            continue;
        }
        let current = fs::read(workspace.join(&file))
            .ok()
            .map(|b| format!("{:x}", Sha256::digest(b)));
        let old = before.get(&file);
        if current.as_ref() != old {
            manifest.push(serde_json::json!({"path":file,"before":old,"after":current}));
        }
    }
    let output = native.join("target/system-search-evidence");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("changed-files.json"),serde_json::to_vec_pretty(&serde_json::json!({"baseline":baseline,"baselineSHA256":format!("{:x}",Sha256::digest(&baseline_bytes)),"changed":manifest})).unwrap()).unwrap();
    let mut text = BTreeSet::new();
    let mut renders = vec![];
    for entry in fs::read_dir(&output).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) != Some("json")
            || p.file_name().unwrap() == "changed-files.json"
        {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&fs::read(&p).unwrap()) else {
            continue;
        };
        for field in ["screenText", "accessibilityText"] {
            if let Some(values) = v[field].as_array() {
                for value in values {
                    if let Some(s) = value.as_str().filter(|s| !s.is_empty()) {
                        text.insert(s.to_owned());
                    }
                }
            }
        }
        if v.get("screenText").is_some() {
            assert_eq!(v["windowShown"], false);
            renders.push(p.file_name().unwrap().to_string_lossy().to_string());
        }
    }
    assert!(
        renders.len() >= 16,
        "Run the existing owned native process suite before handoff"
    );
    fs::write(
        output.join("ui-copy.txt"),
        text.into_iter().collect::<Vec<_>>().join("\n") + "\n",
    )
    .unwrap();
    fs::write(
        output.join("render-manifest.json"),
        serde_json::to_vec_pretty(&renders).unwrap(),
    )
    .unwrap();
}


fn rpc(socket:&Path,value:&Value)->Result<Value,String>{
    let mut stream=UnixStream::connect(socket).map_err(|e|e.to_string())?;stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    serde_json::to_writer(&mut stream,value).map_err(|e|e.to_string())?;stream.write_all(b"\n").map_err(|e|e.to_string())?;
    let mut line=String::new();BufReader::new(stream).read_line(&mut line).map_err(|e|e.to_string())?;serde_json::from_str(&line).map_err(|e|e.to_string())
}
struct Host{child:Child,base:PathBuf,profile:PathBuf,socket:PathBuf,stopped:bool}
impl Host{
    fn call(&self,value:Value)->Value{let reply=rpc(&self.socket,&value).unwrap();assert_eq!(reply["ok"],true,"{value}: {reply}");reply["result"].clone()}
    fn wait(&self,predicate:impl Fn(&Value)->bool)->Value{
        let end=Instant::now()+Duration::from_secs(8);
        loop{let status=self.call(json!({"op":"status"}));if status["configurationUpdating"]!=true && status["nativeUI"]["revision"].as_u64().unwrap_or(0)>0 && status["nativeUI"]["revision"]==status["nativeUI"]["appliedRevision"] && predicate(&status){return status}assert!(Instant::now()<end,"owned browser state did not settle: {status}");std::thread::sleep(Duration::from_millis(5));}
    }
    fn probe(&self,action:&str)->Value{self.call(json!({"op":"native.test","action":action}))}
    fn hide(&self){if self.call(json!({"op":"status"}))["nativeUI"]["visible"]==true{self.call(json!({"op":"native.test","action":"key-event","code":53,"chars":"\u{1b}","flags":0}));self.wait(|s|s["nativeUI"]["visible"]==false);}}
    fn hidden(&self)->Value{let view=self.probe("read");for key in ["windowShown","windowKey","windowMain"]{assert_eq!(view[key],false,"owned browser panel escaped hidden boundary: {view}");}view}
    fn stop(mut self,output:&Path){
        self.call(json!({"op":"quit"}));let end=Instant::now()+Duration::from_secs(4);
        let status=loop{if let Some(status)=self.child.try_wait().unwrap(){break status}assert!(Instant::now()<end,"owned native browser host failed to quit");std::thread::sleep(Duration::from_millis(5));};assert!(status.success());self.stopped=true;
        let mut stdout=String::new();self.child.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();let reply:Value=serde_json::from_str(&stdout).unwrap();assert_eq!(reply["result"]["state"],"stopped");fs::write(output,serde_json::to_vec_pretty(&reply).unwrap()).unwrap();
    }
}
impl Drop for Host{fn drop(&mut self){if !self.stopped{let _=rpc(&self.socket,&json!({"op":"quit"}));let end=Instant::now()+Duration::from_secs(3);while self.child.try_wait().ok().flatten().is_none(){if Instant::now()>=end{let _=self.child.kill();break}std::thread::sleep(Duration::from_millis(5));}let _=self.child.wait();}let _=fs::remove_dir_all(&self.base);}}

include!("support/owned_keyboard.rs");
fn qualified_system_ui_image()->(PathBuf,PathBuf,OwnedKeyboardFixture){
    let binary=PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native"));assert_eq!(fixture_sha(&binary),std::env::var("RUYCAST_BROWSER_NATIVE_SHA").expect("independently qualified current native image hash required"));
    let fixture=build_owned_keyboard_fixture();(binary,fixture.artifact.clone(),fixture)
}
#[test]
fn owned_keyboard_fixture_prepares_pinned_sources_without_loading_sdk() {let fixture=build_owned_keyboard_fixture();assert!(fixture.artifact.is_file());}
#[test]
fn owned_keyboard_fixture_rechecks_changed_bytes_symlinks_and_sources_without_loading_sdk() {
    use std::os::unix::fs::{DirBuilderExt,MetadataExt,symlink};
    let parent=PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").unwrap());assert!(parent.is_absolute());let meta=fs::symlink_metadata(&parent).unwrap();assert!(meta.is_dir()&&!meta.file_type().is_symlink());assert_eq!(fs::read(parent.join(".ruycast-ui-fixture-parent")).unwrap(),b"owned-ui-fixture-parent-v1");
    let directory=parent.join(format!("keyboard-negative-{}",std::process::id()));fs::DirBuilder::new().mode(0o700).create(&directory).unwrap();
    let mut fixture=OwnedKeyboardFixture{artifact:directory.join("own-bytes"),directory,expected_sha:format!("{:x}",Sha256::digest(b"ownedfixture")),sourcepins:keyboard_fixture_sources(),owner:meta.uid()};
    fs::write(&fixture.artifact,b"ownedfixture").unwrap();fixture.verify();
    fs::write(&fixture.artifact,b"changed").unwrap();assert!(std::panic::catch_unwind(||fixture.verify()).is_err());
    fs::write(&fixture.artifact,b"ownedfixture").unwrap();let target=fixture.directory.join("target");fs::rename(&fixture.artifact,&target).unwrap();symlink(&target,&fixture.artifact).unwrap();assert!(std::panic::catch_unwind(||fixture.verify()).is_err());
    fs::remove_file(&fixture.artifact).unwrap();fs::rename(target,&fixture.artifact).unwrap();fixture.sourcepins.clear();assert!(std::panic::catch_unwind(||fixture.verify()).is_err());
}
#[test]
fn owned_keyboard_fixture_loader_rejects_unknown_effects_without_loading_sdk() {
    assert!(std::panic::catch_unwind(||assert_keyboard_source_hash("caps_sdk.rs","modified","qualified")).is_err());
    let artifact=Path::new("/owned/keyboard.dylib");
    let policy=keyboard_loader_policy();
    let commands=format!("cmd LC_ID_DYLIB\n name {} (offset 24)\nSection\n sectname __mod_init_func\n segname __DATA_CONST\n size 0x0000000000000008\n flags 0x00000009\nSection\n sectname __mod_term_func\n segname __DATA_CONST\n size 0x0000000000000008\n flags 0x0000000a\n",artifact.display());
    let mut dependencies=format!("{}:\n {} (x)\n",artifact.display(),artifact.display());
    for path in policy["dependencies"].as_array().unwrap(){dependencies.push_str(&format!(" {} (x)\n",path.as_str().unwrap()));}
    check_keyboard_loader(&commands,&dependencies,artifact);
    for (kind,symbol) in [("initializers","_ruycast_keyboard_host_fixture_install"),("finalizers","_ruycast_keyboard_fixture_cleanup")] {
        let offsets="image:\nContents\n0000000000000010 00000020 00000000\n";
        let symbols=format!("0000000000000020 (__TEXT,__text) external {symbol}");
        check_keyboard_pointers(offsets,&symbols,kind);
        assert!(std::panic::catch_unwind(||check_keyboard_pointers(offsets,"0000000000000020 (__TEXT,__text) external _unknown",kind)).is_err());
        assert!(std::panic::catch_unwind(||check_keyboard_pointers("image:\nContents\n0000000000000010 00000020 00000000 00000030 00000000\n",&symbols,kind)).is_err());
    }
    for extra in ["LC_RPATH","LC_DYLD_ENVIRONMENT","LC_ROUTINES","__interpose","__init_offsets"] {
        assert!(std::panic::catch_unwind(||check_keyboard_loader(&format!("{commands}{extra}"),&dependencies,artifact)).is_err());
    }
    for (name,flags) in [("__unknown_init","0x00000009"),("__unknown_term","0x0000000a"),("__unknown_tls","0x00000015"),("__unknown_offsets","0x00000016")] {
        assert!(std::panic::catch_unwind(||check_keyboard_loader(&format!("{commands}Section\n sectname {name}\n segname __DATA\n size 0x0000000000000008\n flags {flags}\n"),&dependencies,artifact)).is_err());
    }
    assert!(std::panic::catch_unwind(||check_keyboard_loader(&commands,&format!("{dependencies} /unknown/library (x)\n"),artifact)).is_err());
    assert!(std::panic::catch_unwind(||check_keyboard_loader(&commands.replace("name /owned/keyboard.dylib","name /unknown/identity"),&dependencies,artifact)).is_err());
}
#[test]
fn browser_original_native_return_and_forty_eight_hidden_variants_are_owned() {
    use serde_json::{json,Value};
    use std::{os::unix::fs::DirBuilderExt,process::Stdio,time::{Duration,Instant}};
    let (binary,sdk,fixture_owner)=qualified_system_ui_image();
    let parent=PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("short owned fixture parent required"));assert!(parent.is_absolute());
    let output=PathBuf::from(std::env::var_os("RUYCAST_BROWSER_EVIDENCE").expect("new owned browser evidence directory required"));assert!(output.is_absolute());fs::create_dir(&output).unwrap();let mut captures=vec![];
    for theme in ["light","dark"]{for width in [600,760]{
        let base=parent.join(format!("bt-{}-{theme}-{width}",std::process::id()));fs::DirBuilder::new().mode(0o700).create(&base).unwrap();let profile=base.join("p");fs::create_dir(&profile).unwrap();let apps=base.join("Applications");fs::create_dir(&apps).unwrap();
        fs::write(base.join(".ruycast-test-fixture"),b"own-disposable-fixture-v1").unwrap();fs::write(base.join(".ruycast-keyboard-fixture"),b"own-keyboard-fixture-v1").unwrap();
        let mut config:Value=serde_json::from_slice(include_bytes!("../../config/default.json")).unwrap();config["theme"]=json!(theme);config["clipboard"]["enabled"]=json!(false);config["commands"]=json!([{"id":"fixture.notes","title":"Project Notes","kind":"builtin","target":"settings"},{"id":"fixture.local","title":"Local Command","kind":"builtin","target":"reload-config"}]);fs::write(profile.join("config.json"),serde_json::to_vec(&config).unwrap()).unwrap();
        fixture_owner.verify();
        let child=Command::new(&binary).arg("--native-hidden").env_clear().env("HOME",&base).env("RUYCAST_HOME",&profile).env("RUYCAST_APP_BINARY",&binary).env("RUYCAST_NATIVE_TEST_APP_ROOTS",serde_json::to_string(&vec![apps]).unwrap()).env("RUYCAST_NATIVE_TEST_SYSTEM_SEARCH","owned").env("RUYCAST_NATIVE_TEST_MAIN_TABS","1").env("RUYCAST_NATIVE_TEST_WIDTH",width.to_string()).env("DYLD_INSERT_LIBRARIES",&sdk).env("RUYCAST_NATIVE_TEST_KEYBOARD","1").env("RUYCAST_KEYWORD_FIXTURE_HOME",&base).env("RUYCAST_KEYBOARD_FIXTURE_HOME",&base).env("RUYCAST_WORKSPACE_FIXTURE_HOME",&base).env("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD","trace").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let host=Host{child,base,profile:profile.clone(),socket:profile.join("control.sock"),stopped:false};let end=Instant::now()+Duration::from_secs(8);loop{if rpc(&host.socket,&json!({"op":"status"})).is_ok(){break}assert!(Instant::now()<end,"owned native browser host did not start");std::thread::sleep(Duration::from_millis(10));}
        host.wait(|s|s["keyboardGeneration"].as_u64().unwrap_or(0)>0);host.call(json!({"op":"clipboard.pause","paused":true}));assert_eq!(host.call(json!({"op":"system.fixture"}))["reads"],0,"startup unexpectedly polled Safari");
        host.call(json!({"op":"show","mode":"commands","query":"tabbladen"}));host.wait(|s|s["nativeUI"]["visible"]==true&&s["nativeUI"]["systemSearchBusy"]==false);let entry=host.hidden();assert_eq!(entry["rows"],1);assert_eq!(entry["selectedId"],"builtin.safari-tabs","Safari entrypoint missing or duplicated");
        for mode in ["tabs","commands"]{
            host.hide();host.call(json!({"op":"system.fixture","action":"browser-variant","variant":"normal"}));host.call(json!({"op":"system.fixture","action":"clear-effects"}));host.call(json!({"op":"show","mode":mode,"query":""}));host.wait(|s|s["nativeUI"]["visible"]==true&&s["nativeUI"]["systemSearchBusy"]==false);
            let view=host.hidden();let accessible=if mode=="tabs"{"Search Safari tabs…"}else{"Search apps, tabs or commands…"};assert!(view["accessibilityText"].as_array().unwrap().iter().any(|s|s==accessible),"{view}");assert_eq!(view["responderSearch"],true);
            let matches=host.call(json!({"op":"search","mode":mode,"query":"other","limit":1}));assert_eq!(matches.as_array().unwrap().len(),1);assert_eq!(matches[0]["tab"]["url"],"https://example.test/other");
            assert_eq!(view["selectedRow"],0);let prior=view["selectedId"].clone();
            host.call(json!({"op":"native.test","action":"key-event","code":125,"chars":"\u{f701}","flags":0}));let selected=host.wait(|s|s["nativeUI"]["selectedId"]!=prior)["nativeUI"]["selectedId"].as_str().unwrap().to_owned();assert_eq!(host.hidden()["selectedRow"],1);assert!(selected.starts_with(if mode=="tabs"{"system.tab:"}else{"system.tab:root."}));if mode=="tabs"{assert!(!selected.starts_with("system.tab:root."));}
            host.key(40,1<<20,"k");
            let provider_menu=host.hidden();
            for expected in ["Refresh","Allow Safari Access"] {assert!(provider_menu["actionLabels"].as_array().unwrap().iter().any(|label|label.as_str().is_some_and(|label|label.starts_with(expected))),"Selected tab lost provider action {expected}: {provider_menu}");}
            host.key(53,0,"");
            host.call(json!({"op":"native.test","action":"key-event","code":36,"chars":"\r","flags":0}));host.wait(|s|s["nativeUI"]["visible"]==false&&s["nativeUI"]["systemSearchBusy"]==false);assert_eq!(host.call(json!({"op":"system.fixture"}))["effects"],json!(["select-tab","front-window","activate"]));host.hidden();assert_eq!(host.call(json!({"op":"usage"})),json!({}));
            let variants: [&str;6]=if mode=="tabs"{["normal","duplicate","long","empty","error","loading"]}else{["normal","typed","long","no-tabs","loading","empty"]};
            for variant in variants{
                host.hide();let fixture=match variant{"typed"=>"normal","no-tabs"=>"empty","error"=>"denied",other=>other};host.call(json!({"op":"system.fixture","action":"browser-variant","variant":fixture}));let query=if variant=="typed"{"Project"}else if ["empty","error","loading"].contains(&variant){"unmatched-address"}else{""};host.call(json!({"op":"show","mode":mode,"query":query}));
                let status=host.wait(|s|s["nativeUI"]["visible"]==true && s["nativeUI"]["systemSearchBusy"]==(variant=="loading"));let view=host.hidden();assert!(view["accessibilityText"].as_array().unwrap().iter().any(|s|s==accessible));
                let text=view["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
                match variant {
                    "normal"|"duplicate"|"typed"=>{assert!(text.contains("https://example.test/notes")&&text.contains("https://example.test/other"),"two captured tab rows missing: {view}");}
                    "long"=>assert!(text.contains("A Very Long Safari Tab Title")&&text.contains("https://example.test/projects/a-long-address"),"long tab metadata missing: {view}"),
                    "no-tabs"=>{assert!(!text.contains("example.test"));assert!(text.contains("Local Command"));}
                    "empty"=>{assert_eq!(view["rows"],0);assert!(text.contains(if mode=="commands"{"No commands found"}else{"No results"}));}
                    "error"=>{assert_eq!(view["rows"],0);assert!(text.contains("Automation"),"permission error missing: {view}");}
                    "loading"=>{assert_eq!(view["rows"],0);assert!(text.contains("Loading…"),"loading state missing: {view}");}
                    _=>unreachable!(),
                }
                let name=format!("{mode}-{theme}-{width}-{variant}");let capture=host.probe("screenshot");for key in ["windowShown","windowKey","windowMain"]{assert_eq!(capture[key],false,"capture escaped hidden boundary: {capture}");}let after=host.call(json!({"op":"status"}));assert_eq!(after["nativeUI"]["systemSearchBusy"],variant=="loading","loading state elapsed before actual render: {name}");
                let png=fs::read(host.profile.join("native-probe.png")).unwrap();assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));let pixels=u32::from_be_bytes(png[16..20].try_into().unwrap());assert!(pixels==width||pixels==width*2,"wrong actual width: {name}: {pixels}");fs::write(output.join(format!("{name}.png")),&png).unwrap();let data=json!({"view":view,"capture":capture,"status":status,"after":after,"mode":mode,"theme":theme,"width":width,"variant":variant,"pngSHA256":format!("{:x}",Sha256::digest(&png))});fs::write(output.join(format!("{name}.json")),serde_json::to_vec_pretty(&data).unwrap()).unwrap();captures.push(data);
            }
        }
        let logs=rpc(&host.socket,&json!({"op":"logs","limit":100})).unwrap();let encoded=serde_json::to_string(&logs).unwrap();for private in ["Project Notes","example.test","A Very Long Safari"]{assert!(!encoded.contains(private),"browser metadata leaked into logs: {encoded}");}let path=PathBuf::from(logs["path"].as_str().unwrap());assert!(path.starts_with(&host.profile));let bytes=fs::read(path).unwrap();let encoded=String::from_utf8(bytes).unwrap();assert!(!encoded.contains("Project Notes")&&!encoded.contains("example.test"));assert_eq!(host.call(json!({"op":"usage"})),json!({}));host.stop(&output.join(format!("quit-{theme}-{width}.json")));
    }}
    fixture_owner.verify();
    assert_eq!(captures.len(),48);fs::write(output.join("matrix.json"),serde_json::to_vec_pretty(&json!({"captures":captures,"realSafari":false,"physicalInput":false,"visibleWindow":false,"modes":2,"themes":2,"widths":2,"variantsPerMode":6})).unwrap()).unwrap();
}

impl Host {
    fn cli(&self, args: &[&str]) -> Value {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native"));
        let result = Command::new(root().parent().unwrap().join(".build/rust/release/ruycast"))
            .args(args).env_clear().env("HOME", &self.base).env("RUYCAST_HOME", &self.profile)
            .env("RUYCAST_APP_BINARY", binary).output().unwrap();
        assert!(result.status.success(), "owned CLI {args:?}: {}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        serde_json::from_slice(&result.stdout).unwrap()
    }
    fn key(&self, code: u16, flags: u64, chars: &str) {
        self.call(json!({"op":"native.test","action":"key-event","code":code,"flags":flags,"chars":chars}));
    }
    fn select_row(&self, row: usize) {
        while self.hidden()["selectedRow"].as_u64().unwrap() < row as u64 {
            let before=self.hidden()["selectedId"].clone();self.key(125,0,"\u{f701}");
            self.wait(|s|s["nativeUI"]["selectedId"]!=before);
        }
        assert_eq!(self.hidden()["selectedRow"],row);
    }
    fn action(&self, title: &str) {
        if self.hidden()["actionsVisible"]!=true {self.key(40,1<<20,"k");}
        self.wait(|_|self.hidden()["actionsVisible"]==true);
        let view=self.hidden();let index=view["actionLabels"].as_array().unwrap().iter()
            .position(|v|v.as_str().is_some_and(|v|v.starts_with(title))).expect("actual native action absent");
        self.call(json!({"op":"native.test","action":"action-button","index":index}));
    }
    fn capture(&self, output: &Path, mode: &str, theme: &str, width: u32, variant: &str) -> Value {
        let status=self.call(json!({"op":"status"}));let view=self.hidden();
        let capture=self.probe("screenshot");
        for key in ["windowShown","windowKey","windowMain"] {assert_eq!(capture[key],false,"{capture}");}
        let after=self.call(json!({"op":"status"}));
        assert_eq!(status["nativeUI"]["systemSearchBusy"],variant=="loading");
        assert_eq!(after["nativeUI"]["systemSearchBusy"],variant=="loading","actual capture completed after loading: {variant}");
        let png=fs::read(self.profile.join("native-probe.png")).unwrap();assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let pixels=u32::from_be_bytes(png[16..20].try_into().unwrap());assert!(pixels==width||pixels==width*2);
        let height=u32::from_be_bytes(png[20..24].try_into().unwrap());assert!(height==490||height==980,"unexpected native panel height {height}");
        let name=format!("{mode}-{theme}-{width}-{variant}");fs::write(output.join(format!("{name}.png")),&png).unwrap();
        let value=json!({"view":view,"capture":capture,"status":status,"after":after,"mode":mode,"theme":theme,"width":width,"variant":variant,"pngSHA256":format!("{:x}",Sha256::digest(&png))});
        fs::write(output.join(format!("{name}.json")),serde_json::to_vec_pretty(&value).unwrap()).unwrap();value
    }
}

fn owned_system_ui_host(mode: &str, theme: &str, width: u32, binary: &Path, fixture: &OwnedKeyboardFixture, output: &Path) -> (Host,Vec<u8>,Vec<u8>) {
    use std::os::unix::fs::DirBuilderExt;
    let sdk=&fixture.artifact;
    assert!(["windows","shortcuts","menus"].contains(&mode));
    let parent=PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("short owned fixture parent required"));assert!(parent.is_absolute());
    assert!(fs::symlink_metadata(&parent).is_ok_and(|m|m.is_dir()),"own parent must be a real directory");
    let marker=parent.join(".ruycast-ui-fixture-parent");assert!(fs::symlink_metadata(&marker).is_ok_and(|m|m.is_file()));assert_eq!(fs::read(marker).unwrap(),b"owned-ui-fixture-parent-v1");
    let base=parent.join(format!("su-{}-{mode}-{theme}-{width}",std::process::id()));fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
    let profile=base.join("p");fs::create_dir(&profile).unwrap();let apps=base.join("Applications");fs::create_dir(&apps).unwrap();
    fs::write(base.join(".ruycast-test-fixture"),b"own-disposable-fixture-v1").unwrap();fs::write(base.join(".ruycast-keyboard-fixture"),b"own-keyboard-fixture-v1").unwrap();
    let mut config:Value=serde_json::from_slice(include_bytes!("../../config/default.json")).unwrap();
    config["theme"]=json!(theme);config["clipboard"]["enabled"]=json!(false);
    config["commands"]=json!([{"id":"fixture.persisted","title":"Persisted Fixture","kind":"builtin","target":"reload-config"}]);
    fs::write(profile.join("config.json"),serde_json::to_vec(&config).unwrap()).unwrap();
    let usage=br#"{"fixture.persisted":7}"#.to_vec();
    let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    let ranking=serde_json::to_vec(&json!({"schema":1,"entries":{"fixture.persisted":now}})).unwrap();
    fs::write(profile.join("usage.json"),&usage).unwrap();fs::write(profile.join("search-ranking.json"),&ranking).unwrap();
    let stderr=fs::File::create(output.join(format!("host-{mode}-{theme}-{width}.stderr"))).unwrap();
    fixture.verify();
    let child=Command::new(binary).arg("--native-hidden").env_clear().env("HOME",&base).env("RUYCAST_HOME",&profile)
        .env("RUYCAST_APP_BINARY",binary).env("RUYCAST_NATIVE_TEST_APP_ROOTS",serde_json::to_string(&vec![apps]).unwrap())
        .env("RUYCAST_NATIVE_TEST_SYSTEM_SEARCH","owned").env("RUYCAST_NATIVE_TEST_MAIN_TABS","1").env("RUYCAST_NATIVE_TEST_WIDTH",width.to_string())
        .env("DYLD_INSERT_LIBRARIES",sdk).env("RUYCAST_NATIVE_TEST_KEYBOARD","1").env("RUYCAST_KEYWORD_FIXTURE_HOME",&base)
        .env("RUYCAST_KEYBOARD_FIXTURE_HOME",&base).env("RUYCAST_WORKSPACE_FIXTURE_HOME",&base).env("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD","trace")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::from(stderr)).spawn().unwrap();
    let host=Host{child,base,profile:profile.clone(),socket:profile.join("control.sock"),stopped:false};
    let end=Instant::now()+Duration::from_secs(8);while rpc(&host.socket,&json!({"op":"status"})).is_err(){assert!(Instant::now()<end,"owned system UI host did not start");std::thread::sleep(Duration::from_millis(10));}
    host.wait(|s|s["keyboardGeneration"].as_u64().unwrap_or(0)>0);host.call(json!({"op":"clipboard.pause","paused":true}));
    assert_eq!(host.call(json!({"op":"usage"})),json!({"fixture.persisted":7}));
    assert_eq!(fs::read(profile.join("usage.json")).unwrap(),usage);assert_eq!(fs::read(profile.join("search-ranking.json")).unwrap(),ranking);
    (host,usage,ranking)
}

#[test]
fn window_original_native_return_and_twenty_four_hidden_states_preserve_durable_choices() {
    owned_system_ui_matrix("windows");
}
#[test]
fn shortcuts_original_native_return_and_twenty_four_hidden_states_keep_uuid_identity() {
    owned_system_ui_matrix("shortcuts");
}
fn owned_system_ui_matrix(mode: &str) {
    let (binary,_sdk,fixture_owner)=qualified_system_ui_image();
    let parent=PathBuf::from(std::env::var_os("RUYCAST_SYSTEM_UI_EVIDENCE").expect("new owned system UI evidence parent required"));assert!(parent.is_absolute());
    let output=parent.join(mode);fs::create_dir(&output).unwrap();let mut captures=vec![];
    for theme in ["light","dark"] {for width in [600,760] {
        let (host,usage,ranking)=owned_system_ui_host(mode,theme,width,&binary,&fixture_owner,&output);
        let variant_action=if mode=="windows"{"window-variant"}else{"shortcut-variant"};
        let set=|variant:&str|{host.call(json!({"op":"system.fixture","action":variant_action,"variant":variant}));};
        set("normal");
        let analytics_before=host.call(json!({"op":"analytics"}));
        let prior_analytics=analytics_before["commands"].as_object().unwrap().keys().cloned().collect::<BTreeSet<_>>();
        let entry=host.cli(&["search",if mode=="windows"{"windows"}else{"opdrachten"}]);
        assert_eq!(entry["result"].as_array().unwrap().iter().filter(|c|c["target"]==if mode=="windows"{"switch-windows"}else{"apple-shortcuts"}).count(),1,"unique discoverable entry: {entry}");
        let result=host.cli(&["search","","--mode",mode]);let rows=result["result"].as_array().unwrap();
        assert_eq!(rows.len(),if mode=="windows"{5}else{3},"{result}");
        let ids=rows.iter().map(|c|c["id"].as_str().unwrap()).collect::<BTreeSet<_>>();assert_eq!(ids.len(),rows.len());
        let (cli_id,launcher_row)=if mode=="windows" {
            assert_eq!(rows[0]["window"]["application"],"Editor Fixture");
            assert_eq!(rows[0]["window"]["title"],"Project Notes");
            assert_eq!(rows[2]["window"]["minimized"],true);
            assert!(rows.iter().all(|c|c.get("hotkey").is_none()));
            (rows[0]["id"].as_str().unwrap().to_owned(),2)
        } else {
            assert!(ids.contains("apple-shortcut.aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"));
            assert!(ids.contains("apple-shortcut.11111111-2222-3333-4444-555555555555"));
            for row in rows {assert_eq!(row["arguments"][0],"run");assert_eq!(row["arguments"][1],row["id"].as_str().unwrap().trim_start_matches("apple-shortcut."));}
            ("apple-shortcut.aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),rows.iter().position(|c|c["id"]=="apple-shortcut.11111111-2222-3333-4444-555555555555").unwrap())
        };
        host.call(json!({"op":"show","mode":mode,"query":""}));host.wait(|s|s["nativeUI"]["visible"]==true&&s["nativeUI"]["systemSearchBusy"]==false);
        assert_eq!(host.hidden()["responderSearch"],true);
        host.call(json!({"op":"system.fixture","action":"clear-effects"}));
        assert_eq!(host.cli(&["run",&cli_id])["ok"],true);
        assert_eq!(host.call(json!({"op":"status"}))["nativeUI"]["visible"],true,"CLI must not hide an already-open palette");
        let expected_cli=if mode=="windows"{json!(["activate","AXRaise"])}else{json!(["shortcut:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"])};
        assert_eq!(host.call(json!({"op":"system.fixture"}))["effects"],expected_cli);
        assert_eq!(fs::read(host.profile.join("search-ranking.json")).unwrap(),ranking,"CLI must not change launcher ranking");
        if mode=="windows" {
            host.call(json!({"op":"system.fixture","action":"clear-effects"}));
            assert_eq!(host.cli(&["run",rows[2]["id"].as_str().unwrap()])["ok"],true);
            assert_eq!(host.call(json!({"op":"system.fixture"}))["effects"],json!(["activate","restore","AXRaise"]));
            assert_eq!(host.call(json!({"op":"usage"})),json!({"fixture.persisted":7}));
            assert_eq!(fs::read(host.profile.join("usage.json")).unwrap(),usage);
        }
        host.hide();set("normal");host.call(json!({"op":"show","mode":mode,"query":""}));host.wait(|s|s["nativeUI"]["visible"]==true&&s["nativeUI"]["systemSearchBusy"]==false);
        let view=host.hidden();let label=if mode=="windows"{"Search windows…"}else{"Search Apple Shortcuts…"};
        assert!(view["accessibilityText"].as_array().unwrap().iter().any(|v|v==label));assert_eq!(view["responderSearch"],true);
        host.select_row(launcher_row);host.call(json!({"op":"system.fixture","action":"clear-effects"}));
        host.key(36,0,"\r");host.wait(|s|s["nativeUI"]["visible"]==false&&s["nativeUI"]["systemSearchBusy"]==false);
        assert_eq!(host.call(json!({"op":"system.fixture"}))["effects"],if mode=="windows"{json!(["activate","restore","AXRaise"])}else{json!(["shortcut:11111111-2222-3333-4444-555555555555"])});
        host.hidden();
        if mode=="windows" {
            assert_eq!(host.call(json!({"op":"usage"})),json!({"fixture.persisted":7}));
            assert_eq!(fs::read(host.profile.join("usage.json")).unwrap(),usage);assert_eq!(fs::read(host.profile.join("search-ranking.json")).unwrap(),ranking);
            assert_eq!(host.call(json!({"op":"search-ranking"}))["entries"],1);
            let analytics=host.call(json!({"op":"analytics"}));let commands=analytics["commands"].as_object().unwrap();
            let key=format!("{:x}",Sha256::digest(b"ruycast.command:system.window"));
            let mut expected=prior_analytics.clone();expected.insert(key.clone());
            assert_eq!(commands.keys().cloned().collect::<BTreeSet<_>>(),expected);assert!(commands.contains_key(&key));
            let text=serde_json::to_string(&analytics).unwrap();for id in ids {assert!(!text.contains(id));}
        } else {
            let counts=host.call(json!({"op":"usage"}));assert_eq!(counts["fixture.persisted"],7);assert_eq!(counts["apple-shortcut.11111111-2222-3333-4444-555555555555"],1);
            let bytes:Value=serde_json::from_slice(&fs::read(host.profile.join("search-ranking.json")).unwrap()).unwrap();assert!(bytes["entries"].get("apple-shortcut.11111111-2222-3333-4444-555555555555").is_some());assert!(bytes["entries"].get(&cli_id).is_none());
        }
        let variants=if mode=="windows"{["normal","minimized","long","empty","denied","loading"]}else{["normal","duplicate","long","empty","error","loading"]};
        for variant in variants {
            host.hide();set(variant);host.call(json!({"op":"show","mode":mode,"query":""}));
            host.wait(|s|s["nativeUI"]["visible"]==true&&s["nativeUI"]["systemSearchBusy"]==(variant=="loading"));
            if variant=="minimized" {host.select_row(2);}
            let view=host.hidden();let text=view["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
            match variant {
                "normal"|"minimized"=>assert_eq!(view["rows"],if mode=="windows"{5}else{3}),
                "duplicate"=>{assert_eq!(view["rows"],2);assert!(text.matches("Focus").count()>=2,"{view}");},
                "long"=>assert!(text.contains(if mode=="windows"{"Project Notes with a Very Long"}else{"A Very Long Shortcut Name"}),"long closed fixture absent: {view}"),
                "empty"=>{assert_eq!(view["rows"],0);assert!(text.contains("No results"),"{view}");},
                "denied"=>{assert_eq!(view["rows"],0);assert!(text.contains("Accessibility")&&text.contains("System Settings"),"{view}");},
                "error"=>{assert_eq!(view["rows"],0);assert!(text.contains("Could not load shortcuts. Check Shortcuts, then refresh."),"{view}");},
                "loading"=>{assert_eq!(view["rows"],0);assert!(text.contains("Loading…"),"{view}");},
                _=>unreachable!(),
            }
            captures.push(host.capture(&output,mode,theme,width,variant));
            if ["denied","error"].contains(&variant) {
                set("normal");host.action("Refresh");host.wait(|s|{
                    let recovered=host.hidden();
                    let text=recovered["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
                    s["nativeUI"]["systemSearchBusy"]==false&&s["nativeUI"]["visible"]==true
                        &&recovered["rows"]==if mode=="windows"{5}else{3}
                        &&!["Accessibility","System Settings","Could not load shortcuts"].iter().any(|error|text.contains(error))
                });
                let recovered=host.hidden();assert_eq!(recovered["rows"],if mode=="windows"{5}else{3});
                let text=recovered["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
                for old_error in ["Accessibility","System Settings","Could not load shortcuts"] {assert!(!text.contains(old_error),"recovery kept old error: {recovered}");}
            }
            if variant=="loading" {
                host.key(53,0,"\u{1b}");host.wait(|s|s["nativeUI"]["visible"]==false&&s["nativeUI"]["systemSearchBusy"]==false);
                set("normal");host.call(json!({"op":"show","mode":mode,"query":""}));host.wait(|s|{
                    let recovered=host.hidden();
                    let text=recovered["screenText"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
                    s["nativeUI"]["systemSearchBusy"]==false&&s["nativeUI"]["visible"]==true
                        &&recovered["rows"]==if mode=="windows"{5}else{3}
                        &&!["Accessibility","System Settings","Could not load shortcuts"].iter().any(|error|text.contains(error))
                });assert_eq!(host.hidden()["rows"],if mode=="windows"{5}else{3});
            }
        }
        if mode=="windows" {
            assert_eq!(host.call(json!({"op":"usage"})),json!({"fixture.persisted":7}));assert_eq!(fs::read(host.profile.join("usage.json")).unwrap(),usage);assert_eq!(fs::read(host.profile.join("search-ranking.json")).unwrap(),ranking);
        }
        let logs=rpc(&host.socket,&json!({"op":"logs","limit":100})).unwrap();let logpath=PathBuf::from(logs["path"].as_str().unwrap());assert!(logpath.starts_with(&host.profile));
        let text=String::from_utf8(fs::read(logpath).unwrap()).unwrap();assert!(!text.contains("PRIVATE-SHORTCUT"));
        host.stop(&output.join(format!("quit-{theme}-{width}.json")));
    }}
    fixture_owner.verify();
    assert_eq!(captures.len(),24);
    fs::write(output.join("matrix.json"),serde_json::to_vec_pretty(&json!({"captures":captures,"mode":mode,"themes":2,"widths":2,"states":6,"realOS":false,"physicalInput":false,"visibleWindow":false})).unwrap()).unwrap();
}

#[test]
fn menu_original_native_return_and_twenty_four_hidden_states_preserve_captured_actions() {
    let (binary, _sdk, fixture_owner) = qualified_system_ui_image();
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_SYSTEM_UI_EVIDENCE")
            .expect("new owned menu evidence parent required"),
    );
    assert!(parent.is_absolute());
    let output = parent.join("menus");
    fs::create_dir(&output).unwrap();
    let mut captures = vec![];
    for theme in ["light", "dark"] {
        for width in [600, 760] {
            let (host, usage, ranking) =
                owned_system_ui_host("menus", theme, width, &binary, &fixture_owner, &output);
            let set = |variant: &str| {
                host.call(json!({"op":"system.fixture","action":"menu-variant","variant":variant}));
            };
            let settled_menu = |rows: u64, query: &str| {
                host.wait(|s| {
                    s["nativeUI"]["visible"] == true
                        && s["nativeUI"]["systemScope"] == "Menus"
                        && s["nativeUI"]["systemSearchBusy"] == false
                        && host.hidden()["rows"] == rows
                        && host.hidden()["modelQuery"] == query
                })
            };
            let durable = || {
                assert_eq!(
                    host.call(json!({"op":"usage"})),
                    json!({"fixture.persisted":7})
                );
                assert_eq!(fs::read(host.profile.join("usage.json")).unwrap(), usage);
                assert_eq!(
                    fs::read(host.profile.join("search-ranking.json")).unwrap(),
                    ranking
                );
                assert_eq!(host.call(json!({"op":"search-ranking"}))["entries"], 1);
            };
            let denied_cli = |args: &[&str]| {
                let result =
                    Command::new(root().parent().unwrap().join(".build/rust/release/ruycast"))
                        .args(args)
                        .env_clear()
                        .env("HOME", &host.base)
                        .env("RUYCAST_HOME", &host.profile)
                        .env("RUYCAST_APP_BINARY", &binary)
                        .output()
                        .unwrap();
                assert!(
                    !result.status.success(),
                    "denied menu CLI unexpectedly succeeded: {}",
                    String::from_utf8_lossy(&result.stdout)
                );
                let value: Value = serde_json::from_slice(&result.stdout).unwrap();
                assert_eq!(value["ok"], false);
                value
            };
            set("enabled");
            let analytics_before = host.call(json!({"op":"analytics"}));
            let prior_keys = analytics_before["commands"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>();
            let entry = host.cli(&["search", "menu-actions"]);
            assert_eq!(
                entry["result"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|c| c["target"] == "search-menu-items")
                    .count(),
                1
            );
            let found = host.cli(&["search", "", "--mode", "menus"]);
            let rows = found["result"].as_array().unwrap();
            assert_eq!(rows.len(), 4);
            assert!(rows.iter().all(|c| c.get("hotkey").is_none()
                && c.get("aliases")
                    .is_none_or(|aliases| aliases.as_array().is_some_and(Vec::is_empty))));
            let opens = rows
                .iter()
                .filter(|c| c["title"] == "Open…")
                .collect::<Vec<_>>();
            assert_eq!(opens.len(), 2);
            assert_ne!(opens[0]["id"], opens[1]["id"]);
            let foreground = serde_json::to_string(&found["foreground"]).unwrap();
            assert!(found["foreground"].is_object());
            for row in &opens {
                assert_eq!(row["menu"]["application"], "TextEdit Fixture");
                assert_eq!(row["menu"]["path"], "File");
                assert_eq!(row["menu"]["shortcut"]["label"], "Command + O");
            }
            let disabled = rows.iter().find(|c| c["title"] == "Save").unwrap();
            let unknown = rows.iter().find(|c| c["title"] == "Unknown State").unwrap();
            assert_eq!(disabled["enabled"], false);
            assert_eq!(unknown["enabled"], false);
            host.call(json!({"op":"system.fixture","action":"clear-effects"}));
            denied_cli(&["run", opens[0]["id"].as_str().unwrap()]);
            denied_cli(&[
                "run",
                disabled["id"].as_str().unwrap(),
                "--foreground",
                &foreground,
            ]);
            assert_eq!(
                host.call(json!({"op":"system.fixture"}))["effects"],
                json!([])
            );
            // Only the requested captured equal-title element remains enabled.
            for (variant, chosen, refused) in [
                ("first-only", opens[0], opens[1]),
                ("second-only", opens[1], opens[0]),
            ] {
                set(variant);
                host.call(json!({"op":"system.fixture","action":"clear-effects"}));
                assert_eq!(
                    host.cli(&[
                        "run",
                        chosen["id"].as_str().unwrap(),
                        "--foreground",
                        &foreground
                    ])["ok"],
                    true
                );
                assert_eq!(
                    host.call(json!({"op":"system.fixture"}))["effects"],
                    json!(["activate", "AXPress"])
                );
                host.call(json!({"op":"system.fixture","action":"clear-effects"}));
                denied_cli(&[
                    "run",
                    refused["id"].as_str().unwrap(),
                    "--foreground",
                    &foreground,
                ]);
                assert_eq!(
                    host.call(json!({"op":"system.fixture"}))["effects"],
                    json!([])
                );
            }
            durable();
            set("enabled");
            host.call(json!({"op":"show","mode":"menus","query":""}));
            settled_menu(4, "");
            let view = host.hidden();
            assert!(view["accessibilityText"]
                .as_array()
                .unwrap()
                .iter()
                .any(|text| text == "Search app menu…"));
            assert_eq!(view["responderSearch"], true);
            host.select_row(2);
            let disabled_id = host.hidden()["selectedId"].clone();
            host.key(40, 1 << 20, "k");
            host.wait(|_| host.hidden()["actionsVisible"] == true);
            let actions = host.hidden();
            for forbidden in [
                "Open",
                "Run",
                "Set Alias",
                "Set Hotkey",
                "Edit",
                "Enable",
                "Disable",
            ] {
                assert!(
                    !actions["actionLabels"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|label| label.starts_with(forbidden)),
                    "disabled menu action leaked: {actions}"
                );
            }
            host.key(53, 0, "\u{1b}");
            host.wait(|_| host.hidden()["actionsVisible"] == false);
            assert_eq!(host.hidden()["selectedId"], disabled_id);
            host.call(json!({"op":"system.fixture","action":"clear-effects"}));
            host.key(36, 0, "\r");
            settled_menu(4, "");
            assert_eq!(
                host.call(json!({"op":"system.fixture"}))["effects"],
                json!([])
            );
            assert_eq!(
                host.call(json!({"op":"status"}))["nativeUI"]["visible"],
                true
            );
            host.hide();
            host.call(json!({"op":"show","mode":"menus","query":""}));
            settled_menu(4, "");
            assert_eq!(host.hidden()["selectedRow"], 0);
            host.call(json!({"op":"system.fixture","action":"clear-effects"}));
            host.key(36, 0, "\r");
            host.wait(|s| {
                s["nativeUI"]["visible"] == false && s["nativeUI"]["systemSearchBusy"] == false
            });
            assert_eq!(
                host.call(json!({"op":"system.fixture"}))["effects"],
                json!(["activate", "AXPress"])
            );
            durable();
            let analytics = host.call(json!({"op":"analytics"}));
            let key = format!("{:x}", Sha256::digest(b"ruycast.command:system.menu"));
            let mut expected_keys = prior_keys.clone();
            expected_keys.insert(key);
            assert_eq!(
                analytics["commands"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                expected_keys
            );
            let text = serde_json::to_string(&analytics).unwrap();
            for row in rows {
                assert!(!text.contains(row["id"].as_str().unwrap()));
            }
            host.call(json!({"op":"show","mode":"menus","query":"Open"}));
            settled_menu(2, "Open");
            set("refreshed");
            host.action("Refresh");
            host.wait(|s| {
                s["nativeUI"]["visible"] == true && s["nativeUI"]["systemSearchBusy"] == false && {
                    let view = host.hidden();
                    view["rows"] == 2
                        && view["modelQuery"] == "Open"
                        && view["screenText"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter_map(Value::as_str)
                            .any(|text| text.contains("Refreshed Fixture"))
                }
            });
            for variant in ["enabled", "disabled", "long", "empty", "denied", "loading"] {
                host.hide();
                set(variant);
                host.call(json!({"op":"show","mode":"menus","query":""}));
                if variant == "loading" {
                    host.wait(|s| {
                        s["nativeUI"]["visible"] == true
                            && s["nativeUI"]["systemSearchBusy"] == true
                    });
                } else if ["empty", "denied"].contains(&variant) {
                    settled_menu(0, "");
                } else {
                    settled_menu(4, "");
                }
                if variant == "disabled" {
                    host.select_row(2);
                }
                let view = host.hidden();
                let text = view["screenText"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n");
                match variant {
                    "enabled" | "disabled" => {
                        assert!(
                            text.contains("Open…")
                                && text.contains("Save")
                                && text.contains("Unknown State")
                        );
                    }
                    "long" => {
                        assert!(
                            text.contains("Export Presentation with All Linked Resources")
                                && text.contains("Presentation and Sharing")
                                && text.contains("A Long Application Name")
                        );
                    }
                    "empty" => assert!(text.contains("No results")),
                    "denied" => {
                        assert!(text.contains("Accessibility") && text.contains("System Settings"))
                    }
                    "loading" => assert!(text.contains("Loading…")),
                    _ => unreachable!(),
                }
                captures.push(host.capture(&output, "menus", theme, width, variant));
                if ["denied", "loading"].contains(&variant) {
                    if variant == "loading" {
                        host.key(53, 0, "\u{1b}");
                        host.wait(|s| {
                            s["nativeUI"]["visible"] == false
                                && s["nativeUI"]["systemSearchBusy"] == false
                        });
                    }
                    set("enabled");
                    if variant == "denied" {
                        host.action("Refresh");
                    } else {
                        host.call(json!({"op":"show","mode":"menus","query":""}));
                    }
                    settled_menu(4, "");
                    let view = host.hidden();
                    assert!(!view["screenText"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|text| text.contains("Accessibility")
                            || text.contains("System Settings")));
                }
            }
            durable();
            let logs = rpc(&host.socket, &json!({"op":"logs","limit":100})).unwrap();
            let logpath = PathBuf::from(logs["path"].as_str().unwrap());
            assert!(logpath.starts_with(&host.profile));
            let text = String::from_utf8(fs::read(logpath).unwrap()).unwrap();
            for private in [
                "TextEdit Fixture",
                "Refreshed Fixture",
                "Linked Resources",
                "Presentation and Sharing",
            ] {
                assert!(
                    !text.contains(private),
                    "menu metadata leaked to log: {text}"
                );
            }
            host.stop(&output.join(format!("quit-{theme}-{width}.json")));
        }
    }
    fixture_owner.verify();
    assert_eq!(captures.len(), 24);
    fs::write(output.join("matrix.json"), serde_json::to_vec_pretty(&json!({"captures":captures,"mode":"menus","themes":2,"widths":2,"states":6,"realOS":false,"physicalInput":false,"visibleWindow":false})).unwrap()).unwrap();
}
