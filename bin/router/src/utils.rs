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
