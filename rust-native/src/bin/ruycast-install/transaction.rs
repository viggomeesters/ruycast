use super::*;
use std::{
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
fn ensure(path: &Path) -> Result<()> {
    fs::no_links(path)?;
    std::fs::create_dir_all(path).map_err(err)?;
    if !path.is_dir() {
        return Err("Expected installation directory.".into());
    }
    Ok(())
}
fn bundle(path: &Path, id: &str, r: &mut dyn tools::Runner) -> Result<Option<Value>> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(err(e)),
        Ok(m) => {
            if m.file_type().is_symlink() || !m.is_dir() {
                return Err("Unrelated app path preserved.".into());
            }
            let info = tools::plist(r, &path.join("Contents/Info.plist"))?;
            if info["CFBundleIdentifier"] != id {
                return Err("Unrelated app identity preserved.".into());
            }
            let _ = fs::tree(path)?;
            Ok(Some(info))
        }
    }
}
fn agent(
    path: &Path,
    label: &str,
    app: &Path,
    host: &str,
    r: &mut dyn tools::Runner,
) -> Result<Option<Vec<u8>>> {
    if std::fs::symlink_metadata(path).is_err() {
        return Ok(None);
    }
    let value = tools::plist(r, path)?;
    if value["Label"] != label
        || value["ProgramArguments"] != json!([app.join("Contents/MacOS").join(host)])
    {
        return Err("Unrelated login registration preserved.".into());
    }
    Ok(Some(fs::bytes(path)?))
}
fn call(r: &mut dyn tools::Runner, home: &Path, args: Vec<String>, checked: bool) -> Result<()> {
    let c = tools::Call::new("/bin/launchctl", args).env(home, None, None);
    if checked {
        tools::checked(r, c).map(|_| ())
    } else {
        r.run(c).map(|_| ())
    }
}
pub fn install(o: &Options, lease: &Lease, r: &mut dyn tools::Runner) -> Result<Value> {
    fs::no_links(&o.repo)?;
    fs::no_links(&o.home)?;
    fs::no_links(&o.apps)?;
    lease.validate()?;
    if lease
        .environment()?
        .get("RUYCAST_ARTIFACT_LEASE_ROOT")
        .map(PathBuf::from)
        != Some(o.repo.canonicalize().map_err(err)?)
    {
        return Err("Installer artifact lease belongs to a different source repository.".into());
    }
    let (receipt, receipt_bytes) = receipt::verify(&o.repo, &o.home, lease, r)?;
    ensure(&o.apps)?;
    let app = o.apps.join("Ruycast.app");
    let old_app = o.apps.join("Rustcast.app");
    let current_info = bundle(&app, NEW_ID, r)?;
    let old_info = bundle(&old_app, OLD_ID, r).ok().flatten();
    let legacy_cli = o.home.join(".local/bin/rustcast");
    let legacy = old_info.is_some()
        || std::fs::symlink_metadata(&old_app).is_err()
            && current_info.is_some()
            && profile::link(&legacy_cli, &[app.join("Contents/Resources/ruycast")])
                .ok()
                .flatten()
                .is_some();
    let signing = signing::prepare(&o.signer, &o.repo, &o.home, r)?;
    signing::check_installed(&app, &o.signer, &signing, &o.home, r)?;
    let mut allowed = vec![app.join("Contents/Resources/ruycast")];
    if legacy {
        allowed.push(old_app.join("Contents/Resources/rustcast"));
    }
    let mut links = vec![o.home.join(".local/bin/ruycast")];
    if legacy {
        links.push(legacy_cli);
    }
    let prior_links: Vec<_> = links
        .iter()
        .map(|p| profile::link(p, &allowed).map(|v| (p.clone(), v)))
        .collect::<Result<_>>()?;
    let agents = o.home.join("Library/LaunchAgents");
    let new_agent = agents.join(format!("{NEW_ID}.plist"));
    let old_agent = agents.join(format!("{OLD_ID}.plist"));
    let new_before = agent(&new_agent, NEW_ID, &app, "RuycastApp", r)?;
    let old_before = if legacy {
        agent(&old_agent, OLD_ID, &old_app, "RustcastApp", r)?
    } else {
        None
    };
    let previous = if new_before.is_some() {
        tools::plist(r, &new_agent)?
    } else {
        json!({})
    };
    let config = o.config.clone().or_else(|| {
        previous["EnvironmentVariables"]["RUYCAST_CONFIG_DIR"]
            .as_str()
            .map(str::to_owned)
    });
    if previous["EnvironmentVariables"]
        .get("RUYCAST_CONFIG_DIR")
        .is_some_and(|v| !v.is_string())
    {
        return Err("Invalid installed configuration override.".into());
    }
    let (old_profile, new_profile) = profile::paths(o, legacy);
    let active = profile::active(&old_profile, &new_profile)?;
    fs::directory(&active, 0o700)?;
    let initial_hashes = profile::hashes(&active, config.as_deref(), &o.home)?;
    let authored_config = profile::config(&active, config.as_deref(), &o.home)?;
    let existing_config = std::fs::symlink_metadata(&authored_config).is_ok();
    if existing_config {
        let value: Value = serde_json::from_slice(&fs::bytes(&authored_config)?).map_err(err)?;
        rustcast_core::validate(&value)?;
    }
    let build = current_info
        .as_ref()
        .map(|v| match v.get("CFBundleVersion") {
            None => Ok(0),
            Some(Value::Number(n)) => n
                .as_u64()
                .ok_or_else(|| "Existing build number is not numeric.".to_string()),
            Some(Value::String(s)) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
                s.parse::<u64>().map_err(err)
            }
            _ => Err("Existing build number is not numeric.".to_string()),
        })
        .transpose()?
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("Build number overflow.")?
        .max(7);
    // All staging is exclusive. A same-named user's unknown .dist app is never read.
    let work = fs::fresh(&o.apps, ".ruycast-install-")?;
    let candidate = work.join("Ruycast.app");
    let staged = (|| -> Result<()> {
        ensure(&candidate.join("Contents/MacOS"))?;
        ensure(&candidate.join("Contents/Resources"))?;
        for (source, component) in receipt.paths() {
            let target = if component == "host" {
                "Contents/MacOS/RuycastApp"
            } else {
                "Contents/Resources/ruycast"
            };
            let bytes = fs::bytes(&o.repo.join(&source))?;
            if receipt.artifacts.get(&source) != Some(&digest(&bytes)) {
                return Err("Staged binaries differ from v3 receipt.".into());
            }
            fs::create(&candidate.join(target), &bytes, 0o755)?;
        }
        let icon = o.repo.join("assets/AppIcon.icns");
        let icon_present = std::fs::symlink_metadata(&icon).is_ok();
        if icon_present {
            fs::create(
                &candidate.join("Contents/Resources/AppIcon.icns"),
                &fs::bytes(&icon)?,
                0o644,
            )?;
        }
        profile::adapter(o, &app, &candidate)?;
        let mut info = json!({"CFBundleIdentifier":NEW_ID,"CFBundleName":"Ruycast","CFBundleDisplayName":"Ruycast","CFBundleExecutable":"RuycastApp","CFBundlePackageType":"APPL","CFBundleShortVersionString":VERSION,"CFBundleVersion":build.to_string(),"LSMinimumSystemVersion":"14.0","LSUIElement":true,"NSHighResolutionCapable":true,"NSAppleEventsUsageDescription":"Ruycast searches open Safari tabs and switches to the tab you select."});
        if icon_present {
            info["CFBundleIconFile"] = json!("AppIcon");
        }
        fs::create(
            &candidate.join("Contents/Info.plist"),
            &tools::plist_bytes(&info)?,
            0o644,
        )?;
        signing::sign(&candidate, &o.signer, &o.repo, &o.home, r)?;
        if !existing_config {
            tools::checked(
                r,
                tools::Call::new(
                    candidate.join("Contents/MacOS/RuycastApp"),
                    ["--init-config"],
                )
                .env(&o.home, Some(&active), config.as_deref()),
            )?;
        }
        // Missing-only initialization must actually produce a readable, valid
        // selected configuration before any existing app is stopped.
        let initialized: Value =
            serde_json::from_slice(&fs::bytes(&authored_config)?).map_err(err)?;
        rustcast_core::validate(&initialized)?;
        let after_init = profile::hashes(&active, config.as_deref(), &o.home)?;
        if initial_hashes
            .iter()
            .any(|(k, v)| after_init.get(k) != Some(v))
        {
            return Err("Existing configuration or history changed while initializing; installation preserved.".into());
        }
        lease.validate()?;
        let (_, now) = receipt::verify(&o.repo, &o.home, lease, r)?;
        if now != receipt_bytes {
            return Err("Build receipt changed during staging.".into());
        }
        Ok(())
    })();
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&work);
        return Err(e);
    }
    let candidate_files = fs::tree(&candidate)?;
    let app_before = if app.exists() {
        Some(fs::tree(&app)?)
    } else {
        None
    };
    let old_app_before = if legacy && old_app.exists() {
        Some(fs::tree(&old_app)?)
    } else {
        None
    };
    let backups = o.home.join(".local/state/ruycast/install-backups");
    fs::directory(&backups, 0o700)?;
    let backup = fs::fresh(&backups, "install-")?;
    if app_before.is_some() {
        fs::copy_tree(&app, &backup.join("Ruycast.app"))?;
    }
    if old_app_before.is_some() {
        fs::copy_tree(&old_app, &backup.join("Rustcast.app"))?;
    }
    for (path, before) in [(&new_agent, &new_before), (&old_agent, &old_before)] {
        if let Some(bytes) = before {
            fs::create(&backup.join(path.file_name().unwrap()), bytes, 0o600)?;
        }
    }
    let saved_links: BTreeMap<_, _> = prior_links
        .iter()
        .map(|(p, t)| {
            (
                p.display().to_string(),
                t.as_ref().map(|p| p.display().to_string()),
            )
        })
        .collect();
    fs::create(
        &backup.join("cli-links.json"),
        &serde_json::to_vec_pretty(&saved_links).map_err(err)?,
        0o600,
    )?;
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    let installed_cli = app.join("Contents/Resources/ruycast");
    let mut replaced = false;
    let mut migrated = false;
    let mut wrote_agent: Option<Vec<u8>> = None;
    let mut updated_links = vec![];
    let mut guard = None;
    let mut before = BTreeMap::new();
    let transaction = (|| -> Result<Value> {
        let cli_source = o.repo.join(&receipt.paths()[1].0);
        let _ = r.run(tools::Call::new(cli_source, ["quit"]).env(
            &o.home,
            Some(&active),
            config.as_deref(),
        ))?;
        call(
            r,
            &o.home,
            vec!["bootout".into(), format!("{domain}/{NEW_ID}")],
            false,
        )?;
        if legacy {
            call(
                r,
                &o.home,
                vec!["bootout".into(), format!("{domain}/{OLD_ID}")],
                false,
            )?;
        }
        guard = Some(fs::Lock::acquire(&active.join("instance.lock"), 60)?);
        before = profile::hashes(&active, config.as_deref(), &o.home)?;
        for (name, path) in profile::data(&active, config.as_deref(), &o.home)? {
            fs::create(&backup.join(name), &fs::bytes(&path)?, 0o600)?;
        }
        profile::migrate(&old_profile, &new_profile)?;
        migrated = true;
        profile::socket(&new_profile)?;
        if profile::hashes(&new_profile, config.as_deref(), &o.home)? != before {
            return Err("Profile data changed during migration.".into());
        }
        if app_before
            .as_ref()
            .map(|old| fs::tree(&app).map(|now| now != *old))
            .transpose()?
            .unwrap_or_else(|| std::fs::symlink_metadata(&app).is_ok())
        {
            return Err("Installed app changed before replacement; preserved.".into());
        }
        if fs::tree(&candidate)? != candidate_files {
            return Err("Signed candidate changed before replacement.".into());
        }
        lease.validate()?;
        fs::rename(&candidate, &app, app_before.is_some())?;
        replaced = true;
        fs::sync_dir(&o.apps)?;
        ensure(&agents)?;
        let mut environment = json!({"RUYCAST_HOME":new_profile});
        if let Some(config) = config.as_deref().filter(|s| !s.is_empty()) {
            environment["RUYCAST_CONFIG_DIR"] = json!(config);
        }
        let agent = json!({"Label":NEW_ID,"ProgramArguments":[app.join("Contents/MacOS/RuycastApp")],"RunAtLoad":true,"KeepAlive":{"SuccessfulExit":false},"ThrottleInterval":10,"EnvironmentVariables":environment,"StandardOutPath":new_profile.join("launcher.log"),"StandardErrorPath":new_profile.join("launcher.log")});
        let agent_bytes = tools::plist_bytes(&agent)?;
        wrote_agent = Some(agent_bytes.clone());
        fs::replace(&new_agent, &agent_bytes, new_before.as_deref(), 0o600)?;
        let log = new_profile.join("launcher.log");
        if std::fs::symlink_metadata(&log).is_err() {
            fs::create(&log, b"", 0o600)?;
        } else {
            let _ = fs::bytes(&log)?;
            std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o600)).map_err(err)?;
        }
        for (path, original) in &prior_links {
            updated_links.push((path.clone(), original.clone()));
            profile::set_link(path, &installed_cli, original.as_deref())?;
        }
        call(
            r,
            &o.home,
            vec!["enable".into(), format!("{domain}/{NEW_ID}")],
            true,
        )?;
        drop(guard.take());
        call(
            r,
            &o.home,
            vec![
                "bootstrap".into(),
                domain.clone(),
                new_agent.display().to_string(),
            ],
            true,
        )?;
        let deadline = Instant::now() + Duration::from_secs(8);
        let status = loop {
            let result = r.run(
                tools::Call::new(&installed_cli, ["status"])
                    .env(&o.home, Some(&new_profile), config.as_deref())
                    .timeout(Duration::from_millis(500)),
            );
            lease.validate()?;
            if let Ok(value) = result {
                if value.success {
                    let value = value.json()?;
                    if value["ok"] == true {
                        break value;
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err("Startup could not be verified; restoring prior installation.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if status["result"]["name"] != "Ruycast" || status["result"]["version"] != VERSION {
            return Err("Installed host identity/version differs from tested source.".into());
        }
        let pid = status["result"]["pid"]
            .as_i64()
            .and_then(|pid| i32::try_from(pid).ok())
            .filter(|pid| *pid > 1)
            .ok_or("Installed host did not report a valid process identity.")?;
        r.running_host(pid, &app.join("Contents/MacOS/RuycastApp"))?;
        let expected_config = profile::config(&new_profile, config.as_deref(), &o.home)?;
        let reported = status["result"]["configPath"]
            .as_str()
            .ok_or("Installed host did not report its configuration path.")?;
        if std::fs::canonicalize(reported).map_err(err)?
            != std::fs::canonicalize(&expected_config).map_err(err)?
        {
            return Err("Installed host uses a different configuration folder.".into());
        }
        let after = profile::hashes(&new_profile, config.as_deref(), &o.home)?;
        for name in [
            "config.json",
            "config-location.json",
            "selected-config.json",
        ] {
            if before.get(name) != after.get(name) {
                return Err("Configuration changed during installation.".into());
            }
        }
        lease.validate()?;
        signing::verify(&app, &signing, &o.home, r)?;
        call(
            r,
            &o.home,
            vec!["print".into(), format!("{domain}/{NEW_ID}")],
            true,
        )?;
        lease.validate()?;
        signing::record(&app, &o.signer, &o.repo, &signing, &o.home, r)?;
        Ok(
            json!({"installed":app,"cli":prior_links[0].0,"legacyCLI":if legacy{Some(&prior_links[1].0)}else{None},"config":expected_config,"legacyProfile":old_profile,"loginAgent":new_agent,"signing":{"method":"persistent-local-certificate","certificate_sha1":signing["certificate_sha1"],"requirement":signing["requirement"]},"backup":backup,"preservedDataHashes":before,"status":status}),
        )
    })();
    if let Err(error) = transaction {
        drop(guard.take());
        // Rollback is conditional on our installed bytes still owning each path.
        // A competing edit is preserved and reported instead of overwritten.
        let rollback = (|| -> Result<()> {
            if replaced {
                let _ = r.run(tools::Call::new(&installed_cli, ["quit"]).env(
                    &o.home,
                    Some(&new_profile),
                    config.as_deref(),
                ));
            }
            call(
                r,
                &o.home,
                vec!["bootout".into(), format!("{domain}/{NEW_ID}")],
                false,
            )?;
            let rollback_profile = if migrated { &new_profile } else { &active };
            let rollback_guard = fs::Lock::acquire(&rollback_profile.join("instance.lock"), 60)?;
            profile::socket(rollback_profile)?;
            if replaced {
                if fs::tree(&app)? != candidate_files {
                    return Err(
                        "Installed candidate was externally changed; app and backup preserved."
                            .into(),
                    );
                }
                if app_before.is_some() {
                    fs::rename(&candidate, &app, true)?;
                } else {
                    fs::rename(&app, &candidate, false)?;
                }
                fs::sync_dir(&o.apps)?;
            }
            if let Some(written) = &wrote_agent {
                let current = fs::bytes(&new_agent).ok();
                if current.as_ref() == Some(written) {
                    if let Some(prior) = &new_before {
                        fs::replace(&new_agent, prior, Some(written), 0o600)?;
                    } else {
                        std::fs::remove_file(&new_agent).map_err(err)?;
                    }
                } else if current != new_before {
                    return Err("Login registration changed; preserved.".into());
                }
            }
            for (path, original) in updated_links.iter().rev() {
                let current = std::fs::read_link(path).ok();
                if current == *original
                    && !(current.is_none() && std::fs::symlink_metadata(path).is_ok())
                {
                    continue;
                }
                if let Some(original) = original {
                    profile::set_link(path, original, Some(&installed_cli))?;
                } else {
                    if std::fs::read_link(path).map_err(err)? != installed_cli {
                        return Err("CLI link changed; preserved.".into());
                    }
                    std::fs::remove_file(path).map_err(err)?;
                }
            }
            drop(rollback_guard);
            let restore = if old_before.is_some() {
                &old_agent
            } else {
                &new_agent
            };
            if (old_before.is_some() || new_before.is_some()) && restore.exists() {
                call(
                    r,
                    &o.home,
                    vec![
                        "bootstrap".into(),
                        domain.clone(),
                        restore.display().to_string(),
                    ],
                    false,
                )?;
            }
            Ok(())
        })();
        if let Err(rollback) = rollback {
            return Err(format!(
                "{error} Rollback requires repair: {rollback} Backup preserved: {}",
                backup.display()
            ));
        }
        let _ = std::fs::remove_dir_all(&work);
        return Err(error);
    }
    let mut result = transaction.unwrap();
    let lsregister=PathBuf::from("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister");
    let mut warnings = vec![];
    // No fallible acceptance operation follows retirement. Unknown changes to
    // the legacy product are retained, not treated as owned cleanup.
    if legacy {
        if let Some(expected) = old_app_before {
            if fs::tree(&old_app).ok().as_ref() == Some(&expected) {
                if lsregister.is_file() {
                    let _ = r.run(
                        tools::Call::new(&lsregister, ["-u", &old_app.display().to_string()])
                            .env(&o.home, None, None),
                    );
                }
                if std::fs::remove_dir_all(&old_app).is_err() {
                    warnings.push("Legacy app cleanup could not complete.");
                }
            } else {
                warnings.push("Legacy app changed and was preserved.");
            }
        }
        if let Some(prior) = old_before {
            if fs::bytes(&old_agent).ok().as_ref() == Some(&prior) {
                if std::fs::remove_file(&old_agent).is_err() {
                    warnings.push("Legacy registration cleanup could not complete.");
                }
            } else {
                warnings.push("Legacy registration changed and was preserved.");
            }
        }
    }
    if lsregister.is_file() {
        let _ = r.run(
            tools::Call::new(lsregister, ["-f", &app.display().to_string()])
                .env(&o.home, None, None),
        );
    }
    if std::fs::remove_dir_all(&work).is_err() {
        warnings.push("Owned staging backup retained.");
    }
    result["warnings"] = json!(warnings);
    Ok(result)
}
