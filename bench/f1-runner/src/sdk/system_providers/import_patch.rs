//! Exact main-image import-cell transaction. No global framework interposition.
//! Only sdk::observe_main_imports may supply cells; no caller address is accepted.
use crate::sdk::{
    observe_main_imports,
    owned::{AliveLease, Result},
    ImportReplacement, ImportSpec,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicUsize, Ordering},
};
#[repr(C, packed(4))]
struct Region {
    protection: i32,
    max_protection: i32,
    inheritance: i32,
    shared: u32,
    reserved: u32,
    offset: u64,
    behavior: i32,
    user_wired_count: u16,
}
unsafe extern "C" {
    static vm_page_size: u32;
    static mach_task_self_: u32;
    fn mach_vm_region(
        task: u32,
        address: *mut u64,
        size: *mut u64,
        flavor: i32,
        info: *mut i32,
        count: *mut u32,
        object: *mut u32,
    ) -> i32;
    fn mach_port_deallocate(task: u32, name: u32) -> i32;
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page {
    pub address: usize,
    pub size: usize,
    pub protection: i32,
}
fn geometry() -> Result<usize> {
    let page = unsafe { vm_page_size } as usize;
    if !(4096..=65536).contains(&page) || !page.is_power_of_two() {
        return Err("invalid own loader page geometry".into());
    }
    Ok(page)
}
fn page_for(cell: usize) -> Result<Page> {
    let size = geometry()?;
    if !cell.is_multiple_of(std::mem::align_of::<AtomicUsize>()) {
        return Err("unaligned/PAC cell unsupported".into());
    }
    let base = cell & !(size - 1);
    let end = cell
        .checked_add(std::mem::size_of::<usize>())
        .ok_or("own address overflow")?;
    if end > base + size {
        return Err("cross-page import cell".into());
    }
    let mut address = base as u64;
    let mut region_size = 0;
    let mut info = std::mem::MaybeUninit::<Region>::zeroed();
    let mut count = (std::mem::size_of::<Region>() / 4) as u32;
    let mut object = 0;
    let task = unsafe { mach_task_self_ };
    let status = unsafe {
        mach_vm_region(
            task,
            &mut address,
            &mut region_size,
            9,
            info.as_mut_ptr().cast(),
            &mut count,
            &mut object,
        )
    };
    if object != 0 {
        unsafe {
            mach_port_deallocate(task, object);
        }
    }
    if status != 0
        || count as usize != std::mem::size_of::<Region>() / 4
        || address > base as u64
        || address
            .checked_add(region_size)
            .is_none_or(|n| n < (base + size) as u64)
    {
        return Err("own import page is not a complete mapped region".into());
    }
    let info = unsafe { info.assume_init() };
    let protection = info.protection;
    if protection & libc::PROT_READ == 0
        || protection & libc::PROT_EXEC != 0
        || info.max_protection & libc::PROT_WRITE == 0
    {
        return Err("unqualified executable/unwritable import page".into());
    }
    Ok(Page {
        address: base,
        size,
        protection,
    })
}
fn protect(page: &Page, writable: bool) -> Result<()> {
    let protection = if writable {
        page.protection | libc::PROT_WRITE
    } else {
        page.protection
    };
    if unsafe { libc::mprotect(page.address as *mut libc::c_void, page.size, protection) } != 0 {
        return Err("own import page protection transition failed".into());
    }
    Ok(())
}
fn compare_write(binding: &ImportReplacement, before: usize, after: usize) -> Result<()> {
    let cell = unsafe { &*(binding.cell_address() as *const AtomicUsize) };
    cell.compare_exchange(before, after, Ordering::SeqCst, Ordering::SeqCst)
        .map(|_| ())
        .map_err(|_| "import cell changed during transaction".into())
}
struct Transaction {
    pages: Vec<Page>,
    written: Vec<ImportReplacement>,
    committed: bool,
}
impl Transaction {
    fn restore_protections(&self) -> Result<()> {
        for page in &self.pages {
            protect(page, false)?;
        }
        Ok(())
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.committed {
            for page in &self.pages {
                if protect(page, true).is_err() {
                    unsafe { libc::_exit(92) }
                }
            }
            for binding in self.written.iter().rev() {
                if compare_write(binding, binding.replacement, binding.original).is_err() {
                    unsafe { libc::_exit(92) }
                }
            }
        }
        if self.restore_protections().is_err() {
            unsafe { libc::_exit(92) }
        }
    }
}
/// The caller supplies its hardcoded source-owned symbol roster. No lazy stub,
/// guessed original, signed/chained/PAC representation or partial roster passes.
/// Replacements stay installed for process lifetime, including late callbacks;
/// retirement revokes their lifecycle instead of restoring real SDK fallbacks.
pub fn install(specs: &[ImportSpec], life: &dyn AliveLease) -> Result<Vec<ImportReplacement>> {
    install_exact_presence(specs, specs, life)
}
/// An explicit source-owned role requires every selected symbol and absence of
/// every other symbol in its fixed known universe. Never infer optional cells.
pub(crate) fn install_exact_presence(
    specs: &[ImportSpec],
    universe: &[ImportSpec],
    life: &dyn AliveLease,
) -> Result<Vec<ImportReplacement>> {
    let expected = specs.iter().map(|s| s.symbol).collect::<BTreeSet<_>>();
    if specs.is_empty() || expected.len() != specs.len() || specs.iter().any(|selected| {
        !universe.iter().any(|known|known.symbol==selected.symbol
            && known.original==selected.original && known.replacement==selected.replacement)
    }) {
        return Err("invalid fixed import role specification".into());
    }
    life.before_effect()?;
    let before = observe_main_imports(universe)?;
    let actual = before.iter().map(|b| b.symbol).collect::<BTreeSet<_>>();
    if expected != actual || before.is_empty() {
        return Err("main-image import role presence/absence differs".into());
    }
    let mut pages = BTreeMap::new();
    let mut cells = BTreeSet::new();
    for binding in &before {
        if !cells.insert(binding.cell_address()) || binding.selected_address() != binding.original {
            return Err("duplicate/already-bound/lazy/PAC/changed import cell".into());
        }
        let page = page_for(binding.cell_address())?;
        pages.insert(page.address, page);
    }
    let mut transaction = Transaction {
        pages: vec![],
        written: vec![],
        committed: false,
    };
    for page in pages.into_values() {
        life.before_effect()?;
        protect(&page, true)?;
        transaction.pages.push(page);
    }
    for binding in &before {
        life.before_effect()?;
        compare_write(binding, binding.original, binding.replacement)?;
        transaction.written.push(binding.clone());
    }
    transaction.restore_protections()?;
    // The last dynamic guard may change cells or page protections. Capture and
    // verify the actual final state only after it returns; no lease callback follows.
    life.before_effect()?;
    let after = observe_main_imports(universe)?;
    let before_set = before
        .iter()
        .map(|b| (b.symbol, b.cell_address(), b.replacement))
        .collect::<BTreeSet<_>>();
    let after_set = after
        .iter()
        .map(|b| (b.symbol, b.cell_address(), b.selected_address()))
        .collect::<BTreeSet<_>>();
    if before_set != after_set {
        return Err("post-patch actual import cells differ".into());
    }
    for page in &transaction.pages {
        if page_for(page.address)? != *page {
            return Err("original own import protection not restored".into());
        }
    }
    transaction.committed = true;
    Ok(after)
}
/// Own mmap allocator canary, unrelated to monitor inventory. Checks the actual
/// loader scalar against writable/read-only own pages and exact restoration.
pub fn own_geometry_canary() -> Result<usize> {
    let size = geometry()?;
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err("own canary mapping failed".into());
    }
    struct Mapping(*mut libc::c_void, usize);
    impl Drop for Mapping {
        fn drop(&mut self) {
            unsafe {
                libc::munmap(self.0, self.1);
            }
        }
    }
    let _mapping = Mapping(ptr, size);
    unsafe {
        ptr.cast::<usize>().write_volatile(0xF126);
    }
    let mut page = page_for(ptr as usize)?;
    if page.size != size {
        return Err("own canary geometry differs".into());
    }
    page.protection = libc::PROT_READ;
    protect(&page, false)?;
    let observed = page_for(ptr as usize)?;
    if observed != page {
        return Err("own canary protection differs".into());
    }
    protect(&page, true)?;
    unsafe {
        ptr.cast::<usize>().write_volatile(0xF190);
    }
    protect(&page, false)?;
    if page_for(ptr as usize)? != page || unsafe { ptr.cast::<usize>().read_volatile() } != 0xF190 {
        return Err("own canary restoration failed".into());
    }
    Ok(size)
}
