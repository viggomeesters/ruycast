//! Selected Foundation data methods. Core alone qualifies and retains receiver
//! identities. No unknown object is messaged or cast to infer its authority.
use super::*;
use objc2_foundation::NSFastEnumerationState as EnumerationState;
use std::ffi::{c_char, c_void};
#[path = "workspace_foundation_data.rs"]
pub mod data;
pub use data::{Data, FoundationValue, Text, Url};

fn boundary<T>(fallback: T, body: impl FnOnce() -> Result<T>) -> T {
    if !foundation_entry_allowed() {
        return fallback;
    }
    let Ok(_entry) = enter_foundation_leaf() else {
        return fallback;
    };
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => fallback,
        Err(payload) => {
            std::mem::forget(payload);
            if let Some(installed) = INSTALLED.get() {
                installed.model.fault();
            }
            fallback
        }
    }
}
fn text(pointer: *mut AnyObject) -> Result<Arc<Text>> {
    match foundation_value(pointer)? {
        FoundationValue::String(value) => Ok(value),
        _ => Err("qualified Foundation receiver is not text".into()),
    }
}
fn bytes(pointer: *mut AnyObject) -> Result<Arc<Data>> {
    match foundation_value(pointer)? {
        FoundationValue::Data(value) => Ok(value),
        _ => Err("qualified Foundation receiver is not data".into()),
    }
}
fn url(pointer: *mut AnyObject) -> Result<Arc<Url>> {
    match foundation_value(pointer)? {
        FoundationValue::Url(value) => Ok(value),
        _ => Err("qualified Foundation receiver is not URL".into()),
    }
}
fn returned(value: FoundationValue) -> Result<*mut AnyObject> {
    let pointer = new_foundation_value(value)?;
    // SAFETY: consumes this call's fresh common factory +1.
    unsafe { autorelease_foundation(pointer) }
}
fn returned_text(value: Option<&Text>) -> Result<*mut AnyObject> {
    value.map_or(Ok(std::ptr::null_mut()), |value| {
        returned(FoundationValue::String(Arc::new(value.clone())))
    })
}
unsafe fn copy_slice<T: Copy>(pointer: *const T, length: usize) -> Result<Vec<T>> {
    if length == 0 {
        return Ok(Vec::new());
    }
    if pointer.is_null()
        || !(pointer as usize).is_multiple_of(std::mem::align_of::<T>())
        || length > isize::MAX as usize / std::mem::size_of::<T>()
    {
        return Err("invalid qualified Foundation source buffer".into());
    }
    // SAFETY: only called after Core has claimed a selected initializer. The
    // selected source ABI caller owns a readable buffer of this exact length.
    Ok(unsafe { std::slice::from_raw_parts(pointer, length) }.to_vec())
}
unsafe extern "C" fn init_characters(
    receiver: *mut AnyObject,
    _: Sel,
    pointer: *const u16,
    length: usize,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        bind_foundation_initializer(receiver, FoundationRole::String, || {
            Ok(FoundationValue::String(Arc::new(Text::new(unsafe {
                copy_slice(pointer, length)?
            }))))
        })
    })
}
unsafe extern "C" fn init_text_bytes(
    receiver: *mut AnyObject,
    _: Sel,
    pointer: *const c_void,
    length: usize,
    encoding: usize,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        bind_foundation_initializer(receiver, FoundationRole::String, || {
            if encoding != 4 {
                return Err("unqualified NSString encoding".into());
            }
            let input = unsafe { copy_slice(pointer.cast::<u8>(), length)? };
            Ok(FoundationValue::String(Arc::new(Text::from_utf8(&input)?)))
        })
    })
}
pub(super) fn string_bytes_initializer(
) -> unsafe extern "C" fn(*mut AnyObject, Sel, *const c_void, usize, usize) -> *mut AnyObject {
    init_text_bytes
}
unsafe extern "C" fn text_length(receiver: *mut AnyObject, _: Sel) -> usize {
    boundary(0, || Ok(text(receiver)?.utf16().len()))
}
unsafe extern "C" fn character(receiver: *mut AnyObject, _: Sel, index: usize) -> u16 {
    boundary(0, || {
        text(receiver)?
            .utf16()
            .get(index)
            .copied()
            .ok_or_else(|| "text index outside string".into())
    })
}
unsafe extern "C" fn characters(
    receiver: *mut AnyObject,
    _: Sel,
    output: *mut u16,
    range: objc2_foundation::NSRange,
) {
    boundary((), || {
        let value = text(receiver)?;
        let source = value.range(range.location, range.length)?;
        if source.is_empty() {
            return Ok(());
        }
        if output.is_null() || !(output as usize).is_multiple_of(std::mem::align_of::<u16>()) {
            return Err("invalid selected NSString output buffer".into());
        }
        // SAFETY: selected caller provides space for exactly range.length
        // UTF-16 units; source is our retained scalar and range is checked.
        unsafe { std::ptr::copy(source.as_ptr(), output, source.len()) };
        Ok(())
    });
}
unsafe extern "C" fn utf8(receiver: *mut AnyObject, _: Sel) -> *const c_char {
    boundary(std::ptr::null(), || {
        // Core retains this Arc for the receiver's normal ARC lifetime. The
        // temporary clone can drop without invalidating the borrowed buffer.
        Ok(text(receiver)?
            .utf8_with_nul()
            .map_or(std::ptr::null(), |bytes| bytes.as_ptr().cast()))
    })
}
unsafe extern "C" fn byte_length(receiver: *mut AnyObject, _: Sel, encoding: usize) -> usize {
    boundary(0, || {
        if encoding != 4 {
            return Err("unqualified NSString encoding".into());
        }
        Ok(text(receiver)?.utf8_len().unwrap_or(0))
    })
}
unsafe extern "C" fn text_equal(receiver: *mut AnyObject, _: Sel, other: *mut AnyObject) -> Bool {
    boundary(Bool::NO, || {
        Ok(Bool::new(text(receiver)?.utf16() == text(other)?.utf16()))
    })
}
unsafe extern "C" fn init_data(
    receiver: *mut AnyObject,
    _: Sel,
    pointer: *const c_void,
    length: usize,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        bind_foundation_initializer(receiver, FoundationRole::Data, || {
            Ok(FoundationValue::Data(Arc::new(Data::new(unsafe {
                copy_slice(pointer.cast::<u8>(), length)?
            }))))
        })
    })
}
unsafe extern "C" fn data_length(receiver: *mut AnyObject, _: Sel) -> usize {
    boundary(0, || Ok(bytes(receiver)?.bytes().len()))
}
unsafe extern "C" fn data_bytes(receiver: *mut AnyObject, _: Sel) -> *const c_void {
    boundary(std::ptr::null(), || {
        Ok(bytes(receiver)?.bytes().as_ptr().cast())
    })
}
unsafe extern "C" fn absolute_string(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        returned_text(Some(url(receiver)?.string()))
    })
}
unsafe extern "C" fn file_url(receiver: *mut AnyObject, _: Sel) -> Bool {
    boundary(Bool::NO, || Ok(Bool::new(url(receiver)?.is_file())))
}
unsafe extern "C" fn url_host(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        returned_text(url(receiver)?.host())
    })
}
unsafe extern "C" fn url_path(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        returned_text(url(receiver)?.path())
    })
}
unsafe extern "C" fn url_last_component(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        returned_text(url(receiver)?.last_component())
    })
}

unsafe extern "C" fn url_with_string(
    class: *mut AnyClass,
    _: Sel,
    input: *mut AnyObject,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        foundation_class(class, FoundationRole::Url)?;
        returned(FoundationValue::Url(Arc::new(Url::parse(
            text(input)?.as_ref(),
        )?)))
    })
}
unsafe extern "C" fn file_with_path(
    class: *mut AnyClass,
    _: Sel,
    input: *mut AnyObject,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        foundation_class(class, FoundationRole::Url)?;
        let input = text(input)?;
        // The common context grants only declared application paths or its
        // owned descriptor roots. Unknown/system paths are not queried here.
        let directory = foundation_path_is_directory(input.utf16())?;
        returned(FoundationValue::Url(Arc::new(Url::file_path(
            &input, directory,
        )?)))
    })
}
unsafe extern "C" fn file_with_path_directory(
    class: *mut AnyClass,
    _: Sel,
    input: *mut AnyObject,
    directory: Bool,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        foundation_class(class, FoundationRole::Url)?;
        returned(FoundationValue::Url(Arc::new(Url::file_path(
            text(input)?.as_ref(),
            directory.as_bool(),
        )?)))
    })
}
unsafe extern "C" fn data_with_bytes(
    class: *mut AnyClass,
    _: Sel,
    pointer: *const c_void,
    length: usize,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        foundation_class(class, FoundationRole::Data)?;
        returned(FoundationValue::Data(Arc::new(Data::new(unsafe {
            copy_slice(pointer.cast::<u8>(), length)?
        }))))
    })
}
unsafe extern "C" fn number_with_bool(class: *mut AnyClass, _: Sel, value: Bool) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        foundation_class(class, FoundationRole::Bool)?;
        returned(FoundationValue::Bool(value.as_bool()))
    })
}
unsafe extern "C" fn bool_value(receiver: *mut AnyObject, _: Sel) -> Bool {
    boundary(Bool::NO, || match foundation_value(receiver)? {
        FoundationValue::Bool(value) => Ok(Bool::new(value)),
        _ => Err("qualified Foundation receiver is not Boolean".into()),
    })
}
unsafe fn references(
    input: *const *mut AnyObject,
    count: usize,
) -> Result<Vec<FoundationReference>> {
    let pointers = unsafe { copy_slice(input, count)? };
    pointers.into_iter().map(foundation_reference).collect()
}
unsafe extern "C" fn init_array(
    receiver: *mut AnyObject,
    _: Sel,
    input: *const *mut AnyObject,
    count: usize,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        bind_foundation_array_initializer(receiver, || unsafe { references(input, count) })
    })
}
#[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
unsafe extern "C" fn array_with_objects(
    class: *mut AnyClass, selector: Sel, input: *const *mut AnyObject, count: usize,
) -> *mut AnyObject {
    unsafe { owned_array_with_objects(class, selector, input, count) }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[unsafe(naked)]
unsafe extern "C" fn array_with_objects(
    _: *mut AnyClass, _: Sel, _: *const *mut AnyObject, _: usize,
) -> *mut AnyObject {
    core::arch::naked_asm!("mov x4, x30", "b {checked}", checked = sym website_array_with_objects);
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
static WEBSITE_ARRAY_FACTORY: OnceLock<crate::sdk::metadata::MethodSnapshot> = OnceLock::new();

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn bind_website_array_factory() -> Result<()> {
    use crate::sdk::metadata;
    let class = metadata::loaded_class(c"NSArray")?;
    let array = unsafe { &*(class.address() as *const AnyClass) };
    let original = metadata::method(class, c"arrayWithObjects:count:", true)?
        .ok_or("Reference original array factory absent")?;
    if original.owner() != array.metaclass() as *const AnyClass as usize
        || original.encoding() != b"@32@0:8r^@16Q24"
        || original.code().image() != b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"
        || original.code().offset() != 0x58a78
    {
        return Err("Reference original array factory differs".into());
    }
    WEBSITE_ARRAY_FACTORY.set(original).map_err(|_| "Reference original array factory replay".into())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
unsafe extern "C" fn website_array_with_objects(
    class: *mut AnyClass, selector: Sel, input: *const *mut AnyObject, count: usize, caller: usize,
) -> *mut AnyObject {
    use crate::sdk::{metadata, private_network};
    if private_network::assembly_image_is_live()
        && count == 4
        && metadata::code_identity(caller).is_ok_and(|code| {
            code.offset() == 0xf5f34
                && code.image() == b"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit"
                && code.uuid() == [0xb6, 0xb4, 0xbd, 0xad, 0x64, 0x28, 0x3e, 0x64,
                    0x87, 0x47, 0x27, 0x57, 0x00, 0x10, 0x9b, 0x46]
        })
    {
        let result = (|| -> Result<*mut AnyObject> {
            let expected = [c"NSBitmapImageRep", c"NSPICTImageRep", c"NSPDFImageRep", c"_NSSVGImageRep"];
            let original = WEBSITE_ARRAY_FACTORY.get().ok_or("Reference decoder array factory unbound")?;
            if selector != objc2::sel!(arrayWithObjects:count:)
                || metadata::loaded_class(c"NSArray")?.address() != class as usize
                || metadata::code_identity(original.implementation())? != *original.code()
            {
                return Err("Reference decoder array factory identity differs".into());
            }
            for (pointer, role) in unsafe { copy_slice(input, count)? }.into_iter().zip(expected) {
                if metadata::loaded_class(role)?.address() != pointer as usize {
                    return Err("Reference decoder class order differs".into());
                }
            }
            let factory: unsafe extern "C" fn(*mut AnyClass, Sel, *const *mut AnyObject, usize) -> *mut AnyObject =
                unsafe { std::mem::transmute(original.implementation()) };
            Ok(unsafe { factory(class, selector, input, count) })
        })();
        return match result {
            Ok(array) => array,
            Err(_) => {
                private_network::report_reference_website_dispatch_refusal("Reference decoder array refused");
                unsafe { libc::_exit(89) }
            }
        };
    }
    unsafe { owned_array_with_objects(class, selector, input, count) }
}

unsafe fn owned_array_with_objects(
    class: *mut AnyClass,
    _: Sel,
    input: *const *mut AnyObject,
    count: usize,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        foundation_class(class, FoundationRole::Array)?;
        let values = unsafe { references(input, count)? };
        let pointer = new_foundation_array(values)?;
        // SAFETY: consumes this call's fresh common array factory +1.
        unsafe { autorelease_foundation(pointer) }
    })
}
unsafe extern "C" fn array_count(receiver: *mut AnyObject, _: Sel) -> usize {
    boundary(0, || Ok(foundation_array_elements(receiver)?.len()))
}
unsafe extern "C" fn array_at(receiver: *mut AnyObject, _: Sel, index: usize) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        foundation_array_elements(receiver)?
            .get(index)
            .ok_or("array index outside collection")?
            .as_ptr()
    })
}
unsafe extern "C" fn init_dictionary(
    receiver: *mut AnyObject,
    _: Sel,
    objects: *const *mut AnyObject,
    keys: *const *mut AnyObject,
    count: usize,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        bind_foundation_dictionary_initializer(receiver, || {
            let objects = unsafe { references(objects, count)? };
            let keys = unsafe { references(keys, count)? };
            Ok(keys.into_iter().zip(objects).collect())
        })
    })
}
unsafe extern "C" fn dictionary_count(receiver:*mut AnyObject,_:Sel)->usize {
    boundary(0,||Ok(foundation_dictionary_entries(receiver)?.len()))
}
unsafe extern "C" fn dictionary_at(receiver:*mut AnyObject,_:Sel,key:*mut AnyObject)->*mut AnyObject {
    boundary(std::ptr::null_mut(),|| {
        let key=foundation_utf16(key)?;
        let entries=foundation_dictionary_entries(receiver)?;
        let mut result=std::ptr::null_mut();
        for (candidate,value) in &entries {
            if foundation_utf16(candidate.as_ptr()?)?==key {result=value.as_ptr()?;break;}
        }
        foundation_dictionary_entries(receiver)?;
        Ok(result)
    })
}
fn dictionary_array(receiver:*mut AnyObject,keys:bool)->Result<*mut AnyObject> {
    // Keep this exact physical parent +1/serial across every allocator/guard;
    // a newly resolved dictionary at the same address is not the same witness.
    let parent=foundation_reference(receiver)?;
    let values=foundation_dictionary_entries(receiver)?.into_iter()
        .map(|(key,value)|if keys {key} else {value}).collect();
    let pointer=new_foundation_array(values)?;
    let pending=unsafe {super::super::factory::OwnedReceiver::from_owned(pointer)}?;
    foundation_dictionary_entries(receiver)?;
    if parent.as_ptr()!=Ok(receiver) {return Err("dictionary parent changed during array construction".into());}
    let returned=unsafe {autorelease_foundation(pending.into_raw())}?;
    foundation_dictionary_entries(receiver)?;
    if parent.as_ptr()!=Ok(receiver) {return Err("dictionary parent changed during array return".into());}
    Ok(returned)
}
unsafe extern "C" fn dictionary_keys(receiver:*mut AnyObject,_:Sel)->*mut AnyObject {
    boundary(std::ptr::null_mut(),||dictionary_array(receiver,true))
}
unsafe extern "C" fn dictionary_values(receiver:*mut AnyObject,_:Sel)->*mut AnyObject {
    boundary(std::ptr::null_mut(),||dictionary_array(receiver,false))
}
unsafe extern "C" fn dictionary_get_objects(receiver:*mut AnyObject,_:Sel,objects:*mut *mut AnyObject,
    keys:*mut *mut AnyObject,count:usize) {
    boundary((),|| {
        let entries=foundation_dictionary_entries(receiver)?;
        if count>entries.len() {return Err("dictionary copy count exceeds entries".into());}
        for pointer in [objects,keys] {
            if !pointer.is_null() && !(pointer as usize).is_multiple_of(std::mem::align_of::<*mut AnyObject>()) {
                return Err("dictionary copy output is unaligned".into());
            }
        }
        let resolved=entries[..count].iter().map(|(key,value)|Ok((key.as_ptr()?,value.as_ptr()?)))
            .collect::<Result<Vec<_>>>()?;
        foundation_dictionary_entries(receiver)?;
        for (index,(key,value)) in resolved.into_iter().enumerate() {
            if !objects.is_null() {unsafe {objects.add(index).write(value)};}
            if !keys.is_null() {unsafe {keys.add(index).write(key)};}
        }
        Ok(())
    })
}

static IMMUTABLE_COLLECTION: std::ffi::c_ulong = 0;
unsafe extern "C" fn enumerate(
    receiver: *mut AnyObject,
    _: Sel,
    state: *mut EnumerationState,
    output: *mut *mut AnyObject,
    capacity: usize,
) -> usize {
    boundary(0, || {
        let values = foundation_array_elements(receiver)?;
        unsafe {enumerate_references(&values,state,output,capacity,|| {
            foundation_array_elements(receiver)?;
            Ok(())
        })}
    })
}
unsafe extern "C" fn dictionary_enumerate(receiver:*mut AnyObject,_:Sel,state:*mut EnumerationState,
    output:*mut *mut AnyObject,capacity:usize)->usize {
    boundary(0,|| {
        let values=foundation_dictionary_entries(receiver)?.into_iter().map(|(key,_)|key).collect::<Vec<_>>();
        unsafe {enumerate_references(&values,state,output,capacity,|| {
            foundation_dictionary_entries(receiver)?;
            Ok(())
        })}
    })
}
unsafe fn enumerate_references(values:&[FoundationReference],state:*mut EnumerationState,
    output:*mut *mut AnyObject,capacity:usize,final_check:impl FnOnce()->Result<()>)->Result<usize> {
        if state.is_null()
            || !(state as usize).is_multiple_of(std::mem::align_of::<EnumerationState>())
        {
            return Err("invalid selected enumeration state".into());
        }
        // Do not hold a Rust mutable borrow across child/guardian callbacks:
        // those callbacks may reenter this exact selected caller's state.
        let original_state=unsafe {std::ptr::addr_of!((*state).state).read()};
        let offset = usize::try_from(original_state).map_err(|_| "enumeration offset overflow")?;
        if offset > values.len() {
            return Err("invalid enumeration offset".into());
        }
        let count = capacity.min(values.len() - offset);
        if count == 0 {
            return Ok(0);
        }
        if output.is_null()
            || !(output as usize).is_multiple_of(std::mem::align_of::<*mut AnyObject>())
        {
            return Err("invalid selected enumeration output".into());
        }
        // Resolve every child before exposing any part of this batch.
        let batch = values[offset..offset + count]
            .iter()
            .map(FoundationReference::as_ptr)
            .collect::<Result<Vec<_>>>()?;
        let next = std::ffi::c_ulong::try_from(offset + count)
            .map_err(|_| "enumeration state overflow")?;
        final_check()?;
        if unsafe {std::ptr::addr_of!((*state).state).read()}!=original_state {
            return Err("enumeration state changed during callback".into());
        }
        unsafe { std::ptr::copy_nonoverlapping(batch.as_ptr(), output, count) };
        // All potentially reentrant checks completed before this short borrow.
        let state=unsafe {&mut *state};
        state.itemsPtr = output;
        state.mutationsPtr = (&IMMUTABLE_COLLECTION as *const std::ffi::c_ulong).cast_mut();
        state.state = next;
        Ok(count)
}

// One typed roster supplies addresses and metadata signatures together.
macro_rules! roster {
    ($consumer:ident) => { $consumer! {
        (c"NSString",c"initWithCharacters:length:",false,init_characters,unsafe extern "C" fn(*mut AnyObject,Sel,*const u16,usize)->*mut AnyObject),
        (c"NSString",c"initWithBytes:length:encoding:",false,init_text_bytes,unsafe extern "C" fn(*mut AnyObject,Sel,*const c_void,usize,usize)->*mut AnyObject),
        (c"NSString",c"length",false,text_length,unsafe extern "C" fn(*mut AnyObject,Sel)->usize),
        (c"NSString",c"characterAtIndex:",false,character,unsafe extern "C" fn(*mut AnyObject,Sel,usize)->u16),
        (c"NSString",c"getCharacters:range:",false,characters,unsafe extern "C" fn(*mut AnyObject,Sel,*mut u16,objc2_foundation::NSRange)),
        (c"NSString",c"UTF8String",false,utf8,unsafe extern "C" fn(*mut AnyObject,Sel)->*const c_char),
        (c"NSString",c"lengthOfBytesUsingEncoding:",false,byte_length,unsafe extern "C" fn(*mut AnyObject,Sel,usize)->usize),
        (c"NSString",c"isEqualToString:",false,text_equal,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->Bool),
        (c"NSData",c"initWithBytes:length:",false,init_data,unsafe extern "C" fn(*mut AnyObject,Sel,*const c_void,usize)->*mut AnyObject),
        (c"NSData",c"dataWithBytes:length:",true,data_with_bytes,unsafe extern "C" fn(*mut AnyClass,Sel,*const c_void,usize)->*mut AnyObject),
        (c"NSData",c"length",false,data_length,unsafe extern "C" fn(*mut AnyObject,Sel)->usize),
        (c"NSData",c"bytes",false,data_bytes,unsafe extern "C" fn(*mut AnyObject,Sel)->*const c_void),
        (c"NSURL",c"URLWithString:",true,url_with_string,unsafe extern "C" fn(*mut AnyClass,Sel,*mut AnyObject)->*mut AnyObject),
        (c"NSURL",c"fileURLWithPath:",true,file_with_path,unsafe extern "C" fn(*mut AnyClass,Sel,*mut AnyObject)->*mut AnyObject),
        (c"NSURL",c"fileURLWithPath:isDirectory:",true,file_with_path_directory,unsafe extern "C" fn(*mut AnyClass,Sel,*mut AnyObject,Bool)->*mut AnyObject),
        (c"NSURL",c"absoluteString",false,absolute_string,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSURL",c"isFileURL",false,file_url,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
        (c"NSURL",c"host",false,url_host,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSURL",c"path",false,url_path,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSURL",c"lastPathComponent",false,url_last_component,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSNumber",c"numberWithBool:",true,number_with_bool,unsafe extern "C" fn(*mut AnyClass,Sel,Bool)->*mut AnyObject),
        (c"NSNumber",c"boolValue",false,bool_value,unsafe extern "C" fn(*mut AnyObject,Sel)->Bool),
        (c"NSArray",c"initWithObjects:count:",false,init_array,unsafe extern "C" fn(*mut AnyObject,Sel,*const *mut AnyObject,usize)->*mut AnyObject),
        (c"NSArray",c"arrayWithObjects:count:",true,array_with_objects,unsafe extern "C" fn(*mut AnyClass,Sel,*const *mut AnyObject,usize)->*mut AnyObject),
        (c"NSArray",c"count",false,array_count,unsafe extern "C" fn(*mut AnyObject,Sel)->usize),
        (c"NSArray",c"objectAtIndex:",false,array_at,unsafe extern "C" fn(*mut AnyObject,Sel,usize)->*mut AnyObject),
        (c"NSArray",c"countByEnumeratingWithState:objects:count:",false,enumerate,unsafe extern "C" fn(*mut AnyObject,Sel,*mut EnumerationState,*mut *mut AnyObject,usize)->usize),
        (c"NSDictionary",c"initWithObjects:forKeys:count:",false,init_dictionary,unsafe extern "C" fn(*mut AnyObject,Sel,*const *mut AnyObject,*const *mut AnyObject,usize)->*mut AnyObject),
        (c"NSDictionary",c"count",false,dictionary_count,unsafe extern "C" fn(*mut AnyObject,Sel)->usize),
        (c"NSDictionary",c"objectForKey:",false,dictionary_at,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->*mut AnyObject),
        (c"NSDictionary",c"objectForKeyedSubscript:",false,dictionary_at,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->*mut AnyObject),
        (c"NSDictionary",c"allKeys",false,dictionary_keys,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSDictionary",c"allValues",false,dictionary_values,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSDictionary",c"getObjects:andKeys:count:",false,dictionary_get_objects,unsafe extern "C" fn(*mut AnyObject,Sel,*mut *mut AnyObject,*mut *mut AnyObject,usize)),
        (c"NSDictionary",c"countByEnumeratingWithState:objects:count:",false,dictionary_enumerate,unsafe extern "C" fn(*mut AnyObject,Sel,*mut EnumerationState,*mut *mut AnyObject,usize)->usize),
    }};
}
macro_rules! exports {
    ($(($class:expr,$selector:expr,$meta:expr,$function:ident,$ty:ty)),*$(,)?) => {
        pub(super) fn replacements()->Vec<(&'static CStr,&'static CStr,bool,usize)> {
            vec![$(($class,$selector,$meta,$function as *const () as usize)),*]
        }
        pub(super) fn signatures()->Vec<(&'static CStr,&'static CStr,bool,Vec<objc2::encode::Encoding>)> {
            vec![$(($class,$selector,$meta,signature::<$ty>())),*]
        }
    };
}
fn signature<F: objc2::runtime::MethodImplementation>() -> Vec<objc2::encode::Encoding> {
    use objc2::encode::{Encode, EncodeArguments, EncodeReturn, Encoding};
    let mut result = vec![F::Return::ENCODING_RETURN, Encoding::Object, Sel::ENCODING];
    result.extend_from_slice(F::Arguments::ENCODINGS);
    result
}
roster!(exports);
