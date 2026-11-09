// Source-included data-only endpoint. Compile with rustc --test and producer
// pins; the ordinary Cargo example cannot load the SDK or execute this route.
#![cfg_attr(test, no_main)]

#[cfg(not(test))]
fn main() {
    std::process::exit(2);
}

#[cfg(test)]
#[path = "../src/sdk/data_family_preentry.rs"]
pub mod data_family_preentry;
#[cfg(test)]
#[path = "../src/sdk/mod.rs"]
pub mod sdk_facade;
#[cfg(test)]
pub mod sdk {
    pub use crate::data_family_preentry;
    pub use crate::sdk_facade::*;
}
#[cfg(test)]
pub use sdk::{metadata, owned};
#[cfg(test)]
#[path = "../src/guardian.rs"]
pub mod guardian;
#[cfg(test)]
#[path = "../src/guardian_alive.rs"]
pub mod guardian_alive;
#[cfg(test)]
#[path = "../src/sdk/workspace.rs"]
pub mod workspace;

// Producer-generated Rust literals only. No runtime environment/file values
// construct Pins or authority. The producer binds this include as source.
#[cfg(test)]
include!(env!("F1_DATA_PINS_SOURCE"));

#[cfg(test)]
fn run() -> owned::Result<()> {
    let pins = make_data_pins();
    // First effect: the closed FoundationData admission, never a model fallback.
    let admission = sdk::data_family_preentry::enter(&pins)?;
    let fixture = admission.fixture();
    let fixture_path = fixture.path().to_owned();
    let context = owned::ModuleContext {
        home: fixture.clone(),
        cache: fixture.clone(),
        temp: fixture.clone(),
        applications: fixture,
        life: admission.life(),
    };
    let frame = workspace::Rect {
        origin: workspace::Point { x: 0.0, y: 0.0 },
        size: workspace::Point { x: 64.0, y: 64.0 },
    };
    let model = workspace::Workspace::new(
        context,
        vec![],
        vec![workspace::Screen {
            frame,
            visible: frame,
            scale: 1.0,
        }],
        workspace::Point { x: 0.0, y: 0.0 },
    )?;
    admission.prepare(model.clone())?;
    for _ in 0..2 {
        prove_foundation(&model, &fixture_path)?;
    }
    if model.frontmost()?.is_some() {
        return Err("application appeared in Foundation-only model".into());
    }
    Ok(())
}

#[cfg(test)]
fn source_class(name: &std::ffi::CStr) -> owned::Result<&'static objc2::runtime::AnyClass> {
    let source = metadata::loaded_class(name)?;
    // The sealed admission qualified these exact six class metadata identities.
    Ok(unsafe { &*(source.address() as *const objc2::runtime::AnyClass) })
}

#[cfg(test)]
fn prove_foundation(
    model: &std::sync::Arc<workspace::Workspace>,
    fixture: &std::path::Path,
) -> owned::Result<()> {
    use objc2::{
        msg_send,
        rc::{Retained, Weak, autoreleasepool},
        runtime::{AnyClass, AnyObject, Bool},
    };
    use std::ffi::c_void;
    let families = [
        source_class(c"NSString")?,
        source_class(c"NSData")?,
        source_class(c"NSURL")?,
        source_class(c"NSNumber")?,
        source_class(c"NSArray")?,
        source_class(c"NSDictionary")?,
    ];
    let retained = autoreleasepool(|_| -> owned::Result<Vec<Retained<AnyObject>>> {
        let mut values = Vec::new();
        let text = "Fixture\0 Ω 😀";
        let utf16 = text.encode_utf16().collect::<Vec<_>>();
        let allocated = unsafe { workspace::factory::allocate(families[0]) }?;
        let initialized: *mut AnyObject =
            unsafe { msg_send![allocated,initWithCharacters:utf16.as_ptr(),length:utf16.len()] };
        if initialized != allocated {
            return Err("NSString init did not preserve owned +1".into());
        }
        let string = unsafe { Retained::from_raw(initialized) }.ok_or("nil owned NSString")?;
        check_text(&string, text)?;
        let initialized = unsafe { workspace::factory::allocate(families[0]) }?;
        let bytes_text: *mut AnyObject = unsafe {
            msg_send![initialized,initWithBytes:text.as_ptr().cast::<c_void>(),length:text.len(),encoding:4_usize]
        };
        if bytes_text != initialized {
            return Err("NSString byte init changed owned +1".into());
        }
        let bytes_text =
            unsafe { Retained::from_raw(bytes_text) }.ok_or("nil UTF8 owned NSString")?;
        let equal: Bool = unsafe { msg_send![&*string,isEqualToString:&*bytes_text] };
        if !equal.as_bool() {
            return Err("two real NSString initializers disagree".into());
        }
        let payload = [0_u8, 7, 255, 42];
        let data: *mut AnyObject = unsafe {
            msg_send![families[1],dataWithBytes:payload.as_ptr().cast::<c_void>(),length:payload.len()]
        };
        let data = unsafe { Retained::retain(data) }.ok_or("nil +0 NSData")?;
        check_data(&data, &payload)?;
        let allocated = unsafe { workspace::factory::allocate(families[1]) }?;
        let data_init: *mut AnyObject = unsafe {
            msg_send![allocated,initWithBytes:payload.as_ptr().cast::<c_void>(),length:payload.len()]
        };
        if data_init != allocated {
            return Err("NSData init changed owned +1".into());
        }
        let data_init = unsafe { Retained::from_raw(data_init) }.ok_or("nil +1 NSData")?;
        check_data(&data_init, &payload)?;
        let empty: *mut AnyObject = unsafe {
            msg_send![families[1],dataWithBytes:std::ptr::null::<c_void>(),length:0_usize]
        };
        let empty = unsafe { Retained::retain(empty) }.ok_or("nil empty NSData")?;
        check_data(&empty, &[])?;
        let url_text = owned_text("https://example.com/owned/item")?;
        let url: *mut AnyObject = unsafe { msg_send![families[2],URLWithString:&*url_text] };
        let url = unsafe { Retained::retain(url) }.ok_or("nil owned NSURL")?;
        let is_file: Bool = unsafe { msg_send![&*url, isFileURL] };
        if is_file.as_bool() {
            return Err("network URL is a file URL".into());
        }
        let absolute: *mut AnyObject = unsafe { msg_send![&*url, absoluteString] };
        let host: *mut AnyObject = unsafe { msg_send![&*url, host] };
        let url_path: *mut AnyObject = unsafe { msg_send![&*url, path] };
        let component: *mut AnyObject = unsafe { msg_send![&*url, lastPathComponent] };
        for (p, expected) in [
            (absolute, "https://example.com/owned/item"),
            (host, "example.com"),
            (url_path, "/owned/item"),
            (component, "item"),
        ] {
            let result = unsafe { Retained::retain(p) }.ok_or("nil owned URL text result")?;
            check_text(&result, expected)?;
        }
        let path = owned_text(fixture.to_str().ok_or("UTF8 fixture path")?)?;
        let file1: *mut AnyObject = unsafe { msg_send![families[2],fileURLWithPath:&*path] };
        let file2: *mut AnyObject =
            unsafe { msg_send![families[2],fileURLWithPath:&*path,isDirectory:Bool::YES] };
        for p in [file1, file2] {
            let p = unsafe { Retained::retain(p) }.ok_or("nil owned file URL")?;
            let flag: Bool = unsafe { msg_send![&*p, isFileURL] };
            if !flag.as_bool() {
                return Err("owned fixture URL not file URL".into());
            }
            values.push(p);
        }
        let number: *mut AnyObject = unsafe { msg_send![families[3],numberWithBool:Bool::YES] };
        let number = unsafe { Retained::retain(number) }.ok_or("nil owned NSNumber")?;
        let flag: Bool = unsafe { msg_send![&*number, boolValue] };
        if !flag.as_bool() {
            return Err("owned Boolean value changed".into());
        }
        let false_number: *mut AnyObject =
            unsafe { msg_send![families[3],numberWithBool:Bool::NO] };
        let false_number = unsafe { Retained::retain(false_number) }.ok_or("nil false NSNumber")?;
        let false_value: Bool = unsafe { msg_send![&*false_number, boolValue] };
        if false_value.as_bool() {
            return Err("false NSNumber became true".into());
        }
        let objects = [
            Retained::as_ptr(&string).cast_mut(),
            Retained::as_ptr(&data).cast_mut(),
            Retained::as_ptr(&url).cast_mut(),
            Retained::as_ptr(&number).cast_mut(),
        ];
        let array: *mut AnyObject =
            unsafe { msg_send![families[4],arrayWithObjects:objects.as_ptr(),count:objects.len()] };
        let array = unsafe { Retained::retain(array) }.ok_or("nil +0 NSArray")?;
        check_array(&array, &objects)?;
        let allocated = unsafe { workspace::factory::allocate(families[4]) }?;
        let array_init: *mut AnyObject =
            unsafe { msg_send![allocated,initWithObjects:objects.as_ptr(),count:objects.len()] };
        if array_init != allocated {
            return Err("NSArray init changed owned +1".into());
        }
        let array_init = unsafe { Retained::from_raw(array_init) }.ok_or("nil +1 NSArray")?;
        check_array(&array_init, &objects)?;
        let allocated = unsafe { workspace::factory::allocate(families[5]) }?;
        let keys = [Retained::as_ptr(&string).cast_mut()];
        let dictionary_values = [Retained::as_ptr(&number).cast_mut()];
        let dictionary: *mut AnyObject = unsafe {
            msg_send![allocated,initWithObjects:dictionary_values.as_ptr(),forKeys:keys.as_ptr(),count:1_usize]
        };
        if dictionary != allocated {
            return Err("NSDictionary init changed owned +1".into());
        }
        let dictionary = unsafe { Retained::from_raw(dictionary) }.ok_or("nil +1 NSDictionary")?;
        values.extend([
            string,
            bytes_text,
            data,
            data_init,
            url_text,
            url,
            path,
            number,
            array,
            array_init,
            dictionary,
            empty,
            false_number,
        ]);
        let expected_families = [2, 2, 0, 0, 1, 1, 0, 2, 0, 3, 4, 4, 5, 1, 3];
        if values.len() != expected_families.len() {
            return Err("Foundation typed result roster differs".into());
        }
        for (value, index) in values.iter().zip(expected_families) {
            let family = families[index];
            let matches: Bool = unsafe { msg_send![&**value,isKindOfClass:family] };
            if !matches.as_bool() || std::ptr::eq(value.class(), family) {
                return Err(
                    "not the exact expected type-preserving owned Foundation subclass".into(),
                );
            }
        }
        Ok(values)
    })?;
    let weak = retained.iter().map(Weak::from_retained).collect::<Vec<_>>();
    let proxy_classes = retained
        .iter()
        .map(|value| value.class())
        .collect::<Vec<&AnyClass>>();
    if weak.iter().any(|weak| weak.load().is_none()) {
        return Err("owned +0/+1 value did not survive autoreleasepool".into());
    }
    let references = retained
        .iter()
        .map(|value| {
            workspace::factory::RetainedReference::acquire(
                Retained::as_ptr(value).cast_mut(),
                model,
            )
        })
        .collect::<owned::Result<Vec<_>>>()?;
    std::thread::spawn(move || -> owned::Result<()> {
        autoreleasepool(|_| {
            for reference in &references {
                let p = reference.pointer()?;
                let value: Bool = unsafe { msg_send![p,isKindOfClass:source_class(c"NSObject")?] };
                if !value.as_bool() {
                    return Err("immutable Foundation handoff lost NSObject ancestry".into());
                }
            }
            Ok(())
        })
    })
    .join()
    .map_err(|_| "Foundation immutable worker panicked")??;
    drop(retained);
    if weak.iter().any(|weak| weak.load().is_some()) {
        return Err("last Foundation owner not reclaimed".into());
    }
    for proxy in proxy_classes {
        if workspace::factory::live_owned_allocations(proxy)? != 0 {
            return Err("Foundation ledger not empty after pool/last owner".into());
        }
    }
    if workspace::factory::receiver_identity(17_usize as *mut AnyObject).is_ok() {
        return Err("unknown receiver accepted".into());
    }
    let allocated = unsafe { workspace::factory::allocate(families[0]) }?;
    let weak = Weak::new(unsafe { &*allocated });
    let invalid = [255_u8];
    let rejected: *mut AnyObject = unsafe {
        msg_send![allocated,initWithBytes:invalid.as_ptr().cast::<c_void>(),length:invalid.len(),encoding:4_usize]
    };
    if !rejected.is_null() || weak.load().is_some() {
        return Err("invalid UTF8 initializer did not consume fresh +1".into());
    }
    Ok(())
}

#[cfg(test)]
fn owned_text(text: &str) -> owned::Result<objc2::rc::Retained<objc2::runtime::AnyObject>> {
    use objc2::{msg_send, rc::Retained, runtime::AnyObject};
    let allocated = unsafe { workspace::factory::allocate(source_class(c"NSString")?) }?;
    let p: *mut AnyObject = unsafe {
        msg_send![allocated,initWithBytes:text.as_ptr().cast::<std::ffi::c_void>(),length:text.len(),encoding:4_usize]
    };
    if p != allocated {
        return Err("owned UTF8 string changed +1 receiver".into());
    }
    unsafe { Retained::from_raw(p) }.ok_or("nil owned UTF8 string".into())
}
#[cfg(test)]
fn check_text(value: &objc2::runtime::AnyObject, expected: &str) -> owned::Result<()> {
    use objc2::msg_send;
    let units = expected.encode_utf16().collect::<Vec<_>>();
    let count: usize = unsafe { msg_send![value, length] };
    if count != units.len() {
        return Err("owned text UTF16 length differs".into());
    }
    let mut copy = vec![0_u16; count];
    unsafe {
        let _: () = msg_send![value,getCharacters:copy.as_mut_ptr(),range:objc2_foundation::NSRange::new(0,count)];
    }
    if copy != units {
        return Err("owned text range differs".into());
    }
    for (index, expected) in units.iter().enumerate() {
        let actual: u16 = unsafe { msg_send![value,characterAtIndex:index] };
        if actual != *expected {
            return Err("owned UTF16 character differs".into());
        }
    }
    let length: usize = unsafe { msg_send![value,lengthOfBytesUsingEncoding:4_usize] };
    let p: *const std::ffi::c_char = unsafe { msg_send![value, UTF8String] };
    if length != expected.len()
        || p.is_null()
        || unsafe { std::slice::from_raw_parts(p.cast::<u8>(), length + 1) }
            != [expected.as_bytes(), &[0]].concat()
    {
        return Err("owned UTF8 bytes differ".into());
    }
    Ok(())
}
#[cfg(test)]
fn check_data(value: &objc2::runtime::AnyObject, expected: &[u8]) -> owned::Result<()> {
    use objc2::msg_send;
    let length: usize = unsafe { msg_send![value, length] };
    let p: *const std::ffi::c_void = unsafe { msg_send![value, bytes] };
    if length != expected.len()
        || (length > 0 && p.is_null())
        || (length > 0 && unsafe { std::slice::from_raw_parts(p.cast::<u8>(), length) } != expected)
    {
        return Err("owned NSData bytes differ".into());
    }
    Ok(())
}
#[cfg(test)]
fn check_array(
    value: &objc2::runtime::AnyObject,
    expected: &[*mut objc2::runtime::AnyObject],
) -> owned::Result<()> {
    use objc2::{msg_send, runtime::AnyObject};
    let count: usize = unsafe { msg_send![value, count] };
    if count != expected.len() {
        return Err("owned array count differs".into());
    }
    for (index, expected) in expected.iter().enumerate() {
        let p: *mut AnyObject = unsafe { msg_send![value,objectAtIndex:index] };
        if p != *expected {
            return Err("owned array ordered element differs".into());
        }
    }
    let mut state: objc2_foundation::NSFastEnumerationState = unsafe { std::mem::zeroed() };
    let mut objects = [std::ptr::null_mut::<AnyObject>(); 2];
    let mut enumerated = Vec::new();
    loop {
        let count: usize = unsafe {
            msg_send![value,countByEnumeratingWithState:&mut state,objects:objects.as_mut_ptr(),count:objects.len()]
        };
        if count == 0 {
            break;
        }
        if count > objects.len() {
            return Err("owned enumeration overran buffer".into());
        }
        enumerated.extend_from_slice(&objects[..count]);
        if enumerated.len() > expected.len() {
            return Err("owned enumeration did not terminate".into());
        }
    }
    if enumerated != expected {
        return Err("owned enumeration order differs".into());
    }
    Ok(())
}

#[cfg(test)]
#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    match std::panic::catch_unwind(|| match run() {
        Ok(()) => 0,
        Err(error) => {
            use std::io::Write;
            let _ = writeln!(
                std::io::stdout().lock(),
                "{}",
                serde_json::json!({ "ok": false, "error": error })
            );
            1
        }
    }) {
        Ok(status) => status,
        Err(payload) => {
            // A foreign panic payload may itself panic when dropped. The
            // isolated failing process exits; never unwind across this C ABI.
            std::mem::forget(payload);
            2
        }
    }
}
