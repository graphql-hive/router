//! Walks the response data in a `Value` tree for plugins (see `ResponseData::walk`): the
//! projector's walk, field for field with the same conditions and type guards, visiting values
//! instead of writing them. A plugin changes the tree as it goes; the projection bubbles the
//! nulls later.

use std::cell::OnceCell;
use std::collections::HashMap;

use super::error::ProjectionError;
use super::plan::{FieldRecord, ProjectionPlan, Range};
use super::response::{evaluate, resolve_type_name, ConditionFailure, ShapeCursor, TypeName};
use graphql_tools::parser::query as gql;

use crate::executor::indexed::compile::{collect_fields, single_type_condition};
use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::plugins::response_data::{
    DataValue, Nullability, PathFrame, Place, Step, WalkResult,
};
use crate::executor::response::graphql_error::GraphQLError;
use crate::executor::response::value::Value;
use crate::executor::utils::consts::TYPENAME_FIELD_NAME;

pub(crate) fn walk<F: FnMut(&mut DataValue<'_, '_>)>(
    data: &mut Value<'_>,
    plan: &ProjectionPlan,
    root_type: &str,
    schema: &SchemaMetadata,
    variables: &Option<HashMap<String, sonic_rs::Value>>,
    errors: &mut Vec<GraphQLError>,
    visit: F,
) -> WalkResult {
    let Value::Object(object) = data else {
        return Ok(());
    };
    let mut walker = Walker {
        plan,
        schema,
        variables,
        errors,
        visit,
    };
    walker.object(object, plan.root_range(), root_type, None)
}

struct Walker<'w, F> {
    plan: &'w ProjectionPlan,
    schema: &'w SchemaMetadata,
    variables: &'w Option<HashMap<String, sonic_rs::Value>>,
    errors: &'w mut Vec<GraphQLError>,
    visit: F,
}

impl<'w, F: FnMut(&mut DataValue<'_, '_>)> Walker<'w, F> {
    fn object<'v>(
        &mut self,
        object: &mut [(&'v str, Value<'v>)],
        range: Range,
        parent_type: &'w str,
        path: Option<&PathFrame<'_>>,
    ) -> WalkResult {
        let plan = self.plan;
        let mut cursor = 0;
        for field in plan.fields(range) {
            if field.is_typename() || field.is_null_value() {
                continue;
            }
            if let Some(guard) = field.parent_guard {
                if !plan.guard_matches(guard, parent_type) {
                    continue;
                }
            }
            let key = plan.response_key(field);
            // not in the object: the projection writes null, there's nothing to change
            let Some(position) = Value::object_position_from(object, key, &mut cursor) else {
                continue;
            };
            if let Some(condition) = field.condition {
                let value = &object[position].1;
                let parent = TypeName::resolved(parent_type);
                let field_type = OnceCell::new();
                let parent_type_fn = || parent.get();
                let field_type_fn = || {
                    field_type
                        .get_or_init(|| {
                            resolve_type_name(field, Some(value), &parent, plan, self.schema)
                        })
                        .clone()
                };
                match evaluate(
                    condition,
                    plan,
                    &parent_type_fn,
                    &field_type_fn,
                    Some(value),
                    self.variables,
                ) {
                    // an enum value that isn't one is visited: a plugin may set a valid one, the
                    // projection writes null with an error otherwise
                    Ok(()) | Err(ConditionFailure::InvalidEnumValue) => {}
                    Err(ConditionFailure::Fatal(error)) => return Err(error),
                    // left out, or written as null by the projection
                    Err(_) => continue,
                }
            }
            let frame = PathFrame {
                parent: path,
                step: Step::Key(key),
            };
            let nullability = Nullability::Plan {
                non_null: field.is_non_null(),
                shape: ShapeCursor::new(plan.shape(field.nullability())),
            };
            self.value(
                &mut object[position].1,
                field,
                parent_type,
                nullability,
                None,
                &frame,
            )?;
        }
        Ok(())
    }

    /// Visits `value`, then what's under it
    fn value(
        &mut self,
        value: &mut Value<'_>,
        field: &'w FieldRecord,
        parent_type: &'w str,
        nullability: Nullability<'w>,
        list_index: Option<u32>,
        path: &PathFrame<'_>,
    ) -> WalkResult {
        let plan = self.plan;
        let mut node = DataValue {
            schema: self.schema,
            field_name: plan.field_name(field),
            response_key: plan.response_key(field),
            parent_type,
            composite: field.has_children(),
            nullability,
            item: list_index,
            path,
            place: Place::Value(&mut *value),
            errors: &mut *self.errors,
            skip: false,
        };
        (self.visit)(&mut node);
        if node.skip {
            return Ok(());
        }
        match value {
            Value::Array(items) => {
                let item_nullability = nullability.item();
                for (i, item) in items.iter_mut().enumerate() {
                    let frame = PathFrame {
                        parent: Some(path),
                        step: Step::Index(i as u32),
                    };
                    self.value(
                        item,
                        field,
                        parent_type,
                        item_nullability,
                        Some(i as u32),
                        &frame,
                    )?;
                }
            }
            Value::Object(object) if field.has_children() => {
                let type_name = self.object_type(field, object, parent_type)?;
                self.object(object, field.children, type_name, Some(path))?;
            }
            _ => {}
        }
        Ok(())
    }

    /// The type of an object: its `__typename` when it has one, or the field's type, like the
    /// projection finds it
    fn object_type(
        &self,
        field: &'w FieldRecord,
        object: &[(&str, Value<'_>)],
        parent_type: &'w str,
    ) -> Result<&'w str, ProjectionError> {
        let schema = self.schema;
        let typename = Value::object_get(object, TYPENAME_FIELD_NAME).and_then(Value::as_str);
        // the schema's copy of the name, the walk changes the tree it's read from
        if let Some(name) = typename.and_then(|name| schema.object_types.get(name)) {
            return Ok(name);
        }
        let plan = self.plan;
        schema
            .get_type_fields(parent_type)
            .and_then(|fields| fields.get(plan.field_name(field)))
            .map(|field| field.output_type_name.as_str())
            .ok_or_else(|| ProjectionError::MissingField {
                field_name: plan.field_name(field).to_string(),
                type_name: parent_type.to_string(),
            })
    }
}

/// Walks one fetch's data in a `Value` tree, before it's merged, field for field the way the
/// fetch's query selects it (see `SubgraphData::walk`). Field names come from the query, types
/// from the objects' `__typename` and the schema.
// ponytail: parses the fetch's document on each walk, Value pipeline only; keep a parsed copy
// on the fetch if it ever shows in a profile
pub(crate) fn walk_fetch<F: FnMut(&mut DataValue<'_, '_>)>(
    data: &mut Value<'_>,
    document: &str,
    root_type: &str,
    schema: &SchemaMetadata,
    errors: &mut Vec<GraphQLError>,
    visit: F,
) -> WalkResult {
    let Ok(parsed) = gql::parse_query::<&str>(document) else {
        return Ok(());
    };
    let mut operation = None;
    let mut fragments = Vec::new();
    for definition in &parsed.definitions {
        match definition {
            gql::Definition::Operation(op) => {
                operation = Some(match op {
                    gql::OperationDefinition::Query(q) => &q.selection_set,
                    gql::OperationDefinition::Mutation(m) => &m.selection_set,
                    gql::OperationDefinition::SelectionSet(s) => s,
                    gql::OperationDefinition::Subscription(s) => &s.selection_set,
                })
            }
            gql::Definition::Fragment(fragment) => fragments.push(fragment),
        }
    }
    let (Some(operation), Value::Object(object)) = (operation, data) else {
        return Ok(());
    };
    let mut walker = FetchWalker {
        fragments: &fragments,
        schema,
        errors,
        visit,
    };
    walker.object(object, &[operation], root_type, None);
    Ok(())
}

type Set<'d, 'a> = gql::SelectionSet<'a, &'a str>;

struct FetchWalker<'d, 'a, 's, F> {
    fragments: &'d [&'d gql::FragmentDefinition<'a, &'a str>],
    schema: &'s SchemaMetadata,
    errors: &'s mut Vec<GraphQLError>,
    visit: F,
}

impl<'d, 'a, 's, F: FnMut(&mut DataValue<'_, '_>)> FetchWalker<'d, 'a, 's, F> {
    fn object(
        &mut self,
        object: &mut [(&str, Value<'_>)],
        sets: &[&'d Set<'d, 'a>],
        parent_type: &str,
        path: Option<&PathFrame<'_>>,
    ) {
        let mut fields = Vec::new();
        for set in sets {
            // a fragment the document doesn't define: what's before it is still walked
            let _ = collect_fields(self.fragments, set, &mut fields);
        }
        let mut cursor = 0;
        for (key, name, child_sets) in &fields {
            // `__typename` can't change; a key that's a different field per type can't be typed
            let Some(name) = name.filter(|name| *name != TYPENAME_FIELD_NAME) else {
                continue;
            };
            let Some(position) = Value::object_position_from(object, key, &mut cursor) else {
                continue;
            };
            let frame = PathFrame {
                parent: path,
                step: Step::Key(key),
            };
            self.value(
                &mut object[position].1,
                name,
                key,
                child_sets,
                parent_type,
                Nullability::Schema { depth: 0 },
                None,
                &frame,
            );
        }
    }

    /// Visits `value`, then what's under it
    #[allow(clippy::too_many_arguments)]
    fn value(
        &mut self,
        value: &mut Value<'_>,
        name: &'a str,
        key: &'a str,
        sets: &[&'d Set<'d, 'a>],
        parent_type: &str,
        nullability: Nullability<'_>,
        list_index: Option<u32>,
        path: &PathFrame<'_>,
    ) {
        let mut node = DataValue {
            schema: self.schema,
            field_name: name,
            response_key: key,
            parent_type,
            composite: !sets.is_empty(),
            nullability,
            item: list_index,
            path,
            place: Place::Value(&mut *value),
            errors: &mut *self.errors,
            skip: false,
        };
        (self.visit)(&mut node);
        if node.skip {
            return;
        }
        match value {
            Value::Array(items) => {
                for (i, item) in items.iter_mut().enumerate() {
                    let frame = PathFrame {
                        parent: Some(path),
                        step: Step::Index(i as u32),
                    };
                    self.value(
                        item,
                        name,
                        key,
                        sets,
                        parent_type,
                        nullability.item(),
                        Some(i as u32),
                        &frame,
                    );
                }
            }
            Value::Object(object) if !sets.is_empty() => {
                // the object's `__typename`, else the type the query puts its fields on, else
                // the field's type; the schema's copy of a name, the walk changes the tree
                let schema = self.schema;
                let type_name = Value::object_get(object, TYPENAME_FIELD_NAME)
                    .and_then(Value::as_str)
                    .and_then(|name| schema.object_types.get(name))
                    .map(String::as_str)
                    .or_else(|| single_type_condition(sets))
                    .or_else(|| {
                        schema
                            .get_type_fields(parent_type)
                            .and_then(|fields| fields.get(name))
                            .map(|field| field.output_type_name.as_str())
                    })
                    .unwrap_or("");
                self.object(object, sets, type_name, Some(path));
            }
            _ => {}
        }
    }
}
