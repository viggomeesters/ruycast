use super::*;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
fn ticket() -> Ticket {
    Ticket {
        deadline: Instant::now() + Duration::from_secs(2),
        live: Arc::new(|| true),
    }
}
fn setup() -> (Arc<fixture::Fixture>, providers::Provider) {
    let f = Arc::new(fixture::Fixture::default());
    let p = providers::Provider::new(f.clone(), false);
    (f, p)
}
#[test]
fn window_identity_duplicate_titles_and_minimized_restore_are_exact() {
    let (f, p) = setup();
    f.edit(Element(3), |n| {
        n.strings.insert("AXTitle".into(), "Owned Project".into());
    });
    let rows = p.search(Scope::Windows, "", None, &ticket()).unwrap();
    assert_eq!(rows.commands.len(), 2);
    assert_ne!(rows.commands[0].id, rows.commands[1].id);
    p.execute(&rows.commands[1].id, None, &ticket(), &f.activate())
        .unwrap();
    assert_eq!(
        *f.effects.lock().unwrap(),
        ["activate", "restore", "AXRaise"]
    );
    f.effects.lock().unwrap().clear();
    f.edit(Element(1), |n| {
        n.lists.insert("AXWindows".into(), vec![Element(3)]);
    });
    assert!(p
        .execute(&rows.commands[0].id, None, &ticket(), &f.activate())
        .is_err());
    assert!(f.effects.lock().unwrap().is_empty());
}
#[test]
fn window_post_activation_pid_role_title_membership_and_restore_state_are_rechecked() {
    for change in ["pid", "role", "title", "membership", "process"] {
        let (f, p) = setup();
        let rows = p
            .search(Scope::Windows, "Owned Draft", None, &ticket())
            .unwrap();
        let fixture = f.clone();
        let activate: Activate = Arc::new(move |_, _| {
            match change {
                "pid" => fixture.edit(Element(3), |n| n.pid += 1),
                "role" => fixture.edit(Element(3), |n| {
                    n.strings.insert("AXRole".into(), "AXButton".into());
                }),
                "title" => fixture.edit(Element(3), |n| {
                    n.strings.insert("AXTitle".into(), "Replacement".into());
                }),
                "membership" => fixture.edit(Element(1), |n| {
                    n.lists.insert("AXWindows".into(), vec![Element(2)]);
                }),
                _ => fixture.apps.lock().unwrap()[0].identity.seconds += 1,
            }
            Ok(())
        });
        assert!(
            p.execute(&rows.commands[0].id, None, &ticket(), &activate)
                .is_err(),
            "{change}"
        );
        assert!(f.effects.lock().unwrap().is_empty(), "{change}");
    }
}
#[test]
fn menus_disabled_unknown_original_app_and_shortcut_rebuild_fail_closed() {
    let (f, p) = setup();
    let previous = f.previous().unwrap();
    let rows = p
        .search(Scope::Menus, "", Some(previous.clone()), &ticket())
        .unwrap();
    assert_eq!(rows.commands.len(), 2);
    assert_eq!(rows.commands[1].enabled, Some(false));
    assert!(p
        .execute(
            &rows.commands[1].id,
            Some(previous.identity),
            &ticket(),
            &f.activate()
        )
        .is_err());
    assert!(p
        .execute(&rows.commands[0].id, None, &ticket(), &f.activate())
        .is_err());
    assert!(f.effects.lock().unwrap().is_empty());
    let fixture = f.clone();
    let activate: Activate = Arc::new(move |_, _| {
        fixture.edit(Element(7), |n| {
            n.ints.insert("AXMenuItemCmdModifiers".into(), 8);
        });
        Ok(())
    });
    assert!(p
        .execute(
            &rows.commands[0].id,
            Some(previous.identity),
            &ticket(),
            &activate
        )
        .is_err());
    assert!(f.effects.lock().unwrap().is_empty());
    f.edit(Element(7), |n| {
        n.ints.insert("AXMenuItemCmdModifiers".into(), 0);
    });
    p.execute(
        &rows.commands[0].id,
        Some(previous.identity),
        &ticket(),
        &f.activate(),
    )
    .unwrap();
    assert_eq!(*f.effects.lock().unwrap(), ["activate", "AXPress"]);
}
#[test]
fn slow_safari_discovery_does_not_extend_snapshot_cache_lifetime() {
    for scope in [Scope::Tabs, Scope::MainTabs] {
        let fixture = Arc::new(fixture::Fixture::default());
        let provider = providers::Provider::new(fixture.clone(), scope == Scope::MainTabs);
        // The shipped snapshot clock starts after browser lookup, before the
        // two remaining reads. Those reads outlive the one-second cache lease.
        fixture.delay_ms.store(550, Ordering::Release);
        let slow = Ticket {
            deadline: Instant::now() + Duration::from_secs(5),
            live: Arc::new(|| true),
        };
        let before = provider.search(scope, "", None, &slow).unwrap();
        assert_eq!(before.commands[0].title, "Owned Tab");
        fixture.delay_ms.store(0, Ordering::Release);
        fixture.tabs.lock().unwrap()[0].title = "Changed owned tab".into();
        let after = provider.search(scope, "", None, &ticket()).unwrap();
        assert_eq!(after.commands[0].title, "Changed owned tab");
        assert_ne!(before.commands[0].id, after.commands[0].id);
        assert!(fixture.effects.lock().unwrap().is_empty());
    }
}
#[test]
fn safari_position_snapshot_never_falls_back_to_matching_title_or_url() {
    let (f, p) = setup();
    let rows = p
        .search(Scope::Tabs, "owned invalid", None, &ticket())
        .unwrap();
    assert_eq!(rows.commands.len(), 1);
    f.tabs.lock().unwrap()[0].index = 2;
    assert!(p
        .execute(&rows.commands[0].id, None, &ticket(), &f.activate())
        .is_err());
    assert!(f.effects.lock().unwrap().is_empty());
    f.tabs.lock().unwrap()[0].index = 1;
    let mut duplicate = f.tabs.lock().unwrap()[0].clone();
    duplicate.index = 2;
    f.tabs.lock().unwrap().push(duplicate);
    let error = p
        .execute(&rows.commands[0].id, None, &ticket(), &f.activate())
        .unwrap_err();
    assert!(error.contains("same title"));
    assert!(f.effects.lock().unwrap().is_empty());
    f.tabs.lock().unwrap().pop();
    p.execute(&rows.commands[0].id, None, &ticket(), &f.activate())
        .unwrap();
    assert_eq!(
        *f.effects.lock().unwrap(),
        ["select-tab", "front-window", "activate"]
    );
}
#[test]
fn safari_selection_mismatch_stops_before_foreground_effect() {
    let (f, p) = setup();
    let rows = p.search(Scope::Tabs, "", None, &ticket()).unwrap();
    let fixture = f.clone();
    let activate: Activate = Arc::new(move |_, _| {
        fixture.tabs.lock().unwrap()[0].title = "Changed".into();
        Ok(())
    });
    assert!(p
        .execute(&rows.commands[0].id, None, &ticket(), &activate)
        .is_err());
    assert_eq!(*f.effects.lock().unwrap(), ["select-tab", "front-window"]);
}
#[test]
fn safari_original_snapshot_mutations_fail_before_the_next_effect() {
    for mutation in [
        "close",
        "move",
        "title",
        "duplicate",
        "restart",
        "set",
        "raise",
        "activate",
        "timeout",
        "denied",
    ] {
        let (fixture, provider) = setup();
        let captured = provider.search(Scope::Tabs, "", None, &ticket()).unwrap();
        let original = fixture.tabs.lock().unwrap()[0].clone();
        let alter = |fixture: &fixture::Fixture| {
            fixture.tabs.lock().unwrap()[0].title = "Changed owned snapshot".into()
        };
        match mutation {
            "close" => fixture.tabs.lock().unwrap().clear(),
            "move" => fixture.tabs.lock().unwrap()[0].index += 1,
            "title" => alter(&fixture),
            "duplicate" => fixture.tabs.lock().unwrap().push(Tab {
                index: 2,
                ..original
            }),
            "restart" => fixture.apps.lock().unwrap()[0].identity.seconds += 1,
            "denied" => fixture.safari_denied.store(true, Ordering::Release),
            "set" => *fixture.after_safari_select.lock().unwrap() = Some(Box::new(alter)),
            "raise" => *fixture.after_safari_front.lock().unwrap() = Some(Box::new(alter)),
            _ => (),
        }
        let base_activate = fixture.activate();
        let effect_fixture = fixture.clone();
        let activate: Activate = Arc::new(move |identity, ticket| {
            base_activate(identity, ticket)?;
            if mutation == "activate" {
                alter(&effect_fixture);
            }
            Ok(())
        });
        let execution = if mutation == "timeout" {
            Ticket {
                deadline: Instant::now(),
                live: Arc::new(|| true),
            }
        } else {
            ticket()
        };
        assert!(
            provider
                .execute(&captured.commands[0].id, None, &execution, &activate)
                .is_err(),
            "{mutation}"
        );
        let expected: &[&str] = match mutation {
            "set" => &["select-tab"],
            "raise" => &["select-tab", "front-window"],
            "activate" => &["select-tab", "front-window", "activate"],
            _ => &[],
        };
        assert_eq!(*fixture.effects.lock().unwrap(), expected, "{mutation}");
    }
}
#[test]
fn safari_missing_empty_and_original_collection_and_metadata_bounds() {
    let (fixture, provider) = setup();
    fixture.apps.lock().unwrap().clear();
    assert!(provider
        .search(Scope::Tabs, "", None, &ticket())
        .unwrap_err()
        .contains("Open Safari"));
    let (fixture, provider) = setup();
    fixture.tabs.lock().unwrap().clear();
    assert!(provider
        .search(Scope::Tabs, "", None, &ticket())
        .unwrap()
        .commands
        .is_empty());
    for boundary in ["windows", "title", "url", "aggregate"] {
        let (fixture, provider) = setup();
        let base = fixture.tabs.lock().unwrap()[0].clone();
        let mut tabs = fixture.tabs.lock().unwrap();
        match boundary {
            "windows" => {
                *tabs = (1..=65)
                    .map(|window| Tab {
                        window,
                        ..base.clone()
                    })
                    .collect()
            }
            "title" => tabs[0].title = "x".repeat(8193),
            "url" => tabs[0].url = "x".repeat(8193),
            _ => {
                *tabs = (1..=257)
                    .map(|index| Tab {
                        index,
                        title: "x".repeat(8192),
                        url: String::new(),
                        ..base.clone()
                    })
                    .collect()
            }
        }
        drop(tabs);
        let error = provider
            .search(Scope::Tabs, "", None, &ticket())
            .unwrap_err();
        assert!(
            error.contains(if boundary == "windows" {
                "64"
            } else if boundary == "aggregate" {
                "512"
            } else {
                "metadata"
            }),
            "{boundary}: {error}"
        );
        assert!(fixture.effects.lock().unwrap().is_empty());
    }
}
#[test]
fn safari_short_cache_reuses_tokens_and_case_insensitive_url_filter_without_reads() {
    for scope in [Scope::Tabs, Scope::MainTabs] {
        let fixture = Arc::new(fixture::Fixture::default());
        let provider = providers::Provider::new(fixture.clone(), scope == Scope::MainTabs);
        let first = provider.search(scope, "", None, &ticket()).unwrap();
        let reads = fixture.reads.load(Ordering::Acquire);
        let filtered = provider
            .search(scope, "EXAMPLE.INVALID", None, &ticket())
            .unwrap();
        assert_eq!(filtered.commands[0].id, first.commands[0].id);
        // Browser identity is intentionally re-read even on a cache hit.
        assert_eq!(fixture.reads.load(Ordering::Acquire), reads + 1);
        assert!(!first.commands[0].id.contains("example"));
        assert_eq!(first.commands[0].target, first.commands[0].id);
        assert!(fixture.effects.lock().unwrap().is_empty());
    }
}
#[test]
fn safari_cancelled_consent_never_reaches_the_owned_permission_effect() {
    let fixture = Arc::new(fixture::Fixture::default());
    let service = service::Service::new(fixture.clone());
    let cancelled = Ticket {
        deadline: Instant::now() + Duration::from_secs(1),
        live: Arc::new(|| false),
    };
    assert!(service.consent(cancelled, Arc::new(|| {})).is_err());
    assert_eq!(fixture.reads.load(Ordering::Acquire), 0);
    fixture.delay_ms.store(100, Ordering::Release);
    let live = Arc::new(AtomicBool::new(true));
    let check = live.clone();
    let pending = service
        .consent(
            Ticket {
                deadline: Instant::now() + Duration::from_secs(2),
                live: Arc::new(move || check.load(Ordering::Acquire)),
            },
            Arc::new(|| {}),
        )
        .unwrap();
    let until = Instant::now() + Duration::from_secs(1);
    while fixture.reads.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
    live.store(false, Ordering::Release);
    let result = loop {
        if let Some(result) = pending.poll() {
            break result;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(result.is_err());
    assert!(fixture.effects.lock().unwrap().is_empty());
}
#[test]
fn optional_main_tab_caps_preserve_metadata_and_dedicated_results() {
    for (scope, query, expected) in [
        (Scope::MainTabs, "", 5),
        (Scope::MainTabs, "Owned Tab", 50),
        (Scope::Tabs, "", 60),
    ] {
        let fixture = Arc::new(fixture::Fixture::default());
        let original = fixture.tabs.lock().unwrap()[0].clone();
        *fixture.tabs.lock().unwrap() = (1..=60)
            .map(|index| Tab {
                index,
                ..original.clone()
            })
            .collect();
        let service = service::Service::new(fixture.clone());
        let pending = service
            .search(scope, query.into(), None, ticket(), Arc::new(|| {}))
            .unwrap();
        let until = Instant::now() + Duration::from_secs(1);
        let result = loop {
            if let Some(result) = pending.poll() {
                break result.unwrap();
            }
            assert!(Instant::now() < until);
            std::thread::yield_now();
        };
        assert_eq!(result.commands.len(), expected);
        assert_eq!(result.metadata.len(), expected);
        assert!(result
            .commands
            .iter()
            .all(|c| result.metadata.contains_key(&c.id)));
        assert!(fixture.effects.lock().unwrap().is_empty());
    }
}
#[test]
fn root_and_dedicated_tokens_cannot_cross_stores_or_retarget_old_results() {
    let (f, p) = setup();
    let root = providers::Provider::new(f.clone(), true);
    let old = p.search(Scope::Tabs, "", None, &ticket()).unwrap().commands[0]
        .id
        .clone();
    p.refresh();
    let new = p.search(Scope::Tabs, "", None, &ticket()).unwrap().commands[0]
        .id
        .clone();
    let root_id = root
        .search(Scope::MainTabs, "", None, &ticket())
        .unwrap()
        .commands[0]
        .id
        .clone();
    assert_ne!(old, new);
    assert!(root_id.starts_with("system.tab:root."));
    assert!(!root.recognizes(&old));
    assert!(!p.recognizes(&root_id));
}
#[test]
fn absolute_deadline_and_cancellation_stop_traversal_and_every_effect() {
    let (f, p) = setup();
    let rows = p.search(Scope::Windows, "", None, &ticket()).unwrap();
    let before = f.reads.load(Ordering::Acquire);
    let t = Ticket {
        deadline: Instant::now(),
        live: Arc::new(|| true),
    };
    assert!(p.search(Scope::Windows, "", None, &t).is_err());
    assert!(p
        .execute(&rows.commands[0].id, None, &t, &f.activate())
        .is_err());
    assert_eq!(before, f.reads.load(Ordering::Acquire));
    assert!(f.effects.lock().unwrap().is_empty());
    let cancelled = Arc::new(AtomicBool::new(false));
    let check = cancelled.clone();
    let t = Ticket {
        deadline: Instant::now() + Duration::from_secs(1),
        live: Arc::new(move || !check.load(Ordering::Acquire)),
    };
    let cancel = cancelled.clone();
    let activate: Activate = Arc::new(move |_, _| {
        cancel.store(true, Ordering::Release);
        Ok(())
    });
    assert!(p
        .execute(&rows.commands[0].id, None, &t, &activate)
        .is_err());
    assert!(f.effects.lock().unwrap().is_empty());
    f.delay_ms.store(5, Ordering::Release);
    let t = Ticket {
        deadline: Instant::now() + Duration::from_millis(15),
        live: Arc::new(|| true),
    };
    let before = f.reads.load(Ordering::Acquire);
    assert!(p.search(Scope::Windows, "", None, &t).is_err());
    assert!(f.reads.load(Ordering::Acquire) - before <= 3);
}
#[test]
fn result_caps_and_ax_permission_recovery_are_bounded() {
    let (f, p) = setup();
    f.trusted.store(false, Ordering::Release);
    assert!(p
        .search(Scope::Windows, "", None, &ticket())
        .unwrap_err()
        .contains("Accessibility"));
    f.trusted.store(true, Ordering::Release);
    assert_eq!(
        p.search(Scope::Windows, "", None, &ticket())
            .unwrap()
            .commands
            .len(),
        2
    );
    let mut tabs = f.tabs.lock().unwrap();
    let base = tabs[0].clone();
    *tabs = (1..=513)
        .map(|index| Tab {
            index,
            ..base.clone()
        })
        .collect();
    drop(tabs);
    p.refresh();
    assert!(p
        .search(Scope::Tabs, "", None, &ticket())
        .unwrap_err()
        .contains("512"));
}
#[test]
fn app_shortcuts_decode_without_registering_launcher_hotkeys() {
    let (f, p) = setup();
    let rows = p
        .search(Scope::Menus, "Open", f.previous(), &ticket())
        .unwrap();
    assert!(rows.commands[0].hotkey.is_none());
    let Metadata::Menu { menu } = rows.metadata.values().next().unwrap() else {
        panic!()
    };
    assert_eq!(menu.shortcut.as_ref().unwrap().label, "Command + O");
    let mut node = MenuNode {
        element: Element(0),
        role: String::new(),
        title: String::new(),
        enabled: None,
        character: Some("+".into()),
        glyph: None,
        virtual_key: None,
        modifiers: Some(7),
    };
    assert_eq!(
        providers::shortcut(&node).unwrap().keys,
        ["⌃", "⌥", "⇧", "⌘", "+"]
    );
    node.character = Some("ß".into());
    node.modifiers = Some(8);
    assert_eq!(providers::shortcut(&node).unwrap().keys, ["ß"]);
    node.character = None;
    node.glyph = Some(122);
    assert_eq!(providers::shortcut(&node).unwrap().keys, ["F12"]);
    node.modifiers = None;
    assert!(providers::shortcut(&node).is_none());
    for (character,glyph,virtual_key,modifiers,keys,label) in [
        (Some("X"),None,None,13,vec!["⌃","⇧","X"],"Control + Shift + X"),
        (None,Some(100),None,0,vec!["⌘","←"],"Command + Left Arrow"),
        (None,None,Some(36),0,vec!["⌘","↵"],"Command + Return"),
        (None,None,Some(80),0,vec!["⌘","F19"],"Command + F19"),
        (Some("é"),None,Some(0),0,vec!["⌘","É"],"Command + É"),
        (Some("ß"),None,None,0,vec!["⌘","ß"],"Command + ß"),
        (Some("\u{f700}"),None,None,0,vec!["⌘","↑"],"Command + Up Arrow"),
    ] {
        node.character=character.map(str::to_owned);node.glyph=glyph;node.virtual_key=virtual_key;node.modifiers=Some(modifiers);
        let decoded=providers::shortcut(&node).unwrap();assert_eq!(decoded.keys,keys);assert_eq!(decoded.label,label);
    }
    node.character=Some("X".into());node.modifiers=Some(16);
    assert!(providers::shortcut(&node).is_none());
}
#[test]
fn shortcut_uuid_parser_overrides_and_execution_tuple_are_exact() {
    let text=b"Focus (11111111-1111-4111-8111-111111111111)\nFocus (22222222-2222-4222-8222-222222222222)\n";
    let commands = apple_shortcuts::parse(text).unwrap();
    assert_ne!(commands[0].id, commands[1].id);
    assert!(commands.iter().all(apple_shortcuts::recognizes));
    for bad in [
        b"Missing identifier".as_slice(),
        b"Multiline\nFocus (11111111-1111-4111-8111-111111111111)",
        b"Focus (not-a-uuid)",
    ] {
        assert!(apple_shortcuts::parse(bad).is_err());
    }
    let mut duplicate = text.to_vec();
    duplicate.extend_from_slice(text);
    assert!(apple_shortcuts::parse(&duplicate).is_err());
    let mut saved = commands[1].clone();
    saved.aliases = vec!["work".into()];
    saved.hotkey = Some("cmd+alt+w".into());
    assert_eq!(
        apple_shortcuts::merged(&commands, &[saved.clone()], "work")[0].hotkey,
        saved.hotkey
    );
    saved.enabled = Some(false);
    assert_eq!(
        apple_shortcuts::merged(&commands, &[saved.clone()], "").len(),
        1
    );
    saved.arguments[1] = "Focus".into();
    assert!(!apple_shortcuts::recognizes(&saved));
    let unicode=apple_shortcuts::parse("Café 🌿 $(ignored) (33333333-3333-4333-8333-333333333333)\n".as_bytes()).unwrap();
    assert_eq!(unicode[0].title,"Café 🌿 $(ignored)");
    assert_eq!(unicode[0].arguments,["run","33333333-3333-4333-8333-333333333333"]);
    assert!(apple_shortcuts::parse(b"").unwrap().is_empty());
    let oversized=(0..1001).map(|i|format!("Owned {i} ({i:08x}-1111-4111-8111-111111111111)\n")).collect::<String>();
    assert!(apple_shortcuts::parse(oversized.as_bytes()).unwrap_err().contains("1,000"));
    for mutation in ["kind","target","verb","uuid","extra"] {
        let mut command=unicode[0].clone();
        match mutation {"kind"=>command.kind="url".into(),"target"=>command.target="/bin/echo".into(),"verb"=>command.arguments[0]="list".into(),"uuid"=>command.arguments[1]="33333333-3333-4333-8333-333333333334".into(),_=>command.arguments.push("extra".into())}
        assert!(!apple_shortcuts::recognizes(&command),"{mutation}");
    }
}
#[test]
fn owned_shortcut_children_drain_both_pipes_and_never_wait_for_inherited_eof() {
    let t = ticket();
    assert_eq!(
        apple_shortcuts::capture("/bin/sh", &["-c", "printf owned"], &t, 128).unwrap(),
        b"owned"
    );
    for script in [
        "exit 1",
        "exec /bin/sleep 5",
        "while :; do printf xxxxxxxxxx; done",
        "while :; do printf xxxxxxxxxx >&2; done",
        "/bin/sleep 1 & exit 0",
    ] {
        let started = Instant::now();
        let t = Ticket {
            deadline: started + Duration::from_millis(50),
            live: Arc::new(|| true),
        };
        assert!(apple_shortcuts::capture("/bin/sh", &["-c", script], &t, 128).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    // Revoke only after the disposable child has actually entered its body.
    // This is distinct from the existing deadline and before-spawn checks.
    let parent=std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").map(std::path::PathBuf::from)
        .unwrap_or_else(||std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"));
    let directory=parent.join(format!("shortcut-cancel-{}",std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {fn drop(&mut self){std::fs::remove_dir_all(&self.0).unwrap();}}
    let _cleanup=Cleanup(directory.clone());
    let marker=directory.join("entered");
    let live=Arc::new(AtomicBool::new(true));
    let t=Ticket{deadline:Instant::now()+Duration::from_secs(2),live:{let live=live.clone();Arc::new(move||live.load(Ordering::Acquire))}};
    let child_marker=marker.clone();
    let started=Instant::now();
    let worker=std::thread::spawn(move||apple_shortcuts::capture("/bin/sh",&["-c","printf entered > \"$1\"; exec /bin/sleep 5","owned",child_marker.to_str().unwrap()],&t,128));
    let until=Instant::now()+Duration::from_secs(1);
    while std::fs::read(&marker).ok().as_deref()!=Some(b"entered") && Instant::now()<until {
        std::thread::sleep(Duration::from_millis(2));
    }
    let entered=std::fs::read(&marker).ok().as_deref()==Some(b"entered");
    live.store(false,Ordering::Release);
    let result=worker.join().unwrap();
    assert!(entered,"owned shortcut child did not enter");
    assert_eq!(result,Err(CANCELLED.into()));
    assert!(started.elapsed()<Duration::from_secs(1));
}
#[test]
fn main_service_queued_effect_is_revoked_before_native_adapter() {
    let (client, main) = main_bridge::channel();
    let (cancel, woke) = std::sync::mpsc::channel();
    main.attach_wake(Arc::new(move || {
        let _ = cancel.send(());
    }));
    woke.recv().unwrap();
    let live = Arc::new(AtomicBool::new(true));
    let copy = live.clone();
    let t = Ticket {
        deadline: Instant::now() + Duration::from_secs(1),
        live: Arc::new(move || copy.load(Ordering::Acquire)),
    };
    let id = Identity {
        pid: 12345,
        uid: 0,
        seconds: 1,
        micros: 0,
    };
    let worker = std::thread::spawn(move || client.activate(id, &t));
    woke.recv_timeout(Duration::from_secs(1)).unwrap();
    live.store(false, Ordering::Release);
    main.drain_with(
        || panic!("No foreground read"),
        |_, _| panic!("No activation after cancellation"),
    );
    assert!(worker.join().unwrap().is_err());
}

#[test]
fn detached_menu_path_is_never_replaced_by_a_same_title_item() {
    let (f, p) = setup();
    let previous = f.previous().unwrap();
    let rows = p
        .search(Scope::Menus, "Open", Some(previous.clone()), &ticket())
        .unwrap();
    f.edit(Element(6), |n| {
        n.lists.insert("AXChildren".into(), vec![Element(8)]);
    });
    assert!(p
        .execute(
            &rows.commands[0].id,
            Some(previous.identity),
            &ticket(),
            &f.activate()
        )
        .is_err());
    assert!(f.effects.lock().unwrap().is_empty());
}

#[test]
fn generated_uuid_and_builtin_bindings_persist_without_resurrecting_deleted_saved_rows() {
    use crate::{
        actions::{Editing, Mode},
        profile::ProfilePaths,
        state::HostState,
    };
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!(
            "owned-system-bindings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(root.clone());
    let HostState {
        mut configuration, ..
    } = HostState::open(ProfilePaths::new(root.join("p"), root, None)).unwrap();
    let mut edit = Editing::default();
    let apps = crate::app_choice::Choices::default();
    let commands=[apple_shortcuts::parse(b"Owned (aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee)").unwrap().remove(0),serde_json::from_value(serde_json::json!({"id":"builtin.dashboard","title":"Ruycast Dashboard","kind":"builtin","target":"analytics"})).unwrap()];
    for (index, c) in commands.into_iter().enumerate() {
        edit.open(&configuration, &c, Mode::Alias);
        let token = edit.token().unwrap();
        let alias = format!("owned-binding-{index}");
        edit.save_with_apps(&mut configuration, token, Some(alias.clone()), None, &apps)
            .unwrap();
        let saved = configuration
            .current()
            .commands
            .iter()
            .find(|r| r.id == c.id)
            .unwrap()
            .clone();
        assert_eq!(saved.aliases, [alias]);
        assert_eq!(saved.arguments, c.arguments);
        edit.open(&configuration,&saved,Mode::Hotkey);
        let hotkey_token=edit.token().unwrap();
        edit.save_with_apps(&mut configuration,hotkey_token,None,Some(Some("cmd+alt+w".into())),&apps).unwrap();
        let saved=configuration.current().commands.iter().find(|row|row.id==c.id).unwrap().clone();
        assert_eq!(saved.hotkey.as_deref(),Some("cmd+alt+w"));
        // A fresh persisted-state owner has no discovery cache. This is a
        // cold configuration read, not a second host-process acceptance claim.
        let reopened=HostState::open(configuration.paths.clone()).unwrap();
        let restored=reopened.configuration.current().commands.iter().find(|row|row.id==saved.id).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(),serde_json::to_value(&saved).unwrap());
        assert_eq!(restored.arguments,c.arguments);
        drop(reopened);
        edit.open(&configuration, &saved, Mode::Hotkey);
        let token = edit.token().unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(configuration.snapshot()).unwrap();
        value["commands"]
            .as_array_mut()
            .unwrap()
            .retain(|r| r["id"] != saved.id);
        configuration.save(&value).unwrap();
        assert!(edit
            .save_with_apps(
                &mut configuration,
                token,
                None,
                Some(Some("cmd+alt+w".into())),
                &apps
            )
            .is_err());
        assert!(!configuration
            .current()
            .commands
            .iter()
            .any(|r| r.id == saved.id));
        edit.cancel_all();
    }
}

#[test]
fn window_unknown_minimized_untitled_and_old_snapshot_metadata_are_preserved() {
    let (f, p) = setup();
    f.edit(Element(2), |node| {
        node.strings.insert("AXTitle".into(), String::new());
        node.bools.remove("AXMinimized");
    });
    let original = p.search(Scope::Windows, "", None, &ticket()).unwrap();
    let command = original
        .commands
        .iter()
        .find(|row| row.title == "Untitled Window")
        .unwrap();
    let Metadata::Window { window } = original.metadata.get(&command.id).unwrap() else {
        panic!()
    };
    assert_eq!(window.minimized, None);
    assert_eq!(window.application, "Owned Editor");
    f.apps.lock().unwrap()[0].name = "Renamed Owned Editor".into();
    let newer = p.search(Scope::Windows, "", None, &ticket()).unwrap();
    assert!(newer.commands.iter().all(|row| row.id != command.id));
    assert!(newer.metadata.values().all(|row| matches!(row, Metadata::Window { window } if window.application == "Renamed Owned Editor")));
    assert_eq!(window.application, "Owned Editor");
    assert!(f.effects.lock().unwrap().is_empty());
    p.execute(&command.id, None, &ticket(), &f.activate())
        .unwrap();
    assert_eq!(*f.effects.lock().unwrap(), ["activate", "AXRaise"]);
}

#[test]
fn window_invalid_identity_and_unreadable_membership_refuse_before_activation() {
    for change in [
        "role",
        "window-pid",
        "application-pid",
        "membership",
        "unreadable-membership",
        "app-exit",
        "pid-reuse",
        "trust",
    ] {
        let (f, p) = setup();
        let rows = p
            .search(Scope::Windows, "Owned Project", None, &ticket())
            .unwrap();
        assert_eq!(rows.commands.len(), 1);
        match change {
            "role" => f.edit(Element(2), |n| {
                n.strings.insert("AXRole".into(), "AXButton".into());
            }),
            "window-pid" => f.edit(Element(2), |n| n.pid += 1),
            "application-pid" => f.edit(Element(1), |n| n.pid += 1),
            "membership" => f.edit(Element(1), |n| {
                n.lists.insert("AXWindows".into(), vec![Element(3)]);
            }),
            "unreadable-membership" => f.edit(Element(1), |n| n.unreadable_lists = true),
            "app-exit" => f.apps.lock().unwrap().clear(),
            "pid-reuse" => f.apps.lock().unwrap()[0].identity.seconds += 1,
            _ => f.trusted.store(false, Ordering::Release),
        }
        assert!(
            p.execute(&rows.commands[0].id, None, &ticket(), &f.activate())
                .is_err(),
            "{change}"
        );
        assert!(f.effects.lock().unwrap().is_empty(), "{change}");
    }
}

#[test]
fn window_permission_and_app_exit_during_activation_stop_before_raise() {
    for change in ["trust", "app-exit"] {
        let (f, p) = setup();
        let rows = p
            .search(Scope::Windows, "Owned Project", None, &ticket())
            .unwrap();
        let fixture = f.clone();
        let activate: Activate = Arc::new(move |_, _| {
            fixture.effects.lock().unwrap().push("activate".into());
            if change == "trust" {
                fixture.trusted.store(false, Ordering::Release);
            } else {
                fixture.apps.lock().unwrap().clear();
            }
            Ok(())
        });
        assert!(
            p.execute(&rows.commands[0].id, None, &ticket(), &activate)
                .is_err(),
            "{change}"
        );
        assert_eq!(*f.effects.lock().unwrap(), ["activate"], "{change}");
    }
}

#[test]
fn window_restore_status_readback_and_membership_are_verified_before_raise() {
    for change in [
        "restore-failure",
        "still-minimized",
        "closed",
        "trust",
        "app-exit",
        "control",
    ] {
        let (f, p) = setup();
        let rows = p
            .search(Scope::Windows, "Owned Draft", None, &ticket())
            .unwrap();
        assert_eq!(rows.commands.len(), 1);
        if change == "restore-failure" {
            f.restore_failure.store(true, Ordering::Release);
        } else if change != "control" {
            *f.after_window_restore.lock().unwrap() = Some(Box::new(move |fixture| match change {
                "still-minimized" => fixture.edit(Element(3), |n| {
                    n.bools.insert("AXMinimized".into(), true);
                }),
                "closed" => fixture.edit(Element(1), |n| {
                    n.lists.insert("AXWindows".into(), vec![Element(2)]);
                }),
                "trust" => fixture.trusted.store(false, Ordering::Release),
                _ => fixture.apps.lock().unwrap().clear(),
            }));
        }
        let result = p.execute(&rows.commands[0].id, None, &ticket(), &f.activate());
        if change == "control" {
            result.unwrap();
            assert_eq!(
                *f.effects.lock().unwrap(),
                ["activate", "restore", "AXRaise"]
            );
        } else {
            assert!(result.is_err(), "{change}");
            assert_eq!(
                *f.effects.lock().unwrap(),
                ["activate", "restore"],
                "{change}"
            );
        }
        assert!(f.after_window_restore.lock().unwrap().is_none());
    }
}

#[test]
fn window_raise_error_is_not_reported_as_success() {
    let (f, p) = setup();
    let rows = p
        .search(Scope::Windows, "Owned Project", None, &ticket())
        .unwrap();
    f.raise_failure.store(true, Ordering::Release);
    assert!(p
        .execute(&rows.commands[0].id, None, &ticket(), &f.activate())
        .is_err());
    assert_eq!(*f.effects.lock().unwrap(), ["activate", "AXRaise"]);
}

#[test]
fn menu_unknown_enabled_and_unreadable_submenu_never_offer_an_executable_leaf() {
    let (f, p) = setup();
    let app = f.previous().unwrap();
    f.edit(Element(7), |n| {
        n.bools.remove("AXEnabled");
    });
    let rows = p
        .search(Scope::Menus, "Open", Some(app.clone()), &ticket())
        .unwrap();
    assert_eq!(rows.commands.len(), 1);
    assert_eq!(rows.commands[0].enabled, Some(false));
    let Metadata::Menu { menu } = rows.metadata.get(&rows.commands[0].id).unwrap() else {
        panic!()
    };
    assert!(!menu.enabled);
    assert!(p
        .execute(
            &rows.commands[0].id,
            Some(app.identity),
            &ticket(),
            &f.activate()
        )
        .is_err());
    assert!(f.effects.lock().unwrap().is_empty());
    // Attributes still name an enabled submenu; only AXChildren is unreadable.
    // Treating that error as an empty children list would expose a false leaf.
    f.edit(Element(7), |n| {
        n.bools.insert("AXEnabled".into(), true);
        n.lists.insert("AXChildren".into(), vec![Element(8)]);
        n.unreadable_lists = true;
    });
    let rows = p
        .search(Scope::Menus, "Open", Some(app), &ticket())
        .unwrap();
    assert!(rows.commands.is_empty());
    assert!(f.effects.lock().unwrap().is_empty());
}

#[test]
fn menu_original_identity_must_match_the_complete_captured_identity() {
    let (f, p) = setup();
    let app = f.previous().unwrap();
    let rows = p
        .search(Scope::Menus, "Open", Some(app.clone()), &ticket())
        .unwrap();
    for change in ["pid", "start-time"] {
        let mut wrong = app.identity;
        if change == "pid" {
            wrong.pid += 1;
        } else {
            wrong.micros += 1;
        }
        assert!(
            p.execute(&rows.commands[0].id, Some(wrong), &ticket(), &f.activate())
                .is_err(),
            "{change}"
        );
        assert!(f.effects.lock().unwrap().is_empty(), "{change}");
    }
}

#[test]
fn menu_post_activation_item_ancestor_identity_and_trust_are_revalidated() {
    for change in [
        "enabled",
        "unknown-enabled",
        "title",
        "ancestor-title",
        "ancestor-enabled",
        "role",
        "ancestor-role",
        "unreadable-children",
        "pid",
        "ancestor-pid",
        "app-exit",
        "trust",
    ] {
        let (f, p) = setup();
        let app = f.previous().unwrap();
        let rows = p
            .search(Scope::Menus, "Open", Some(app.clone()), &ticket())
            .unwrap();
        assert_eq!(rows.commands.len(), 1);
        let fixture = f.clone();
        let activate: Activate = Arc::new(move |_, _| {
            fixture.effects.lock().unwrap().push("activate".into());
            match change {
                "enabled" => fixture.edit(Element(7), |n| {
                    n.bools.insert("AXEnabled".into(), false);
                }),
                "unknown-enabled" => fixture.edit(Element(7), |n| {
                    n.bools.remove("AXEnabled");
                }),
                "title" => fixture.edit(Element(7), |n| {
                    n.strings.insert("AXTitle".into(), "Changed".into());
                }),
                "ancestor-title" => fixture.edit(Element(5), |n| {
                    n.strings.insert("AXTitle".into(), "Changed".into());
                }),
                "ancestor-enabled" => fixture.edit(Element(5), |n| {
                    n.bools.insert("AXEnabled".into(), false);
                }),
                "role" => fixture.edit(Element(7), |n| {
                    n.strings.insert("AXRole".into(), "AXButton".into());
                }),
                "ancestor-role" => fixture.edit(Element(5), |n| {
                    n.strings.insert("AXRole".into(), "AXGroup".into());
                }),
                "unreadable-children" => fixture.edit(Element(7), |n| n.unreadable_lists = true),
                "pid" => fixture.edit(Element(7), |n| n.pid += 1),
                "ancestor-pid" => fixture.edit(Element(5), |n| n.pid += 1),
                "app-exit" => fixture.apps.lock().unwrap().clear(),
                _ => fixture.trusted.store(false, Ordering::Release),
            }
            Ok(())
        });
        assert!(
            p.execute(
                &rows.commands[0].id,
                Some(app.identity),
                &ticket(),
                &activate
            )
            .is_err(),
            "{change}"
        );
        assert_eq!(*f.effects.lock().unwrap(), ["activate"], "{change}");
    }
}

#[test]
fn owned_application_roots_are_pid_specific_and_default_is_preserved() {
    let f = fixture::Fixture::default();
    assert_eq!(f.application(730001, &ticket()).unwrap(), Element(1));
    assert!(f.application(740000, &ticket()).is_err());
    f.application_roots
        .lock()
        .unwrap()
        .insert(740000, Element(99));
    assert_eq!(f.application(740000, &ticket()).unwrap(), Element(99));
    assert_eq!(f.application(730001, &ticket()).unwrap(), Element(1));
}

fn large_owned_window_fixture() -> (Arc<fixture::Fixture>, providers::Provider) {
    let (f, p) = setup();
    f.trace_read_requests.store(true, Ordering::Release);
    f.apps.lock().unwrap().clear();
    f.nodes.lock().unwrap().clear();
    f.application_roots.lock().unwrap().clear();
    for i in 0..205_u64 {
        let pid = 740000 + i as i32;
        let root = Element(100 + i * 100);
        f.apps.lock().unwrap().push(Application {
            identity: Identity {
                pid,
                uid: unsafe { libc::getuid() },
                seconds: 11,
                micros: 22,
            },
            name: format!("Large Owned {i}"),
            bundle_identifier: None,
        });
        f.application_roots.lock().unwrap().insert(pid, root);
        let mut app = fixture::Node {
            pid,
            ..Default::default()
        };
        let mut windows = Vec::new();
        for j in 0..55_u64 {
            let element = Element(root.0 + j + 1);
            windows.push(element);
            let mut node = fixture::Node {
                pid,
                ..Default::default()
            };
            node.strings.insert("AXRole".into(), "AXWindow".into());
            node.strings.insert("AXTitle".into(), format!("Window {j}"));
            node.bools.insert("AXMinimized".into(), false);
            f.nodes.lock().unwrap().insert(element, node);
        }
        app.lists.insert("AXWindows".into(), windows);
        f.nodes.lock().unwrap().insert(root, app);
    }
    (f, p)
}

#[test]
fn window_scanner_caps_two_hundred_apps_fifty_windows_and_one_hundred_results() {
    let (f, p) = large_owned_window_fixture();
    let rows = p.search(Scope::Windows, "", None, &ticket()).unwrap();
    assert_eq!(rows.commands.len(), 100);
    assert_eq!(rows.metadata.len(), 100);
    let requests = f.list_requests.lock().unwrap();
    assert!(requests
        .iter()
        .all(|(_, key, limit)| key == "AXWindows" && *limit == 50));
    drop(requests);
    f.application_reads.lock().unwrap().clear();
    f.list_requests.lock().unwrap().clear();
    let rows = p
        .search(Scope::Windows, "not-present", None, &ticket())
        .unwrap();
    assert!(rows.commands.is_empty());
    let apps = f.application_reads.lock().unwrap();
    assert_eq!(apps.len(), 200);
    assert_eq!(apps.first(), Some(&740000));
    assert_eq!(apps.last(), Some(&740199));
    let requests = f.list_requests.lock().unwrap();
    assert_eq!(requests.len(), 200);
    assert_eq!(
        requests
            .iter()
            .map(|(node, _, _)| *node)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        200
    );
    assert!(requests
        .iter()
        .all(|(_, key, limit)| key == "AXWindows" && *limit == 50));
    assert!(f.effects.lock().unwrap().is_empty());
}

fn large_owned_menu_fixture() -> (Arc<fixture::Fixture>, providers::Provider) {
    let (f, p) = setup();
    f.trace_read_requests.store(true, Ordering::Release);
    let pid = f.previous().unwrap().identity.pid;
    let root = Element(100);
    let mut menu = fixture::Node {
        pid,
        ..Default::default()
    };
    menu.strings.insert("AXRole".into(), "AXMenuBar".into());
    let leaves = (0..1400_u64).map(|i| Element(1000 + i)).collect::<Vec<_>>();
    menu.lists.insert("AXChildren".into(), leaves.clone());
    f.nodes.lock().unwrap().insert(root, menu);
    f.edit(Element(1), |node| {
        node.elements.insert("AXMenuBar".into(), root);
    });
    for (i, element) in leaves.into_iter().enumerate() {
        let mut leaf = fixture::Node {
            pid,
            ..Default::default()
        };
        leaf.strings.insert("AXRole".into(), "AXMenuItem".into());
        leaf.strings.insert("AXTitle".into(), format!("Item {i}"));
        leaf.bools.insert("AXEnabled".into(), true);
        f.nodes.lock().unwrap().insert(element, leaf);
    }
    (f, p)
}

#[test]
fn menu_scanner_caps_results_nodes_and_depth_without_visiting_hidden_tail() {
    let (f, p) = large_owned_menu_fixture();
    let rows = p
        .search(Scope::Menus, "Item", f.previous(), &ticket())
        .unwrap();
    assert_eq!(rows.commands.len(), 100);
    assert_eq!(rows.metadata.len(), 100);
    f.list_requests.lock().unwrap().clear();
    let rows = p
        .search(Scope::Menus, "not-present", f.previous(), &ticket())
        .unwrap();
    assert!(rows.commands.is_empty());
    let requests = f.list_requests.lock().unwrap();
    assert!((1199..=1200).contains(&requests.len()));
    assert_eq!(
        requests
            .iter()
            .map(|(node, _, _)| *node)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        requests.len()
    );
    assert!(!requests.iter().any(|(node, _, _)| node.0 >= 2199));
    assert!(requests
        .iter()
        .all(|(_, key, limit)| key == "AXChildren" && *limit <= 1199));
    drop(requests);
    assert_eq!(
        p.search(Scope::Menus, "Item 1187", f.previous(), &ticket())
            .unwrap()
            .commands
            .len(),
        1
    );
    assert!(p
        .search(Scope::Menus, "Item 1399", f.previous(), &ticket())
        .unwrap()
        .commands
        .is_empty());

    let (f, p) = setup();
    f.trace_read_requests.store(true, Ordering::Release);
    let pid = f.previous().unwrap().identity.pid;
    f.edit(Element(1), |node| {
        node.elements.insert("AXMenuBar".into(), Element(100));
    });
    for depth in 0..14_u64 {
        let mut node = fixture::Node {
            pid,
            ..Default::default()
        };
        node.strings.insert(
            "AXRole".into(),
            if depth == 0 {
                "AXMenuBar"
            } else {
                "AXMenuItem"
            }
            .into(),
        );
        node.strings
            .insert("AXTitle".into(), format!("Depth {depth}"));
        node.bools.insert("AXEnabled".into(), true);
        if depth < 13 {
            node.lists
                .insert("AXChildren".into(), vec![Element(101 + depth)]);
        }
        f.nodes.lock().unwrap().insert(Element(100 + depth), node);
    }
    assert!(p
        .search(Scope::Menus, "", f.previous(), &ticket())
        .unwrap()
        .commands
        .is_empty());
    assert_eq!(f.list_requests.lock().unwrap().len(), 12);
    assert!(!f
        .list_requests
        .lock()
        .unwrap()
        .iter()
        .any(|(node, _, _)| node.0 >= 112));
    f.edit(Element(111), |node| {
        node.lists.insert("AXChildren".into(), vec![]);
    });
    let rows = p
        .search(Scope::Menus, "Depth 11", f.previous(), &ticket())
        .unwrap();
    assert_eq!(rows.commands.len(), 1);
    assert!(f.effects.lock().unwrap().is_empty());
}

#[test]
fn menu_nonmatching_scan_preserves_original_attribute_read_budget() {
    let (f, p) = large_owned_menu_fixture();
    let rows = p
        .search(Scope::Menus, "not-present", f.previous(), &ticket())
        .unwrap();
    assert!(rows.commands.is_empty());
    let total = f.reads.load(Ordering::Acquire);
    let lists = f.list_requests.lock().unwrap().len();
    let applications = f.application_reads.lock().unwrap().len();
    // This menu path has no applications() call: all remaining counted reads
    // are scalar/element attributes, exactly the original Swift readCount scope.
    let attributes = total - lists - applications;
    println!("owned menu attribute_reads={attributes} list_reads={lists} total_reads={total}");
    assert!(attributes < 6500, "original AX attribute budget exceeded: attributes={attributes}, lists={lists}, total={total}");
}

#[test]
fn window_internal_two_second_budget_returns_partial_while_request_is_live() {
    let (f, p) = setup();
    // Unlike a request-expiry test, this request remains live throughout the
    // original two-second scanner budget. Only disposable attribute reads wait.
    f.delay_ms.store(250, Ordering::Release);
    let request = Ticket {
        deadline: Instant::now() + Duration::from_secs(10),
        live: Arc::new(|| true),
    };
    let rows = p.search(Scope::Windows, "", None, &request).unwrap();
    request.check().unwrap();
    assert_eq!(
        rows.commands.len(),
        1,
        "original internal scanner budget should retain only the completed first window, reads={}",
        f.reads.load(Ordering::Acquire)
    );
    assert!(f.effects.lock().unwrap().is_empty());
}

#[test]
fn menu_internal_two_second_budget_stops_while_request_is_live() {
    let (f, p) = setup();
    f.delay_ms.store(80, Ordering::Release);
    let request = Ticket {
        deadline: Instant::now() + Duration::from_secs(10),
        live: Arc::new(|| true),
    };
    let rows = p.search(Scope::Menus, "", f.previous(), &request).unwrap();
    request.check().unwrap();
    assert_eq!(
        rows.commands.len(),
        1,
        "original internal scanner budget must retain the completed first menu leaf, reads={}",
        f.reads.load(Ordering::Acquire)
    );
    assert!(f.effects.lock().unwrap().is_empty());
}

#[test]
fn window_internal_two_second_budget_can_return_empty_without_cancelling_request() {
    let (f, p) = setup();
    f.delay_ms.store(600, Ordering::Release);
    let request = Ticket {
        deadline: Instant::now() + Duration::from_secs(10),
        live: Arc::new(|| true),
    };
    let rows = p.search(Scope::Windows, "", None, &request).unwrap();
    request.check().unwrap();
    assert!(
        rows.commands.is_empty(),
        "original internal budget expires before any complete window, reads={}",
        f.reads.load(Ordering::Acquire)
    );
    assert!(f.effects.lock().unwrap().is_empty());
}

#[test]
fn menu_node_cap_must_not_turn_a_submenu_into_a_leaf() {
    let (f, p) = large_owned_menu_fixture();
    let mut children = (0..1198_u64).map(|i| Element(1000 + i)).collect::<Vec<_>>();
    children.push(Element(5000));
    f.edit(Element(100), |node| {
        node.lists.insert("AXChildren".into(), children);
    });
    let pid = f.previous().unwrap().identity.pid;
    let mut boundary = fixture::Node {
        pid,
        ..Default::default()
    };
    boundary
        .strings
        .insert("AXRole".into(), "AXMenuItem".into());
    boundary
        .strings
        .insert("AXTitle".into(), "Boundary Submenu".into());
    boundary.bools.insert("AXEnabled".into(), true);
    boundary
        .lists
        .insert("AXChildren".into(), vec![Element(5001)]);
    f.nodes.lock().unwrap().insert(Element(5000), boundary);
    let rows = p
        .search(Scope::Menus, "Boundary Submenu", f.previous(), &ticket())
        .unwrap();
    assert!(
        rows.commands.is_empty(),
        "zero remaining child-list budget must not fabricate an executable leaf: {:?}",
        rows.commands
    );
    assert!(f.effects.lock().unwrap().is_empty());
}

#[test]
fn scan_platform_error_is_not_reclassified_as_internal_budget_exhaustion() {
    for scope in [Scope::Windows, Scope::Menus] {
        let (f, p) = setup();
        let previous = f.previous();
        f.application_roots.lock().unwrap().clear();
        f.delay_ms.store(
            if scope == Scope::Windows { 1100 } else { 2100 },
            Ordering::Release,
        );
        let request = Ticket {
            deadline: Instant::now() + Duration::from_secs(10),
            live: Arc::new(|| true),
        };
        let error = p.search(scope, "", previous, &request).unwrap_err();
        request.check().unwrap();
        assert_eq!(error, "Owned AX application root is unavailable");
        assert!(f.effects.lock().unwrap().is_empty());
    }
}

#[test]
fn menu_initial_identity_validation_precedes_the_internal_scan_budget() {
    let (f, p) = setup();
    // Swift validates the original application before starting its menu clock.
    // Only that initial fixture identity call waits; all scan reads stay fast.
    *f.before_identity.lock().unwrap() = Some(Box::new(|_| {
        std::thread::sleep(Duration::from_millis(2100));
    }));
    let request = Ticket {
        deadline: Instant::now() + Duration::from_secs(10),
        live: Arc::new(|| true),
    };
    let rows = p.search(Scope::Menus, "", f.previous(), &request).unwrap();
    request.check().unwrap();
    assert_eq!(rows.commands.len(), 2, "initial identity validation must not consume the original private menu traversal budget");
    assert!(f.before_identity.lock().unwrap().is_none());
    assert!(f.effects.lock().unwrap().is_empty());
}

#[test]
fn window_original_application_and_title_filters_preserve_distinct_duplicate_targets() {
    let (fixture, provider) = setup();
    let original = fixture.previous().unwrap();
    for node in [Element(2), Element(3)] {
        fixture.edit(node, |window| {
            window
                .strings
                .insert("AXTitle".into(), "Project Notes".into());
        });
    }
    for (element, title) in [(Element(9), "Draft"), (Element(10), "Inspector")] {
        let mut window = fixture.nodes.lock().unwrap()[&Element(2)].clone();
        window.strings.insert("AXTitle".into(), title.into());
        window.bools.remove("AXMinimized");
        fixture.nodes.lock().unwrap().insert(element, window);
    }
    fixture.edit(Element(1), |application| {
        application.lists.insert(
            "AXWindows".into(),
            vec![Element(2), Element(3), Element(9), Element(10)],
        );
    });
    let mut other = original.clone();
    other.identity.pid += 1;
    other.name = "Owned Browser".into();
    fixture.apps.lock().unwrap().push(other.clone());
    fixture
        .application_roots
        .lock()
        .unwrap()
        .insert(other.identity.pid, Element(11));
    let mut application = fixture.nodes.lock().unwrap()[&Element(1)].clone();
    application.pid = other.identity.pid;
    application
        .lists
        .insert("AXWindows".into(), vec![Element(12)]);
    fixture
        .nodes
        .lock()
        .unwrap()
        .insert(Element(11), application);
    let mut untitled = fixture.nodes.lock().unwrap()[&Element(10)].clone();
    untitled.pid = other.identity.pid;
    untitled.strings.insert("AXTitle".into(), String::new());
    fixture.nodes.lock().unwrap().insert(Element(12), untitled);

    let all = provider
        .search(Scope::Windows, "", None, &ticket())
        .unwrap();
    assert_eq!(all.commands.len(), 5);
    assert_eq!(all.metadata.len(), 5);
    assert!(all
        .commands
        .iter()
        .any(|row| row.title == "Untitled Window"));
    let editor = provider
        .search(Scope::Windows, "EDITOR", None, &ticket())
        .unwrap();
    assert_eq!(editor.commands.len(), 4);
    assert!(editor.metadata.values().all(|metadata| matches!(metadata, Metadata::Window { window } if window.application == "Owned Editor")));
    let project = provider
        .search(Scope::Windows, "pRoJeCt", None, &ticket())
        .unwrap();
    assert_eq!(project.commands.len(), 2);
    assert!(project
        .commands
        .iter()
        .all(|row| row.title == "Project Notes"));
    assert_ne!(project.commands[0].id, project.commands[1].id);
    assert!(project
        .commands
        .iter()
        .all(|row| !all.commands.iter().any(|old| old.id == row.id)));
    assert!(provider
        .search(Scope::Windows, "not-present", None, &ticket())
        .unwrap()
        .commands
        .is_empty());
    assert!(fixture.effects.lock().unwrap().is_empty());
}

#[test]
fn menu_original_four_items_duplicate_targets_queries_and_immutable_metadata_are_exact() {
    let (fixture, provider) = setup();
    fixture.apps.lock().unwrap()[0].name = "TextEdit Fixture".into();
    fixture.edit(Element(7), |node| {
        node.strings.insert("AXTitle".into(), "Open…".into());
    });
    fixture.edit(Element(8), |node| {
        node.strings.insert("AXTitle".into(), "Save".into());
    });
    let open = fixture.nodes.lock().unwrap()[&Element(7)].clone();
    fixture
        .nodes
        .lock()
        .unwrap()
        .insert(Element(9), open.clone());
    let mut unknown = open.clone();
    unknown
        .strings
        .insert("AXTitle".into(), "Unknown State".into());
    unknown.bools.remove("AXEnabled");
    fixture.nodes.lock().unwrap().insert(Element(10), unknown);
    let mut unreadable = open.clone();
    unreadable
        .strings
        .insert("AXTitle".into(), "Unreadable Submenu".into());
    unreadable
        .lists
        .insert("AXChildren".into(), vec![Element(12)]);
    unreadable.unreadable_lists = true;
    fixture
        .nodes
        .lock()
        .unwrap()
        .insert(Element(11), unreadable);
    let mut child = open;
    child
        .strings
        .insert("AXTitle".into(), "Hidden Child".into());
    fixture.nodes.lock().unwrap().insert(Element(12), child);
    fixture.edit(Element(6), |node| {
        node.lists.insert(
            "AXChildren".into(),
            vec![Element(7), Element(9), Element(8), Element(10), Element(11)],
        );
    });
    let original_app = fixture.previous().unwrap();
    let initial_reads = fixture.reads.load(Ordering::Acquire);
    let original = provider
        .search(Scope::Menus, "", Some(original_app.clone()), &ticket())
        .unwrap();
    assert!(fixture.reads.load(Ordering::Acquire) > initial_reads);
    assert_eq!(original.commands.len(), 4);
    assert_eq!(original.metadata.len(), 4);
    assert!(original
        .commands
        .iter()
        .all(|row| row.hotkey.is_none() && row.aliases.is_empty()));
    let opens = original
        .commands
        .iter()
        .filter(|row| row.title == "Open…")
        .collect::<Vec<_>>();
    assert_eq!(opens.len(), 2);
    assert_ne!(opens[0].id, opens[1].id);
    let Metadata::Menu { menu: first_info } = original.metadata.get(&opens[0].id).unwrap() else {
        panic!()
    };
    assert_eq!(first_info.application, "TextEdit Fixture");
    assert_eq!(first_info.path, "File");
    assert_eq!(first_info.shortcut.as_ref().unwrap().label, "Command + O");
    for title in ["Save", "Unknown State"] {
        let row = original
            .commands
            .iter()
            .find(|row| row.title == title)
            .unwrap();
        assert_eq!(row.enabled, Some(false));
        let Metadata::Menu { menu } = original.metadata.get(&row.id).unwrap() else {
            panic!()
        };
        assert!(!menu.enabled);
        assert!(provider
            .execute(
                &row.id,
                Some(original_app.identity),
                &ticket(),
                &fixture.activate()
            )
            .is_err());
        assert!(fixture.effects.lock().unwrap().is_empty());
    }
    assert!(!original
        .commands
        .iter()
        .any(|row| row.title.contains("Unreadable") || row.title == "Hidden Child"));
    fixture.apps.lock().unwrap()[0].name = "Renamed Fixture".into();
    let renamed = fixture.previous().unwrap();
    let later = provider
        .search(
            Scope::Menus,
            "Renamed Fixture",
            Some(renamed.clone()),
            &ticket(),
        )
        .unwrap();
    assert_eq!(later.commands.len(), 4);
    assert!(later.metadata.values().all(|metadata| matches!(metadata, Metadata::Menu { menu } if menu.application == "Renamed Fixture")));
    assert_eq!(first_info.application, "TextEdit Fixture");
    assert_eq!(original.commands.len(), 4);
    assert!(later
        .commands
        .iter()
        .all(|row| !original.commands.iter().any(|old| old.id == row.id)));
    for query in ["File Open", "Open"] {
        let found = provider
            .search(Scope::Menus, query, Some(renamed.clone()), &ticket())
            .unwrap();
        assert_eq!(found.commands.len(), 2);
        assert!(found.commands.iter().all(|row| row.title == "Open…"));
    }
    assert!(fixture.effects.lock().unwrap().is_empty());
    // Disable the other equal-title leaf so a same-title fallback cannot pass.
    for (enabled, disabled, chosen, rejected) in [
        (Element(7), Element(9), opens[0], opens[1]),
        (Element(9), Element(7), opens[1], opens[0]),
    ] {
        fixture.edit(enabled, |node| {
            node.bools.insert("AXEnabled".into(), true);
        });
        fixture.edit(disabled, |node| {
            node.bools.insert("AXEnabled".into(), false);
        });
        fixture.effects.lock().unwrap().clear();
        provider
            .execute(
                &chosen.id,
                Some(original_app.identity),
                &ticket(),
                &fixture.activate(),
            )
            .unwrap();
        assert_eq!(*fixture.effects.lock().unwrap(), ["activate", "AXPress"]);
        fixture.effects.lock().unwrap().clear();
        assert!(provider
            .execute(
                &rejected.id,
                Some(original_app.identity),
                &ticket(),
                &fixture.activate()
            )
            .is_err());
        assert!(fixture.effects.lock().unwrap().is_empty());
    }
}

#[test]
fn menu_internal_two_second_budget_returns_empty_after_entered_reads_while_request_is_live() {
    let (fixture, provider) = setup();
    let previous = fixture.previous().unwrap();
    let baseline = provider
        .search(Scope::Menus, "", Some(previous.clone()), &ticket())
        .unwrap();
    assert_eq!(
        baseline.commands.len(),
        2,
        "the same fixture must expose valid matching menu leaves before delay"
    );
    assert_eq!(baseline.metadata.len(), 2);
    fixture.trace_read_requests.store(true, Ordering::Release);
    fixture.application_reads.lock().unwrap().clear();
    fixture.list_requests.lock().unwrap().clear();
    fixture.delay_ms.store(600, Ordering::Release);
    let reads_before = fixture.reads.load(Ordering::Acquire);
    let request = Ticket {
        deadline: Instant::now() + Duration::from_secs(10),
        live: Arc::new(|| true),
    };
    let started = Instant::now();
    let results = provider
        .search(Scope::Menus, "", Some(previous.clone()), &request)
        .unwrap();
    let elapsed = started.elapsed();
    request.check().unwrap();
    assert!((request.live)());
    assert!(elapsed >= Duration::from_secs(2), "empty result must follow the real private scanner budget, not an early filter/type failure: {elapsed:?}");
    assert!(results.commands.is_empty());
    assert!(results.metadata.is_empty());
    assert_eq!(
        *fixture.application_reads.lock().unwrap(),
        [previous.identity.pid]
    );
    let reads = fixture.reads.load(Ordering::Acquire) - reads_before;
    assert!(
        (2..=4).contains(&reads),
        "unexpected reads after entering the genuine owned application/menu-root path: {reads}"
    );
    assert!(
        fixture.list_requests.lock().unwrap().is_empty(),
        "no descendant range should be read after the header exhausts the scanner budget"
    );
    assert!(fixture.effects.lock().unwrap().is_empty());
    println!(
        "owned_menu_empty elapsed_ms={} total_platform_reads={reads} request_still_live=true",
        elapsed.as_millis()
    );
}
