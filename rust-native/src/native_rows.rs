//! Native display-only rows; no command payload or activation authority.
use crate::native_model::RowRef;
use objc2::{
    define_class, msg_send, rc::Retained, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSBundle, NSDate, NSDateFormatter, NSDateFormatterStyle, NSPoint, NSRect, NSSize, NSString,
};
use ruycast_native::presentation::Icon;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
struct Cached {
    key: String,
    at: Instant,
    image: Option<Retained<NSImage>>,
    name: String,
}
thread_local! {static CACHE:RefCell<Vec<Rc<Cached>>>=const {RefCell::new(Vec::new())};}
thread_local! {static WEBSITES:RefCell<Option<ruycast_native::website_icons::Service>>=const {RefCell::new(None)};}
pub struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        stop();
    }
}
pub fn start(wake: std::sync::Arc<dyn Fn() + Send + Sync>, hidden: bool) {
    if hidden {
        return;
    }
    use objc2_foundation::{
        NSSearchPathDirectory, NSSearchPathDomainMask, NSSearchPathForDirectoriesInDomains,
    };
    if let Some(path) = NSSearchPathForDirectoriesInDomains(
        NSSearchPathDirectory::CachesDirectory,
        NSSearchPathDomainMask::UserDomainMask,
        true,
    )
    .iter()
    .next()
    {
        let cache = std::path::PathBuf::from(path.to_string())
            .join("nl.viggomeesters.ruycast/website-icons");
        WEBSITES.with(|s| {
            *s.borrow_mut() = ruycast_native::website_icons::Service::new(cache, wake).ok()
        });
    }
}
pub fn poll() -> bool {
    WEBSITES.with(|s| s.borrow_mut().as_mut().is_some_and(|s| s.poll()))
}
pub fn stop() {
    let websites = WEBSITES.with(|s| s.borrow_mut().take());
    let cache = CACHE.with(|s| std::mem::take(&mut *s.borrow_mut()));
    drop(websites);
    drop(cache);
}
define_class!(
 #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[ivars=bool]
 struct Keycap;
 impl Keycap {
  #[unsafe(method(drawRect:))] fn draw(&self,_:NSRect){
   if *self.ivars() {
    NSColor::labelColor().colorWithAlphaComponent(0.08).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(self.bounds(),4.,4.).fill();
    return;
   }
   NSColor::labelColor().colorWithAlphaComponent(0.045).setFill();
   let p=NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(self.bounds(),5.,5.);p.fill();
   NSColor::labelColor().colorWithAlphaComponent(0.15).setStroke();p.setLineWidth(0.7);p.stroke();
  }
 }
);
define_class!(
 #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[ivars=String]
 struct WindowGlyph;
 impl WindowGlyph {
  #[unsafe(method(isFlipped))] fn flipped(&self)->bool{true}
  #[unsafe(method(drawRect:))] fn draw(&self,_:NSRect){
   NSColor::secondaryLabelColor().setStroke();NSColor::secondaryLabelColor().setFill();
   let line=|points:&[(f64,f64)]|{let p=NSBezierPath::bezierPath();p.setLineWidth(1.4);p.setLineCapStyle(NSLineCapStyle::Round);p.setLineJoinStyle(NSLineJoinStyle::Round);for(i,(x,y))in points.iter().enumerate(){if i==0{p.moveToPoint(NSPoint::new(*x,*y));}else{p.lineToPoint(NSPoint::new(*x,*y));}}p.stroke();};
   let bounds=if self.ivars()=="restore"{rect(2.5,8.5,14.,11.)}else{rect(2.5,4.5,19.,15.)};
   let p=NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds,2.,2.);p.setLineWidth(1.4);p.stroke();
   let region=match self.ivars().as_str(){"left-half"=>Some((0.,0.5)),"right-half"=>Some((0.5,0.5)),"first-third"=>Some((0.,1./3.)),"center-third"=>Some((1./3.,1./3.)),"last-third"=>Some((2./3.,1./3.)),"first-two-thirds"=>Some((0.,2./3.)),"last-two-thirds"=>Some((1./3.,2./3.)),_=>None};
   if let Some((x,w))=region{NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(4.5+15.*x,6.5,15.*w,11.),0.8,0.8).fill();}
   else{match self.ivars().as_str(){
    "restore"=>line(&[(7.5,6.),(7.5,4.5),(21.5,4.5),(21.5,15.5),(19.,15.5)]),
    "maximize"=>{line(&[(7.,10.),(7.,8.),(10.,8.)]);line(&[(14.,8.),(17.,8.),(17.,10.)]);line(&[(7.,14.),(7.,16.),(10.,16.)]);line(&[(14.,16.),(17.,16.),(17.,14.)]);},
    "next-screen"|"previous-screen"=>{let right=self.ivars()=="next-screen";line(&[(7.,12.),(17.,12.)]);line(&[(if right{14.}else{10.},9.),(if right{17.}else{7.},12.),(if right{14.}else{10.},15.)]);line(&[(12.,20.),(12.,22.)]);line(&[(8.,22.),(16.,22.)]);},_=>()
   }}
  }
 }
);
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w.max(1.), h))
}
fn application(id: &str, hidden: bool) -> (Option<Retained<NSImage>>, String) {
    if hidden {
        #[cfg(feature = "test-tools")]
        if let Some((image, name)) = fixture(&format!("app:{id}")) {
            return (image, name.unwrap_or_else(|| id.into()));
        }
        return (None, id.into());
    }
    application_at(id, Instant::now, |id| {
        let url = NSWorkspace::sharedWorkspace()
            .URLForApplicationWithBundleIdentifier(&NSString::from_str(id));
        let name = url
            .as_ref()
            .and_then(|url| NSBundle::bundleWithURL(url))
            .and_then(|bundle| {
                ["CFBundleDisplayName", "CFBundleName"]
                    .iter()
                    .find_map(|key| {
                        bundle
                            .objectForInfoDictionaryKey(&NSString::from_str(key))
                            .and_then(|v| v.downcast::<NSString>().ok())
                            .map(|s| s.to_string())
                    })
            })
            .unwrap_or_else(|| id.into());
        let image = url
            .and_then(|u| u.path())
            .map(|p| NSWorkspace::sharedWorkspace().iconForFile(&p));
        (image, name)
    })
}
fn cached_at(key: &str, at: Instant) -> Option<Rc<Cached>> {
    CACHE.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|e| e.key == key && at.saturating_duration_since(e.at) < Duration::from_secs(30))
            .cloned()
    })
}
// Only Rust Rc bookkeeping runs under the borrow. NSImage retain/release and
// metadata resolution may reenter row rendering, so they stay outside it.
fn publish_cache(key: &str, entry: Option<Rc<Cached>>) {
    let retired = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let mut retired = Vec::new();
        for index in (0..cache.len()).rev() {
            if cache[index].key == key {
                retired.push(cache.remove(index));
            }
        }
        if let Some(entry) = entry {
            if cache.len() >= 512 {
                retired.push(cache.remove(0));
            }
            cache.push(entry);
        }
        retired
    });
    drop(retired);
}
fn application_at(
    id: &str,
    now: impl Fn() -> Instant,
    resolve: impl FnOnce(&str) -> (Option<Retained<NSImage>>, String),
) -> (Option<Retained<NSImage>>, String) {
    let at = now();
    let key = format!("app:{id}");
    if let Some(entry) = cached_at(&key, at) {
        return (entry.image.clone(), entry.name.clone());
    }
    let (image, name) = resolve(id);
    publish_cache(
        &key,
        Some(Rc::new(Cached {
            key: key.clone(),
            at,
            image: image.clone(),
            name: name.clone(),
        })),
    );
    (image, name)
}
fn path_image(path: &str, hidden: bool) -> Option<Retained<NSImage>> {
    if hidden {
        #[cfg(feature = "test-tools")]
        {
            return fixture(&format!("path:{path}")).and_then(|v| v.0);
        }
        #[cfg(not(feature = "test-tools"))]
        {
            return None;
        }
    }
    path_image_at(path, Instant::now, |path| {
        let expanded = NSString::from_str(path).stringByExpandingTildeInPath();
        if !std::path::Path::new(&expanded.to_string()).exists() {
            return None;
        }
        let image = NSWorkspace::sharedWorkspace().iconForFile(&expanded);
        Some(image)
    })
}
fn path_image_at(
    path: &str,
    now: impl Fn() -> Instant,
    resolve: impl FnOnce(&str) -> Option<Retained<NSImage>>,
) -> Option<Retained<NSImage>> {
    let at = now();
    let key = format!("path:{path}");
    if let Some(entry) = cached_at(&key, at) {
        return entry.image.clone();
    }
    let image = resolve(path);
    publish_cache(
        &key,
        image.as_ref().map(|image| {
            Rc::new(Cached {
                key: key.clone(),
                at,
                image: Some(image.clone()),
                name: String::new(),
            })
        }),
    );
    image
}
#[cfg(feature = "test-tools")]
pub fn check_cache(main: MainThreadMarker) -> Result<(), String> {
    use std::{cell::Cell, fs, os::unix::fs::PermissionsExt, path::PathBuf};
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
            .ok_or("Native icon check requires an explicit fixture parent")?,
    );
    if !parent.is_absolute()
        || !fs::symlink_metadata(&parent).is_ok_and(|m| m.is_dir())
        || !fs::symlink_metadata(parent.join(".ruycast-ui-fixture-parent"))
            .is_ok_and(|m| m.is_file())
    {
        return Err("Native icon check requires an existing marked fixture parent".into());
    }
    let dir = parent.join(format!(
        "row-icons-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    ));
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _fixture = Fixture(dir.clone());
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    struct Restore(Vec<Rc<Cached>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let own = CACHE
                .with(|c| std::mem::replace(&mut *c.borrow_mut(), std::mem::take(&mut self.0)));
            drop(own);
        }
    }
    let _restore = Restore(CACHE.with(|c| std::mem::take(&mut *c.borrow_mut())));
    let require = |ok: bool, message: &str| if ok { Ok(()) } else { Err(message.to_string()) };
    let png = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6,
        0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 104, 112, 80, 248,
        15, 194, 12, 48, 6, 0, 64, 180, 7, 125, 193, 185, 247, 76, 0, 0, 0, 0, 73, 69, 78, 68, 174,
        66, 96, 130,
    ];
    let image = NSImage::initWithData(
        NSImage::alloc(),
        &objc2_foundation::NSData::with_bytes(&png),
    )
    .ok_or("Owned native icon image failed")?;
    let start = Instant::now();
    let clock = Cell::new(start);
    let calls = Cell::new(0usize);
    let resolve = |id: &str| {
        calls.set(calls.get() + 1);
        if id == "found" {
            (Some(image.clone()), "Found App".into())
        } else {
            (None, id.into())
        }
    };
    for _ in 0..1000 {
        require(
            application_at("found", || clock.get(), resolve).1 == "Found App",
            "Cached application name changed",
        )?;
        let (missing, name) = application_at("missing", || clock.get(), resolve);
        require(
            missing.is_none() && name == "missing",
            "Missing application fallback changed",
        )?;
    }
    require(
        calls.get() == 2,
        "Repeated rows resolved application metadata again",
    )?;
    clock.set(start + Duration::from_secs(31));
    let _ = application_at("missing", || clock.get(), resolve);
    require(
        calls.get() == 3,
        "Missing application metadata did not refresh",
    )?;
    let slow_calls = Cell::new(0);
    let slow = |id: &str| {
        slow_calls.set(slow_calls.get() + 1);
        clock.set(clock.get() + Duration::from_secs(31));
        (None, id.to_string())
    };
    let _ = application_at("slow", || clock.get(), slow);
    let _ = application_at("slow", || clock.get(), slow);
    require(
        slow_calls.get() == 2,
        "Application cache lifetime started after resolution",
    )?;
    let file = dir.join("owned-file.txt");
    fs::write(&file, b"own fixture").map_err(|e| e.to_string())?;
    let path = file.to_str().ok_or("Invalid owned icon path")?;
    let file_calls = Cell::new(0);
    let file_resolve = |path: &str| {
        file_calls.set(file_calls.get() + 1);
        std::path::Path::new(path).is_file().then(|| image.clone())
    };
    require(
        path_image_at(path, || clock.get(), file_resolve).is_some(),
        "Owned file icon missing",
    )?;
    fs::remove_file(&file).map_err(|e| e.to_string())?;
    clock.set(clock.get() + Duration::from_secs(31));
    require(
        path_image_at(path, || clock.get(), file_resolve).is_none(),
        "Expired icon concealed a deleted file",
    )?;
    fs::write(&file, b"own recreated fixture").map_err(|e| e.to_string())?;
    require(
        path_image_at(path, || clock.get(), file_resolve).is_some(),
        "Owned file icon failed to recover",
    )?;
    require(
        file_calls.get() == 3,
        "Deleted file was cached as a permanent miss",
    )?;
    let slow_file = dir.join("slow-owned-file.txt");
    fs::write(&slow_file, b"own slow fixture").map_err(|e| e.to_string())?;
    let slow_path = slow_file.to_str().ok_or("Invalid owned slow icon path")?;
    let slow_file_calls = Cell::new(0);
    let slow_file_resolve = |path: &str| {
        slow_file_calls.set(slow_file_calls.get() + 1);
        clock.set(clock.get() + Duration::from_secs(31));
        std::path::Path::new(path).is_file().then(|| image.clone())
    };
    let _ = path_image_at(slow_path, || clock.get(), slow_file_resolve);
    let _ = path_image_at(slow_path, || clock.get(), slow_file_resolve);
    require(
        slow_file_calls.get() == 2,
        "File icon cache lifetime started after resolution",
    )?;
    let _ = application_at(
        "reentrant",
        || clock.get(),
        |id| {
            let _ = application_at("nested", || clock.get(), resolve);
            (Some(image.clone()), id.to_string())
        },
    );
    for i in 0..513 {
        let _ = application_at(&format!("bounded-{i}"), || clock.get(), resolve);
    }
    require(
        CACHE.with(|c| c.borrow().len()) == 512,
        "Native row icon cache exceeds its bound",
    )?;
    for (value, expected) in [
        ("shift+ctrl+alt+cmd+left", "✦ ←"),
        ("cmd+alt+n", "⌥ ⌘ N"),
        ("ctrl+left-control", "⌃"),
        ("ctrl+alt+delete", "⌃ ⌥ ⌫"),
        ("right-cmd+a", "R⌘ A"),
    ] {
        let caps = shortcut(main, value, 160., false);
        require(
            texts(&caps) == [expected],
            "Native keycap readback lost its logical group",
        )?;
        caps.setHidden(true);
        require(
            texts(&caps).is_empty(),
            "Hidden keycaps leaked into readback",
        )?;
    }
    let caps = shortcut(main, "left-cmd+a", 160., false);
    require(
        caps.accessibilityLabel()
            .is_some_and(|label| label.to_string() == "Left Command + a"),
        "Native sided keycap accessibility label changed",
    )?;
    Ok(())
}
#[cfg(feature = "test-tools")]
fn fixture(key: &str) -> Option<(Option<Retained<NSImage>>, Option<String>)> {
    use sha2::{Digest, Sha256};
    use std::{fs, io::Read, os::unix::fs::OpenOptionsExt};
    if std::env::var("RUYCAST_NATIVE_TEST_ICON_FIXTURES").as_deref() != Ok("owned") {
        return None;
    }
    let root = fs::canonicalize(std::env::var_os("RUYCAST_HOME")?).ok()?;
    if fs::read_to_string(root.parent()?.join(".ruycast-test-fixture"))
        .ok()?
        .trim()
        != "own-disposable-fixture-v1"
    {
        return None;
    }
    let dir = root.join("icon-fixtures");
    if fs::symlink_metadata(&dir).ok()?.file_type().is_symlink() {
        return None;
    }
    let key = format!("{:x}", Sha256::digest(key.as_bytes()));
    let read = |extension: &str, max: u64| -> Option<Vec<u8>> {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(dir.join(format!("{key}.{extension}")))
            .ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > max {
            return None;
        }
        let mut bytes = vec![];
        file.take(max + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > max {
            return None;
        }
        (bytes.len() as u64 <= max).then_some(bytes)
    };
    let image = read("png", 131072)
        .and_then(|b| ruycast_native::clipboard_image::website_icon(&b).ok())
        .and_then(|b| {
            NSImage::initWithData(NSImage::alloc(), &objc2_foundation::NSData::with_bytes(&b))
        });
    let name = read("name", 1024).and_then(|b| String::from_utf8(b).ok());
    Some((image, name))
}
pub fn source_name(id: &str, hidden: bool) -> String {
    application(id, hidden).1
}
pub fn time(raw: &str, date: bool) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|date_time| {
            let formatter = NSDateFormatter::new();
            formatter.setDateStyle(if date {
                NSDateFormatterStyle::MediumStyle
            } else {
                NSDateFormatterStyle::NoStyle
            });
            formatter.setTimeStyle(NSDateFormatterStyle::ShortStyle);
            formatter
                .stringFromDate(&NSDate::dateWithTimeIntervalSince1970(
                    date_time.timestamp() as f64,
                ))
                .to_string()
        })
        .unwrap_or_else(|| raw.into())
}
pub fn height(row: &RowRef<'_>, library: bool) -> f64 {
    if !library && row.kind == "snippet" && row.aliases.len() > 1 {
        return 74.;
    }
    if !library
        && (row.detail.is_some()
            || (row.kind == "snippet"
                && (row.keyword.is_some_and(|s| !s.is_empty()) || !row.aliases.is_empty())))
    {
        58.
    } else {
        39.
    }
}
pub fn title(
    main: MainThreadMarker,
    row: &RowRef<'_>,
    width: f64,
    hidden: bool,
    library: bool,
) -> Retained<NSView> {
    let height = height(row, library);
    let subtitle = row
        .system
        .then_some(row.subtitle)
        .flatten()
        .filter(|s| !s.is_empty());
    let root = NSView::initWithFrame(NSView::alloc(main), rect(0., 0., width, height));
    let (image, symbol) = match &row.icon {
        Icon::Application(id) => (application(id, hidden).0, "app"),
        Icon::Path(path) => {
            let image = path_image(path, hidden);
            (image, "folder")
        }
        Icon::Symbol(symbol) => (None, *symbol),
        Icon::Website {
            origin,
            application: app,
        } => {
            let png = if hidden {
                None
            } else {
                WEBSITES.with(|s| s.borrow_mut().as_mut().and_then(|s| s.image(origin)))
            };
            let image = png
                .and_then(|b| {
                    NSImage::initWithData(
                        NSImage::alloc(),
                        &objc2_foundation::NSData::with_bytes(&b),
                    )
                })
                .or_else(|| app.as_ref().and_then(|id| application(id, hidden).0));
            #[cfg(feature = "test-tools")]
            let image = if hidden {
                fixture(&format!("website:{origin}"))
                    .and_then(|v| v.0)
                    .or(image)
            } else {
                image
            };
            (image, "link")
        }
        Icon::Window(_) => (None, "macwindow"),
    };
    let image = image.or_else(|| {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            None,
        )
    });
    if let Icon::Window(layout) = &row.icon {
        let icon = WindowGlyph::alloc(main).set_ivars(layout.clone());
        let icon: Retained<WindowGlyph> =
            unsafe { msg_send![super(icon),initWithFrame:rect(2.,7.,24.,24.)] };
        icon.setAccessibilityHidden(true);
        root.addSubview(&icon);
    } else {
        let icon = NSImageView::initWithFrame(NSImageView::alloc(main), rect(2., 7., 24., 24.));
        icon.setImage(image.as_deref());
        icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        icon.setAccessibilityHidden(true);
        root.addSubview(&icon);
    }
    let field = NSTextField::labelWithString(&NSString::from_str(row.title), main);
    field.setFrame(if subtitle.is_some() {
        rect(38., 18., width - 42., 19.)
    } else {
        rect(38., height - 30., width - 42., 22.)
    });
    field.setFont(Some(&NSFont::systemFontOfSize_weight(13., unsafe {
        NSFontWeightSemibold
    })));
    if subtitle.is_none() && height == 39. {
        let text_height = field.intrinsicContentSize().height.ceil();
        field.setFrame(rect(38., (height - text_height) / 2., width - 42., text_height));
    }
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    if row.system && !row.enabled {
        field.setTextColor(Some(&NSColor::secondaryLabelColor()));
    }
    root.addSubview(&field);
    if let Some(subtitle) = subtitle {
        let label = NSTextField::labelWithString(&NSString::from_str(subtitle), main);
        label.setFont(Some(&NSFont::systemFontOfSize(11.)));
        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        label.setFrame(rect(38., 1., width - 42., 16.));
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        root.addSubview(&label);
    } else if let Some(date) = row.detail {
        let label = NSTextField::labelWithString(&NSString::from_str(&time(date, false)), main);
        label.setFont(Some(&NSFont::systemFontOfSize(11.)));
        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        label.setFrame(rect(38., 7., width - 42., 18.));
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        root.addSubview(&label);
    } else if !library && row.kind == "snippet" && height > 39. {
        let mut parts = vec![];
        if let Some(keyword) = row.keyword.filter(|s| !s.is_empty()) {
            parts.push(keyword.into());
        }
        if !row.aliases.is_empty() {
            parts.push(format!("Aliases {}", row.aliases.join(", ")));
        }
        let text = NSTextField::labelWithString(&NSString::from_str(&parts.join(" · ")), main);
        text.setFont(Some(&NSFont::systemFontOfSize(11.)));
        text.setTextColor(Some(&NSColor::secondaryLabelColor()));
        if row.aliases.len() > 1 {
            text.setUsesSingleLineMode(false);
            text.setMaximumNumberOfLines(2);
            text.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
            if let Some(cell) = text.cell() {
                cell.setWraps(true);
                cell.setScrollable(false);
            }
        } else {
            text.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        }
        text.setFrame(rect(38., 7., width - 42., if row.aliases.len() > 1 { 34. } else { 18. }));
        text.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        root.addSubview(&text);
    } else if !library {
        if let Some(alias) = row.aliases.first() {
            let badge = NSTextField::labelWithString(&NSString::from_str(alias), main);
            badge.setFont(Some(&NSFont::systemFontOfSize_weight(11., unsafe {
                NSFontWeightMedium
            })));
            badge.setTextColor(Some(&NSColor::secondaryLabelColor()));
            badge.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            badge.setAlignment(NSTextAlignment::Center);
            let text_size = badge.intrinsicContentSize();
            // AppKit's text cell needs room for its own side bearings as well
            // as the visible padding between the label and the badge edge.
            let badge_width = (text_size.width.ceil() + 18.)
                .min(120.)
                .min((width - 100.).max(1.));
            let title_width = (field.intrinsicContentSize().width.ceil() + 8.)
                .min((width - 50. - badge_width).max(1.));
            field.setFrameSize(NSSize::new(title_width, field.frame().size.height));
            let text_height = text_size.height.ceil();
            let badge_height = text_height + 4.;
            let container = Keycap::alloc(main).set_ivars(true);
            let container: Retained<Keycap> = unsafe {
                msg_send![super(container), initWithFrame: rect(46. + title_width, (height - badge_height) / 2., badge_width, badge_height)]
            };
            let inset = 5_f64.min((badge_width - 1.) / 2.);
            badge.setFrame(rect(inset, 2., badge_width - 2. * inset, text_height));
            container.addSubview(&badge);
            root.addSubview(&container);
        }
    }
    root.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));
    root.setAccessibilityLabel(Some(&NSString::from_str(&texts(&root).join(" "))));
    root
}
pub fn shortcut(main: MainThreadMarker, value: &str, width: f64, imported: bool) -> Retained<NSView> {
    let (keys, label) = ruycast_native::presentation::shortcut(value);
    let label = if imported { format!("{label} (from previous profile, inactive here)") } else { label };
    let root = NSView::initWithFrame(NSView::alloc(main), rect(0., 0., width, 39.));
    root.setIdentifier(Some(&NSString::from_str("shortcut-keycaps")));
    root.setAccessibilityLabel(Some(&NSString::from_str(&label)));
    root.setToolTip(Some(&NSString::from_str(&label)));
    if imported {
        root.setAlphaValue(0.65);
    }
    let keys = keys.split(' ').collect::<Vec<_>>();
    let total = keys
        .iter()
        .map(|key| 19. + if key.chars().count() > 1 { 8. } else { 0. })
        .sum::<f64>()
        + (keys.len().saturating_sub(1) as f64) * 3.;
    let mut x = (width - total).max(0.);
    for key in keys {
        let w = 19. + if key.chars().count() > 1 { 8. } else { 0. };
        let cap = Keycap::alloc(main).set_ivars(false);
        let cap: Retained<Keycap> = unsafe { msg_send![super(cap),initWithFrame:rect(x,9.,w,21.)] };
        cap.setAccessibilityHidden(true);
        cap.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        let text = NSTextField::labelWithString(&NSString::from_str(key), main);
        text.setFont(Some(&NSFont::systemFontOfSize_weight(12., unsafe {
            NSFontWeightMedium
        })));
        text.setTextColor(Some(&NSColor::secondaryLabelColor()));
        text.setAlignment(NSTextAlignment::Center);
        text.setFrame(rect(0., 2., w, 17.));
        cap.addSubview(&text);
        root.addSubview(&cap);
        x += w + 3.;
    }
    root
}
pub fn texts(view: &NSView) -> Vec<String> {
    if view.isHidden() {
        return vec![];
    }
    if let Some(field) = view.downcast_ref::<NSTextField>() {
        return vec![field.stringValue().to_string()];
    }
    let values: Vec<String> = view.subviews().iter().flat_map(|v| texts(&v)).collect();
    if view
        .identifier()
        .is_some_and(|id| id.to_string() == "shortcut-keycaps")
    {
        // One shortcut remains one logical readback value even though each
        // visible native keycap has its own text field.
        vec![values.join(" ")]
    } else {
        values
    }
}
