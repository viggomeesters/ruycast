//! Main-only app/AX authority for keyword replacement. AX text is held only
//! inside an active anchor; no text/history Debug, IPC or persistence interface.
use crate::{
    keyword_io::{Api, Ptr, Range},
    keyword_matcher, templates,
};
use objc2::MainThreadMarker;
use std::{ptr, rc::Rc};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Revoked,
    TimeoutBudget,
    UnsafeInput,
    InvalidData,
    TargetChanged,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Continuity of known metadata only: every choice still requires live
// activation/input guards and the caller's accepted generation authority.
pub enum Continuity {
    Pinned,
    EventGuardsRequired,
    Changed,
}
#[derive(Clone, Copy)]
pub struct Context {
    pub(crate) api: Api,
    _main: MainThreadMarker,
    fixture: bool,
}
pub(crate) struct Cf {
    pub ptr: Ptr,
    pub context: Context,
}
impl Drop for Cf {
    fn drop(&mut self) {
        unsafe { (self.context.api.release)(self.ptr) }
    }
}
struct App {
    ptr: Ptr,
    context: Context,
}
impl Drop for App {
    fn drop(&mut self) {
        unsafe { (self.context.api.app_release)(self.ptr) }
    }
}
struct TargetData {
    app: App,
    pid: i32,
    bundle: Option<String>,
}
#[derive(Clone)]
pub struct Target(Rc<TargetData>);
pub struct Anchor {
    target: Target,
    keyword: String,
    element: Option<Cf>,
    range: Option<Range>,
    value: Option<String>,
}
impl Context {
    pub(crate) fn fixture(&self) -> bool {
        self.fixture
    }
    pub fn new(main: MainThreadMarker, fixture: bool) -> Result<Self, String> {
        Ok(Self {
            api: Api::load(main, fixture)?,
            _main: main,
            fixture,
        })
    }
    pub fn accepts(&self, target: &Target) -> bool {
        self.fixture == target.0.app.context.fixture
    }
    pub(crate) fn check(&self, gate: &impl Fn() -> bool) -> Result<(), Error> {
        if gate() {
            Ok(())
        } else {
            Err(Error::Revoked)
        }
    }
    pub(crate) fn call<T>(
        &self,
        gate: &impl Fn() -> bool,
        work: impl FnOnce() -> T,
    ) -> Result<T, Error> {
        self.check(gate)?;
        let value = work();
        self.check(gate)?;
        Ok(value)
    }
    pub(crate) fn resource(
        &self,
        gate: &impl Fn() -> bool,
        work: impl FnOnce() -> Ptr,
    ) -> Result<Option<Cf>, Error> {
        self.check(gate)?;
        let ptr = work();
        let owned = (!ptr.is_null()).then(|| Cf {
            ptr,
            context: *self,
        });
        self.check(gate)?;
        Ok(owned)
    }
    fn typed(
        &self,
        gate: &impl Fn() -> bool,
        value: &Cf,
        expected: unsafe extern "C" fn() -> usize,
    ) -> Result<bool, Error> {
        let actual = self.call(gate, || unsafe { (self.api.type_id)(value.ptr) })?;
        Ok(actual == self.call(gate, || unsafe { expected() })?)
    }
    fn copy(&self, gate: &impl Fn() -> bool, element: &Cf, kind: u8) -> Result<Option<Cf>, Error> {
        let attribute = self.call(gate, || unsafe { (self.api.attr)(kind) })?;
        if attribute.is_null() {
            return Err(Error::InvalidData);
        }
        // Native permission/subrole may change during earlier metadata calls
        // without the caller's clock changing. Recheck the complete current
        // focus policy immediately before a sensitive editor-value copy.
        // Individual OS calls still cannot form an atomic focus snapshot.
        if kind == 3 && !self.safe_field(gate)? {
            return Err(Error::UnsafeInput);
        }
        self.check(gate)?;
        let mut raw: Ptr = ptr::null_mut();
        let result = unsafe { (self.api.ax_copy)(element.ptr, attribute, &mut raw) };
        // The SDK's nullable CF_RETURNS_RETAINED outvalue is initialized null.
        // Any returned CF object stays owned even on error or gate revocation.
        let value = (!raw.is_null()).then(|| Cf {
            ptr: raw,
            context: *self,
        });
        self.check(gate)?;
        Ok(if result == 0 { value } else { None })
    }
    fn timeout(&self, gate: &impl Fn() -> bool, element: &Cf) -> Result<(), Error> {
        if self.call(gate, || unsafe { (self.api.ax_timeout)(element.ptr, 0.01) })? != 0 {
            return Err(Error::TimeoutBudget);
        }
        Ok(())
    }
    pub(crate) fn text(
        &self,
        gate: &impl Fn() -> bool,
        value: &Cf,
    ) -> Result<Option<String>, Error> {
        self.text_mode(gate, value, false)
    }
    fn anchor_text(&self, gate: &impl Fn() -> bool, value: &Cf) -> Result<Option<String>, Error> {
        self.text_mode(gate, value, true)
    }
    fn text_mode(
        &self,
        gate: &impl Fn() -> bool,
        value: &Cf,
        strict: bool,
    ) -> Result<Option<String>, Error> {
        if !self.typed(gate, value, self.api.string_type)? {
            return Ok(None);
        }
        let count = self.call(gate, || unsafe { (self.api.string_len)(value.ptr) })?;
        let count = usize::try_from(count).map_err(|_| Error::InvalidData)?;
        // No arbitrary document cap: reject unaddressable worst-case UTF8
        // lengths before touching data, and grow output only fallibly in chunks.
        if count.checked_mul(3).is_none_or(|n| n > isize::MAX as usize) {
            return Err(Error::InvalidData);
        }
        let mut units = [0u16; 4096];
        let mut output = String::new();
        let mut high: Option<u16> = None;
        let mut offset = 0;
        while offset < count {
            let size = (count - offset).min(units.len());
            self.call(gate, || unsafe {
                (self.api.string_units)(
                    value.ptr,
                    Range {
                        location: offset as isize,
                        length: size as isize,
                    },
                    units.as_mut_ptr(),
                )
            })?;
            output
                .try_reserve(size * 3 + 3)
                .map_err(|_| Error::InvalidData)?;
            for unit in &units[..size] {
                if let Some(previous) = high.take() {
                    if (0xdc00..=0xdfff).contains(unit) {
                        let scalar =
                            0x10000 + ((previous as u32 - 0xd800) << 10) + (*unit as u32 - 0xdc00);
                        output.push(char::from_u32(scalar).ok_or(Error::InvalidData)?);
                        continue;
                    }
                    if strict {
                        return Err(Error::InvalidData);
                    }
                    output.push(char::REPLACEMENT_CHARACTER);
                }
                if (0xd800..=0xdbff).contains(unit) {
                    high = Some(*unit);
                } else if (0xdc00..=0xdfff).contains(unit) {
                    if strict {
                        return Err(Error::InvalidData);
                    }
                    output.push(char::REPLACEMENT_CHARACTER);
                } else {
                    output.push(char::from_u32(*unit as u32).ok_or(Error::InvalidData)?);
                }
            }
            offset += size;
        }
        if high.is_some() {
            if strict {
                return Err(Error::InvalidData);
            }
            output.try_reserve(3).map_err(|_| Error::InvalidData)?;
            output.push(char::REPLACEMENT_CHARACTER);
        }
        self.check(gate)?;
        Ok(Some(output))
    }
    fn basic_safe(&self, gate: &impl Fn() -> bool) -> Result<bool, Error> {
        let trusted = self.call(gate, || unsafe { (self.api.trusted)() != 0 })?;
        let secure = self.call(gate, || unsafe { (self.api.secure)() != 0 })?;
        Ok(trusted && !secure)
    }
    /// Ordinary typed events read only subrole, never full AX editor text.
    /// An explicit timeout-setter error fails closed before starting a copy.
    /// A successful setter is per-message budget, not total callback latency.
    pub fn safe_field(&self, gate: &impl Fn() -> bool) -> Result<bool, Error> {
        if !self.basic_safe(gate)? {
            return Ok(false);
        }
        let subrole = {
            let system = self.resource(gate, || unsafe { (self.api.ax_system)() })?;
            if let Some(system) = system {
                self.timeout(gate, &system)?;
                let focused = self.copy(gate, &system, 0)?;
                if let Some(focused) = focused {
                    if self.typed(gate, &focused, self.api.ax_type)? {
                        self.timeout(gate, &focused)?;
                        match self.copy(gate, &focused, 1)? {
                            Some(role) => self.text(gate, &role)?,
                            None => None,
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                return Err(Error::InvalidData);
            }
        };
        // All temporary native objects have retired before the final checks.
        Ok(self.basic_safe(gate)? && keyword_matcher::allows_expansion(false, subrole.as_deref()))
    }
    fn front_app(&self, gate: &impl Fn() -> bool) -> Result<Option<App>, Error> {
        self.check(gate)?;
        let raw = unsafe { (self.api.front)() };
        let app = (!raw.is_null()).then(|| App {
            ptr: raw,
            context: *self,
        });
        self.check(gate)?;
        Ok(app)
    }
    pub fn clipboard(&self, gate: &impl Fn() -> bool) -> Result<Option<String>, Error> {
        match self.resource(gate, || unsafe { (self.api.clipboard)() })? {
            Some(v) => self.text(gate, &v),
            None => Ok(None),
        }
    }
    pub fn frontmost(&self, gate: &impl Fn() -> bool) -> Result<Option<Target>, Error> {
        let Some(app) = self.front_app(gate)? else {
            return Ok(None);
        };
        let pid = self.call(gate, || unsafe { (self.api.app_pid)(app.ptr) })?;
        if pid <= 0 {
            return Err(Error::InvalidData);
        }
        let bundle = {
            match self.resource(gate, || unsafe { (self.api.app_bundle)(app.ptr) })? {
                Some(value) => Some(self.text(gate, &value)?.ok_or(Error::InvalidData)?),
                None => None,
            }
        };
        self.check(gate)?;
        Ok(Some(Target(Rc::new(TargetData { app, pid, bundle }))))
    }
    fn focused(&self, target: &Target, gate: &impl Fn() -> bool) -> Result<Option<Cf>, Error> {
        let app = self.resource(gate, || unsafe { (self.api.ax_app)(target.pid()) })?;
        let Some(app) = app else {
            return Err(Error::InvalidData);
        };
        self.timeout(gate, &app)?;
        let focused = self.copy(gate, &app, 0)?;
        match focused {
            Some(value) if self.typed(gate, &value, self.api.ax_type)? => {
                self.timeout(gate, &value)?;
                Ok(Some(value))
            }
            _ => Ok(None),
        }
    }
    fn selection(&self, gate: &impl Fn() -> bool, element: &Cf) -> Result<Option<Range>, Error> {
        let Some(value) = self.copy(gate, element, 2)? else {
            return Ok(None);
        };
        if !self.typed(gate, &value, self.api.value_type)?
            || self.call(gate, || unsafe { (self.api.value_kind)(value.ptr) })? != 4
        {
            return Ok(None);
        }
        let mut range = Range {
            location: 0,
            length: 0,
        };
        let success = self.call(gate, || unsafe {
            (self.api.value_range)(value.ptr, 4, (&mut range as *mut Range).cast()) != 0
        })?;
        Ok(success.then_some(range))
    }
    /// Capture after trigger delivery and before activating any choice field.
    /// Missing element/range/value remain distinct; this is not a grant to
    /// replace text without the caller's live event/activation authority.
    pub fn anchor(
        &self,
        target: &Target,
        keyword: &str,
        gate: &impl Fn() -> bool,
    ) -> Result<Anchor, Error> {
        if !self.accepts(target) {
            return Err(Error::InvalidData);
        }
        if !target.current(gate)? {
            return Err(Error::TargetChanged);
        }
        if !self.safe_field(gate)? {
            return Err(Error::UnsafeInput);
        }
        let element = self.focused(target, gate)?;
        let (range, value) = if let Some(element) = &element {
            let range = self.selection(gate, element)?;
            let value = match self.copy(gate, element, 3)? {
                Some(value) => self.anchor_text(gate, &value)?,
                None => None,
            };
            (range, value)
        } else {
            (None, None)
        };
        if let (Some(range), Some(value)) = (&range, &value) {
            if !keyword_at_caret(value, *range, keyword) {
                return Err(Error::TargetChanged);
            }
        }
        if !target.current(gate)? {
            return Err(Error::TargetChanged);
        }
        if !self.safe_field(gate)? {
            return Err(Error::UnsafeInput);
        }
        Ok(Anchor {
            target: target.clone(),
            keyword: keyword.to_owned(),
            element,
            range,
            value,
        })
    }
}
fn keyword_at_caret(value: &str, range: Range, keyword: &str) -> bool {
    let Ok(at) = usize::try_from(range.location) else {
        return false;
    };
    if range.length != 0 {
        return false;
    }
    let mut units = 0;
    for (byte, ch) in value.char_indices() {
        if units == at {
            return canonical_suffix(&value[..byte], keyword);
        }
        units += ch.len_utf16();
        // A known caret inside a surrogate pair is not a scalar boundary.
        if units > at {
            return false;
        }
    }
    units == at && canonical_suffix(value, keyword)
}

fn canonical_suffix(text: &str, suffix: &str) -> bool {
    let count = suffix.graphemes(true).count();
    if count == 0 {
        return true;
    }
    text.grapheme_indices(true)
        .rev()
        .nth(count - 1)
        .is_some_and(|(at, _)| templates::canonical(&text[at..]) == templates::canonical(suffix))
}
impl Target {
    pub fn matches_notification(
        &self,
        app: &objc2_app_kit::NSRunningApplication,
        gate: &impl Fn() -> bool,
    ) -> Result<bool, Error> {
        let context = self.0.app.context;
        if context.call(gate, || unsafe {
            (context.api.app_terminated)(self.0.app.ptr)
        })? {
            return Ok(false);
        }
        let matches = context.call(gate, || unsafe {
            (context.api.app_matches_notification)(
                self.0.app.ptr,
                std::ptr::from_ref(app).cast_mut().cast(),
            )
        })?;
        Ok(matches
            && !context.call(gate, || unsafe {
                (context.api.app_terminated)(self.0.app.ptr)
            })?)
    }
    pub fn pid(&self) -> i32 {
        self.0.pid
    }
    pub fn bundle(&self) -> Option<&str> {
        self.0.bundle.as_deref()
    }
    pub fn current(&self, gate: &impl Fn() -> bool) -> Result<bool, Error> {
        let context = self.0.app.context;
        if context.call(gate, || unsafe {
            (context.api.app_terminated)(self.0.app.ptr)
        })? {
            return Ok(false);
        }
        let equal = {
            let front = context.front_app(gate)?;
            match front {
                Some(front) => context.call(gate, || unsafe {
                    (context.api.app_equal)(self.0.app.ptr, front.ptr)
                })?,
                None => false,
            }
        };
        Ok(equal
            && !context.call(gate, || unsafe {
                (context.api.app_terminated)(self.0.app.ptr)
            })?)
    }
    /// Caller marks possible activation before entry and never re-mints a
    /// replacement authority from a successful activation or fresh PID lookup.
    pub fn activate(&self, gate: &impl Fn() -> bool) -> Result<bool, Error> {
        let context = self.0.app.context;
        if context.call(gate, || unsafe {
            (context.api.app_terminated)(self.0.app.ptr)
        })? {
            return Ok(false);
        }
        context.call(gate, || unsafe {
            (context.api.app_activate)(self.0.app.ptr)
        })
    }
}
impl Anchor {
    pub fn verify(&self, gate: &impl Fn() -> bool) -> Result<Continuity, Error> {
        let context = self.target.0.app.context;
        if !self.target.current(gate)? {
            return Ok(Continuity::Changed);
        }
        if !context.safe_field(gate)? {
            return Err(Error::UnsafeInput);
        }
        let continuity = if let Some(expected) = &self.element {
            let Some(current) = context.focused(&self.target, gate)? else {
                return Ok(Continuity::Changed);
            };
            if !context.call(gate, || unsafe {
                (context.api.equal)(expected.ptr, current.ptr) != 0
            })? {
                return Ok(Continuity::Changed);
            }
            if let Some(range) = self.range {
                if context.selection(gate, &current)? != Some(range) {
                    return Ok(Continuity::Changed);
                }
            }
            if let Some(expected) = &self.value {
                let Some(value) = context.copy(gate, &current, 3)? else {
                    return Ok(Continuity::Changed);
                };
                let Some(value) = context.anchor_text(gate, &value)? else {
                    return Ok(Continuity::Changed);
                };
                if templates::canonical(&value) != templates::canonical(expected) {
                    return Ok(Continuity::Changed);
                }
                // Canonical equality can change UTF16 length before the caret.
                // Keep the keyword at the actual unchanged UTF16 selection,
                // not merely at the same apparent string value.
                if self
                    .range
                    .is_some_and(|range| !keyword_at_caret(&value, range, &self.keyword))
                {
                    return Ok(Continuity::Changed);
                }
            }
            Continuity::Pinned
        } else {
            Continuity::EventGuardsRequired
        };
        if !self.target.current(gate)? {
            return Ok(Continuity::Changed);
        }
        if !context.safe_field(gate)? {
            return Err(Error::UnsafeInput);
        }
        Ok(continuity)
    }
}
