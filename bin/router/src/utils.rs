use serde::de;
use std::hash::{Hash, Hasher};

pub type BoxError = Box<dyn std::error::Error>;

/// A wrapper for a string slice (`&'a str`) that implements `Hash`, `PartialEq`,
/// and `Eq` based on the pointer address of the slice, not its content.
///
/// Why is this needed?
///
/// In performance-critical code, especially inside tight loops, the cost of hashing
/// the full content of a string can become a bottleneck. This is true even if the
/// strings being hashed are the same few instances repeated over and over.
///
/// This wrapper is designed for the specific scenario where we have string slices
/// that are guaranteed to have the same memory address for the same conceptual value
/// (e.g., type names from a schema that is loaded once and lives for the duration
/// of the request).
///
/// By hashing the pointer address (which is just a number) instead of the string's
/// content, we make `HashSet` or `HashMap` lookups incredibly fast,
/// reducing the operation to a single integer hash and comparison.
///
/// Warning!
///
/// Only use this wrapper when you can guarantee that two strings with the same content
/// will have the same memory address. It is suitable for static strings or strings
/// coming from a long-lived, stable source like a schema, but it would produce
/// incorrect results if used on dynamically generated strings.
#[derive(Debug, Copy, Clone)]
pub struct StrByAddr<'a>(pub &'a str);

impl<'a> Hash for StrByAddr<'a> {
    #[inline(always)]
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(self.0, state);
    }
}

impl<'a> PartialEq for StrByAddr<'a> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(self.0, other.0)
    }
}

impl<'a> Eq for StrByAddr<'a> {}

pub trait MapAccessSerdeExt<'de>: de::MapAccess<'de> {
    #[inline]
    /// Deserializes an optional field value from the current map entry into `slot`.
    /// Returns a duplicate-field error when the same field appears more than once.
    fn deserialize_once_into_option<T>(
        &mut self,
        slot: &mut Option<T>,
        field_name: &'static str,
    ) -> Result<(), Self::Error>
    where
        T: serde::Deserialize<'de>,
    {
        if slot.is_some() {
            return Err(de::Error::duplicate_field(field_name));
        }

        *slot = self.next_value::<Option<T>>()?;
        Ok(())
    }
}

impl<'de, A> MapAccessSerdeExt<'de> for A where A: de::MapAccess<'de> {}

/// Turns a `Vec<u8>` into `ntex::util::Bytes` without copying it.
/// `Bytes::from(Vec<u8>)` copies, which for a response body is a second full copy of the response.
pub fn vec_into_ntex_bytes(vec: Vec<u8>) -> ntex::util::Bytes {
    ntex::util::Bytes::from_ext(VecStorage(vec))
}

struct VecStorage(Vec<u8>);

// The Vec sits behind an Arc, so cloning a `Bytes` is a refcount bump.
// `addr` is the `Arc<Vec<u8>>` pointer, `len` is the Vec's length.
//
// Why the unsafe bits are fine:
// - `addr` always comes from `Arc::into_raw` in `create`, and ntex passes it back untouched
//   (when it slices or advances, it copies instead of moving `addr`), so casting back to
//   `Vec<u8>` gives the same Arc.
// - every `Bytes` handle owns exactly one strong count: `create` gives one, `clone` adds one,
//   `drop` takes one. ntex only clones a live handle, so the Arc is still alive in `clone`.
// - nothing touches the Vec once it's in the Arc, so its pointer and length never change
//   and reading it from many threads is fine.
// ntex-bytes is pinned with `=` in Cargo.toml, so recheck all of this on every bump.
const VEC_STORAGE_VTABLE: ntex_bytes::StorageVTable = ntex_bytes::StorageVTable::new(
    |addr, _| unsafe { (*addr.cast::<Vec<u8>>()).as_ptr() },
    |_, len| len,
    |addr, len| unsafe {
        std::sync::Arc::increment_strong_count(addr.cast::<Vec<u8>>());
        Some((addr, len))
    },
    |addr, _| unsafe { std::sync::Arc::decrement_strong_count(addr.cast::<Vec<u8>>()) },
);

impl ntex_bytes::StorageExt for VecStorage {
    fn create(self) -> (*const u8, usize, &'static ntex_bytes::StorageVTable) {
        let len = self.0.len();
        let addr = std::sync::Arc::into_raw(std::sync::Arc::new(self.0));
        (addr.cast::<u8>(), len, &VEC_STORAGE_VTABLE)
    }
}

#[cfg(test)]
mod tests {
    use super::vec_into_ntex_bytes;

    #[test]
    fn vec_into_ntex_bytes_keeps_the_data_through_clones_and_slices() {
        let data: Vec<u8> = (0..200u8).collect();
        let ptr = data.as_ptr();
        let bytes = vec_into_ntex_bytes(data.clone());
        assert_eq!(&bytes[..], &data[..]);

        let clone = bytes.clone();
        assert_eq!(clone.as_ptr(), bytes.as_ptr(), "a clone shares the buffer");
        drop(bytes);
        assert_eq!(&clone[..], &data[..], "the buffer outlives the original");

        assert_eq!(&clone.slice(10..20)[..], &data[10..20]);
        assert_eq!(&clone[..], &data[..]);

        let moved = vec_into_ntex_bytes(data);
        assert_eq!(moved.as_ptr(), ptr, "the Vec's buffer is used as is");
    }

    // When the socket takes only part of the body, ntex-net calls `advance_to`.
    // For external storage ntex-bytes copies whatever's left into its own buffer,
    // so the zero-copy only holds when the body goes out in one write.
    // If this starts failing on the pointer checks, ntex stopped copying here.
    #[test]
    fn vec_into_ntex_bytes_copies_the_rest_after_a_partial_write() {
        let data: Vec<u8> = (0..=255u8).cycle().take(64 * 1024).collect();
        let mut body = vec_into_ntex_bytes(data.clone());
        // take the pointer from the body, `data` is a different allocation
        let ptr = body.as_ptr();

        body.advance_to(1000);
        assert_eq!(&body[..], &data[1000..]);
        assert_ne!(
            body.as_ptr(),
            ptr.wrapping_add(1000),
            "the rest is a copy, not our buffer"
        );

        // after that it's ntex's own buffer, so the next write just moves the start
        let copy_ptr = body.as_ptr();
        body.advance_to(1000);
        assert_eq!(&body[..], &data[2000..]);
        assert_eq!(body.as_ptr(), copy_ptr.wrapping_add(1000), "copied only once");
    }

    #[test]
    fn vec_into_ntex_bytes_uses_the_length_not_the_capacity() {
        // response buffers are sized up front, so they usually have room to spare
        let mut data = Vec::with_capacity(4096);
        data.extend_from_slice(b"{\"data\":null}");
        let bytes = vec_into_ntex_bytes(data);
        assert_eq!(&bytes[..], b"{\"data\":null}");
        assert_eq!(bytes.len(), 13);

        let empty = vec_into_ntex_bytes(Vec::new());
        assert!(empty.is_empty());
        assert_eq!(&empty.clone()[..], b"");
    }

    // ntex can call the vtable from any thread, so clones get dropped on other threads
    #[test]
    fn vec_into_ntex_bytes_clones_can_be_dropped_on_other_threads() {
        let bytes = vec_into_ntex_bytes(vec![7u8; 1024]);
        let clones: Vec<_> = (0..4).map(|_| bytes.clone()).collect();
        drop(bytes);
        let handles: Vec<_> = clones
            .into_iter()
            .map(|b| std::thread::spawn(move || assert!(b.iter().all(|&x| x == 7))))
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }
}
