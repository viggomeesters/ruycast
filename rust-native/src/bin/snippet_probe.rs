//! Synthetic snippet payload/owned named-board fixture; never accesses the
//! general board, activates another app, or generates input.
use objc2::{rc::autoreleasepool, MainThreadMarker};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
use ruycast_native::{snippet_clipboard, snippet_output};
use serde::Deserialize;
use serde_json::json;
use std::io::{self, Read};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    source: String,
    mode: Option<String>,
    cursor: Option<isize>,
    #[serde(default)]
    label: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    clipboard: bool,
    #[serde(default)]
    seed: bool,
}
fn run() -> Result<serde_json::Value, String> {
    let main = MainThreadMarker::new().ok_or("Main-thread fixture required")?;
    let mut bytes = vec![];
    io::stdin()
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("Fixture input exceeds limit.".into());
    }
    let cases: Vec<Case> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if cases.len() > 2048 {
        return Err("Too many fixture cases.".into());
    }
    let output: Vec<_> = cases.into_iter().map(|case| autoreleasepool(|_| {
        let payload = snippet_output::render(&case.source, case.mode.as_deref(), case.cursor);
        let clipboard = case.clipboard.then(|| {
            let board = NSPasteboard::pasteboardWithUniqueName();
            struct Release<'a>(&'a NSPasteboard);
            impl Drop for Release<'_> {
                fn drop(&mut self) {
                    self.0.clearContents();
                    unsafe { let _: () = objc2::msg_send![self.0, releaseGlobally]; }
                }
            }
            let _release = Release(&board);
            if case.seed {
                board.setString_forType(&objc2_foundation::NSString::from_str("Own previous text"), unsafe { NSPasteboardTypeString });
                board.setString_forType(&objc2_foundation::NSString::from_str("<b>Own previous HTML</b>"), unsafe { NSPasteboardTypeHTML });
            }
            let before = board.changeCount();
            let success = snippet_clipboard::write(main, &payload, &board);
            let mut types = board.types().map(|v| v.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap_or_default();
            types.sort();
            json!({"success":success,"changes":board.changeCount()-before,"text":board.stringForType(unsafe { NSPasteboardTypeString }).map(|s|s.to_string()),"html":board.stringForType(unsafe { NSPasteboardTypeHTML }).map(|s|s.to_string()),"types":types,"items":board.pasteboardItems().map_or(0, |a| a.len())})
        });
        let steps = payload.cursor.and_then(|c| usize::try_from(c).ok()).map(|c|snippet_output::cursor_steps(&payload.text,c));
        json!({"payload":payload,"cursorSteps":steps,"links":snippet_output::links(&case.source),"validURL":snippet_output::valid_url(&case.url),"linkSource":snippet_output::link_source(&case.label,&case.url),"clipboard":clipboard})
    })).collect();
    Ok(json!({"ok":true,"cases":output}))
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--check-html-import") {
        match check_html_import() {
            Ok(value) => println!("{value}"),
            Err(error) => {
                println!("{}", json!({"ok":false,"error":error}));
                std::process::exit(1);
            }
        }
        return;
    }
    if std::env::args().nth(1).as_deref()==Some("private-board") {private_board();return;}
    if std::env::args().nth(1).as_deref() == Some("owned-board") {
        owned_board();
        return;
    }
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}

// The original nested SnippetEditing check imports the actual generated HTML,
// rather than inferring Cocoa behavior from byte equality with its renderer.
// Only fixed synthetic text is imported: no board, document URL or image input.
fn check_html_import() -> Result<serde_json::Value, String> {
    use objc2::{msg_send, rc::Retained, runtime::AnyObject, AnyThread};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSAttributedStringDocumentFormats,
        NSDocumentTypeDocumentOption, NSHTMLTextDocumentType, NSLinkAttributeName,
        NSParagraphStyle, NSParagraphStyleAttributeName,
    };
    use objc2_foundation::{NSArray, NSAttributedString, NSData, NSDictionary, NSRange};
    let main = MainThreadMarker::new().ok_or("HTML import requires the main thread")?;
    let app = NSApplication::sharedApplication(main);
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    if app.activationPolicy() != NSApplicationActivationPolicy::Prohibited {
        return Err("HTML fixture actual activation policy is not prohibited".into());
    }
    fn import(html: &str) -> Result<Retained<NSAttributedString>, String> {
        let options = NSDictionary::from_slices(
            &[unsafe { NSDocumentTypeDocumentOption }],
            &[unsafe { NSHTMLTextDocumentType } as &AnyObject],
        );
        // These are exactly the documented document-type key/string value.
        unsafe {
            NSAttributedString::initWithData_options_documentAttributes_error(
                NSAttributedString::alloc(), &NSData::with_bytes(html.as_bytes()), &options, None,
            )
        }.map_err(|_| "Generated snippet HTML could not be imported".into())
    }
    fn attributes(parsed: &NSAttributedString) -> Result<(usize, bool), String> {
        let mut links = 0;
        let mut tables = false;
        for (key, is_link) in [(unsafe { NSLinkAttributeName }, true), (unsafe { NSParagraphStyleAttributeName }, false)] {
            let mut index = 0;
            while index < parsed.length() {
                let mut range = NSRange::new(0, 0);
                let value = unsafe { parsed.attribute_atIndex_effectiveRange(key, index, &mut range) };
                let end = range.location.checked_add(range.length).ok_or("Imported range overflow")?;
                if range.location > index || end <= index || end > parsed.length() {
                    return Err("Imported attribute range differs".into());
                }
                if let Some(value) = value {
                    if is_link { links += 1; }
                    else {
                        let style = value.downcast_ref::<NSParagraphStyle>().ok_or("Imported paragraph style type differs")?;
                        // The locked bindings gate the typed method behind an
                        // otherwise unused NSTextTable feature. Read its actual
                        // NSArray result without expanding the dependency set.
                        let blocks: Retained<NSArray<AnyObject>> = unsafe { msg_send![style, textBlocks] };
                        tables |= !blocks.is_empty();
                    }
                }
                index = end;
            }
        }
        Ok((links, tables))
    }
    autoreleasepool(|_| {
        let source = "Hoi 👋,\n\nSee [Guide](https://example.com/a_(b)?x=1&y=2)\n\nGoodbye\n";
        for mode in ["teams", "html"] {
            let payload = snippet_output::render(source, Some(mode), None);
            let parsed = import(payload.html.as_deref().ok_or("Rich payload HTML missing")?)?;
            let (links, tables) = attributes(&parsed)?;
            if links != 1 || !parsed.string().to_string().contains("Guide") {
                return Err("Generated hyperlink import differs".into());
            }
            if mode == "teams" && (tables || parsed.string().to_string() != payload.text + "\n") {
                return Err("Teams import contains tables or differs in blank paragraphs".into());
            }
        }
        for sample in ["", "a", "a\n", "\n", "\n\n", "a\n\n", "a\n \t\nb"] {
            let payload = snippet_output::render(sample, Some("teams"), None);
            let parsed = import(payload.html.as_deref().ok_or("Teams payload HTML missing")?)?;
            let visible = sample.split('\n').map(|line| if line.trim_matches([' ', '\t']).is_empty() { "" } else { line }).collect::<Vec<_>>().join("\n");
            if parsed.string().to_string() != visible + "\n" || attributes(&parsed)?.1 {
                return Err("Teams paragraph import lost blank lines or contains a table".into());
            }
        }
        if app.keyWindow().is_some() || app.mainWindow().is_some() || app.windows().iter().any(|window| window.isVisible()) {
            return Err("HTML import fixture created a visible or key window".into());
        }
        Ok(json!({"ok":true,"richModes":2,"blankParagraphSamples":7,"hyperlinkImport":true,"noTextTables":true,"clipboardAccessed":false,"visibleWindowOpened":false,"desktopInputSent":false}))
    })
}

fn private_board(){
    let main=MainThreadMarker::new().expect("Owned main-thread fixture");
    let guard=unsafe{libc::dlsym(libc::RTLD_DEFAULT,c"ruycast_owned_board_general_guard".as_ptr())};
    assert!(!guard.is_null(),"General-board abort provider required before any board access");
    let guard:unsafe extern "C" fn()->bool=unsafe{std::mem::transmute(guard)};
    assert!(unsafe{guard()});
    autoreleasepool(|_|{
        let prepared=ruycast_native::paste::Prepared::utility("Owned synthetic private payload".into(),true,true);
        assert!(prepared.concealed&&prepared.copy_only&&prepared.clipboard.is_none());
        let item=snippet_clipboard::PreparedItem::concealed(main,&prepared.payload.text).unwrap();
        let board=NSPasteboard::pasteboardWithUniqueName();
        let expected=item.clone();
        assert!(matches!(item.publish(&board,||true),snippet_clipboard::Publish::Attempted(true)));
        assert!(expected.matches(&board));
        assert_eq!(board.stringForType(&objc2_foundation::NSString::from_str("org.nspasteboard.ConcealedType")).unwrap().to_string(),"1");
        assert_eq!(board.stringForType(unsafe{NSPasteboardTypeString}).unwrap().to_string(),prepared.payload.text);
        board.clearContents();unsafe{let _:()=objc2::msg_send![&*board,releaseGlobally];}
        println!("{}",json!({"ok":true,"namedBoardOnly":true,"concealedType":true,"utilityPrepared":true,"secretEchoed":false}));
    });
}

fn owned_board() {
    let main = MainThreadMarker::new().expect("Own main-thread fixture");
    let mode: i32 = std::env::args().nth(2).expect("Own mode").parse().unwrap();
    assert!((0..=9).contains(&mode));
    autoreleasepool(|_| {
        let board = NSPasteboard::pasteboardWithUniqueName();
        let payload = snippet_output::Payload {
            text: if mode >= 8 {
                "x".repeat(5000) + "A\0B😀"
            } else {
                "A\0B😀".into()
            },
            html: Some("<b>A</b>".into()),
            cursor: None,
        };
        let item = snippet_clipboard::PreparedItem::new(main, &payload).unwrap();
        board.clearContents();
        assert!(item.write_to(&board));
        let original = item.matches(&board);
        // Complete injection is explicit and only enabled for this synthetic
        // named-board mode. Missing fixture symbols fail before any comparison.
        let attach =
            unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_owned_board_attach".as_ptr()) };
        let detach =
            unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_owned_board_detach".as_ptr()) };
        assert!(
            !attach.is_null() && !detach.is_null(),
            "Own board SDK fixture required"
        );
        let guard = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ruycast_owned_board_general_guard".as_ptr(),
            )
        };
        assert!(!guard.is_null(), "General-board abort guard required");
        let guard: unsafe extern "C" fn() -> bool = unsafe { std::mem::transmute(guard) };
        assert!(unsafe { guard() });
        let attach: unsafe extern "C" fn(*const NSPasteboard, i32) -> bool =
            unsafe { std::mem::transmute(attach) };
        let detach: unsafe extern "C" fn() = unsafe { std::mem::transmute(detach) };
        assert!(unsafe { attach(&*board, mode) });
        let full = item.matches(&board);
        let partial = item.observes_own_content(&board);
        unsafe {
            detach();
        }
        let restored = item.matches(&board);
        board.clearContents();
        unsafe {
            let _: () = objc2::msg_send![&*board, releaseGlobally];
        }
        println!(
            "{}",
            json!({"original":original,"full":full,"partial":partial,"restored":restored,"generalBoardAbortGuard":true})
        );
    });
}
