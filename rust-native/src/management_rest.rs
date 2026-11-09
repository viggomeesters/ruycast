//! Remaining original management defaults; Dashboard is owned by its slice.
use rustcast_core::Command;
pub fn commands(saved: &[Command]) -> Vec<Command> {
    [
    ("builtin.settings","Ruycast Settings","settings",vec!["settings","preferences","manage","beheer","instellingen"]),
    ("builtin.open-config","Open Configuration","open-config",vec!["open-config"]),
    ("builtin.reload-config","Reload Configuration","reload-config",vec!["reload-config"]),
    ("builtin.system-monitor","System Monitor","system-monitor",vec!["system-monitor"]),
].into_iter().filter(|(id,_,target,_)|!saved.iter().any(|c|c.id==*id||(c.kind=="builtin"&&c.target==*target))).map(|(id,title,target,aliases)|serde_json::from_value(serde_json::json!({"id":id,"title":title,"kind":"builtin","target":target,"aliases":aliases})).expect("static management default")).collect()
}
