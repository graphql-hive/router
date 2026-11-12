//! A GraphQL executable document in a few flat arrays.
//!
//! Every node lives in one `Vec` and points at others by index. Names and string values are
//! interned, so a name is a `u32` and comparing two is comparing two numbers. A selection set is a
//! `ListId`: it stays the same while the list changes, so a document can be built up or changed in
//! place, and a finished one can be shared read-only.
//!
//! The parser reads the same language as `parser::query::parse_query`, positions included, so
//! validation errors point at the same places.

mod parse;
mod print;

#[cfg(test)]
mod tests;

pub use parse::{parse, parse_with_limits, ParseError};
pub use print::minify;

use std::collections::HashMap;

/// An interned string.
pub type Sym = u32;
/// No name, alias, type condition or value.
pub const NONE: u32 = u32::MAX;
pub type SelId = u32;
pub type ValueId = u32;
pub type TypeId = u32;

/// A run of items in one of the shared arrays.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct Span {
    pub start: u32,
    pub len: u32,
}

impl Span {
    pub const EMPTY: Span = Span { start: 0, len: 0 };

    #[inline]
    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// A selection set. No `==` on purpose: two lists are equal by what's in them, never by id.
#[derive(Clone, Copy, Debug)]
pub struct ListId(pub u32);

/// Where something starts in the source, one-based, counted the way `parser::Pos` is.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Pos {
    pub line: u32,
    pub column: u32,
}

#[derive(Default, Clone, Debug)]
pub struct Interner {
    map: HashMap<Box<str>, Sym>,
    strings: Vec<Box<str>>,
}

impl Interner {
    #[inline]
    pub fn intern(&mut self, s: &str) -> Sym {
        if let Some(&sym) = self.map.get(s) {
            return sym;
        }
        let sym = self.strings.len() as Sym;
        self.strings.push(s.into());
        self.map.insert(s.into(), sym);
        sym
    }

    #[inline]
    pub fn get(&self, s: &str) -> Option<Sym> {
        self.map.get(s).copied()
    }

    #[inline]
    pub fn str(&self, sym: Sym) -> &str {
        &self.strings[sym as usize]
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Variable(Sym),
    Int(i64),
    Float(f64),
    String(Sym),
    Boolean(bool),
    Null,
    Enum(Sym),
    /// Into `Document::value_items`
    List(Span),
    /// Into `Document::object_fields`, as written: in order, repeats too.
    Object(Span),
}

#[derive(Clone, Copy, Debug)]
pub struct Argument {
    pub name: Sym,
    pub value: ValueId,
}

#[derive(Clone, Copy, Debug)]
pub struct Directive {
    pub name: Sym,
    /// Into `Document::args`
    pub args: Span,
    pub pos: Pos,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Field,
    InlineFragment,
    FragmentSpread,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub kind: Kind,
    /// The field's name, or the fragment's for a spread.
    pub name: Sym,
    pub alias: Sym,
    /// An inline fragment's, `NONE` without one.
    pub type_condition: Sym,
    /// Into `Document::args`
    pub args: Span,
    /// Into `Document::directives`
    pub directives: Span,
    /// Empty for a leaf field and a spread.
    pub children: ListId,
    pub pos: Pos,
}

#[derive(Clone, Copy, Debug)]
pub enum TypeRef {
    Named(Sym),
    List(TypeId),
    NonNull(TypeId),
}

#[derive(Clone, Copy, Debug)]
pub struct VariableDefinition {
    pub name: Sym,
    pub ty: TypeId,
    pub default_value: Option<ValueId>,
    pub pos: Pos,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OperationKind {
    Query,
    Mutation,
    Subscription,
    /// `{ ... }` with no keyword
    Shorthand,
}

#[derive(Clone, Copy, Debug)]
pub struct Operation {
    pub kind: OperationKind,
    pub name: Sym,
    /// Into `Document::variables`
    pub variables: Span,
    /// Into `Document::directives`
    pub directives: Span,
    pub selection_set: ListId,
    /// Of the keyword, or of `{` for the shorthand.
    pub pos: Pos,
}

#[derive(Clone, Copy, Debug)]
pub struct Fragment {
    pub name: Sym,
    pub type_condition: Sym,
    /// Into `Document::directives`
    pub directives: Span,
    pub selection_set: ListId,
    pub pos: Pos,
}

#[derive(Clone, Copy, Debug)]
pub enum Definition {
    Operation(Operation),
    Fragment(Fragment),
}

#[derive(Default, Clone, Debug)]
pub struct Document {
    pub strings: Interner,
    pub values: Vec<Value>,
    pub value_items: Vec<ValueId>,
    pub object_fields: Vec<(Sym, ValueId)>,
    pub args: Vec<Argument>,
    pub directives: Vec<Directive>,
    pub selections: Vec<Selection>,
    /// Where each selection set's items are in `items`.
    pub lists: Vec<Span>,
    pub items: Vec<SelId>,
    pub types: Vec<TypeRef>,
    pub variables: Vec<VariableDefinition>,
    /// In document order.
    pub definitions: Vec<Definition>,
}

impl Document {
    #[inline]
    pub fn str(&self, sym: Sym) -> &str {
        self.strings.str(sym)
    }

    /// The string, or `None` for `NONE`.
    #[inline]
    pub fn opt_str(&self, sym: Sym) -> Option<&str> {
        (sym != NONE).then(|| self.str(sym))
    }

    #[inline]
    pub fn selection(&self, id: SelId) -> &Selection {
        &self.selections[id as usize]
    }

    #[inline]
    pub fn items(&self, list: ListId) -> &[SelId] {
        &self.items[self.lists[list.0 as usize].range()]
    }

    #[inline]
    pub fn is_empty(&self, list: ListId) -> bool {
        self.lists[list.0 as usize].is_empty()
    }

    #[inline]
    pub fn args_of(&self, span: Span) -> &[Argument] {
        &self.args[span.range()]
    }

    #[inline]
    pub fn directives_of(&self, span: Span) -> &[Directive] {
        &self.directives[span.range()]
    }

    #[inline]
    pub fn value(&self, id: ValueId) -> &Value {
        &self.values[id as usize]
    }

    /// A new empty selection set.
    pub fn new_list(&mut self) -> ListId {
        self.lists.push(Span::EMPTY);
        ListId((self.lists.len() - 1) as u32)
    }

    /// A list with these items, put in one run at the end of `items`.
    pub fn list_of(&mut self, items: &[SelId]) -> ListId {
        let start = self.items.len() as u32;
        self.items.extend_from_slice(items);
        self.lists.push(Span {
            start,
            len: items.len() as u32,
        });
        ListId((self.lists.len() - 1) as u32)
    }

    /// Adds `item` to the end of `list`. The list moves to the end of `items` first if it isn't
    /// there already; the old run stays behind.
    pub fn push(&mut self, list: ListId, item: SelId) {
        let span = self.lists[list.0 as usize];
        if span.start + span.len != self.items.len() as u32 {
            let start = self.items.len() as u32;
            self.items.extend_from_within(span.range());
            self.lists[list.0 as usize].start = start;
        }
        self.items.push(item);
        self.lists[list.0 as usize].len += 1;
    }

    /// The operations, in document order.
    pub fn operations(&self) -> impl Iterator<Item = &Operation> {
        self.definitions.iter().filter_map(|d| match d {
            Definition::Operation(op) => Some(op),
            Definition::Fragment(_) => None,
        })
    }

    /// The fragment definitions, in document order.
    pub fn fragments(&self) -> impl Iterator<Item = &Fragment> {
        self.definitions.iter().filter_map(|d| match d {
            Definition::Fragment(f) => Some(f),
            Definition::Operation(_) => None,
        })
    }
}
