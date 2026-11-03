use graphql_tools::parser::query::{Field, InlineFragment, Selection, SelectionSet, TypeCondition};

use crate::query_planner::{
    ast::normalization::{context::NormalizationContext, error::NormalizationError},
    state::supergraph_state::{SupergraphDefinition, SupergraphObjectType, SupergraphState},
};

/// A selection set may target an interface type.
/// However, not all implementing object types may resolve all interface fields
/// (some fields may be marked as external).
/// Type expansion is the process of rewriting a selection on an
/// interface type into multiple inline fragments, each targeting a concrete object type that
/// implements the interface type.
/// With type expansion, the Query Planner will have to find at least
/// one resolvable query path to each field.
/// Instead of looking for Interface.Field edge,
/// the Query Planner will look for Object.Field.
#[inline]
pub fn type_expand(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let expander = Expander {
        state: ctx.supergraph,
        subgraph_name: ctx.subgraph_name,
    };
    for operation in ctx.operations_mut() {
        let (root, selection_set) = operation?;
        expander.handle_selection_set(root, selection_set)?;
    }

    Ok(())
}

struct Expander<'a> {
    state: &'a SupergraphState,
    subgraph_name: Option<&'a str>,
}

impl<'a> Expander<'a> {
    fn definition(&self, type_name: &str) -> Result<&'a SupergraphDefinition, NormalizationError> {
        self.state.definitions.get(type_name).ok_or_else(|| {
            NormalizationError::SchemaTypeNotFound {
                type_name: type_name.to_string(),
            }
        })
    }

    /// The object types implementing `type_def`, sorted by name for a deterministic fragment
    /// order, when it's an interface.
    /// The subgraph has to know the interface, but we expand to every object implementing it,
    /// not only the ones that subgraph declares.
    fn possible_object_types(
        &self,
        type_def: &SupergraphDefinition,
    ) -> Result<Option<Vec<&'a SupergraphObjectType>>, NormalizationError> {
        let SupergraphDefinition::Interface(interface_type) = type_def else {
            return Ok(None);
        };
        let interface_name = interface_type.name.as_str();
        let is_known = self.subgraph_name.is_none_or(|graph_id| {
            self.state
                .possible_types_in_subgraph(interface_name, graph_id)
                .is_some()
        });
        let object_names = self
            .state
            .all_possible_types(interface_name)
            .filter(|_| is_known)
            .ok_or_else(|| NormalizationError::PossibleTypesNotFound {
                type_name: interface_type.name.clone(),
            })?;
        let mut objects = Vec::with_capacity(object_names.len());
        for object_name in object_names {
            let Some(SupergraphDefinition::Object(object)) =
                self.state.definitions.get(object_name)
            else {
                return Err(NormalizationError::SchemaTypeNotFound {
                    type_name: object_name.clone(),
                });
            };
            objects.push(object);
        }
        objects.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Ok(Some(objects))
    }

    #[inline]
    fn handle_selection_set(
        &self,
        type_def: &SupergraphDefinition,
        selection_set: &mut SelectionSet<String>,
    ) -> Result<(), NormalizationError> {
        let old_items = std::mem::take(&mut selection_set.items);
        let mut new_items = Vec::with_capacity(old_items.len());

        // Only perform type expansion if the current type is an interface.
        let possible_object_types = self.possible_object_types(type_def)?;

        for selection in old_items {
            match selection {
                Selection::Field(field) => {
                    // Don't try to look up introspection fields in the schema
                    // Just keep the selection set as-is
                    if field.name.starts_with("__") {
                        new_items.push(Selection::Field(field));
                        continue;
                    }

                    let field_def = type_def.fields().get(&field.name).ok_or_else(|| {
                        NormalizationError::FieldNotFoundInType {
                            field_name: field.name.clone(),
                            type_name: type_def.name().to_string(),
                        }
                    })?;

                    let mut field = match &possible_object_types {
                        Some(possible_object_types) => {
                            match self.handle_type_expansion_candidate(
                                possible_object_types,
                                type_def,
                                field,
                                &mut new_items,
                            )? {
                                Some(field) => field,
                                None => continue,
                            }
                        }
                        None => field,
                    };

                    if !field.selection_set.items.is_empty() {
                        let inner_type_def = self.definition(field_def.field_type.inner_type())?;
                        self.handle_selection_set(inner_type_def, &mut field.selection_set)?;
                    }
                    new_items.push(Selection::Field(field));
                }
                Selection::InlineFragment(mut frag) => {
                    if let Some(type_cond) = &frag.type_condition {
                        let TypeCondition::On(type_name) = type_cond;
                        if let Some(type_def) = self.state.definitions.get(type_name) {
                            self.handle_selection_set(type_def, &mut frag.selection_set)?;
                        }
                    } else {
                        self.handle_selection_set(type_def, &mut frag.selection_set)?;
                    }
                    new_items.push(Selection::InlineFragment(frag));
                }
                other => {
                    new_items.push(other);
                }
            }
        }

        selection_set.items = new_items;
        Ok(())
    }

    /// Expands `field` into a fragment per object type when it has to be, and gives it back
    /// when it doesn't.
    #[inline]
    fn handle_type_expansion_candidate<'f>(
        &self,
        possible_object_types: &[&SupergraphObjectType],
        type_def: &SupergraphDefinition,
        field: Field<'f, String>,
        new_items: &mut Vec<Selection<'f, String>>,
    ) -> Result<Option<Field<'f, String>>, NormalizationError> {
        let SupergraphDefinition::Interface(interface_type) = type_def else {
            return Ok(Some(field));
        };

        let should_expand = possible_object_types.iter().any(|obj| {
            // Expand if any object type implementing the interface:
            // 1. Does not have the field.
            // 2. Has the field, but it's marked as external or is overridden.
            match obj.fields.get(&field.name) {
                None => true,
                Some(obj_field) => obj_field.join_field.iter().any(|jf| {
                    jf.external
                        || jf.used_overridden
                        || jf
                            .override_value
                            .as_ref()
                            .is_some_and(|name| self.state.subgraph_exists_by_name(name))
                }),
            }
        });

        if !should_expand {
            // An interface may be defined in multiple subgraphs.
            // If a field on that interface is not resolvable in all of those subgraphs,
            // we must expand to concrete types to ensure we can find a resolvable query path.
            let subgraphs_with_interface = interface_type
                .join_type
                .iter()
                // if one of the interfaces is @interfaceObject,
                // ignore it
                .filter(|jt| !jt.is_interface_object)
                .count();

            if subgraphs_with_interface <= 1 {
                // No need to expand if interface is in at most one subgraph.
                return Ok(Some(field));
            }

            let Some(field_def) = interface_type.fields.get(&field.name) else {
                return Ok(Some(field));
            };
            if field_def.join_field.is_empty() {
                // The field is available everywhere the interface is defined if there's no join info
                return Ok(Some(field));
            }

            // Check if the field is contributed by the interface object,
            // which means it's available in all subgraphs that define the interface.
            let interface_object_in_graphs: Vec<_> = interface_type
                .join_type
                .iter()
                .filter(|jt| jt.is_interface_object)
                .map(|jt| &jt.graph_id)
                .collect();

            let is_interface_object_field =
                field_def.join_field.iter().all(|j| j.graph_id.is_none())
                    || field_def.join_field.iter().any(|jf| {
                        !jf.external
                            && jf
                                .graph_id
                                .as_ref()
                                .is_some_and(|g| interface_object_in_graphs.contains(&g))
                    });

            if is_interface_object_field {
                return Ok(Some(field));
            }

            // All subgraphs where this field is resolvable
            let subgraphs_resolving_field = field_def
                .join_field
                .iter()
                .filter(|jf| !jf.external && jf.graph_id.is_some())
                .count();

            if subgraphs_resolving_field >= subgraphs_with_interface {
                // All subgraphs are resolving the field
                return Ok(Some(field));
            }
        }

        // The last object takes the field, only the ones before it get copies.
        let directives = field.directives.clone();
        let mut field = Some(field);
        for (index, obj) in possible_object_types.iter().enumerate() {
            let mut new_field = if index + 1 == possible_object_types.len() {
                field.take().expect("the last object takes the field")
            } else {
                field
                    .clone()
                    .expect("the field is there until the last object")
            };
            if !new_field.selection_set.items.is_empty() {
                if let Some(obj_field) = obj.fields.get(&new_field.name) {
                    let inner_type_def = self.definition(obj_field.field_type.inner_type())?;
                    self.handle_selection_set(inner_type_def, &mut new_field.selection_set)?;
                }
            }
            new_items.push(Selection::InlineFragment(InlineFragment {
                type_condition: Some(TypeCondition::On(obj.name.clone())),
                directives: directives.clone(),
                selection_set: SelectionSet {
                    span: Default::default(),
                    items: vec![Selection::Field(new_field)],
                },
                position: Default::default(),
            }));
        }

        Ok(None)
    }
}
