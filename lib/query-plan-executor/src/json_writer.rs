///
/// I took it from https://github.com/zotta/json-writer-rs/blob/f45e2f25cede0e06be76a94f6e45608780a835d4/src/lib.rs#L853
///

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

    return result;
}
static REPLACEMENTS: [u8; 256] = get_replacements();
static HEX: [u8; 16] = *b"0123456789ABCDEF";

///
/// Escapes and append part of string
///
#[inline(always)]
pub fn write_and_escape_string(output_buffer: &mut String, input: &str) {
    output_buffer.push('"');

    // All of the relevant characters are in the ansi range (<128).
    // This means we can safely ignore any utf-8 characters and iterate over the bytes directly
    let mut num_bytes_written: usize = 0;
    let mut index: usize = 0;
    let bytes = input.as_bytes();
    while index < bytes.len() {
        let cur_byte = bytes[index];
        let replacement = REPLACEMENTS[cur_byte as usize];
        if replacement != 0 {
            if num_bytes_written < index {
                // Checks can be omitted here:
                // We know that index is smaller than the output_buffer length.
                // We also know that num_bytes_written is smaller than index
                // We also know that the boundaries are not in the middle of an utf-8 multi byte sequence, because those characters are not escaped
                output_buffer.push_str(unsafe { input.get_unchecked(num_bytes_written..index) });
            }
            if replacement == b'u' {
                let bytes: [u8; 6] = [
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[((cur_byte / 16) & 0xF) as usize],
                    HEX[(cur_byte & 0xF) as usize],
                ];
                // Checks can be omitted here: We know bytes is a valid utf-8 string (see above)
                output_buffer.push_str(unsafe { std::str::from_utf8_unchecked(&bytes) });
            } else {
                let bytes: [u8; 2] = [b'\\', replacement];
                // Checks can be omitted here: We know bytes is a valid utf-8 string, because the replacement table only contains characters smaller than 128
                output_buffer.push_str(unsafe { std::str::from_utf8_unchecked(&bytes) });
            }
            num_bytes_written = index + 1;
        }
        index += 1;
    }
    if num_bytes_written < bytes.len() {
        // Checks can be omitted here:
        // We know that num_bytes_written is smaller than index
        // We also know that num_bytes_written not in the middle of an utf-8 multi byte sequence, because those are not escaped
        output_buffer.push_str(unsafe { input.get_unchecked(num_bytes_written..bytes.len()) });
    }

    output_buffer.push('"');
}
