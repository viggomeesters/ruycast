//! Bounded, native 64-bit Mach-O identity and backed-section reader.
//! UUID and executable/data bytes are invariant across diagnostic stripping.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Identity {
    pub uuid: String,
    pub backed_sections_sha256: BTreeMap<String, String>,
}

fn bad() -> String {
    "Expected one well-formed native 64-bit Mach-O image.".into()
}
fn bytes_at(data: &[u8], offset: usize, length: usize) -> Result<&[u8]> {
    data.get(offset..offset.checked_add(length).ok_or_else(bad)?)
        .ok_or_else(bad)
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes_at(data, offset, 4)?.try_into().map_err(|_| bad())?,
    ))
}
fn u64_at(data: &[u8], offset: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes_at(data, offset, 8)?.try_into().map_err(|_| bad())?,
    ))
}
fn name(data: &[u8]) -> Result<&str> {
    let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
    let value = std::str::from_utf8(&data[..end]).map_err(|_| bad())?;
    if value.is_empty() || value.bytes().any(|b| !b.is_ascii_graphic() || b == b',') {
        return Err(bad());
    }
    Ok(value)
}

pub(super) fn parse(data: &[u8]) -> Result<Identity> {
    if data.len() < 32 || u32_at(data, 0)? != 0xfeed_facf {
        return Err(bad());
    }
    // Explicit thin, supported native CPU architectures. Universal images and
    // foreign endian images must not silently acquire release authority.
    if !matches!(u32_at(data, 4)?, 0x0100_0007 | 0x0100_000c) {
        return Err(bad());
    }
    let commands = u32_at(data, 16)? as usize;
    let command_bytes = u32_at(data, 20)? as usize;
    let end = 32usize.checked_add(command_bytes).ok_or_else(bad)?;
    if end > data.len() || commands > command_bytes / 8 {
        return Err(bad());
    }
    let mut offset = 32usize;
    let mut uuid = None;
    let mut sections = BTreeMap::new();
    for _ in 0..commands {
        let kind = u32_at(data, offset)?;
        let size = u32_at(data, offset + 4)? as usize;
        let next = offset.checked_add(size).ok_or_else(bad)?;
        if size < 8 || !size.is_multiple_of(8) || next > end {
            return Err(bad());
        }
        match kind {
            0x1b => {
                if size != 24 || uuid.is_some() {
                    return Err(bad());
                }
                let raw = bytes_at(data, offset + 8, 16)?;
                let hex = raw.iter().map(|b| format!("{b:02X}")).collect::<String>();
                uuid = Some(format!(
                    "{}-{}-{}-{}-{}",
                    &hex[..8],
                    &hex[8..12],
                    &hex[12..16],
                    &hex[16..20],
                    &hex[20..]
                ));
            }
            0x19 => {
                if size < 72 {
                    return Err(bad());
                }
                let count = u32_at(data, offset + 64)? as usize;
                let required = count
                    .checked_mul(80)
                    .and_then(|n| n.checked_add(72))
                    .ok_or_else(bad)?;
                if required != size {
                    return Err(bad());
                }
                for index in 0..count {
                    let section = offset + 72 + index * 80;
                    let section_name = name(bytes_at(data, section, 16)?)?;
                    let segment_name = name(bytes_at(data, section + 16, 16)?)?;
                    let length = usize::try_from(u64_at(data, section + 40)?).map_err(|_| bad())?;
                    let start = u32_at(data, section + 48)? as usize;
                    let flags = u32_at(data, section + 64)?;
                    if length == 0 || start == 0 || matches!(flags & 0xff, 1 | 12 | 18) {
                        continue;
                    }
                    let backed = bytes_at(data, start, length)?;
                    let key = format!("{segment_name},{section_name}");
                    if sections
                        .insert(key, format!("{:x}", Sha256::digest(backed)))
                        .is_some()
                    {
                        return Err(bad());
                    }
                }
            }
            _ => (),
        }
        offset = next;
    }
    if offset != end || sections.is_empty() {
        return Err(bad());
    }
    Ok(Identity {
        uuid: uuid.ok_or_else(bad)?,
        backed_sections_sha256: sections,
    })
}

pub(super) fn read(path: &Path) -> Result<Identity> {
    parse(&regular_bytes(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn put32(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(data: &mut [u8], at: usize, value: u64) {
        data[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn image() -> Vec<u8> {
        let mut data = vec![0; 32 + 24 + 152 + 4];
        put32(&mut data, 0, 0xfeed_facf);
        put32(&mut data, 4, 0x0100_000c);
        put32(&mut data, 16, 2);
        put32(&mut data, 20, 176);
        put32(&mut data, 32, 0x1b);
        put32(&mut data, 36, 24);
        data[40..56].copy_from_slice(&[0x12; 16]);
        put32(&mut data, 56, 0x19);
        put32(&mut data, 60, 152);
        put32(&mut data, 120, 1);
        data[128..134].copy_from_slice(b"__text");
        data[144..150].copy_from_slice(b"__TEXT");
        put64(&mut data, 168, 4);
        put32(&mut data, 176, 208);
        data[208..212].copy_from_slice(b"code");
        data
    }
    #[test]
    fn identity_binds_actual_code_and_uuid_not_unbacked_padding() {
        let original = image();
        let identity = parse(&original).unwrap();
        assert_eq!(identity.uuid, "12121212-1212-1212-1212-121212121212");
        assert_eq!(identity.backed_sections_sha256.len(), 1);
        let mut changed = original.clone();
        changed.push(1);
        assert_eq!(parse(&changed).unwrap(), identity);
        changed[208] ^= 1;
        assert_ne!(parse(&changed).unwrap(), identity);
        let mut changed = original;
        changed[40] ^= 1;
        assert_ne!(parse(&changed).unwrap(), identity);
    }
    #[test]
    fn malformed_bounds_counts_architecture_and_missing_identity_fail_closed() {
        let data = image();
        for cut in [0, 4, 31, 32, 39, 55, 207, 211] {
            assert!(parse(&data[..cut]).is_err());
        }
        for (at, value) in [
            (0, 0xcafe_babe),
            (4, 7),
            (16, 3),
            (20, 168),
            (36, 16),
            (120, u32::MAX),
            (176, u32::MAX),
            (32, 0),
        ] {
            let mut changed = data.clone();
            put32(&mut changed, at, value);
            assert!(parse(&changed).is_err(), "offset {at}");
        }
        let mut changed = data;
        put64(&mut changed, 168, u64::MAX);
        assert!(parse(&changed).is_err());
    }
}
