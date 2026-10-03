//! Little-endian reads at an offset, `None` past the end.

pub(crate) fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    array_at(bytes, at).map(u32::from_le_bytes)
}

pub(crate) fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    array_at(bytes, at).map(u64::from_le_bytes)
}

pub(crate) fn id_at(bytes: &[u8], at: usize) -> Option<[u8; 16]> {
    array_at(bytes, at)
}

fn array_at<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..)?.get(..N)?.try_into().ok()
}
