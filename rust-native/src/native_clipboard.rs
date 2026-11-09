//! Clipboard history preview within the existing palette, never a separate window.
use crate::native::Controller;
use objc2::{
    define_class, msg_send,
    rc::{Retained, Weak},
    sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize, NSString};
use ruycast_native::clipboard_history::Surface;
use std::cell::RefCell;
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
struct Binding {
    owner: Weak<Controller>,
    operation: String,
    id: Option<String>,
    revision: u64,
    epoch: u64,
    dismissal: u64,
}
struct ButtonState {
    binding: RefCell<Option<Binding>>,
}
type SurfaceKey = (u64, u64, u64, Option<String>);
define_class!(
 #[unsafe(super=NSButton)] #[thread_kind=MainThreadOnly] #[ivars=ButtonState]
 struct ClipboardButton;
 impl ClipboardButton {
  #[unsafe(method(invoke:))]
  fn invoke(&self,_:Option<&objc2_foundation::NSObject>){if let Some(b)=self.ivars().binding.borrow().as_ref(){if let Some(owner)=b.owner.load(){owner.clipboard_control(&b.operation,b.id.clone(),b.revision,b.epoch,b.dismissal);}}}
 }
);
pub struct Panel {
    root: Retained<NSView>,
    text: Retained<NSTextView>,
    text_scroll: Retained<NSScrollView>,
    image: Retained<NSImageView>,
    metadata: Retained<NSTextField>,
    buttons: RefCell<Vec<Retained<ClipboardButton>>>,
    key: RefCell<Option<SurfaceKey>>,
    #[cfg(feature = "test-tools")]
    retained: RefCell<Option<Retained<ClipboardButton>>>,
    width: f64,
    height: f64,
}
impl Panel {
    pub fn new(main: MainThreadMarker, parent: &NSView, width: f64, height: f64) -> Self {
        let root = NSView::new(main);
        root.setFrame(rect(0., 0., width, height));
        root.setHidden(true);
        parent.addSubview(&root);
        let text_scroll = NSScrollView::new(main);
        text_scroll.setHasVerticalScroller(true);
        text_scroll.setAutohidesScrollers(true);
        text_scroll.setDrawsBackground(false);
        let text = NSTextView::new(main);
        text.setEditable(false);
        text.setSelectable(true);
        text.setDrawsBackground(false);
        text.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
            13.,
            unsafe { NSFontWeightRegular },
        )));
        text.setTextColor(Some(&NSColor::labelColor()));
        text.setAccessibilityLabel(Some(&NSString::from_str("Clipboard content")));
        text_scroll.setDocumentView(Some(&text));
        root.addSubview(&text_scroll);
        let image = NSImageView::new(main);
        image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        image.setAccessibilityLabel(Some(&NSString::from_str("Clipboard image")));
        root.addSubview(&image);
        let metadata = NSTextField::wrappingLabelWithString(&NSString::from_str(""), main);
        metadata.setFont(Some(&NSFont::systemFontOfSize(11.)));
        metadata.setTextColor(Some(&NSColor::secondaryLabelColor()));
        metadata.setSelectable(true);
        root.addSubview(&metadata);
        Self {
            root,
            text,
            text_scroll,
            image,
            metadata,
            buttons: RefCell::new(vec![]),
            key: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            retained: RefCell::new(None),
            width,
            height,
        }
    }
    pub fn apply(
        &self,
        owner: &Controller,
        surface: Option<&Surface>,
        epoch: u64,
        dismissal: u64,
        list: &NSScrollView,
        hidden: bool,
    ) {
        let size = unsafe { self.root.superview() }
            .map(|v| v.bounds().size)
            .unwrap_or(NSSize::new(self.width, self.height));
        let width = size.width;
        let height = size.height;
        let Some(s) = surface else {
            self.root.setHidden(true);
            self.cleanup();
            list.setFrameSize(NSSize::new(width - 32., list.frame().size.height));
            return;
        };
        self.root.setHidden(false);
        let key = (epoch, dismissal, s.revision, s.selected_id.clone());
        if self.key.borrow().as_ref() != Some(&key) {
            self.cleanup();
            *self.key.borrow_mut() = Some(key);
            if let Some(parent) = unsafe { self.root.superview() } {
                for _ in 0..2 {
                    let button = ClipboardButton::alloc(self.root.mtm()).set_ivars(ButtonState {
                        binding: RefCell::new(None),
                    });
                    let button: Retained<ClipboardButton> =
                        unsafe { msg_send![super(button),initWithFrame:rect(0.,0.,70.,28.)] };
                    button.setBezelStyle(NSBezelStyle::Push);
                    button.setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
                    unsafe {
                        button.setTarget(Some(&*button));
                        button.setAction(Some(sel!(invoke:)));
                    }
                    parent.addSubview(&button);
                    self.buttons.borrow_mut().push(button);
                }
            }
        }
        // Root is a transparent overlay, constrained to the preview area so it never
        // steals the existing table/search mouse target.
        let list_width = (width - 32.) * 0.4;
        list.setFrameSize(NSSize::new(list_width, list.frame().size.height));
        let x = list_width + 28.;
        let w = width - x - 18.;
        self.root.setFrame(rect(x, 50., w, height - 170.));
        let h = self.root.frame().size.height;
        self.text_scroll.setFrame(rect(8., 125., w - 16., h - 133.));
        self.text.setFrame(rect(0., 0., w - 30., h - 133.));
        if let Some(container) = unsafe { self.text.textContainer() } {
            container.setWidthTracksTextView(true);
        }
        self.image.setFrame(rect(8., 125., w - 16., h - 133.));
        self.metadata.setFrame(rect(8., 36., w - 16., 84.));
        let mut metadata = if s.selected_id.is_some() {
            use unicode_segmentation::UnicodeSegmentation;
            let kind = if s.preview.is_some() || s.kind == "image" {
                "Image"
            } else if s.kind == "files" {
                "Files"
            } else {
                "Text"
            };
            let mut value = format!(
                "Source  {}\nType  {}",
                crate::native_rows::source_name(&s.source, hidden),
                kind
            );
            if kind == "Text" {
                value.push_str(&format!("\nCharacters  {}", s.text.graphemes(true).count()));
            }
            let date = chrono::DateTime::parse_from_rfc3339(&s.date)
                .ok()
                .map(|date| {
                    use objc2_foundation::{NSDate, NSDateFormatter, NSDateFormatterStyle};
                    let formatter = NSDateFormatter::new();
                    formatter.setDateStyle(NSDateFormatterStyle::MediumStyle);
                    formatter.setTimeStyle(NSDateFormatterStyle::ShortStyle);
                    formatter
                        .stringFromDate(&NSDate::dateWithTimeIntervalSince1970(
                            date.timestamp() as f64
                        ))
                        .to_string()
                })
                .unwrap_or_else(|| s.date.clone());
            value.push_str(&format!("\nCopied  {date}"));
            value
        } else {
            String::new()
        };
        let preview = s.selected_id.as_ref().and(s.preview.as_ref());
        if let Some(preview) = preview {
            use objc2_foundation::{NSByteCountFormatter, NSByteCountFormatterCountStyle};
            let size = NSByteCountFormatter::stringFromByteCount_countStyle(
                preview.bytes.min(i64::MAX as usize) as i64,
                NSByteCountFormatterCountStyle::Memory,
            );
            metadata.push_str(&format!(
                "\nDimensions  {} × {}\nSize  {}",
                preview.width, preview.height, size
            ));
        }
        self.metadata.setStringValue(&NSString::from_str(&metadata));
        let message = if s.selected_id.is_none() {
            "Select an item".into()
        } else if s.loading {
            "Loading preview…".into()
        } else if let Some(error) = &s.error {
            error.clone()
        } else if s.kind == "files" {
            s.files
                .iter()
                .map(|raw| {
                    objc2_foundation::NSURL::URLWithString(&NSString::from_str(raw))
                        .and_then(|u| u.path())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| raw.clone())
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else if s.kind == "image" && s.preview.is_none() {
            "Image preview is unavailable".into()
        } else {
            s.text.clone()
        };
        self.text.setString(&NSString::from_str(&message));
        self.text_scroll.setHidden(preview.is_some());
        self.image.setHidden(preview.is_none());
        self.image.setImage(
            preview
                .and_then(|p| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&p.png)))
                .as_deref(),
        );
        for (i, b) in self.buttons.borrow().iter().enumerate() {
            let (op, title) = match i {
                0 => ("pause", if s.paused { "Resume" } else { "Pause" }),
                _ => ("back", "Back"),
            };
            b.setFrame(if i == 1 {
                rect(20., 10., 60., 26.)
            } else {
                rect(list_width - 58., height - 122., 70., 26.)
            });
            b.setHidden(false);
            b.setTitle(&NSString::from_str(title));
            b.setAccessibilityLabel(Some(&NSString::from_str(title)));
            b.setEnabled(true);
            *b.ivars().binding.borrow_mut() = Some(Binding {
                owner: Weak::new(owner),
                operation: op.into(),
                id: s.selected_id.clone(),
                revision: s.revision,
                epoch,
                dismissal,
            });
        }
    }
    pub fn cleanup(&self) {
        for b in self.buttons.borrow_mut().drain(..) {
            b.ivars().binding.borrow_mut().take();
            b.setEnabled(false);
            unsafe {
                b.setTarget(None);
                b.setAction(None);
            }
            b.removeFromSuperview();
        }
        self.key.borrow_mut().take();
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self, action: &str) -> Result<(), String> {
        match action {
            "retain" => *self.retained.borrow_mut() = self.buttons.borrow().first().cloned(),
            "stale" => {
                if let Some(button) = self.retained.borrow().as_ref() {
                    unsafe {
                        let _: () = msg_send![&**button,invoke:None::<&objc2_foundation::NSObject>];
                    }
                }
            }
            "back" => {
                let button = self
                    .buttons
                    .borrow()
                    .get(1)
                    .cloned()
                    .ok_or("Clipboard Back unavailable")?;
                unsafe {
                    button.performClick(None);
                }
            }
            _ => (),
        }
        Ok(())
    }
    pub fn text(&self) -> Vec<String> {
        if self.root.isHidden() {
            vec![]
        } else {
            let mut v = vec![
                self.text.string().to_string(),
                self.metadata.stringValue().to_string(),
            ];
            v.extend(
                self.buttons
                    .borrow()
                    .iter()
                    .filter(|b| !b.isHidden())
                    .map(|b| b.title().to_string()),
            );
            v
        }
    }
    pub fn accessibility(&self) -> Vec<String> {
        if self.root.isHidden() {
            return vec![];
        }
        let mut text = vec![
            self.text.string().to_string(),
            self.metadata.stringValue().to_string(),
        ];
        for b in self.buttons.borrow().iter() {
            if b.isHidden() {
                continue;
            }
            let view: &NSView = b;
            if let Some(label) = view.accessibilityLabel() {
                text.push(label.to_string());
            }
        }
        if let Some(label) = self.text.accessibilityLabel() {
            text.push(label.to_string());
        }
        if !self.image.isHidden() {
            let view: &NSView = &self.image;
            if let Some(label) = view.accessibilityLabel() {
                text.push(label.to_string());
            }
        }
        text
    }
}
