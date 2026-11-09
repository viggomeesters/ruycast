//! Execute the frozen original Swift object, not a second implementation of it.
//! The only generated Swift is a test JSON-call interface; see ORACLE-BOUNDARY.
use objc2::rc::autoreleasepool;
use objc2_foundation::NSString;
use ruycast_native::utilities::{Effect, Storage, Utilities};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
const ADAPTER: &str = r#"
import Foundation
import LauncherCore
import LauncherModels
@main struct Oracle {
 static func main() throws {
  let cases=try JSONSerialization.jsonObject(with:FileHandle.standardInput.readDataToEndOfFile()) as! [[String:Any]]
  let base=URL(fileURLWithPath:CommandLine.arguments[1]);var output=[[String:Any]]()
  for (index,item) in cases.enumerated() {
   do {
    let action=item["action"] as! String
    if action=="preview" {output.append(["ok":true,"result":Utilities.preview(item["text"] as! String)]);continue}
    if action=="decode" {let data=try JSONSerialization.data(withJSONObject:item["profile"]!);let value=try JSONDecoder().decode(Utilities.Profile.self,from:data);output.append(["ok":true,"result":try JSONSerialization.jsonObject(with:JSONEncoder().encode(value))]);continue}
    let root=base.appendingPathComponent("case-\(index)");try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
    var keychain:Data?=try JSONSerialization.data(withJSONObject:[["value":"fixture-old-password","created":"2000-01-01T00:00:00Z"]])
    var storage=Utilities.Storage();storage.readPasswords={keychain};storage.writePasswords={keychain=$0};storage.random={count in Array((0..<count).map{UInt8($0)})};storage.now={Date(timeIntervalSince1970:1_700_000_000)}
    let utilities=Utilities(root:root,storage:storage)
    if let initial=item["initial"] {try JSONSerialization.data(withJSONObject:initial).write(to:root.appendingPathComponent("utilities.json"))}
    if action=="import" {let counts=try utilities.importProfile(item["profile"] as! [String:Any]);let persisted=try JSONSerialization.jsonObject(with:Data(contentsOf:root.appendingPathComponent("utilities.json")));output.append(["ok":true,"result":["counts":counts,"profile":persisted]]);continue}
    for value in ["first\nline","second","first\nline"] {_=try utilities.execute(target:"utility-set-text",input:value)}
    let texts=try utilities.commands(mode:"text-history",query:"");_=try utilities.execute(target:texts[1].target)
    var pasted="";if case .paste(let text)=try utilities.execute(target:"utility-paste-text") {pasted=text}
    var password="",secret=false;if case .copy(let text,let concealed)=try utilities.execute(target:"utility-password") {password=text;secret=concealed}
    let passwords=try utilities.commands(mode:"password-history",query:"")
    var username="";if case .copy(let text,_)=try utilities.execute(target:"utility-username") {username=text}
    let usernames=try utilities.commands(mode:"username-history",query:"AMBER")
    let encoded:[String:Any]=["texts":try JSONSerialization.jsonObject(with:JSONEncoder().encode(texts)),"passwords":try JSONSerialization.jsonObject(with:JSONEncoder().encode(passwords)),"usernames":try JSONSerialization.jsonObject(with:JSONEncoder().encode(usernames)),"pasted":pasted,"password":password,"secret":secret,"username":username,"keychain":try JSONSerialization.jsonObject(with:keychain!),"profile":try JSONSerialization.jsonObject(with:Data(contentsOf:root.appendingPathComponent("utilities.json")))]
    output.append(["ok":true,"result":encoded])
   }catch{output.append(["ok":false])}
  }
  let data=try JSONSerialization.data(withJSONObject:output,options:[.sortedKeys,.fragmentsAllowed]);print(String(decoding:data,as:UTF8.self))
 }
}
"#;
#[derive(Default, Clone)]
struct Memory {
    profile: std::rc::Rc<std::cell::RefCell<Option<Vec<u8>>>>,
    passwords: std::rc::Rc<std::cell::RefCell<Option<Vec<u8>>>>,
}
impl Storage for Memory {
    fn read_profile(&mut self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.profile.borrow().clone())
    }
    fn save_profile(&mut self, bytes: &[u8]) -> Result<(), String> {
        *self.profile.borrow_mut() = Some(bytes.to_vec());
        Ok(())
    }
    fn read_passwords(&mut self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.passwords.borrow().clone())
    }
    fn save_passwords(&mut self, bytes: &[u8]) -> Result<(), String> {
        *self.passwords.borrow_mut() = Some(bytes.to_vec());
        Ok(())
    }
    fn random(&mut self, count: usize) -> Result<Vec<u8>, String> {
        Ok((0..count).map(|v| v as u8).collect())
    }
    fn created(&mut self) -> String {
        "2023-11-14T22:13:20Z".into()
    }
    fn title_matches(&self, title: &str, query: &str) -> bool {
        autoreleasepool(|_| {
            NSString::from_str(title)
                .localizedCaseInsensitiveContainsString(&NSString::from_str(query))
        })
    }
}
fn rust_case(case: &Value) -> Value {
    let result = (|| -> Result<Value, String> {
        match case["action"].as_str().unwrap() {
            "preview" => {
                return Ok(json!(ruycast_native::utilities::preview(
                    case["text"].as_str().unwrap()
                )))
            }
            "decode" => {
                let profile: ruycast_native::utilities::Profile =
                    serde_json::from_value(case["profile"].clone())
                        .map_err(|_| "invalid profile")?;
                return serde_json::to_value(profile).map_err(|e| e.to_string());
            }
            _ => {}
        }
        let memory = Memory {
            profile: std::rc::Rc::new(std::cell::RefCell::new(
                case.get("initial").map(|v| serde_json::to_vec(v).unwrap()),
            )),
            passwords: std::rc::Rc::new(std::cell::RefCell::new(Some(
                json!([{"value":"fixture-old-password","created":"2000-01-01T00:00:00Z"}])
                    .to_string()
                    .into_bytes(),
            ))),
        };
        let mut utilities = Utilities::new(memory.clone());
        if case["action"] == "import" {
            let counts = utilities.import_profile(case["profile"].clone())?;
            return Ok(
                json!({"counts":counts,"profile":serde_json::from_slice::<Value>(memory.profile.borrow().as_ref().unwrap()).unwrap()}),
            );
        }
        for text in ["first\nline", "second", "first\nline"] {
            utilities.execute("utility-set-text", Some(text))?;
        }
        let texts = utilities.commands("text-history", "")?;
        utilities.execute(&texts[1].target, None)?;
        let Effect::Paste(pasted) = utilities.execute("utility-paste-text", None)? else {
            return Err("missing paste".into());
        };
        let Effect::Copy {
            text: password,
            secret,
        } = utilities.execute("utility-password", None)?
        else {
            return Err("missing copy".into());
        };
        let passwords = utilities.commands("password-history", "")?;
        let Effect::Copy { text: username, .. } = utilities.execute("utility-username", None)?
        else {
            return Err("missing username".into());
        };
        let usernames = utilities.commands("username-history", "AMBER")?;
        let profile: Value =
            serde_json::from_slice(memory.profile.borrow().as_ref().unwrap()).unwrap();
        let keychain: Value =
            serde_json::from_slice(memory.passwords.borrow().as_ref().unwrap()).unwrap();
        Ok(
            json!({"texts":texts,"passwords":passwords,"usernames":usernames,"pasted":pasted,"password":password,"secret":secret,"username":username,"profile":profile,"keychain":keychain}),
        )
    })();
    match result {
        Ok(value) => json!({"ok":true,"result":value}),
        Err(_) => json!({"ok":false}),
    }
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(command: &mut Command, name: &str, out: &Path) {
    let result = command.output().unwrap();
    fs::write(out.join(format!("{name}.stdout")), &result.stdout).unwrap();
    fs::write(out.join(format!("{name}.stderr")), &result.stderr).unwrap();
    assert!(
        result.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn frozen_swift_utilities_and_codable_match_rust_on_owned_cases() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = root.parent().unwrap();
    let out = root.join("target/utility-oracle-evidence");
    fs::create_dir_all(&out).unwrap();
    let parent = PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .expect("Set RUYCAST_NATIVE_TEST_FIXTURE_DIR to an owned absolute fixture directory"));
    assert!(parent.is_absolute());
    fs::create_dir_all(&parent).unwrap();
    let temporary = parent.join(format!("ruycast-utilities-oracle-{}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&temporary)
        .unwrap();
    let _cleanup = Temporary(temporary.clone());
    let adapter = temporary.join("main.swift");
    fs::write(&adapter, ADAPTER).unwrap();
    fs::write(out.join("oracle-main.swift"), ADAPTER).unwrap();
    let modules = temporary.join("modules");
    fs::create_dir(&modules).unwrap();
    let cmodule = modules.join("module.modulemap");
    let map = format!(
        "module CRuycastCore {{ header \"{}\" export * }}\n",
        source
            .join("Sources/CRuycastCore/include/ruycast_core.h")
            .display()
    );
    fs::write(&cmodule, &map).unwrap();
    fs::write(out.join("oracle-module.modulemap"), &map).unwrap();
    run(
        Command::new("swiftc")
            .args([
                "-emit-library",
                "-emit-module",
                "-parse-as-library",
                "-module-name",
                "LauncherModels",
            ])
            .arg(source.join("Sources/LauncherModels/Models.swift"))
            .arg("-emit-module-path")
            .arg(modules.join("LauncherModels.swiftmodule"))
            .arg("-o")
            .arg(temporary.join("libLauncherModels.dylib"))
            .args([
                "-Xlinker",
                "-install_name",
                "-Xlinker",
                "@rpath/libLauncherModels.dylib",
            ]),
        "models",
        &out,
    );
    run(
        Command::new("swiftc")
            .args([
                "-emit-library",
                "-emit-module",
                "-parse-as-library",
                "-module-name",
                "LauncherCore",
                "-I",
            ])
            .arg(&modules)
            .arg("-L")
            .arg(&temporary)
            .arg("-lLauncherModels")
            .arg(source.join("Sources/LauncherCore/Files.swift"))
            .arg(source.join("Sources/LauncherCore/RustBridge.swift"))
            .arg("-emit-module-path")
            .arg(modules.join("LauncherCore.swiftmodule"))
            .arg("-L")
            .arg(source.join(".build/rust/release"))
            .args(["-lrustcast_core", "-liconv", "-lresolv", "-o"])
            .arg(temporary.join("libLauncherCore.dylib"))
            .args([
                "-Xlinker",
                "-install_name",
                "-Xlinker",
                "@rpath/libLauncherCore.dylib",
            ]),
        "core",
        &out,
    );
    let binary = temporary.join("utility-oracle");
    run(
        Command::new("swiftc")
            .args(["-parse-as-library", "-I"])
            .arg(&modules)
            .arg("-L")
            .arg(&temporary)
            .args(["-lLauncherCore", "-lLauncherModels"])
            .arg(source.join("Sources/RuycastApp/Utilities.swift"))
            .arg(&adapter)
            .arg("-o")
            .arg(&binary),
        "adapter",
        &out,
    );
    let mut cases = vec![json!({"action":"scenario"})];
    for text in [
        "".to_string(),
        " \n\t ".into(),
        "a\nb\tc\u{a0}d".into(),
        "👩‍👩‍👧‍👦".repeat(101),
        "e\u{301}".repeat(101),
        "🇳🇱".repeat(100),
        "literal\0middle".into(),
    ] {
        cases.push(json!({"action":"preview","text":text}));
    }
    for profile in [
        json!({}),
        json!({"savedText":"present"}),
        json!({"textHistory":[],"usernameHistory":[]}),
        json!({"savedText":null,"textHistory":[],"usernameHistory":[]}),
        json!({"textHistory":null,"usernameHistory":[]}),
        json!({"textHistory":[],"usernameHistory":[],"unknown":"ignored"}),
    ] {
        cases.push(json!({"action":"decode","profile":profile}));
    }
    for whitespace in [" ", "\r\n", "\u{a0}", "\u{2003}", "\u{202f}", "\u{3000}"] {
        for suffix in ["\u{301}", "\u{fe0f}", "\u{200d}"] {
            cases.push(json!({"action":"preview","text":format!("a{whitespace}{suffix}b")}));
        }
    }
    for stamp in [
        "2026-10-25T00:00:00Z",
        "2026-10-25T00:00:00.123Z",
        "2026-10-25T00:00:00+02:00",
        "invalid",
    ] {
        cases.push(json!({"action":"import","profile":{"textHistory":[{"value":"fixture imported","created":stamp}],"usernameHistory":[]}}));
    }
    cases.push(json!({"action":"import","initial":{"savedText":"existing","textHistory":[],"usernameHistory":[]},"profile":{"savedText":"incoming","textHistory":[],"usernameHistory":[]}}));
    fs::write(
        out.join("cases.json"),
        serde_json::to_vec_pretty(&cases).unwrap(),
    )
    .unwrap();
    let mut child = Command::new(&binary)
        .arg(&temporary)
        .env("DYLD_LIBRARY_PATH", &temporary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&cases).unwrap())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    fs::write(out.join("oracle.stdout"), &result.stdout).unwrap();
    fs::write(out.join("oracle.stderr"), &result.stderr).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let swift: Vec<Value> = serde_json::from_slice(&result.stdout).unwrap();
    let rust: Vec<_> = cases.iter().map(rust_case).collect();
    fs::write(
        out.join("rust.json"),
        serde_json::to_vec_pretty(&rust).unwrap(),
    )
    .unwrap();
    let mut hashes = serde_json::Map::new();
    for path in [
        adapter,
        cmodule,
        binary.clone(),
        temporary.join("libLauncherModels.dylib"),
        temporary.join("libLauncherCore.dylib"),
        source.join("Sources/RuycastApp/Utilities.swift"),
        source.join("Sources/LauncherModels/Models.swift"),
        source.join("Sources/LauncherCore/Files.swift"),
        source.join("Sources/LauncherCore/RustBridge.swift"),
    ] {
        hashes.insert(
            path.to_string_lossy().into_owned(),
            json!(format!("{:x}", Sha256::digest(fs::read(&path).unwrap()))),
        );
    }
    fs::copy(binary, out.join("utility-oracle")).unwrap();
    fs::copy(
        temporary.join("libLauncherModels.dylib"),
        out.join("libLauncherModels.dylib"),
    )
    .unwrap();
    fs::copy(
        temporary.join("libLauncherCore.dylib"),
        out.join("libLauncherCore.dylib"),
    )
    .unwrap();
    fs::write(
        out.join("bindings.json"),
        serde_json::to_vec_pretty(&hashes).unwrap(),
    )
    .unwrap();
    for (index, (swift, rust)) in swift.iter().zip(&rust).enumerate() {
        assert_eq!(swift, rust, "Oracle case {index}: {}", cases[index]);
    }
    assert_eq!(swift.len(), rust.len());
}
