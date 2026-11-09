//! Target-bound synchronous output. Issuance is not OS delivery; clipboard and
//! keyboard possibility are deliberately separate, including panic paths.
use crate::{
    clipboard_ownership::OwnWrites,
    keyword_matcher::Replacement,
    keyword_target::{Cf, Context, Target},
    snippet_output,
};
use std::{cell::Cell, ptr, rc::Rc};
pub const GENERATED_TAG: i64 = 0x5255535443415354;
#[derive(Clone, Copy, Debug, Default)]
pub struct Outcome {
    pub keyboard_possible: bool,
    pub complete: bool,
}
impl Outcome {
    pub fn consume_trigger(self) -> bool {
        self.complete || self.keyboard_possible
    }
}
thread_local! {static EMITTING:Cell<bool>=const{Cell::new(false)};}
struct Emission;
impl Drop for Emission {
    fn drop(&mut self) {
        EMITTING.with(|v| v.set(false));
    }
}
struct Pair {
    context: Context,
    up: Cf,
    down_possible: bool,
    up_possible: bool,
}
impl Pair {
    fn release_key(&mut self, clear: bool) {
        if !self.down_possible || self.up_possible {
            return;
        }
        if clear {
            unsafe {
                (self.context.api.set_unicode)(self.up.ptr, 0, ptr::null());
                (self.context.api.set_flags)(self.up.ptr, 0);
            }
        }
        self.up_possible = true;
        unsafe {
            (self.context.api.post)(0, self.up.ptr);
        }
    }
}
impl Drop for Pair {
    fn drop(&mut self) {
        self.release_key(true);
    }
}
struct Emitter<'a, G: Fn() -> bool, X: Fn() -> bool> {
    context: Context,
    target: &'a Target,
    gate: &'a G,
    extra: X,
    possible: &'a Cell<bool>,
    issued: Cell<bool>,
    failed: Cell<bool>,
}
impl<'a, G: Fn() -> bool, X: Fn() -> bool> Emitter<'a, G, X> {
    fn new(
        context: Context,
        target: &'a Target,
        gate: &'a G,
        extra: X,
        possible: &'a Cell<bool>,
    ) -> Self {
        Self {
            context,
            target,
            gate,
            extra,
            possible,
            issued: Cell::new(false),
            failed: Cell::new(false),
        }
    }
    fn valid(&self) -> bool {
        self.context.accepts(self.target)
            && (self.gate)()
            && (self.extra)()
            && self.context.safe_field(self.gate).unwrap_or(false)
            && (self.extra)()
            && self.target.current(self.gate).unwrap_or(false)
            && (self.gate)()
    }
    fn pair(&self, key: u16, flags: u64, text: Option<&str>) -> bool {
        if self.failed.get() {
            return false;
        }
        let result = (|| {
            if !self.valid() {
                return false;
            }
            let units = text.map(|v| v.encode_utf16().collect::<Vec<_>>());
            let down = self
                .context
                .resource(self.gate, || unsafe {
                    (self.context.api.keyboard_event)(ptr::null_mut(), key, true)
                })
                .ok()
                .flatten();
            let Some(down) = down else {
                return false;
            };
            let up = self
                .context
                .resource(self.gate, || unsafe {
                    (self.context.api.keyboard_event)(ptr::null_mut(), key, false)
                })
                .ok()
                .flatten();
            let Some(up) = up else {
                return false;
            };
            for event in [&down, &up] {
                if self
                    .context
                    .call(self.gate, || unsafe {
                        (self.context.api.set_flags)(event.ptr, flags)
                    })
                    .is_err()
                {
                    return false;
                }
                if self
                    .context
                    .call(self.gate, || unsafe {
                        (self.context.api.set_field)(event.ptr, 42, GENERATED_TAG)
                    })
                    .is_err()
                {
                    return false;
                }
                if let Some(units) = &units {
                    if self
                        .context
                        .call(self.gate, || unsafe {
                            (self.context.api.set_unicode)(event.ptr, units.len(), units.as_ptr())
                        })
                        .is_err()
                    {
                        return false;
                    }
                }
            }
            let mut pair = Pair {
                context: self.context,
                up,
                down_possible: false,
                up_possible: false,
            };
            if !self.valid() {
                return false;
            }
            self.issued.set(true);
            self.possible.set(true);
            pair.down_possible = true;
            unsafe {
                (self.context.api.post)(0, down.ptr);
            }
            let live = self.valid();
            pair.release_key(!live);
            live && self.valid()
        })();
        if !result {
            self.failed.set(true);
        }
        result
    }
    fn outcome(&self, complete: bool) -> Outcome {
        Outcome {
            keyboard_possible: self.issued.get(),
            complete: complete && !self.failed.get() && self.valid(),
        }
    }
}
pub fn apply_plain(
    context: Context,
    target: &Target,
    replacement: &Replacement,
    gate: &impl Fn() -> bool,
    possible: &Cell<bool>,
) -> Outcome {
    if !context.accepts(target) || EMITTING.with(|v| v.replace(true)) {
        return Outcome::default();
    }
    let _emission = Emission;
    let output = Emitter::new(context, target, gate, || true, possible);
    if !output.valid() {
        return Outcome::default();
    }
    let done = replacement.try_apply(
        |key| output.pair(key, 0, None),
        |text| output.pair(0, 0, Some(text)),
    );
    output.outcome(done)
}
struct Item {
    ptr: crate::keyword_io::Ptr,
    context: Context,
}
impl Drop for Item {
    fn drop(&mut self) {
        unsafe {
            (self.context.api.item_release)(self.ptr);
        }
    }
}
impl Item {
    fn new(
        context: Context,
        payload: &snippet_output::Payload,
        gate: &impl Fn() -> bool,
    ) -> Option<Self> {
        if !gate() {
            return None;
        }
        let html = payload.html.as_deref();
        let ptr = unsafe {
            (context.api.item_create)(
                payload.text.as_ptr(),
                payload.text.len(),
                html.map_or(ptr::null(), |v| v.as_ptr()),
                html.map_or(0, str::len),
                html.is_some(),
            )
        };
        let item = (!ptr.is_null()).then(|| Self { ptr, context });
        if !gate() {
            return None;
        }
        item
    }
}
/// Only a fully issued paste creates this delayed cursor intention. The caller
/// binds a weak original request/input lease, not fresh PID or current clocks.
pub struct Cursor {
    target: Target,
    steps: usize,
    context: Context,
}
impl Cursor {
    pub fn batch(
        mut self,
        gate: &impl Fn() -> bool,
        possible: &Cell<bool>,
    ) -> (Outcome, Option<Self>) {
        if EMITTING.with(|v| v.replace(true)) {
            return (Outcome::default(), None);
        }
        let _emission = Emission;
        let started = std::time::Instant::now();
        let outcome = {
            let output = Emitter::new(self.context, &self.target, gate, || true, possible);
            let mut count = 0;
            while self.steps > 0 {
                if !output.pair(123, 0, None) {
                    return (output.outcome(false), None);
                }
                self.steps -= 1;
                count += 1;
                if count >= 32 || started.elapsed() >= std::time::Duration::from_millis(8) {
                    break;
                }
            }
            output.outcome(self.steps == 0)
        };
        let complete = self.steps == 0;
        if !gate() {
            return (outcome, None);
        }
        (outcome, (!complete).then_some(self))
    }
}
pub struct Rich {
    pub outcome: Outcome,
    pub cursor: Option<Cursor>,
}
/// Full item preparation precedes any mutation. After clear/write the board may
/// have changed even when final trigger passes; no general restore is promised.
pub struct RichEffects<'a> {
    pub keyboard_possible: &'a Cell<bool>,
    pub clipboard_possible: &'a Cell<bool>,
    pub own: &'a Rc<OwnWrites>,
}
pub fn apply_rich(
    context: Context,
    target: &Target,
    replacement: &Replacement,
    mode: &str,
    gate: &impl Fn() -> bool,
    effects: RichEffects<'_>,
) -> Rich {
    let RichEffects {
        keyboard_possible,
        clipboard_possible,
        own,
    } = effects;
    let refused = || Rich {
        outcome: Outcome::default(),
        cursor: None,
    };
    if !context.accepts(target) || EMITTING.with(|v| v.replace(true)) {
        return refused();
    }
    let _emission = Emission;
    let initial = Emitter::new(context, target, gate, || true, keyboard_possible);
    if !initial.valid() {
        return refused();
    }
    let payload = snippet_output::render(
        replacement.text(),
        Some(mode),
        replacement.cursor().and_then(|v| isize::try_from(v).ok()),
    );
    let steps = payload
        .cursor
        .and_then(|v| usize::try_from(v).ok())
        .map(|v| snippet_output::cursor_steps(&payload.text, v).min(10_000))
        .unwrap_or(0);
    let Some(item) = Item::new(context, &payload, gate) else {
        return refused();
    };
    if !initial.valid() {
        return refused();
    }
    let Some(_writing) = own.begin() else {
        return refused();
    };
    let previous = unsafe { (context.api.board_change)() };
    if !initial.valid() {
        return refused();
    }
    clipboard_possible.set(true); // Possible board effect before native entry.
    let cleared = unsafe { (context.api.board_clear)() };
    if cleared == previous {
        return refused();
    }
    own.set(Some(cleared));
    if !initial.valid() || unsafe { (context.api.board_change)() } != cleared {
        return refused();
    }
    let written = unsafe { (context.api.board_write)(item.ptr) };
    let before = unsafe { (context.api.board_change)() };
    let matches = unsafe { (context.api.board_matches)(item.ptr) };
    let ours = matches || unsafe { (context.api.board_ours)(item.ptr) };
    let after = unsafe { (context.api.board_change)() };
    if ours && before == after {
        own.set(Some(after));
    }
    if !written || !matches || before != after || !initial.valid() {
        return refused();
    }
    // Count guards prevent observed external replacement between erases/paste;
    // this is not an atomic pasteboard transaction or an ABA guarantee.
    let output = Emitter::new(
        context,
        target,
        gate,
        || unsafe { (context.api.board_change)() == after },
        keyboard_possible,
    );
    for _ in 0..replacement.erase_count() {
        if !output.pair(51, 0, None) {
            return Rich {
                outcome: output.outcome(false),
                cursor: None,
            };
        }
    }
    let complete = output.pair(9, 1 << 20, None);
    let outcome = output.outcome(complete);
    let cursor = (outcome.complete && steps > 0).then(|| Cursor {
        target: target.clone(),
        steps,
        context,
    });
    Rich { outcome, cursor }
}
