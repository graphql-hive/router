//! I took it from https://github.com/zotta/json-writer-rs/blob/f45e2f25cede0e06be76a94f6e45608780a835d4/src/lib.rs#L853

use std::io;

const fn get_replacements() -> [u8; 256] {
    // NOTE: Only characters smaller than 128 are allowed here.
    // Trying to escape values above 128 would generate invalid utf-8 output
    // -----
    // see https://www.json.org/json-en.html
    let mut result = [0u8; 256];
    // Escape everything from 0 to 0x1F
    let mut i = 0;
    while i < 0x20 {
        result[i] = b'u';
        i += 1;
    }
    result[b'\"' as usize] = b'"';
    result[b'\\' as usize] = b'\\';
    result[b'/' as usize] = b'/';
    result[8] = b'b';
    result[0xc] = b'f';
    result[b'\n' as usize] = b'n';
    result[b'\r' as usize] = b'r';
    result[b'\t' as usize] = b't';
    result[0] = b'u';

    result
}

static REPLACEMENTS: [u8; 256] = get_replacements();
static HEX: [u8; 16] = *b"0123456789ABCDEF";

/// Escapes and append part of string
#[inline(always)]
pub fn write_and_escape_string_writer(writer: &mut impl io::Write, input: &str) -> io::Result<()> {
    writer.write_all(b"\"")?;

    let bytes = input.as_bytes();
    let mut last_write = 0;

    for (i, &byte) in bytes.iter().enumerate() {
        let replacement = REPLACEMENTS[byte as usize];
        if replacement != 0 {
            if last_write < i {
                writer.write_all(&bytes[last_write..i])?;
            }

            if replacement == b'u' {
                let hex_bytes: [u8; 6] = [
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[((byte / 16) & 0xF) as usize],
                    HEX[(byte & 0xF) as usize],
                ];
                writer.write_all(&hex_bytes)?;
            } else {
                let escaped_bytes: [u8; 2] = [b'\\', replacement];
                writer.write_all(&escaped_bytes)?;
            }
            last_write = i + 1;
        }
    }

    if last_write < bytes.len() {
        writer.write_all(&bytes[last_write..])?;
    }

    writer.write_all(b"\"")
}
