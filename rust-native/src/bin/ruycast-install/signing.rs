//! Persistent local identity. Never rotate, downgrade, edit trust or expose secrets.
use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
const SCHEMA: &str = "ruycast.local-signing.v1";
pub fn requirement(fingerprint: &str) -> String {
    format!("identifier \"{NEW_ID}\" and certificate leaf = H\"{fingerprint}\"")
}
fn root_ok(root: &Path, repo: &Path) -> Result<()> {
    fs::no_links(repo)?;
    fs::no_links(root)?;
    if root.starts_with(repo) || root.to_str().is_none_or(|p| p.contains(['\r', '\n'])) {
        return Err("Signing secrets must remain outside the repository.".into());
    }
    if let Ok(m) = std::fs::symlink_metadata(root) {
        if !m.is_dir() || m.mode() & 0o077 != 0 || m.uid() != unsafe { libc::getuid() } {
            return Err("Signing directory must be private and owned.".into());
        }
    }
    Ok(())
}
fn private(root: &Path, name: &str) -> Result<Vec<u8>> {
    let p = root.join(name);
    let m = std::fs::symlink_metadata(&p)
        .map_err(|_| "Missing or unsafe signing state; preserve and repair it.".to_string())?;
    if m.mode() & 0o077 != 0 || m.uid() != unsafe { libc::getuid() } {
        return Err("Missing or unsafe signing state; preserve and repair it.".into());
    }
    let bytes = fs::bytes(&p)?;
    if name != "identity.keychain-db" && bytes.len() > 65536 {
        return Err("Signing state exceeds its bound.".into());
    }
    Ok(bytes)
}
pub fn load(root: &Path, repo: &Path) -> Result<Value> {
    root_ok(root, repo)?;
    let raw = private(root, "identity.json")?;
    for name in ["identity.keychain-db", "keychain-password"] {
        let _ = private(root, name)?;
    }
    let certificate = private(root, "certificate.pem")?;
    let value: Value =
        serde_json::from_slice(&raw).map_err(|_| "Invalid pinned signing identity.".to_string())?;
    let fingerprint = value["certificate_sha1"].as_str().unwrap_or("");
    if value["schema"] != SCHEMA
        || fingerprint.len() != 40
        || !fingerprint
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
        || value["certificate_sha256"] != digest(&certificate)
        || value["requirement"] != requirement(fingerprint)
    {
        return Err("Pinned signing identity, certificate or requirement changed; automatic replacement forbidden.".into());
    }
    Ok(value)
}
fn security(r: &mut dyn tools::Runner, home: &Path, args: &[String]) -> Result<()> {
    // security -i parses one quoted command from stdin. Secrets never enter argv.
    if args.iter().any(|a| a.contains(['\n', '\r', '\0'])) {
        return Err("Invalid private keychain command.".into());
    }
    let mut call = tools::Call::new("/usr/bin/security", ["-i"]).env(home, None, None);
    call.input = (args
        .iter()
        .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ")
        + "\n")
        .into_bytes();
    tools::checked(r, call)
        .map(|_| ())
        .map_err(|_| "security: private keychain operation failed.".into())
}
pub fn prepare(root: &Path, repo: &Path, home: &Path, r: &mut dyn tools::Runner) -> Result<Value> {
    root_ok(root, repo)?;
    if std::fs::symlink_metadata(root.join("identity.json")).is_ok() {
        return load(root, repo);
    }
    if root.exists() && std::fs::read_dir(root).map_err(err)?.next().is_some() {
        return Err(
            "Incomplete signing state; repair instead of generating a new identity.".into(),
        );
    }
    fs::directory(root, 0o700)?;
    let secret = r.secret()?;
    if secret.len() != 64 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid signing entropy.".into());
    }
    fs::create(&root.join("keychain-password"), secret.as_bytes(), 0o600)?;
    let keychain = root.join("identity.keychain-db").display().to_string();
    security(
        r,
        home,
        &[
            "create-keychain".into(),
            "-p".into(),
            secret.clone(),
            keychain.clone(),
        ],
    )?;
    tools::checked(
        r,
        tools::Call::new(
            "/usr/bin/security",
            ["set-keychain-settings", "-lut", "600", &keychain],
        )
        .env(home, None, None),
    )?;
    security(
        r,
        home,
        &[
            "unlock-keychain".into(),
            "-p".into(),
            secret.clone(),
            keychain.clone(),
        ],
    )?;
    let work = fs::fresh(root, "provision-")?;
    let result = (|| {
        let openssl = if Path::new("/opt/homebrew/opt/openssl@3/bin/openssl").is_file() {
            "/opt/homebrew/opt/openssl@3/bin/openssl"
        } else {
            "/usr/bin/openssl"
        };
        let private_key = work.join("private.pem").display().to_string();
        let cert = root.join("certificate.pem").display().to_string();
        let p12 = work.join("identity.p12").display().to_string();
        let password_file = format!("file:{}", root.join("keychain-password").display());
        tools::checked(
            r,
            tools::Call::new(
                openssl,
                [
                    "req",
                    "-x509",
                    "-newkey",
                    "rsa:2048",
                    "-sha256",
                    "-noenc",
                    "-days",
                    "3650",
                    "-subj",
                    "/CN=Ruycast Local Signing/O=Viggo Local Apps",
                    "-addext",
                    "keyUsage=critical,digitalSignature",
                    "-addext",
                    "extendedKeyUsage=critical,codeSigning",
                    "-addext",
                    "basicConstraints=critical,CA:FALSE",
                    "-keyout",
                    &private_key,
                    "-out",
                    &cert,
                ],
            )
            .env(home, None, None),
        )?;
        tools::checked(
            r,
            tools::Call::new(
                openssl,
                [
                    "pkcs12",
                    "-export",
                    "-name",
                    "Ruycast Local Signing",
                    "-keypbe",
                    "PBE-SHA1-3DES",
                    "-certpbe",
                    "PBE-SHA1-3DES",
                    "-macalg",
                    "sha1",
                    "-inkey",
                    &private_key,
                    "-in",
                    &cert,
                    "-out",
                    &p12,
                    "-passout",
                    &password_file,
                ],
            )
            .env(home, None, None),
        )?;
        security(
            r,
            home,
            &[
                "import".into(),
                p12,
                "-f".into(),
                "pkcs12".into(),
                "-k".into(),
                keychain.clone(),
                "-P".into(),
                secret.clone(),
                "-T".into(),
                "/usr/bin/codesign".into(),
            ],
        )?;
        security(
            r,
            home,
            &[
                "set-key-partition-list".into(),
                "-S".into(),
                "apple-tool:,apple:,codesign:".into(),
                "-s".into(),
                "-k".into(),
                secret.clone(),
                keychain.clone(),
            ],
        )?;
        let output = tools::checked(
            r,
            tools::Call::new(
                openssl,
                ["x509", "-in", &cert, "-noout", "-fingerprint", "-sha1"],
            )
            .env(home, None, None),
        )?
        .text()?;
        let fingerprint = output
            .trim()
            .split('=')
            .next_back()
            .ok_or("Invalid certificate fingerprint.")?
            .replace(':', "");
        for path in [
            root.join("identity.keychain-db"),
            root.join("certificate.pem"),
        ] {
            fs::no_links(&path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(err)?;
        }
        let value = json!({"schema":SCHEMA,"name":"Ruycast Local Signing","certificate_sha1":fingerprint,"certificate_sha256":fs::hash(&root.join("certificate.pem"))?,"requirement":requirement(&fingerprint)});
        fs::create(
            &root.join("identity.json"),
            &serde_json::to_vec_pretty(&value).map_err(err)?,
            0o600,
        )?;
        load(root, repo)
    })();
    // This is exclusively our reserved private-key staging directory. An
    // incomplete persistent signer remains intact for deliberate repair.
    let _ = std::fs::remove_dir_all(&work);
    result
}
pub fn verify(app: &Path, value: &Value, home: &Path, r: &mut dyn tools::Runner) -> Result<()> {
    tools::checked(
        r,
        tools::Call::new(
            "/usr/bin/codesign",
            [
                "--verify",
                "--deep",
                "--strict",
                "-R",
                &format!(
                    "={}",
                    value["requirement"]
                        .as_str()
                        .ok_or("Missing signing requirement.")?
                ),
                &app.display().to_string(),
            ],
        )
        .env(home, None, None),
    )
    .map(|_| ())
}
fn enrolled(root: &Path, value: &Value) -> Result<bool> {
    let marker = root.join("installed-identity.json");
    if std::fs::symlink_metadata(&marker).is_err() {
        return Ok(false);
    }
    let current: Value =
        serde_json::from_slice(&private(root, "installed-identity.json")?).map_err(err)?;
    if current
        != json!({"schema":"ruycast.installed-signing.v1","certificate_sha1":value["certificate_sha1"],"requirement":value["requirement"]})
    {
        return Err("Installed signing marker changed; preserve pinned identity.".into());
    }
    Ok(true)
}
pub fn check_installed(
    app: &Path,
    root: &Path,
    value: &Value,
    home: &Path,
    r: &mut dyn tools::Runner,
) -> Result<()> {
    if !app.exists() {
        return Ok(());
    }
    let was_enrolled = enrolled(root, value)?;
    tools::checked(
        r,
        tools::Call::new(
            "/usr/bin/codesign",
            ["--verify", "--deep", "--strict", &app.display().to_string()],
        )
        .env(home, None, None),
    )?;
    let details = tools::checked(
        r,
        tools::Call::new(
            "/usr/bin/codesign",
            ["--display", "--verbose=2", &app.display().to_string()],
        )
        .env(home, None, None),
    )?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&details.stdout),
        String::from_utf8_lossy(&details.stderr)
    );
    if text.lines().any(|l| l == "Signature=adhoc")
        && text.lines().any(|l| l == format!("Identifier={NEW_ID}"))
    {
        if was_enrolled {
            return Err("Ad-hoc downgrade refused after persistent enrollment.".into());
        }
        return Ok(());
    }
    verify(app, value, home, r)
}
pub fn sign(
    app: &Path,
    root: &Path,
    repo: &Path,
    home: &Path,
    r: &mut dyn tools::Runner,
) -> Result<Value> {
    let value = load(root, repo)?;
    let keychain = root.join("identity.keychain-db").display().to_string();
    let current = tools::checked(
        r,
        tools::Call::new("/usr/bin/security", ["list-keychains", "-d", "user"])
            .env(home, None, None),
    )?
    .text()?;
    // security emits one double-quoted path per line. Refuse ambiguous output;
    // never replace the user's search list with a guessed interpretation.
    let mut entries = vec![];
    for line in current.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let text = line
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or("Unrecognized keychain search-list output.")?;
        if text.contains(['"', '\\', '\r', '\n']) {
            return Err("Ambiguous keychain search-list entry.".into());
        }
        entries.push(text.to_owned());
    }
    if !entries.contains(&keychain) {
        let mut args = vec![
            "list-keychains".into(),
            "-d".into(),
            "user".into(),
            "-s".into(),
        ];
        args.extend(entries);
        args.push(keychain.clone());
        tools::checked(
            r,
            tools::Call::new("/usr/bin/security", args).env(home, None, None),
        )?;
    }
    let secret = String::from_utf8(private(root, "keychain-password")?)
        .map_err(|_| "Invalid private signing password.".to_string())?;
    security(
        r,
        home,
        &[
            "unlock-keychain".into(),
            "-p".into(),
            secret,
            keychain.clone(),
        ],
    )?;
    tools::checked(
        r,
        tools::Call::new(
            "/usr/bin/codesign",
            [
                "--force",
                "--sign",
                value["certificate_sha1"].as_str().unwrap(),
                "--keychain",
                &keychain,
                "--timestamp=none",
                "--identifier",
                NEW_ID,
                "--requirements",
                &format!("=designated => {}", value["requirement"].as_str().unwrap()),
                &app.display().to_string(),
            ],
        )
        .env(home, None, None),
    )?;
    verify(app, &value, home, r)?;
    Ok(value)
}
pub fn record(
    app: &Path,
    root: &Path,
    repo: &Path,
    value: &Value,
    home: &Path,
    r: &mut dyn tools::Runner,
) -> Result<()> {
    if load(root, repo)? != *value {
        return Err("Pinned signer changed during installation.".into());
    }
    verify(app, value, home, r)?;
    if !enrolled(root, value)? {
        fs::create(&root.join("installed-identity.json"),&serde_json::to_vec_pretty(&json!({"schema":"ruycast.installed-signing.v1","certificate_sha1":value["certificate_sha1"],"requirement":value["requirement"]})).map_err(err)?,0o600)?;
    }
    Ok(())
}
