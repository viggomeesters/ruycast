//! Standalone own-image import observation. std-only, no constructors,
//! interposition tables, Objective-C, native package or source execution.
use std::ffi::CStr;
use std::{collections::BTreeSet, ptr};
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct ImportSpec {
    pub symbol: &'static CStr,
    pub original: usize,
    pub replacement: usize,
}

#[derive(Clone, Debug)]
pub struct ImportReplacement {
    pub symbol: &'static CStr,
    pub original: usize,
    pub replacement: usize,
    slot: usize,
}
impl ImportReplacement {
    /// # Safety
    /// `slot` must be an actual, readable import cell retained for the complete
    /// module lifetime. It is not a JSON/address assertion supplied by a receipt.
    pub unsafe fn from_import_cell(
        symbol: &'static CStr,
        original: usize,
        replacement: usize,
        slot: *const usize,
    ) -> Result<Self> {
        if slot.is_null() || original == 0 || replacement == 0 || original == replacement {
            return Err("invalid import binding".into());
        }
        Ok(Self {
            symbol,
            original,
            replacement,
            slot: slot as usize,
        })
    }
    fn selected(&self) -> usize {
        unsafe { (self.slot as *const usize).read_volatile() }
    }
    /// Read-only actual loader observation. This does not grant write access.
    pub fn selected_address(&self) -> usize {
        self.selected()
    }
    /// Address of the parsed, lifetime-pinned main-image import cell. A separate
    /// source-reviewed installer owns permission changes/compare-and-patch.
    pub fn cell_address(&self) -> usize {
        self.slot
    }
}

/// A read-only observation of an original Main import. This is not a patch,
/// image, symbol-type, object-lifetime or source-provenance capability.
/// Only the shared actual Main parser may construct its private cell binding.
#[derive(Clone, Debug)]
pub struct UnpatchedImportObservation {
    pub symbol: &'static CStr,
    original: usize,
    slot: usize,
}
impl UnpatchedImportObservation {
    pub fn original_address(&self) -> usize { self.original }
    pub fn cell_address(&self) -> usize { self.slot }
    pub fn selected_address(&self) -> usize {
        unsafe { (self.slot as *const usize).read_volatile() }
    }
    pub fn verify_unpatched(&self) -> Result<()> {
        if self.original==0 || self.selected_address()!=self.original {
            return Err(format!("original Main import changed/patched: {:?}", self.symbol));
        }
        Ok(())
    }
}

struct MainImportCell {
    symbol: &'static CStr,
    slot: usize,
}

unsafe extern "C" {
    fn _dyld_image_count() -> u32;
    fn _dyld_get_image_header(index: u32) -> *const u8;
    fn _dyld_get_image_vmaddr_slide(index: u32) -> isize;
}

pub fn observe_main_imports(specs: &[ImportSpec]) -> Result<Vec<ImportReplacement>> {
    if specs.is_empty()
        || specs.len() > 4096
        || specs
            .iter()
            .any(|s| s.original == 0 || s.replacement == 0 || s.original == s.replacement)
        || specs
            .iter()
            .map(|s| s.symbol)
            .collect::<BTreeSet<_>>()
            .len()
            != specs.len()
    {
        return Err("invalid/duplicate explicit import specification".into());
    }
    let symbols=specs.iter().map(|row|row.symbol).collect::<Vec<_>>();
    observe_main_cells(&symbols)?.into_iter().map(|cell| {
        let spec=specs.iter().find(|row|row.symbol==cell.symbol)
            .ok_or("parsed import is outside explicit specification")?;
        unsafe {ImportReplacement::from_import_cell(spec.symbol,spec.original,spec.replacement,cell.slot as *const usize)}
    }).collect()
}

/// Observe one original cell per explicitly declared symbol, without inventing
/// a replacement. The caller must separately qualify each expected source
/// address and its type/image/lifetime. Never supplies write permission.
pub fn observe_unpatched_main_imports(
    specs: &[(&'static CStr,usize)],
) -> Result<Vec<UnpatchedImportObservation>> {
    if specs.is_empty() || specs.len()>4096 || specs.iter().any(|(_,address)|*address==0)
        || specs.iter().map(|(symbol,_)|*symbol).collect::<BTreeSet<_>>().len()!=specs.len() {
        return Err("invalid/duplicate original Main import specification".into());
    }
    let symbols=specs.iter().map(|(symbol,_)|*symbol).collect::<Vec<_>>();
    let cells=observe_main_cells(&symbols)?;
    if cells.len()!=specs.len() || cells.iter().map(|cell|cell.symbol).collect::<BTreeSet<_>>().len()!=specs.len() {
        return Err("original Main import missing or duplicated".into());
    }
    let mut result=Vec::with_capacity(cells.len());
    for cell in cells {
        if cell.slot==0 || !cell.slot.is_multiple_of(std::mem::align_of::<usize>()) {
            return Err("unaligned original Main import cell".into());
        }
        let original=specs.iter().find(|(symbol,_)|*symbol==cell.symbol)
            .map(|(_,address)|*address).ok_or("original Main import outside specification")?;
        let observed=UnpatchedImportObservation{symbol:cell.symbol,original,slot:cell.slot};
        observed.verify_unpatched()?;
        result.push(observed);
    }
    for row in &result { row.verify_unpatched()?; }
    Ok(result)
}

// One parser owns actual MH_EXECUTE selection, LINKEDIT/symbol/string/section
// bounds and private cell discovery for both APIs. It issues no patch object.
fn observe_main_cells(symbols_requested: &[&'static CStr]) -> Result<Vec<MainImportCell>> {
    unsafe {
        let word = |p: *const u8| ptr::read_unaligned(p.cast::<u32>());
        let images = _dyld_image_count();
        if images > 4096 {
            return Err("own loader image bounds".into());
        }
        // Inserted dylibs can occupy index0. Select MH_EXECUTE, never assume
        // the first image is the fake/endpoint being qualified.
        let (image, header) = (0..images)
            .filter_map(|i| {
                let h = _dyld_get_image_header(i);
                (!h.is_null() && word(h) == 0xfeedfacf && word(h.add(12)) == 2).then_some((i, h))
            })
            .next()
            .ok_or("own main executable unavailable")?;
        let wide = |p: *const u8| ptr::read_unaligned(p.cast::<u64>());
        if word(header) != 0xfeedfacf {
            return Err("own image must be Mach-O64".into());
        }
        let count = word(header.add(16)) as usize;
        let bytes = word(header.add(20)) as usize;
        if count > 4096 || bytes > 16 * 1024 * 1024 {
            return Err("own load-command bounds".into());
        }
        let slide = _dyld_get_image_vmaddr_slide(image);
        let commands = header.add(32);
        let mut offset = 0;
        let mut link = None;
        let mut symbols = None;
        let mut indirect = None;
        let mut sections = vec![];
        for _ in 0..count {
            if offset + 8 > bytes {
                return Err("truncated own load command".into());
            }
            let p = commands.add(offset);
            let kind = word(p);
            let size = word(p.add(4)) as usize;
            if size < 8 || offset + size > bytes {
                return Err("invalid own load command".into());
            }
            match kind {
                0x19 if size >= 72 => {
                    let name = std::slice::from_raw_parts(p.add(8), 16);
                    let vm = wide(p.add(24));
                    let file = wide(p.add(40));
                    let file_size = wide(p.add(48));
                    if name.starts_with(b"__LINKEDIT\0") {
                        link = Some((
                            (vm as isize + slide) as usize,
                            file as usize,
                            file_size as usize,
                        ));
                    }
                    let n = word(p.add(64)) as usize;
                    if n > 4096 || 72 + n * 80 > size {
                        return Err("invalid own sections".into());
                    }
                    for i in 0..n {
                        let sec = p.add(72 + i * 80);
                        let flags = word(sec.add(64)) & 0xff;
                        if flags == 6 || flags == 7 {
                            let addr = wide(sec.add(32));
                            let len = wide(sec.add(40)) as usize;
                            if len > 16 * 1024 * 1024 || !len.is_multiple_of(8) {
                                return Err("invalid import section".into());
                            }
                            sections.push((
                                (addr as isize + slide) as usize,
                                len / 8,
                                word(sec.add(68)) as usize,
                            ));
                        }
                    }
                }
                0x2 if size >= 24 => {
                    symbols = Some((
                        word(p.add(8)) as usize,
                        word(p.add(12)) as usize,
                        word(p.add(16)) as usize,
                        word(p.add(20)) as usize,
                    ))
                }
                0xb if size >= 80 => {
                    indirect = Some((word(p.add(56)) as usize, word(p.add(60)) as usize))
                }
                _ => (),
            };
            offset += size;
        }
        let (address, file, size) = link.ok_or("own LINKEDIT missing")?;
        let (sym_offset, sym_count, str_offset, str_bytes) =
            symbols.ok_or("own symbol table missing")?;
        let (ind_offset, ind_count) = indirect.ok_or("own indirect table missing")?;
        let range = |start: usize, len: usize| -> Result<*const u8> {
            let end = start.checked_add(len).ok_or("LINKEDIT overflow")?;
            if start < file || end > file.checked_add(size).ok_or("LINKEDIT overflow")? {
                return Err("LINKEDIT table out of bounds".into());
            }
            Ok((address + (start - file)) as *const u8)
        };
        let syms = range(
            sym_offset,
            sym_count.checked_mul(16).ok_or("symbol overflow")?,
        )?;
        let strings = std::slice::from_raw_parts(range(str_offset, str_bytes)?, str_bytes);
        let indices = range(
            ind_offset,
            ind_count.checked_mul(4).ok_or("indirect overflow")?,
        )?;
        let mut result = vec![];
        for (address, count, first) in sections {
            if first.checked_add(count).is_none_or(|n| n > ind_count) {
                return Err("import index out of bounds".into());
            }
            for j in 0..count {
                let index = word(indices.add((first + j) * 4)) as usize;
                if index & 0xc0000000 != 0 {
                    continue;
                }
                if index >= sym_count {
                    return Err("symbol index out of bounds".into());
                }
                let name = word(syms.add(index * 16)) as usize;
                let rest = strings.get(name..).ok_or("symbol string out of bounds")?;
                let nul = rest
                    .iter()
                    .position(|b| *b == 0)
                    .ok_or("unterminated own symbol")?;
                let name = &rest[..nul];
                for &symbol in symbols_requested {
                    if name.strip_prefix(b"_") == Some(symbol.to_bytes()) {
                        result.push(MainImportCell{symbol,slot:address + j * 8});
                    }
                }
            }
        }
        if result.is_empty() {
            return Err("no owned bootstrap import cells found".into());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod unpatched_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize,Ordering};

    #[test]
    fn own_cell_change_refuses_original_observation_without_patch_authority() {
        let cell=AtomicUsize::new(0x1234);
        // A synthetic own cell tests only the leaf's equality/recheck contract,
        // never real Main discovery, source custody or a Swift object grant.
        let row=UnpatchedImportObservation{symbol:c"owned_fixture",original:0x1234,
            slot:std::ptr::from_ref(&cell) as usize};
        assert_eq!(row.original_address(),0x1234);
        assert_eq!(row.cell_address(),std::ptr::from_ref(&cell) as usize);
        row.verify_unpatched().unwrap();
        cell.store(0x5678,Ordering::SeqCst);
        assert_eq!(row.selected_address(),0x5678);
        assert!(row.verify_unpatched().is_err());
    }
}
