//! The response data a plugin's `on_execute` end hook gets.
//!
//! It's the data as the client will get it: only the fields the operation asked for, under
//! their response keys, in the operation's order. How the router stores the data while it runs
//! the plan (a `Value` tree or slots) stays hidden, so both pipelines hand plugins the same
//! thing. A plugin can read anything, but can only change data in ways that keep the response
//! matching the operation: set a leaf (scalar or enum) to a value of its type, or null a
//! nullable position. No fields come or go, nothing moves, no type changes.
//!
//! The data is read lazily: a plugin that never reads it costs nothing, and data no plugin
//! changed is sent as the writer wrote it.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::fmt;

use serde::Serialize;
// serde_json's objects keep their key order here (`preserve_order`), sonic-rs' don't once
// they're changed
use serde_json::Value as Json;

use crate::executor::introspection::schema::{FieldNullability, FieldTypeInfo, SchemaMetadata};
use crate::query_planner::ast::operation::OperationDefinition;
use crate::query_planner::ast::selection_item::SelectionItem;
use crate::query_planner::ast::selection_set::SelectionSet;

const DATA_PREFIX: &[u8] = b"{\"data\":";

/// A step of a path into the response data, like the segments of a GraphQL error's `path`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataPathSegment<'a> {
    /// A field, by its response key (the alias when it has one)
    Key(&'a str),
    /// An item of a list
    Index(usize),
}

impl<'a> From<&'a str> for DataPathSegment<'a> {
    fn from(key: &'a str) -> Self {
        Self::Key(key)
    }
}

impl From<usize> for DataPathSegment<'_> {
    fn from(index: usize) -> Self {
        Self::Index(index)
    }
}

/// Why a change to the response data was refused. The data is left as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataEditError {
    /// Nothing is at that path in the response: the operation doesn't ask for it, or it's
    /// under a null or behind a type condition the object doesn't match
    NotFound,
    /// The position is Non-Null in the schema
    NonNull,
    /// The position holds an object or a list. Only leaf values can be set
    NotALeaf,
    /// The value isn't of the position's type
    WrongType,
    /// `__typename` says what the object is, it can't change
    Typename,
    /// The response key is a different field depending on the object's type
    Ambiguous,
}

impl fmt::Display for DataEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "nothing at that path in the response",
            Self::NonNull => "the position is Non-Null",
            Self::NotALeaf => "only scalar and enum values can be set",
            Self::WrongType => "the value isn't of the position's type",
            Self::Typename => "__typename can't change",
            Self::Ambiguous => "the response key is a different field depending on the type",
        })
    }
}

impl std::error::Error for DataEditError {}

/// The response data, see the module docs.
pub struct ResponseData<'exec> {
    /// `{"data":` and the data, as the response writer wrote it
    body: Vec<u8>,
    parsed: OnceCell<Json>,
    edited: bool,
    operation: &'exec OperationDefinition,
    root_type: &'exec str,
    schema: &'exec SchemaMetadata,
}

impl<'exec> ResponseData<'exec> {
    /// `body` is `{"data":` and the data, the start of a response.
    pub(crate) fn new(
        body: Vec<u8>,
        operation: &'exec OperationDefinition,
        root_type: &'exec str,
        schema: &'exec SchemaMetadata,
    ) -> Self {
        debug_assert!(body.starts_with(DATA_PREFIX));
        Self {
            body,
            parsed: OnceCell::new(),
            edited: false,
            operation,
            root_type,
            schema,
        }
    }

    /// `{"data":` and the data, with the changes plugins made: the start of the response.
    pub(crate) fn into_body(self) -> Vec<u8> {
        let Some(parsed) = self.parsed.into_inner().filter(|_| self.edited) else {
            return self.body;
        };
        let mut body = self.body;
        body.truncate(DATA_PREFIX.len());
        // a `Json` always serializes
        let _ = serde_json::to_writer(&mut body, &parsed);
        body
    }

    /// The data as JSON, what the client gets under `data`.
    pub fn to_json(&self) -> Cow<'_, [u8]> {
        match self.parsed.get().filter(|_| self.edited) {
            Some(parsed) => Cow::Owned(serde_json::to_vec(parsed).unwrap_or_default()),
            None => Cow::Borrowed(&self.body[DATA_PREFIX.len()..]),
        }
    }

    /// The value at `path`, `None` when there's nothing there. An empty path is the data itself.
    pub fn get(&self, path: &[DataPathSegment<'_>]) -> Option<DataValue<'_>> {
        let mut value = self.parsed();
        for segment in path {
            value = match *segment {
                DataPathSegment::Key(key) => value.as_object()?.get(key)?,
                DataPathSegment::Index(index) => value.as_array()?.get(index)?,
            };
        }
        Some(DataValue(value))
    }

    /// Sets a scalar or enum at `path` to `value`, which must be of its type: a string for
    /// `String` and `ID`, a 32-bit integer for `Int`, a number for `Float`, a boolean for
    /// `Boolean`, one of the enum's values for an enum, any JSON for a custom scalar. `null`
    /// works like [`set_null`](Self::set_null).
    pub fn set<T: Serialize>(
        &mut self,
        path: &[DataPathSegment<'_>],
        value: T,
    ) -> Result<(), DataEditError> {
        let value = serde_json::to_value(&value).map_err(|_| DataEditError::WrongType)?;
        if value.is_null() {
            return self.set_null(path);
        }
        let (type_name, nullability) = self.position_type(path)?;
        if nullability.list_item().is_some() {
            return Err(DataEditError::NotALeaf);
        }
        let fits = match type_name {
            "String" | "ID" => value.is_string(),
            "Int" => value.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
            "Float" => value.is_number(),
            "Boolean" => value.is_boolean(),
            _ => match self.schema.enum_values.get(type_name) {
                Some(values) => value.as_str().is_some_and(|v| values.contains(v)),
                None if self.schema.is_scalar_type(type_name) => true,
                None => return Err(DataEditError::NotALeaf),
            },
        };
        if !fits {
            return Err(DataEditError::WrongType);
        }
        *self.find_mut(path)? = value;
        self.edited = true;
        Ok(())
    }

    /// Sets the value at `path` to null. The position must be nullable: a Non-Null field, or an
    /// item of a list of Non-Null items, can't be null.
    pub fn set_null(&mut self, path: &[DataPathSegment<'_>]) -> Result<(), DataEditError> {
        let (_, nullability) = self.position_type(path)?;
        if nullability.is_non_null() {
            return Err(DataEditError::NonNull);
        }
        *self.find_mut(path)? = Json::Null;
        self.edited = true;
        Ok(())
    }

    fn parsed(&self) -> &Json {
        // the writer's JSON always parses; if it ever didn't, there'd be nothing to read
        self.parsed.get_or_init(|| {
            serde_json::from_slice(&self.body[DATA_PREFIX.len()..]).unwrap_or_default()
        })
    }

    /// The value at `path`, when there's one
    fn find_mut(&mut self, path: &[DataPathSegment<'_>]) -> Result<&mut Json, DataEditError> {
        self.parsed();
        let mut value = self.parsed.get_mut().ok_or(DataEditError::NotFound)?;
        for segment in path {
            value = match *segment {
                DataPathSegment::Key(key) => value.as_object_mut().and_then(|o| o.get_mut(key)),
                DataPathSegment::Index(index) => {
                    value.as_array_mut().and_then(|a| a.get_mut(index))
                }
            }
            .ok_or(DataEditError::NotFound)?;
        }
        Ok(value)
    }

    /// The type and nullability of the position at `path`, from the operation and the schema
    fn position_type(
        &self,
        path: &[DataPathSegment<'_>],
    ) -> Result<(&'exec str, &'exec FieldNullability), DataEditError> {
        let mut parent = self.root_type;
        let mut sets = vec![&self.operation.selection_set];
        let mut current: Option<(&'exec str, &'exec FieldNullability)> = None;
        for segment in path {
            match *segment {
                DataPathSegment::Key(key) => {
                    if let Some((type_name, nullability)) = current {
                        if nullability.list_item().is_some() {
                            return Err(DataEditError::NotFound);
                        }
                        parent = type_name;
                    }
                    let mut found = None;
                    for set in std::mem::take(&mut sets) {
                        self.field(set, parent, key, &mut found)?;
                    }
                    let (info, next) = found.ok_or(DataEditError::NotFound)?;
                    current = Some((&info.output_type_name, &info.nullability));
                    sets = next;
                }
                DataPathSegment::Index(_) => {
                    let (type_name, nullability) = current.ok_or(DataEditError::NotFound)?;
                    let item = nullability.list_item().ok_or(DataEditError::NotFound)?;
                    current = Some((type_name, item));
                }
            }
        }
        current.ok_or(DataEditError::NotFound)
    }

    /// The field `key` names in `set`, whose objects are of type `parent`: its type and its
    /// selections, added to what `found` has from other sets
    #[allow(clippy::type_complexity)]
    fn field(
        &self,
        set: &'exec SelectionSet,
        parent: &'exec str,
        key: &str,
        found: &mut Option<(&'exec FieldTypeInfo, Vec<&'exec SelectionSet>)>,
    ) -> Result<(), DataEditError> {
        for item in &set.items {
            match item {
                SelectionItem::Field(field) if field.selection_identifier() == key => {
                    if field.name == "__typename" {
                        return Err(DataEditError::Typename);
                    }
                    let Some(info) = self
                        .schema
                        .get_type_fields(parent)
                        .and_then(|fields| fields.get(&field.name))
                    else {
                        continue;
                    };
                    match found {
                        // ponytail: fields of different types under one key (`... on A { x: a }
                        // ... on B { x: b }`) are refused, picking one by the object's
                        // `__typename` would allow them
                        Some((seen, _))
                            if seen.output_type_name != info.output_type_name
                                || seen.nullability != info.nullability =>
                        {
                            return Err(DataEditError::Ambiguous)
                        }
                        Some((_, sets)) => sets.push(&field.selections),
                        None => *found = Some((info, vec![&field.selections])),
                    }
                }
                SelectionItem::Field(_) | SelectionItem::FragmentSpread(_) => {}
                SelectionItem::InlineFragment(fragment) => {
                    self.field(&fragment.selections, &fragment.type_condition, key, found)?
                }
            }
        }
        Ok(())
    }
}

/// A value of the response data, to read.
#[derive(Clone, Copy)]
pub struct DataValue<'a>(&'a Json);

impl<'a> DataValue<'a> {
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    pub fn as_str(&self) -> Option<&'a str> {
        self.0.as_str()
    }

    pub fn as_bool(&self) -> Option<bool> {
        self.0.as_bool()
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.0.as_i64()
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.0.as_f64()
    }

    /// A field of an object, by its response key
    pub fn get(&self, key: &str) -> Option<DataValue<'a>> {
        self.0.as_object()?.get(key).map(DataValue)
    }

    /// An item of a list
    pub fn at(&self, index: usize) -> Option<DataValue<'a>> {
        self.0.as_array()?.get(index).map(DataValue)
    }

    /// The number of items of a list, `None` when it's not a list
    pub fn list_len(&self) -> Option<usize> {
        self.0.as_array().map(|items| items.len())
    }

    /// The response keys of an object, in order. Empty when it's not an object.
    pub fn keys(&self) -> impl Iterator<Item = &'a str> {
        self.0
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys().map(String::as_str))
    }

    /// The value as JSON
    pub fn to_json(&self) -> String {
        self.0.to_string()
    }
}

impl fmt::Debug for DataValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::{DataEditError, DataPathSegment as P, ResponseData};
    use crate::executor::plugins::hooks::on_supergraph_load::Supergraph;
    use crate::executor::slots::tests::HEADER;
    use crate::query_planner::ast::normalization::normalize_operation;
    use crate::query_planner::utils::parsing::parse_operation;

    const TYPES: &str = r#"
type Query @join__type(graph: A) {
  products: [Product!]!
  product: Product
}
type Product @join__type(graph: A) {
  id: ID!
  name: String
  price: Float!
  stock: Int
  state: State
  tags: [String!]
  meta: JSON
}
enum State @join__type(graph: A) {
  ACTIVE @join__enumValue(graph: A)
  RETIRED @join__enumValue(graph: A)
}
scalar JSON @join__type(graph: A)
"#;

    #[test]
    fn edits_keep_the_response_matching_the_operation() {
        let supergraph =
            Supergraph::from_sdl("test", &format!("{HEADER}{TYPES}"), Default::default())
                .expect("builds");
        let document = parse_operation(
            "{ products { __typename id title: name price stock state tags meta } product { name } }",
        );
        let normalized = normalize_operation(&supergraph.planner.supergraph, &document, None)
            .expect("normalizes");
        let body = br#"{"data":{"products":[{"__typename":"Product","id":"1","title":"a","price":1.5,"stock":3,"state":"ACTIVE","tags":["x"],"meta":null}],"product":null}"#;
        let mut data = ResponseData::new(
            body.to_vec(),
            &normalized.operation,
            "Query",
            &supergraph.metadata,
        );
        let at = |key: &'static str| [P::Key("products"), P::Index(0), P::Key(key)];

        // nothing changed: the bytes as written
        assert_eq!(&*data.to_json(), &body[8..]);
        assert_eq!(data.get(&at("title")).and_then(|v| v.as_str()), Some("a"));
        assert_eq!(
            data.get(&[P::Key("products")]).and_then(|v| v.list_len()),
            Some(1)
        );

        // by response key, with the field's type
        assert_eq!(data.set(&at("title"), 1), Err(DataEditError::WrongType));
        assert_eq!(data.set(&at("name"), "b"), Err(DataEditError::NotFound));
        assert_eq!(data.set(&at("title"), "b"), Ok(()));
        assert_eq!(data.set(&at("price"), "x"), Err(DataEditError::WrongType));
        assert_eq!(data.set(&at("price"), 2), Ok(()));
        assert_eq!(
            data.set(&at("stock"), 1u64 << 40),
            Err(DataEditError::WrongType)
        );
        assert_eq!(
            data.set(&at("state"), "NOPE"),
            Err(DataEditError::WrongType)
        );
        assert_eq!(data.set(&at("state"), "RETIRED"), Ok(()));
        assert_eq!(
            data.set(&at("meta"), [1, 2]),
            Ok(()),
            "a custom scalar takes any JSON"
        );

        // nullability, lists, what can't change
        assert_eq!(data.set_null(&at("price")), Err(DataEditError::NonNull));
        assert_eq!(data.set(&at("id"), ()), Err(DataEditError::NonNull));
        assert_eq!(data.set(&at("tags"), "y"), Err(DataEditError::NotALeaf));
        let first_tag = [P::Key("products"), P::Index(0), P::Key("tags"), P::Index(0)];
        assert_eq!(data.set(&first_tag, "y"), Ok(()));
        assert_eq!(data.set_null(&first_tag), Err(DataEditError::NonNull));
        assert_eq!(data.set_null(&at("tags")), Ok(()));
        assert_eq!(
            data.set(&at("__typename"), "Other"),
            Err(DataEditError::Typename)
        );
        assert_eq!(
            data.set_null(&[P::Key("products")]),
            Err(DataEditError::NonNull)
        );
        assert_eq!(
            data.set(&[P::Key("product"), P::Key("name")], "c"),
            Err(DataEditError::NotFound),
            "under a null"
        );
        assert_eq!(data.set_null(&at("stock")), Ok(()));

        assert_eq!(
            String::from_utf8(data.into_body()).unwrap(),
            r#"{"data":{"products":[{"__typename":"Product","id":"1","title":"b","price":2,"stock":null,"state":"RETIRED","tags":null,"meta":[1,2]}],"product":null}"#
        );
    }
}
