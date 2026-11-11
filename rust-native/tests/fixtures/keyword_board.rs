//! Owned in-memory board and item component; no native pasteboard access.
#![allow(
    dead_code,
    reason = "Internal board state is shared with the subsequent keyword event/provider assembly"
)]
use super::keyword_core::{self as core, Object, Ptr};
use std::sync::Mutex;
#[derive(Default)]
pub(super) struct State {
    pub text: Vec<u8>,
    pub html: Vec<u8>,
    pub has_text: bool,
    pub has_html: bool,
    pub extra: bool,
    pub count: isize,
    pub clears: u32,
    pub writes: u32,
    pub clip: Vec<u16>,
    pub clipboard_reads: u32,
    pub pastes: u32,
    pub paste_text: Vec<u8>,
}
pub(super) static STATE: Mutex<State> = Mutex::new(State {
    text: Vec::new(),
    html: Vec::new(),
    has_text: false,
    has_html: false,
    extra: false,
    count: 0,
    clears: 0,
    writes: 0,
    clip: Vec::new(),
    clipboard_reads: 0,
    pastes: 0,
    paste_text: Vec::new(),
});
unsafe fn bytes<'a>(data: *const u8, n: usize) -> &'a [u8] {
    if n == 0 {
        &[]
    } else {
        assert!(!data.is_null());
        std::slice::from_raw_parts(data, n)
    }
}
// Preserve the original fixture's byte-to-unit behavior, including its lack of
// normalization/validation. Item UTF8 normally comes from validated product text.
fn clip_utf8(data: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let mut scalar = u32::from(data[i]);
        i += 1;
        if scalar >= 0xc0 {
            let more = if scalar < 0xe0 {
                1
            } else if scalar < 0xf0 {
                2
            } else {
                3
            };
            scalar &= if more == 1 {
                31
            } else if more == 2 {
                15
            } else {
                7
            };
            for _ in 0..more {
                assert!(i < data.len());
                scalar = (scalar << 6) | u32::from(data[i] & 63);
                i += 1;
            }
        }
        if scalar <= 0xffff {
            out.push(scalar as u16);
        } else {
            scalar -= 0x10000;
            out.push((0xd800 + (scalar >> 10)) as u16);
            out.push((0xdc00 + (scalar & 1023)) as u16);
        }
    }
    out
}
pub(super) fn replace(text: &[u8], html: &[u8], has_html: bool, extra: bool) {
    let mut state = STATE.lock().unwrap();
    state.text = text.to_vec();
    state.html = if has_html { html.to_vec() } else { Vec::new() };
    state.has_html = has_html;
    state.has_text = true;
    state.extra = extra;
    state.count = state.count.wrapping_add(1);
    state.clip = clip_utf8(text);
}
macro_rules! sdk {
    ($local:ident => $native:ident($($arg:ident: $ty:ty),*) -> $ret:ty $body:block) => {
        #[export_name = concat!("ruycast_keyword_fixture_", stringify!($native))]
        pub(super) unsafe extern "C" fn $local($($arg: $ty),*) -> $ret $body
    };
}
sdk!(item_create => item_create(text:*const u8,n:usize,html:*const u8,h:usize,has_html:bool)->Ptr {
    if core::call(c"item_create") || core::call(c"item_text") {return std::ptr::null_mut();}
    let item=core::object(13); (*item).utf8=bytes(text,n).to_vec();
    if has_html {
        // Preserve the original HTML fault cleanup after allocating the text item.
        if core::call(c"item_html") {core::release(item); return std::ptr::null_mut();}
        (*item).html=Some(bytes(html,h).to_vec());
    }
    item.cast()
});
sdk!(item_release => item_release(value:Ptr)->() {
    let item=value.cast::<Object>(); assert!(!item.is_null() && (*item).kind==13);
    core::call(c"item_release"); core::release(item);
});
sdk!(board_clear => board_clear()->isize {
    if core::call(c"board_clear") {return STATE.lock().unwrap().count;}
    let own={let mut state=STATE.lock().unwrap();
        state.text.clear(); state.html.clear(); state.has_text=false; state.has_html=false; state.extra=false; state.clip.clear();
        state.count=state.count.wrapping_add(1); state.clears=state.clears.wrapping_add(1); state.count};
    core::call(c"board_cleared"); own
});
sdk!(board_write => board_write(value:Ptr)->bool {
    let item=value.cast::<Object>(); assert!(!item.is_null() && (*item).kind==13);
    if core::call(c"board_write") {return false;}
    let partial=core::call(c"board_partial");
    replace(&(*item).utf8,(*item).html.as_deref().unwrap_or(&[]),!partial && (*item).html.is_some(),false);
    {let mut state=STATE.lock().unwrap(); state.writes=state.writes.wrapping_add(1);}
    core::call(c"board_written"); !partial
});
sdk!(board_change => board_change()->isize {
    let snapshot=STATE.lock().unwrap().count; core::call(c"board_change"); snapshot
});
unsafe fn ours(item: *mut Object, partial: bool) -> bool {
    let state = STATE.lock().unwrap();
    state.has_text
        && !state.extra
        && state.text == (*item).utf8
        && if partial {
            !state.has_html || ((*item).html.as_deref() == Some(state.html.as_slice()))
        } else {
            state.has_html == (*item).html.is_some()
                && (!state.has_html || (*item).html.as_deref() == Some(state.html.as_slice()))
        }
}
sdk!(board_matches => board_matches(value:Ptr)->bool {
    let item=value.cast::<Object>(); assert!(!item.is_null() && (*item).kind==13);
    let answer=ours(item,false); !core::call(c"board_matches") && answer
});
sdk!(board_ours => board_ours(value:Ptr)->bool {
    let item=value.cast::<Object>(); assert!(!item.is_null() && (*item).kind==13);
    let answer=ours(item,true); !core::call(c"board_ours") && answer
});
sdk!(board_external => board_external(text:*const u8,n:usize,html:*const u8,h:usize,has_html:bool,extra:bool)->() {
    replace(bytes(text,n),bytes(html,if has_html {h} else {0}),has_html,extra);
});
sdk!(clipboard => clipboard()->Ptr {
    {let mut state=STATE.lock().unwrap(); state.clipboard_reads=state.clipboard_reads.wrapping_add(1);}
    if core::call(c"clipboard") {return std::ptr::null_mut();}
    let clip = STATE.lock().unwrap().clip.clone();
    core::string(&clip).cast()
});
sdk!(set_clipboard => set_clipboard(units:*const u16,n:usize)->() {
    let data=if n==0 {&[]} else {assert!(!units.is_null()); std::slice::from_raw_parts(units,n)};
    let mut state=STATE.lock().unwrap(); state.count=state.count.wrapping_add(1);
    state.has_text=true; state.has_html=false; state.extra=true; state.clip=data.to_vec();
});
