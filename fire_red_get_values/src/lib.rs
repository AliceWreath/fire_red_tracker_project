//! # FireRed Get Values
//! 
//! Low-level byte parsing utilities used throughout the codebase.
//! 
//! ## Two families of functions
//! 
//! ### `get_*` - parse from hex-string token buffers
//! 
//! These functions accept `&[&str]` slices of hexadecimal byte tokens as
//! returned by RetroArch's `READ_CORE_MEMORY` response. Each token is a
//! two-character ASCII hex string (e.g. `"A4"`). Values are read as 
//! **little-endian**, matching the GBA's native byte order.
//! 
//! ### `read_*` - read from raw byte slices (little-endian)
//! 
//! These functions accept a `&[u8]` plus a byte `offset` and read the value
//! in **little-endian** order. The are used when working directly with ROM or
//! memory buffers rather than RetroArch protocol responses.
//! 
//! ### `read_*_raw` - read from raw byte slices (big-endian)
//! 
//! Identical layout to the `read_*` family but interpret the bytes as
//! **big-endian**. Used when parsing network or protocol data that uses
//! big-endian byte order.
//! 
//! ## Error handling
//! 
//! All functions return `0` / `None` on out-of-bounds access or parse failure
//! rather than panicking, so callers can treat missing data as a zero value
//! without additional error handling.


// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Converts a slice of hex-string tokens into a `Vec<u8>`.
/// 
/// Each token is parsed as a base-16 unsigned byte. Tokens that are not valid
/// hex strings are silently skipped via `filter_map`, so the output `Vec` may
/// be shorter than `buffer` if any tokens are malformed.
/// 
/// # Arguments
/// * `buffer` - Slice of hex byte strings (e.g. `&["08", "A4", "00", "00"`).
fn get_bytes(buffer: &[&str]) -> Vec<u8> {
        let bytes: Vec<u8> = buffer[..]
            .iter()
            .filter_map(|h| u8::from_str_radix(h, 16).ok())
            .collect();
        bytes
}


// ---------------------------------------------------------------------------
// get_* family - parse from hex-string token buffers (little-endian)
// ---------------------------------------------------------------------------

/// Returns up to `n` bytes parsed from the hex-string token buffer.
/// 
/// Returns `None` if `buffer` contains fewer than `n` tokens, printing a
/// diagnostic to stderr. Only the first `n` tokens are parsed.
/// 
/// # Arguments
/// * `n`               - Number of bytes to read.
/// * `buffer`          0 Slice of hex byte string tokens.
pub fn get_n_bytes(n: usize, buffer: &[&str]) -> Option<Vec<u8>> {
    if buffer.len() < n {
        eprintln!("get_n_bytes: requested {n} bytes but buffer len is only {}", buffer.len());
        return None;
    }
    
    let byte_list = get_bytes(&buffer[..n]);

    Some(byte_list)
}

/// Reads a little-endian `u32` from the first 4 tokens of `buffer`
/// 
/// Returns `0` if fewer than 4 tokens are present or if any conversion fails.
/// 
/// # Arguments
/// * `buffer` - Slice of at least 4 hex byte string tokens.
pub fn get_u32(buffer: &[&str]) -> u32 {
    if buffer.len() < 4 {
        return 0;
    }
    let bytes: Vec<u8> = buffer.iter()
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();

    bytes.get(..4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes).unwrap_or(0)
}

/// Reads a little-endian `i32` from the first 4 tokens of `buffer`
/// 
/// Returns `0` if fewer than 4 tokens are present or if any conversion fails.
/// 
/// # Arguments
/// * `buffer` - Slice of at least 4 hex byte string tokens.
pub fn get_i32(buffer: &[&str]) -> i32 {
    if buffer.len() < 4 {
        return 0;
    }
    let bytes: Vec<u8> = buffer.iter()
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();

    bytes.get(..4)
        .and_then(|b| b.try_into().ok())
        .map(i32::from_le_bytes).unwrap_or(0)
}

/// Reads a little-endian `u16` from the first 2 tokens of `buffer`
/// 
/// Returns `0` if fewer than 2 tokens are present or if any conversion fails.
/// 
/// # Arguments
/// * `buffer` - Slice of at least 2 hex byte string tokens.
pub fn get_u16(buffer: &[&str]) -> u16 {
    if buffer.len() < 2{
        return 0;
    }
    let bytes: Vec<u8> = buffer.iter()
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();

    bytes.get(..2)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes).unwrap_or(0)
}

/// Reads a little-endian `i16` from the first 2 tokens of `buffer`
/// 
/// Returns `0` if fewer than 2 tokens are present or if any conversion fails.
/// 
/// # Arguments
/// * `buffer` - Slice of at least 2 hex byte string tokens.
pub fn get_i16(buffer: &[&str]) -> i16 {
    if buffer.len() < 2{
        return 0;
    }
    let bytes: Vec<u8> = buffer.iter()
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();

    bytes.get(..2)
        .and_then(|b| b.try_into().ok())
        .map(i16::from_le_bytes).unwrap_or(0)
}

/// Reads a single `u8` from the first token of `buffer`.
///
/// Returns `0` if the buffer is empty or the token is not a valid hex byte.
///
/// # Arguments
/// * `buffer` — Slice of at least 1 hex byte string token.
pub fn get_u8(buffer: &[&str]) -> u8 {
    if buffer.is_empty() {
        return 0;
    }
    let bytes = get_bytes(buffer);
    if bytes.is_empty() {
        return 0;
    }
    u8::from_le_bytes([bytes[0]])
}
 
// ---------------------------------------------------------------------------
// read_* family — read from raw byte slices (little-endian)
// ---------------------------------------------------------------------------

/// Reads a single `u8` from `bytes` at `offset`
/// 
/// Returns `0` if `offset` is out of bounds.
/// 
/// # Arguments
/// * `bytes`       - Raw byte slice(e.g. a ROM or memory buffer)
/// * `offset`      - Byte offset to read from.
pub fn read_u8(bytes: &[u8], offset: usize) -> u8 {
    bytes.get(offset).copied().unwrap_or(0)
}

/// Reads a little-endian `u32` from `bytes` at `offset`
/// 
/// Returns `0` if `offset..offset+4` is out of bounds.
/// 
/// # Arguments
/// * `bytes`       - Raw byte slice(e.g. a ROM or memory buffer)
/// * `offset`      - Byte offset of the first (least-significant) byte
pub fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    bytes.get(offset..offset.saturating_add(4))
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

/// Reads a little-endian `u16` from `bytes` at `offset`
/// 
/// Returns `0` if `offset..offset+2` is out of bounds.
/// 
/// # Arguments
/// * `bytes`       - Raw byte slice(e.g. a ROM or memory buffer)
/// * `offset`      - Byte offset of the first (least-significant) byte
pub fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    bytes.get(offset..offset.saturating_add(2))
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes)
        .unwrap_or(0)
}

/// Reads a little-endian `i16` from `bytes` at `offset`
/// 
/// Returns `0` if `offset..offset+2` is out of bounds.
/// 
/// # Arguments
/// * `bytes`       - Raw byte slice(e.g. a ROM or memory buffer)
/// * `offset`      - Byte offset of the first (least-significant) byte
pub fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    bytes.get(offset..offset.saturating_add(2))
        .and_then(|b| b.try_into().ok())
        .map(i16::from_le_bytes)
        .unwrap_or(0)
}

/// Reads a little-endian `i32` from `bytes` at `offset`
/// 
/// Returns `0` if `offset..offset+4` is out of bounds.
/// 
/// # Arguments
/// * `bytes`       - Raw byte slice(e.g. a ROM or memory buffer)
/// * `offset`      - Byte offset of the first (least-significant) byte
pub fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    bytes.get(offset..offset.saturating_add(4))
        .and_then(|b| b.try_into().ok())
        .map(i32::from_le_bytes)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// read_*_raw family — read from raw byte slices (big-endian)
// ---------------------------------------------------------------------------

/// Reads a single `u8` from `bytes` at `offset`
/// 
/// Byte order is irrelevant for single bytes; this is the big-endian
/// counterpart of [`read_u8`] for API symmetry.
/// 
/// Returns `0` if `offset` is out of bounds.
/// 
/// # Arguments
/// * `bytes`           - Raw bytes slice.
/// * `offset`          - Byte offset to read from.
pub fn read_u8_raw(bytes: &[u8], offset: usize) -> u8 {
    bytes.get(offset).copied().unwrap_or(0)
}

/// Reads a big-endian `u32` from `bytes` at `offset`
/// 
/// Returns `0` if `offset..offset+4` is out of bounds.
/// 
/// # Arguments
/// * `bytes`           - Raw bytes slice.
/// * `offset`          - Byte offset of the most-significant byte
pub fn read_u32_raw(bytes: &[u8], offset: usize) -> u32 {
    bytes.get(offset..offset.saturating_add(4))
        .and_then(|b| b.try_into().ok())
        .map(u32::from_be_bytes)
        .unwrap_or(0)
}

/// Reads a big-endian `u16` from `bytes` at `offset`
/// 
/// Returns `0` if `offset..offset+2` is out of bounds.
/// 
/// # Arguments
/// * `bytes`           - Raw bytes slice.
/// * `offset`          - Byte offset of the most-significant byte
pub fn read_u16_raw(bytes: &[u8], offset: usize) -> u16 {
    bytes.get(offset..offset.saturating_add(2))
        .and_then(|b| b.try_into().ok())
        .map(u16::from_be_bytes)
        .unwrap_or(0)
}

/// Reads a big-endian `i16` from `bytes` at `offset`
/// 
/// Returns `0` if `offset..offset+2` is out of bounds.
/// 
/// # Arguments
/// * `bytes`           - Raw bytes slice.
/// * `offset`          - Byte offset of the most-significant byte
pub fn read_i16_raw(bytes: &[u8], offset: usize) -> i16 {
    bytes.get(offset..offset.saturating_add(2))
        .and_then(|b| b.try_into().ok())
        .map(i16::from_be_bytes)
        .unwrap_or(0)
}

/// Reads a big-endian `i32` from `bytes` at `offset`
/// 
/// Returns `0` if `offset..offset+4` is out of bounds.
/// 
/// # Arguments
/// * `bytes`           - Raw bytes slice.
/// * `offset`          - Byte offset of the most-significant byte
pub fn read_i32_raw(bytes: &[u8], offset: usize) -> i32 {
    bytes.get(offset..offset.saturating_add(4))
        .and_then(|b| b.try_into().ok())
        .map(i32::from_be_bytes)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── read_u8 ──────────────────────────────────────────────────────────────

    #[test]
    fn read_u8_first_byte() {
        assert_eq!(read_u8(&[0xAB, 0xCD], 0), 0xAB);
    }

    #[test]
    fn read_u8_second_byte() {
        assert_eq!(read_u8(&[0xAB, 0xCD], 1), 0xCD);
    }

    #[test]
    fn read_u8_out_of_bounds_returns_zero() {
        assert_eq!(read_u8(&[0x01], 1), 0);
        assert_eq!(read_u8(&[], 0), 0);
    }

    // ── read_u16 ─────────────────────────────────────────────────────────────

    #[test]
    fn read_u16_little_endian() {
        assert_eq!(read_u16(&[0x34, 0x12], 0), 0x1234);
    }

    #[test]
    fn read_u16_with_offset() {
        assert_eq!(read_u16(&[0x00, 0x78, 0x56], 1), 0x5678);
    }

    #[test]
    fn read_u16_out_of_bounds_returns_zero() {
        assert_eq!(read_u16(&[0x01], 0), 0);
        assert_eq!(read_u16(&[0x01, 0x02], 1), 0);
    }

    // ── read_u32 ─────────────────────────────────────────────────────────────

    #[test]
    fn read_u32_little_endian() {
        assert_eq!(read_u32(&[0x78, 0x56, 0x34, 0x12], 0), 0x12345678);
    }

    #[test]
    fn read_u32_with_offset() {
        assert_eq!(read_u32(&[0x00, 0x78, 0x56, 0x34, 0x12], 1), 0x12345678);
    }

    #[test]
    fn read_u32_out_of_bounds_returns_zero() {
        assert_eq!(read_u32(&[0x01, 0x02, 0x03], 0), 0);
    }

    // ── read_u32_raw (big-endian) ─────────────────────────────────────────────

    #[test]
    fn read_u32_raw_big_endian() {
        assert_eq!(read_u32_raw(&[0x12, 0x34, 0x56, 0x78], 0), 0x12345678);
    }

    #[test]
    fn read_u32_raw_differs_from_little_endian() {
        let buf = [0x01, 0x00, 0x00, 0x00];
        assert_eq!(read_u32(&buf, 0),     0x0000_0001);
        assert_eq!(read_u32_raw(&buf, 0), 0x0100_0000);
    }

    // ── get_u32 / get_u16 / get_u8 ───────────────────────────────────────────

    #[test]
    fn get_u32_little_endian() {
        assert_eq!(get_u32(&["78", "56", "34", "12"]), 0x12345678);
    }

    #[test]
    fn get_u32_too_few_tokens_returns_zero() {
        assert_eq!(get_u32(&["78", "56"]), 0);
    }

    #[test]
    fn get_u16_little_endian() {
        assert_eq!(get_u16(&["CD", "AB"]), 0xABCD);
    }

    #[test]
    fn get_u8_valid_hex_token() {
        assert_eq!(get_u8(&["AB"]), 0xAB);
    }

    #[test]
    fn get_u8_empty_returns_zero() {
        assert_eq!(get_u8(&[]), 0);
    }

    #[test]
    fn get_n_bytes_returns_correct_slice() {
        let result = get_n_bytes(2, &["01", "02", "03"]).unwrap();
        assert_eq!(result, vec![0x01, 0x02]);
    }

    #[test]
    fn get_n_bytes_too_short_returns_none() {
        assert!(get_n_bytes(3, &["01", "02"]).is_none());
    }
}