//! Writes the client response from slots. The same walk as `projection::response`, field for
//! field: conditions, type guards, enum checks, `__typename`, forced nulls and null bubbling
//! all behave the same. Values are copied as the subgraph wrote them.

use std::cell::OnceCell;
use std::collections::HashMap;

use bytes::BufMut;
use sonic_rs::JsonValueTrait;

use super::compile::{FieldSlot, SlotPlan, NO_SLOT};
use super::store::{Arena, Slot};
use crate::executor::execution::plan::ExecutionResultExtensions;
use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::projection::error::ProjectionError;
use crate::executor::projection::plan::{
    Condition, ConditionId, FieldRecord, ProjectionPlan, Range,
};
use crate::executor::projection::response::{trim_error_paths, ShapeCursor};
use crate::executor::response::graphql_error::GraphQLError;

/// Whether a value that was written as `null` has to bubble up
#[derive(Clone, Copy, PartialEq, Eq)]
enum Null {
    Bubble,
    Keep,
}

enum Failure {
    InvalidParentType,
    InvalidFieldType,
    Skip,
    InvalidEnumValue,
    Fatal(ProjectionError),
}

impl From<ProjectionError> for Failure {
    fn from(err: ProjectionError) -> Self {
        Failure::Fatal(err)
    }
}

/// A type name that is known, or found when first needed (see `projection::response`).
enum TypeName<'a, 'c> {
    Resolved(&'a str),
    Deferred {
        field: &'a FieldRecord,
        value: Slot,
        parent: &'c TypeName<'a, 'c>,
        cached: OnceCell<Result<&'a str, ProjectionError>>,
    },
}

struct Writer<'a> {
    arena: &'a Arena,
    slots: &'a SlotPlan,
    plan: &'a ProjectionPlan,
    schema: &'a SchemaMetadata,
    variables: &'a Option<HashMap<String, sonic_rs::Value>>,
    errors: Vec<GraphQLError>,
    buffer: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
pub fn write_response(
    arena: &Arena,
    slots: &SlotPlan,
    data: Slot,
    errors: Vec<GraphQLError>,
    extensions: &ExecutionResultExtensions<'_>,
    operation_type_name: &str,
    variables: &Option<HashMap<String, sonic_rs::Value>>,
    size_hint: usize,
    schema: &SchemaMetadata,
) -> Result<Vec<u8>, ProjectionError> {
    let plan = &*slots.projection;
    let mut out = Writer {
        arena,
        slots,
        plan,
        schema,
        variables,
        errors,
        buffer: Vec::with_capacity(size_hint),
    };
    out.buffer.extend_from_slice(b"{\"data\":");
    if let Slot::Object { first, .. } = data {
        let checkpoint = out.buffer.len();
        let mut opened = false;
        let null = out.object_fields(
            first,
            plan.root_range(),
            &TypeName::Resolved(operation_type_name),
            &mut opened,
        )?;
        if null == Null::Bubble {
            out.buffer.truncate(checkpoint);
            out.buffer.extend_from_slice(b"null");
        } else if opened {
            out.buffer.push(b'}');
        } else {
            out.buffer.extend_from_slice(b"{}");
        }
    } else {
        out.buffer.extend_from_slice(b"null");
    }

    if !out.errors.is_empty() {
        trim_error_paths(&mut out.errors, plan);
        out.buffer.extend_from_slice(b",\"errors\":");
        sonic_rs::to_writer(&mut out.buffer, &out.errors)
            .map_err(|e| ProjectionError::ErrorsSerializationFailure(e.to_string()))?;
    }
    if !extensions.is_empty() {
        out.buffer.extend_from_slice(b",\"extensions\":");
        sonic_rs::to_writer(&mut out.buffer, extensions)
            .map_err(|e| ProjectionError::ExtensionsSerializationFailure(e.to_string()))?;
    }
    out.buffer.push(b'}');
    Ok(out.buffer)
}

impl<'a> Writer<'a> {
    fn type_name(&self, name: &TypeName<'a, '_>) -> Result<&'a str, ProjectionError> {
        match name {
            TypeName::Resolved(name) => Ok(name),
            TypeName::Deferred {
                field,
                value,
                parent,
                cached,
            } => cached
                .get_or_init(|| self.resolve_type_name(field, *value, parent))
                .clone(),
        }
    }

    /// The type of a field's value: its `__typename` when it has one, or the field's type in
    /// the schema.
    fn resolve_type_name(
        &self,
        field: &'a FieldRecord,
        value: Slot,
        parent: &TypeName<'a, '_>,
    ) -> Result<&'a str, ProjectionError> {
        if field.is_typename() {
            return Ok("String");
        }
        if let Some(name) = self.typename_of(value) {
            return Ok(name);
        }
        if let Some(name) = self.slots.field_types[self.plan.field_index(field)].as_deref() {
            return Ok(name);
        }
        let parent = self.type_name(parent)?;
        let fields = self
            .schema
            .get_type_fields(parent)
            .ok_or_else(|| ProjectionError::MissingType(parent.to_string()))?;
        fields
            .get(self.plan.field_name(field))
            .map(|field| field.output_type_name.as_str())
            .ok_or_else(|| ProjectionError::MissingField {
                field_name: self.plan.field_name(field).to_string(),
                type_name: parent.to_string(),
            })
    }

    #[inline]
    fn typename_of(&self, value: Slot) -> Option<&'a str> {
        let Slot::Object { layout, first } = value else {
            return None;
        };
        let slot = self.slots.layouts[layout as usize].typename;
        if slot == NO_SLOT {
            return None;
        }
        self.arena
            .plain_str(self.arena.slots[(first + slot as u32) as usize])
    }

    fn value(
        &mut self,
        value: Slot,
        field: &'a FieldRecord,
        parent: &TypeName<'a, '_>,
        nullability: Option<ShapeCursor<'_>>,
    ) -> Result<Null, ProjectionError> {
        match value {
            Slot::List { start, len } => {
                let checkpoint = self.buffer.len();
                let item_shape = nullability.and_then(ShapeCursor::list_item);
                let item_non_null = item_shape.is_some_and(ShapeCursor::is_non_null);
                self.buffer.push(b'[');
                for i in 0..len {
                    if i > 0 {
                        self.buffer.push(b',');
                    }
                    let item = self.arena.items[(start + i) as usize];
                    let null = self.value(item, field, parent, item_shape.or(nullability))?;
                    if null == Null::Bubble && item_non_null {
                        self.buffer.truncate(checkpoint);
                        self.buffer.extend_from_slice(b"null");
                        return Ok(Null::Bubble);
                    }
                }
                self.buffer.push(b']');
                Ok(Null::Keep)
            }
            Slot::Object { first, .. } if field.has_children() => {
                let checkpoint = self.buffer.len();
                let type_name = TypeName::Deferred {
                    field,
                    value,
                    parent,
                    cached: OnceCell::new(),
                };
                let mut opened = false;
                let null = self.object_fields(first, field.children, &type_name, &mut opened)?;
                if null == Null::Bubble {
                    self.buffer.truncate(checkpoint);
                    self.buffer.extend_from_slice(b"null");
                    return Ok(Null::Bubble);
                }
                if opened {
                    self.buffer.push(b'}');
                } else {
                    self.buffer.extend_from_slice(b"{}");
                }
                Ok(Null::Keep)
            }
            Slot::Null | Slot::Missing => {
                self.buffer.extend_from_slice(b"null");
                Ok(Null::Bubble)
            }
            Slot::True => {
                self.buffer.extend_from_slice(b"true");
                Ok(Null::Keep)
            }
            Slot::False => {
                self.buffer.extend_from_slice(b"false");
                Ok(Null::Keep)
            }
            Slot::Raw(raw) => {
                let body = &self.arena.bodies[raw.body as usize];
                let (start, len) = (raw.start as usize, raw.len as usize);
                match body.get(start..start + VALUE_WINDOW) {
                    // most values are short: one fixed-size copy instead of a memcpy call
                    Some(window) if len <= VALUE_WINDOW => put_window::<VALUE_WINDOW>(
                        &mut self.buffer,
                        window.try_into().expect("VALUE_WINDOW bytes"),
                        len,
                    ),
                    _ => self.buffer.extend_from_slice(&body[start..start + len]),
                }
                Ok(Null::Keep)
            }
            Slot::Object { layout, first } => {
                // an object where the client selects nothing under it: written as it is
                self.arena.write_whole(
                    &self.slots.layouts,
                    Slot::Object { layout, first },
                    &mut self.buffer,
                );
                Ok(Null::Keep)
            }
        }
    }

    fn object_fields(
        &mut self,
        first: u32,
        range: Range,
        parent: &TypeName<'a, '_>,
        opened: &mut bool,
    ) -> Result<Null, ProjectionError> {
        let plan = self.plan;
        let fields = plan.fields(range);
        for (offset, field) in fields.iter().enumerate() {
            if let Some(guard) = field.parent_guard {
                if !plan.guard_matches(guard, self.type_name(parent)?) {
                    continue;
                }
            }
            let field_slot = &self.slots.fields[range.start as usize + offset];
            let slot = field_slot.slot;
            let value = if slot == NO_SLOT {
                Slot::Missing
            } else {
                self.arena.slots[(first + slot as u32) as usize]
            };

            let condition = match field.condition {
                Some(condition) => {
                    let field_type = OnceCell::new();
                    self.evaluate(condition, field, value, parent, &field_type)
                }
                None => Ok(()),
            };

            match condition {
                Ok(()) => {
                    self.key(opened, field_slot, field);
                    let null = if field.is_null_value() {
                        self.buffer.extend_from_slice(b"null");
                        Null::Bubble
                    } else if field.is_typename() {
                        let name = self.type_name(parent)?;
                        self.buffer.push(b'"');
                        self.buffer.put(name.as_bytes());
                        self.buffer.push(b'"');
                        Null::Keep
                    } else {
                        let nullability = matches!(value, Slot::List { .. })
                            .then(|| ShapeCursor::new(plan.shape(field.nullability())));
                        self.value(value, field, parent, nullability)?
                    };
                    if null == Null::Bubble && field.is_non_null() {
                        return Ok(Null::Bubble);
                    }
                }
                Err(Failure::Fatal(error)) => return Err(error),
                Err(Failure::Skip | Failure::InvalidParentType) => continue,
                Err(failure @ (Failure::InvalidEnumValue | Failure::InvalidFieldType)) => {
                    self.key(opened, field_slot, field);
                    self.buffer.extend_from_slice(b"null");
                    if matches!(failure, Failure::InvalidEnumValue) {
                        self.errors
                            .push(GraphQLError::from("Value is not a valid enum value"));
                    }
                    if field.is_non_null() {
                        return Ok(Null::Bubble);
                    }
                }
            }
        }
        Ok(Null::Keep)
    }

    #[inline(always)]
    fn key(&mut self, opened: &mut bool, slot: &FieldSlot, field: &FieldRecord) {
        self.buffer.push(if *opened { b',' } else { b'{' });
        *opened = true;
        if slot.key_len != 0 {
            put_window(&mut self.buffer, &slot.key, slot.key_len as usize);
        } else {
            self.buffer.push(b'"');
            // field names and aliases never need escaping
            self.buffer
                .extend_from_slice(self.plan.response_key(field).as_bytes());
            self.buffer.extend_from_slice(b"\":");
        }
    }

    fn evaluate(
        &self,
        id: ConditionId,
        field: &'a FieldRecord,
        value: Slot,
        parent: &TypeName<'a, '_>,
        field_type: &OnceCell<Result<&'a str, ProjectionError>>,
    ) -> Result<(), Failure> {
        let plan = self.plan;
        let variable = |name| {
            self.variables
                .as_ref()
                .and_then(|values| values.get(plan.text(name)))
                .and_then(|value| value.as_bool())
        };
        match plan.condition(id) {
            Condition::Include(name) => match variable(name) {
                Some(true) => Ok(()),
                _ => Err(Failure::Skip),
            },
            Condition::Skip(name) => match variable(name) {
                Some(true) => Err(Failure::Skip),
                _ => Ok(()),
            },
            Condition::ParentType(guard) => {
                if plan.guard_matches(guard, self.type_name(parent)?) {
                    Ok(())
                } else {
                    Err(Failure::InvalidParentType)
                }
            }
            Condition::FieldType(guard) => {
                let name = field_type
                    .get_or_init(|| {
                        let value = if value == Slot::Missing {
                            Slot::Missing
                        } else {
                            value
                        };
                        self.resolve_type_name(field, value, parent)
                    })
                    .clone()?;
                if plan.guard_matches(guard, name) {
                    Ok(())
                } else {
                    Err(Failure::InvalidFieldType)
                }
            }
            Condition::EnumValues(set) => match self.string_value(value) {
                Some(name) if !plan.set_matches(set, &name) => Err(Failure::InvalidEnumValue),
                _ => Ok(()),
            },
            Condition::And(left, right) => self
                .evaluate(left, field, value, parent, field_type)
                .and_then(|_| self.evaluate(right, field, value, parent, field_type)),
            Condition::Or(left, right) => self
                .evaluate(left, field, value, parent, field_type)
                .or_else(|_| self.evaluate(right, field, value, parent, field_type)),
        }
    }

    /// The value as a string, when it's one
    fn string_value(&self, value: Slot) -> Option<std::borrow::Cow<'a, str>> {
        self.arena.string(value)
    }
}

/// Values this short are copied as a fixed window from the body
const VALUE_WINDOW: usize = 32;

/// Appends the first `len` bytes of `window`. It stores the whole window into the spare
/// capacity and moves the length by `len`: a fixed-size copy compiles to a few stores, where a
/// `memcpy` of a short, variable length costs more than the copy.
#[inline(always)]
fn put_window<const W: usize>(buffer: &mut Vec<u8>, window: &[u8; W], len: usize) {
    debug_assert!(len <= W);
    buffer.reserve(W);
    // SAFETY: `reserve` made room for `W` bytes past the length, and the first `len` of them
    // are initialized by the copy before the length covers them
    unsafe {
        let end = buffer.as_mut_ptr().add(buffer.len());
        std::ptr::copy_nonoverlapping(window.as_ptr(), end, W);
        buffer.set_len(buffer.len() + len);
    }
}
