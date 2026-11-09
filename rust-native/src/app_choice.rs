//! Worker-prepared display metadata; bundle identity never comes from a title.
use rustcast_core::Command;
use std::{collections::HashMap, sync::Arc};
#[derive(Debug)]
pub struct Entry {
    pub command_id: String,
    pub id: String,
    pub label: String,
}
#[derive(Clone, Debug)]
pub struct Choices(pub Arc<Vec<Entry>>, Arc<HashMap<String, usize>>);
impl Default for Choices {
    fn default() -> Self {
        static EMPTY: std::sync::OnceLock<Choices> = std::sync::OnceLock::new();
        EMPTY
            .get_or_init(|| Self(Arc::new(vec![]), Arc::new(HashMap::new())))
            .clone()
    }
}
impl PartialEq for Choices {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
fn canonical(s: &str) -> String {
    objc2::rc::autoreleasepool(|_| {
        objc2_foundation::NSString::from_str(s)
            .precomposedStringWithCanonicalMapping()
            .to_string()
    })
}
impl Choices {
    pub fn new(apps: &[Command]) -> Self {
        let apps: Vec<_> = apps
            .iter()
            .filter(|a| a.kind == "app" && !a.target.trim().is_empty())
            .collect();
        let mut counts = HashMap::<String, usize>::new();
        counts.insert("Default app".into(), 1); // Reserved default-opening option.
        let keys: Vec<_> = apps.iter().map(|a| canonical(&a.title)).collect();
        for key in &keys {
            *counts.entry(key.clone()).or_default() += 1;
        }
        let entries: Vec<_> = apps
            .iter()
            .zip(keys)
            .map(|(a, key)| Entry {
                command_id: a.id.clone(),
                id: a.target.clone(),
                label: if counts[&key] > 1 {
                    format!("{} — {}", a.title, a.target)
                } else {
                    a.title.clone()
                },
            })
            .collect();
        let index = entries
            .iter()
            .enumerate()
            .map(|(i, a)| (canonical(&a.id), i))
            .collect();
        Self(Arc::new(entries), Arc::new(index))
    }
    pub fn index(&self, id: &str) -> Option<usize> {
        self.1.get(&canonical(id)).copied()
    }
    pub fn contains_origin(&self, command: &Command) -> bool {
        command.kind == "app"
            && self
                .index(&command.target)
                .and_then(|i| self.0.get(i))
                .is_some_and(|a| canonical(&a.command_id) == canonical(&command.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_canonical_and_identity_is_bundle_plus_origin() {
        let a:Command=serde_json::from_value(serde_json::json!({"id":"a","title":"Café","kind":"app","target":"dev.café","aliases":[]})).unwrap();
        let mut b = a.clone();
        b.id = "b".into();
        b.title = "Cafe\u{301}".into();
        b.target = "dev.other".into();
        let choices = Choices::new(&[a.clone(), b]);
        assert_eq!(choices.index("dev.cafe\u{301}"), Some(0));
        assert!(choices.0[0].label.ends_with("— dev.café"));
        assert!(choices.contains_origin(&a));
        let mut spoof = a;
        spoof.id = "b".into();
        assert!(!choices.contains_origin(&spoof));
        assert_eq!(Choices::default(), Choices::default());
        let mut invalid = spoof;
        invalid.target = " ".into();
        assert!(Choices::new(&[invalid]).0.is_empty());
    }
}
