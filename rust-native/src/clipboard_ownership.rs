//! Main-only pasteboard observation. Inflight means defer/reobserve, not that
//! an external change in that interval belongs to us. No content is retained.
use std::{cell::Cell, rc::Rc};
#[derive(Default)]
pub struct OwnWrites {
    count: Cell<Option<isize>>,
    depth: Cell<usize>,
}
pub struct Writing(Rc<OwnWrites>);
impl OwnWrites {
    pub fn get(&self) -> Option<isize> {
        self.count.get()
    }
    pub fn set(&self, count: Option<isize>) {
        self.count.set(count);
    }
    pub fn active(&self) -> bool {
        self.depth.get() != 0
    }
    pub fn allows_capture(&self, count: isize) -> bool {
        !self.active() && self.count.get() != Some(count)
    }
    pub fn begin(self: &Rc<Self>) -> Option<Writing> {
        let next = self.depth.get().checked_add(1)?;
        self.depth.set(next);
        Some(Writing(self.clone()))
    }
}
impl Drop for Writing {
    fn drop(&mut self) {
        self.0.depth.set(self.0.depth.get().saturating_sub(1));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_writes_defer_but_do_not_own_external_replacement() {
        let own = Rc::new(OwnWrites::default());
        let a = own.begin().unwrap();
        own.set(Some(3));
        let b = own.begin().unwrap();
        assert!(!own.allows_capture(4));
        drop(a);
        assert!(own.active());
        drop(b);
        assert!(!own.allows_capture(3));
        assert!(own.allows_capture(4));
    }
}
