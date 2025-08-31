use hive_router_query_planner::ast::{
    operation::OperationDefinition,
    selection_item::SelectionItem,
    selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
};

/// Represents an operation that has been partitioned into a part for subgraphs
/// and an optional part for introspection (router-level).
pub struct PartitionedOperation {
    /// Resolved by the subgraphs
    pub downstream_operation: OperationDefinition,
    /// Resolved by the router
    pub introspection_operation: Option<OperationDefinition>,
}

/// Partitions an operation into a part for subgraphs and an optional part for introspection (router-level)
pub fn partition_operation(mut op: OperationDefinition) -> PartitionedOperation {
    let selection_set = std::mem::take(&mut op.selection_set);
    let (downstream_selection_set, introspection_selection_set) =
        partition_selection_set(selection_set, SelectionSetLevel::Root);

    let introspection_operation = if introspection_selection_set.is_empty() {
        None
    } else {
        let used_variables = introspection_selection_set.variable_usages();
        let introspection_variable_definitions = op.variable_definitions.as_ref().map(|defs| {
            defs.iter()
                .filter(|def| used_variables.contains(&def.name))
                .cloned()
                .collect()
        });

        Some(OperationDefinition {
            name: op.name.clone(),
            operation_kind: op.operation_kind.clone(),
            selection_set: introspection_selection_set,
            variable_definitions: introspection_variable_definitions,
        })
    };

    let downstream_operation = OperationDefinition {
        name: op.name,
        operation_kind: op.operation_kind,
        selection_set: downstream_selection_set,
        variable_definitions: op.variable_definitions,
    };

    PartitionedOperation {
        downstream_operation,
        introspection_operation,
    }
}

#[derive(Clone)]
enum SelectionSetLevel {
    Root,
    Nested,
}

impl SelectionSetLevel {
    fn is_root(&self) -> bool {
        matches!(self, SelectionSetLevel::Root)
    }
}

fn partition_selection_set(
    selection_set: SelectionSet,
    level: SelectionSetLevel,
) -> (SelectionSet, SelectionSet) {
    let mut downstream_items = vec![];
    let mut introspection_items = vec![];

    for item in selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                // pass root level __typename to introspection
                if (level.is_root() && field.name.starts_with("__"))
                    ||
                    // do NOT pass non-root level __typename to introspection
                    field.name.starts_with("__") && field.name != "__typename"
                {
                    introspection_items.push(SelectionItem::Field(field));
                } else {
                    let is_leaf = field.is_leaf();
                    let FieldSelection {
                        name,
                        alias,
                        arguments,
                        selections,
                        skip_if,
                        include_if,
                    } = field;

                    let (downstream_selections, introspection_selections) =
                        partition_selection_set(selections, SelectionSetLevel::Nested);

                    let has_downstream = !downstream_selections.is_empty();
                    let has_introspection = !introspection_selections.is_empty();

                    let need_down = is_leaf || has_downstream;
                    let need_intro = has_introspection;

                    let mut name = Some(name);
                    let mut alias = alias;
                    let mut arguments = arguments;
                    let mut skip_if = skip_if;
                    let mut include_if = include_if;

                    if need_down {
                        downstream_items.push(SelectionItem::Field(FieldSelection {
                            name: take_or_clone(&mut name, need_intro).expect("Name is required"),
                            alias: take_or_clone(&mut alias, need_intro),
                            arguments: take_or_clone(&mut arguments, need_intro),
                            skip_if: take_or_clone(&mut skip_if, need_intro),
                            include_if: take_or_clone(&mut include_if, need_intro),
                            selections: downstream_selections,
                        }));
                    }

                    if need_intro {
                        introspection_items.push(SelectionItem::Field(FieldSelection {
                            name: name.take().expect("Name is required"),
                            alias: alias.take(),
                            arguments: arguments.take(),
                            skip_if: skip_if.take(),
                            include_if: include_if.take(),
                            selections: introspection_selections,
                        }));
                    }
                }
            }
            SelectionItem::InlineFragment(inline_fragment) => {
                let mut type_condition = Some(inline_fragment.type_condition);
                let mut skip_if = inline_fragment.skip_if;
                let mut include_if = inline_fragment.include_if;
                let (downstream_selections, introspection_selections) =
                    partition_selection_set(inline_fragment.selections, level.clone());

                let has_downstream = !downstream_selections.is_empty();
                let has_introspection = !introspection_selections.is_empty();

                let need_down = has_downstream;
                let need_intro = has_introspection;

                if need_down {
                    let downstream_frag = InlineFragmentSelection {
                        type_condition: take_or_clone(&mut type_condition, need_intro)
                            .expect("type_condition is required"),
                        skip_if: take_or_clone(&mut skip_if, need_intro),
                        include_if: take_or_clone(&mut include_if, need_intro),
                        selections: downstream_selections,
                    };
                    downstream_items.push(SelectionItem::InlineFragment(downstream_frag));
                }

                if need_intro {
                    let introspection_frag = InlineFragmentSelection {
                        type_condition: type_condition.take().expect("type_condition is required"),
                        skip_if: skip_if.take(),
                        include_if: include_if.take(),
                        selections: introspection_selections,
                    };
                    introspection_items.push(SelectionItem::InlineFragment(introspection_frag));
                }
            }
            SelectionItem::FragmentSpread(name) => {
                downstream_items.push(SelectionItem::FragmentSpread(name));
            }
        }
    }

    (
        SelectionSet {
            items: downstream_items,
        },
        SelectionSet {
            items: introspection_items,
        },
    )
}

fn take_or_clone<T: Clone>(slot: &mut Option<T>, need_clone: bool) -> Option<T> {
    if need_clone {
        slot.clone()
    } else {
        slot.take()
    }
}
