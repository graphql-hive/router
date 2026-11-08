//! Walks the response data in cells for plugins (see `ResponseData::walk`): the writer's walk,
//! field for field with the same conditions and type guards, visiting values instead of writing
//! them. A plugin changes the cells as it goes; the writer bubbles the nulls later.

use std::cell::OnceCell;
use std::collections::HashMap;

use super::compile::{FetchShape, IndexedResponsePlan, ObjectLayout, NO_FIELD, NO_OBJECT};
use bytes::Bytes;

use super::store::{plain_str_of, Arena, JsonSpan, ValueCell, EDITS_BODY};
use super::write::{Failure, TypeName, Writer};
use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::plugins::response_data::{
    DataValue, Nullability, PathFrame, Place, Step, WalkResult,
};
use crate::executor::projection::error::ProjectionError;
use crate::executor::projection::plan::{FieldRecord, Range};
use crate::executor::projection::response::ShapeCursor;
use crate::executor::response::graphql_error::GraphQLError;

#[allow(clippy::too_many_arguments)]
pub(crate) fn walk<F: FnMut(&mut DataValue<'_, '_>)>(
    arena: &mut Arena,
    indexed: &IndexedResponsePlan,
    data: ValueCell,
    root_type: &str,
    schema: &SchemaMetadata,
    variables: &Option<HashMap<String, sonic_rs::Value>>,
    errors: &mut Vec<GraphQLError>,
    visit: F,
) -> WalkResult {
    let ValueCell::Object { first, .. } = data else {
        return Ok(());
    };
    let mut walker = Walker {
        arena,
        indexed,
        schema,
        variables,
        errors,
        visit,
    };
    walker.object(first, indexed.projection.root_range(), root_type, None)
}

struct Walker<'w, F> {
    arena: &'w mut Arena,
    indexed: &'w IndexedResponsePlan,
    schema: &'w SchemaMetadata,
    variables: &'w Option<HashMap<String, sonic_rs::Value>>,
    errors: &'w mut Vec<GraphQLError>,
    visit: F,
}

impl<'w, F: FnMut(&mut DataValue<'_, '_>)> Walker<'w, F> {
    fn object(
        &mut self,
        first: u32,
        range: Range,
        parent_type: &str,
        path: Option<&PathFrame<'_>>,
    ) -> WalkResult {
        let indexed = self.indexed;
        let plan = &*indexed.projection;
        for (offset, field) in plan.fields(range).iter().enumerate() {
            if field.is_typename() || field.is_null_value() {
                continue;
            }
            if let Some(guard) = field.parent_guard {
                if !plan.guard_matches(guard, parent_type) {
                    continue;
                }
            }
            let field_index = indexed.fields[range.start as usize + offset].field_index;
            // not in the layout: the writer writes null, there's nothing to change
            if field_index == NO_FIELD {
                continue;
            }
            let index = first + field_index as u32;
            if let Some(condition) = field.condition {
                let value = self.arena.cells[index as usize];
                let reader = Writer::reader(&*self.arena, indexed, self.schema, self.variables);
                match reader.evaluate(
                    condition,
                    field,
                    value,
                    &TypeName::Resolved(parent_type),
                    &OnceCell::new(),
                ) {
                    // an enum value that isn't one is visited: a plugin may set a valid one, the
                    // writer writes null with an error otherwise
                    Ok(()) | Err(Failure::InvalidEnumValue) => {}
                    Err(Failure::Fatal(error)) => return Err(error),
                    // left out, or written as null by the writer
                    Err(_) => continue,
                }
            }
            let frame = PathFrame {
                parent: path,
                step: Step::Key(plan.response_key(field)),
            };
            let nullability = Nullability::Plan {
                non_null: field.is_non_null(),
                shape: ShapeCursor::new(plan.shape(field.nullability())),
            };
            self.value(false, index, field, parent_type, nullability, None, &frame)?;
        }
        Ok(())
    }

    /// Visits the value at `cells[index]` (`items[index]` for an item), then what's under it
    #[allow(clippy::too_many_arguments)]
    fn value(
        &mut self,
        item: bool,
        index: u32,
        field: &'w FieldRecord,
        parent_type: &str,
        nullability: Nullability<'w>,
        list_index: Option<u32>,
        path: &PathFrame<'_>,
    ) -> WalkResult {
        let indexed = self.indexed;
        let plan = &*indexed.projection;
        let mut value = DataValue {
            schema: self.schema,
            field_name: plan.field_name(field),
            response_key: plan.response_key(field),
            parent_type,
            composite: field.has_children(),
            nullability,
            item: list_index,
            path,
            place: Place::Cell {
                arena: &mut *self.arena,
                item,
                index,
            },
            errors: &mut *self.errors,
            skip: false,
        };
        (self.visit)(&mut value);
        if value.skip {
            return Ok(());
        }
        // read again: the plugin may have changed it
        let cell = if item {
            self.arena.items[index as usize]
        } else {
            self.arena.cells[index as usize]
        };
        match cell {
            ValueCell::List { start, len } => {
                let item_nullability = nullability.item();
                for i in 0..len {
                    let frame = PathFrame {
                        parent: Some(path),
                        step: Step::Index(i),
                    };
                    self.value(
                        true,
                        start + i,
                        field,
                        parent_type,
                        item_nullability,
                        Some(i),
                        &frame,
                    )?;
                }
            }
            ValueCell::Object { layout, first } if field.has_children() => {
                // like the writer: the object's `__typename`, else the field's type. The name is
                // read from the subgraph's body, kept alive here: the walk changes cells, never
                // bodies.
                let typename = typename_of(self.arena, &self.indexed.layouts, layout, first);
                let type_name = match typename.as_ref().and_then(|(body, span)| {
                    // SAFETY: the body's bytes are what the parser read
                    unsafe {
                        plain_str_of(&body[span.start as usize..(span.start + span.len) as usize])
                    }
                }) {
                    Some(name) => name,
                    None => self.field_type(field, parent_type)?,
                };
                self.object(first, field.children, type_name, Some(path))?;
            }
            _ => {}
        }
        Ok(())
    }

    /// The field's type: found when the plan was compiled, or in the schema
    fn field_type(
        &self,
        field: &'w FieldRecord,
        parent_type: &str,
    ) -> Result<&'w str, ProjectionError> {
        let indexed = self.indexed;
        let plan = &*indexed.projection;
        if let Some(name) = indexed.field_types[plan.field_index(field)].as_deref() {
            return Ok(name);
        }
        self.schema
            .get_type_fields(parent_type)
            .and_then(|fields| fields.get(plan.field_name(field)))
            .map(|field| field.output_type_name.as_str())
            .ok_or_else(|| ProjectionError::MissingField {
                field_name: plan.field_name(field).to_string(),
                type_name: parent_type.to_string(),
            })
    }
}

/// The body and range of an object's `__typename`, when it has one. The body is a cheap clone
/// that keeps the name alive while the walk changes the cells.
fn typename_of(
    arena: &Arena,
    layouts: &[ObjectLayout],
    layout: u32,
    first: u32,
) -> Option<(Bytes, JsonSpan)> {
    let field_index = layouts[layout as usize].typename;
    if field_index == NO_FIELD {
        return None;
    }
    match arena.cells[(first + field_index as u32) as usize] {
        ValueCell::RawJson(span) if span.body != EDITS_BODY => {
            Some((arena.bodies[span.body as usize].clone(), span))
        }
        _ => None,
    }
}

/// Walks one fetch's data in cells, before it's merged, field for field the way the fetch's
/// query selects it (see `SubgraphData::walk`). Field names and types come from the fetch's
/// shape and the schema.
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_fetch<F: FnMut(&mut DataValue<'_, '_>)>(
    arena: &mut Arena,
    layouts: &[ObjectLayout],
    shape: &FetchShape,
    data: ValueCell,
    root_type: &str,
    schema: &SchemaMetadata,
    errors: &mut Vec<GraphQLError>,
    visit: F,
) -> WalkResult {
    let ValueCell::Object { first, .. } = data else {
        return Ok(());
    };
    let mut walker = FetchWalker {
        arena,
        layouts,
        shape,
        schema,
        errors,
        visit,
    };
    walker.object(0, first, root_type, None);
    Ok(())
}

struct FetchWalker<'w, F> {
    arena: &'w mut Arena,
    layouts: &'w [ObjectLayout],
    shape: &'w FetchShape,
    schema: &'w SchemaMetadata,
    errors: &'w mut Vec<GraphQLError>,
    visit: F,
}

impl<'w, F: FnMut(&mut DataValue<'_, '_>)> FetchWalker<'w, F> {
    fn object(&mut self, object: u32, first: u32, parent_type: &str, path: Option<&PathFrame<'_>>) {
        let shape = self.shape;
        let (start, end) = shape.objects[object as usize].nodes;
        for node_index in start..end {
            let node = &shape.nodes[node_index as usize];
            let name = shape.name(node_index as usize);
            // `__typename` can't change; a key that's a different field per type can't be typed
            if node.field_index == NO_FIELD || name.is_empty() || name == "__typename" {
                continue;
            }
            let quoted = shape.key(node);
            let key = std::str::from_utf8(&quoted[1..quoted.len() - 1]).unwrap_or("");
            let frame = PathFrame {
                parent: path,
                step: Step::Key(key),
            };
            self.value(
                false,
                first + node.field_index as u32,
                node.object,
                name,
                key,
                parent_type,
                Nullability::Schema { depth: 0 },
                None,
                &frame,
            );
        }
    }

    /// Visits the value at `cells[index]` (`items[index]` for an item), then what's under it.
    /// `object` is the shape object of the field's objects.
    #[allow(clippy::too_many_arguments)]
    fn value(
        &mut self,
        item: bool,
        index: u32,
        object: u32,
        name: &'w str,
        key: &'w str,
        parent_type: &str,
        nullability: Nullability<'w>,
        list_index: Option<u32>,
        path: &PathFrame<'_>,
    ) {
        let mut value = DataValue {
            schema: self.schema,
            field_name: name,
            response_key: key,
            parent_type,
            composite: object != NO_OBJECT,
            nullability,
            item: list_index,
            path,
            place: Place::Cell {
                arena: &mut *self.arena,
                item,
                index,
            },
            errors: &mut *self.errors,
            skip: false,
        };
        (self.visit)(&mut value);
        if value.skip {
            return;
        }
        let cell = if item {
            self.arena.items[index as usize]
        } else {
            self.arena.cells[index as usize]
        };
        match cell {
            ValueCell::List { start, len } => {
                for i in 0..len {
                    let frame = PathFrame {
                        parent: Some(path),
                        step: Step::Index(i),
                    };
                    self.value(
                        true,
                        start + i,
                        object,
                        name,
                        key,
                        parent_type,
                        nullability.item(),
                        Some(i),
                        &frame,
                    );
                }
            }
            ValueCell::Object { layout, first } if object != NO_OBJECT => {
                // the object's `__typename`, else the type the query puts its fields on, else
                // the field's type
                let typename = typename_of(self.arena, self.layouts, layout, first);
                let schema = self.schema;
                let type_name = typename
                    .as_ref()
                    .and_then(|(body, span)| {
                        // SAFETY: the body's bytes are what the parser read
                        unsafe {
                            plain_str_of(
                                &body[span.start as usize..(span.start + span.len) as usize],
                            )
                        }
                    })
                    .or_else(|| self.shape.object_type(object))
                    .or_else(|| {
                        schema
                            .get_type_fields(parent_type)
                            .and_then(|fields| fields.get(name))
                            .map(|field| field.output_type_name.as_str())
                    })
                    .unwrap_or("");
                self.object(object, first, type_name, Some(path));
            }
            _ => {}
        }
    }
}
