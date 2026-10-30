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
//!   objects/lists). Fields with arguments first look up a key such as
//!   `price(currency: "EUR")`; `@skip` and `@include` are applied.
//! * Handles federation's `_entities` resolver by matching each
//!   `representations[i]` against the configured `entities: [..]` list using
//!   the rule "entity has at least all of representation's fields".
//!
//! This is intentionally a small, schema-less executor: it knows nothing
//! about the subgraph schema, just walks the query AST against pre-baked
//! JSON. That is enough to feed the demand-control fixtures.
//!
//! `strict_mock_subgraphs` is the same mock, but `_entities` checks what the
//! router asks for. An entity lookup fails the request when, after `@skip`,
//! `@include` and type conditions, it selects no field (a request that
//! shouldn't have gone out), or a field the entity's JSON doesn't have (a
//! field asked for on the wrong type). It also counts lookups per entity.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use graphql_tools::parser::{
    parse_query,
    query::{
        Definition, Directive, Field, FragmentDefinition, OperationDefinition, Selection,
        SelectionSet, Value,
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
    mock_subgraphs_with(config, None)
}

/// How many times each entity was looked up, by `subgraph/Type:id`.
#[derive(Clone, Default)]
pub struct Lookups(Arc<Mutex<BTreeMap<String, usize>>>);

impl Lookups {
    /// The counts so far, and starts over.
    pub fn take(&self) -> BTreeMap<String, usize> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }

    fn record(&self, subgraph: &str, representation: &JsonValue) {
        let typename = representation["__typename"].as_str().unwrap_or("?");
        let key = match &representation["id"] {
            JsonValue::String(id) => format!("{subgraph}/{typename}:{id}"),
            _ => format!("{subgraph}/{representation}"),
        };
        *self.0.lock().unwrap().entry(key).or_default() += 1;
    }
}

/// Like `mock_subgraphs`, with the `_entities` checks described at the top.
pub fn strict_mock_subgraphs(
    config: JsonValue,
) -> (
    impl Fn(RequestLike) -> Option<ResponseLike> + Send + Sync + 'static,
    Lookups,
) {
    let lookups = Lookups::default();
    (mock_subgraphs_with(config, Some(lookups.clone())), lookups)
}

fn mock_subgraphs_with(
    config: JsonValue,
    strict: Option<Lookups>,
) -> impl Fn(RequestLike) -> Option<ResponseLike> + Send + Sync + 'static {
    let configs: BTreeMap<String, SubgraphMock> = config
        .as_object()
        .expect("mock_subgraphs: top-level must be an object")
        .iter()
        .map(|(name, value)| {
            let mut mock = SubgraphMock::from_json(value);
            mock.name = name.clone();
            mock.strict = strict.clone();
            (name.clone(), mock)
        })
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

#[derive(Clone)]
struct SubgraphMock {
    name: String,
    query: JsonValue,
    mutation: Option<JsonValue>,
    entities: Vec<JsonValue>,
    strict: Option<Lookups>,
}

impl SubgraphMock {
    fn from_json(value: &JsonValue) -> Self {
        Self {
            name: String::new(),
            strict: None,
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

        if let Some(lookups) = &self.strict {
            if let Err(message) =
                self.check_entity_calls(selection_set, &variables, &fragments, lookups)
            {
                return error_response(message);
            }
        }

        let data = self.resolve_selection_set(selection_set, root, &variables, &fragments);
        let errors = self.entity_errors(selection_set, &variables);

        if errors.is_empty() {
            json!({ "data": data })
        } else {
            json!({ "data": data, "errors": errors })
        }
    }

    /// An entity with `"__error": "message"` resolves to null, with that error at its place.
    fn entity_errors(
        &self,
        selection_set: &SelectionSet<'_, String>,
        variables: &JsonValue,
    ) -> Vec<JsonValue> {
        let mut errors = Vec::new();
        for selection in &selection_set.items {
            let Selection::Field(field) = selection else {
                continue;
            };
            if field.name != "_entities" || !is_included(&field.directives, variables) {
                continue;
            }
            let key = field.alias.as_ref().unwrap_or(&field.name);
            let representations = field
                .arguments
                .iter()
                .find(|(name, _)| name == "representations")
                .map(|(_, value)| coerce_value(value, variables))
                .unwrap_or(JsonValue::Null);
            for (index, representation) in
                representations.as_array().into_iter().flatten().enumerate()
            {
                if let Some(message) = self
                    .find_entity(representation)
                    .and_then(|entity| entity["__error"].as_str())
                {
                    errors.push(json!({ "message": message, "path": [key, index] }));
                }
            }
        }
        errors
    }

    /// The strict checks, for every `_entities` field that isn't skipped.
    fn check_entity_calls(
        &self,
        selection_set: &SelectionSet<'_, String>,
        variables: &JsonValue,
        fragments: &BTreeMap<String, FragmentDefinition<'_, String>>,
        lookups: &Lookups,
    ) -> Result<(), String> {
        for selection in &selection_set.items {
            let Selection::Field(field) = selection else {
                continue;
            };
            if field.name != "_entities" || !is_included(&field.directives, variables) {
                continue;
            }
            let representations = field
                .arguments
                .iter()
                .find(|(name, _)| name == "representations")
                .map(|(_, value)| coerce_value(value, variables))
                .unwrap_or(JsonValue::Null);
            for representation in representations.as_array().into_iter().flatten() {
                lookups.record(&self.name, representation);
                let Some(entity) = self.find_entity(representation) else {
                    continue;
                };
                let mut asked = Vec::new();
                active_fields(
                    &field.selection_set,
                    entity,
                    variables,
                    fragments,
                    &mut asked,
                );
                if !asked.iter().any(|name| name != "__typename") {
                    return Err(format!(
                        "{}: {representation} was looked up for no field",
                        self.name
                    ));
                }
                if let Some(missing) = asked
                    .iter()
                    .find(|name| *name != "__typename" && entity.get(name.as_str()).is_none())
                {
                    return Err(format!(
                        "{}: {representation} has no `{missing}`",
                        self.name
                    ));
                }
            }
        }
        Ok(())
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
            Selection::Field(field) if !is_included(&field.directives, variables) => {}
            Selection::InlineFragment(fragment)
                if !is_included(&fragment.directives, variables) => {}
            Selection::FragmentSpread(spread) if !is_included(&spread.directives, variables) => {}
            Selection::Field(field) => {
                let key = field.alias.clone().unwrap_or_else(|| field.name.clone());
                let value = self.resolve_field(field, parent, variables, fragments);
                out.insert(key, value);
            }
            Selection::InlineFragment(frag) => {
                // strict mocks also skip fragments of other types, like a real subgraph
                if let (Some(_), Some(graphql_tools::parser::query::TypeCondition::On(name))) =
                    (&self.strict, &frag.type_condition)
                {
                    if parent["__typename"].as_str().is_some_and(|t| t != name) {
                        return;
                    }
                }
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
                    Some(entity) if entity.get("__error").is_some() => JsonValue::Null,
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

        let raw = arguments_key(field, variables)
            .and_then(|key| parent.get(&key))
            .or_else(|| parent.get(field.name.as_str()))
            .cloned();
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

/// The fields `selection_set` asks of `entity`, after `@skip`, `@include` and type conditions.
fn active_fields(
    selection_set: &SelectionSet<'_, String>,
    entity: &JsonValue,
    variables: &JsonValue,
    fragments: &BTreeMap<String, FragmentDefinition<'_, String>>,
    out: &mut Vec<String>,
) {
    let typename = entity["__typename"].as_str();
    let applies = |type_condition: Option<&str>| {
        type_condition.is_none_or(|type_condition| typename.is_none_or(|t| t == type_condition))
    };
    for selection in &selection_set.items {
        match selection {
            Selection::Field(field) if is_included(&field.directives, variables) => {
                out.push(field.name.clone());
            }
            Selection::InlineFragment(fragment)
                if is_included(&fragment.directives, variables)
                    && applies(fragment.type_condition.as_ref().map(|condition| {
                        let graphql_tools::parser::query::TypeCondition::On(name) = condition;
                        name.as_str()
                    })) =>
            {
                active_fields(&fragment.selection_set, entity, variables, fragments, out);
            }
            Selection::FragmentSpread(spread) if is_included(&spread.directives, variables) => {
                if let Some(fragment) = fragments.get(&spread.fragment_name) {
                    let graphql_tools::parser::query::TypeCondition::On(name) =
                        &fragment.type_condition;
                    if applies(Some(name)) {
                        active_fields(&fragment.selection_set, entity, variables, fragments, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Applies `@skip` and `@include` like a real subgraph.
fn is_included(directives: &[Directive<'_, String>], variables: &JsonValue) -> bool {
    directives.iter().all(|directive| {
        let condition = directive
            .arguments
            .iter()
            .find(|(name, _)| name == "if")
            .map(|(_, value)| coerce_value(value, variables));
        !matches!(
            (directive.name.as_str(), condition),
            ("skip", Some(JsonValue::Bool(true))) | ("include", Some(JsonValue::Bool(false)))
        )
    })
}

/// Renders an argument-bearing field as `name(arg: value, ...)` for keyed mock lookup.
fn arguments_key(field: &Field<'_, String>, variables: &JsonValue) -> Option<String> {
    if field.arguments.is_empty() {
        return None;
    }
    let arguments: Vec<String> = field
        .arguments
        .iter()
        .map(|(name, value)| format!("{name}: {}", coerce_value(value, variables)))
        .collect();
    Some(format!("{}({})", field.name, arguments.join(", ")))
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
