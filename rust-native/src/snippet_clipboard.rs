//! Main-owned payload write. Prepare the complete item before clearing the
//! destination; callers own permission/target/intent and all later paste effects.
use crate::snippet_output::Payload;
use objc2::{rc::Retained, runtime::ProtocolObject, MainThreadMarker};
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeHTML, NSPasteboardTypeString};
use objc2_foundation::{NSArray, NSRange, NSString};

// External pasteboard NSStrings need not be valid Unicode. Their UTF8String
// may be null; Display/to_string is therefore not a safe equality bridge.
// Compare exact UTF-16 (including NUL/surrogates) with bounded stack storage.
fn same_units(left: &NSString, right: &NSString) -> bool {
    let length = left.length();
    if length != right.length() {
        return false;
    }
    let mut a = [0u16; 4096];
    let mut b = [0u16; 4096];
    let mut offset = 0;
    while offset < length {
        let count = (length - offset).min(a.len());
        let range = NSRange::new(offset, count);
        // Both stack buffers hold count initialized u16s; the range lies
        // within the retained immutable NSString snapshots' lengths.
        unsafe {
            left.getCharacters_range(std::ptr::NonNull::from(&mut a[0]), range);
            right.getCharacters_range(std::ptr::NonNull::from(&mut b[0]), range);
        }
        if a[..count] != b[..count] {
            return false;
        }
        offset += count;
    }
    true
}

#[derive(Clone)]
pub struct PreparedItem {
    item: Retained<NSPasteboardItem>,
    extra: Vec<Retained<NSPasteboardItem>>,
    _main: MainThreadMarker,
}
pub enum Publish {
    Cancelled,
    Attempted(bool),
}
impl PreparedItem {
    /// Password/Keychain and explicit private utility output. All types are
    /// staged before clearing; the shared history observer rejects this marker.
    pub fn concealed(main:MainThreadMarker,text:&str)->Option<Self>{
        let prepared=Self::new(main,&crate::snippet_output::Payload{text:text.into(),html:None,cursor:None})?;
        if !prepared.item.setString_forType(&NSString::from_str("1"),&NSString::from_str("org.nspasteboard.ConcealedType")){return None}
        Some(prepared)
    }
    pub fn new(main: MainThreadMarker, payload: &Payload) -> Option<Self> {
        let item = NSPasteboardItem::new();
        if !item.setString_forType(&NSString::from_str(&payload.text), unsafe {
            NSPasteboardTypeString
        }) {
            return None;
        }
        if let Some(html) = &payload.html {
            if !item.setString_forType(&NSString::from_str(html), unsafe { NSPasteboardTypeHTML }) {
                return None;
            }
        }
        Some(Self { item, extra: vec![], _main: main })
    }
    pub fn history(main:MainThreadMarker,payload:&crate::clipboard_history::Payload)->Option<Self>{
        use crate::clipboard_history::{Payload,validate_files};
        use objc2_foundation::NSData;
        let mut values=vec![];
        match payload {
            Payload::Text(text)=>return Self::new(main,&crate::snippet_output::Payload{text:text.clone(),html:None,cursor:None}),
            Payload::Image(bytes)=>{
                let item=NSPasteboardItem::new();
                if !item.setData_forType(&NSData::with_bytes(bytes),&NSString::from_str("public.png")){return None}
                values.push(item);
            }
            Payload::Files(files)=>{
                validate_files(files).ok()?;
                for file in files {let item=NSPasteboardItem::new();if !item.setString_forType(&NSString::from_str(file),&NSString::from_str("public.file-url")){return None}values.push(item);}
            }
        }
        let item=values.remove(0);Some(Self{item,extra:values,_main:main})
    }
    pub fn write_to(&self, board: &NSPasteboard) -> bool {
        let items=std::iter::once(&self.item).chain(self.extra.iter()).map(|i|ProtocolObject::from_ref(&**i)).collect::<Vec<_>>();
        board.writeObjects(&NSArray::from_slice(&items))
    }
    /// Observed equality only; caller samples count before and after this read.
    pub fn matches(&self, board: &NSPasteboard) -> bool {
        self.compare(board, false)
    }
    pub fn observes_own_content(&self, board: &NSPasteboard) -> bool {
        self.compare(board, true)
    }
    fn compare(&self, board: &NSPasteboard, partial: bool) -> bool {
        let Some(items) = board.pasteboardItems() else {
            return false;
        };
        if items.count() != 1+self.extra.len() { return false; }
        if !self.extra.is_empty() || self.item.types().iter().any(|t|["public.png","public.file-url","org.nspasteboard.ConcealedType"].contains(&t.to_string().as_str())) {
            for (i,expected) in std::iter::once(&self.item).chain(self.extra.iter()).enumerate(){
                let actual=items.objectAtIndex(i);let kinds=actual.types();let wanted=expected.types();
                if kinds.is_empty()||(!partial&&kinds.len()!=wanted.len()){return false}
                for kind in kinds {if !wanted.iter().any(|k|same_units(&k,&kind)){return false}
                    let Some(a)=actual.dataForType(&kind) else{return false};let Some(b)=expected.dataForType(&kind) else{return false};if a.to_vec()!=b.to_vec(){return false}
                }
            }return true;
        }
        let actual = items.objectAtIndex(0);
        let expected_types = self.item.types();
        let actual_types = actual.types();
        if actual_types.is_empty()
            || actual_types
                .iter()
                .any(|v| !expected_types.iter().any(|e| same_units(&v, &e)))
            || (!partial
                && (actual_types.len() != expected_types.len()
                    || expected_types
                        .iter()
                        .any(|e| !actual_types.iter().any(|v| same_units(&v, &e)))))
        {
            return false;
        }
        for (index, kind) in unsafe { [NSPasteboardTypeString, NSPasteboardTypeHTML] }
            .into_iter()
            .enumerate()
        {
            let expected = self.item.stringForType(kind);
            let value = actual.stringForType(kind);
            if partial && index == 1 && value.is_none() {
                continue;
            }
            if !match (&expected, &value) {
                (Some(a), Some(b)) => same_units(a, b),
                (None, None) => true,
                _ => false,
            } {
                return false;
            }
        }
        true
    }
    pub fn publish(self, board: &NSPasteboard, authorize: impl FnOnce() -> bool) -> Publish {
        if !authorize() {
            return Publish::Cancelled;
        }
        board.clearContents();
        Publish::Attempted(self.write_to(board))
    }
}
pub fn write(main: MainThreadMarker, payload: &Payload, board: &NSPasteboard) -> bool {
    PreparedItem::new(main, payload)
        .is_some_and(|item| matches!(item.publish(board, || true), Publish::Attempted(true)))
}
