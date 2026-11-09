use super::*;
use std::os::unix::fs::{symlink, FileTypeExt, MetadataExt};
pub fn paths(o: &Options, legacy: bool) -> (PathBuf, PathBuf) {
    match &o.profile {
        Some(p) => (p.clone(), p.clone()),
        None => {
            let new = o.home.join(".config/ruycast");
            (
                if legacy {
                    o.home.join(".config/rustcast")
                } else {
                    new.clone()
                },
                new,
            )
        }
    }
}
pub fn active(old: &Path, new: &Path) -> Result<PathBuf> {
    fs::no_links(new)?;
    if old == new {
        return Ok(new.to_owned());
    }
    if let Ok(m) = std::fs::symlink_metadata(old) {
        if m.file_type().is_symlink() {
            let target = std::fs::read_link(old).map_err(err)?;
            let target = lexical(&if target.is_absolute() {
                target
            } else {
                old.parent().unwrap().join(target)
            })?;
            if target != new || !new.is_dir() {
                return Err("Legacy profile link differs; preserved.".into());
            }
            return Ok(new.to_owned());
        }
        fs::no_links(old)?;
        if new.exists() {
            return Err("Both profiles exist; refusing to merge either.".into());
        }
        if !m.is_dir() {
            return Err("Profile is not a directory; preserved.".into());
        }
        return Ok(old.to_owned());
    }
    Ok(new.to_owned())
}
pub fn migrate(old: &Path, new: &Path) -> Result<PathBuf> {
    let active = active(old, new)?;
    if active == old && old != new {
        std::fs::create_dir_all(new.parent().unwrap()).map_err(err)?;
        fs::rename(old, new, false)?;
    }
    fs::directory(new, 0o700)?;
    if old != new && std::fs::symlink_metadata(old).is_err() {
        symlink(new, old).map_err(err)?;
    }
    Ok(new.to_owned())
}
pub fn config(profile: &Path, override_dir: Option<&str>, home: &Path) -> Result<PathBuf> {
    ruycast_native::profile::ProfilePaths::new(profile.to_owned(), home.to_owned(), override_dir.map(str::to_owned))
    .config_file()
}
pub fn data(
    profile: &Path,
    override_dir: Option<&str>,
    home: &Path,
) -> Result<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    for name in [
        "config.json",
        "config-location.json",
        "clipboard.json",
        "usage.json",
        "paused.json",
        "utilities.json",
        "search-ranking.json",
    ] {
        let path = profile.join(name);
        if std::fs::symlink_metadata(&path).is_ok() {
            let _ = fs::bytes(&path)?;
            out.insert(name.into(), path);
        }
    }
    let selected = config(profile, override_dir, home)?;
    if selected != profile.join("config.json") {
        fs::no_links(&selected)?;
        match std::fs::symlink_metadata(&selected) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(err(error)),
            Ok(_) => {
                let _ = fs::bytes(&selected)?;
                out.insert("selected-config.json".into(), selected);
            }
        }
    }
    Ok(out)
}
pub fn hashes(
    profile: &Path,
    override_dir: Option<&str>,
    home: &Path,
) -> Result<BTreeMap<String, String>> {
    data(profile, override_dir, home)?
        .into_iter()
        .map(|(k, p)| Ok((k, fs::hash(&p)?)))
        .collect()
}
pub fn socket(profile: &Path) -> Result<()> {
    let path = profile.join("control.sock");
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_socket() && m.uid() == unsafe { libc::getuid() } => {
            std::fs::remove_file(path).map_err(err)
        }
        Ok(_) => Err("Unexpected control socket path preserved.".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(err(e)),
    }
}
fn lexical(path: &Path) -> Result<PathBuf> {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return Err("Invalid CLI link.".into());
                }
            }
            Component::CurDir => (),
            _ => out.push(c),
        }
    }
    Ok(out)
}
pub fn link(path: &Path, allowed: &[PathBuf]) -> Result<Option<PathBuf>> {
    fs::no_links(path.parent().unwrap())?;
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            let prior = std::fs::read_link(path).map_err(err)?;
            let full = lexical(&if prior.is_absolute() {
                prior.clone()
            } else {
                path.parent().unwrap().join(&prior)
            })?;
            if !allowed.contains(&full) {
                return Err("Unrelated CLI symlink preserved.".into());
            }
            Ok(Some(prior))
        }
        Ok(_) => Err("Existing non-symlink CLI preserved.".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(err(e)),
    }
}
pub fn set_link(path: &Path, target: &Path, expected: Option<&Path>) -> Result<()> {
    if std::fs::read_link(path).ok().as_deref() != expected
        || expected.is_none() && std::fs::symlink_metadata(path).is_ok()
    {
        return Err("CLI link changed during installation; preserved.".into());
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
    let work = fs::fresh(path.parent().unwrap(), ".ruycast-link-")?;
    let staged = work.join("link");
    symlink(target, &staged).map_err(err)?;
    let result = if expected.is_none() {
        fs::rename(&staged, path, false)
    } else {
        std::fs::rename(&staged, path).map_err(err)
    };
    let _ = std::fs::remove_dir_all(work);
    result?;
    fs::sync_dir(path.parent().unwrap())
}
pub fn adapter(o: &Options, app: &Path, stage: &Path) -> Result<()> {
    let mut source = o
        .adapter
        .clone()
        .unwrap_or_else(|| o.repo.join("scripts/personal_bridge.py"));
    if o.adapter.is_some() {
        fs::absolute(&source)?;
        let _ = fs::bytes(&source)?;
    } else if std::fs::symlink_metadata(&source).is_err() {
        source = app.join("Contents/Resources/personal_bridge.py");
    }
    if std::fs::symlink_metadata(&source).is_ok() {
        let before = fs::bytes(&source)?;
        fs::create(
            &stage.join("Contents/Resources/personal_bridge.py"),
            &before,
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::symlink_metadata(&source)
                    .map_err(err)?
                    .permissions()
                    .mode()
                    & 0o777
            },
        )?;
        if fs::bytes(&source)? != before {
            return Err("Personal adapter changed during staging.".into());
        }
    }
    Ok(())
}
