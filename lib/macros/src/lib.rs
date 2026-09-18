//! Derives `hive_router`'s internal `HeapSize` trait.
//!
//! The generated impl sums `heap_size()` over every field, so a field added later is counted
//! without anyone touching the impl - which is the whole point of deriving it rather than
//! writing it out. Scalars cost nothing, since the trait already answers 0 for them.
//!
//! The expansion names the trait as `crate::heap_size::HeapSize`, so this only works inside
//! `hive-router`. That crate is the only consumer; anywhere else it fails to compile.

use proc_macro::TokenStream;

mod heap_size;
mod sum;

/// Sums the heap of every field.
///
/// `#[heap_size(bound = "...")]` on the type replaces the bounds the derive would infer, for
/// a generic whose fields are associated types rather than the parameter itself - the same
/// escape `#[serde(bound = "...")]` provides, and needed in the same places:
///
/// ```ignore
/// #[derive(HeapSize)]
/// #[heap_size(bound = "S::Operation: HeapSize, S::Requires: HeapSize")]
/// pub struct FetchNode<S: PlanState = Executable> { .. }
/// ```
#[proc_macro_derive(HeapSize, attributes(heap_size))]
pub fn derive_heap_size(input: TokenStream) -> TokenStream {
    heap_size::derive_heap_size(input)
}
