//! LZ4 block decoding (the block format, without frames).

/// A literal or match length of 15 continues in the bytes that follow.
const MORE: usize = 15;
/// The shortest match.
const MIN_MATCH: usize = 4;
/// One input byte expands to at most this many output bytes (a long match
/// costs one length byte per 255 bytes copied).
const MAX_RATIO: usize = 255;

/// Decode an LZ4 block that must expand to exactly `size` bytes.
pub(crate) fn decompress(block: &[u8], size: usize) -> Result<Vec<u8>, &'static str> {
    if size > block.len().saturating_mul(MAX_RATIO) {
        return Err("claims more data than its compressed size can hold");
    }
    let mut out = Vec::with_capacity(size);
    let mut at = 0;
    loop {
        let token = *block.get(at).ok_or("ends before its last literals")?;
        at += 1;
        let literals = length(block, &mut at, usize::from(token >> 4))?;
        let end = at
            .checked_add(literals)
            .filter(|&end| end <= block.len())
            .ok_or("literals run past the end")?;
        if out.len() + literals > size {
            return Err("expands past its stated size");
        }
        out.extend_from_slice(&block[at..end]);
        at = end;
        if at == block.len() {
            break;
        }
        let distance = block
            .get(at..at + 2)
            .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
            .ok_or("ends inside a match offset")?;
        at += 2;
        if distance == 0 || distance > out.len() {
            return Err("match points before the start");
        }
        let matched = length(block, &mut at, usize::from(token & 0x0f))? + MIN_MATCH;
        if out.len() + matched > size {
            return Err("expands past its stated size");
        }
        let from = out.len() - distance;
        if matched <= distance {
            out.extend_from_within(from..from + matched);
        } else {
            // Overlapping: the match repeats bytes it is writing.
            for index in from..from + matched {
                out.push(out[index]);
            }
        }
    }
    if out.len() == size {
        Ok(out)
    } else {
        Err("expands to less than its stated size")
    }
}

/// A token's 4-bit length, extended by the bytes that follow when it is 15.
fn length(block: &[u8], at: &mut usize, nibble: usize) -> Result<usize, &'static str> {
    let mut length = nibble;
    if nibble == MORE {
        loop {
            let byte = *block.get(*at).ok_or("ends inside a length")?;
            *at += 1;
            length += usize::from(byte);
            if byte != 255 {
                break;
            }
        }
    }
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::decompress;

    #[test]
    fn literals_only() {
        assert_eq!(decompress(b"\x50hello", 5).unwrap(), b"hello");
    }

    #[test]
    fn overlapping_match_repeats_bytes() {
        // "ab", then a 10-byte match two bytes back, then the last literal.
        let block = b"\x26ab\x02\x00\x10z";
        assert_eq!(decompress(block, 13).unwrap(), b"ababababababz");
    }

    #[test]
    fn long_lengths_continue_in_extra_bytes() {
        let mut block = vec![0xf0, 255, 3];
        block.extend([b'x'; 273]);
        assert_eq!(decompress(&block, 273).unwrap(), vec![b'x'; 273]);
    }

    #[test]
    fn damage_is_an_error() {
        assert!(decompress(b"\x50hel", 5).is_err(), "short literals");
        assert!(
            decompress(b"\x10a\x05\x00", 9).is_err(),
            "match before start"
        );
        assert!(decompress(b"\x50hello", 4).is_err(), "too long");
        assert!(decompress(b"\x50hello", 6).is_err(), "too short");
        assert!(decompress(b"", 0).is_err(), "no token");
        assert!(decompress(b"\x50hello", usize::MAX).is_err(), "huge size");
    }
}
