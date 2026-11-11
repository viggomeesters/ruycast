//! Replay genuine frozen Swift utility results through the original Rust Memory consumer.
use objc2::rc::autoreleasepool;
use objc2_foundation::NSString;
use ruycast_native::utilities::{Effect, Storage, Utilities};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
const GOLDEN: &str = "rust-native/tests/utility_reference.json";
const SHA: &str = "8f794a224f1f6173aad72be6ccddd8e630273e9f6c85656a323506626180081d";
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
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

#[test]
fn frozen_swift_utilities_and_codable_match_rust_on_owned_cases() {
    let root = root();
    let bytes = fs::read(root.join(GOLDEN)).unwrap();
    assert_eq!(hash(&bytes), SHA);
    let golden: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(golden["schema"], "ruycast.utilities-swift-goldens.v1");
    assert_eq!(golden["caseCount"], 37);
    let cases = golden["cases"].as_array().unwrap();
    let expected = golden["expected"].as_array().unwrap();
    assert_eq!(cases.len(), 37);
    assert_eq!(expected.len(), 37);
    let mut categories = BTreeMap::new();
    for case in cases {
        *categories
            .entry(case["action"].as_str().unwrap())
            .or_insert(0) += 1;
    }
    assert_eq!(
        categories,
        BTreeMap::from([
            ("scenario", 1),
            ("preview", 25),
            ("decode", 6),
            ("import", 5)
        ])
    );
    assert_eq!(
        serde_json::to_value(categories).unwrap(),
        golden["categories"]
    );
    // Retain the original consumer, full ordered records and length assertion.
    let rust: Vec<_> = cases.iter().map(rust_case).collect();
    assert_eq!(rust.len(), expected.len());
    for (index, (swift, rust)) in expected.iter().zip(&rust).enumerate() {
        assert_eq!(swift, rust, "Oracle case {index}: {}", cases[index]);
    }
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        fs::create_dir_all(&directory).unwrap();
        let sources = [
            GOLDEN,
            "rust-native/tests/utility_oracle.rs",
            "rust-native/src/utilities.rs",
            "rust-native/src/native_text.rs",
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust/Cargo.toml",
            "rust/Cargo.lock",
            "rust/src/lib.rs",
            "rust-toolchain.toml",
        ]
        .into_iter()
        .map(|p| (p, hash(&fs::read(root.join(p)).unwrap())))
        .collect::<BTreeMap<_, _>>();
        let executable = std::env::current_exe().unwrap();
        assert!(executable.is_absolute() && executable.is_file());
        let artifact = if executable.starts_with(&root) {
            executable
        } else {
            let copy = root.join(format!(
                "rust-native/target/utility-oracle-rust-{}",
                std::process::id()
            ));
            assert!(!copy.exists());
            fs::create_dir_all(copy.parent().unwrap()).unwrap();
            fs::copy(&executable, &copy).unwrap();
            assert_eq!(
                hash(&fs::read(executable).unwrap()),
                hash(&fs::read(&copy).unwrap())
            );
            copy
        };
        let name = artifact.strip_prefix(&root).unwrap().to_str().unwrap();
        fs::write(PathBuf::from(directory).join("utility-reference-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":{name:hash(&fs::read(&artifact).unwrap())},"goldenSHA256":SHA,"cases":37,"scope":"Frozen genuine Swift utility corpus through unchanged Rust Memory consumer; no runtime Swift compiler, product file writes, Keychain reads, clipboard access or applied desktop effects"})).unwrap()).unwrap();
    }
}
