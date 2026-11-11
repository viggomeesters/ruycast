//! Transient physical keyboard state and sided shortcut filtering. No listener,
//! permission request, persistent mapping, text history or native handle.
use crate::recorder::{COMMAND, CONTROL, LEFT_COMMAND, OPTION, RIGHT_COMMAND, SHIFT};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

pub const CAPS: u64 = 1 << 16;
pub const FUNCTION: u64 = 1 << 23;
pub const HYPER: u64 = CONTROL | OPTION | SHIFT | COMMAND;
pub const GENERATED_HYPER: i64 = 0x52554859504552;
pub const GENERATED_SNIPPET: i64 = 0x5255535443415354;
pub const LEFT_OPTION: u64 = 0x20;
pub const RIGHT_OPTION: u64 = 0x40;

#[derive(Default)]
pub struct TapSequence {
    held: BTreeSet<u16>,
    pressed: Option<(u16, Instant)>,
    released: Option<(u16, Instant)>,
}

impl TapSequence {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn observe(&mut self, key: u16, down: bool, repeated: bool, now: Instant) -> Option<bool> {
        if repeated {
            self.pressed = None;
            self.released = None;
            return None;
        }
        if down {
            if !self.held.insert(key) || self.held.len() > 1 {
                self.pressed = None;
                self.released = None;
                return None;
            }
            if self.released.is_some_and(|(previous, _)| previous != key) {
                self.released = None;
            }
            self.pressed = Some((key, now));
            return None;
        }
        if !self.held.remove(&key) || !self.pressed.take().is_some_and(|(previous, start)| {
            previous == key && now.saturating_duration_since(start) <= Duration::from_millis(250)
        }) {
            self.pressed = None;
            self.released = None;
            return None;
        }
        if self.released.take().is_some_and(|(previous, end)| {
            previous == key && now.saturating_duration_since(end) <= Duration::from_millis(350)
        }) {
            return Some(true);
        }
        self.released = Some((key, now));
        Some(false)
    }
}

pub fn modifier_key(code: u16) -> Option<(&'static str, &'static str, u64, u64)> {
    Some(match code {
        59 => ("left-ctrl", "ctrl", CONTROL, 0x1),
        62 => ("right-ctrl", "ctrl", CONTROL, 0x2000),
        58 => ("left-alt", "alt", OPTION, LEFT_OPTION),
        61 => ("right-alt", "alt", OPTION, RIGHT_OPTION),
        55 => ("left-cmd", "cmd", COMMAND, LEFT_COMMAND),
        54 => ("right-cmd", "cmd", COMMAND, RIGHT_COMMAND),
        56 => ("left-shift", "shift", SHIFT, 0x2),
        60 => ("right-shift", "shift", SHIFT, 0x4),
        63 => ("fn", "fn", FUNCTION, FUNCTION),
        _ => return None,
    })
}

#[derive(Clone, Debug)]
struct Tap {
    device: u64,
    usage: u32,
    time: f64,
}
#[derive(Default, Debug)]
pub struct Physical {
    held: BTreeMap<u64, BTreeSet<u32>>,
    candidate: Option<Tap>,
}
impl Physical {
    pub fn held(&self) -> &BTreeMap<u64, BTreeSet<u32>> {
        &self.held
    }
    pub fn usages(&self) -> BTreeSet<u32> {
        self.held.values().flat_map(|v| v.iter().copied()).collect()
    }
    pub fn caps_held(&self) -> bool {
        self.held.values().any(|v| v.contains(&0x39))
    }
    pub fn cancel_tap(&mut self) {
        self.candidate = None;
    }
    pub fn input(&mut self, device: u64, usage: u32, down: bool, time: f64) -> Option<u32> {
        let already = self.held.get(&device).is_some_and(|v| v.contains(&usage));
        if already == down {
            return None;
        }
        if down {
            let solo = self.held.is_empty();
            self.candidate = if solo && [0xe0, 0xe2].contains(&usage) {
                Some(Tap {
                    device,
                    usage,
                    time,
                })
            } else {
                None
            };
            self.held.entry(device).or_default().insert(usage);
            None
        } else {
            let action = self.candidate.take().and_then(|c| {
                (c.device == device && c.usage == usage && time >= c.time && time - c.time <= 0.5)
                    .then_some(usage)
            });
            if let Some(keys) = self.held.get_mut(&device) {
                keys.remove(&usage);
                if keys.is_empty() {
                    self.held.remove(&device);
                }
            }
            action
        }
    }
    pub fn remove(&mut self, device: u64) {
        self.held.remove(&device);
        self.cancel_tap();
    }
    pub fn reset(&mut self) {
        self.held.clear();
        self.cancel_tap();
    }
    pub fn modifiers(&self) -> u64 {
        self.held
            .values()
            .flat_map(|v| v.iter())
            .fold(0, |bits, usage| {
                bits | match usage {
                    0xe0 => CONTROL | 0x1,
                    0xe1 => SHIFT | 0x2,
                    0xe2 => OPTION | 0x20,
                    0xe3 => COMMAND | LEFT_COMMAND,
                    0xe4 => CONTROL | 0x2000,
                    0xe5 => SHIFT | 0x4,
                    0xe6 => OPTION | 0x40,
                    0xe7 => COMMAND | RIGHT_COMMAND,
                    _ => 0,
                }
            })
    }
}
pub fn generated(tag: i64) -> bool {
    [GENERATED_HYPER, GENERATED_SNIPPET].contains(&tag)
}
pub fn rewritten_flags(original: u64, held: bool, tag: i64) -> u64 {
    if generated(tag) {
        original
    } else {
        (original & !CAPS) | if held { HYPER } else { 0 }
    }
}

/// A permitted environment is not an installed/healthy hook. The native owner
/// must still acquire and enable every resource before reporting `On`.
#[derive(Clone, Copy, Debug)]
pub struct Environment {
    pub sleeping: bool,
    pub raycast_running: bool,
    pub secure_input: bool,
    pub accessibility: bool,
    pub input_monitoring: bool,
}
impl Environment {
    pub fn blocked(self, hyper: bool, modifiers: bool, sided: bool) -> Option<&'static str> {
        if !hyper && !modifiers && !sided {
            Some("Off")
        } else if self.sleeping {
            Some("Paused")
        } else if self.raycast_running {
            Some("Quit Raycast to use keyboard shortcuts")
        } else if self.secure_input {
            Some("Paused during secure input")
        } else if !self.accessibility {
            Some("Allow Accessibility in System Settings")
        } else if !self.input_monitoring {
            Some("Allow Input Monitoring in System Settings")
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub key: u16,
    pub flags: u64,
    pub side: u64,
    pub side_mask: u64,
    pub action: String,
}
impl Binding {
    pub fn parse(value: &str, action: String) -> Result<Self, String> {
        let canonical = rustcast_core::shortcut(value)?;
        let mut parts = canonical.split('+').collect::<Vec<_>>();
        let key = parts.pop().expect("Validated shortcut has a key");
        let command_side = if parts.contains(&"right-cmd") {
            RIGHT_COMMAND
        } else if parts.contains(&"left-cmd") {
            LEFT_COMMAND
        } else {
            0
        };
        let right_alt = parts.contains(&"right-alt");
        if command_side == 0 && !right_alt && !parts.contains(&"fn") {
            return Err("An extended shortcut is required.".into());
        }
        let side = command_side | if right_alt { RIGHT_OPTION } else { 0 };
        let side_mask = if command_side != 0 { LEFT_COMMAND | RIGHT_COMMAND } else { 0 }
            | if right_alt { LEFT_OPTION | RIGHT_OPTION } else { 0 };
        let code = match key {
            "left-control" => Some(59),
            "left-option" => Some(58),
            _ => (0..=127).find(|&c| {
                crate::recorder::key_name(c)
                    == Some(if key == "backspace" { "delete" } else { key })
            }),
        }
        .ok_or_else(|| format!("Unsupported shortcut key: {key}"))?;
        let flags = parts.iter().fold(0, |bits, m| {
            bits | match *m {
                "ctrl" => CONTROL,
                "alt" | "right-alt" => OPTION,
                "cmd" | "left-cmd" | "right-cmd" => COMMAND,
                "fn" => FUNCTION,
                "shift" => SHIFT,
                _ => 0,
            }
        });
        Ok(Self {
            key: code,
            flags,
            side,
            side_mask,
            action,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Down,
    Up,
    Other,
}
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Decision {
    pub consumed: bool,
    pub action: Option<String>,
}
#[derive(Default, Debug)]
pub struct CommandFilter {
    bindings: Vec<Binding>,
    swallowed: BTreeSet<u16>,
}
impl CommandFilter {
    /// As with the original filter, the owner resets physical state when its
    /// configuration changes; assigning the binding list alone is not reset.
    pub fn set_bindings(&mut self, bindings: Vec<Binding>) {
        self.bindings = bindings;
    }
    pub fn reset(&mut self) {
        self.swallowed.clear();
    }
    /// Preview does not change swallowed ownership. The native sink admits a
    /// new action before committing consumption, without cloning bindings.
    pub fn preview(&self, event: Event, key: u16, flags: u64, repeated: bool) -> Decision {
        if event == Event::Up {
            return Decision {
                consumed: self.swallowed.contains(&key),
                action: None,
            };
        }
        if event != Event::Down {
            return Decision::default();
        }
        if self.swallowed.contains(&key) {
            return Decision {
                consumed: true,
                action: None,
            };
        }
        let modifiers = flags & (COMMAND | CONTROL | OPTION | SHIFT | FUNCTION);
        let binding = (!repeated)
            .then(|| {
                self.bindings
                    .iter()
                    .rev()
                    .filter(|b| b.key == key && b.side == flags & b.side_mask && b.flags == modifiers)
                    .max_by_key(|binding| binding.side_mask.count_ones())
            })
            .flatten();
        if let Some(binding) = binding {
            Decision {
                consumed: true,
                action: Some(binding.action.clone()),
            }
        } else {
            Decision::default()
        }
    }
    pub fn commit(&mut self, event: Event, key: u16, consumed: bool) {
        if consumed {
            match event {
                Event::Down => {
                    self.swallowed.insert(key);
                }
                Event::Up => {
                    self.swallowed.remove(&key);
                }
                Event::Other => {}
            }
        }
    }
    pub fn handle(&mut self, event: Event, key: u16, flags: u64, repeated: bool) -> Decision {
        let decision = self.preview(event, key, flags, repeated);
        self.commit(event, key, decision.consumed);
        decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn double_taps_require_two_short_releases_of_the_same_key() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        for key in [59, 58, 55, 49, 63, 54] {
            let mut sequence = TapSequence::default();
            assert_eq!(sequence.observe(key, true, false, at(0)), None);
            assert_eq!(sequence.observe(key, false, false, at(50)), Some(false));
            assert_eq!(sequence.observe(key, true, false, at(100)), None);
            assert_eq!(sequence.observe(key, false, false, at(150)), Some(true));
            assert_eq!(sequence.observe(key, false, false, at(151)), None);
            sequence.observe(key, true, false, at(200));
            assert_eq!(sequence.observe(key, false, false, at(500)), None);
            sequence.observe(key, true, false, at(600));
            sequence.reset();
            assert_eq!(sequence.observe(key, false, false, at(610)), None);
            sequence.observe(key, true, false, at(700));
            sequence.observe(0, true, false, at(710));
            assert_eq!(sequence.observe(key, false, false, at(720)), None);
            assert_eq!(sequence.observe(0, false, false, at(730)), None);
        }
    }

    #[test]
    fn first_equal_binding_wins_while_more_specific_sides_keep_priority() {
        let mut filter = CommandFilter::default();
        filter.set_bindings(vec![
            Binding::parse("right-cmd+k", "first".into()).unwrap(),
            Binding::parse("right-cmd+k", "second".into()).unwrap(),
        ]);
        assert_eq!(
            filter
                .preview(Event::Down, 40, COMMAND | RIGHT_COMMAND, false)
                .action
                .as_deref(),
            Some("first")
        );
        filter.set_bindings(vec![
            Binding::parse("fn+alt+k", "generic first".into()).unwrap(),
            Binding::parse("fn+right-alt+k", "specific first".into()).unwrap(),
            Binding::parse("fn+right-alt+k", "specific second".into()).unwrap(),
            Binding::parse("fn+alt+k", "generic last".into()).unwrap(),
        ]);
        assert_eq!(
            filter
                .preview(Event::Down, 40, FUNCTION | OPTION | RIGHT_OPTION, false)
                .action
                .as_deref(),
            Some("specific first")
        );
        assert_eq!(
            filter
                .preview(Event::Down, 40, FUNCTION | OPTION | LEFT_OPTION, false)
                .action
                .as_deref(),
            Some("generic first")
        );
    }
    #[test]
    fn right_option_and_fn_chords_keep_their_modifier_boundaries() {
        let mut filter = CommandFilter::default();
        filter.set_bindings(vec![
            Binding::parse("right-alt+k", "right".into()).unwrap(),
            Binding::parse("fn+k", "function".into()).unwrap(),
            Binding::parse("fn+alt+k", "generic option".into()).unwrap(),
            Binding::parse("fn+right-alt+k", "right option with fn".into()).unwrap(),
        ]);
        let key = 40;
        assert_eq!(filter.preview(Event::Down, key, OPTION | LEFT_OPTION, false).action, None);
        assert_eq!(filter.preview(Event::Down, key, OPTION | RIGHT_OPTION, false).action.as_deref(), Some("right"));
        assert_eq!(filter.preview(Event::Down, key, FUNCTION, false).action.as_deref(), Some("function"));
        assert_eq!(filter.preview(Event::Down, key, FUNCTION | COMMAND, false).action, None);
        assert_eq!(filter.preview(Event::Down, key, FUNCTION | OPTION | RIGHT_OPTION, false).action.as_deref(), Some("right option with fn"));
    }
    #[test]
    fn isolated_tap_boundary_duplicates_strays_and_chords() {
        let mut p = Physical::default();
        assert_eq!(p.input(1, 0xe0, true, 1.), None);
        assert_eq!(p.input(1, 0xe0, true, 1.1), None);
        assert_eq!(p.input(2, 4, false, 1.2), None);
        assert_eq!(p.input(1, 0xe0, false, 1.5), Some(0xe0));
        p.input(1, 0xe2, true, 2.);
        assert_eq!(p.input(1, 0xe2, false, 2.500_001), None);
        p.input(1, 0xe2, true, 3.);
        p.input(2, 4, true, 3.1);
        p.input(2, 4, false, 3.2);
        assert_eq!(p.input(1, 0xe2, false, 3.3), None);
        assert!(p.held().is_empty());
    }
    #[test]
    fn both_devices_and_removal_preserve_physical_modifiers() {
        let mut p = Physical::default();
        p.input(1, 0x39, true, 0.);
        p.input(2, 0x39, true, 0.);
        p.input(2, 0xe7, true, 0.);
        p.input(1, 0x39, false, 0.1);
        assert!(p.caps_held());
        assert_eq!(p.modifiers(), COMMAND | RIGHT_COMMAND);
        p.remove(2);
        assert!(!p.caps_held());
        assert_eq!(p.modifiers(), 0);
        p.reset();
        assert!(p.usages().is_empty());
    }
    #[test]
    fn generated_events_and_unowned_flags_are_not_rewritten() {
        let physical = COMMAND | RIGHT_COMMAND | CAPS | (1 << 40);
        assert_eq!(
            rewritten_flags(physical, true, 0),
            (physical & !CAPS) | HYPER
        );
        assert_eq!(rewritten_flags(physical, false, 0), physical & !CAPS);
        for tag in [GENERATED_HYPER, GENERATED_SNIPPET] {
            assert_eq!(rewritten_flags(physical, true, tag), physical);
        }
    }
    #[test]
    fn sided_press_repeat_and_release_keep_ownership() {
        let mut f = CommandFilter::default();
        f.set_bindings(vec![Binding::parse("right-cmd+k", "right".into()).unwrap()]);
        assert_eq!(
            f.handle(Event::Down, 40, COMMAND, false),
            Decision::default()
        );
        assert_eq!(
            f.handle(
                Event::Down,
                40,
                COMMAND | RIGHT_COMMAND | LEFT_COMMAND,
                false
            ),
            Decision::default()
        );
        assert_eq!(
            f.handle(Event::Down, 40, COMMAND | RIGHT_COMMAND | FUNCTION, false),
            Decision::default()
        );
        assert_eq!(
            f.handle(Event::Down, 40, COMMAND | RIGHT_COMMAND, false)
                .action
                .as_deref(),
            Some("right")
        );
        f.set_bindings(vec![]);
        assert!(f.handle(Event::Down, 40, 0, true).consumed);
        assert!(f.handle(Event::Up, 40, 0, false).consumed);
        assert!(!f.handle(Event::Up, 40, 0, false).consumed);
        f.reset();
    }
}
