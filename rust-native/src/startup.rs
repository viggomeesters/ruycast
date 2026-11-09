//! Pure profile selection shared by production entrypoints and isolated probes.
//! Planning never creates a directory, reads a selector or starts native services.
use crate::profile::ProfilePaths;
use std::{
    ffi::OsString,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Default)]
pub struct Environment {
    pub profile: Option<OsString>,
    pub legacy_profile: Option<OsString>,
    pub home: Option<PathBuf>,
    pub config: Option<String>,
    pub legacy_config: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Production,
    ExplicitProbe,
}

fn absolute(path: &Path, cwd: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty()
        || path.as_os_str().as_bytes().contains(&0)
        || path.to_str().is_none()
    {
        return Err("Choose a nonempty UTF-8 profile path without NUL characters.".into());
    }
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    };
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("Choose an absolute profile path without '..' components.".into());
    }
    Ok(path.components().collect())
}
pub fn paths(
    environment: &Environment,
    cwd: &Path,
    fallback_home: Option<&Path>,
    policy: Policy,
) -> Result<ProfilePaths, String> {
    let home = environment
        .home
        .as_deref()
        .filter(|p| !p.as_os_str().is_empty())
        .or(fallback_home);
    let explicit = environment
        .profile
        .as_ref()
        .or(environment.legacy_profile.as_ref());
    let root = match explicit {
        Some(value) => absolute(Path::new(value), cwd)?,
        None if policy == Policy::ExplicitProbe => {
            return Err(
                "An isolated probe requires a nonempty explicit RUYCAST_HOME or RUSTCAST_HOME."
                    .into(),
            )
        }
        None => home
            .ok_or("Cannot determine the home directory for the default Ruycast profile.")?
            .join(".config/ruycast"),
    };
    let home = match home {
        Some(home) => absolute(home, cwd)?,
        None => PathBuf::new(),
    };
    let root = absolute(&root, cwd)?;
    let config_override = environment
        .config
        .clone()
        .or_else(|| environment.legacy_config.clone());
    let paths = ProfilePaths::new(root, home, config_override);
    // Validate explicit config overrides using the existing tilde/absolute-path
    // authority, but do not inspect config-location.json during launch planning.
    if let Some(value) = paths.config_override.as_deref().filter(|s| !s.is_empty()) {
        paths.absolute(value)?;
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_default_and_aliases_never_change_probe_authority() {
        let cwd = Path::new("/owned/work");
        let fallback = Path::new("/owned/home");
        let empty = Environment::default();
        assert_eq!(
            paths(&empty, cwd, Some(fallback), Policy::Production)
                .unwrap()
                .root,
            Path::new("/owned/home/.config/ruycast")
        );
        assert!(paths(&empty, cwd, Some(fallback), Policy::ExplicitProbe).is_err());
        let mut env = Environment {
            legacy_profile: Some("legacy".into()),
            home: Some("/owned/Å".into()),
            legacy_config: Some("~/settings".into()),
            ..Default::default()
        };
        let selected = paths(&env, cwd, None, Policy::ExplicitProbe).unwrap();
        assert_eq!(selected.root, Path::new("/owned/work/legacy"));
        assert_eq!(
            selected
                .absolute(selected.config_override.as_ref().unwrap())
                .unwrap(),
            Path::new("/owned/Å/settings")
        );
        env.profile = Some("/owned/new".into());
        env.config = Some("/owned/config".into());
        let selected = paths(&env, cwd, None, Policy::Production).unwrap();
        assert_eq!(selected.root, Path::new("/owned/new"));
        assert_eq!(selected.config_override.as_deref(), Some("/owned/config"));
        env.profile = Some(OsString::new());
        assert!(paths(&env, cwd, None, Policy::Production).is_err());
    }
    #[test]
    fn invalid_roots_and_missing_home_fail_without_filesystem_fallback() {
        let cwd = Path::new("/owned/work");
        assert!(paths(&Environment::default(), cwd, None, Policy::Production).is_err());
        for value in ["../escape", "/owned/../escape", "bad\0profile"] {
            let env = Environment {
                profile: Some(value.into()),
                ..Default::default()
            };
            assert!(paths(&env, cwd, None, Policy::Production).is_err());
        }
        use std::os::unix::ffi::OsStringExt;
        let invalid = Environment {
            profile: Some(OsString::from_vec(vec![0xff])),
            ..Default::default()
        };
        assert!(paths(&invalid, cwd, None, Policy::ExplicitProbe).is_err());
        let env = Environment {
            profile: Some("/owned/profile".into()),
            ..Default::default()
        };
        assert_eq!(
            paths(&env, cwd, None, Policy::ExplicitProbe).unwrap().root,
            Path::new("/owned/profile")
        );
    }
}
