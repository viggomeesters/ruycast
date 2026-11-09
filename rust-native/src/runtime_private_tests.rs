#[test]
fn watched_reload_preserves_one_fresh_cli_reply_cancellation_and_shutdown() {
    use std::io::{BufRead,BufReader,Write}; use std::os::unix::net::UnixStream;
    fn client(socket: std::path::PathBuf) -> std::thread::JoinHandle<Value> {
        std::thread::spawn(move||{let mut stream=UnixStream::connect(socket).unwrap();stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();writeln!(stream,"{}",json!({"op":"reload"})).unwrap();let mut line=String::new();BufReader::new(stream).read_line(&mut line).unwrap();serde_json::from_str(&line).unwrap()})
    }
    for mode in ["complete","cancel","quit"] {
        let (_temp,mut state,mut palette,_ui,_)=private_fixture("watch.old",false);
        let mut management=WindowRuntime::new(crate::session::Session::new(vec![]),Default::default(),&state);
        let(events,requests)=mpsc::sync_channel(32);state.events=events.clone();
        let limits=crate::control::Limits{dispatch:if mode=="cancel"{Duration::from_millis(60)}else{Duration::from_secs(3)},..Default::default()};
        let mut server=crate::control::ControlServer::start(&state.configuration.paths.root,events.clone(),limits).unwrap();
        state.writer=Some(crate::configuration_jobs::Writer::new(server.profile_lease(),events,None));
        let(entered,wait)=mpsc::sync_channel(1);let(release,hold)=mpsc::sync_channel(1);
        state.writer.as_mut().unwrap().start(&state.configuration,Arc::new(||true),false,move|store|{let mut payload=ConfigPayload::reload(store)?;payload.response=Some(json!({"ok":true,"watchOnly":true}));entered.send(()).unwrap();hold.recv_timeout(Duration::from_secs(2)).unwrap();Ok(payload)}).unwrap();
        state.config_context=Some(ConfigContext::Watch);wait.recv_timeout(Duration::from_secs(1)).unwrap();
        let socket=state.configuration.paths.root.join("control.sock");let mut reply=Some(client(socket.clone()));
        let request=match requests.recv_timeout(Duration::from_secs(1)).unwrap(){HostEvent::Request(request)=>request,_=>panic!("owned reload request")};
        state.admit_cli_reload(request);
        assert!(matches!(state.config_context,Some(ConfigContext::WatchThenCli(_))));
        if mode=="complete" {
            let rejected=client(socket);
            let request=match requests.recv_timeout(Duration::from_secs(1)).unwrap(){HostEvent::Request(request)=>request,_=>panic!("second owned reload request")};
            state.admit_cli_reload(request);assert_eq!(rejected.join().unwrap()["ok"],false);
            assert!(matches!(state.config_context,Some(ConfigContext::WatchThenCli(_))));
        }
        let mut replacement=serde_json::to_value(state.configuration.current()).unwrap();
        replacement["commands"].as_array_mut().unwrap().push(json!({"id":"watch.new","title":"New","kind":"script","target":"/owned/fake"}));
        std::fs::write(&state.configuration.file,serde_json::to_vec(&replacement).unwrap()).unwrap();
        if mode=="cancel" {std::thread::sleep(Duration::from_millis(90));}
        let mut closed_before_release=None;
        if mode=="quit" {state.writer.as_ref().unwrap().stop();let mut deliveries=vec![];state.cancel_configuration_reply(&mut deliveries);assert_eq!(deliveries.len(),1);assert!(state.config_context.is_none());let response=reply.take().unwrap().join().unwrap();assert_eq!(response["ok"],false);closed_before_release=Some(response);}
        release.send(()).unwrap();
        let deadline=Instant::now()+Duration::from_secs(2);
        while state.config_busy() || state.config_context.is_some() {finish_configuration(&mut state,&mut palette,&mut management);assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));}
        let response=closed_before_release.unwrap_or_else(||reply.take().unwrap().join().unwrap());
        assert_eq!(response["ok"],mode=="complete","{mode}: {response}");
        assert!(response.get("watchOnly").is_none());
        assert_eq!(state.configuration.current().commands.iter().any(|c|c.id=="watch.new"),mode=="complete");
        if mode=="complete" {assert_eq!(response["result"]["commands"],state.configuration.current().commands.len());assert!(response["configurationWrite"].is_object());}
        server.stop();
    }
}
#[test]
fn utility_history_stale_show_logs_original_error_without_mutating_native_intent() {
    let (_temp, mut state, mut window, ui, mut command) = private_fixture("owned-trace-input", false);
    command.kind = "builtin".into(); command.target = "utility-text-history".into();
    let received = Instant::now();
    window.visibility_intent = Some(received+Duration::from_secs(1));
    let before = (window.session.visible,window.session.query.clone(),window.visibility_intent);
    let (result, trace) = utilities::show_command(&mut state,&mut window,&ui,&command,received,&json!({"input":"trace-input"}));
    assert_eq!(result.unwrap_err(), "A newer palette action superseded this request.");
    assert_eq!((window.session.visible,window.session.query.clone(),window.visibility_intent),before);
    let logs = state.activity.entries(&json!({"commandID":"owned-[redacted]","errorsOnly":true}));
    assert_eq!(logs[0]["message"], "A newer palette action superseded this request.");
    assert_eq!(logs[0]["traceID"], trace.unwrap());
    assert!(!state.activity.counts.contains_key(&command.id));
}
#[test]
fn prepared_local_catalog_reuses_index_and_separates_ranking_usage_and_projection() {
    let (_temp, mut state, mut window, _ui, mut a) = private_fixture("cache.a", false);
    a.title = "Owned cache".into();
    let mut b = a.clone(); b.id = "cache.b".into();
    let mut config = serde_json::to_value(state.configuration.current()).unwrap();
    config["commands"] = json!([a, b]);
    state.configuration.save(&config).unwrap();
    window.session.set_query("palette stays");
    let palette = (window.session.query.clone(), window.session.results_revision(), window.session.selection);
    let ids = |commands: Vec<rustcast_core::Command>| commands.into_iter().map(|c|c.id).collect::<Vec<_>>();
    assert_eq!(ids(system::local(&state,"Owned cache",0)), ["cache.a", "cache.b"]);
    let work = crate::catalog::preparation_work();
    for _ in 0..1000 { assert_eq!(ids(system::local(&state,"Owned cache",0)), ["cache.a", "cache.b"]); }
    let after = crate::catalog::preparation_work();
    assert_eq!((after.0-work.0,after.1-work.1,after.2-work.2), (0,0,1000));
    assert_eq!((window.session.query.clone(),window.session.results_revision(),window.session.selection),palette);
    state.ranking.record_success("cache.b", 1_700_000_000.).unwrap();
    assert_eq!(ids(system::local(&state,"Owned cache",0)), ["cache.b", "cache.a"]);
    let ranked=crate::catalog::preparation_work();
    assert_eq!((ranked.0-after.0,ranked.1-after.1),(0,1));
    for _ in 0..1000 { assert_eq!(ids(system::local(&state,"Owned cache",0)), ["cache.b", "cache.a"]); }
    let repeated=crate::catalog::preparation_work();
    assert_eq!((repeated.0,repeated.1),(ranked.0,ranked.1));
    state.ranking.reset().unwrap();
    assert_eq!(ids(system::local(&state,"Owned cache",0)), ["cache.a", "cache.b"]);
    assert_eq!(crate::catalog::preparation_work().1,ranked.1+1);
    state.activity.counts.insert("cache.b".into(),7); state.activity.usage_revision+=1;
    assert_eq!(system::local(&state,"",0)[0].id,"cache.b");
    assert_eq!(ids(system::local(&state,"Owned cache",0)), ["cache.a", "cache.b"]);
    let before_theme=crate::catalog::preparation_work();
    config["theme"]=json!("graphite"); state.configuration.save(&config).unwrap();
    assert_eq!(ids(system::local(&state,"Owned cache",0)), ["cache.a", "cache.b"]);
    let after_theme=crate::catalog::preparation_work();
    assert_eq!((after_theme.0,after_theme.1),(before_theme.0,before_theme.1));
    config["commands"][0]["title"]=json!("Changed cache");
    config["commands"][0]["enabled"]=json!(false);
    state.configuration.save(&config).unwrap();
    assert_eq!(ids(system::local(&state,"Owned cache",0)),["cache.b"]);
    let changed=crate::catalog::preparation_work(); assert_eq!(changed.0,after_theme.0+1);
    let mut app=b; app.id="app.owned-cache".into(); app.kind="application".into(); app.target="/owned/Cache.app".into();
    state.applications=vec![app]; state.applications_revision+=1;
    assert_eq!(ids(system::local(&state,"Owned cache",0)),["cache.b","app.owned-cache"]);
    assert_eq!(crate::catalog::preparation_work().0,changed.0+1);
    config["commands"][1]["target"]=json!("/owned/new-script");
    config["commands"][1]["arguments"]=json!(["literal {query}","NUL\u{0}tail"]);
    state.configuration.save(&config).unwrap();
    let literal=system::local(&state,"Owned cache",1).remove(0);
    assert_eq!(literal.id,"cache.b");
    assert_eq!(literal.target,"/owned/new-script");
    assert_eq!(literal.arguments,["literal {query}","NUL\u{0}tail"]);
}

#[test]
fn prepared_palette_theme_does_not_reindex_reprepare_or_reevaluate() {
    let (_temp, mut state, mut window, ui, _) = private_fixture("cache.palette", false);
    window.session.set_query("Owned private"); window.publish(&state,&ui);
    let results=window.session.results_revision();
    let selected=window.session.selected().unwrap().id.clone();
    let work=crate::catalog::preparation_work();
    let revision=state.configuration.revision;
    let mut config=serde_json::to_value(state.configuration.current()).unwrap();
    config["theme"]=json!("graphite"); state.configuration.save(&config).unwrap();
    assert!(state.configuration.revision>revision);
    window.sync(&state);
    assert_eq!(window.config_revision,state.configuration.revision);
    assert_eq!(crate::catalog::preparation_work(),work);
    assert_eq!(window.session.results_revision(),results);
    assert_eq!(window.session.selected().unwrap().id,selected);
    config["commands"][0]["title"]=json!("Different command"); state.configuration.save(&config).unwrap();
    window.sync(&state);
    assert!(window.session.results_revision()>results);
    assert!(window.session.selected().is_none());
    assert_eq!(crate::catalog::preparation_work().0,work.0+1);
}

#[test]
fn clipboard_pipeline_unicode_latest_identity_hidden_mutation_and_reentry_use_real_owner() {
    let (_temp, mut state, mut window, ui, _) = private_fixture("script.fixture", false);
    let mut config = serde_json::to_value(state.configuration.current()).unwrap();
    config["clipboard"]["limit"] = json!(500);
    state.configuration.save(&config).unwrap();
    let settings = state.configuration.current().clipboard.clone();
    let large = "Über CAFÉ Ω ".repeat(600);
    let items = (0..200).map(|i| json!({"id":format!("raycast-fixture-{i}"),
        "text":format!("Fixture {i}: {large}"),"date":crate::clipboard_history::now(),"source":"fixture"})).collect::<Vec<_>>();
    let mut store =
        crate::clipboard_history::Store::open(&state.configuration.paths.root, &settings).unwrap();
    for batch in items.chunks(100) {
        store.import(&json!(batch), &settings, || true).unwrap();
    }
    state.clipboard = Some(store);
    show_clipboard(&state, &mut window, &ui, Instant::now()).unwrap();
    window.publish(&state, &ui);
    assert_eq!(window.session.row_count(), 200);
    ui.query("CAFÉ");
    process_inputs(&mut state, &mut window, &ui);
    window.publish(&state, &ui);
    let reference = state
        .clipboard
        .as_ref()
        .unwrap()
        .items
        .iter()
        .filter(|clip| {
            objc2_foundation::NSString::from_str(&clip.text).localizedCaseInsensitiveContainsString(
                &objc2_foundation::NSString::from_str("CAFÉ"),
            )
        })
        .map(|clip| format!("clipboard:{}", clip.id))
        .collect::<Vec<_>>();
    assert_eq!(
        window
            .session
            .rows()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>(),
        reference
    );
    assert!(window.session.activated_id.is_none());
    let old = ui.view();
    ui.query("missing stale clipboard query");
    ui.action(Action::Activate {
        id: old.rows[0].id.clone(),
        catalog_revision: old.catalog_revision,
        epoch: old.epoch,
    });
    process_inputs(&mut state, &mut window, &ui);
    window.publish(&state, &ui);
    assert_eq!(window.session.row_count(), 0);
    assert_eq!(state.execution.count(), 0);
    assert!(window.session.activated_id.is_none());
    for i in 0..1000 {
        ui.query(&format!("missing {i}"));
    }
    ui.query("Fixture 42:");
    process_inputs(&mut state, &mut window, &ui);
    window.publish(&state, &ui);
    assert_eq!(window.session.query, "Fixture 42:");
    assert_eq!(window.session.row_count(), 1);
    let selected = window.session.selected().unwrap().id.clone();
    assert!(window
        .session
        .selected()
        .unwrap()
        .target
        .starts_with("Fixture 42:"));
    assert_eq!(ui.view().query, "Fixture 42:");
    state
        .clipboard
        .as_mut()
        .unwrap()
        .capture(
            crate::clipboard_history::Payload::Text("Fixture 42: new".into()),
            "fixture".into(),
            crate::clipboard_history::now(),
            &settings,
            || true,
        )
        .unwrap();
    window.publish(&state, &ui);
    assert_eq!(window.session.row_count(), 2);
    assert_eq!(window.session.selected().unwrap().id, selected);
    assert_eq!(
        ui.view().clipboard.unwrap().selected_id.as_deref(),
        selected.strip_prefix("clipboard:")
    );

    let retained_ids=window.filtered_clips(&state).iter().map(|clip|clip.id.clone()).collect::<Vec<_>>();
    assert_eq!(retained_ids.len(),2);
    let retained_query=window.clipboard_query.clone();
    let retained_selection=window.clipboard_selection;
    let retained_storage=window.filtered_clip_ids.as_ptr();
    window.show(Some("terminal"), Instant::now(), &ui).unwrap();
    window.publish(&state, &ui);
    assert!(!window.session.clipboard_mode);
    assert!(ui.view().clipboard.is_none() && window.clipboard_preview.is_none());
    assert!(window.clipboard_preview_key.is_none());
    assert_eq!(window.filtered_clips(&state).iter().map(|clip|clip.id.clone()).collect::<Vec<_>>(),retained_ids,
        "Commands mode must retain the actual clipboard publication.");
    assert!((0..window.session.catalog.len()).all(|i| !window.session.catalog.command(i).id.starts_with("clipboard:")));
    let revision = state.clipboard.as_ref().unwrap().revision;
    let cached_revision = window.clipboard_revision;
    let data = serde_json::to_value(&state.clipboard.as_ref().unwrap().items).unwrap();
    for i in 0..1000 {
        ui.query(&format!("terminal {i}"));
        process_inputs(&mut state, &mut window, &ui);
        window.publish(&state, &ui);
    }
    process_inputs(&mut state, &mut window, &ui);
    window.publish(&state, &ui);
    assert_eq!(state.clipboard.as_ref().unwrap().revision, revision);
    assert_eq!(window.clipboard_revision, cached_revision);
    assert!(serde_json::to_value(&state.clipboard.as_ref().unwrap().items).unwrap() == data);
    assert!(ui.view().clipboard.is_none() && window.clipboard_preview.is_none());
    assert!(window.clipboard_preview_key.is_none());
    assert_eq!(window.filtered_clips(&state).iter().map(|clip|clip.id.clone()).collect::<Vec<_>>(),retained_ids);
    assert_eq!(window.clipboard_query,retained_query);
    assert_eq!(window.clipboard_selection,retained_selection);
    assert_eq!(window.filtered_clip_ids.as_ptr(),retained_storage);
    assert_eq!(window.clipboard_data_revision,revision);
    assert!((0..window.session.catalog.len()).all(|i| !window.session.catalog.command(i).id.starts_with("clipboard:")));

    state
        .clipboard
        .as_mut()
        .unwrap()
        .delete(selected.strip_prefix("clipboard:"), &settings, || true)
        .unwrap();
    assert_eq!(state.clipboard.as_ref().unwrap().revision, revision + 1);
    let deleted = selected.strip_prefix("clipboard:").unwrap();
    assert!(!state.clipboard.as_ref().unwrap().items.iter().any(|clip| clip.id == deleted));
    let disk: Vec<rustcast_core::Clip> = serde_json::from_slice(&std::fs::read(state.configuration.paths.root.join("clipboard.json")).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(&disk).unwrap(), serde_json::to_value(&state.clipboard.as_ref().unwrap().items).unwrap());
    assert!(!disk.iter().any(|clip| clip.id == deleted));
    window.publish(&state, &ui);
    assert!(ui.view().clipboard.is_none());
    assert_eq!(window.clipboard_revision, cached_revision);
    assert_eq!(window.clipboard_data_revision,revision+1);
    assert_eq!(window.filtered_clips(&state).iter().map(|clip|clip.id.clone()).collect::<Vec<_>>(),retained_ids.iter().filter(|id|id.as_str()!=deleted).cloned().collect::<Vec<_>>());
    assert_eq!(window.clipboard_query,retained_query);
    assert_eq!(window.clipboard_selection,retained_selection,"Hidden pruning preserves the original inactive selection index; re-entry resets it.");
    assert!(window.clipboard_preview.is_none() && window.clipboard_preview_key.is_none());
    assert!((0..window.session.catalog.len()).all(|i| !window.session.catalog.command(i).id.starts_with("clipboard:")));
    show_clipboard(&state, &mut window, &ui, Instant::now()).unwrap();
    window.publish(&state, &ui);
    assert_eq!(window.session.row_count(), 200);
    assert!(!window.session.rows().any(|row| row.id == selected));
    let retained = serde_json::to_value(&state.clipboard.as_ref().unwrap().items).unwrap();
    window.show(Some("terminal"), Instant::now(), &ui).unwrap();
    window.publish(&state, &ui);
    state
        .clipboard
        .as_mut()
        .unwrap()
        .delete(None, &settings, || true)
        .unwrap();
    window.publish(&state, &ui);
    assert!(state.clipboard.as_ref().unwrap().items.is_empty() && ui.view().clipboard.is_none());
    assert!(window.filtered_clips(&state).is_empty());
    assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(state.configuration.paths.root.join("clipboard.json")).unwrap()).unwrap(), json!([]));
    assert!(window.clipboard_preview.is_none() && window.clipboard_preview_key.is_none());
    assert!((0..window.session.catalog.len()).all(|i| !window.session.catalog.command(i).id.starts_with("clipboard:")));
    assert!(!state
        .configuration
        .paths
        .root
        .join("clipboard-assets")
        .exists());
    // The original restores its own retained clips directly. Raycast import
    // deliberately excludes native capture IDs, so restore through the existing
    // owned maintenance transaction rather than weakening import validation.
    state
        .clipboard
        .as_mut()
        .unwrap()
        .update(
            &settings,
            || true,
            |_| Ok(serde_json::from_value(retained).unwrap()),
        )
        .unwrap();
    window.publish(&state,&ui);
    assert!(window.filtered_clips(&state).is_empty(),"Hidden insertion must not repopulate a cleared publication.");
    show_clipboard(&state, &mut window, &ui, Instant::now()).unwrap();
    window.publish(&state, &ui);
    assert_eq!(window.session.row_count(), 200);
    assert_eq!(window.filtered_clips(&state).len(),200);
    assert!(window.clipboard_query.is_empty());
    assert_eq!(window.clipboard_selection,0);
    assert!(!window.session.rows().any(|row| row.id == selected));
}

#[test]
fn clipboard_hidden_publication_prunes_ids_and_refreshes_payload_without_searching() {
    let (_temp,mut state,mut window,ui,_)=private_fixture("script.fixture",false);
    let settings=state.configuration.current().clipboard.clone();
    let mut store=crate::clipboard_history::Store::open(&state.configuration.paths.root,&settings).unwrap();
    store.import(&json!([
        {"id":"raycast-cache-a","text":"matching original A","source":"fixture","date":crate::clipboard_history::now()},
        {"id":"raycast-cache-b","text":"matching original B","source":"fixture","date":crate::clipboard_history::now()}
    ]),&settings,||true).unwrap();
    state.clipboard=Some(store);
    // An unopened clipboard has no previous publication to populate on insert.
    window.publish(&state,&ui);
    assert!(window.filtered_clips(&state).is_empty());
    assert!(window.clipboard_query.is_empty());
    assert_eq!(window.clipboard_selection,0);
    show_clipboard(&state,&mut window,&ui,Instant::now()).unwrap();
    ui.query("matching");
    process_inputs(&mut state,&mut window,&ui);
    window.publish(&state,&ui);
    let old=ui.view();
    let retained=window.filtered_clips(&state).iter().map(|c|c.id.clone()).collect::<Vec<_>>();
    assert_eq!(retained.len(),2);
    let query=window.clipboard_query.clone();
    let selection=window.clipboard_selection;
    window.show(Some("commands query"),Instant::now(),&ui).unwrap();
    window.publish(&state,&ui);
    let replaced=retained[0].clone();
    let mutate=replaced.clone();
    let new_clip=serde_json::from_value::<rustcast_core::Clip>(json!({"id":"raycast-cache-new","text":"matching NEW","source":"fixture","date":crate::clipboard_history::now()})).unwrap();
    state.clipboard.as_mut().unwrap().update(&settings,||true,move|current|{
        let mut clips=current.to_vec();
        clips.iter_mut().find(|c|c.id==mutate).unwrap().text="changed payload Ω".into();
        clips.insert(0,new_clip);
        Ok(clips)
    }).unwrap();
    window.publish(&state,&ui);
    assert_eq!(window.filtered_clips(&state).iter().map(|c|c.id.clone()).collect::<Vec<_>>(),retained);
    assert_eq!(window.filtered_clips(&state)[0].text,"changed payload Ω");
    assert!(!window.filtered_clips(&state).iter().any(|c|c.id=="raycast-cache-new"));
    assert_eq!(window.clipboard_query,query);
    assert_eq!(window.clipboard_selection,selection);
    assert!(ui.view().clipboard.is_none()&&window.clipboard_preview.is_none());
    // Retaining a publication must not make its old activation live again.
    ui.action(Action::Activate{id:old.rows[0].id.clone(),catalog_revision:old.catalog_revision,epoch:old.epoch});
    process_inputs(&mut state,&mut window,&ui);
    window.publish(&state,&ui);
    assert_eq!(state.execution.count(),0);
    assert!(window.session.activated_id.is_none());
    state.clipboard.as_mut().unwrap().delete(Some(&replaced),&settings,||true).unwrap();
    window.publish(&state,&ui);
    assert_eq!(window.filtered_clips(&state).iter().map(|c|c.id.clone()).collect::<Vec<_>>(),retained[1..]);
    assert_eq!(window.clipboard_selection,selection);
    show_clipboard(&state,&mut window,&ui,Instant::now()).unwrap();
    window.publish(&state,&ui);
    assert!(window.clipboard_query.is_empty());
    assert_eq!(window.filtered_clips(&state).len(),2);
    assert!(window.filtered_clips(&state).iter().any(|c|c.id=="raycast-cache-new"));
    let active_selected=window.session.selected().unwrap().id.strip_prefix("clipboard:").unwrap().to_string();
    state.clipboard.as_mut().unwrap().delete(Some(&active_selected),&settings,||true).unwrap();
    window.publish(&state,&ui);
    assert_eq!(window.clipboard_selection,-1,"The original loses selection when a visible selected identity disappears.");
    assert_eq!(window.session.selection,None);
    assert!(ui.view().clipboard.unwrap().selected_id.is_none());
    ui.query("missing unrelated Ω");
    process_inputs(&mut state,&mut window,&ui);
    window.publish(&state,&ui);
    assert!(window.filtered_clips(&state).is_empty());
    assert_eq!(window.clipboard_selection,0,"An empty active query result keeps the original numeric zero, not null.");
    assert_eq!(window.session.selection,None);
}

#[test]
fn clipboard_query_and_data_change_reset_selection_before_the_same_publication() {
    let (_temp,mut state,mut window,ui,_)=private_fixture("script.fixture",false);
    let settings=state.configuration.current().clipboard.clone();
    let mut store=crate::clipboard_history::Store::open(&state.configuration.paths.root,&settings).unwrap();
    store.import(&json!([
        {"id":"raycast-boundary-a","text":"shared first","source":"fixture","date":crate::clipboard_history::now()},
        {"id":"raycast-boundary-b","text":"shared second","source":"fixture","date":crate::clipboard_history::now()}
    ]),&settings,||true).unwrap();
    state.clipboard=Some(store);
    show_clipboard(&state,&mut window,&ui,Instant::now()).unwrap();
    ui.query("shared");process_inputs(&mut state,&mut window,&ui);window.publish(&state,&ui);
    let inserted=serde_json::from_value::<rustcast_core::Clip>(json!({"id":"raycast-boundary-new","text":"shared new","source":"fixture","date":crate::clipboard_history::now()})).unwrap();
    state.clipboard.as_mut().unwrap().update(&settings,||true,move|current|{
        let mut clips=current.to_vec();clips.insert(0,inserted);Ok(clips)
    }).unwrap();
    ui.query("share");process_inputs(&mut state,&mut window,&ui);window.publish(&state,&ui);
    assert_eq!(window.clipboard_query,"share");
    assert_eq!(window.clipboard_selection,0);
    assert_eq!(window.session.selected().unwrap().id,"clipboard:raycast-boundary-new");
    let retained=window.filtered_clip_ids.clone();
    ui.query("shared first"); // not yet consumed when the owner leaves clipboard
    window.show(Some("commands"),Instant::now(),&ui).unwrap();
    process_inputs(&mut state,&mut window,&ui);window.publish(&state,&ui);
    assert_eq!(window.filtered_clip_ids,retained);
    assert_eq!(window.clipboard_query,"share");
    assert!(window.clipboard_preview.is_none()&&window.clipboard_preview_key.is_none());
}

#[test]
fn clipboard_pipeline_twenty_unrelated_atomic_writes_do_not_notify_configuration_owner() {
    use std::sync::atomic::Ordering;
    let (_temp, mut state, _, _, _) = private_fixture("script.fixture", false);
    let file = state.configuration.file.clone();
    let reads = state.configuration.paths.count_reads_for_test(file.clone());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(!state.configuration.reload().unwrap());
    assert_eq!(
        reads.load(Ordering::Relaxed),
        1,
        "real unchanged reload must read bytes"
    );
    let (tx, rx) = mpsc::sync_channel(32);
    let mut observer =
        crate::observe::ConfigurationObserver::start(state.configuration.paths.clone(), tx)
            .unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        crate::control::HostEvent::ConfigurationChanged
    ));
    observer.acknowledge();
    assert!(!state.reload_watched().unwrap());
    let reads_before = reads.load(Ordering::Relaxed);
    assert!(
        reads_before > 1,
        "real initial observer event must reach configuration read"
    );
    let before = state.configuration.snapshot().to_vec();
    for i in 0..20 {
        crate::private_storage::write(
            &state.configuration.paths.root.join("unrelated.json"),
            i.to_string().as_bytes(),
            true,
        )
        .unwrap();
    }
    assert!(matches!(
        rx.recv_timeout(Duration::from_millis(250)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(state.configuration.snapshot(), before);
    assert_eq!(
        reads.load(Ordering::Relaxed),
        reads_before,
        "unrelated writes must not reread configuration bytes"
    );
    let mut changed = serde_json::to_value(state.configuration.current()).unwrap();
    changed["clipboard"]["limit"] = json!(51);
    crate::private_storage::write(&file, &serde_json::to_vec(&changed).unwrap(), true).unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        crate::control::HostEvent::ConfigurationChanged
    ));
    observer.acknowledge();
    assert!(state.reload_watched().unwrap());
    assert!(
        reads.load(Ordering::Relaxed) > reads_before,
        "changed configuration must still reach real read boundary"
    );
    assert_eq!(state.configuration.current().clipboard.limit, 51);
    assert_eq!(
        serde_json::from_slice::<Value>(state.configuration.snapshot()).unwrap(),
        changed
    );
    observer.stop();
}

#[test]
fn clipboard_pipeline_observed_atomic_inplace_invalid_and_identical_recovery_reach_owner_views() {
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    let (_temp, mut state, mut palette, ui, _) = private_fixture("script.fixture", false);
    let mut management = WindowRuntime::new(crate::session::Session::management(vec![]), Default::default(), &state);
    let (tx, rx) = mpsc::sync_channel(32);
    state.events = tx.clone();
    let mut control = install_writer(&mut state);
    let mut observer = crate::observe::ConfigurationObserver::start(state.configuration.paths.clone(), tx).unwrap();
    let apply_observed = |state: &mut SharedState, palette: &mut WindowRuntime, management: &mut WindowRuntime| {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let event = rx.recv_timeout(until.saturating_duration_since(Instant::now())).expect("real configuration vnode event");
            if matches!(event, HostEvent::ConfigurationChanged) { break; }
        }
        observer.acknowledge();
        state.config_dirty = true;
        state.queue_reload(ConfigContext::Watch, Arc::new(||true), false).unwrap();
        state.config_dirty = false;
        settle(state, management);
        palette.publish(state, &ui);
    };
    apply_observed(&mut state, &mut palette, &mut management);
    let mut changed = serde_json::to_value(state.configuration.current()).unwrap();
    changed["commands"].as_array_mut().unwrap().push(json!({"id":"fixture.atomic","title":"Atomic update","kind":"url","target":"https://example.test/atomic"}));
    crate::private_storage::write(&state.configuration.file, &serde_json::to_vec(&changed).unwrap(), true).unwrap();
    apply_observed(&mut state, &mut palette, &mut management);
    assert!(state.configuration.current().commands.iter().any(|command| command.id == "fixture.atomic"));
    assert!(ui.view().rows.iter().any(|row| row.id == "fixture.atomic"));
    changed["commands"].as_array_mut().unwrap().push(json!({"id":"fixture.inplace","title":"In-place update","kind":"url","target":"https://example.test/inplace"}));
    let retained_file = std::fs::File::open(&state.configuration.file).unwrap();
    let inode = retained_file.metadata().unwrap().ino();
    let mut file = std::fs::OpenOptions::new().write(true).truncate(true).open(&state.configuration.file).unwrap();
    file.write_all(&serde_json::to_vec(&changed).unwrap()).unwrap();
    drop(file);
    assert_eq!(std::fs::metadata(&state.configuration.file).unwrap().ino(), inode);
    apply_observed(&mut state, &mut palette, &mut management);
    assert!(state.configuration.current().commands.iter().any(|command| command.id == "fixture.inplace"));
    assert!(ui.view().rows.iter().any(|row| row.id == "fixture.inplace"));
    let valid = std::fs::read(&state.configuration.file).unwrap();
    assert_eq!(state.configuration.snapshot(), valid);
    let revision = state.configuration.revision;
    let rows = ui.view().rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
    crate::private_storage::write(&state.configuration.file, b"not json", true).unwrap();
    apply_observed(&mut state, &mut palette, &mut management);
    assert!(state.configuration.last_error.is_some());
    assert!(ui.view().message.starts_with("Config:"));
    assert_eq!(state.configuration.revision, revision);
    assert_eq!(state.configuration.snapshot(), valid);
    assert_eq!(ui.view().rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(), rows);
    crate::private_storage::write(&state.configuration.file, &valid, true).unwrap();
    apply_observed(&mut state, &mut palette, &mut management);
    assert!(state.configuration.last_error.is_none());
    assert!(ui.view().message.is_empty());
    assert_eq!(state.configuration.revision, revision);
    assert_eq!(state.configuration.snapshot(), valid);
    assert_eq!(std::fs::read(&state.configuration.file).unwrap(), valid);
    assert_eq!(ui.view().rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(), rows);
    observer.stop();
    control.stop();
}

fn private_fixture(
    id: &str,
    provider: bool,
) -> (
    Temp,
    SharedState,
    WindowRuntime,
    Arc<Connection>,
    rustcast_core::Command,
) {
    let (temp, mut state, _, _, _) = setup();
    std::fs::write(
        state.configuration.paths.home.join(".ruycast-test-fixture"),
        b"own-disposable-fixture-v1",
    )
    .unwrap();
    let command:rustcast_core::Command=serde_json::from_value(json!({"id":id,"title":"Owned private fixture","kind":"script","target":"/owned/fake","arguments":if provider{vec!["personal_bridge.py"]}else{vec![]}})).unwrap();
    let mut config = serde_json::to_value(state.configuration.current()).unwrap();
    config["commands"] = json!([command]);
    state.configuration.save(&config).unwrap();
    let (tx, _) = mpsc::sync_channel(32);
    let ui = Connection::new(tx, true);
    let mut window = WindowRuntime::new(
        crate::session::Session::new(vec![command.clone()]),
        Default::default(),
        &state,
    );
    window.session.visible = true;
    window.execution_serial = 7;
    window.publish(&state, &ui);
    (temp, state, window, ui, command)
}
fn private_finished(
    state: &SharedState,
    window: &WindowRuntime,
    ui: &Arc<Connection>,
    command: &rustcast_core::Command,
    payload: Value,
    initial_result: Option<u64>,
) -> crate::execution::Finished<ExecutionContext> {
    let mut capture = rustcast_core::script::Capture::empty();
    capture.stdout.eof = true;
    capture.stdout.valid_utf8 = true;
    crate::execution::Finished {
        id: 1,
        context: ExecutionContext {
            capture_result: None,
            initial_result,
            analytics: None,
            snippet_hide: None,
            destination: ExecutionDestination::Native {
                source: "launcher",
                ui: Arc::downgrade(ui),
                epoch: window.epoch,
                dismissal: ui.latest_dismissal(),
                intent: ui.latest_intent(),
                serial: window.execution_serial,
                input_token: None,
                result_origin: None,
            },
            command: command.clone(),
            file: state.configuration.file.clone(),
            trace: "owned-private-trace".into(),
            logged_id: command.id.clone(),
            log_secrets: Vec::new(),
            started: Instant::now(),
            origin: ExecutionOrigin::Saved,
            requested_key: None,
            _services: Default::default(),
        },
        result: Ok(crate::execution::Outcome::Script(
            crate::supervision::Outcome {
                response: json!({"ok":true,"result":{"stdout":payload.to_string(),"stderr":"","exitCode":0}}),
                capture: Some(capture),
            },
        )),
    }
}
#[test]
fn private_native_choices_detail_and_pinned_plain_success_clear_without_clearing_newer_results() {
    let (_temp, mut state, mut window, ui, command) = private_fixture("provider.fixture", true);
    let payload = json!({"ok":true,"detail":"Owned report","choices":{"title":"Owned choices","placeholder":"Find","items":[{"id":"one","title":"One","subtitle":"","value":"synthetic-secret-choice"}]}});
    let first = private_finished(&state, &window, &ui, &command, payload.clone(), None);
    let mut management = WindowRuntime::new(
        crate::session::Session::new(vec![]),
        Default::default(),
        &state,
    );
    finish_executions(&mut state, &mut window, &mut management, vec![first]);
    let surface = window.results.surface().unwrap();
    assert_eq!(surface.detail.as_deref(), Some("Owned report"));
    assert_eq!(surface.rows.len(), 1);
    assert!(!format!("{surface:?}").contains("synthetic-secret-choice"));
    let token = surface.token;
    let success = private_finished(
        &state,
        &window,
        &ui,
        &command,
        json!({"ok":true}),
        Some(token),
    );
    finish_executions(&mut state, &mut window, &mut management, vec![success]);
    assert!(window.results.token().is_none());
    assert!(ui.result_cancelled(token));
    let proof = {
        let mut c = rustcast_core::script::Capture::empty();
        c.stdout.eof = true;
        c.stdout.valid_utf8 = true;
        c
    };
    let interpreted = crate::script_result::interpret_command(
        &command,
        json!({"ok":true,"result":{"stdout":payload.to_string()}}),
        false,
        crate::script_result::Audience::Native,
        Some(&proof),
    );
    window
        .results
        .present(&command, &state.configuration.file, interpreted, None)
        .unwrap();
    let newer = window.results.token().unwrap();
    let stale = private_finished(
        &state,
        &window,
        &ui,
        &command,
        json!({"ok":true}),
        Some(token),
    );
    finish_executions(&mut state, &mut window, &mut management, vec![stale]);
    assert_eq!(window.results.token(), Some(newer));
    let logs = std::fs::read_to_string(state.configuration.paths.root.join("command-log.json"))
        .unwrap_or_default();
    assert!(!logs.contains("synthetic-secret-choice"));
}

#[test]
fn generic_capture_envelopes_and_bound_followups_never_enter_capture_opener() {
    let (_temp, mut state, mut window, ui, command) = private_fixture("capture.quick-note", false);
    assert!(state.services.is_none());
    let mut management = WindowRuntime::new(
        crate::session::Session::new(vec![]),
        Default::default(),
        &state,
    );
    for payload in [
        json!({"ok":true,"detail":"Owned capture report","result":{"note_path":"/synthetic/never-open.md"}}),
        json!({"ok":true,"choices":{"title":"Owned choices","placeholder":"Select","items":[{"id":"one","title":"One","subtitle":"","value":"literal"}]},"result":{"note_path":"/synthetic/never-open.md"}}),
    ] {
        let completed = private_finished(&state, &window, &ui, &command, payload.clone(), None);
        finish_executions(&mut state, &mut window, &mut management, vec![completed]);
        let surface = window
            .results
            .surface()
            .expect("Generic capture presentation must survive");
        assert!(
            surface.error.is_empty(),
            "Unexpected capture-opener error: {}",
            surface.error
        );
        assert_eq!(surface.detail.as_deref(), payload["detail"].as_str());
        assert_eq!(surface.has_choices, payload["choices"].is_object());
        assert!(window.message.is_empty(), "{}", window.message);
        assert_eq!(
            state.execution.count(),
            0,
            "Generic results must not enqueue an opener"
        );
    }
    let token = window.results.token().unwrap();
    let request = window
        .results
        .prepare(token, Some("one"), &command, &state.configuration.file)
        .unwrap();
    let origin = window
        .results
        .admit(request, &command, &state.configuration.file, |_, _, _| {
            Ok(())
        })
        .unwrap();
    let mut followup = private_finished(
        &state,
        &window,
        &ui,
        &command,
        json!({"ok":true,"result":{"note_path":"/synthetic/never-open-followup.md"}}),
        None,
    );
    let ExecutionDestination::Native { result_origin, .. } = &mut followup.context.destination
    else {
        panic!("owned native destination")
    };
    *result_origin = Some(Arc::new(origin));
    finish_executions(&mut state, &mut window, &mut management, vec![followup]);
    assert!(
        window.results.token().is_none(),
        "Successful bound follow-up must close cleanly"
    );
    assert!(window.message.is_empty(), "{}", window.message);
    assert_eq!(state.execution.count(), 0);
}
#[test]
#[cfg(feature = "test-tools")]
fn capture_native_saved_truth_on_missing_sdk_opener_and_stale_intent_has_no_native_effect() {
    for stale in [false, true] {
        let (_temp, mut state, mut window, ui, command) =
            private_fixture("capture.quick-note", false);
        let (services, _server) = crate::main_services::channel();
        state.services = Some(services);
        let completed = private_finished(
            &state,
            &window,
            &ui,
            &command,
            json!({"ok":true,"note_path":"/owned/never-open.md"}),
            None,
        );
        if stale {
            ui.global_action();
        }
        let mut management = WindowRuntime::new(
            crate::session::Session::new(vec![]),
            Default::default(),
            &state,
        );
        finish_executions(&mut state, &mut window, &mut management, vec![completed]);
        if stale {
            assert_eq!(state.execution.count(), 0);
            continue;
        }
        assert_eq!(state.execution.count(), 1);
        let end = Instant::now() + Duration::from_secs(2);
        let completed = loop {
            let result = state.execution.harvest();
            if !result.is_empty() {
                break result;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        };
        finish_executions(&mut state, &mut window, &mut management, completed);
        assert!(
            window.message.starts_with("Note saved"),
            "{}",
            window.message
        );
        assert_eq!(state.execution.count(), 0);
        window.publish(&state, &ui);
        assert!(ui.view().message.starts_with("Note saved"));
        println!("UI-COPY {}", ui.view().message);
    }
}

#[cfg(feature = "test-tools")]
static PRIVATE_OPEN_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "test-tools")]
static PRIVATE_OPEN_MODE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "test-tools")]
static PRIVATE_OPEN_RELEASE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-tools")]
static PRIVATE_OPEN_URL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
#[cfg(feature = "test-tools")]
unsafe extern "C-unwind" fn private_open(
    _this: *mut objc2::runtime::AnyObject,
    _sel: objc2::runtime::Sel,
    url: *mut objc2_foundation::NSURL,
) -> objc2::runtime::Bool {
    use std::sync::atomic::Ordering;
    PRIVATE_OPEN_CALLS.fetch_add(1, Ordering::SeqCst);
    *PRIVATE_OPEN_URL.lock().unwrap() = unsafe { url.as_ref() }
        .and_then(|url| url.absoluteString())
        .map(|s| s.to_string());
    if PRIVATE_OPEN_MODE.load(Ordering::SeqCst) == 2 {
        let end = Instant::now() + Duration::from_secs(3);
        while !PRIVATE_OPEN_RELEASE.load(Ordering::SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    objc2::runtime::Bool::new(PRIVATE_OPEN_MODE.load(Ordering::SeqCst) != 1)
}

#[test]
#[cfg(feature = "test-tools")]
#[ignore = "Requires unchanged complete owned workspace SDK shim installed before process startup"]
fn capture_native_owned_sdk_success_failure_and_unknown_open_use_one_granted_dispatch() {
    use std::sync::atomic::Ordering;
    let fixture_home = std::path::PathBuf::from(
        std::env::var_os("RUYCAST_WORKSPACE_FIXTURE_HOME").expect("owned fixture home"),
    );
    assert!(crate::workspace::fixture_installed(&fixture_home));
    use objc2::ClassType;
    let method = objc2_app_kit::NSWorkspace::class()
        .instance_method(objc2::sel!(openURL:))
        .unwrap();
    let old = unsafe {
        method.set_implementation(std::mem::transmute::<
            unsafe extern "C-unwind" fn(
                *mut objc2::runtime::AnyObject,
                objc2::runtime::Sel,
                *mut objc2_foundation::NSURL,
            ) -> objc2::runtime::Bool,
            objc2::runtime::Imp,
        >(private_open))
    };
    struct Restore<'a>(&'a objc2::runtime::Method, objc2::runtime::Imp);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            PRIVATE_OPEN_RELEASE.store(true, Ordering::SeqCst);
            unsafe {
                self.0.set_implementation(self.1);
            }
        }
    }
    let _restore = Restore(method, old);
    for mode in 0..3 {
        PRIVATE_OPEN_CALLS.store(0, Ordering::SeqCst);
        PRIVATE_OPEN_MODE.store(mode, Ordering::SeqCst);
        PRIVATE_OPEN_RELEASE.store(false, Ordering::SeqCst);
        *PRIVATE_OPEN_URL.lock().unwrap() = None;
        let (_temp, mut state, mut window, ui, command) =
            private_fixture("capture.quick-note", false);
        let (events, receive) = mpsc::sync_channel(32);
        state.events = events.clone();
        state.execution =
            crate::execution::Manager::new(state.configuration.paths.home.clone(), events);
        let (services, server) = crate::main_services::channel();
        state.services = Some(services);
        let note = state.configuration.paths.home.join("Owned Ω &?# +.md");
        let expected = crate::script_result::capture_open_request(
            &command,
            &json!({"ok":true,"result":{"note_path":note}}),
        )
        .unwrap()
        .unwrap()
        .target;
        let completed = private_finished(
            &state,
            &window,
            &ui,
            &command,
            json!({"ok":true,"note_path":note}),
            None,
        );
        let mut management = WindowRuntime::new(
            crate::session::Session::new(vec![]),
            Default::default(),
            &state,
        );
        finish_executions(&mut state, &mut window, &mut management, vec![completed]);
        assert_eq!(state.execution.count(), 1);
        let end = Instant::now() + Duration::from_secs(3);
        let completed = loop {
            for event in receive.try_iter() {
                match event {
                    HostEvent::OpenCommit(commit) => {
                        handle_open_grant(&mut state, &mut window, &mut management, commit, false)
                    }
                    HostEvent::OpenHide(commit) => {
                        handle_open_grant(&mut state, &mut window, &mut management, commit, true)
                    }
                    _ => {}
                }
            }
            server.drain_with_hide(
                || panic!("private result must not read clipboard"),
                |token| ui.acknowledge_open_hide(&token.key, || true),
            );
            if mode == 2 && PRIVATE_OPEN_CALLS.load(Ordering::SeqCst) == 1 {
                ui.global_action();
            }
            let completed = state.execution.harvest();
            if !completed.is_empty() {
                break completed;
            }
            assert!(Instant::now() < end, "owned open did not complete");
            std::thread::sleep(Duration::from_millis(2));
        };
        if mode == 2 {
            let context = &completed[0].context;
            assert!(context.capture_result.is_some());
            let Ok(crate::execution::Outcome::Workspace(opened)) = &completed[0].result else {
                panic!("owned workspace outcome")
            };
            assert_eq!(opened.response["code"], "open-outcome-unknown");
            assert_eq!(opened.response["actionCompleted"], true);
        }
        PRIVATE_OPEN_RELEASE.store(true, Ordering::SeqCst);
        finish_executions(&mut state, &mut window, &mut management, completed);
        assert_eq!(PRIVATE_OPEN_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(
            PRIVATE_OPEN_URL.lock().unwrap().as_deref(),
            Some(expected.as_str())
        );
        if mode == 0 {
            assert!(window.message.is_empty());
        } else if mode == 1 {
            assert!(window.message.starts_with("Note saved"));
        }
        let end = Instant::now() + Duration::from_secs(2);
        while state.execution.live_workers() > 0 {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(state.execution.shutdown().is_empty());
    }
}

#[test]
fn clipboard_pipeline_admitted_maintenance_waits_for_held_capture() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    const NAME: &str =
        "runtime::inline_tests::clipboard_pipeline_admitted_maintenance_waits_for_held_capture";
    const CHILD: &str = "RUYCAST_HELD_CAPTURE_CHILD";
    if let Some(base) = std::env::var_os(CHILD).map(std::path::PathBuf::from) {
        assert!(base.is_absolute());
        assert_eq!(
            std::fs::read(base.join(".ruycast-test-fixture")).unwrap(),
            b"own-disposable-fixture-v1"
        );
        let (_temp, mut state, mut window, _, _) = setup();
        let profile = base.join("p");
        state.configuration = crate::profile::ConfigStore::open(ProfilePaths::new(
            profile.clone(),
            base.clone(),
            None,
        ))
        .unwrap();
        let (tx, rx) = mpsc::sync_channel(32);
        state.events = tx.clone();
        let mut server = crate::control::ControlServer::start(
            &profile,
            tx.clone(),
            crate::control::Limits {
                dispatch: Duration::from_secs(10),
                ..Default::default()
            },
        )
        .unwrap();
        state.writer = Some(crate::configuration_jobs::Writer::new(
            server.profile_lease(),
            tx,
            None,
        ));
        let mut management = WindowRuntime::new(
            crate::session::Session::new(vec![]),
            Default::default(),
            &state,
        );
        let marker = base.join("entered");
        let release = base.join("release");
        let port = crate::clipboard_history::CapturePort::default();
        let mut completed = Vec::new();
        for operation in [
            "clipboard.import",
            "clipboard.delete",
            "reload",
            "clipboard.clear",
        ] {
            std::fs::write(&marker, b"unarmed").unwrap();
            let _ = std::fs::remove_file(&release);
            let settings = state.configuration.current().clipboard.clone();
            port.publish(&settings, false);
            let mut store = state.clipboard.take().unwrap_or_else(|| {
                crate::clipboard_history::Store::open(&profile, &settings).unwrap()
            });
            store.delete(None, &settings, || true).unwrap();
            store
                .capture(
                    crate::clipboard_history::Payload::Text("baseline".into()),
                    "fixture".into(),
                    crate::clipboard_history::now(),
                    &settings,
                    || true,
                )
                .unwrap();
            let baseline = store.items[0].id.clone();
            let capture = format!("capture before {operation}");
            let text = capture.clone();
            let owned_settings = settings.clone();
            let lease = port.acquire().unwrap();
            std::fs::remove_file(&marker).unwrap();
            let worker = std::thread::spawn(move || {
                let _lease = lease;
                let receipt = store
                    .capture(
                        crate::clipboard_history::Payload::Text(text),
                        "fixture".into(),
                        crate::clipboard_history::now(),
                        &owned_settings,
                        || true,
                    )
                    .unwrap();
                assert!(receipt.durable);
                store
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            while !marker.exists() {
                assert!(
                    !worker.is_finished(),
                    "capture failed to enter existing fsync shim"
                );
                assert!(Instant::now() < deadline, "capture did not reach fsync");
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(!worker.is_finished());
            assert!(!release.exists());
            assert!(port.acquire().is_none());
            let disk: Value =
                serde_json::from_slice(&std::fs::read(profile.join("clipboard.json")).unwrap())
                    .unwrap();
            assert!(disk
                .as_array()
                .unwrap()
                .iter()
                .any(|clip| clip["text"] == capture));
            if operation == "reload" {
                let mut config = serde_json::to_value(state.configuration.current()).unwrap();
                config["clipboard"]["limit"] = json!(100);
                crate::private_storage::write(
                    &state.configuration.file,
                    &serde_json::to_vec(&config).unwrap(),
                    true,
                )
                .unwrap();
            }
            let request = match operation {
                "clipboard.import" => {
                    json!({"op":operation,"items":[{"id":"raycast-admitted","text":"imported","date":crate::clipboard_history::now(),"source":"fixture"}]})
                }
                "clipboard.delete" => json!({"op":operation,"id":baseline}),
                _ => json!({"op":operation}),
            };
            let socket = profile.join("control.sock");
            let client = std::thread::spawn(move || {
                let mut socket = UnixStream::connect(socket).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(8)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                writeln!(socket, "{request}").unwrap();
                let mut response = String::new();
                BufReader::new(socket).read_line(&mut response).unwrap();
                serde_json::from_str::<Value>(&response).unwrap()
            });
            let envelope = loop {
                match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                    HostEvent::Request(envelope) => break envelope,
                    _ => continue,
                }
            };
            assert_eq!(envelope.value["op"], operation);
            assert!(envelope.context.is_live());
            assert!(
                !worker.is_finished(),
                "admission must occur while capture is held"
            );
            assert!(port.acquire().is_none());
            assert!(!release.exists());
            std::fs::write(&release, b"release own capture").unwrap();
            state.clipboard = Some(worker.join().unwrap());
            assert!(port.acquire().is_some());
            if operation == "reload" {
                let context = envelope.context.clone();
                state
                    .queue_reload(
                        ConfigContext::Cli(envelope),
                        Arc::new(move || context.is_live()),
                        false,
                    )
                    .unwrap();
                settle(&mut state, &mut management);
                assert_eq!(state.configuration.current().clipboard.limit, 100);
            } else {
                clipboard_request(&mut state, &mut window, None, envelope);
            }
            let response = client.join().unwrap();
            assert_eq!(response["ok"], true, "{operation}: {response}");
            let store = state.clipboard.as_ref().unwrap();
            let disk: Value =
                serde_json::from_slice(&std::fs::read(profile.join("clipboard.json")).unwrap())
                    .unwrap();
            assert_eq!(disk, serde_json::to_value(&store.items).unwrap());
            let mut texts = store
                .items
                .iter()
                .map(|clip| clip.text.clone())
                .collect::<Vec<_>>();
            texts.sort();
            let mut expected = match operation {
                "clipboard.import" => vec!["baseline".to_string(), capture, "imported".to_string()],
                "clipboard.delete" => vec![capture],
                "reload" => vec!["baseline".to_string(), capture],
                _ => vec![],
            };
            expected.sort();
            assert_eq!(texts, expected);
            assert!(!profile.join("clipboard-assets").exists());
            completed.push(operation);
        }
        server.stop();
        std::fs::write(
            base.join("results.json"),
            serde_json::to_vec(&completed).unwrap(),
        )
        .unwrap();
        return;
    }
    let parent = std::path::PathBuf::from(
        std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("explicit owned fixture parent"),
    );
    assert!(parent.is_absolute() && parent.is_dir());
    let base = parent.join(format!("h{:x}", std::process::id()));
    std::fs::create_dir(&base).unwrap();
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _cleanup = Temp(base.clone());
    std::fs::write(
        base.join(".ruycast-test-fixture"),
        b"own-disposable-fixture-v1",
    )
    .unwrap();
    std::fs::write(base.join("entered"), b"unarmed").unwrap();
    let dylib = base.join("sync.dylib");
    let compile = std::process::Command::new("/usr/bin/cc")
        .args(["-Wall", "-Wextra", "-Werror", "-dynamiclib"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fcntl_sync_shim.c"))
        .arg("-o")
        .arg(&dylib)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let log = base.join("child.log");
    let output = std::fs::File::create(&log).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env(CHILD, &base)
        .env("HOME", &base)
        .env("TMPDIR", &base)
        .env("RUYCAST_NATIVE_TEST_FIXTURE_DIR", &base)
        .env("CARGO_INCREMENTAL", "0")
        .env("DYLD_INSERT_LIBRARIES", &dylib)
        .env("RUYCAST_TEST_FSYNC_DIRECTORY", base.join("p"))
        .env("RUYCAST_TEST_FSYNC_MARKER", base.join("entered"))
        .env("RUYCAST_TEST_FSYNC_RELEASE", base.join("release"))
        .env("RUYCAST_TEST_FSYNC_DELAY_ONLY", "1")
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!(
                "owned child timeout: {}",
                std::fs::read_to_string(&log).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.success(),
        "owned child {status}: {}",
        std::fs::read_to_string(&log).unwrap()
    );
    let results: Vec<String> =
        serde_json::from_slice(&std::fs::read(base.join("results.json")).unwrap()).unwrap();
    assert_eq!(
        results,
        [
            "clipboard.import",
            "clipboard.delete",
            "reload",
            "clipboard.clear"
        ]
    );
    println!("owned held-capture admission and owner results: {results:?}");
}
