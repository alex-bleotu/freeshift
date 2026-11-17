pub fn read_varint(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if index == 10 {
            return None;
        }
        value |= u64::from(byte & 0x7F) << (7 * index);
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
    }
    None
}

pub fn varint_fields(bytes: &[u8]) -> Option<Vec<(u32, u64)>> {
    let mut fields = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        let (tag, length) = read_varint(&bytes[position..])?;
        position += length;
        let field = u32::try_from(tag >> 3).ok()?;
        match tag & 0x07 {
            0 => {
                let (value, length) = read_varint(&bytes[position..])?;
                position += length;
                fields.push((field, value));
            }
            1 => position += 8,
            2 => {
                let (size, length) = read_varint(&bytes[position..])?;
                position += length + usize::try_from(size).ok()?;
            }
            5 => position += 4,
            _ => return None,
        }
        if position > bytes.len() {
            return None;
        }
    }
    Some(fields)
}

pub fn field(fields: &[(u32, u64)], number: u32) -> Option<u64> {
    fields
        .iter()
        .find(|(candidate, _)| *candidate == number)
        .map(|(_, value)| *value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_single_and_multi_byte_varints() {
        assert_eq!(read_varint(&[0x01]), Some((1, 1)));
        assert_eq!(read_varint(&[0xAC, 0x02]), Some((300, 2)));
        assert_eq!(read_varint(&[0xFF, 0xFF, 0x03]), Some((65535, 3)));
        assert_eq!(read_varint(&[0x80]), None);
    }

    #[test]
    fn collects_varint_fields_and_skips_others() {
        let bytes = [0x08, 0x00, 0x12, 0x02, 0xAA, 0xBB, 0x10, 0x01, 0x1D, 1, 2, 3, 4];
        assert_eq!(varint_fields(&bytes), Some(vec![(1, 0), (2, 1)]));
    }

    #[test]
    fn rejects_truncated_input() {
        assert_eq!(varint_fields(&[0x12, 0x05, 0x01]), None);
        assert_eq!(varint_fields(&[0x08]), None);
    }
}
