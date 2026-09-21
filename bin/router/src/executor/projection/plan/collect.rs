use crate::executor::introspection::schema::{FieldNullability, SchemaMetadata};
use crate::executor::utils::consts::TYPENAME_FIELD_NAME;
use crate::query_planner::ast::{
    selection_item::SelectionItem,
    selection_set::{FieldSelection, SelectionSet},
};
use crate::telemetry::logging::targets;
use ahash::HashSet as AHashSet;
use bumpalo::collections::Vec as BumpVec;
use bumpalo::Bump;
use tracing::warn;

use super::ir::{Condition, Field, TypeSet, Value};

struct FieldInfo<'arena> {
    output_type: &'arena str,
    nullability: &'arena FieldNullability,
    is_typename: bool,
}

pub(super) struct Collector<'arena> {
    schema: &'arena SchemaMetadata,
    arena: &'arena Bump,
}

impl<'arena> Collector<'arena> {
    pub(super) fn collect_fields<'a>(
        selection_set: &'a SelectionSet,
        schema: &'a SchemaMetadata,
        parent_type: &'a str,
        parent_cond: Condition<'a>,
        arena: &'a Bump,
    ) -> BumpVec<'a, Field<'a>> {
        Collector { schema, arena }.collect_selection_set(selection_set, parent_type, parent_cond)
    }

    fn collect_selection_set(
        &self,
        selection_set: &'arena SelectionSet,
        parent_type: &'arena str,
        parent_cond: Condition<'arena>,
    ) -> BumpVec<'arena, Field<'arena>> {
        let mut collected = BumpVec::new_in(self.arena);
        for item in &selection_set.items {
            match item {
                SelectionItem::Field(field) => {
                    self.collect_field(field, parent_type, parent_cond, &mut collected)
                }
                SelectionItem::InlineFragment(fragment) => {
                    let fragment_scope = self.type_set(&fragment.type_condition);
                    let frag_cond = parent_cond
                        .and(Condition::ParentType(fragment_scope), self.arena)
                        .with_directives(
                            fragment.include_if.as_deref(),
                            fragment.skip_if.as_deref(),
                            self.arena,
                        );
                    let frag_fields = self.collect_selection_set(
                        &fragment.selections,
                        &fragment.type_condition,
                        frag_cond,
                    );
                    for mut frag_field in frag_fields {
                        frag_field.parent_scope = Some(fragment_scope);
                        collected.push(frag_field);
                    }
                }
                SelectionItem::FragmentSpread(_) => {
                    unreachable!("fragments are inlined before projection")
                }
            }
        }
        collected
    }

    fn collect_field(
        &self,
        field: &'arena FieldSelection,
        parent_type: &'arena str,
        parent_cond: Condition<'arena>,
        collected: &mut BumpVec<'arena, Field<'arena>>,
    ) {
        let Some(info) = self.field_info(field, parent_type) else {
            return;
        };

        let parent_scope = self.parent_type_set(&parent_cond);

        let mut output_types: Option<TypeSet<'arena>> = None;

        let abstract_outype = self.schema.is_union_type(info.output_type)
            || self.schema.is_interface_type(info.output_type);
        let mut condition = parent_cond;
        if abstract_outype {
            let output_types = *output_types.get_or_insert_with(|| self.type_set(info.output_type));
            condition = condition.and(Condition::FieldType(output_types), self.arena);
        }

        condition = condition.with_directives(
            field.include_if.as_deref(),
            field.skip_if.as_deref(),
            self.arena,
        );

        if let Some(allowed) = self.schema.enum_values.get(info.output_type) {
            condition = condition.and(
                Condition::EnumValues(TypeSet::from_schema(self.arena, allowed)),
                self.arena,
            );
        }
        let condition = condition.without_redundant_parent_scope(&parent_scope, self.arena);

        let value = if field.selections.items.is_empty() {
            Value::Passthrough
        } else if matches!(
            field.selections.items.as_slice(),
            [SelectionItem::Field(FieldSelection {
                omit_from_response: true,
                ..
            })]
        ) {
            Value::Children(BumpVec::new_in(self.arena))
        } else {
            // Child selections keep the parent's variable checks,
            // narrowed to the types this field can actually return.
            let inherited_scope = match parent_scope {
                Some(_) => Condition::ParentType(
                    *output_types.get_or_insert_with(|| self.type_set(info.output_type)),
                ),
                None => Condition::Always,
            };
            let children_cond = parent_cond
                .inherited_by_child(self.arena)
                .and(inherited_scope, self.arena)
                .with_directives(
                    field.include_if.as_deref(),
                    field.skip_if.as_deref(),
                    self.arena,
                );
            Value::Children(self.collect_selection_set(
                &field.selections,
                info.output_type,
                children_cond,
            ))
        };

        collected.push(Field {
            field_name: &field.name,
            response_key: field.alias.as_deref().unwrap_or(&field.name),
            is_typename: info.is_typename,
            nullability: info.nullability,
            parent_scope,
            condition,
            value,
        });
    }

    fn field_info(
        &self,
        field: &'arena FieldSelection,
        parent_type: &'arena str,
    ) -> Option<FieldInfo<'arena>> {
        let is_typename = field.name == TYPENAME_FIELD_NAME;
        if is_typename {
            return Some(FieldInfo {
                output_type: "String",
                nullability: self.arena.alloc(FieldNullability::type_name()),
                is_typename,
            });
        }

        let Some(fields) = self.schema.type_fields.get(parent_type) else {
            warn!(
                target: targets::EXECUTOR,
                parent_type = parent_type,
                "type is missing from schema metadata, skipping its fields in the projection plan"
            );
            return None;
        };

        let Some(info) = fields.get(&field.name) else {
            warn!(
                target: targets::EXECUTOR,
                parent_type = parent_type,
                field = field.name.as_str(),
                "field is missing from schema metadata, skipping it in the projection plan"
            );
            return None;
        };

        Some(FieldInfo {
            output_type: info.output_type_name.as_str(),
            nullability: &info.nullability,
            is_typename,
        })
    }

    fn type_set(&self, type_name: &'arena str) -> TypeSet<'arena> {
        if self.schema.is_object_type(type_name) || self.schema.is_scalar_type(type_name) {
            return TypeSet::exact(self.arena, type_name);
        }

        let possible = self.schema.get_possible_types(type_name);
        let mut names =
            BumpVec::with_capacity_in(possible.map_or(0, AHashSet::len) + 1, self.arena);
        if let Some(possible) = possible {
            names.extend(possible.iter().map(String::as_str));
        }
        names.push(type_name);
        TypeSet::sorted(names)
    }

    fn parent_type_set(&self, parent_cond: &Condition<'arena>) -> Option<TypeSet<'arena>> {
        use Condition::*;
        let combine = |left,
                       right,
                       join: fn(
            TypeSet<'arena>,
            TypeSet<'arena>,
            &'arena Bump,
        ) -> TypeSet<'arena>| match (left, right) {
            (Some(left), Some(right)) => Some(join(left, right, self.arena)),
            (left, right) => left.or(right),
        };
        match parent_cond {
            ParentType(fragment_scope) => Some(*fragment_scope),
            And(l, r) => combine(
                self.parent_type_set(l),
                self.parent_type_set(r),
                TypeSet::intersect,
            ),
            Or(l, r) => combine(
                self.parent_type_set(l),
                self.parent_type_set(r),
                TypeSet::union,
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::introspection::schema::SchemaWithMetadata;
    use crate::executor::projection::plan::ProjectionPlan;
    use crate::executor::projection::response::project_by_operation;
    use crate::executor::response::value::Value as JsonValue;
    use crate::query_planner::ast::normalization::normalize_operation;
    use crate::query_planner::utils::parsing::{parse_operation, parse_schema};
    use crate::query_planner::{planner::Planner, state::supergraph_state::SupergraphState};

    #[test]
    fn covariant_overrides_resolve_to_the_declared_parent_type() {
        let schema = parse_schema(
            r#"
            interface Node { id: ID!, item: Item }
            type A implements Node { id: ID!, item: ItemA }
            type B implements Node { id: ID!, item: ItemB }
            interface Item { id: ID! }
            type ItemA implements Item { id: ID! }
            type ItemB implements Item { id: ID! }
            type Query { node: Node }
            "#,
        );
        let planner = Planner::new_from_supergraph(&schema, Default::default()).unwrap();
        let schema = planner.consumer_schema.schema_metadata();
        for _ in 0..64 {
            let arena = Bump::new();
            let collector = Collector {
                schema: &schema,
                arena: &arena,
            };
            let node_types = collector.type_set("Node");
            assert!(node_types.len() > 1);
            let merger = super::super::merge::Merger::new(&schema, &arena);
            assert_eq!(
                merger.child_parent_type("Node", Some(node_types), "item"),
                Some("Item")
            );
            assert_eq!(
                merger.child_parent_type("Node", Some(node_types), "id"),
                Some("ID")
            );
        }
    }

    #[test]
    fn nested_unguarded_merge_resolves_under_the_field_output_type() {
        let schema = parse_schema(
            r#"
                interface Node { id: ID!, item: Item }
                type A implements Node { id: ID!, item: Item }
                type B implements Node { id: ID!, item: Item }
                type Item { id: ID!, sub: Sub }
                interface Sub { id: ID!, name: String }
                type X implements Sub { id: ID!, name: String }
                type Y implements Sub { id: ID!, name: String }
                type Query { node: Node }
                "#,
        );
        let supergraph = SupergraphState::new(&schema);
        let planner = Planner::new_from_supergraph(&schema, Default::default()).unwrap();
        let operation = parse_operation(
            r#"
              query Example {
                node {
                  item { sub { name } }
                  ... on A { item { sub { ... on X { name } } } }
                }
              }
              "#,
        );
        let normalized = normalize_operation(&supergraph, &operation, None).unwrap();
        let schema_metadata = planner.consumer_schema.schema_metadata();
        let (_, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);

        let project = |node_type: &'static str, sub_type: &'static str| {
            let data = JsonValue::Object(vec![(
                "node",
                JsonValue::Object(vec![
                    ("__typename", JsonValue::String(node_type.into())),
                    (
                        "item",
                        JsonValue::Object(vec![(
                            "sub",
                            JsonValue::Object(vec![
                                ("__typename", JsonValue::String(sub_type.into())),
                                ("name", JsonValue::String("sub-name".into())),
                            ]),
                        )]),
                    ),
                ]),
            )]);
            let output = project_by_operation(
                &data,
                vec![],
                &Default::default(),
                "Query",
                &plan,
                &None,
                256,
                &schema_metadata,
            )
            .unwrap();
            String::from_utf8(output).unwrap()
        };

        let expected = r#"{"data":{"node":{"item":{"sub":{"name":"sub-name"}}}}}"#;
        assert_eq!(project("A", "X"), expected);
        assert_eq!(project("A", "Y"), expected);
        assert_eq!(project("B", "X"), expected);
    }
}
