// Source-only typechecking input. Never pass this to an executable test/run.
// Real private descriptor/nonce/source/compiler pins must come from the
// qualified source-owned producer before its fresh compilation.
fn make_data_pins() -> sdk::data_family_preentry::Pins<'static> {
    panic!("compile-only Data pins have no runnable authority")
}
