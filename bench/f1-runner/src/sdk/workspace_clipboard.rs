//! Board/Item methods use the common arena's typed model and Foundation
//! references. No personal pasteboard, raw object probe, or second registry.
use super::*;
use crate::sdk::clipboard;

macro_rules! roster {
    ($consumer:ident) => { $consumer! {
        (c"NSPasteboard",c"generalPasteboard",true,general,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
        (c"NSPasteboard",c"pasteboardWithUniqueName",true,unique,unsafe extern "C" fn(*mut AnyClass,Sel)->*mut AnyObject),
        (c"NSPasteboard",c"pasteboardWithName:",true,named,unsafe extern "C" fn(*mut AnyClass,Sel,*mut AnyObject)->*mut AnyObject),
        (c"NSPasteboard",c"name",false,name,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSPasteboard",c"changeCount",false,change_count,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
        (c"NSPasteboard",c"clearContents",false,clear,unsafe extern "C" fn(*mut AnyObject,Sel)->isize),
        (c"NSPasteboard",c"releaseGlobally",false,release_global,unsafe extern "C" fn(*mut AnyObject,Sel)),
        (c"NSPasteboard",c"types",false,board_types,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSPasteboard",c"pasteboardItems",false,items,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSPasteboard",c"stringForType:",false,board_string,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->*mut AnyObject),
        (c"NSPasteboard",c"dataForType:",false,board_data,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->*mut AnyObject),
        (c"NSPasteboard",c"setString:forType:",false,board_set_string,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,*mut AnyObject)->Bool),
        (c"NSPasteboard",c"setData:forType:",false,board_set_data,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,*mut AnyObject)->Bool),
        (c"NSPasteboard",c"writeObjects:",false,write_objects,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->Bool),
        (c"NSPasteboard",c"readObjectsForClasses:options:",false,read_objects,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,*mut AnyObject)->*mut AnyObject),
        (c"NSPasteboardItem",c"init",false,init_item,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSPasteboardItem",c"types",false,item_types,unsafe extern "C" fn(*mut AnyObject,Sel)->*mut AnyObject),
        (c"NSPasteboardItem",c"stringForType:",false,item_string,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->*mut AnyObject),
        (c"NSPasteboardItem",c"dataForType:",false,item_data,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject)->*mut AnyObject),
        (c"NSPasteboardItem",c"setString:forType:",false,item_set_string,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,*mut AnyObject)->Bool),
        (c"NSPasteboardItem",c"setData:forType:",false,item_set_data,unsafe extern "C" fn(*mut AnyObject,Sel,*mut AnyObject,*mut AnyObject)->Bool),
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
use foundation::{Data, FoundationValue, Text, Url};

/// Explicit isolated common-arena probe. Its caller has prepared the genuine
/// eight typed factories using the existing authentic compiler-pin producer.
/// This does not initialize the original app or access a desktop pasteboard.
#[cfg(test)]
pub(crate) fn prove_private_board(model:&Arc<Workspace>)->*mut AnyObject {
    let constants=prepare_clipboard_constants(model).unwrap();
    struct Guard<'a>(&'a ClipboardConstants);
    impl crate::sdk::owned::AliveLease for Guard<'_> {
        fn is_alive(&self)->bool {self.0.pure_model().is_ok()}
        fn before_effect(&self)->Result<()> {self.0.verify()}
    }
    let imports=crate::sdk::import_patch::install(&constants.import_specs().unwrap(),&Guard(&constants)).unwrap();
    assert_eq!(imports.len(),5);
    assert!(imports.iter().all(|row|row.selected_address()==row.replacement));
    constants.verify_pure().unwrap();
    let class=crate::sdk::metadata::loaded_class(c"NSPasteboard").unwrap().address() as *mut AnyClass;
    let board=unsafe{general(class,objc2::sel!(generalPasteboard))};
    assert!(!board.is_null());
    assert_eq!(board,unsafe{general(class,objc2::sel!(generalPasteboard))});
    let text=new_foundation_value(FoundationValue::String(Arc::new(Text::new("private 🦀".encode_utf16().collect())))).unwrap();
    let _text=unsafe{super::super::factory::OwnedReceiver::from_owned(text)}.unwrap();
    let kind=constants.slots[0] as *mut AnyObject;
    let before=unsafe{change_count(board,objc2::sel!(changeCount))};
    assert!(unsafe{board_set_string(board,objc2::sel!(setString:forType:),text,kind)}.as_bool());
    assert_eq!(unsafe{change_count(board,objc2::sel!(changeCount))},before+1);
    let read=unsafe{board_string(board,objc2::sel!(stringForType:),kind)};
    assert_eq!(foundation_utf16(read).unwrap(),"private 🦀".encode_utf16().collect::<Vec<_>>());
    let bytes=vec![0,137,80,78,71,255];
    let data=new_foundation_value(FoundationValue::Data(Arc::new(Data::new(bytes.clone())))).unwrap();
    let _data=unsafe{super::super::factory::OwnedReceiver::from_owned(data)}.unwrap();
    for index in [2,3] {
        let kind=constants.slots[index] as *mut AnyObject;
        assert!(unsafe{board_set_data(board,objc2::sel!(setData:forType:),data,kind)}.as_bool());
        let read=unsafe{board_data(board,objc2::sel!(dataForType:),kind)};
        assert_eq!(foundation_bytes(read).unwrap(),bytes);
    }
    // Foreign receivers are rejected before decoding/releasing a raw object.
    assert!(unsafe{board_string(std::ptr::dangling_mut(),objc2::sel!(stringForType:),kind)}.is_null());
    assert!(!unsafe{board_set_string(std::ptr::dangling_mut(),objc2::sel!(setString:forType:),text,kind)}.as_bool());
    println!("private clipboard: same arena, five selected data cells, Unicode/text and PNG/TIFF data readback, foreign receiver refusal");
    board
}
#[cfg(test)]
pub(crate) fn prove_retired_private_board(board:*mut AnyObject) {
    assert!(unsafe{board_types(board,objc2::sel!(types))}.is_null());
    assert_eq!(unsafe{change_count(board,objc2::sel!(changeCount))},0);
    println!("private clipboard: retired receiver denied");
}

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
fn returned(value: FoundationValue) -> Result<*mut AnyObject> {
    let pointer = new_foundation_value(value)?;
    // SAFETY: consumes this call's fresh common factory +1.
    unsafe { autorelease_foundation(pointer) }
}
fn returned_value(value: Option<clipboard::Value>, want_text: bool) -> Result<*mut AnyObject> {
    match value {
        None => Ok(std::ptr::null_mut()),
        Some(clipboard::Value::Text(units)) if want_text => {
            returned(FoundationValue::String(Arc::new(Text::new(units))))
        }
        Some(clipboard::Value::Data(bytes)) if want_text => {
            let text = Text::from_utf8(&bytes)?;
            returned(FoundationValue::String(Arc::new(text)))
        }
        Some(clipboard::Value::Data(bytes)) => {
            returned(FoundationValue::Data(Arc::new(Data::new(bytes))))
        }
        Some(clipboard::Value::Text(units)) => {
            let text = Text::new(units);
            let bytes = text
                .utf8_with_nul()
                .ok_or("clipboard text has no UTF-8 data representation")?;
            returned(FoundationValue::Data(Arc::new(Data::new(
                bytes[..bytes.len() - 1].to_vec(),
            ))))
        }
    }
}
fn array(values: Vec<FoundationReference>) -> Result<*mut AnyObject> {
    let pointer = new_foundation_array(values)?;
    // SAFETY: consumes this call's fresh common array factory +1.
    unsafe { autorelease_foundation(pointer) }
}
fn types(values: Vec<Vec<u16>>) -> Result<*mut AnyObject> {
    let mut children = Vec::new();
    for value in values {
        let pointer = new_foundation_value(FoundationValue::String(Arc::new(Text::new(value))))?;
        // SAFETY: pointer is this call's fresh common factory +1 return.
        children.push(unsafe { foundation_created_reference(pointer)? });
    }
    array(children)
}
unsafe extern "C" fn general(class: *mut AnyClass, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        clipboard_class(class, ClipboardRole::Board)?;
        let pointer = clipboard_general()?;
        // SAFETY: the common board factory returned a new caller-owned +1.
        unsafe { autorelease_clipboard(pointer) }
    })
}
unsafe extern "C" fn unique(class: *mut AnyClass, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        clipboard_class(class, ClipboardRole::Board)?;
        let pointer = clipboard_unique()?;
        // SAFETY: the common board factory returned a new caller-owned +1.
        unsafe { autorelease_clipboard(pointer) }
    })
}
unsafe extern "C" fn named(class: *mut AnyClass, _: Sel, name: *mut AnyObject) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        clipboard_class(class, ClipboardRole::Board)?;
        let pointer = clipboard_named(&foundation_utf16(name)?)?;
        // SAFETY: the common board factory returned a new caller-owned +1.
        unsafe { autorelease_clipboard(pointer) }
    })
}
unsafe extern "C" fn name(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        returned(FoundationValue::String(Arc::new(Text::new(
            clipboard_board(receiver)?.name()?,
        ))))
    })
}
unsafe extern "C" fn change_count(receiver: *mut AnyObject, _: Sel) -> isize {
    boundary(0, || clipboard_board(receiver)?.change_count())
}
unsafe extern "C" fn clear(receiver: *mut AnyObject, _: Sel) -> isize {
    boundary(0, || clipboard_board(receiver)?.clear())
}
unsafe extern "C" fn release_global(receiver: *mut AnyObject, _: Sel) {
    boundary((), || clipboard_release(receiver));
}
unsafe extern "C" fn board_types(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        types(clipboard_board(receiver)?.types()?)
    })
}
unsafe extern "C" fn items(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let models = clipboard_board(receiver)?.bound_items()?;
        let mut children = Vec::new();
        for item in models {
            // Exact model Arc retains its attachment/owner generation.
            let pointer = new_clipboard_item(item)?;
            // SAFETY: fresh wrapper +1 from this call; consumed exactly once.
            children.push(unsafe { foundation_created_reference(pointer)? });
        }
        array(children)
    })
}
unsafe extern "C" fn board_string(
    receiver: *mut AnyObject,
    _: Sel,
    kind: *mut AnyObject,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        returned_value(
            clipboard_board(receiver)?.value(&foundation_utf16(kind)?)?,
            true,
        )
    })
}
unsafe extern "C" fn board_data(
    receiver: *mut AnyObject,
    _: Sel,
    kind: *mut AnyObject,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        returned_value(
            clipboard_board(receiver)?.value(&foundation_utf16(kind)?)?,
            false,
        )
    })
}
unsafe extern "C" fn board_set_string(
    receiver: *mut AnyObject,
    _: Sel,
    value: *mut AnyObject,
    kind: *mut AnyObject,
) -> Bool {
    boundary(Bool::NO, || {
        let board = clipboard_board(receiver)?;
        let kind = foundation_utf16(kind)?;
        let value = clipboard::Value::Text(foundation_utf16(value)?);
        board.set(kind, value)?;
        Ok(Bool::YES)
    })
}
unsafe extern "C" fn board_set_data(
    receiver: *mut AnyObject,
    _: Sel,
    value: *mut AnyObject,
    kind: *mut AnyObject,
) -> Bool {
    boundary(Bool::NO, || {
        let board = clipboard_board(receiver)?;
        let kind = foundation_utf16(kind)?;
        let value = clipboard::Value::Data(foundation_bytes(value)?);
        board.set(kind, value)?;
        Ok(Bool::YES)
    })
}
unsafe extern "C" fn write_objects(
    receiver: *mut AnyObject,
    _: Sel,
    objects: *mut AnyObject,
) -> Bool {
    boundary(Bool::NO, || {
        let board = clipboard_board(receiver)?;
        let values = foundation_array_elements(objects)?
            .iter()
            .map(|value| clipboard_write_object(value.as_ptr()?))
            .collect::<Result<Vec<_>>>()?;
        board.write(&values)?;
        Ok(Bool::YES)
    })
}
unsafe extern "C" fn read_objects(
    receiver: *mut AnyObject,
    _: Sel,
    classes: *mut AnyObject,
    options: *mut AnyObject,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let board = clipboard_board(receiver)?;
        let classes = foundation_array_elements(classes)?;
        let mut accepts_url = false;
        for class in classes {
            if !foundation_is_url_class(class.as_ptr()?)? {
                return Err("unsupported qualified clipboard read class".into());
            }
            accepts_url = true;
        }
        if !options.is_null() {
            let key = foundation_url_reading_file_only_key()?;
            // The common dictionary resolver rejects unknown keys/value kinds.
            let _file_only =
                foundation_bool_for_key(options, &foundation_utf16(key)?)?.unwrap_or(false);
        }
        if !accepts_url {
            return array(Vec::new());
        }
        let mut children = Vec::new();
        for units in board.file_urls()? {
            let value = Url::parse(&Text::new(units))?;
            if !value.is_file() {
                return Err("clipboard model yielded nonfile URL".into());
            }
            let pointer = new_foundation_value(FoundationValue::Url(Arc::new(value)))?;
            // SAFETY: pointer is this call's fresh common factory +1 return.
            children.push(unsafe { foundation_created_reference(pointer)? });
        }
        array(children)
    })
}
unsafe extern "C" fn init_item(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        bind_clipboard_item_initializer(receiver)
    })
}
unsafe extern "C" fn item_types(receiver: *mut AnyObject, _: Sel) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        types(clipboard_item(receiver)?.snapshot()?.types())
    })
}
unsafe extern "C" fn item_string(
    receiver: *mut AnyObject,
    _: Sel,
    kind: *mut AnyObject,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let kind = foundation_utf16(kind)?;
        returned_value(clipboard_item(receiver)?.snapshot()?.value(&kind), true)
    })
}
unsafe extern "C" fn item_data(
    receiver: *mut AnyObject,
    _: Sel,
    kind: *mut AnyObject,
) -> *mut AnyObject {
    boundary(std::ptr::null_mut(), || {
        let kind = foundation_utf16(kind)?;
        returned_value(clipboard_item(receiver)?.snapshot()?.value(&kind), false)
    })
}
unsafe extern "C" fn item_set_string(
    receiver: *mut AnyObject,
    _: Sel,
    value: *mut AnyObject,
    kind: *mut AnyObject,
) -> Bool {
    boundary(Bool::NO, || {
        let item = clipboard_item(receiver)?;
        let kind = foundation_utf16(kind)?;
        let value = clipboard::Value::Text(foundation_utf16(value)?);
        item.set(kind, value)?;
        Ok(Bool::YES)
    })
}
unsafe extern "C" fn item_set_data(
    receiver: *mut AnyObject,
    _: Sel,
    value: *mut AnyObject,
    kind: *mut AnyObject,
) -> Bool {
    boundary(Bool::NO, || {
        let item = clipboard_item(receiver)?;
        let kind = foundation_utf16(kind)?;
        let value = clipboard::Value::Data(foundation_bytes(value)?);
        item.set(kind, value)?;
        Ok(Bool::YES)
    })
}
