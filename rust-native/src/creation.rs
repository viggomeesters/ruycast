//! On-demand quicklink prefill, invoked only by an explicit creation action.
//! Called by the native main service after a live creation-draft grant.
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::{NSCharacterSet, NSString};
pub fn prefill(board: &NSPasteboard) -> Option<String> {
    let text = board.stringForType(unsafe { NSPasteboardTypeString })?;
    prefill_text(&text)
}
fn prefill_text(text: &NSString) -> Option<String> {
    // Keep Foundation's trim semantics. UTF16 length remains valid for lone
    // surrogates; NSString.is_empty uses UTF8 length and misclassifies them.
    // Observe repaired Swift-compatible text after trimming, without an extra
    // full String -> NSString -> String conversion.
    let trimmed =
        text.stringByTrimmingCharactersInSet(&NSCharacterSet::whitespaceAndNewlineCharacterSet());
    (trimmed.length() != 0).then(|| crate::native_text::repaired(&trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creation_keeps_complete_text_and_uses_foundation_whitespace() {
        objc2::rc::autoreleasepool(|_| {
            let text = "a".repeat(1_000_001);
            assert_eq!(
                prefill_text(&NSString::from_str(&format!(" \n{text}\u{0085}"))),
                Some(text)
            );
            assert_eq!(
                prefill_text(&NSString::from_str("\u{200b}value\u{200b}")),
                Some("value".into())
            );
            assert_eq!(prefill_text(&NSString::from_str(" \n\u{200b}")), None);
        });
    }
}
