use std::ffi::{c_char, c_void, CStr, CString};

#[link(name = "rustcast_core", kind = "static")]
unsafe extern "C" {
    fn rustcast_abi_version() -> u32;
    fn rustcast_call(operation: *const c_char, payload: *const c_char) -> *mut c_char;
    fn rustcast_free(response: *mut c_char);
    fn rustcast_index_new(catalog: *const c_char) -> *mut c_void;
    fn rustcast_index_search_ranked_into(
        index: *const c_void,
        query: *const u8,
        length: usize,
        recency: *const f64,
        recency_count: usize,
        output: *mut usize,
        capacity: usize,
    ) -> isize;
    fn rustcast_index_free(index: *mut c_void);
}

unsafe fn call(operation: &CString, payload: &CString) -> String {
    let response = unsafe { rustcast_call(operation.as_ptr(), payload.as_ptr()) };
    assert!(!response.is_null(), "Rust ABI returned no response");
    let value = unsafe { CStr::from_ptr(response) }
        .to_str()
        .expect("Rust ABI response must be UTF-8")
        .to_owned();
    unsafe { rustcast_free(response) };
    value
}

fn main() {
    assert_eq!(unsafe { rustcast_abi_version() }, 1);

    let defaults = unsafe {
        call(
            &CString::new("defaults").unwrap(),
            &CString::new("{}").unwrap(),
        )
    };
    assert!(defaults.contains(r#""ok":true"#) && defaults.contains(r#""commands"#));

    let failure = unsafe {
        call(
            &CString::new("core-abi-probe-unknown-operation").unwrap(),
            &CString::new("{}").unwrap(),
        )
    };
    assert!(failure.contains(r#""ok":false"#));
    assert!(failure.contains("Unknown core operation: core-abi-probe-unknown-operation"));

    let catalog = CString::new(
        r#"[{"id":"core-probe","title":"Rust ABI probe","kind":"url","target":"https://example.com"}]"#,
    )
    .unwrap();
    let index = unsafe { rustcast_index_new(catalog.as_ptr()) };
    assert!(!index.is_null(), "Rust ABI could not create a search index");
    let query = b"abi probe";
    let mut output = [usize::MAX; 1];
    let count = unsafe {
        rustcast_index_search_ranked_into(
            index,
            query.as_ptr(),
            query.len(),
            std::ptr::null(),
            0,
            output.as_mut_ptr(),
            output.len(),
        )
    };
    assert_eq!(count, 1);
    assert_eq!(output, [0]);
    unsafe { rustcast_index_free(index) };
}
