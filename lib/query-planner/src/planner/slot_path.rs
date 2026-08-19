//! Response paths compiled against the merged shape.
//!
//! Flatten paths, fetch rewrites and `requires` selections all address the response tree by
//! response key. Once the tree is slot-addressed those keys are gone from the data, so the
//! paths are resolved to slots once, at plan time, against the shape for each position.

use std::collections::BTreeSet;

use crate::{
    ast::{selection_item::SelectionItem, selection_set::SelectionSet},
    planner::{
        plan_nodes::{
            FetchNodePathSegment, FetchRewrite, FlattenNodePath, FlattenNodePathSegment,
        },
        response_shape::ResponseShape,
    },
};

pub const TYPENAME_FIELD_NAME: &str = "__typename";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotPathSegment {
    /// Descend into a field's value. `key` is carried only so GraphQL error paths can still
    /// name the field; nothing on the hot path reads it.
    Slot { slot: usize, key: String },
    /// Descend into each element of a list.
    List,
    /// Continue only when `__typename` at this position satisfies one of the conditions.
    /// `typename_slot` is `None` when the position carries no `__typename`, which the
    /// executor treats the same way a missing value was treated before: the gate passes.
    TypenameEquals {
        typename_slot: Option<usize>,
        conditions: BTreeSet<String>,
    },
}

/// A rewrite with every response key resolved to a slot.
#[derive(Debug, Clone)]
pub enum SlotRewrite {
    /// Move a value between two slots at the end of `path`. A key rename is a move once the
    /// key itself no longer exists in the data.
    RenameSlot {
        path: Vec<SlotPathSegment>,
        from: usize,
        to: usize,
    },
    /// Set a string at the end of `path`.
    SetValue {
        path: Vec<SlotPathSegment>,
        value: String,
    },
}

/// One step of a `requires` selection set, resolved to slots.
#[derive(Debug, Clone)]
pub enum RequiresStep {
    /// Emit `key` and the leaf value at `slot`.
    Leaf { key: String, slot: usize },
    /// Emit `key` and recurse into the value at `slot`.
    Enter {
        key: String,
        slot: usize,
        steps: Vec<RequiresStep>,
    },
    /// Recurse into the current position only when `__typename` satisfies `type_condition`.
    OnType {
        typename_slot: Option<usize>,
        type_condition: String,
        steps: Vec<RequiresStep>,
    },
}

pub fn compile_flatten_path(path: &FlattenNodePath, shape: &ResponseShape) -> Vec<SlotPathSegment> {
    let mut compiled = Vec::with_capacity(path.as_slice().len());
    let mut node = Some(shape);

    for segment in path.as_slice() {
        match segment {
            FlattenNodePathSegment::Field(key) => {
                let slot = node.and_then(|shape| shape.slot_of(key));
                // A path the shape does not describe cannot address anything in the data.
                // Emitting a slot past the end makes the traversal find nothing, which is
                // the same outcome the key lookup had.
                let slot = slot.unwrap_or(usize::MAX);
                compiled.push(SlotPathSegment::Slot {
                    slot,
                    key: key.clone(),
                });
                node = node.and_then(|shape| shape.child(slot));
            }
            FlattenNodePathSegment::List => compiled.push(SlotPathSegment::List),
            FlattenNodePathSegment::TypeCondition(conditions) => {
                compiled.push(SlotPathSegment::TypenameEquals {
                    typename_slot: node.and_then(|shape| shape.slot_of(TYPENAME_FIELD_NAME)),
                    conditions: conditions.clone(),
                });
            }
        }
    }

    compiled
}

pub fn compile_rewrites(rewrites: &[FetchRewrite], shape: &ResponseShape) -> Vec<SlotRewrite> {
    rewrites
        .iter()
        .filter_map(|rewrite| compile_rewrite(rewrite, shape))
        .collect()
}

fn compile_rewrite(rewrite: &FetchRewrite, shape: &ResponseShape) -> Option<SlotRewrite> {
    match rewrite {
        FetchRewrite::ValueSetter(setter) => {
            let (path, _) = compile_fetch_path(&setter.path, shape)?;
            Some(SlotRewrite::SetValue {
                path,
                value: setter.set_value_to.clone(),
            })
        }
        FetchRewrite::KeyRenamer(renamer) => {
            // The last segment names the field being renamed, so the move happens in its
            // parent's shape.
            let (parent_path, last) = renamer.path.split_at(renamer.path.len().checked_sub(1)?);
            let (path, parent_shape) = compile_fetch_path(parent_path, shape)?;
            let FetchNodePathSegment::Key(from_key) = last.first()? else {
                return None;
            };
            Some(SlotRewrite::RenameSlot {
                path,
                from: parent_shape.slot_of(from_key)?,
                to: parent_shape.slot_of(&renamer.rename_key_to)?,
            })
        }
    }
}

fn compile_fetch_path<'a>(
    path: &[FetchNodePathSegment],
    shape: &'a ResponseShape,
) -> Option<(Vec<SlotPathSegment>, &'a ResponseShape)> {
    let mut compiled = Vec::with_capacity(path.len());
    let mut node = shape;

    for segment in path {
        match segment {
            FetchNodePathSegment::Key(key) => {
                let slot = node.slot_of(key)?;
                compiled.push(SlotPathSegment::Slot {
                    slot,
                    key: key.clone(),
                });
                node = node.child(slot)?;
            }
            FetchNodePathSegment::TypenameEquals(conditions) => {
                compiled.push(SlotPathSegment::TypenameEquals {
                    typename_slot: node.slot_of(TYPENAME_FIELD_NAME),
                    conditions: conditions.clone(),
                });
            }
        }
    }

    Some((compiled, node))
}

/// Compiles a `requires` selection set against the shape of the position it reads from.
pub fn compile_requires(selections: &SelectionSet, shape: &ResponseShape) -> Vec<RequiresStep> {
    let mut steps = Vec::with_capacity(selections.items.len());

    for item in &selections.items {
        match item {
            SelectionItem::Field(field) => {
                let key = field.name.clone();
                // `requires` names schema fields, but a fetch may have aliased them, so fall
                // back to the response key.
                let Some(slot) = shape
                    .slot_of(&key)
                    .or_else(|| shape.slot_of(field.selection_identifier()))
                else {
                    continue;
                };

                if field.selections.is_empty() {
                    steps.push(RequiresStep::Leaf { key, slot });
                } else if let Some(child) = shape.child(slot) {
                    steps.push(RequiresStep::Enter {
                        key,
                        slot,
                        steps: compile_requires(&field.selections, child),
                    });
                }
            }
            SelectionItem::InlineFragment(fragment) => {
                steps.push(RequiresStep::OnType {
                    typename_slot: shape.slot_of(TYPENAME_FIELD_NAME),
                    type_condition: fragment.type_condition.clone(),
                    steps: compile_requires(&fragment.selections, shape),
                });
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }

    steps
}

/// Renders a compiled path the way the response path used to read, for error messages and
/// diagnostics. Cold path only.
pub fn slot_path_to_string(path: &[SlotPathSegment]) -> String {
    let mut out = String::new();
    for segment in path {
        match segment {
            SlotPathSegment::Slot { key, .. } => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            SlotPathSegment::List => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push('@');
            }
            SlotPathSegment::TypenameEquals { conditions, .. } => {
                out.push_str("|[");
                out.push_str(&conditions.iter().cloned().collect::<Vec<_>>().join("|"));
                out.push(']');
            }
        }
    }
    out
}
