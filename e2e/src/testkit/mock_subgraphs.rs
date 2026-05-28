//! Mock-subgraph harness for `TestSubgraphsBuilder.with_on_request`.
//!
//! Accepts a JSON description of canned subgraph responses and turns it into
//! an `on_request` closure that:
//!
//! * Routes each incoming HTTP request to a per-subgraph mock based on the
//!   request path (`/{subgraph_name}`).
//! * Parses the GraphQL request body, walks the operation's top-level
//!   selection set and resolves each field against the subgraph's mock map
//!   (with field aliases respected and full selection-set walking for nested
//!   objects/lists).
//! * Handles federation's `_entities` resolver by matching each
//!   `representations[i]` against the configured `entities: [..]` list using
//!   the rule "entity has at least all of representation's fields".
//!
//! This is intentionally a small, schema-less executor: it knows nothing
//! about the subgraph schema, just walks the query AST against pre-baked
//! JSON. That is enough to feed the demand-control fixtures.

use std::collections::BTreeMap;

use bytes::Bytes;
use graphql_tools::parser::{
    parse_query,
    query::{
        Definition, Field, FragmentDefinition, OperationDefinition, Selection, SelectionSet, Value,
    },
};
use serde_json::{json, Value as JsonValue};

use super::{RequestLike, ResponseLike};

/// Build an `on_request` closure compatible with `TestSubgraphsBuilder.with_on_request`
/// from a JSON description of canned subgraph responses:
///
/// ```json
/// {
///   "subgraphName": {
///     "query": { "rootField": ... },
///     "mutation": { "rootField": ... },
///     "entities": [ { "__typename": "X", "id": "1", ... }, ... ]
///   }
/// }
/// ```
pub fn mock_subgraphs(
    config: JsonValue,
) -> impl Fn(RequestLike) -> Option<ResponseLike> + Send + Sync + 'static {
    let configs: BTreeMap<String, SubgraphMock> = config
        .as_object()
        .expect("mock_subgraphs: top-level must be an object")
        .iter()
        .map(|(name, value)| (name.clone(), SubgraphMock::from_json(value)))
        .collect();

    move |req: RequestLike| {
        let subgraph_name = req.path.trim_start_matches('/').to_string();
        let mock = configs.get(&subgraph_name)?;
        let body = req.body.as_ref()?;
        let response_json = mock.handle(body);
        Some(ResponseLike {
            status: axum::http::StatusCode::OK,
            headers: {
                let mut h = http::HeaderMap::new();
                h.insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("application/json"),
                );
                h
            },
            body: Some(Bytes::from(response_json.to_string())),
        })
    }
}

#[derive(Debug, Clone)]
struct SubgraphMock {
    query: JsonValue,
    mutation: Option<JsonValue>,
    entities: Vec<JsonValue>,
}

impl SubgraphMock {
    fn from_json(value: &JsonValue) -> Self {
        Self {
            query: value.get("query").cloned().unwrap_or_else(|| json!({})),
            mutation: value.get("mutation").cloned(),
            entities: value
                .get("entities")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn handle(&self, body_bytes: &[u8]) -> JsonValue {
        let request: JsonValue = match serde_json::from_slice(body_bytes) {
            Ok(v) => v,
            Err(e) => return error_response(format!("invalid JSON body: {e}")),
        };

        let query_str = match request.get("query").and_then(|q| q.as_str()) {
            Some(q) => q,
            None => return error_response("missing `query` in subgraph request"),
        };
        let variables = request
            .get("variables")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let operation_name = request.get("operationName").and_then(|n| n.as_str());

        let document = match parse_query::<String>(query_str) {
            Ok(d) => d,
            Err(e) => return error_response(format!("parse error: {e}")),
        };

        let fragments: BTreeMap<String, FragmentDefinition<'_, String>> = document
            .definitions
            .iter()
            .filter_map(|def| match def {
                Definition::Fragment(f) => Some((f.name.clone(), f.clone())),
                _ => None,
            })
            .collect();

        let operation = document.definitions.iter().find_map(|def| match def {
            Definition::Operation(op) => Some(op),
            _ => None,
        });

        let operation = match operation {
            Some(op) => op,
            None => return error_response("no operation in subgraph query"),
        };

        let (is_mutation, selection_set) = match operation {
            OperationDefinition::Query(q) => {
                if let Some(name) = operation_name {
                    if q.name.as_deref() != Some(name) {
                        // Mock harness does not support multi-operation
                        // documents; fall through anyway.
                    }
                }
                (false, &q.selection_set)
            }
            OperationDefinition::SelectionSet(ss) => (false, ss),
            OperationDefinition::Mutation(m) => (true, &m.selection_set),
            OperationDefinition::Subscription(_) => {
                return error_response("subscription not supported in mock harness");
            }
        };

        let root = if is_mutation {
            match self.mutation.as_ref() {
                Some(m) => m,
                None => return error_response("mutation not configured for subgraph"),
            }
        } else {
            &self.query
        };

        let data = self.resolve_selection_set(selection_set, root, &variables, &fragments);

        json!({ "data": data })
    }

    fn resolve_selection_set(
        &self,
        selection_set: &SelectionSet<'_, String>,
        parent: &JsonValue,
        variables: &JsonValue,
        fragments: &BTreeMap<String, FragmentDefinition<'_, String>>,
    ) -> JsonValue {
        let mut out = serde_json::Map::new();
        for sel in &selection_set.items {
            self.collect_selection(sel, parent, variables, fragments, &mut out);
        }
        JsonValue::Object(out)
    }

    fn collect_selection(
        &self,
        selection: &Selection<'_, String>,
        parent: &JsonValue,
        variables: &JsonValue,
        fragments: &BTreeMap<String, FragmentDefinition<'_, String>>,
        out: &mut serde_json::Map<String, JsonValue>,
    ) {
        match selection {
            Selection::Field(field) => {
                let key = field.alias.clone().unwrap_or_else(|| field.name.clone());
                let value = self.resolve_field(field, parent, variables, fragments);
                out.insert(key, value);
            }
            Selection::InlineFragment(frag) => {
                for sel in &frag.selection_set.items {
                    self.collect_selection(sel, parent, variables, fragments, out);
                }
            }
            Selection::FragmentSpread(spread) => {
                if let Some(frag) = fragments.get(&spread.fragment_name) {
                    for sel in &frag.selection_set.items {
                        self.collect_selection(sel, parent, variables, fragments, out);
                    }
                }
            }
        }
    }

    fn resolve_field(
        &self,
        field: &Field<'_, String>,
        parent: &JsonValue,
        variables: &JsonValue,
        fragments: &BTreeMap<String, FragmentDefinition<'_, String>>,
    ) -> JsonValue {
        // Federation entity resolver: `_entities(representations: [...]) { ... on T { ... } }`
        if field.name == "_entities" {
            let representations = field
                .arguments
                .iter()
                .find(|(name, _)| name == "representations")
                .map(|(_, v)| coerce_value(v, variables))
                .unwrap_or(JsonValue::Null);
            let representations = representations.as_array().cloned().unwrap_or_default();

            let entities: Vec<JsonValue> = representations
                .iter()
                .map(|repr| match self.find_entity(repr) {
                    Some(entity) => self.resolve_selection_set(
                        &field.selection_set,
                        entity,
                        variables,
                        fragments,
                    ),
                    None => JsonValue::Null,
                })
                .collect();
            return JsonValue::Array(entities);
        }

        // __typename: walk parent.
        if field.name == "__typename" {
            return parent.get("__typename").cloned().unwrap_or(JsonValue::Null);
        }

        let raw = parent.get(field.name.as_str()).cloned();
        let value = match raw {
            Some(v) => v,
            None => return JsonValue::Null,
        };

        if field.selection_set.items.is_empty() {
            return value;
        }
        self.walk_value(&value, &field.selection_set, variables, fragments)
    }

    fn walk_value(
        &self,
        value: &JsonValue,
        selection_set: &SelectionSet<'_, String>,
        variables: &JsonValue,
        fragments: &BTreeMap<String, FragmentDefinition<'_, String>>,
    ) -> JsonValue {
        match value {
            JsonValue::Array(items) => JsonValue::Array(
                items
                    .iter()
                    .map(|item| self.walk_value(item, selection_set, variables, fragments))
                    .collect(),
            ),
            JsonValue::Object(_) => {
                self.resolve_selection_set(selection_set, value, variables, fragments)
            }
            JsonValue::Null => JsonValue::Null,
            other => other.clone(),
        }
    }

    fn find_entity(&self, representation: &JsonValue) -> Option<&JsonValue> {
        let repr_obj = representation.as_object()?;
        self.entities.iter().find(|entity| {
            let entity_obj = match entity.as_object() {
                Some(o) => o,
                None => return false,
            };
            repr_obj
                .iter()
                .all(|(k, v)| entity_obj.get(k).is_some_and(|ev| is_a_match(ev, v)))
        })
    }
}

/// "Entity contains at least every field of the representation" check.
fn is_a_match(entity: &JsonValue, repr: &JsonValue) -> bool {
    match (entity, repr) {
        (JsonValue::String(a), JsonValue::String(b)) => a == b,
        (JsonValue::Number(a), JsonValue::Number(b)) => a == b,
        (JsonValue::Bool(a), JsonValue::Bool(b)) => a == b,
        (JsonValue::Null, JsonValue::Null) => true,
        (JsonValue::Object(eo), JsonValue::Object(ro)) => ro
            .iter()
            .all(|(k, v)| eo.get(k).is_some_and(|ev| is_a_match(ev, v))),
        (JsonValue::Array(ea), JsonValue::Array(ra)) => {
            ea.len() == ra.len() && ea.iter().all(|av| ra.iter().any(|bv| is_a_match(av, bv)))
        }
        _ => false,
    }
}

fn coerce_value(value: &Value<'_, String>, variables: &JsonValue) -> JsonValue {
    match value {
        Value::Variable(name) => variables
            .get(name.as_str())
            .cloned()
            .unwrap_or(JsonValue::Null),
        Value::Int(i) => json!(i.as_i64()),
        Value::Float(f) => json!(*f),
        Value::String(s) => JsonValue::String(s.clone()),
        Value::Boolean(b) => JsonValue::Bool(*b),
        Value::Null => JsonValue::Null,
        Value::Enum(e) => JsonValue::String(e.clone()),
        Value::List(items) => {
            JsonValue::Array(items.iter().map(|v| coerce_value(v, variables)).collect())
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), coerce_value(v, variables));
            }
            JsonValue::Object(out)
        }
    }
}

fn error_response(message: impl Into<String>) -> JsonValue {
    json!({
        "errors": [{
            "message": message.into(),
            "extensions": { "code": "MOCK_SUBGRAPH_ERROR" }
        }]
    })
}
