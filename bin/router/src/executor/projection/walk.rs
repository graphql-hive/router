//! Walks the response data in a `Value` tree for plugins (see `ResponseData::walk`): the
//! projector's walk, field for field with the same conditions and type guards, visiting values
//! instead of writing them. A plugin changes the tree as it goes; the projection bubbles the
//! nulls later.

use std::cell::OnceCell;
use std::collections::HashMap;

use super::error::ProjectionError;
use super::plan::{FieldRecord, ProjectionPlan, Range};
use super::response::{evaluate, resolve_type_name, ConditionFailure, ShapeCursor, TypeName};
use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::plugins::response_data::{DataValue, PathFrame, Place, Step, WalkResult};
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
                    Ok(()) => {}
                    Err(ConditionFailure::Fatal(error)) => return Err(error),
                    // left out, or written as null by the projection
                    Err(_) => continue,
                }
            }
            let frame = PathFrame {
                parent: path,
                step: Step::Key(key),
            };
            let shape = ShapeCursor::new(plan.shape(field.nullability()));
            self.value(
                &mut object[position].1,
                field,
                parent_type,
                shape,
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
        shape: ShapeCursor<'w>,
        list_index: Option<u32>,
        path: &PathFrame<'_>,
    ) -> WalkResult {
        let mut node = DataValue {
            plan: self.plan,
            schema: self.schema,
            field,
            parent_type,
            shape,
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
                let item_shape = shape.list_item().unwrap_or(shape);
                for (i, item) in items.iter_mut().enumerate() {
                    let frame = PathFrame {
                        parent: Some(path),
                        step: Step::Index(i as u32),
                    };
                    self.value(item, field, parent_type, item_shape, Some(i as u32), &frame)?;
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
