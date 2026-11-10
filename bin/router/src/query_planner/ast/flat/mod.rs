//! Experiment: an operation kept in a few flat arrays instead of a tree of boxes and strings.
//!
//! Every node lives in one `Vec` and points at others by index. A list of nodes (selections,
//! arguments, directives) is a `Span` into a shared array. Names are interned, so a name is a
//! `u32` and comparing two is comparing two numbers.
//!
//! After parsing, nothing is changed in place: normalization appends new nodes and new lists.
//! So a subtree that ends up in many places (a fragment spread in ten spots, a fragment copied
//! for every type it can be) is just its index, never copied.

pub mod convert;
pub mod normalize;
pub mod parse;

use ahash::HashMap;

/// An interned string.
pub type Sym = u32;
/// No name, alias or type condition.
pub const NONE: Sym = Sym::MAX;
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

#[derive(Default, Clone)]
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

#[derive(Clone, Copy, Debug)]
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
    /// Into `Document::object_fields`, sorted by name and without repeats, like a `BTreeMap`
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
    pub type_condition: Sym,
    /// Into `Document::args`
    pub args: Span,
    /// Into `Document::directives`
    pub directives: Span,
    /// Into `Document::children`
    pub children: Span,
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
    pub selection_set: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct Fragment {
    pub name: Sym,
    pub type_condition: Sym,
    pub selection_set: Span,
}

#[derive(Clone, Copy, Debug)]
pub enum Definition {
    Operation(Operation),
    Fragment(Fragment),
}

#[derive(Default, Clone)]
pub struct Document {
    pub strings: Interner,
    pub values: Vec<Value>,
    pub value_items: Vec<ValueId>,
    pub object_fields: Vec<(Sym, ValueId)>,
    pub args: Vec<Argument>,
    pub directives: Vec<Directive>,
    pub selections: Vec<Selection>,
    pub children: Vec<SelId>,
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
}
