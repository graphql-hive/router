//! Experiment: an operation kept in a few flat arrays instead of a tree of boxes and strings.
//!
//! Every node lives in one `Vec` and points at others by index. A list of nodes (selections,
//! arguments, directives) is a `Span` into a shared array. Names are interned, so a name is a
//! `u32` and comparing two is comparing two numbers.
//!
//! After parsing, nothing is changed in place: normalization appends new nodes and new lists.
//! So a subtree that ends up in many places (a fragment spread in ten spots, a fragment copied
//! for every type it can be) is just its index, never copied.

pub mod arena;
pub mod convert;
pub mod normalize;
pub mod plan_document;
pub mod selection_tree;
pub mod tree;

pub use graphql_tools::flat::{
    Argument, Directive, Document, Interner, Kind, OperationKind, Pos, SelId, Span, Sym, TypeId,
    TypeRef, Value, ValueId, VariableDefinition, NONE,
};

/// A selection while normalization works on it: its selections are a `Span` of the normalizer's
/// own list of items, so a changed list is a new `Span` and an unchanged one compares equal.
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub kind: Kind,
    /// The field's name, or the fragment's for a spread.
    pub name: Sym,
    pub alias: Sym,
    pub type_condition: Sym,
    /// Into `Document::args`
    pub args: Span,
    /// Into `Document::directives`
    pub directives: Span,
    /// Into the normalizer's items
    pub children: Span,
}

/// An operation while normalization works on it, see `Selection`.
#[derive(Clone, Copy, Debug)]
pub struct Operation {
    pub kind: OperationKind,
    pub name: Sym,
    /// Into `Document::variables`
    pub variables: Span,
    pub selection_set: Span,
}

/// A fragment definition while normalization works on it, see `Selection`.
#[derive(Clone, Copy, Debug)]
pub struct Fragment {
    pub name: Sym,
    pub type_condition: Sym,
    pub selection_set: Span,
}

#[cfg(test)]
mod tests;
