//! Integer conversions that Win32 structures need again and again.

/// `size_of::<T>()` for the `cbSize`-style fields of Win32 structures.
pub(crate) fn size_u32<T>() -> u32 {
    u32::try_from(size_of::<T>()).expect("Win32 structures are far smaller than 4 GiB")
}

/// A length as the `u32` byte or element count a system call takes.
pub(crate) fn len_u32(len: usize) -> u32 {
    u32::try_from(len).expect("buffer lengths passed to Win32 fit in 32 bits")
}

/// Combines the two halves of a 64-bit value split over two `u32` fields.
pub(crate) fn join_u64(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

/// A FILETIME tick count as the signed value the file APIs report.
pub(crate) fn ticks_i64(high: u32, low: u32) -> i64 {
    i64::from_ne_bytes(join_u64(high, low).to_ne_bytes())
}

/// The low 16 bits of a message parameter.
pub(crate) fn loword(value: usize) -> u16 {
    u16::try_from(value & 0xFFFF).unwrap_or(0)
}

/// Bits 16 to 31 of a message parameter.
pub(crate) fn hiword(value: usize) -> u16 {
    loword(value >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_halves() {
        assert_eq!(join_u64(1, 2), 0x1_0000_0002);
        assert_eq!(ticks_i64(0, 5), 5);
        assert_eq!(ticks_i64(0x01DC_0000, 0), 0x01DC_0000_0000_0000);
        assert_eq!((loword(0x1234_5678), hiword(0x1234_5678)), (0x5678, 0x1234));
    }
}
