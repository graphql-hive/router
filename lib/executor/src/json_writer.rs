//! I took it from https://github.com/cloudwego/sonic-rs/blob/5ad7f96877fec7d3d33a5971b8bafe5af40fd3ff/src/util/string.rs
use bytes::BufMut;
use std::slice::from_raw_parts;

use crate::utils::consts::NULL;

#[inline(always)]
pub fn write_and_escape_string(writer: &mut Vec<u8>, input: &str) {
    format_string(input, writer, true);
}

#[cfg(not(all(target_feature = "neon", target_arch = "aarch64")))]
use sonic_simd::u8x32;
#[cfg(all(target_feature = "neon", target_arch = "aarch64"))]
use sonic_simd::{bits::NeonBits, u8x16};
use sonic_simd::{BitMask, Mask, Simd};

#[inline(always)]
unsafe fn load<V: Simd>(ptr: *const u8) -> V {
    let chunk = from_raw_parts(ptr, V::LANES);
    V::from_slice_unaligned_unchecked(chunk)
}

const QUOTE_TAB: [(u8, [u8; 8]); 256] = [
    // 0x00 ~ 0x1f
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
    // 0x20 ~ 0x2f
    (0, [0; 8]),
    (0, [0; 8]),
    (2, *b"\\\"\0\0\0\0\0\0"),
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
    (2, *b"\\\\\0\0\0\0\0\0"),
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

// only check the src length.
#[inline(always)]
unsafe fn escape_unchecked(src: &mut *const u8, nb: &mut usize, dst: &mut *mut u8) {
    assert!(*nb >= 1);
    loop {
        let ch = *(*src);
        let cnt = QUOTE_TAB[ch as usize].0 as usize;
        assert!(
            cnt != 0,
            "char is {}, cnt is {},  NEED_ESCAPED is {}",
            ch as char,
            cnt,
            NEED_ESCAPED[ch as usize]
        );
        std::ptr::copy_nonoverlapping(QUOTE_TAB[ch as usize].1.as_ptr(), *dst, 8);
        (*dst) = (*dst).add(cnt);
        (*src) = (*src).add(1);
        (*nb) -= 1;
        if (*nb) == 0 || NEED_ESCAPED[*(*src) as usize] == 0 {
            return;
        }
    }
}

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
fn format_string(value: &str, dst: &mut Vec<u8>, need_quote: bool) {
    // 1. Calculate the worst-case required size for the new string data.
    let worst_case_required = value.len() * 6 + 32 + 3; // 6x for \uXXXX, 32 for SIMD padding, 3 for quotes/null
    let original_len = dst.len();

    // 2. Ensure the vector has enough TOTAL capacity to hold the new data.
    dst.reserve(worst_case_required);

    // This is the original assertion that caused the panic. It's incorrect for an
    // appending buffer and has been removed.
    // assert!(dst.len() >= value.len() * 6 + 32 + 3);

    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    let mut v: u8x16;
    #[cfg(not(all(target_arch = "aarch64", target_feature = "neon")))]
    let mut v: u8x32;

    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    const LANES: usize = 16;
    #[cfg(not(all(target_arch = "aarch64", target_feature = "neon")))]
    const LANES: usize = 32;

    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    #[inline]
    fn escaped_mask(v: u8x16) -> NeonBits {
        let x1f = u8x16::splat(0x1f); // 0x00 ~ 0x20
        let blash = u8x16::splat(b'\\');
        let quote = u8x16::splat(b'"');
        let v = v.le(&x1f) | v.eq(&blash) | v.eq(&quote);
        v.bitmask()
    }

    #[cfg(not(all(target_arch = "aarch64", target_feature = "neon")))]
    #[inline]
    fn escaped_mask(v: u8x32) -> u32 {
        let x1f = u8x32::splat(0x1f); // 0x00 ~ 0x20
        let blash = u8x32::splat(b'\\');
        let quote = u8x32::splat(b'"');
        let v = v.le(&x1f) | v.eq(&blash) | v.eq(&quote);
        v.bitmask()
    }

    unsafe {
        let slice = value.as_bytes();
        let mut sptr = slice.as_ptr();
        // Get a pointer to the END of the existing data in the buffer.
        let dstart = dst.as_mut_ptr().add(original_len);
        let mut dptr = dstart;
        let mut nb: usize = slice.len();

        if need_quote {
            *dptr = b'"';
            dptr = dptr.add(1);
        }
        while nb >= LANES {
            v = load(sptr);
            v.write_to_slice_unaligned_unchecked(std::slice::from_raw_parts_mut(dptr, LANES));
            let mask = escaped_mask(v);
            if mask.all_zero() {
                nb -= LANES;
                dptr = dptr.add(LANES);
                sptr = sptr.add(LANES);
            } else {
                let cn = mask.first_offset();
                nb -= cn;
                dptr = dptr.add(cn);
                sptr = sptr.add(cn);
                escape_unchecked(&mut sptr, &mut nb, &mut dptr);
            }
        }

        let mut temp: [u8; LANES] = [0u8; LANES];
        while nb > 0 {
            v = if check_cross_page(sptr, LANES) {
                std::ptr::copy_nonoverlapping(sptr, temp[..].as_mut_ptr(), nb);
                load(temp[..].as_ptr())
            } else {
                load(sptr)
            };
            v.write_to_slice_unaligned_unchecked(std::slice::from_raw_parts_mut(dptr, LANES));

            let mask = escaped_mask(v).clear_high_bits(LANES - nb);
            if mask.all_zero() {
                dptr = dptr.add(nb);
                break;
            } else {
                let cn = mask.first_offset();
                nb -= cn;
                dptr = dptr.add(cn);
                sptr = sptr.add(cn);
                escape_unchecked(&mut sptr, &mut nb, &mut dptr);
            }
        }
        if need_quote {
            *dptr = b'"';
            dptr = dptr.add(1);
        }
        // Calculate how many bytes we've written...
        let written_len = dptr.offset_from(dstart) as usize;
        // ...and update the vector's length to reflect the new data.
        dst.set_len(original_len + written_len);
    }
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
