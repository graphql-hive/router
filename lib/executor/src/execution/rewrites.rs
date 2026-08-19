use std::collections::BTreeSet;

use hive_router_query_planner::planner::slot_path::{SlotPathSegment, SlotRewrite};

use crate::{introspection::schema::PossibleTypes, response::value::Value};

pub trait SlotRewriteExt {
    fn rewrite(&self, possible_types: &PossibleTypes, value: &mut Value<'_>);
}

impl SlotRewriteExt for SlotRewrite {
    fn rewrite(&self, possible_types: &PossibleTypes, value: &mut Value<'_>) {
        match self {
            // Renaming a key is a move between slots: the response tree carries values by
            // position, so there is no key left to rewrite.
            SlotRewrite::RenameSlot { path, from, to } => {
                walk(possible_types, value, path, &mut |target| {
                    let Some(moved) = target.slot_mut(*from).map(std::mem::take) else {
                        return;
                    };
                    if let Some(destination) = target.slot_mut(*to) {
                        *destination = moved;
                    }
                });
            }
            SlotRewrite::SetValue { path, value: new } => {
                walk(possible_types, value, path, &mut |target| {
                    *target = Value::String(new.as_str().to_owned().into());
                });
            }
        }
    }
}

fn entity_satisfies_any_type_condition(
    possible_types: &PossibleTypes,
    type_name: &str,
    conditions: &BTreeSet<String>,
) -> bool {
    conditions
        .iter()
        .any(|condition| possible_types.entity_satisfies_type_condition(type_name, condition))
}

/// Walks every position `path` addresses and hands each one to `apply`.
fn walk<'a, F>(
    possible_types: &PossibleTypes,
    value: &mut Value<'a>,
    path: &[SlotPathSegment],
    apply: &mut F,
) where
    F: FnMut(&mut Value<'a>),
{
    // Lists are transparent: every element sits at the same response position.
    if let Value::Array(items) = value {
        for item in items {
            walk(possible_types, item, path, apply);
        }
        return;
    }

    let Some((segment, remaining)) = path.split_first() else {
        apply(value);
        return;
    };

    match segment {
        SlotPathSegment::List => walk(possible_types, value, remaining, apply),
        SlotPathSegment::TypenameEquals {
            typename_slot,
            conditions,
        } => {
            // A position with no `__typename` cannot be excluded, so the gate passes — the
            // same outcome as the missing-key lookup this replaced.
            let type_name = typename_slot
                .and_then(|slot| value.slot(slot))
                .and_then(Value::as_str);
            if type_name.is_none_or(|type_name| {
                entity_satisfies_any_type_condition(possible_types, type_name, conditions)
            }) {
                walk(possible_types, value, remaining, apply);
            }
        }
        SlotPathSegment::Slot { slot, .. } => {
            if let Some(child) = value.slot_mut(*slot) {
                walk(possible_types, child, remaining, apply);
            }
        }
    }
}
