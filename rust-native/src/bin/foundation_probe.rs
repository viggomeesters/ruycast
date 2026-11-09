//! Explicit-feature disposable process fixture. Never used by the installed app.
use ruycast_native::{
    profile::{ConfigStore, ProfilePaths},
    state::HostState,
};
use serde_json::json;
use std::path::PathBuf;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(args.len(), 2, "Expected mode and disposable fixture base");
    let base = PathBuf::from(&args[1]);
    assert!(base.is_absolute());
    let paths = ProfilePaths::new(base.join("profile"), base.clone(), None);
    let mut store = ConfigStore::open(paths.clone()).expect("prepared disposable config");
    let original = store.snapshot().to_vec();
    let source = store.file.clone();
    let result = match args[0].as_str() {
        "save" => {
            let mut value = rustcast_core::default_config();
            value["clipboard"]["limit"] = 51.into();
            let receipt = store.save(&value);
            json!({"ok":receipt.is_ok(),"durable":receipt.as_ref().map(|r|r.durable).ok(),
                "warning":receipt.as_ref().ok().and_then(|r|r.warning.as_ref()).is_some(),
                "activeLimit":store.current().clipboard.limit,"revision":store.revision,
                "fileEqualsSnapshot":std::fs::read(&store.file).unwrap()==store.snapshot()})
        }
        "inline-followup-warning" | "inline-cancel-warning" => {
            use ruycast_native::actions::{Editing, InlineSubmission, Mode};
            let a = store.current().commands[0].clone();
            let b = store.current().commands[1].clone();
            let mut edit = Editing::default();
            edit.transition_inline(&mut store, &a, Mode::Alias, None, || true)
                .unwrap();
            let token = edit.token().unwrap();
            if args[0] == "inline-followup-warning" {
                let mut value: serde_json::Value =
                    serde_json::from_slice(store.snapshot()).unwrap();
                value["commands"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|c| c["id"] != b.id);
                std::fs::write(&store.file, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            let outcome = edit
                .transition_inline(
                    &mut store,
                    &b,
                    Mode::Alias,
                    Some(InlineSubmission {
                        token,
                        attempt: 1,
                        aliases: Some("published-inline".into()),
                        hotkey: None,
                    }),
                    || args[0] != "inline-cancel-warning",
                )
                .unwrap();
            json!({"ok":true,"warning":outcome.warning.is_some(),"error":outcome.error,"opened":outcome.opened,"draftRetained":edit.token().is_some(),"aliases":store.current().commands[0].aliases,"fileEqualsSnapshot":std::fs::read(&store.file).unwrap()==store.snapshot()})
        }
        "new-command" => {
            let count = store.current().commands.len();
            let mut edit = ruycast_native::actions::Editing::default();
            edit.create(&store, "script", None).unwrap();
            let token = edit.token().unwrap();
            let receipt = edit.save(&mut store, token, None, None);
            json!({"ok":receipt.is_ok(),"durable":receipt.as_ref().map(|r|r.durable).ok(),"warning":receipt.as_ref().ok().and_then(|r|r.warning.as_ref()).is_some(),"countAdded":store.current().commands.len()-count,"draftRetained":edit.view().is_some(),"fileEqualsSnapshot":std::fs::read(&store.file).unwrap()==store.snapshot()})
        }
        "clipboard-prefill" => objc2::rc::autoreleasepool(|_| {
            use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
            use objc2_foundation::NSString;
            let board = NSPasteboard::pasteboardWithUniqueName();
            let kind = unsafe { NSPasteboardTypeString };
            board.clearContents();
            assert!(board
                .setString_forType(&NSString::from_str("  https://example.com/path  \n"), kind));
            let prefilled = ruycast_native::creation::prefill(&board);
            board.clearContents();
            assert!(board.setString_forType(&NSString::from_str(" \n "), kind));
            let blank = ruycast_native::creation::prefill(&board).is_none();
            board.clearContents();
            assert!(board.setString_forType(&NSString::from_str(&"x".repeat(1_000_001)), kind));
            let complete = ruycast_native::creation::prefill(&board) == Some("x".repeat(1_000_001));
            board.clearContents();
            json!({"ok":true,"prefill":prefilled,"emptyIgnored":blank,"largeTextRetained":complete,"generalPasteboardRead":false})
        }),
        "binding" | "disable" => {
            let command = store.current().commands[0].clone();
            let mut edit = ruycast_native::actions::Editing::default();
            let receipt = if args[0] == "binding" {
                edit.open(&store, &command, ruycast_native::actions::Mode::Alias);
                let token = edit.token().unwrap();
                edit.save(&mut store, token, Some("changed-alias".into()), None)
            } else {
                ruycast_native::actions::disable(&mut store, &command)
            };
            json!({"ok":receipt.is_ok(),"durable":receipt.as_ref().map(|r|r.durable).ok(),"warning":receipt.as_ref().ok().and_then(|r|r.warning.as_ref()).is_some(),"aliases":store.current().commands[0].aliases,"enabled":store.current().commands[0].enabled,"draftRetained":edit.view().is_some(),"fileEqualsSnapshot":std::fs::read(&store.file).unwrap()==store.snapshot()})
        }
        "relocate" => {
            let receipt = store.relocate(base.join("portable").to_str().unwrap());
            json!({"ok":receipt.is_ok(),"durable":receipt.as_ref().map(|r|r.durable).ok(),
                "warning":receipt.as_ref().ok().and_then(|r|r.warning.as_ref()).is_some(),
                "sourcePreserved":std::fs::read(&source).unwrap()==original,
                "selectorExists":paths.root.join("config-location.json").exists(),
                "targetExists":base.join("portable/config.json").exists(),
                "activeTarget":store.file==base.join("portable/config.json")})
        }
        "ranking" => {
            let mut value = rustcast_core::default_config();
            value["commands"] = serde_json::to_value(ruycast_native::session::fixture(12)).unwrap();
            store.save(&value).unwrap();
            let mut host = HostState::open(paths).unwrap();
            host.session.set_query("microsoft");
            let selected = host.session.selected().unwrap().id.clone();
            host.session.actions_open = true;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();
            host.record_success("fixture.8", now).unwrap();
            json!({"ok":true,"immediateRankRefresh":host.session.rows().next().unwrap().id=="fixture.8",
                "selectedIdentityPreserved":host.session.selected().unwrap().id==selected,
                "actionsPreserved":host.session.actions_open})
        }
        "state" => {
            let mut host = HostState::open(paths).unwrap();
            host.session.set_query("finder");
            let id = host.session.selected().unwrap().id.clone();
            std::fs::write(&source, b"invalid").unwrap();
            let rejected = host.reload().is_err();
            let retained = host.session.selected().unwrap().id == id;
            let mut value = rustcast_core::default_config();
            value["commands"][0]["enabled"] = false.into();
            std::fs::write(&source, serde_json::to_vec(&value).unwrap()).unwrap();
            let changed = host.reload().unwrap();
            json!({"ok":rejected&&retained&&changed,"lastGoodCatalogRetained":retained,
                "selectedRemovedWithoutReplacement":host.session.selected().is_none(),"revision":host.configuration.revision})
        }
        _ => panic!("unknown fixture"),
    };
    println!("{result}");
}
