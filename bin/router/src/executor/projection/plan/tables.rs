use std::collections::HashMap;

use crate::executor::introspection::schema::FieldNullability;
use crate::executor::projection::plan::{ConditionId, GuardId};

use super::ir::TypeSet;
use super::{Condition, Guard, Range, SetId, ShapeFlags, ShapeId, SymbolId};

/// Table indexes are `u32`. You would need billions of fields to overflow one,
/// so this just panics instead of every encoder function returning a `Result`.
pub(super) fn id(value: usize) -> u32 {
    u32::try_from(value).expect("projection plan table index fits in u32")
}

/// Appends `value` unless the table already holds it, returning its index.
fn intern<T: PartialEq>(table: &mut Vec<T>, value: T) -> usize {
    match table.iter().position(|existing| *existing == value) {
        Some(index) => index,
        None => {
            table.push(value);
            table.len() - 1
        }
    }
}

fn intern_slice<T: PartialEq + Copy>(
    ranges: &mut Vec<Range>,
    pool: &mut Vec<T>,
    value: &[T],
) -> usize {
    if let Some(index) = ranges.iter().position(|range| range.slice(pool) == value) {
        return index;
    }
    ranges.push(Range {
        start: id(pool.len()),
        len: id(value.len()),
    });
    pool.extend_from_slice(value);
    ranges.len() - 1
}

#[derive(Default)]
pub(super) struct TablesBuilder {
    pub(super) text: String,
    pub(super) symbols: Vec<Range>,
    symbol_keys: HashMap<String, SymbolId>,
    pub(super) conditions: Vec<Condition>,
    pub(super) guards: Vec<Guard>,
    pub(super) sets: Vec<Range>,
    pub(super) set_members: Vec<SymbolId>,
    pub(super) shapes: Vec<Range>,
    pub(super) shape_bytes: Vec<u8>,
}

impl TablesBuilder {
    pub(super) fn symbol_range(&self, id: SymbolId) -> Range {
        self.symbols[id.0 as usize]
    }

    pub(super) fn intern_symbol(&mut self, text: &str) -> SymbolId {
        if let Some(id) = self.symbol_keys.get(text) {
            return *id;
        }

        let range = Range {
            start: id(self.text.len()),
            len: id(text.len()),
        };
        self.text.push_str(text);

        let symbol = SymbolId(id(self.symbols.len()));

        self.symbols.push(range);
        self.symbol_keys.insert(text.to_string(), symbol);
        symbol
    }

    pub(super) fn intern_members(&mut self, members: TypeSet<'_>) -> SetId {
        debug_assert!(
            members.windows(2).all(|pair| pair[0] < pair[1]),
            "intern_members expects members in strictly increasing text order"
        );

        let symbols: Vec<SymbolId> = members
            .iter()
            .map(|member| self.intern_symbol(member))
            .collect();

        SetId(id(intern_slice(
            &mut self.sets,
            &mut self.set_members,
            &symbols,
        )))
    }

    pub(super) fn intern_guard(&mut self, types: TypeSet<'_>) -> GuardId {
        let guard = match types.as_ref() {
            [only] => Guard::Exact(self.intern_symbol(only)),
            _ => Guard::Set(self.intern_members(types)),
        };
        GuardId::from_index(intern(&mut self.guards, guard))
    }

    pub(super) fn intern_condition(&mut self, condition: Condition) -> ConditionId {
        ConditionId::from_index(intern(&mut self.conditions, condition))
    }

    pub(super) fn intern_shape(&mut self, nullability: &FieldNullability) -> ShapeId {
        let mut bytes = Vec::new();
        let mut current = nullability;

        loop {
            let (is_list, non_null, next) = match current {
                FieldNullability::Leaf { non_null } => (false, *non_null, None),
                FieldNullability::List { non_null, item } => (true, *non_null, Some(item.as_ref())),
            };
            let mut flags = ShapeFlags::empty();
            flags.set(ShapeFlags::LIST, is_list);
            flags.set(ShapeFlags::NON_NULL, non_null);
            bytes.push(flags.bits());
            let Some(next) = next else { break };
            current = next;
        }

        let index = intern_slice(&mut self.shapes, &mut self.shape_bytes, &bytes);
        let shape = id(index);
        // `FieldMeta` only spares 28 bits for this, so a bigger id would land
        // on the flag bits and quietly corrupt them.
        assert!(
            shape <= super::FieldMeta::SHAPE_ID_MASK,
            "too many distinct nullability shapes for a projection plan"
        );
        ShapeId(shape)
    }
}
