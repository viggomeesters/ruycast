use super::*;
use objc2_app_kit::NSRunningApplication;
use objc2_foundation::NSString;
use std::{os::unix::fs::PermissionsExt, thread, time::Duration};

const ID: &str = "nl.viggomeesters.ruycast.preview";
const NAME: &str = "Ruycast Rust Preview.app";

fn metadata(app: &Path, runner: &mut dyn tools::Runner) -> Result<(u64, String)> {
    let plist = tools::plist(runner, &app.join("Contents/Info.plist"))?;
    if plist["CFBundleIdentifier"] != ID || plist["CFBundleExecutable"] != "RuycastApp" {
        return Err("Expected a Ruycast Rust Preview bundle.".into());
    }
    let build = plist["CFBundleVersion"]
        .as_str()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or("Expected a positive preview build number.")?;
    let version = plist["CFBundleShortVersionString"]
        .as_str()
        .ok_or("Missing preview version.")?;
    Ok((build, version.to_owned()))
}

fn running() -> bool {
    NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(ID))
        .iter()
        .any(|app| !app.isTerminated())
}

fn verify_identity(
    app: &Path,
    identity: &Value,
    home: &Path,
    runner: &mut dyn tools::Runner,
) -> Result<()> {
    signing::verify(app, identity, home, runner)?;
    let details = tools::checked(
        runner,
        tools::Call::new(
            "/usr/bin/codesign",
            ["--display", "-r-", &app.display().to_string()],
        )
        .env(home, None, None),
    )?;
    let expected = format!(
        "designated => {}",
        identity["requirement"].as_str().unwrap()
    );
    if ![&details.stdout, &details.stderr].into_iter().any(|bytes| {
        String::from_utf8_lossy(bytes)
            .lines()
            .any(|line| line.eq_ignore_ascii_case(&expected))
    }) {
        return Err("Preview designated requirement differs from the enrolled identity.".into());
    }
    Ok(())
}

fn installed_process(app: &Path, process: &NSRunningApplication) -> bool {
    let executable = app.join("Contents/MacOS/RuycastApp");
    !process.isTerminated()
        && process
            .executableURL()
            .and_then(|url| url.path())
            .is_some_and(|path| Path::new(&path.to_string()) == executable)
}

fn wait_until_stopped() -> bool {
    for _ in 0..40 {
        if !running() {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }
    !running()
}

fn status(
    app: &Path,
    home: &Path,
    profile: &Path,
    version: &str,
    runner: &mut dyn tools::Runner,
) -> Result<Value> {
    let output = tools::checked(
        runner,
        tools::Call::new(app.join("Contents/Resources/ruycast"), ["status"])
            .env(home, Some(profile), None)
            .timeout(Duration::from_secs(2)),
    )?;
    let value = output.json()?;
    if value["ok"] != true || value["result"]["pid"].as_u64().is_none() {
        return Err("Preview status unavailable.".into());
    }
    let pid = value["result"]["pid"].as_i64();
    let matches =
        NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(ID))
            .iter()
            .any(|process| {
                Some(i64::from(process.processIdentifier())) == pid
                    && installed_process(app, &process)
            });
    let result = &value["result"];
    if !matches
        || result["profile"].as_str().map(Path::new) != Some(profile)
        || result["implementation"] != "rust"
        || result["version"] != version
        || ["accessibility", "inputMonitoring"]
            .into_iter()
            .any(|key| !result[key].is_boolean())
    {
        return Err("Could not verify the running Rust preview and profile.".into());
    }
    Ok(value["result"].clone())
}

fn launch(app: &Path, home: &Path, runner: &mut dyn tools::Runner) -> Result<()> {
    tools::checked(
        runner,
        tools::Call::new(
            "/usr/bin/open",
            [
                "-gj",
                "--env",
                &format!(
                    "RUYCAST_HOME={}",
                    home.join("Library/Application Support/Ruycast Rust Preview")
                        .display()
                ),
                "--env",
                "RUYCAST_CONFIG_DIR=",
                &app.display().to_string(),
            ],
        )
        .env(home, None, None),
    )?;
    Ok(())
}

pub fn install(args: &[String]) -> Result<()> {
    let [source] = args else {
        return Err("Usage: ruycast-install install-preview ABSOLUTE_APP".into());
    };
    let source = fs::absolute(Path::new(source))?;
    let repo = fs::absolute(&std::env::current_dir().map_err(err)?)?;
    let home = fs::absolute(Path::new(&objc2_foundation::NSHomeDirectory().to_string()))?;
    let apps = home.join("Applications");
    fs::no_links(&apps)?;
    let app = apps.join(NAME);
    if source.starts_with(&app) || source == app {
        return Err("Provide a separate, staged preview bundle.".into());
    }
    let signer = home.join(".local/share/ruycast/signing");
    let profile = home.join("Library/Application Support/Ruycast Rust Preview");
    let mut runner = tools::System;
    let _lock = fs::Lock::acquire(&apps.join(".ruycast-preview-install.lock"), 0)?;

    // This is an update of the enrolled preview. The existing signature pins
    // the signer even if the on-disk signing configuration has changed.
    let mut identity = signing::load(&signer, &repo)?;
    identity["requirement"] = json!(signing::requirement_for(
        ID,
        identity["certificate_sha1"].as_str().unwrap(),
    ));
    fs::no_links(&app)?;
    verify_identity(&app, &identity, &home, &mut runner)?;
    let (previous_build, previous_version) = metadata(&app, &mut runner)?;
    let stage = fs::fresh(&apps, ".ruycast-preview-update-")?;
    let candidate = stage.join(NAME);
    let mut swapped = false;
    let result = (|| {
        fs::copy_tree(&source, &candidate)?;
        let (build, version) = metadata(&candidate, &mut runner)?;
        for relative in ["Contents/MacOS/RuycastApp", "Contents/Resources/ruycast"] {
            let metadata = std::fs::metadata(candidate.join(relative)).map_err(err)?;
            if !metadata.is_file() || metadata.permissions().mode() & 0o100 == 0 {
                return Err(format!(
                    "Preview executable missing or not executable: {relative}"
                ));
            }
        }
        if build <= previous_build {
            return Err("The preview update must have a higher build number.".into());
        }
        signing::sign_bundle(&candidate, &signer, &repo, &home, &mut runner, ID)?;
        verify_identity(&candidate, &identity, &home, &mut runner)?;
        let host = tools::checked(
            &mut runner,
            tools::Call::new(candidate.join("Contents/MacOS/RuycastApp"), ["--version"])
                .env(&home, Some(&profile), None)
                .timeout(Duration::from_secs(2)),
        )?
        .json()?;
        if host["ok"] != true
            || host["result"]["implementation"] != "rust"
            || host["result"]["localPreview"] != true
            || host["result"]["version"] != version
        {
            return Err(
                "Build the candidate with the local-preview feature before installing.".into(),
            );
        }
        let before = status(&app, &home, &profile, &previous_version, &mut runner)?;
        let old_pid = before["pid"].as_u64().unwrap();
        let quit = tools::checked(
            &mut runner,
            tools::Call::new(app.join("Contents/Resources/ruycast"), ["quit"])
                .env(&home, Some(&profile), None)
                .timeout(Duration::from_secs(2)),
        );
        if !wait_until_stopped() {
            return Err("The running preview did not exit; update cancelled.".into());
        }
        if let Err(error) = quit {
            launch(&app, &home, &mut runner)?;
            return Err(format!(
                "Preview shutdown returned an error; restarted the previous build: {error}"
            ));
        }
        let profile_lock = match fs::Lock::acquire(&profile.join("instance.lock"), 10) {
            Ok(lock) => lock,
            Err(error) => {
                launch(&app, &home, &mut runner)?;
                return Err(error);
            }
        };
        if let Err(error) = fs::rename(&app, &candidate, true) {
            drop(profile_lock);
            launch(&app, &home, &mut runner)?;
            return Err(error);
        }
        drop(profile_lock);
        swapped = true;
        let readback = (|| {
            launch(&app, &home, &mut runner)?;
            for attempt in 0..40 {
                if let Ok(after) = status(&app, &home, &profile, &version, &mut runner) {
                    if after["pid"].as_u64() != Some(old_pid)
                        && (before["health"] != "healthy" || after["health"] == "healthy")
                        && ["accessibility", "inputMonitoring"]
                            .into_iter()
                            .all(|key| before[key] != true || after[key] == true)
                    {
                        return Ok(after);
                    }
                }
                if attempt == 8 && !running() {
                    launch(&app, &home, &mut runner)?;
                }
                thread::sleep(Duration::from_millis(250));
            }
            Err("Preview startup or permission readback failed.".to_owned())
        })();
        match readback {
            Ok(after) => {
                println!(
                    "{}",
                    json!({"build":build,"rollback":candidate,"accessibility":after["accessibility"],"inputMonitoring":after["inputMonitoring"]})
                );
                Ok(())
            }
            Err(error) => {
                for process in NSRunningApplication::runningApplicationsWithBundleIdentifier(
                    &NSString::from_str(ID),
                )
                .iter()
                {
                    if installed_process(&app, &process) {
                        process.terminate();
                    }
                }
                if !wait_until_stopped() {
                    return Err(format!(
                        "{error} Preview still running. Previous bundle: {}.",
                        candidate.display()
                    ));
                }
                let profile_lock = fs::Lock::acquire(&profile.join("instance.lock"), 10).map_err(|cause| format!("{error} Cannot lock preview for rollback: {cause}. Previous bundle: {}.", candidate.display()))?;
                fs::rename(&app, &candidate, true).map_err(|cause| {
                    format!(
                        "{error} Rollback failed: {cause}. Previous bundle: {}.",
                        candidate.display()
                    )
                })?;
                drop(profile_lock);
                launch(&app, &home, &mut runner)?;
                for _ in 0..40 {
                    if status(&app, &home, &profile, &previous_version, &mut runner).is_ok() {
                        return Err(format!("{error} Previous preview restored and running."));
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                Err(format!(
                    "{error} Previous bundle restored, but startup could not be verified."
                ))
            }
        }
    })();
    // Once swapped, the staging path may hold the previous signed app.
    // Retain it as the rollback copy, including when a rollback fails.
    if !swapped {
        let _ = std::fs::remove_dir_all(stage);
    }
    result
}
