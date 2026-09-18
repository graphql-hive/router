//! How much heap a cached value hangs on to, for cache byte budgets.
//!
//! `heap_size` counts what a value owns behind its pointers, not `size_of::<Self>()`,
//! so nesting composes. Like [`ShrinkMemory`](crate::query_planner::ast::shrink::ShrinkMemory):
//! when a cached type grows a field, count it here too - `e2e/src/bin/memory_baseline.rs`
//! catches a field somebody forgot.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::num::NonZeroU32;
use std::sync::Arc;

use graphql_tools::parser::query;
use graphql_tools::parser::Pos;
use graphql_tools::validation::utils::ValidationError;

/// What an entry costs moka beyond its key and value: the hash map node, the eviction
/// bookkeeping, and the entry header. Without it a cache of near-empty values (a validation
/// result is an empty `Vec` whenever an operation is valid) would look free and grow without
/// bound. Checked against the real thing by `e2e/src/bin/memory_baseline.rs`.
const MOKA_ENTRY_OVERHEAD: usize = 200;

/// What one cache entry is charged against a byte budget, on insert.
pub(crate) fn entry_weight<K, V: HeapSize>(value: &V) -> u32 {
    let bytes = MOKA_ENTRY_OVERHEAD + size_of::<K>() + size_of::<V>() + value.heap_size();
    // Weights are u32; clamp huge values.
    bytes.min(u32::MAX as usize) as u32
}

/// Bytes a value owns on the heap.
pub(crate) trait HeapSize {
    /// Heap bytes reachable from this value, not counting `size_of::<Self>()`.
    fn heap_size(&self) -> usize;
}

/// A `BTreeMap` node holds up to 11 entries and is allocated whole however few of its slots
/// are filled, so a one-argument map pays for all eleven. Charging per entry instead
/// undercharged small maps by around 6x - and small is what a field's arguments are.
const BTREE_NODE_CAPACITY: usize = 11;

/// `LeafNode` layout as of current std: parent pointer, length, then key/value arrays.
const fn btree_node_size<K, V>() -> usize {
    2 * size_of::<usize>() + BTREE_NODE_CAPACITY * (size_of::<K>() + size_of::<V>())
}

/// Optimistic past one node: ignores partly-filled splits and internal nodes.
/// Exact at 11 entries or fewer.
const fn btree_nodes(len: usize) -> usize {
    len.div_ceil(BTREE_NODE_CAPACITY)
}

/// `Arc`/`Rc` put a strong and a weak count in front of the value.
const REFCOUNT_HEADER: usize = 2 * size_of::<usize>();

/// hashbrown keeps one control byte per bucket next to the bucket array.
const HASH_CONTROL_BYTE: usize = 1;

macro_rules! no_heap {
    ($($t:ty),* $(,)?) => {
        $(impl HeapSize for $t {
            #[inline]
            fn heap_size(&self) -> usize { 0 }
        })*
    };
}

no_heap!(
    (),
    bool,
    char,
    f32,
    f64,
    i8,
    i16,
    i32,
    i64,
    isize,
    u8,
    u16,
    u32,
    u64,
    usize,
    NonZeroU32,
    str,
    &str,
    Pos,
);

impl HeapSize for String {
    #[inline]
    fn heap_size(&self) -> usize {
        self.capacity()
    }
}

impl<T: HeapSize> HeapSize for [T] {
    #[inline]
    fn heap_size(&self) -> usize {
        self.iter().map(HeapSize::heap_size).sum()
    }
}

impl<T: HeapSize> HeapSize for Vec<T> {
    #[inline]
    fn heap_size(&self) -> usize {
        self.capacity() * size_of::<T>() + self.as_slice().heap_size()
    }
}

impl<T: HeapSize + ?Sized> HeapSize for Box<T> {
    #[inline]
    fn heap_size(&self) -> usize {
        size_of_val(&**self) + (**self).heap_size()
    }
}

/// Counted in full by every owner; shared subtrees over-charge rather than under-charge a budget.
impl<T: HeapSize + ?Sized> HeapSize for Arc<T> {
    #[inline]
    fn heap_size(&self) -> usize {
        REFCOUNT_HEADER + size_of_val(&**self) + (**self).heap_size()
    }
}

impl<T: HeapSize> HeapSize for Option<T> {
    #[inline]
    fn heap_size(&self) -> usize {
        self.as_ref().map_or(0, HeapSize::heap_size)
    }
}

impl<A: HeapSize, B: HeapSize> HeapSize for (A, B) {
    #[inline]
    fn heap_size(&self) -> usize {
        self.0.heap_size() + self.1.heap_size()
    }
}

impl<K: HeapSize, V: HeapSize> HeapSize for BTreeMap<K, V> {
    fn heap_size(&self) -> usize {
        btree_nodes(self.len()) * btree_node_size::<K, V>()
            + self
                .iter()
                .map(|(key, value)| key.heap_size() + value.heap_size())
                .sum::<usize>()
    }
}

/// A `BTreeSet<T>` is a `BTreeMap<T, ()>`, and a zero-sized value takes no room in the node.
impl<T: HeapSize> HeapSize for BTreeSet<T> {
    fn heap_size(&self) -> usize {
        btree_nodes(self.len()) * btree_node_size::<T, ()>()
            + self.iter().map(HeapSize::heap_size).sum::<usize>()
    }
}

impl<K: HeapSize, V: HeapSize, S> HeapSize for HashMap<K, V, S> {
    fn heap_size(&self) -> usize {
        self.capacity() * (size_of::<K>() + size_of::<V>() + HASH_CONTROL_BYTE)
            + self
                .iter()
                .map(|(key, value)| key.heap_size() + value.heap_size())
                .sum::<usize>()
    }
}

impl<T: HeapSize, S> HeapSize for HashSet<T, S> {
    fn heap_size(&self) -> usize {
        self.capacity() * (size_of::<T>() + HASH_CONTROL_BYTE)
            + self.iter().map(HeapSize::heap_size).sum::<usize>()
    }
}

impl HeapSize for query::Document<'_, String> {
    fn heap_size(&self) -> usize {
        self.definitions.heap_size()
    }
}

impl HeapSize for query::Definition<'_, String> {
    fn heap_size(&self) -> usize {
        match self {
            query::Definition::Operation(operation) => operation.heap_size(),
            query::Definition::Fragment(fragment) => fragment.heap_size(),
        }
    }
}

impl HeapSize for query::OperationDefinition<'_, String> {
    fn heap_size(&self) -> usize {
        match self {
            query::OperationDefinition::SelectionSet(set) => set.heap_size(),
            query::OperationDefinition::Query(query) => {
                query.name.heap_size()
                    + query.variable_definitions.heap_size()
                    + query.directives.heap_size()
                    + query.selection_set.heap_size()
            }
            query::OperationDefinition::Mutation(mutation) => {
                mutation.name.heap_size()
                    + mutation.variable_definitions.heap_size()
                    + mutation.directives.heap_size()
                    + mutation.selection_set.heap_size()
            }
            query::OperationDefinition::Subscription(subscription) => {
                subscription.name.heap_size()
                    + subscription.variable_definitions.heap_size()
                    + subscription.directives.heap_size()
                    + subscription.selection_set.heap_size()
            }
        }
    }
}

impl HeapSize for query::FragmentDefinition<'_, String> {
    fn heap_size(&self) -> usize {
        self.name.heap_size()
            + self.type_condition.heap_size()
            + self.directives.heap_size()
            + self.selection_set.heap_size()
    }
}

impl HeapSize for query::TypeCondition<'_, String> {
    fn heap_size(&self) -> usize {
        match self {
            query::TypeCondition::On(name) => name.heap_size(),
        }
    }
}

impl HeapSize for query::SelectionSet<'_, String> {
    fn heap_size(&self) -> usize {
        self.items.heap_size()
    }
}

impl HeapSize for query::Selection<'_, String> {
    fn heap_size(&self) -> usize {
        match self {
            query::Selection::Field(field) => {
                field.alias.heap_size()
                    + field.name.heap_size()
                    + field.arguments.heap_size()
                    + field.directives.heap_size()
                    + field.selection_set.heap_size()
            }
            query::Selection::FragmentSpread(spread) => {
                spread.fragment_name.heap_size() + spread.directives.heap_size()
            }
            query::Selection::InlineFragment(fragment) => {
                fragment.type_condition.heap_size()
                    + fragment.directives.heap_size()
                    + fragment.selection_set.heap_size()
            }
        }
    }
}

impl HeapSize for query::VariableDefinition<'_, String> {
    fn heap_size(&self) -> usize {
        self.name.heap_size() + self.var_type.heap_size() + self.default_value.heap_size()
    }
}

impl HeapSize for query::Directive<'_, String> {
    fn heap_size(&self) -> usize {
        self.name.heap_size() + self.arguments.heap_size()
    }
}

impl HeapSize for query::Type<'_, String> {
    fn heap_size(&self) -> usize {
        match self {
            query::Type::NamedType(name) => name.heap_size(),
            query::Type::ListType(inner) | query::Type::NonNullType(inner) => inner.heap_size(),
        }
    }
}

impl HeapSize for query::Value<'_, String> {
    fn heap_size(&self) -> usize {
        match self {
            query::Value::Variable(name) | query::Value::Enum(name) => name.heap_size(),
            query::Value::String(text) => text.heap_size(),
            query::Value::List(items) => items.heap_size(),
            query::Value::Object(fields) => fields.heap_size(),
            query::Value::Int(_)
            | query::Value::Float(_)
            | query::Value::Boolean(_)
            | query::Value::Null => 0,
        }
    }
}

impl HeapSize for ValidationError {
    fn heap_size(&self) -> usize {
        // `error_code` is a `&'static str`, so the message and the positions are all of it
        self.locations.heap_size() + self.message.heap_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_count_the_buffer_and_the_items_behind_it() {
        let mut strings = Vec::with_capacity(4);
        strings.push(String::from("hello"));

        assert_eq!(strings.heap_size(), 4 * size_of::<String>() + 5);
    }

    #[test]
    fn should_count_what_a_pointer_owns_but_not_the_pointer() {
        let boxed: Box<str> = "abcd".into();
        assert_eq!(boxed.heap_size(), 4);

        let shared: Arc<String> = Arc::new(String::from("abcd"));
        assert_eq!(
            shared.heap_size(),
            REFCOUNT_HEADER + size_of::<String>() + 4
        );

        assert_eq!(None::<String>.heap_size(), 0);
    }

    #[test]
    fn should_charge_a_btree_map_by_the_node() {
        // measured with a counting allocator: one entry allocates 384 bytes, of which 16 are
        // the key's own characters, so the node itself is 368 - all eleven slots, always
        let mut map: BTreeMap<String, u64> = BTreeMap::new();
        assert_eq!(map.heap_size(), 0, "an empty map has not allocated yet");

        let names = |map: &BTreeMap<String, u64>| map.keys().map(String::capacity).sum::<usize>();

        map.insert(String::from("argument0"), 0);
        assert_eq!(map.heap_size(), 368 + names(&map));

        for index in 1..11u64 {
            map.insert(format!("argument{index}"), index);
        }
        assert_eq!(
            map.heap_size(),
            368 + names(&map),
            "eleven entries still fit in the one node"
        );

        map.insert(String::from("argument11"), 11);
        assert_eq!(
            map.heap_size(),
            2 * 368 + names(&map),
            "the twelfth entry forces a second node"
        );
    }

    #[test]
    fn should_grow_with_the_parsed_document() {
        let parse = |query: &str| {
            graphql_tools::parser::query::parse_query::<String>(query)
                .expect("valid test query")
                .into_static()
                .heap_size()
        };

        let short_name = parse("{ a }");
        let long_name = parse("{ aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa }");

        assert!(
            long_name - short_name >= 32,
            "the extra 32 characters of the field name have to be paid for: \
             {long_name} vs {short_name}"
        );

        let one_field = parse("{ a }");
        let three_fields = parse("{ a b c }");

        assert!(
            three_fields > one_field,
            "more selections weigh more: {three_fields} vs {one_field}"
        );
    }
}
