//! Known runtime failures, separate from candidate/release acceptance.
use crate::keyboard_service::HookHealth;
use rustcast_core::Config;

#[derive(Default, Clone, Copy, Debug)]
pub struct Requirements {
    pub hyper: bool,
    pub snippets: bool,
    pub sided: bool,
    pub modifier_taps: bool,
}
impl Requirements {
    pub fn from_config(config: &Config) -> Self {
        let mut needed = Self {
            hyper: config.hyper_key_enabled,
            snippets: config.snippet_expansion_enabled,
            ..Self::default()
        };
        for key in std::iter::once(config.launcher_hotkey.as_str())
            .chain(std::iter::once(config.clipboard_hotkey.as_str()))
            .chain(
                config
                    .commands
                    .iter()
                    .filter(|c| c.enabled != Some(false))
                    .filter_map(|c| c.hotkey.as_deref()),
            )
        {
            let key = key.to_lowercase();
            needed.sided |= key.starts_with("double:") || key.split('+').any(|part| {
                matches!(part, "right-alt" | "fn")
                    || config.separate_command_keys.unwrap_or(true) && matches!(part, "left-cmd" | "right-cmd")
            });
            needed.modifier_taps |= key
                .split('+')
                .next_back()
                .is_some_and(|p| matches!(p, "left-control" | "left-option"));
        }
        needed
    }
    pub fn input(self) -> bool {
        self.hyper || self.snippets || self.sided || self.modifier_taps
    }
}
pub struct Known<'a> {
    pub permissions: Option<&'a crate::permissions::Snapshot>,
    pub storage_error: bool,
    pub headless: bool,
    pub keyboard_present: bool,
    pub keyboard_error: bool,
    pub keyboard_pending: bool,
    pub recording: bool,
    pub paused: bool,
    pub hooks: &'a HookHealth,
    pub keyword: &'a str,
}
pub fn healthy(needed: Requirements, known: &Known<'_>) -> bool {
    if known.storage_error || known.keyboard_error {
        return false;
    }
    if known.headless {
        return true;
    }
    if needed.input()
        && known
            .permissions
            .is_some_and(|p| !p.accessibility || !p.input_monitoring || p.raycast_running)
    {
        return false;
    }
    if !known.keyboard_present || known.keyboard_pending {
        return false;
    }
    // Recording and sleep/session pause intentionally suspend global routing;
    // those gates alone do not prove a failed physical listener.
    if known.recording || known.paused {
        return true;
    }
    (!needed.hyper || known.hooks.hyper == "On")
        && (!needed.sided || known.hooks.sided == "On")
        && (!needed.modifier_taps || known.hooks.modifier_taps == "On")
        && (!needed.snippets || known.keyword == "On")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deliberate_disabled_and_headless_are_not_errors_but_known_failures_are() {
        let hooks = HookHealth::default();
        let mut k = Known {
            permissions: None,
            storage_error: false,
            headless: false,
            keyboard_present: true,
            keyboard_error: false,
            keyboard_pending: false,
            recording: false,
            paused: false,
            hooks: &hooks,
            keyword: "Off",
        };
        assert!(healthy(Requirements::default(), &k));
        k.keyboard_present = false;
        assert!(!healthy(Requirements::default(), &k));
        k.headless = true;
        assert!(healthy(
            Requirements {
                hyper: true,
                snippets: true,
                ..Default::default()
            },
            &k
        ));
        k.storage_error = true;
        assert!(!healthy(Requirements::default(), &k));
        k.storage_error = false;
        k.keyboard_error = true;
        assert!(!healthy(Requirements::default(), &k));
    }
    #[test]
    fn actual_hook_and_keyword_state_are_required_without_fabricating_permissions() {
        let hooks = HookHealth {
            hyper: "On".into(),
            sided: "On".into(),
            modifier_taps: "On".into(),
            ..Default::default()
        };
        let needed = Requirements {
            hyper: true,
            snippets: true,
            sided: true,
            modifier_taps: true,
        };
        let make = |h| Known {
            permissions: None,
            storage_error: false,
            headless: false,
            keyboard_present: true,
            keyboard_error: false,
            keyboard_pending: false,
            recording: false,
            paused: false,
            hooks: h,
            keyword: "On",
        };
        assert!(healthy(needed, &make(&hooks)));
        for permissions in [
            crate::permissions::Snapshot {
                accessibility: false,
                input_monitoring: true,
                raycast_running: false,
            },
            crate::permissions::Snapshot {
                accessibility: true,
                input_monitoring: false,
                raycast_running: false,
            },
            crate::permissions::Snapshot {
                accessibility: true,
                input_monitoring: true,
                raycast_running: true,
            },
        ] {
            let mut k = make(&hooks);
            k.permissions = Some(&permissions);
            assert!(!healthy(needed, &k));
        }
        // Original PolishChecks isolates these three preference/trust/conflict
        // tuples. Combined requirements alone do not prove the disabled case.
        for (needed, trusted, raycast_running, expected) in [
            (Requirements { hyper: true, ..Default::default() }, true, true, false),
            (Requirements { snippets: true, ..Default::default() }, false, false, false),
            (Requirements::default(), false, true, true),
        ] {
            let permissions = crate::permissions::Snapshot {
                accessibility: trusted,
                input_monitoring: true,
                raycast_running,
            };
            let mut known = make(&hooks);
            known.permissions = Some(&permissions);
            assert_eq!(healthy(needed, &known), expected);
        }
        let recovering = HookHealth {
            sided: "Recovering".into(),
            ..hooks.clone()
        };
        assert!(!healthy(needed, &make(&recovering)));
        let mut k = make(&recovering);
        k.recording = true;
        assert!(healthy(needed, &k));
        k.recording = false;
        k.paused = true;
        assert!(healthy(needed, &k));
        k.keyboard_error = true;
        assert!(!healthy(needed, &k));
    }
    #[test]
    fn disabled_and_combined_command_keys_follow_configuration() {
        let mut value = rustcast_core::default_config();
        value["hyperKeyEnabled"] = false.into();
        value["snippetExpansionEnabled"] = false.into();
        value["launcherHotkey"] = "cmd+space".into();
        value["clipboardHotkey"] = "cmd+shift+v".into();
        value["commands"] = serde_json::json!([{"id":"disabled","title":"Disabled","kind":"app","target":"owned","hotkey":"left-cmd+a","enabled":false}]);
        let mut config = rustcast_core::validate(&value).unwrap();
        assert!(!Requirements::from_config(&config).input());
        config.commands[0].enabled = Some(true);
        config.separate_command_keys = Some(true);
        assert!(Requirements::from_config(&config).sided);
        config.separate_command_keys = Some(false);
        assert!(!Requirements::from_config(&config).sided);
        config.launcher_hotkey = "ctrl+left-control".into();
        assert!(Requirements::from_config(&config).modifier_taps);
    }
}
