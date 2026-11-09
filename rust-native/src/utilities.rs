//! Utilities domain. The serialized host owns this value; native effects and
//! Keychain access are injected so tests never inspect a personal pasteboard or
//! password entry. Password values never enter command metadata.
use rustcast_core::Command;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

type Result<T> = std::result::Result<T, String>;
const MAX_BYTES: usize = 12_000_000;
pub const KEYCHAIN_SERVICE: &str = "viggo-raycast-utilities";
pub const KEYCHAIN_ACCOUNT: &str = "password-history";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub value: String,
    pub created: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_text: Option<String>,
    pub text_history: Vec<Entry>,
    pub username_history: Vec<Entry>,
}

/// Storage methods must preserve invalid input on failure. `save_profile` must
/// use the shared private atomic writer and report uncertain publication to its
/// caller; callers must not blindly retry an uncertain write.
pub trait Storage {
    fn read_profile(&mut self) -> Result<Option<Vec<u8>>>;
    fn save_profile(&mut self, bytes: &[u8]) -> Result<()>;
    fn read_passwords(&mut self) -> Result<Option<Vec<u8>>>;
    fn save_passwords(&mut self, bytes: &[u8]) -> Result<()>;
    fn random(&mut self, count: usize) -> Result<Vec<u8>>;
    fn created(&mut self) -> String;
    fn title_matches(&self, title: &str, query: &str) -> bool;
    fn take_publication_warning(&mut self) -> Option<String> {
        None
    }
}

#[derive(PartialEq, Eq)]
pub enum Effect {
    Saved(&'static str),
    Paste(String),
    Copy { text: String, secret: bool },
}
// Deliberately no Debug implementation: an execution error/log must never
// accidentally print the contents of Copy or Paste.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Password,
    Username,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Password => "password",
            Self::Username => "username",
        }
    }
    fn limit(self) -> usize {
        if self == Self::Password {
            50
        } else {
            100
        }
    }
    fn mode(mode: &str) -> Option<Self> {
        match mode.strip_prefix("utility-").unwrap_or(mode) {
            "text-history" => Some(Self::Text),
            "password-history" => Some(Self::Password),
            "username-history" => Some(Self::Username),
            _ => None,
        }
    }
}

pub struct Utilities<S> {
    storage: S,
    selected: HashMap<String, (Kind, Entry)>,
}
impl<S: Storage> Utilities<S> {
    pub(crate) fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }
    pub fn new(storage: S) -> Self {
        Self {
            storage,
            selected: HashMap::new(),
        }
    }
    pub fn take_publication_warning(&mut self) -> Option<String> {
        self.storage.take_publication_warning()
    }
    fn profile(&mut self) -> Result<Profile> {
        let Some(bytes) = self.storage.read_profile()? else {
            return Ok(Profile::default());
        };
        if bytes.len() > MAX_BYTES {
            return Err("Utilities history is too large to load.".into());
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            "Utilities history could not be read. Keep the file and restore a valid copy.".into()
        })
    }
    fn save(&mut self, profile: &Profile) -> Result<()> {
        let bytes =
            serde_json::to_vec(profile).map_err(|_| "Could not encode utilities history.")?;
        self.storage.save_profile(&bytes)
    }
    fn passwords(&mut self) -> Result<Vec<Entry>> {
        let Some(bytes) = self.storage.read_passwords()? else {
            return Ok(vec![]);
        };
        if bytes.len() > MAX_BYTES {
            return Err("Password history is too large to load.".into());
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            "Password history has an invalid format. The Keychain entry was preserved.".into()
        })
    }
    pub fn commands(&mut self, mode: &str, query: &str) -> Result<Vec<Command>> {
        let Some(kind) = Kind::mode(mode) else {
            return Ok(vec![]);
        };
        let entries = match kind {
            Kind::Text => self.profile()?.text_history,
            Kind::Username => self.profile()?.username_history,
            Kind::Password => self.passwords()?,
        };
        self.selected.retain(|_, (old, _)| *old != kind);
        let mut rows = vec![];
        for (index, entry) in entries.into_iter().take(kind.limit()).enumerate() {
            let title = if kind == Kind::Password {
                format!("Password · {}", entry.created)
            } else {
                preview(&entry.value)
            };
            if !query.is_empty() && !self.storage.title_matches(&title, query) {
                continue;
            }
            let token = format!(
                "{:x}",
                Sha256::digest(format!("{}:{index}:{}", kind.label(), entry.created))
            );
            let command = serde_json::from_value(json!({"id":format!("utility.history.{token}"),"title":title,"kind":"builtin","target":format!("utility.history:{token}")}))
                .map_err(|_| "Could not prepare history command.")?;
            self.selected.insert(token, (kind, entry));
            rows.push(command);
        }
        Ok(rows)
    }
    pub fn import_profile(&mut self, value: Value) -> Result<Value> {
        let fields = value.as_object().ok_or("Invalid utilities import.")?;
        if fields.keys().any(|key| {
            !matches!(
                key.as_str(),
                "savedText" | "textHistory" | "usernameHistory"
            )
        }) {
            return Err("Unsupported utilities import field.".into());
        }
        let incoming: Profile =
            serde_json::from_value(value).map_err(|_| "Invalid utilities import.")?;
        if incoming
            .saved_text
            .as_ref()
            .is_some_and(|v| v.len() > 100_000)
            || incoming.text_history.len() > 100
            || incoming.username_history.len() > 100
            || incoming
                .text_history
                .iter()
                .chain(&incoming.username_history)
                .any(|e| e.value.len() > 100_000 || !valid_created(&e.created))
        {
            return Err("Invalid utilities import.".into());
        }
        let mut profile = self.profile()?;
        if profile.saved_text.is_none() {
            profile.saved_text = incoming.saved_text;
        }
        let mut added = 0;
        for (existing, incoming) in [
            (&mut profile.text_history, incoming.text_history),
            (&mut profile.username_history, incoming.username_history),
        ] {
            for entry in incoming {
                if !existing.iter().any(|old| old.value == entry.value) {
                    existing.push(entry);
                    added += 1;
                }
            }
            existing.truncate(100);
        }
        self.save(&profile)?;
        Ok(
            json!({"added":added,"textHistory":profile.text_history.len(),"usernameHistory":profile.username_history.len()}),
        )
    }
    pub fn execute(&mut self, target: &str, input: Option<&str>) -> Result<Effect> {
        match target {
            "utility-set-text" => {
                let input = input.ok_or("Enter the text to save.")?;
                if input.len() > 100_000 {
                    return Err("Saved text must be smaller than 100 KB.".into());
                }
                let mut profile = self.profile()?;
                profile.saved_text = Some(input.into());
                profile.text_history.retain(|entry| entry.value != input);
                profile.text_history.insert(
                    0,
                    Entry {
                        value: input.into(),
                        created: self.storage.created(),
                    },
                );
                profile.text_history.truncate(100);
                self.save(&profile)?;
                Ok(Effect::Saved("Text saved"))
            }
            "utility-paste-text" => {
                let text = self
                    .profile()?
                    .saved_text
                    .filter(|s| !s.is_empty())
                    .ok_or("Save text with Set Text first.")?;
                Ok(Effect::Paste(text))
            }
            "utility-password" => {
                let bytes = self.storage.random(24)?;
                if bytes.len() != 24 {
                    return Err("Could not generate a password.".into());
                }
                let text = password(&bytes);
                let entry = Entry {
                    value: text.clone(),
                    created: self.storage.created(),
                };
                let mut entries = self.passwords()?;
                entries.insert(0, entry);
                entries.truncate(50);
                let data = serde_json::to_vec(&entries)
                    .map_err(|_| "Could not encode password history.")?;
                self.storage.save_passwords(&data)?;
                Ok(Effect::Copy { text, secret: true })
            }
            "utility-username" => {
                let bytes = self.storage.random(4)?;
                if bytes.len() != 4 {
                    return Err("Could not generate a username.".into());
                }
                let words = [
                    "amber", "atlas", "cedar", "luna", "moss", "north", "raven", "river", "solar",
                    "violet",
                ];
                let text = format!(
                    "{}-{}-{:02x}{:02x}",
                    words[usize::from(bytes[0]) % words.len()],
                    words[usize::from(bytes[1]) % words.len()],
                    bytes[2],
                    bytes[3]
                );
                let mut profile = self.profile()?;
                profile.username_history.insert(
                    0,
                    Entry {
                        value: text.clone(),
                        created: self.storage.created(),
                    },
                );
                profile.username_history.truncate(100);
                self.save(&profile)?;
                Ok(Effect::Copy {
                    text,
                    secret: false,
                })
            }
            _ => {
                let token = target
                    .strip_prefix("utility.history:")
                    .ok_or("This history item expired. Search again.")?;
                let (kind, entry) = self
                    .selected
                    .get(token)
                    .cloned()
                    .ok_or("This history item expired. Search again.")?;
                if kind == Kind::Text {
                    let mut profile = self.profile()?;
                    profile.saved_text = Some(entry.value);
                    self.save(&profile)?;
                    Ok(Effect::Saved("Active text changed"))
                } else {
                    Ok(Effect::Copy {
                        text: entry.value,
                        secret: kind == Kind::Password,
                    })
                }
            }
        }
    }
}

pub fn preview(text: &str) -> String {
    // Swift splits Characters, not scalars: a whitespace-led composed
    // character (for example SPACE + combining acute) is removed as a whole.
    let mut compact = String::with_capacity(text.len().min(100_000));
    let mut separator = false;
    for grapheme in text.graphemes(true) {
        if grapheme.chars().next().is_some_and(char::is_whitespace) {
            separator = !compact.is_empty();
        } else {
            if separator {
                compact.push(' ');
            }
            compact.push_str(grapheme);
            separator = false;
        }
    }
    if compact.is_empty() {
        return "(empty)".into();
    }
    let mut graphemes = compact.graphemes(true);
    let mut value = graphemes.by_ref().take(100).collect::<String>();
    if graphemes.next().is_some() {
        value.push('…');
    }
    value
}
fn valid_created(value: &str) -> bool {
    // Use the same native formatter/default options as the frozen Swift source,
    // rather than approximating its accepted date grammar with RFC3339.
    objc2::rc::autoreleasepool(|_| {
        objc2_foundation::NSISO8601DateFormatter::new()
            .dateFromString(&objc2_foundation::NSString::from_str(value))
            .is_some()
    })
}
fn password(bytes: &[u8]) -> String {
    // Exactly24 secure bytes encode to32 URL-safe characters, without padding.
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(32);
    for chunk in bytes.chunks_exact(3) {
        let value = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        for shift in [18, 12, 6, 0] {
            out.push(char::from(ALPHABET[((value >> shift) & 63) as usize]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Memory {
        profile: Option<Vec<u8>>,
        passwords: Option<Vec<u8>>,
        fail_write: bool,
        fail_random: bool,
    }
    impl Storage for Memory {
        fn read_profile(&mut self) -> Result<Option<Vec<u8>>> {
            Ok(self.profile.clone())
        }
        fn save_profile(&mut self, bytes: &[u8]) -> Result<()> {
            if self.fail_write {
                return Err("owned publication failed".into());
            }
            self.profile = Some(bytes.into());
            Ok(())
        }
        fn read_passwords(&mut self) -> Result<Option<Vec<u8>>> {
            Ok(self.passwords.clone())
        }
        fn save_passwords(&mut self, bytes: &[u8]) -> Result<()> {
            if self.fail_write {
                return Err("owned Keychain write failed".into());
            }
            self.passwords = Some(bytes.into());
            Ok(())
        }
        fn random(&mut self, count: usize) -> Result<Vec<u8>> {
            if self.fail_random {
                Err("owned secure random failed".into())
            } else {
                Ok((0..count).map(|n| n as u8).collect())
            }
        }
        fn created(&mut self) -> String {
            "2026-11-01T12:34:56Z".into()
        }
        fn title_matches(&self, title: &str, query: &str) -> bool {
            title.to_lowercase().contains(&query.to_lowercase())
        }
    }
    #[test]
    fn legacy_profile_import_preserves_active_text_order_and_invalid_bytes() {
        let mut utilities = Utilities::new(Memory::default());
        utilities
            .execute("utility-set-text", Some("current"))
            .unwrap();
        let incoming = json!({"savedText":"incoming","textHistory":[{"value":"current","created":"2026-10-31T00:00:00Z"},{"value":"earlier","created":"2026-10-30T00:00:00Z"}],"usernameHistory":[]});
        assert_eq!(utilities.import_profile(incoming).unwrap()["added"], 1);
        let profile = utilities.profile().unwrap();
        assert_eq!(profile.saved_text.as_deref(), Some("current"));
        assert_eq!(
            profile
                .text_history
                .iter()
                .map(|e| e.value.as_str())
                .collect::<Vec<_>>(),
            ["current", "earlier"]
        );
        let before = utilities.storage.profile.clone();
        assert!(utilities.import_profile(json!({"textHistory":[]})).is_err());
        assert_eq!(utilities.storage.profile, before);
        assert!(utilities
            .import_profile(json!({"passwordHistory":[]}))
            .is_err());
        assert_eq!(utilities.storage.profile, before);
        utilities.storage.profile = Some(b"{broken legacy data".to_vec());
        assert!(utilities
            .execute("utility-set-text", Some("replacement"))
            .is_err());
        assert_eq!(
            utilities.storage.profile.as_deref(),
            Some(b"{broken legacy data".as_slice())
        );
    }
    #[test]
    fn password_is_persisted_before_concealed_effect_and_never_in_metadata() {
        let mut utilities = Utilities::new(Memory::default());
        let Effect::Copy { text, secret } = utilities.execute("utility-password", None).unwrap()
        else {
            panic!("copy effect missing")
        };
        assert!(secret);
        assert_eq!(text, "AAECAwQFBgcICQoLDA0ODxAREhMUFRYX");
        assert!(utilities.storage.profile.is_none());
        let rows = utilities.commands("password-history", "").unwrap();
        let metadata = serde_json::to_string(&rows).unwrap();
        assert!(!metadata.contains(&text));
        assert_eq!(rows[0].title, "Password · 2026-11-01T12:34:56Z");
        assert!(matches!(
            utilities.execute(&rows[0].target, None).unwrap(),
            Effect::Copy { secret: true, .. }
        ));
        utilities.storage.fail_write = true;
        let before = utilities.storage.passwords.clone();
        assert!(utilities.execute("utility-password", None).is_err());
        assert_eq!(utilities.storage.passwords, before);
        utilities.storage.fail_random = true;
        assert!(utilities.execute("utility-password", None).is_err());
        assert_eq!(utilities.storage.passwords, before);
    }
    #[test]
    fn malformed_keychain_is_preserved_and_emits_no_copy() {
        let original = b"invalid owned entry".to_vec();
        let mut utilities = Utilities::new(Memory {
            passwords: Some(original.clone()),
            ..Memory::default()
        });
        assert!(utilities.execute("utility-password", None).is_err());
        assert_eq!(utilities.storage.passwords, Some(original));
    }
    #[test]
    fn history_selection_expires_on_same_kind_refresh_and_keeps_other_kind() {
        let mut utilities = Utilities::new(Memory::default());
        utilities
            .execute("utility-set-text", Some("first"))
            .unwrap();
        let target = utilities.commands("text-history", "").unwrap()[0]
            .target
            .clone();
        utilities.execute("utility-username", None).unwrap();
        utilities.commands("username-history", "").unwrap();
        assert!(utilities.execute(&target, None).is_ok());
        utilities.commands("text-history", "missing").unwrap();
        assert!(utilities.execute(&target, None).is_err());
    }
    #[test]
    fn preview_counts_graphemes_and_collapses_whitespace() {
        let text = "👩‍👩‍👧‍👦".repeat(101);
        assert_eq!(preview(&text), format!("{}…", "👩‍👩‍👧‍👦".repeat(100)));
        assert_eq!(preview(" \n\t "), "(empty)");
        assert_eq!(preview(" a\n b\t c "), "a b c");
        assert_eq!(preview("a \u{301}b"), "a b");
    }
}
