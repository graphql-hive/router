//! The response data a plugin's `on_execute` end hook gets.
//!
//! A plugin walks the data the client asked for, value by value, the same way the response
//! writer does: only the fields the operation selects, under their response keys, each with the
//! field it belongs to. On the way it can read values and change them, but only in ways that keep
//! the response matching the operation and the schema:
//! - set a scalar or enum to a value of its type,
//! - set any value to null, with an error if it wants one. In a Non-Null position the null
//!   bubbles up to the closest nullable parent when the response is written, like a null from a
//!   subgraph does.
//!
//! No field comes or goes, lists stay lists and objects stay objects.
//!
//! Nothing is copied or converted: the walk goes over the router's own storage (slots or a
//! `Value` tree) and changes it in place before the response is written. Walking allocates
//! nothing; a value a plugin sets is kept in a buffer the router reuses across requests.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;

use crate::executor::execution::plan::CoerceVariablesPayload;
use crate::executor::introspection::schema::{FieldNullability, FieldTypeInfo, SchemaMetadata};
use crate::executor::projection::error::ProjectionError;
use crate::executor::projection::plan::ProjectionPlan;
use crate::executor::projection::response::{project_data, ShapeCursor};
use crate::executor::response::graphql_error::{
    GraphQLError, GraphQLErrorPath, GraphQLErrorPathSegment,
};
use crate::executor::response::value::Value;
use crate::executor::slots::compile::SlotPlan;
use crate::executor::slots::store::{Arena, Raw, Slot, EDITS_BODY};
use crate::executor::slots::write::write_data;
use crate::query_planner::state::supergraph_state::OperationKind;

/// Why a change to a value was refused. The value is left as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataEditError {
    /// The value is an object or a list. Only scalars and enums can be set; any value can be
    /// set to null.
    NotALeaf,
    /// The new value isn't of the field's type
    WrongType,
}

impl fmt::Display for DataEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotALeaf => "only scalar and enum values can be set",
            Self::WrongType => "the value isn't of the field's type",
        })
    }
}

impl std::error::Error for DataEditError {}

/// The response data, see the module docs.
pub struct ResponseData<'exec> {
    store: Store<'exec>,
    schema: Arc<SchemaMetadata>,
    variables: Arc<CoerceVariablesPayload>,
    kind: OperationKind,
    /// Errors plugins attached to the nulls they set
    errors: Vec<GraphQLError>,
}

// one per request, the size difference doesn't matter
#[allow(clippy::large_enum_variant)]
enum Store<'exec> {
    Slots {
        arena: Arena,
        plan: Arc<SlotPlan>,
        data: Slot,
    },
    Value {
        data: Value<'exec>,
        plan: Arc<ProjectionPlan>,
    },
}

impl<'exec> ResponseData<'exec> {
    pub(crate) fn from_slots(
        arena: Arena,
        data: Slot,
        plan: Arc<SlotPlan>,
        schema: Arc<SchemaMetadata>,
        variables: Arc<CoerceVariablesPayload>,
        kind: OperationKind,
    ) -> Self {
        Self {
            store: Store::Slots { arena, plan, data },
            schema,
            variables,
            kind,
            errors: Vec::new(),
        }
    }

    pub(crate) fn from_value(
        data: Value<'exec>,
        plan: Arc<ProjectionPlan>,
        schema: Arc<SchemaMetadata>,
        variables: Arc<CoerceVariablesPayload>,
        kind: OperationKind,
    ) -> Self {
        Self {
            store: Store::Value { data, plan },
            schema,
            variables,
            kind,
            errors: Vec::new(),
        }
    }

    /// The slots back, with the errors plugins attached
    pub(crate) fn into_slots(self) -> (Arena, Slot, Vec<GraphQLError>) {
        match self.store {
            Store::Slots { arena, data, .. } => (arena, data, self.errors),
            Store::Value { .. } => unreachable!("the data of the slot pipeline is slots"),
        }
    }

    /// The `Value` back, with the errors plugins attached
    pub(crate) fn into_value(self) -> (Value<'exec>, Vec<GraphQLError>) {
        match self.store {
            Store::Value { data, .. } => (data, self.errors),
            Store::Slots { .. } => unreachable!("the data of the Value pipeline is a Value"),
        }
    }

    /// Visits every value of the response the client asked for, parents before their children:
    /// each field's value, and each item of a list. `__typename` isn't visited, it can't change.
    ///
    /// ```ignore
    /// payload.data.walk(|value| {
    ///     if value.is("User", "email") {
    ///         let _ = value.set_null_with_error("email is private");
    ///     }
    /// });
    /// ```
    ///
    /// Changes made on the way count right away: the walk goes under the new value, so nothing
    /// is visited under a value set to null. [`DataValue::skip_children`] leaves out what's under
    /// a value.
    ///
    /// An entity the router fetched once for several positions keeps its nested objects in one
    /// place, so a change inside them shows at every position of that entity.
    // ponytail: shared nested objects of deduplicated entities; copy-on-write if a plugin ever
    // needs to change one position of the same entity and not the others
    pub fn walk<F: FnMut(&mut DataValue<'_, '_>)>(&mut self, visit: F) {
        let schema = &*self.schema;
        let variables = &self.variables.variables_map;
        let root = root_type(schema, &self.kind);
        let walked = match &mut self.store {
            Store::Slots { arena, plan, data } => crate::executor::slots::walk::walk(
                arena,
                plan,
                *data,
                root,
                schema,
                variables,
                &mut self.errors,
                visit,
            ),
            Store::Value { data, plan } => crate::executor::projection::walk::walk(
                data,
                plan,
                root,
                schema,
                variables,
                &mut self.errors,
                visit,
            ),
        };
        // a plan the walk can't follow, the writer can't either: it reports it
        if let Err(error) = walked {
            tracing::debug!(%error, "walking the response data stopped");
        }
    }

    /// The data as JSON, what the client gets under `data` with the changes so far. It writes
    /// the response for it, so it costs as much.
    pub fn to_json(&self) -> Vec<u8> {
        let schema = &*self.schema;
        let variables = &self.variables.variables_map;
        let root = root_type(schema, &self.kind);
        let written = match &self.store {
            Store::Slots { arena, plan, data } => {
                write_data(arena, plan, *data, Vec::new(), root, variables, 0, schema)
            }
            Store::Value { data, plan } => {
                project_data(data, Vec::new(), root, plan, variables, 0, schema)
            }
        };
        match written {
            Ok((mut body, _)) => {
                body.drain(..DATA_PREFIX.len());
                body
            }
            Err(_) => b"null".to_vec(),
        }
    }
}

const DATA_PREFIX: &[u8] = b"{\"data\":";

fn root_type<'a>(schema: &'a SchemaMetadata, kind: &OperationKind) -> &'a str {
    match kind {
        OperationKind::Query => schema.query_type_name.as_deref().unwrap_or("Query"),
        OperationKind::Mutation => schema.mutation_type_name.as_deref().unwrap_or("Mutation"),
        OperationKind::Subscription => schema
            .subscription_type_name
            .as_deref()
            .unwrap_or("Subscription"),
    }
}

/// Where a walk is: a frame per step, on the walk's own stack. The path is only put together
/// when an error needs it.
pub(crate) struct PathFrame<'p> {
    pub parent: Option<&'p PathFrame<'p>>,
    pub step: Step<'p>,
}

#[derive(Clone, Copy)]
pub(crate) enum Step<'p> {
    Key(&'p str),
    Index(u32),
}

impl PathFrame<'_> {
    fn to_path(&self) -> GraphQLErrorPath {
        let mut segments = Vec::new();
        let mut frame = Some(self);
        while let Some(current) = frame {
            segments.push(match current.step {
                Step::Key(key) => GraphQLErrorPathSegment::String(key.to_string()),
                Step::Index(index) => GraphQLErrorPathSegment::Index(index as usize),
            });
            frame = current.parent;
        }
        segments.reverse();
        GraphQLErrorPath { segments }
    }
}

/// Where a value is stored
pub(crate) enum Place<'w, 'v> {
    /// `slots[index]` (a field of an object) or `items[index]` (an item of a list)
    Slot {
        arena: &'w mut Arena,
        item: bool,
        index: u32,
    },
    Value(&'w mut Value<'v>),
}

/// How nullable a value is
#[derive(Clone, Copy)]
pub(crate) enum Nullability<'w> {
    /// From the client operation's projection plan: whether this value is Non-Null, and the
    /// shape of its type from this level in (for the items of a list)
    Plan {
        non_null: bool,
        shape: ShapeCursor<'w>,
    },
    /// From the schema, looked up when asked: how many lists deep the value is in its field
    Schema { depth: u8 },
}

impl<'w> Nullability<'w> {
    /// The nullability of an item of this list
    pub(crate) fn item(self) -> Self {
        match self {
            Nullability::Plan { shape, .. } => {
                let shape = shape.list_item().unwrap_or(shape);
                Nullability::Plan {
                    non_null: shape.is_non_null(),
                    shape,
                }
            }
            Nullability::Schema { depth } => Nullability::Schema {
                depth: depth.saturating_add(1),
            },
        }
    }
}

/// A value of the response during a walk: a field's value, or an item of a list. It reads the
/// value and changes it, see [`ResponseData::walk`].
pub struct DataValue<'w, 'v> {
    pub(crate) schema: &'w SchemaMetadata,
    pub(crate) field_name: &'w str,
    pub(crate) response_key: &'w str,
    /// The type of the object the field is on
    pub(crate) parent_type: &'w str,
    /// The field selects fields under it: its values are objects, or lists of them
    pub(crate) composite: bool,
    pub(crate) nullability: Nullability<'w>,
    pub(crate) item: Option<u32>,
    pub(crate) path: &'w PathFrame<'w>,
    pub(crate) place: Place<'w, 'v>,
    pub(crate) errors: &'w mut Vec<GraphQLError>,
    pub(crate) skip: bool,
}

impl<'w, 'v> DataValue<'w, 'v> {
    /// The field's name in the schema
    pub fn field_name(&self) -> &'w str {
        self.field_name
    }

    /// The field's response key: its alias, or its name
    pub fn response_key(&self) -> &'w str {
        self.response_key
    }

    /// The type of the object the field is on
    pub fn parent_type(&self) -> &'w str {
        self.parent_type
    }

    /// Whether the value belongs to the field `field_name` of `type_name`: the object's type, or
    /// an interface or union it's part of
    pub fn is(&self, type_name: &str, field_name: &str) -> bool {
        self.field_name == field_name
            && (self.parent_type == type_name
                || self
                    .schema
                    .possible_types
                    .entity_satisfies_type_condition(self.parent_type, type_name))
    }

    /// The index in its list, for an item of a list
    pub fn index(&self) -> Option<usize> {
        self.item.map(|index| index as usize)
    }

    /// The field's type in the schema
    fn field_info(&self) -> Option<&'w FieldTypeInfo> {
        self.schema
            .get_type_fields(self.parent_type)?
            .get(self.field_name)
    }

    /// The nullability of this value's position, from the schema: `depth` lists into the field
    fn schema_nullability(&self, depth: u8) -> Option<&'w FieldNullability> {
        let mut nullability = &self.field_info()?.nullability;
        for _ in 0..depth {
            nullability = nullability.list_item()?;
        }
        Some(nullability)
    }

    /// Whether the position is Non-Null: a null set here bubbles up
    pub fn is_non_null(&self) -> bool {
        match self.nullability {
            Nullability::Plan { non_null, .. } => non_null,
            Nullability::Schema { depth } => self
                .schema_nullability(depth)
                .is_some_and(FieldNullability::is_non_null),
        }
    }

    /// Whether the position holds a list
    fn is_list_position(&self) -> bool {
        match self.nullability {
            Nullability::Plan { shape, .. } => shape.list_item().is_some(),
            Nullability::Schema { depth } => self
                .schema_nullability(depth)
                .is_some_and(|nullability| nullability.list_item().is_some()),
        }
    }

    /// The path of the value in the response, like a GraphQL error's
    pub fn path(&self) -> GraphQLErrorPath {
        self.path.to_path()
    }

    /// Leaves out the values under this one for the rest of the walk
    pub fn skip_children(&mut self) {
        self.skip = true;
    }

    fn slot(&self) -> Option<(Slot, &Arena)> {
        match &self.place {
            Place::Slot { arena, item, index } => {
                let table = if *item { &arena.items } else { &arena.slots };
                Some((table[*index as usize], &**arena))
            }
            Place::Value(_) => None,
        }
    }

    fn value(&self) -> Option<&Value<'v>> {
        match &self.place {
            Place::Value(value) => Some(value),
            Place::Slot { .. } => None,
        }
    }

    /// The bytes of a scalar as JSON, for numbers
    fn number_text(&self) -> Option<&str> {
        let (Slot::Raw(raw), arena) = self.slot()? else {
            return None;
        };
        let bytes = arena.raw(raw);
        match bytes.first() {
            Some(b'-' | b'0'..=b'9') => std::str::from_utf8(bytes).ok(),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        match self.slot() {
            Some((slot, _)) => matches!(slot, Slot::Null | Slot::Missing),
            None => matches!(self.value(), Some(Value::Null)),
        }
    }

    pub fn is_list(&self) -> bool {
        match self.slot() {
            Some((slot, _)) => matches!(slot, Slot::List { .. }),
            None => matches!(self.value(), Some(Value::Array(_))),
        }
    }

    pub fn is_object(&self) -> bool {
        match self.slot() {
            Some((slot, _)) => matches!(slot, Slot::Object { .. }),
            None => matches!(self.value(), Some(Value::Object(_))),
        }
    }

    /// The number of items, for a list
    pub fn list_len(&self) -> Option<usize> {
        match self.slot() {
            Some((Slot::List { len, .. }, _)) => Some(len as usize),
            Some(_) => None,
            None => match self.value() {
                Some(Value::Array(items)) => Some(items.len()),
                _ => None,
            },
        }
    }

    /// The value as a string, for a string or an enum. It's borrowed unless the string has
    /// escapes.
    pub fn as_str(&self) -> Option<Cow<'_, str>> {
        match self.slot() {
            Some((slot, arena)) => arena.string(slot),
            None => match self.value() {
                Some(Value::String(value)) => Some(Cow::Borrowed(&**value)),
                _ => None,
            },
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self.slot() {
            Some((Slot::True, _)) => Some(true),
            Some((Slot::False, _)) => Some(false),
            Some(_) => None,
            None => match self.value() {
                Some(Value::Bool(value)) => Some(*value),
                _ => None,
            },
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        if self.slot().is_some() {
            return self.number_text()?.parse().ok();
        }
        match self.value()? {
            Value::I64(value) => Some(*value),
            Value::U64(value) => i64::try_from(*value).ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        if self.slot().is_some() {
            return self.number_text()?.parse().ok();
        }
        match self.value()? {
            Value::F64(value) => Some(*value),
            Value::I64(value) => Some(*value as f64),
            Value::U64(value) => Some(*value as f64),
            _ => None,
        }
    }

    /// Sets the value to `value`, which must be of the field's type: a string for `String` and
    /// `ID`, a 32-bit integer for `Int`, a number for `Float`, a boolean for `Boolean`, one of
    /// the enum's values for an enum, any JSON for a custom scalar. Only scalars and enums can
    /// be set. `null` works like [`set_null`](Self::set_null).
    pub fn set<T: Serialize>(&mut self, value: T) -> Result<(), DataEditError> {
        if self.composite || self.is_list_position() {
            return Err(DataEditError::NotALeaf);
        }
        let type_name = self
            .field_info()
            .map(|field| field.output_type_name.as_str())
            .ok_or(DataEditError::WrongType)?;
        let schema = self.schema;
        match &mut self.place {
            Place::Slot { arena, item, index } => {
                let start = arena.edits.len();
                serde_json::to_writer(&mut arena.edits, &value)
                    .map_err(|_| DataEditError::WrongType)?;
                let json = &arena.edits[start..];
                let slot = match fits(schema, type_name, json) {
                    Ok(slot) => slot,
                    Err(error) => {
                        arena.edits.truncate(start);
                        return Err(error);
                    }
                };
                let slot = slot.unwrap_or(Slot::Raw(Raw {
                    body: EDITS_BODY,
                    start: start as u32,
                    len: (arena.edits.len() - start) as u32,
                }));
                if !matches!(slot, Slot::Raw(_)) {
                    arena.edits.truncate(start);
                }
                let table = if *item {
                    &mut arena.items
                } else {
                    &mut arena.slots
                };
                table[*index as usize] = slot;
            }
            Place::Value(target) => {
                // ponytail: the Value pipeline serializes into a new buffer and builds a new
                // `Value`; it allocates, the slot pipeline doesn't
                let json = serde_json::to_vec(&value).map_err(|_| DataEditError::WrongType)?;
                let slot = fits(schema, type_name, &json)?;
                **target = match slot {
                    Some(Slot::Null) => Value::Null,
                    Some(Slot::True) => Value::Bool(true),
                    Some(Slot::False) => Value::Bool(false),
                    _ => json_to_value(schema, type_name, json)?,
                };
            }
        }
        Ok(())
    }

    /// Sets the value to null, whatever it is. In a Non-Null position the null bubbles up to the
    /// closest nullable parent when the response is written.
    pub fn set_null(&mut self) {
        match &mut self.place {
            Place::Slot { arena, item, index } => {
                let table = if *item {
                    &mut arena.items
                } else {
                    &mut arena.slots
                };
                table[*index as usize] = Slot::Null;
            }
            Place::Value(target) => **target = Value::Null,
        }
    }

    /// [`set_null`](Self::set_null), and adds `error` to the response's errors with the path of
    /// this value
    pub fn set_null_with_error(&mut self, error: impl Into<GraphQLError>) {
        self.set_null();
        let mut error = error.into();
        error.path = Some(self.path());
        self.errors.push(error);
    }
}

/// Whether `json` is a value of `type_name`. A value that isn't kept as bytes comes back as its
/// slot (`null`, `true`, `false`); one that is, as `None`.
fn fits(
    schema: &SchemaMetadata,
    type_name: &str,
    json: &[u8],
) -> Result<Option<Slot>, DataEditError> {
    match json {
        b"null" => return Ok(Some(Slot::Null)),
        b"true" | b"false" if type_name == "Boolean" || schema.is_scalar_type(type_name) => {
            return Ok(Some(if json == b"true" {
                Slot::True
            } else {
                Slot::False
            }))
        }
        _ => {}
    }
    let is_number = matches!(json.first(), Some(b'-' | b'0'..=b'9'));
    let fits = match type_name {
        "String" | "ID" => json.first() == Some(&b'"'),
        "Int" => {
            is_number
                && std::str::from_utf8(json)
                    .ok()
                    .is_some_and(|text| text.parse::<i32>().is_ok())
        }
        "Float" => is_number,
        "Boolean" => false,
        _ => match schema.enum_values.get(type_name) {
            Some(values) => json
                .strip_prefix(b"\"")
                .and_then(|inner| inner.strip_suffix(b"\""))
                .and_then(|inner| std::str::from_utf8(inner).ok())
                .is_some_and(|inner| values.contains(inner)),
            None if schema.is_scalar_type(type_name) => true,
            None => return Err(DataEditError::NotALeaf),
        },
    };
    if fits {
        Ok(None)
    } else {
        Err(DataEditError::WrongType)
    }
}

/// A `Value` of a scalar `fits` checked
fn json_to_value(
    schema: &SchemaMetadata,
    type_name: &str,
    json: Vec<u8>,
) -> Result<Value<'static>, DataEditError> {
    let custom = !matches!(type_name, "String" | "ID" | "Int" | "Float")
        && !schema.enum_values.contains_key(type_name);
    let text = String::from_utf8(json).map_err(|_| DataEditError::WrongType)?;
    if custom {
        return Ok(Value::RawJson(Cow::Owned(text)));
    }
    if text.starts_with('"') {
        let value: String = sonic_rs::from_str(&text).map_err(|_| DataEditError::WrongType)?;
        return Ok(Value::String(Cow::Owned(value)));
    }
    if let Ok(value) = text.parse::<i64>() {
        return Ok(Value::I64(value));
    }
    text.parse::<f64>()
        .map(Value::F64)
        .map_err(|_| DataEditError::WrongType)
}

/// A plan the walk can't follow, like the writer can't
pub(crate) type WalkResult = Result<(), ProjectionError>;

/// A subgraph's data, as a plugin's `on_subgraph_execute` end hook gets it: what one fetch got
/// back, before it's merged into the response. Walked and changed like [`ResponseData`], with
/// the fetch's own query: its response keys (`_entities` for an entity fetch), and fields typed
/// by the objects' `__typename` and the schema.
pub struct SubgraphData<'exec> {
    store: FetchStore<'exec>,
    schema: &'exec SchemaMetadata,
    root_type: &'exec str,
    /// Errors plugins attached to the nulls they set
    errors: Vec<GraphQLError>,
}

// one per fetch with end hooks, the size difference doesn't matter
#[allow(clippy::large_enum_variant)]
enum FetchStore<'exec> {
    Slots {
        arena: Arena,
        plan: Arc<SlotPlan>,
        fetch: i64,
        data: Slot,
    },
    Value {
        data: Value<'exec>,
        document: &'exec str,
    },
}

impl<'exec> SubgraphData<'exec> {
    pub(crate) fn from_slots(
        arena: Arena,
        data: Slot,
        plan: Arc<SlotPlan>,
        fetch: i64,
        schema: &'exec SchemaMetadata,
        kind: &OperationKind,
    ) -> Self {
        Self {
            store: FetchStore::Slots {
                arena,
                plan,
                fetch,
                data,
            },
            schema,
            root_type: root_type(schema, kind),
            errors: Vec::new(),
        }
    }

    pub(crate) fn from_value(
        data: Value<'exec>,
        document: &'exec str,
        schema: &'exec SchemaMetadata,
        kind: &OperationKind,
    ) -> Self {
        Self {
            store: FetchStore::Value { data, document },
            schema,
            root_type: root_type(schema, kind),
            errors: Vec::new(),
        }
    }

    /// The slots back, with the errors plugins attached
    pub(crate) fn into_slots(self) -> (Arena, Slot, Vec<GraphQLError>) {
        match self.store {
            FetchStore::Slots { arena, data, .. } => (arena, data, self.errors),
            FetchStore::Value { .. } => unreachable!("the data of the slot pipeline is slots"),
        }
    }

    /// The `Value` back, with the errors plugins attached
    pub(crate) fn into_value(self) -> (Value<'exec>, Vec<GraphQLError>) {
        match self.store {
            FetchStore::Value { data, .. } => (data, self.errors),
            FetchStore::Slots { .. } => unreachable!("the data of the Value pipeline is a Value"),
        }
    }

    /// Visits every value the fetch got, parents before their children, like
    /// [`ResponseData::walk`]. Errors attached to nulls have paths in the subgraph's response
    /// (`["_entities", 0, "email"]`), and the router maps them to the client's like the
    /// subgraph's own errors.
    pub fn walk<F: FnMut(&mut DataValue<'_, '_>)>(&mut self, visit: F) {
        let schema = self.schema;
        let walked = match &mut self.store {
            FetchStore::Slots {
                arena,
                plan,
                fetch,
                data,
            } => match plan.fetches.get(fetch) {
                Some(program) => crate::executor::slots::walk::walk_fetch(
                    arena,
                    &plan.layouts,
                    &program.shape,
                    *data,
                    self.root_type,
                    schema,
                    &mut self.errors,
                    visit,
                ),
                None => Ok(()),
            },
            FetchStore::Value { data, document } => crate::executor::projection::walk::walk_fetch(
                data,
                document,
                self.root_type,
                schema,
                &mut self.errors,
                visit,
            ),
        };
        if let Err(error) = walked {
            tracing::debug!(%error, "walking the subgraph data stopped");
        }
    }
}
