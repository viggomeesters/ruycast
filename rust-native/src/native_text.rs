//! Swift-compatible NSString -> String repair, without UTF8String assumptions
//! or a second full-sized UTF16 allocation. Call on immutable native snapshots.
use objc2_foundation::{NSRange, NSString};

struct Units<'a> {
    value: &'a NSString,
    length: usize,
    offset: usize,
    buffer: [u16; 4096],
    index: usize,
    used: usize,
}
impl Iterator for Units<'_> {
    type Item = u16;
    fn next(&mut self) -> Option<u16> {
        if self.index == self.used {
            if self.offset == self.length {
                return None;
            }
            self.used = (self.length - self.offset).min(self.buffer.len());
            // Initialized buffer has used elements; range is inside the
            // immutable NSString's captured UTF16 length. No Rust lock held.
            unsafe {
                self.value.getCharacters_range(
                    std::ptr::NonNull::from(&mut self.buffer[0]),
                    NSRange::new(self.offset, self.used),
                );
            }
            self.offset += self.used;
            self.index = 0;
        }
        let value = self.buffer[self.index];
        self.index += 1;
        Some(value)
    }
}
pub fn repaired(value: &NSString) -> String {
    let length = value.length();
    let units = Units {
        value,
        length,
        offset: 0,
        buffer: [0; 4096],
        index: 0,
        used: 0,
    };
    let mut output = String::with_capacity(length);
    // The decoder spans refills, preserving surrogate pairs at chunk edges.
    for scalar in char::decode_utf16(units) {
        output.push(scalar.unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::{rc::autoreleasepool, AnyThread};
    #[test]
    fn native_utf16_units_include_nul_all_units_and_cross_chunk_pairs() {
        autoreleasepool(|_| {
            let mut fixtures = vec![
                vec![],
                vec![0xD800],
                vec![0xDC00],
                vec![0xD800, 0xD800, 0xDC00, 0xDC00],
            ];
            let mut boundary = vec![120; 4095];
            boundary.extend([0xD83D, 0xDE00, 0]);
            fixtures.push(boundary);
            fixtures.push((0..=65535u16).flat_map(|u| [u, 65]).collect());
            for mut units in fixtures {
                let value = unsafe {
                    NSString::initWithCharacters_length(
                        NSString::alloc(),
                        std::ptr::NonNull::new(units.as_mut_ptr()).unwrap(),
                        units.len(),
                    )
                };
                assert_eq!(repaired(&value), String::from_utf16_lossy(&units));
            }
        });
    }
}
