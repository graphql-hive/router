//! I took it from https://github.com/cloudwego/sonic-rs/blob/5ad7f96877fec7d3d33a5971b8bafe5af40fd3ff/src/util/string.rs
use bytes::BufMut;
use std::slice::from_raw_parts;

use crate::utils::consts::NULL;

#[inline(always)]
pub fn write_and_escape_string(writer: &mut Vec<u8>, input: &str) {
    format_string(input, writer, true);
}

pub fn write_f64(writer: &mut Vec<u8>, value: f64) {
    if !value.is_finite() {
        // JSON does not allow infinite or nan values. In browsers JSON.stringify(Number.NaN) = "null"
        writer.put(NULL);
        return;
    }

    let mut buffer = ryu::Buffer::new();
    let s = buffer.format_finite(value);
    writer.put(s.as_bytes())
}

pub fn write_u64(writer: &mut Vec<u8>, value: u64) {
    let mut buf = itoa::Buffer::new();
    writer.put(buf.format(value).as_bytes());
}

pub fn write_i64(writer: &mut Vec<u8>, value: i64) {
    let mut buf = itoa::Buffer::new();
    writer.put(buf.format(value).as_bytes());
}

#[cfg(not(all(target_feature = "neon", target_arch = "aarch64")))]
use sonic_simd::u8x32;
#[cfg(all(target_feature = "neon", target_arch = "aarch64"))]
use sonic_simd::{bits::NeonBits, u8x16};
use sonic_simd::{BitMask, Mask, Simd};

/// Loads a SIMD vector from a pointer.
///
/// SAFETY:
/// The caller must ensure that `ptr` is valid for reading `V::LANES` bytes.
/// Note that for the end of the string, this might read slightly past the valid data,
/// which is handled by `check_cross_page` to avoid page faults.
#[inline(always)]
unsafe fn load_simd_chunk<V: Simd>(ptr: *const u8) -> V {
    let chunk = from_raw_parts(ptr, V::LANES);
    V::from_slice_unaligned_unchecked(chunk)
}

/// Lookup table for escape sequences.
/// Format: `(length_of_escape_sequence, [bytes; 8])`
/// The bytes array is null-padded.
/// Example: `\n` (newline) -> `(2, b"\\n\0...")`
const QUOTE_TAB: [(u8, [u8; 8]); 256] = [
    // 0x00 ~ 0x1f  (Control characters)
    (6, *b"\\u0000\0\0"),
    (6, *b"\\u0001\0\0"),
    (6, *b"\\u0002\0\0"),
    (6, *b"\\u0003\0\0"),
    (6, *b"\\u0004\0\0"),
    (6, *b"\\u0005\0\0"),
    (6, *b"\\u0006\0\0"),
    (6, *b"\\u0007\0\0"),
    (2, *b"\\b\0\0\0\0\0\0"),
    (2, *b"\\t\0\0\0\0\0\0"),
    (2, *b"\\n\0\0\0\0\0\0"),
    (6, *b"\\u000b\0\0"),
    (2, *b"\\f\0\0\0\0\0\0"),
    (2, *b"\\r\0\0\0\0\0\0"),
    (6, *b"\\u000e\0\0"),
    (6, *b"\\u000f\0\0"),
    (6, *b"\\u0010\0\0"),
    (6, *b"\\u0011\0\0"),
    (6, *b"\\u0012\0\0"),
    (6, *b"\\u0013\0\0"),
    (6, *b"\\u0014\0\0"),
    (6, *b"\\u0015\0\0"),
    (6, *b"\\u0016\0\0"),
    (6, *b"\\u0017\0\0"),
    (6, *b"\\u0018\0\0"),
    (6, *b"\\u0019\0\0"),
    (6, *b"\\u001a\0\0"),
    (6, *b"\\u001b\0\0"),
    (6, *b"\\u001c\0\0"),
    (6, *b"\\u001d\0\0"),
    (6, *b"\\u001e\0\0"),
    (6, *b"\\u001f\0\0"),
    // 0x20 ~ 0x2f (Includes quote " and backslash \)
    (0, [0; 8]),
    (0, [0; 8]),
    (2, *b"\\\"\0\0\0\0\0\0"), // " -> \"
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    // 0x30 ~ 0x3f
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    // 0x40 ~ 0x4f
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    // 0x50 ~ 0x5f
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (2, *b"\\\\\0\0\0\0\0\0"), // \ -> \\
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    // 0x60 ~ 0xff
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
    (0, [0; 8]),
];

/// Boolean lookup table indicating if a character needs escaping.
/// 1 = needs escape, 0 = safe.
const NEED_ESCAPED: [u8; 256] = [
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Handes escaping of a sequence of characters character-by-character.
///
/// This is the slow path used when SIMD detection finds at least one character
/// that needs escaping in the current chunk.
#[inline(always)]
unsafe fn escape_unchecked(
    src_ptr: &mut *const u8,
    remaining_bytes: &mut usize,
    dst_ptr: &mut *mut u8,
) {
    assert!(*remaining_bytes >= 1);
    loop {
        let byte = *(*src_ptr);
        let escape_len = QUOTE_TAB[byte as usize].0 as usize;
        assert!(
            escape_len != 0,
            "char is {}, cnt is {},  NEED_ESCAPED is {}",
            byte as char,
            escape_len,
            NEED_ESCAPED[byte as usize]
        );
        // Copy the escape sequence (e.g., "\u0000") to the destination.
        // We copy 8 bytes blindly because the buffer is guaranteed to have enough space.
        std::ptr::copy_nonoverlapping(QUOTE_TAB[byte as usize].1.as_ptr(), *dst_ptr, 8);
        // Advance pointers
        (*dst_ptr) = (*dst_ptr).add(escape_len);
        (*src_ptr) = (*src_ptr).add(1);
        (*remaining_bytes) -= 1;

        // Stop if we run out of bytes or if the next character is safe (does not need escaping).
        // If it's safe, we return to the fast SIMD loop.
        if (*remaining_bytes) == 0 || NEED_ESCAPED[*(*src_ptr) as usize] == 0 {
            return;
        }
    }
}

/// Checks if reading `step` bytes from `ptr` would cross a 4KB memory page boundary.
///
/// This is critical when using SIMD loads on the tail of a string, as reading past
/// the end of the string into an unmapped page would cause a segfault.
#[inline(always)]
fn check_cross_page(ptr: *const u8, step: usize) -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let page_size = 4096;
        ((ptr as usize & (page_size - 1)) + step) > page_size
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // not check page cross in fallback envs, always true
        true
    }
}

#[inline(always)]
fn format_string(input_str: &str, writer: &mut Vec<u8>, need_quote: bool) {
    // 1. Calculate the worst-case required size for the new string data.
    // Each character could potentially expand to 6 bytes (\uXXXX).
    // +32 for SIMD padding safety (loading/writing 32 bytes at once).
    // +3 for quotes ("...") and null termination or alignment slop.
    let worst_case_required = input_str.len() * 6 + 32 + 3;
    let original_len = writer.len();

    // 2. Ensure the vector has enough TOTAL capacity to hold the new data.
    // This allows us to use unsafe pointer writes without bounds checking in the loop.
    writer.reserve(worst_case_required);

    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    let mut chunk: u8x16;
    #[cfg(not(all(target_arch = "aarch64", target_feature = "neon")))]
    let mut chunk: u8x32;

    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    const LANES: usize = 16;
    #[cfg(not(all(target_arch = "aarch64", target_feature = "neon")))]
    const LANES: usize = 32;

    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    #[inline]
    fn escaped_mask(v: u8x16) -> NeonBits {
        let x1f = u8x16::splat(0x1f); // 0x00 ~ 0x20
        let backslash = u8x16::splat(b'\\');
        let quote = u8x16::splat(b'"');
        let v = v.le(&x1f) | v.eq(&backslash) | v.eq(&quote);
        v.bitmask()
    }

    #[cfg(not(all(target_arch = "aarch64", target_feature = "neon")))]
    #[inline]
    fn escaped_mask(v: u8x32) -> u32 {
        let x1f = u8x32::splat(0x1f); // 0x00 ~ 0x20
        let backslash = u8x32::splat(b'\\');
        let quote = u8x32::splat(b'"');
        let v = v.le(&x1f) | v.eq(&backslash) | v.eq(&quote);
        v.bitmask()
    }

    unsafe {
        let input_bytes = input_str.as_bytes();
        let mut src_ptr = input_bytes.as_ptr();
        // Get a pointer to the END of the existing data in the buffer (where we start writing).
        let dst_start_ptr = writer.as_mut_ptr().add(original_len);
        let mut dst_ptr = dst_start_ptr;
        let mut remaining_len: usize = input_bytes.len();

        if need_quote {
            *dst_ptr = b'"';
            dst_ptr = dst_ptr.add(1);
        }

        // --- Main SIMD Loop ---
        // Process the string in chunks of `LANES` bytes (16 or 32).
        while remaining_len >= LANES {
            // Load a chunk from the input.
            chunk = load_simd_chunk(src_ptr);

            // Speculatively write the chunk to the destination assuming no escapes needed.
            // This works because we reserved enough space. If we need to escape,
            // we will overwrite this or move the pointer differently.
            chunk
                .write_to_slice_unaligned_unchecked(std::slice::from_raw_parts_mut(dst_ptr, LANES));
            let mask = escaped_mask(chunk);

            if mask.all_zero() {
                // Fast path: No characters in this chunk need escaping.
                remaining_len -= LANES;
                dst_ptr = dst_ptr.add(LANES);
                src_ptr = src_ptr.add(LANES);
            } else {
                // Slow path: Found at least one character needing escape.
                // `first_offset` tells us how many valid characters are before the first escapable one.
                let cn = mask.first_offset();
                remaining_len -= cn;
                dst_ptr = dst_ptr.add(cn);
                src_ptr = src_ptr.add(cn);
                escape_unchecked(&mut src_ptr, &mut remaining_len, &mut dst_ptr);
            }
        }

        // --- Tail Handling ---
        // Handle the remaining bytes (less than `LANES`).
        let mut temp: [u8; LANES] = [0u8; LANES];
        while remaining_len > 0 {
            // If we are near a page boundary, we can't do an unaligned load that crosses the page
            // because the next page might not be mapped.
            chunk = if check_cross_page(src_ptr, LANES) {
                std::ptr::copy_nonoverlapping(src_ptr, temp[..].as_mut_ptr(), remaining_len);
                load_simd_chunk(temp[..].as_ptr())
            } else {
                // Safe to load even if it reads past the end of string (but within the page)
                load_simd_chunk(src_ptr)
            };
            // Write speculatively
            chunk
                .write_to_slice_unaligned_unchecked(std::slice::from_raw_parts_mut(dst_ptr, LANES));

            // Calculate mask, but ignore "garbage" bits from reading past the end of the string
            let mask = escaped_mask(chunk).clear_high_bits(LANES - remaining_len);

            if mask.all_zero() {
                dst_ptr = dst_ptr.add(remaining_len);
                break;
            } else {
                let safe_len = mask.first_offset();
                remaining_len -= safe_len;
                dst_ptr = dst_ptr.add(safe_len);
                src_ptr = src_ptr.add(safe_len);
                escape_unchecked(&mut src_ptr, &mut remaining_len, &mut dst_ptr);
            }
        }
        if need_quote {
            *dst_ptr = b'"';
            dst_ptr = dst_ptr.add(1);
        }
        // Calculate how many bytes we've written...
        let written_len = dst_ptr.offset_from(dst_start_ptr) as usize;
        // ...and update the vector's length to reflect the new data.
        writer.set_len(original_len + written_len);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_quote() {
        let mut dst: Vec<u8> = Vec::with_capacity(1000);

        format_string("", &mut dst, true);
        assert_eq!(dst.as_slice(), b"\"\"");

        format_string("\x00", &mut dst, true);
        assert_eq!(dst.as_slice(), b"\"\"\"\\u0000\"");

        format_string("test", &mut dst, true);
        assert_eq!(dst.as_slice(), b"\"\"\"\\u0000\"\"test\"");

        format_string("test\"test", &mut dst, true);
        assert_eq!(dst.as_slice(), b"\"\"\"\\u0000\"\"test\"\"test\\\"test\"");

        format_string("\\testtest\"", &mut dst, true);
        assert_eq!(
            dst.as_slice(),
            b"\"\"\"\\u0000\"\"test\"\"test\\\"test\"\"\\\\testtest\\\"\""
        );

        let long_str = "this is a long string that should be \\\"quoted and escaped multiple \
                        times to test the performance and correctness of the function.";
        format_string(long_str, &mut dst, true);
        assert_eq!(dst.as_slice(), b"\"\"\"\\u0000\"\"test\"\"test\\\"test\"\"\\\\testtest\\\"\"\"this is a long string that should be \\\\\\\"quoted and escaped multiple times to test the performance and correctness of the function.\"");
    }
}
