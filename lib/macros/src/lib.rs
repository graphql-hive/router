//! Derives `hive_router`'s internal `HeapSize` trait.
//!
//! Only usable inside `hive-router`: the expansion names `crate::heap_size::HeapSize`.

use proc_macro::TokenStream;

mod heap_size;
mod sum;

/// Sums the heap of every field.
///
/// `#[heap_size(bound = "...")]` replaces the inferred bounds, like `#[serde(bound = "...")]`,
/// for generics whose fields are associated types rather than the parameter itself:
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
