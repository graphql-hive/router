use std::hash::{Hash, Hasher};

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
