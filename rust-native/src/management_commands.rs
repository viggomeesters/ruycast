//! Standard management commands. Saved IDs or builtin targets override defaults.
use rustcast_core::Command;
pub fn system(saved: &[Command]) -> Vec<Command> {
    let mut remaining = crate::management_rest::commands(saved);
    let mut commands = Vec::new();
    if let Some(index) = remaining.iter().position(|command| command.target == "settings") {
        commands.push(remaining.remove(index));
    }
    commands.extend([("builtin.dashboard", "Ruycast Dashboard", "analytics", vec!["analytics", "dashboard", "activity"])]
        .into_iter().filter(|(id,_,target,_)| !saved.iter().any(|c| c.id==*id || (c.kind=="builtin" && c.target==*target)))
        .map(|(id,title,target,aliases)| serde_json::from_value(serde_json::json!({"id":id,"title":title,"kind":"builtin","target":target,"aliases":aliases})).expect("static management command")));
    commands.extend(remaining);
    commands.extend(crate::system_search::commands(saved));
    commands
}

pub fn catalog(saved: &[Command]) -> Vec<Command> {
    saved.iter().cloned().chain(system(saved)).collect()
}
pub fn dashboard(command: &Command) -> bool {
    command.kind == "builtin" && command.target == "analytics"
}
/// Match AppState.perform's pre-attempt validation, including Foundation whitespace.
pub fn validate_input(command: &Command, values: &serde_json::Value) -> Result<(), String> {
    if values
        .as_object()
        .is_none_or(|m| m.values().any(|v| !v.is_string()))
    {
        return Err("Input values must be strings.".into());
    }
    if let Some(prompt) = &command.input_prompt {
        let input = values["input"].as_str().unwrap_or("");
        let empty = objc2::rc::autoreleasepool(|_| {
            objc2_foundation::NSString::from_str(input)
                .stringByTrimmingCharactersInSet(
                    &objc2_foundation::NSCharacterSet::whitespaceAndNewlineCharacterSet(),
                )
                .is_empty()
        });
        if empty {
            return Err(format!("Enter {prompt}."));
        }
    }
    crate::execution::budget(command, values["input"].as_str())
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    #[test]
    fn original_nine_defaults_keep_source_order_and_saved_overrides() {
        let defaults = system(&[]);
        assert_eq!(defaults.iter().map(|c|c.id.as_str()).collect::<Vec<_>>(), [
            "builtin.settings", "builtin.dashboard", "builtin.open-config", "builtin.reload-config", "builtin.system-monitor",
            "builtin.window-search", "builtin.menu-actions", "builtin.safari-tabs", "builtin.apple-shortcuts",
        ]);
        assert!(!defaults.iter().any(|c|matches!(c.target.as_str(), "lock-screen"|"search-files"|"processes")));
        let saved: Vec<Command> = serde_json::from_value(serde_json::json!([
            {"id":"my-dashboard","title":"Mine","kind":"builtin","target":"analytics","enabled":false},
            {"id":"builtin.settings","title":"Overridden ID","kind":"url","target":"https://example.test"},
            {"id":"custom","title":"First","kind":"builtin","target":"open-config","aliases":["custom-config"]}
        ])).unwrap();
        let catalog = catalog(&saved);
        assert_eq!(serde_json::to_value(&catalog[..saved.len()]).unwrap(), serde_json::to_value(&saved).unwrap());
        assert_eq!(catalog.len(), 9);
        assert!(!catalog.iter().any(|c|c.id=="builtin.dashboard"||c.id=="builtin.open-config"));
        for command in &catalog {
            assert_eq!(catalog.iter().filter(|c|c.id==command.id).count(),1);
        }
    }
}
