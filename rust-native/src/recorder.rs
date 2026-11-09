//! Layout-independent physical key recording, matching the shipped recorder.
//! No hook or event listener: the native focused control supplies these events.
pub const CONTROL: u64 = 1 << 18;
pub const OPTION: u64 = 1 << 19;
pub const SHIFT: u64 = 1 << 17;
pub const COMMAND: u64 = 1 << 20;
pub const LEFT_COMMAND: u64 = 0x8;
pub const RIGHT_COMMAND: u64 = 0x10;
pub fn key_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0 => "a",
        1 => "s",
        2 => "d",
        3 => "f",
        4 => "h",
        5 => "g",
        6 => "z",
        7 => "x",
        8 => "c",
        9 => "v",
        11 => "b",
        12 => "q",
        13 => "w",
        14 => "e",
        15 => "r",
        16 => "y",
        17 => "t",
        18 => "1",
        19 => "2",
        20 => "3",
        21 => "4",
        22 => "6",
        23 => "5",
        25 => "9",
        26 => "7",
        28 => "8",
        29 => "0",
        30 => "]",
        31 => "o",
        32 => "u",
        33 => "[",
        34 => "i",
        35 => "p",
        37 => "l",
        38 => "j",
        40 => "k",
        45 => "n",
        46 => "m",
        48 => "tab",
        49 => "space",
        36 => "return",
        123 => "left",
        124 => "right",
        125 => "down",
        126 => "up",
        39 => "'",
        41 => ";",
        50 => "`",
        51 => "delete",
        53 => "escape",
        43 => ",",
        47 => ".",
        44 => "/",
        27 => "-",
        24 => "=",
        42 => "\\",
        122 => "f1",
        120 => "f2",
        99 => "f3",
        118 => "f4",
        96 => "f5",
        97 => "f6",
        98 => "f7",
        100 => "f8",
        101 => "f9",
        109 => "f10",
        103 => "f11",
        111 => "f12",
        _ => return None,
    })
}

#[derive(Debug, PartialEq)]
pub enum Recorded {
    Ignore,
    Cancel,
    Clear,
    Shortcut(String),
}
#[derive(Default)]
pub struct Recorder {
    solo: Option<u16>,
}
impl Recorder {
    pub fn reset(&mut self) {
        self.solo = None;
    }
    pub fn key(&mut self, code: u16, flags: u64, repeated: bool, separate: bool) -> Recorded {
        if repeated {
            return Recorded::Ignore;
        }
        self.solo = None;
        if code == 53 {
            return Recorded::Cancel;
        }
        let mods = flags & (CONTROL | OPTION | SHIFT | COMMAND);
        if code == 51 && mods == 0 {
            return Recorded::Clear;
        }
        let mut names: Vec<&str> = [
            (CONTROL, "ctrl"),
            (OPTION, "alt"),
            (SHIFT, "shift"),
            (COMMAND, "cmd"),
        ]
        .into_iter()
        .filter(|(flag, _)| mods & flag != 0)
        .map(|(_, name)| name)
        .collect();
        if names.is_empty() {
            return Recorded::Ignore;
        }
        if separate && names.len() != 4 {
            if let Some(i) = names.iter().position(|&name| name == "cmd") {
                match flags & (LEFT_COMMAND | RIGHT_COMMAND) {
                    LEFT_COMMAND => names[i] = "left-cmd",
                    RIGHT_COMMAND => names[i] = "right-cmd",
                    _ => {}
                }
            }
        }
        let Some(key) = key_name(code) else {
            return Recorded::Ignore;
        };
        Recorded::Shortcut(format!("{}+{key}", names.join("+")))
    }
    pub fn flags(&mut self, code: u16, flags: u64) -> Recorded {
        let flags = flags & (CONTROL | OPTION | SHIFT | COMMAND);
        if (code == 59 && flags == CONTROL) || (code == 58 && flags == OPTION) {
            self.solo = Some(code);
        } else if flags == 0 {
            if let Some(code) = self.solo.take() {
                return Recorded::Shortcut(
                    if code == 59 {
                        "ctrl+left-control"
                    } else {
                        "alt+left-option"
                    }
                    .into(),
                );
            }
        } else {
            self.solo = None;
        }
        Recorded::Ignore
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_keys_sides_hyper_and_clear_match_shipped_contract() {
        let mut r = Recorder::default();
        assert_eq!(
            r.key(45, COMMAND | RIGHT_COMMAND, false, true),
            Recorded::Shortcut("right-cmd+n".into())
        );
        assert_eq!(
            r.key(45, COMMAND | LEFT_COMMAND, false, true),
            Recorded::Shortcut("left-cmd+n".into())
        );
        assert_eq!(
            r.key(45, COMMAND | RIGHT_COMMAND, false, false),
            Recorded::Shortcut("cmd+n".into())
        );
        assert_eq!(
            r.key(
                45,
                COMMAND | CONTROL | SHIFT | OPTION | RIGHT_COMMAND,
                false,
                true
            ),
            Recorded::Shortcut("ctrl+alt+shift+cmd+n".into())
        );
        assert_eq!(r.key(51, 0, false, true), Recorded::Clear);
        assert_eq!(
            r.key(51, COMMAND, false, true),
            Recorded::Shortcut("cmd+delete".into())
        );
        assert_eq!(r.key(53, COMMAND, false, true), Recorded::Cancel);
        assert_eq!(r.key(45, COMMAND, true, true), Recorded::Ignore);
        assert_eq!(r.key(45, 0, false, true), Recorded::Ignore);
    }
    #[test]
    fn solo_modifier_candidates_are_cancelled_by_chords_and_reset() {
        let mut r = Recorder::default();
        assert_eq!(r.flags(59, CONTROL), Recorded::Ignore);
        assert_eq!(
            r.flags(59, 0),
            Recorded::Shortcut("ctrl+left-control".into())
        );
        r.flags(58, OPTION);
        assert_eq!(r.flags(58, 0), Recorded::Shortcut("alt+left-option".into()));
        r.flags(59, CONTROL);
        r.key(45, CONTROL, false, true);
        assert_eq!(r.flags(59, 0), Recorded::Ignore);
        r.flags(58, OPTION);
        r.reset();
        assert_eq!(r.flags(58, 0), Recorded::Ignore);
    }
}
