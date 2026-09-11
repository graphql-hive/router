use std::mem::size_of;
use std::num::NonZeroU32;
use std::sync::Arc;

mod collect;
mod display;
mod encode;
mod ir;
mod merge;
mod rewrite;
mod tables;

use crate::executor::projection::plan::collect::Collector;
use crate::executor::projection::plan::encode::Encoder;
use crate::executor::projection::plan::merge::Merger;
use crate::query_planner::ast::operation::OperationDefinition;
use crate::query_planner::state::supergraph_state::OperationKind;
use bumpalo::Bump;

impl ProjectionPlan {
    pub fn from_operation<'schema>(
        operation: &OperationDefinition,
        schema: &'schema crate::executor::introspection::schema::SchemaMetadata,
    ) -> (&'schema str, ProjectionPlan) {
        let root = match operation.operation_kind {
            None | Some(OperationKind::Query) => schema.query_type_name.as_ref(),
            Some(OperationKind::Mutation) => schema.mutation_type_name.as_ref(),
            Some(OperationKind::Subscription) => schema.subscription_type_name.as_ref(),
        }
        .expect("validated operation has a root type");
        let arena = Bump::new();
        let collected = Collector::collect_fields(
            &operation.selection_set,
            schema,
            root,
            ir::Condition::Always,
            &arena,
        );
        let merged = Merger::merge_fields(collected, schema, root, &arena);
        (root, Encoder::encode(&merged))
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SymbolId(pub(crate) u32);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShapeId(pub(crate) u32);

// Uses `NonZeroU32` so `Option` stays 4 bytes: `None` is 0,
// `Some` stores table index + 1.
// Use `index()` to get the real index back.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GuardId(pub(crate) NonZeroU32);

// Uses `NonZeroU32` so `Option` stays 4 bytes: `None` is 0,
// `Some` stores table index + 1.
// Use `index()` to get the real index back.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConditionId(pub(crate) NonZeroU32);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SetId(pub(crate) u32);

/// A slice of one of the flat tables: fields, text, set members or shape bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    pub start: u32,
    pub len: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldRecord {
    /// The schema field name.
    pub field_name: SymbolId,
    /// The name written to the response.
    pub response_key: Range,
    /// Nullability and field flags, packed into one number.
    meta: FieldMeta,
    pub parent_guard: Option<GuardId>,
    pub condition: Option<ConditionId>,
    pub children: Range,
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FieldMeta(u32);

///
/// ┌ non-null (bit 31)
/// │┌ typename (bit 30)
/// ││┌┬ value kind (bits 29-28)
/// ││││
/// vvvv
/// 1001 0000 0000 0000 0000 0000 0000 0101
///      ^                                ^
///      └──────── shape id (bits 27-0) ──┘
impl FieldMeta {
    const SHAPE_BITS: u32 = 28;
    const SHAPE_ID_MASK: u32 = (1 << Self::SHAPE_BITS) - 1;

    /// Value kinds use two bits: 00 = passthrough, 01 = children, 10 = null.
    const VALUE_KIND: u32 = 0b11 << Self::SHAPE_BITS;
    pub(crate) const CHILDREN: u32 = 0b01 << Self::SHAPE_BITS;
    const NULL: u32 = 0b10 << Self::SHAPE_BITS;
    pub(crate) const TYPENAME: u32 = 0b0100 << Self::SHAPE_BITS;
    pub(crate) const NON_NULL: u32 = 0b1000 << Self::SHAPE_BITS;

    #[inline]
    pub(crate) fn new(shape: ShapeId, flags: u32) -> Self {
        debug_assert!(shape.0 <= Self::SHAPE_ID_MASK);
        debug_assert_eq!(flags & Self::SHAPE_ID_MASK, 0);
        Self(shape.0 | flags)
    }
}

/// Operations on a field's packed metadata.
impl FieldRecord {
    /// Returns the field's nullability shape.
    #[inline]
    pub fn nullability(&self) -> ShapeId {
        ShapeId(self.meta.0 & FieldMeta::SHAPE_ID_MASK)
    }

    /// Whether this field has nested selections.
    #[inline]
    pub fn has_children(&self) -> bool {
        self.meta.0 & FieldMeta::VALUE_KIND == FieldMeta::CHILDREN
    }

    /// Whether a rewrite forced this field to `null`.
    #[inline]
    pub fn is_null_value(&self) -> bool {
        self.meta.0 & FieldMeta::VALUE_KIND == FieldMeta::NULL
    }

    #[inline]
    fn set_value_kind(&mut self, kind: u32) {
        self.meta.0 = (self.meta.0 & !FieldMeta::VALUE_KIND) | kind;
    }

    #[inline]
    pub(crate) fn set_null(&mut self) {
        self.set_value_kind(FieldMeta::NULL);
    }

    #[inline]
    pub(crate) fn set_passthrough(&mut self) {
        self.set_value_kind(0);
    }

    #[inline]
    pub fn is_typename(&self) -> bool {
        self.meta.0 & FieldMeta::TYPENAME != 0
    }

    #[inline]
    pub fn is_non_null(&self) -> bool {
        self.meta.0 & FieldMeta::NON_NULL != 0
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Include(SymbolId),
    Skip(SymbolId),
    ParentType(GuardId),
    FieldType(GuardId),
    EnumValues(SetId),
    And(ConditionId, ConditionId),
    Or(ConditionId, ConditionId),
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guard {
    Exact(SymbolId),
    Set(SetId),
}

#[derive(Clone, Debug, Default)]
pub struct ProjectionTables {
    pub text: Box<str>,
    pub symbols: Box<[Range]>,
    pub conditions: Box<[Condition]>,
    pub guards: Box<[Guard]>,
    pub sets: Box<[Range]>,
    pub set_members: Box<[SymbolId]>,
    pub shapes: Box<[Range]>,
    pub shape_bytes: Box<[u8]>,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectionPlan {
    roots: Range,
    fields: Box<[FieldRecord]>,
    tables: Arc<ProjectionTables>,
}

bitflags::bitflags! {
    /// One byte per level of a field's type, from the outside in.
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct ShapeFlags: u8 {
        const LIST = 1 << 0;
        const NON_NULL = 1 << 1;
    }
}

/// Use a linear search for small sets.
const LINEAR_SET_LOOKUP_MAX_LEN: usize = 8;

impl Range {
    pub fn end(self) -> usize {
        self.start as usize + self.len as usize
    }

    #[inline]
    fn slice<T>(self, table: &[T]) -> &[T] {
        &table[self.start as usize..self.end()]
    }
}

impl GuardId {
    fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

impl ConditionId {
    fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

impl ProjectionPlan {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.roots.len == 0
    }

    #[inline]
    pub fn fields(&self, range: Range) -> &[FieldRecord] {
        range.slice(&self.fields)
    }

    #[inline]
    pub fn root_fields(&self) -> &[FieldRecord] {
        self.fields(self.roots)
    }

    #[inline]
    pub fn text(&self, id: SymbolId) -> &str {
        let range = self.tables.symbols[id.0 as usize];
        self.text_range(range)
    }

    #[inline]
    pub fn text_range(&self, range: Range) -> &str {
        &self.tables.text[range.start as usize..range.end()]
    }

    #[inline]
    pub fn shape(&self, id: ShapeId) -> &[u8] {
        self.tables.shapes[id.0 as usize].slice(&self.tables.shape_bytes)
    }

    #[inline]
    pub fn members(&self, id: SetId) -> &[SymbolId] {
        self.tables.sets[id.0 as usize].slice(&self.tables.set_members)
    }

    #[inline]
    pub fn set_matches(&self, id: SetId, name: &str) -> bool {
        let members = self.members(id);
        if members.len() <= LINEAR_SET_LOOKUP_MAX_LEN {
            members.iter().any(|member| self.text(*member) == name)
        } else {
            members
                .binary_search_by(|member| self.text(*member).cmp(name))
                .is_ok()
        }
    }

    #[inline]
    pub fn field_name(&self, field: &FieldRecord) -> &str {
        self.text(field.field_name)
    }

    #[inline]
    pub fn response_key(&self, field: &FieldRecord) -> &str {
        self.text_range(field.response_key)
    }

    #[inline]
    pub fn guard_matches(&self, id: GuardId, type_name: &str) -> bool {
        let guard = self.tables.guards[id.index()];
        match guard {
            Guard::Exact(symbol) => self.text(symbol) == type_name,
            Guard::Set(set) => self.set_matches(set, type_name),
        }
    }

    #[inline]
    pub fn condition(&self, id: ConditionId) -> Condition {
        self.tables.conditions[id.index()]
    }

    #[inline]
    pub fn guard(&self, id: GuardId) -> Guard {
        self.tables.guards[id.index()]
    }
}

const _: () = assert!(size_of::<FieldRecord>() == 32);
const _: () = assert!(size_of::<Condition>() == 12);
const _: () = assert!(size_of::<Option<GuardId>>() == 4);
