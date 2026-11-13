//! Main-owned AppKit service adapter, shared by GUI and headless entrypoints.
use objc2::{rc::autoreleasepool, MainThreadMarker};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use ruycast_native::{main_services, profile::ProfilePaths};
use std::rc::Rc;
use std::sync::{mpsc, Arc};

/// Timer callbacks only mark work and wake the source. The source retires the
/// completed timer and rearms outside its callback stack, including common modes.
pub struct ClipboardMonitor {
    main:MainThreadMarker,
    clipboard:Rc<Clipboard>,
    port:ruycast_native::clipboard_history::CapturePort,
    events:mpsc::SyncSender<ruycast_native::control::HostEvent>,
    due:Rc<std::cell::Cell<bool>>,
    timer:Option<crate::main_loop::Timer>,
    wake:Option<Arc<dyn Fn()+Send+Sync>>,
}
impl ClipboardMonitor {
    pub fn new(main:MainThreadMarker,clipboard:Rc<Clipboard>,port:ruycast_native::clipboard_history::CapturePort,events:mpsc::SyncSender<ruycast_native::control::HostEvent>)->Self{
        Self{main,clipboard,port,events,due:Rc::new(std::cell::Cell::new(false)),timer:None,wake:None}
    }
    pub fn start(&mut self,wake:Arc<dyn Fn()+Send+Sync>)->Result<(),String>{self.wake=Some(wake);self.arm()}
    fn arm(&mut self)->Result<(),String>{
        let due=self.due.clone();let wake=self.wake.clone().ok_or("Clipboard observer source unavailable.")?;
        self.timer=Some(crate::main_loop::Timer::new_common(self.main,0.4,move||{due.set(true);wake();})?);Ok(())
    }
    pub fn pump(&mut self)->Result<(),String>{if self.due.replace(false){self.timer.take();self.clipboard.poll(&self.port,&self.events);self.arm()?;}Ok(())}
    pub fn stop(&mut self){self.timer.take();self.wake=None;self.due.set(false);}
}
impl Drop for ClipboardMonitor {fn drop(&mut self){self.stop();}}

#[cfg(feature="test-tools")]
pub fn shared_fixture_board()->Option<objc2::rc::Retained<NSPasteboard>>{unsafe{let symbol=libc::dlsym(libc::RTLD_DEFAULT,c"ruycast_clipboard_history_fixture_board".as_ptr());if symbol.is_null(){return None}let provider:unsafe extern "C" fn()->*mut NSPasteboard=std::mem::transmute(symbol);let board=objc2::rc::Retained::retain(provider())?;if !board.name().to_string().starts_with("ruycast-clipboard-history-fixture-"){return None}Some(board)}}
pub struct Clipboard {
    attribution_uncertain:std::cell::Cell<bool>,
    observation: std::cell::RefCell<(Option<isize>,Option<i32>)>,
    history_board: std::cell::RefCell<Option<objc2::rc::Retained<NSPasteboard>>>,
    _main: MainThreadMarker,
    fixture: bool,
    own_write: Rc<ruycast_native::clipboard_ownership::OwnWrites>,
    #[cfg(feature = "test-tools")]
    home: std::path::PathBuf,
}
impl Clipboard {
    pub fn new(
        main: MainThreadMarker,
        paths: &ProfilePaths,
        fixture: bool,
    ) -> Result<Self, String> {
        if fixture {
            #[cfg(feature = "test-tools")]
            {
                ruycast_native::applications::fixture_roots(paths)?;
            }
            #[cfg(not(feature = "test-tools"))]
            {
                return Err("Clipboard fixtures require test tools.".into());
            }
        }
        #[cfg(not(feature = "test-tools"))]
        let _ = paths;
        Ok(Self {
            attribution_uncertain:std::cell::Cell::new(false),
            observation: Default::default(),
            history_board: Default::default(),
            _main: main,
            fixture,
            own_write: Rc::new(ruycast_native::clipboard_ownership::OwnWrites::default()),
            #[cfg(feature = "test-tools")]
            home: paths.home.clone(),
        })
    }
    fn new_history_board(&self)->objc2::rc::Retained<NSPasteboard>{
        if self.fixture {
            #[cfg(feature="test-tools")]
            if let Some(board)=shared_fixture_board(){return board}
            NSPasteboard::pasteboardWithUniqueName()
        }else{NSPasteboard::generalPasteboard()}
    }
    pub fn watch(self:&Rc<Self>)->ruycast_native::workspace_activation::Watch {
        let center=if self.fixture{objc2_foundation::NSNotificationCenter::new()}else{objc2_app_kit::NSWorkspace::sharedWorkspace().notificationCenter()};
        let weak=Rc::downgrade(self);let failed=Rc::downgrade(self);
        ruycast_native::workspace_activation::Watch::new(self._main,center,Rc::new(||Some(1)),Rc::new(move|_|{if let Some(owner)=weak.upgrade(){owner.attribution_uncertain.set(true);}}),Rc::new(move||{if let Some(owner)=failed.upgrade(){owner.attribution_uncertain.set(true);}}))
    }
    /// Main-only bounded snapshot. Hidden tests must own their board and source;
    /// they never consult the real foreground process or the general pasteboard.
    pub fn poll(&self,port:&ruycast_native::clipboard_history::CapturePort,events:&mpsc::SyncSender<ruycast_native::control::HostEvent>){
        self.poll_with_application(port,events,||if self.fixture{None}else{objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()});
    }
    fn poll_with_application(&self,port:&ruycast_native::clipboard_history::CapturePort,events:&mpsc::SyncSender<ruycast_native::control::HostEvent>,application:impl Fn()->Option<objc2::rc::Retained<objc2_app_kit::NSRunningApplication>>){
        use ruycast_native::clipboard_history::{Payload,Capture};
        let Some(policy)=port.policy() else{return};
        if policy.paused||!policy.settings.enabled||port.admission_busy()||self.own_write.active(){return}
        if self.fixture {
            #[cfg(feature="test-tools")]
            if std::env::var("RUYCAST_NATIVE_TEST_CLIPBOARD").as_deref()!=Ok("named")||!ruycast_native::workspace::fixture_installed(&self.home){return}
            #[cfg(not(feature="test-tools"))]
            return;
        }
        let board=self.history_board.borrow_mut().get_or_insert_with(||self.new_history_board()).clone();
        let count=board.changeCount();let mut observed=self.observation.borrow_mut();
        // Settle activation uncertainty once, but unchanged ordinary polls need
        // neither foreground attribution nor clipboard payload reads.
        if observed.0==Some(count)&&!self.attribution_uncertain.get(){return}
        let app=application();
        let pid=app.as_ref().map(|a|a.processIdentifier());
        let uncertain=self.attribution_uncertain.replace(false);
        let previous=observed.0.replace(count);let old_pid=std::mem::replace(&mut observed.1,pid);
        if uncertain||previous.is_none()||previous==Some(count)||old_pid!=pid||!self.own_write.allows_capture(count){return}
        if board.types().is_some_and(|types|types.iter().any(|t|["org.nspasteboard.ConcealedType","org.nspasteboard.TransientType","org.nspasteboard.AutoGeneratedType"].contains(&t.to_string().as_str()))){return}
        let source=app.as_ref().and_then(|a|a.bundleIdentifier()).map(|s|s.to_string()).unwrap_or_else(||if self.fixture{"fixture".into()}else{"unknown".into()});
        if policy.settings.excluded_apps.contains(&source){return}
        let Some(lease)=port.acquire()else{observed.0=previous;observed.1=old_pid;return};
        // The first policy clone may precede a complete settings adoption.
        // The admitted lease now excludes the next writer's metadata phase.
        let Some(policy)=port.policy()else{return};
        if policy.paused||!policy.settings.enabled||policy.settings.excluded_apps.contains(&source){return}
        let payload=(||->Result<Option<Payload>,String>{
            use objc2::{ClassType,runtime::AnyObject};
            use objc2_foundation::{NSArray,NSDictionary,NSNumber,NSURL};
            let classes=NSArray::from_slice(&[NSURL::class()]);
            let only_files=NSNumber::numberWithBool(true);
            let options=NSDictionary::<objc2_foundation::NSString,AnyObject>::from_slices(&[unsafe{objc2_app_kit::NSPasteboardURLReadingFileURLsOnlyKey}],&[only_files.as_ref()]);
            let mut urls=vec![];
            if let Some(values)=unsafe{board.readObjectsForClasses_options(&classes,Some(&options))}{
                if values.count()>1000{return Err("Clipboard file list exceeds the supported size.".into())}
                for value in values {let url=value.downcast::<NSURL>().map_err(|_|"Invalid clipboard file reference.")?;let raw=url.absoluteString().ok_or("Invalid clipboard file reference.")?;if raw.length()>16384{return Err("Clipboard file list exceeds the supported size.".into())}urls.push(ruycast_native::native_text::repaired(&raw));}
            }
            if !urls.is_empty(){ruycast_native::clipboard_history::validate_files(&urls)?;return Ok(Some(Payload::Files(urls)))}
            for kind in ["public.png","public.tiff"] {if let Some(data)=board.dataForType(&objc2_foundation::NSString::from_str(kind)){if data.is_empty()||data.length()>policy.settings.max_asset_bytes.unwrap_or(15_000_000){return Err("Clipboard image exceeds its storage limit.".into())}return Ok(Some(Payload::Image(data.to_vec())))}}
            Ok(board.stringForType(unsafe{NSPasteboardTypeString}).filter(|s|s.length()<=policy.settings.max_text_bytes).map(|s|ruycast_native::native_text::repaired(&s)).filter(|s|!s.is_empty()&&s.len()<=policy.settings.max_text_bytes).map(Payload::Text))
        })();
        let after=application().map(|a|a.processIdentifier());
        if board.changeCount()!=count||after!=pid{return}
        match payload {
            Ok(Some(payload))=>{let _=events.try_send(ruycast_native::control::HostEvent::ClipboardCapture(Capture{payload,source,at:ruycast_native::clipboard_history::now(),_lease:lease}));},
            Err(error)=>{let _=events.try_send(ruycast_native::control::HostEvent::ClipboardCaptureError(error));},
            Ok(None)=>{}
        }
    }
    #[cfg(feature="test-tools")]
    pub fn fixture(&self,value:&serde_json::Value,port:&ruycast_native::clipboard_history::CapturePort,events:&mpsc::SyncSender<ruycast_native::control::HostEvent>)->Result<serde_json::Value,String>{
        if !self.fixture||std::env::var("RUYCAST_NATIVE_TEST_CLIPBOARD").as_deref()!=Ok("named")||!ruycast_native::workspace::fixture_installed(&self.home){return Err("Owned clipboard fixture boundary required.".into())}
        if value["checkPerformancePoll"]==true{check_owned_poll(&self.home)?;return Ok(serde_json::json!({"ok":true,"ownedPerformancePoll":true}))}
        if let Some(operation)=value["creationBoard"].as_str(){
            if std::env::var("RUYCAST_NATIVE_TEST_CREATION_BOARD").as_deref()!=Ok("shared"){return Err("Explicit shared creation board fixture required.".into())}
            if !["read","clear","nontext"].contains(&operation){return Err("Unknown owned creation board operation.".into())}
            let board=self.creation_fixture_board();
            if operation!="read"{board.clearContents();}
            let binary_type=objc2_foundation::NSString::from_str("org.example.quicklink-fixture");
            if operation=="nontext"&&!board.setData_forType(Some(&objc2_foundation::NSData::with_bytes(&[0,1,2])),&binary_type){return Err("Owned nontext board write failed.".into())}
            return Ok(serde_json::json!({"namedBoard":true,"name":board.name().to_string(),"count":board.changeCount(),"text":board.stringForType(unsafe{NSPasteboardTypeString}).map(|s|ruycast_native::native_text::repaired(&s)),"types":board.types().map(|types|types.iter().map(|s|s.to_string()).collect::<Vec<_>>()).unwrap_or_default(),"nontextData":board.dataForType(&binary_type).map(|data|data.to_vec())}));
        }
        let board=self.history_board.borrow_mut().get_or_insert_with(||self.new_history_board()).clone();
        self.poll(port,events);
        let payload:ruycast_native::clipboard_history::Payload=serde_json::from_value(value["payload"].clone()).map_err(|e|e.to_string())?;
        let item=ruycast_native::snippet_clipboard::PreparedItem::history(self._main,&payload).ok_or("Owned fixture item invalid.")?;
        board.clearContents();if !item.write_to(&board){return Err("Owned fixture write failed.".into())}
        if let Some(private)=value["private"].as_str(){if !["org.nspasteboard.ConcealedType","org.nspasteboard.TransientType","org.nspasteboard.AutoGeneratedType"].contains(&private){return Err("Unknown fixture private type.".into())}board.setString_forType(&objc2_foundation::NSString::from_str("1"),&objc2_foundation::NSString::from_str(private));}
        if value["own"]==true{self.own_write.set(Some(board.changeCount()));}
        if value["activation"]==true{self.attribution_uncertain.set(true);}
        let before=board.changeCount();if value["defer"]!=true{self.poll(port,events);}Ok(serde_json::json!({"namedBoard":true,"count":before,"busy":port.busy.load(std::sync::atomic::Ordering::Acquire)}))
    }
    #[cfg(feature="test-tools")]
    fn creation_fixture_board(&self)->objc2::rc::Retained<NSPasteboard>{
        let cached=self.history_board.borrow().clone();
        if let Some(board)=cached{return board}
        let board=self.new_history_board();
        self.history_board.replace(Some(board.clone()));
        board
    }
    pub fn own_writes(&self) -> Rc<ruycast_native::clipboard_ownership::OwnWrites> {
        self.own_write.clone()
    }
    pub fn read(&self) -> Result<Option<String>, String> {
        self.read_with(read_board, "clipboard-read")
    }
    pub fn prefill(&self) -> Result<Option<String>, String> {
        self.read_with(
            |board| Ok(ruycast_native::creation::prefill(board)),
            "creation-prefill-read",
        )
    }
    fn read_with(
        &self,
        reader: impl Fn(&NSPasteboard) -> Result<Option<String>, String>,
        marker: &str,
    ) -> Result<Option<String>, String> {
        #[cfg(not(feature = "test-tools"))]
        let _ = marker;
        autoreleasepool(|_| {
            if self.fixture {
                #[cfg(feature = "test-tools")]
                {
                    if std::env::var("RUYCAST_NATIVE_TEST_CLIPBOARD").as_deref() != Ok("named")
                        || !ruycast_native::workspace::fixture_installed(&self.home)
                    {
                        return Err("Clipboard fixture boundary is not available; general pasteboard access is disabled.".into());
                    }
                    if std::env::var("RUYCAST_NATIVE_TEST_CREATION_BOARD").as_deref()==Ok("shared") {
                        let board=self.creation_fixture_board();
                        let result=reader(&board);
                        std::fs::write(self.home.join(marker),b"own-shared-named-main-thread\n").map_err(|e|e.to_string())?;
                        return result;
                    }
                    // An actual private named AppKit pasteboard; never fall
                    // back to the user's general pasteboard, even if empty.
                    let board = NSPasteboard::pasteboardWithUniqueName();
                    board.clearContents();
                    struct Release<'a>(&'a NSPasteboard);
                    impl Drop for Release<'_> {
                        fn drop(&mut self) {
                            // SDK NSPasteboard.h declares this selector; generated
                            // bindings omit the legacy oneway method.
                            unsafe {
                                let _: () = objc2::msg_send![self.0, releaseGlobally];
                            }
                        }
                    }
                    let _release = Release(&board);
                    if let Ok(text) = std::env::var("RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT") {
                        if text.len() > 1_048_576 {
                            return Err("Clipboard text exceeds the supported size.".into());
                        }
                        if !board.setString_forType(
                            &objc2_foundation::NSString::from_str(&text),
                            unsafe { NSPasteboardTypeString },
                        ) {
                            return Err("Could not prepare own named pasteboard fixture.".into());
                        }
                    }
                    let result = reader(&board);
                    std::fs::write(self.home.join(marker), b"own-named-main-thread\n")
                        .map_err(|e| e.to_string())?;
                    return result;
                }
                #[cfg(not(feature = "test-tools"))]
                return Err("Clipboard fixture is unavailable.".into());
            }
            // Production ignores all test injection variables.
            reader(&NSPasteboard::generalPasteboard())
        })
    }
}
fn read_board(board: &NSPasteboard) -> Result<Option<String>, String> {
    let Some(text) = board.stringForType(unsafe { NSPasteboardTypeString }) else {
        return Ok(None);
    };
    let text = ruycast_native::native_text::repaired(&text);
    Ok(Some(text))
}

/// Actual production reader/prefill functions, exclusively an own named board.
/// Fault provider symbols are required before native board creation.
#[cfg(feature = "test-tools")]
pub fn check_owned_clipboard(reader: &str, mode: &str) -> Result<serde_json::Value, String> {
    let _main = MainThreadMarker::new().ok_or("Main fixture required")?;
    let mode: i32 = mode.parse().map_err(|_| "Own mode required")?;
    if !(0..=17).contains(&mode) || !matches!(reader, "read" | "prefill") {
        return Err("Unknown own clipboard case".into());
    }
    unsafe fn symbol<T: Copy>(name: &std::ffi::CStr) -> Result<T, String> {
        let ptr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        if ptr.is_null() {
            return Err("Complete owned clipboard provider required".into());
        }
        assert_eq!(
            std::mem::size_of::<T>(),
            std::mem::size_of::<*mut libc::c_void>()
        );
        Ok(unsafe { std::mem::transmute_copy(&ptr) })
    }
    let guard: unsafe extern "C" fn() -> bool =
        unsafe { symbol(c"ruycast_owned_board_general_guard")? };
    let attach: unsafe extern "C" fn(*const NSPasteboard, i32) -> bool =
        unsafe { symbol(c"ruycast_owned_board_attach")? };
    let detach: unsafe extern "C" fn() = unsafe { symbol(c"ruycast_owned_board_detach")? };
    if !unsafe { guard() } {
        return Err("General clipboard abortguard required".into());
    }
    autoreleasepool(|_| {
        let board = NSPasteboard::pasteboardWithUniqueName();
        struct Release<'a> {
            board: &'a NSPasteboard,
            detach: unsafe extern "C" fn(),
        }
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                unsafe {
                    (self.detach)();
                }
                self.board.clearContents();
                unsafe {
                    let _: () = objc2::msg_send![self.board, releaseGlobally];
                }
            }
        }
        let _release = Release {
            board: &board,
            detach,
        };
        board.clearContents();
        assert!(board.setString_forType(
            &objc2_foundation::NSString::from_str("OWN ORIGINAL"),
            unsafe { NSPasteboardTypeString }
        ));
        if !unsafe { attach(&*board, mode) } {
            return Err("Own provider attach failed".into());
        }
        let value = if reader == "read" {
            read_board(&board)?
        } else {
            ruycast_native::creation::prefill(&board)
        };
        unsafe {
            detach();
        }
        let restored = read_board(&board)? == Some("OWN ORIGINAL".into());
        Ok(
            serde_json::json!({"ok":true,"value":value,"restored":restored,"generalBoardAbortGuard":true}),
        )
    })
}

/// The headless owner runs on a worker too. Main waits in CF with no periodic
/// timer; service submissions and final owner completion are its wake sources.
pub fn headless(paths: ProfilePaths, fixture: bool) -> Result<serde_json::Value, String> {
    use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    let main = MainThreadMarker::new().ok_or("Native services require the main thread")?;
    let clipboard = Clipboard::new(main, &paths, fixture)?;
    let (client, services) = main_services::channel();
    services.system.fixture(fixture);
    let paste_client = client.paste();
    let mut paste = crate::native_paste::Service::new(
        main,
        &paths,
        true,
        fixture,
        Arc::new(move || paste_client.notify()),
        clipboard.own_writes(),
    );
    let (tx, rx) = mpsc::sync_channel(32);
    let auto_quit = Rc::new(RefCell::new(if fixture { None } else { Some(crate::native_auto_quit::Monitor::new(main, paths.clone())) }));
    let source_auto_quit = auto_quit.clone();
    let (finish, finished) = mpsc::sync_channel(1);
    let done = Rc::new(Cell::new(false));
    let observed = done.clone();
    let outcome = Rc::new(RefCell::new(None));
    let output = outcome.clone();
    let source = crate::main_loop::Source::new(
        main,
        move || {
            if let Some(monitor)=source_auto_quit.borrow_mut().as_mut(){let _=monitor.pump();}
            services.drain_with_prefill(
                || clipboard.read(),
                |_| Err("No native window is available.".into()),
                || clipboard.prefill(),
            );
            if let Err(error) = paste.pump(services.take_pastes(), |_| {
                Err("No native window is available.".into())
            }) {
                paste.stop();
                panic!("{error}");
            }
            if let Ok(result) = finished.try_recv() {
                if let Some(monitor)=source_auto_quit.borrow_mut().as_mut(){monitor.stop();}
                paste.stop();
                observed.set(true);
                *output.borrow_mut() = Some(result);
            }
        },
        || {},
    )?;
    let signal = source.signal();
    // Attach before starting workers so neither startup nor a quick exit can
    // lose its wake. Client and server share only the thread-safe signal.
    let wake_signal = signal.clone();
    client.attach_wake(Arc::new(move || wake_signal.wake()));
    if let Some(monitor)=auto_quit.borrow_mut().as_mut(){let signal=signal.clone();monitor.start(Arc::new(move||signal.wake()))?;}
    let stop = tx.clone();
    let worker = std::thread::Builder::new()
        .name("ruycast-runtime".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ruycast_native::runtime::run_with_services(
                    paths,
                    tx,
                    rx,
                    None,
                    fixture,
                    Some(client),
                )
            }))
            .unwrap_or_else(|_| Err("Rust runtime worker failed.".into()));
            let _ = finish.send(result);
            signal.wake();
        })
        .map_err(|e| e.to_string())?;
    while !done.get() && !source.failed() {
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 1.0e12, true);
    }
    if source.failed() {
        let _ = stop.send(ruycast_native::control::HostEvent::ServerStopped);
    }
    worker
        .join()
        .map_err(|_| "Rust runtime worker failed.".to_string())?;
    if source.failed() {
        return Err("Main-thread service callback failed.".into());
    }
    let result = outcome
        .borrow_mut()
        .take()
        .ok_or("Missing runtime outcome")?;
    result
}

impl Drop for Clipboard {fn drop(&mut self){if self.fixture {if let Some(board)=self.history_board.get_mut().take(){unsafe{let _:()=objc2::msg_send![&*board,releaseGlobally];}}}}}

#[cfg(feature="test-tools")]
pub fn check_owned_poll(root:&std::path::Path)->Result<(),String>{
    use std::cell::{Cell,RefCell};
    use objc2_foundation::NSString;
    let main=MainThreadMarker::new().ok_or("Owned polling test requires the real main thread")?;
    if !root.is_absolute()||!ruycast_native::workspace::fixture_installed(root){return Err("Explicit attested owned root required".into())}
    let board=NSPasteboard::pasteboardWithUniqueName();
    let clipboard=Clipboard{attribution_uncertain:Cell::new(false),observation:Default::default(),history_board:RefCell::new(Some(board.clone())),_main:main,fixture:true,own_write:Rc::new(Default::default()),home:root.into()};
    let default:serde_json::Value=serde_json::from_str(include_str!("../../config/default.json")).map_err(|e|e.to_string())?;
    let settings=serde_json::from_value(default["clipboard"].clone()).map_err(|e|e.to_string())?;
    let port=ruycast_native::clipboard_history::CapturePort::default();port.publish(&settings,false);
    let (tx,rx)=mpsc::sync_channel(8);
    let reads=Cell::new(0usize);
    let application=||{reads.set(reads.get()+1);Some(objc2_app_kit::NSRunningApplication::currentApplication())};
    board.clearContents();assert!(board.setString_forType(&NSString::from_str("own initial"),unsafe{NSPasteboardTypeString}));
    clipboard.poll_with_application(&port,&tx,application);assert!(!port.busy.load(std::sync::atomic::Ordering::Acquire));
    reads.set(0);for _ in 0..1000{clipboard.poll_with_application(&port,&tx,application)}
    assert_eq!(reads.get(),0,"1000 unchanged production polls must not consult attribution");
    assert!(rx.try_recv().is_err());
    clipboard.attribution_uncertain.set(true);clipboard.poll_with_application(&port,&tx,application);
    assert_eq!(reads.get(),1,"activation uncertainty must settle on unchanged poll");
    clipboard.poll_with_application(&port,&tx,application);assert_eq!(reads.get(),1,"settled attribution repeats work");
    board.clearContents();assert!(board.setString_forType(&NSString::from_str("own stable"),unsafe{NSPasteboardTypeString}));
    clipboard.poll_with_application(&port,&tx,application);
    let capture=rx.try_recv().map_err(|_|"Stable owned payload was lost")?;
    match capture {ruycast_native::control::HostEvent::ClipboardCapture(c)=>assert!(matches!(c.payload,ruycast_native::clipboard_history::Payload::Text(ref s) if s=="own stable")),_=>return Err("Unexpected own event".into())}
    board.clearContents();assert!(board.setString_forType(&NSString::from_str("own uncertain transition"),unsafe{NSPasteboardTypeString}));
    clipboard.attribution_uncertain.set(true);
    clipboard.poll_with_application(&port,&tx,application);
    assert!(!port.admission_busy()&&rx.try_recv().is_err(),"Uncertain changed source must not capture its text");
    board.clearContents();assert!(board.setString_forType(&NSString::from_str("own settled again"),unsafe{NSPasteboardTypeString}));
    clipboard.poll_with_application(&port,&tx,application);
    match rx.try_recv().map_err(|_|"Text capture did not resume after uncertain transition")? {ruycast_native::control::HostEvent::ClipboardCapture(c)=>assert!(matches!(c.payload,ruycast_native::clipboard_history::Payload::Text(ref s) if s=="own settled again")),_=>return Err("Unexpected own event".into())}
    assert!(!port.busy.load(std::sync::atomic::Ordering::Acquire));
    board.clearContents();assert!(board.setString_forType(&NSString::from_str("own before attribution mutation"),unsafe{NSPasteboardTypeString}));
    let mutations=Cell::new(0);clipboard.poll_with_application(&port,&tx,||{mutations.set(mutations.get()+1);if mutations.get()==1{board.clearContents();assert!(board.setString_forType(&NSString::from_str("own after attribution mutation"),unsafe{NSPasteboardTypeString}));}Some(objc2_app_kit::NSRunningApplication::currentApplication())});
    assert!(rx.try_recv().is_err(),"Mutation during attribution was captured");assert!(!port.busy.load(std::sync::atomic::Ordering::Acquire));
    clipboard.poll_with_application(&port,&tx,application);
    let capture=rx.try_recv().map_err(|_|"Next poll lost the changed owned board count")?;
    match capture {ruycast_native::control::HostEvent::ClipboardCapture(c)=>assert!(matches!(c.payload,ruycast_native::clipboard_history::Payload::Text(ref s) if s=="own after attribution mutation")),_=>return Err("Unexpected own event".into())}
    board.clearContents();assert!(board.setString_forType(&NSString::from_str("own source changed"),unsafe{NSPasteboardTypeString}));
    let source_reads=Cell::new(0);clipboard.poll_with_application(&port,&tx,||{source_reads.set(source_reads.get()+1);if source_reads.get()==1{Some(objc2_app_kit::NSRunningApplication::currentApplication())}else{None}});
    assert_eq!(source_reads.get(),2);assert!(rx.try_recv().is_err(),"Source change during snapshot was accepted");
    board.clearContents();assert!(board.setString_forType(&NSString::from_str("own capture admission raced"),unsafe{NSPasteboardTypeString}));
    let prior=*clipboard.observation.borrow();
    clipboard.poll_with_application(&port,&tx,||{port.busy.store(true,std::sync::atomic::Ordering::SeqCst);Some(objc2_app_kit::NSRunningApplication::currentApplication())});
    assert!(rx.try_recv().is_err());assert_eq!(*clipboard.observation.borrow(),prior,"refused admission must preserve the full prior board/source observation");
    port.busy.store(false,std::sync::atomic::Ordering::SeqCst);
    clipboard.poll_with_application(&port,&tx,application);
    match rx.try_recv().map_err(|_|"Next poll lost an admission-refused owned board")? {ruycast_native::control::HostEvent::ClipboardCapture(c)=>assert!(matches!(c.payload,ruycast_native::clipboard_history::Payload::Text(ref s) if s=="own capture admission raced")),_=>return Err("Unexpected own event".into())}
    for lane in 0..4 {
        port.publish(&settings,false);
        board.clearContents();assert!(board.setString_forType(&NSString::from_str("own policy changed before admission"),unsafe{NSPasteboardTypeString}));
        let mut newer=settings.clone();
        if lane==0{newer.enabled=false;}
        if lane==2{newer.excluded_apps=vec![objc2_app_kit::NSRunningApplication::currentApplication().bundleIdentifier().map(|s|s.to_string()).unwrap_or_else(||"fixture".into())];}
        if lane==3{newer.max_text_bytes=1;}
        clipboard.poll_with_application(&port,&tx,||{port.publish(&newer,lane==1);Some(objc2_app_kit::NSRunningApplication::currentApplication())});
        if rx.try_recv().is_ok(){return Err(format!("New active capture policy was ignored after lease admission, owned lane {lane}"));}
        assert!(!port.busy.load(std::sync::atomic::Ordering::SeqCst));
    }
    port.publish(&settings,false);
    // Keep the actual accepted event and its lease unpublished while the board
    // changes. Persist through the same Store::capture used by the owner loop;
    // the private helper never reads or writes the general pasteboard.
    let history=root.join("owned-poll-history");
    {use std::os::unix::fs::DirBuilderExt;std::fs::DirBuilder::new().mode(0o700).create(&history).map_err(|e|e.to_string())?;}
    let mut store=ruycast_native::clipboard_history::Store::open(&history,&settings)?;
    assert!(store.items.is_empty(),"Owned poll fixture must begin with fresh history");
    board.clearContents();assert!(board.setString_forType(&NSString::from_str("own already in flight"),unsafe{NSPasteboardTypeString}));
    clipboard.poll_with_application(&port,&tx,application);
    let accepted=match rx.try_recv().map_err(|_|"Own accepted capture did not enter its real event queue")? {ruycast_native::control::HostEvent::ClipboardCapture(c)=>c,_=>return Err("Unexpected own event".into())};
    assert!(port.admission_busy());
    assert!(matches!(accepted.payload,ruycast_native::clipboard_history::Payload::Text(ref s) if s=="own already in flight"));
    board.clearContents();assert!(board.setString_forType(&NSString::from_str("own after in flight"),unsafe{NSPasteboardTypeString}));
    reads.set(0);clipboard.poll_with_application(&port,&tx,application);
    assert_eq!(reads.get(),0,"in-flight poll consulted attribution");
    assert!(rx.try_recv().is_err(),"In-flight poll queued a replacement capture");
    let ruycast_native::clipboard_history::Capture{payload,source,at,_lease}=accepted;
    assert!(store.capture(payload,source,at,&settings,||true)?.durable);
    assert!(port.admission_busy(),"Accepted capture retains its lease through persistence");
    assert_eq!(store.items.len(),1);assert_eq!(store.items[0].text,"own already in flight");
    let disk:serde_json::Value=serde_json::from_slice(&std::fs::read(history.join("clipboard.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    assert_eq!(disk,serde_json::to_value(&store.items).map_err(|e|e.to_string())?);
    drop(_lease);assert!(!port.admission_busy());
    clipboard.poll_with_application(&port,&tx,application);
    let next=match rx.try_recv().map_err(|_|"Poll lost the changed board after accepted capture completed")? {ruycast_native::control::HostEvent::ClipboardCapture(c)=>c,_=>return Err("Unexpected own event".into())};
    assert!(matches!(next.payload,ruycast_native::clipboard_history::Payload::Text(ref s) if s=="own after in flight"));
    let ruycast_native::clipboard_history::Capture{payload,source,at,_lease}=next;
    assert!(store.capture(payload,source,at,&settings,||true)?.durable);
    drop(_lease);assert!(!port.admission_busy());
    assert_eq!(store.items.iter().map(|clip|clip.text.as_str()).collect::<Vec<_>>(),["own after in flight","own already in flight"]);
    let disk:serde_json::Value=serde_json::from_slice(&std::fs::read(history.join("clipboard.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    assert_eq!(disk,serde_json::to_value(&store.items).map_err(|e|e.to_string())?);
    assert!(!history.join("clipboard-assets").exists());
    reads.set(0);
    port.publish(&settings,true);clipboard.poll_with_application(&port,&tx,application);assert_eq!(reads.get(),0,"paused poll consulted attribution");
    let mut disabled=settings;disabled.enabled=false;port.publish(&disabled,false);clipboard.poll_with_application(&port,&tx,application);assert_eq!(reads.get(),0,"disabled poll consulted attribution");
    Ok(())
}
