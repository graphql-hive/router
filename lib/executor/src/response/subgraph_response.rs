use core::fmt;
use std::sync::Arc;

use bumpalo::Bump;
use bytes::Bytes;
use hive_router_query_planner::planner::response_shape::ResponseShape;
use http::{HeaderMap, StatusCode};
use serde::{
    de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor},
    Deserialize,
};
use sonic_rs::LazyValue;

use crate::{
    executors::error::SubgraphExecutorError,
    response::{arena::ResponseArena, graphql_error::GraphQLError, value::Value},
};

#[derive(Debug, Default)]
pub struct SubgraphResponse<'a> {
    pub data: Value<'a>,
    pub errors: Option<Vec<GraphQLError>>,
    /// Arbitrary subgraph JSON with no shape known in advance, so it is not a response
    /// `Value` — see `extensions::aggregator`.
    pub extensions: Option<sonic_rs::Value>,
    pub headers: Option<Arc<HeaderMap>>,
    pub bytes: Option<Bytes>,
    /// The arena `data` was allocated in. Nothing reads it; it is carried so the values stay
    /// valid, and handed to `ResponsesStorage` when the response is merged.
    pub arena: Option<ResponseArena>,
    pub status: Option<StatusCode>,
}

impl SubgraphResponse<'_> {
    pub fn append_error(&mut self, error: GraphQLError) {
        if let Some(errors) = &mut self.errors {
            errors.push(error);
        } else {
            self.errors = Some(vec![error]);
        }
    }
}

static EMPTY_RESPONSE_SHAPE: ResponseShape = ResponseShape {
    fields: Vec::new(),
    raw: false,
    inert: true,
};

struct SubgraphResponseSeed<'a, 'de> {
    response_shape: &'a ResponseShape,
    arena: &'de Bump,
}

impl<'a, 'de> DeserializeSeed<'de> for SubgraphResponseSeed<'a, 'de> {
    type Value = SubgraphResponse<'de>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_subgraph_response_with_shape(deserializer, self.response_shape, self.arena)
    }
}

fn deserialize_subgraph_response_with_shape<'a, 'de, D>(
    deserializer: D,
    response_shape: &'a ResponseShape,
    arena: &'de Bump,
) -> Result<SubgraphResponse<'de>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_map(SubgraphResponseVisitor {
        response_shape,
        arena,
    })
}

struct SubgraphResponseVisitor<'a, 'de> {
    response_shape: &'a ResponseShape,
    arena: &'de Bump,
}

impl<'a, 'de> Visitor<'de> for SubgraphResponseVisitor<'a, 'de> {
    type Value = SubgraphResponse<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a GraphQL response object with data, errors, and extensions fields")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut data = None;
        let mut errors = None;
        let mut extensions = None;

        while let Some(key) = map.next_key::<&str>()? {
            match key {
                "data" => {
                    if data.is_some() {
                        return Err(de::Error::duplicate_field("data"));
                    }
                    data = Some(map.next_value_seed(ValueSeed {
                        arena: self.arena,
                        response_shape: self.response_shape,
                    })?);
                }
                "errors" => {
                    if errors.is_some() {
                        return Err(de::Error::duplicate_field("errors"));
                    }
                    errors = Some(map.next_value()?);
                }
                "extensions" => {
                    if extensions.is_some() {
                        return Err(de::Error::duplicate_field("extensions"));
                    }
                    // Extensions intentionally stay on the structured path.
                    extensions = Some(map.next_value()?);
                }
                _ => {
                    let _ = map.next_value::<de::IgnoredAny>()?;
                }
            }
        }

        Ok(SubgraphResponse {
            data: data.unwrap_or(Value::Null),
            errors,
            extensions,
            headers: None,
            bytes: None,
            // Filled in by the caller that owns the arena this parsed into.
            arena: None,
            status: None,
        })
    }
}

#[derive(Clone, Copy)]
struct ValueSeed<'a, 'de> {
    response_shape: &'a ResponseShape,
    arena: &'de Bump,
}

impl<'a, 'de> DeserializeSeed<'de> for ValueSeed<'a, 'de> {
    type Value = Value<'de>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_value_with_shape(deserializer, self.response_shape, self.arena)
    }
}

fn deserialize_value_with_shape<'a, 'de, D>(
    deserializer: D,
    response_shape: &'a ResponseShape,
    arena: &'de Bump,
) -> Result<Value<'de>, D::Error>
where
    D: Deserializer<'de>,
{
    if response_shape.raw {
        let raw = LazyValue::deserialize(deserializer)?;
        // Borrowed for any slice-backed input, which is all this deserializer ever sees
        // (`Deserializer::from_slice`). The owned case only arises for `FastStr`-backed
        // input; erroring out is better than silently escaping raw JSON as a string.
        let raw = match raw.as_raw_cow() {
            std::borrow::Cow::Borrowed(raw) => raw,
            std::borrow::Cow::Owned(_) => {
                return Err(de::Error::custom(
                    "raw JSON passthrough requires slice-backed input",
                ))
            }
        };
        // A passthrough `null` still has to be a real `Value::Null`: projection propagates
        // it through non-null positions, and merge treats a null source as a no-op. A
        // `RawJson("null")` would silently defeat both.
        if raw == "null" {
            return Ok(Value::Null);
        }
        return Ok(Value::RawJson(raw));
    }

    deserializer.deserialize_any(ShapedValueVisitor {
        response_shape,
        arena,
    })
}

struct ShapedValueVisitor<'a, 'de> {
    response_shape: &'a ResponseShape,
    arena: &'de Bump,
}

impl<'a, 'de> Visitor<'de> for ShapedValueVisitor<'a, 'de> {
    type Value = Value<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any valid JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::I64(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::U64(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(Value::F64(value))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value))
    }

    /// Only reached for a string that had to be unescaped, so it cannot borrow the buffer —
    /// it goes in the arena instead, and is a plain `&str` like every other string here.
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(self.arena.alloc_str(value)))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(self.arena.alloc_str(&value)))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        // Lists are transparent: every element sits at the same response position.
        //
        // sonic gives no `size_hint`, so the vector grows from empty. The shape used to
        // remember the length last seen here and pre-size from it, which measured 2-8% of
        // deserialization on list-heavy payloads — but it was a mutable counter on a cached
        // plan, shared by every request and every user that plan serves, and that is a poor
        // trade for a fraction of a percent end to end.
        let mut elements =
            bumpalo::collections::Vec::with_capacity_in(seq.size_hint().unwrap_or(0), self.arena);
        while let Some(elem) = seq.next_element_seed(ValueSeed {
            response_shape: self.response_shape,
            arena: self.arena,
        })? {
            elements.push(elem);
        }
        Ok(Value::Array(elements.into_bump_slice_mut()))
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        // Exactly one slot per field in the shape, so the boxed slice below needs no
        // reallocation. Anything the subgraph omits stays `Absent`, which is distinct from a
        // `null` it actually answered — `requires` leaves the first out of a representation
        // and sends the second.
        let slots: &'de mut [Value<'de>] = self
            .arena
            .alloc_slice_fill_default(self.response_shape.fields.len());
        // Keys the plan never asked for are parsed and thrown away here, so the loop keeps a
        // single call site (see below).
        let mut discard = Value::Absent;

        // Subgraphs answer in the order the fetch asked, so the cursor lands on the right
        // field with one string compare; `resolve_slot` falls back to a scan when it does not.
        let mut cursor = 0usize;
        while let Some(key) = map.next_key::<&'de str>()? {
            // One call site on purpose, including for unknown keys. Branching here between
            // `next_value_seed` and `next_value`/`IgnoredAny` instantiates two fully-inlined
            // parse paths inside the loop, which measured ~25% slower on key-dense payloads;
            // routing everything through the seed keeps the loop body small and lets the
            // branch live one level down.
            let (target, child) = match self.response_shape.resolve_slot(&mut cursor, key) {
                Some(slot) => (&mut slots[slot], &self.response_shape.fields[slot].shape),
                None => (&mut discard, &EMPTY_RESPONSE_SHAPE),
            };
            *target = map.next_value_seed(ValueSeed {
                response_shape: child,
                arena: self.arena,
            })?;
        }

        Ok(Value::Object(slots))
    }
}

impl<'a> SubgraphResponse<'a> {
    /// Parses a subgraph response.
    ///
    /// `response_shape` is not optional in practice: the response tree is slot-addressed, so
    /// a shape is the only thing that says where a field's value goes. Passing `None` — or a
    /// shape that does not describe the payload — yields empty objects rather than an error,
    /// because a key the plan never asked for is legitimately ignored and there is no way to
    /// tell that case apart from a missing shape.
    ///
    /// Every fetch carries a `ResponseShape`, so production callers always have one. A client
    /// that has no shape (a test harness, or anything pointing this at a server it did not
    /// plan for) should read `bytes` instead of `data`.
    ///
    /// ponytail: `Option` is kept only because `WsClient` doubles as a generic client. Making
    /// it required, with a named constant for the unshaped case, would turn a silent
    /// data-discard into something the caller has to acknowledge.
    pub fn deserialize_from_bytes<'de>(
        bytes: Bytes,
        response_shape: Option<&ResponseShape>,
    ) -> Result<SubgraphResponse<'de>, SubgraphExecutorError> {
        let bytes_ref: &[u8] = &bytes;

        // SAFETY: The byte slice `bytes_ref` is transmuted to `'static`.
        // This is safe because the returned `SubgraphResponse` stores the `bytes` (Arc-backed
        // reference-counted buffer) in its `bytes` field, keeping the underlying data alive as
        // long as the `SubgraphResponse` does. The `data` field of `SubgraphResponse` contains
        // values that borrow from this buffer, creating a self-referential struct, which is why
        // `unsafe` is required.
        let bytes_ref: &'de [u8] = unsafe { std::mem::transmute(bytes_ref) };
        let mut deserializer = sonic_rs::Deserializer::from_slice(bytes_ref);

        // The tree is allocated here and the arena travels with it, on the same reasoning as
        // the bytes above: see `ResponseArena`.
        let arena = ResponseArena::new();

        SubgraphResponseSeed {
            response_shape: response_shape.unwrap_or(&EMPTY_RESPONSE_SHAPE),
            arena: arena.borrow_unbounded(),
        }
        .deserialize(&mut deserializer)
        .map_err(|e| SubgraphExecutorError::ResponseDeserializationFailure(e, None))
        .and_then(|mut resp: SubgraphResponse<'de>| {
            deserializer
                .end()
                .map_err(|e| SubgraphExecutorError::ResponseDeserializationFailure(e, None))?;
            resp.bytes = Some(bytes);
            resp.arena = Some(arena);

            if resp.data.is_null() && resp.errors.is_none() {
                return Err(SubgraphExecutorError::MalformedResponse(None));
            }

            Ok(resp)
        })
    }
}

impl SubgraphResponse<'static> {
    /// Parses a bare `data` object against a shape, for tests and benchmarks.
    ///
    /// The returned response owns the bytes its values borrow from, so it has to outlive any
    /// use of `data`.
    pub fn parse_data_with_shape(json: &str, shape: &ResponseShape) -> SubgraphResponse<'static> {
        let envelope = format!(r#"{{"data":{json}}}"#);
        Self::deserialize_from_bytes(Bytes::from(envelope), Some(shape))
            .expect("valid data payload")
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use hive_router_query_planner::planner::response_shape::ResponseShape;
    use hive_router_query_planner::{
        graph::PlannerOverrideContext,
        planner::{plan_nodes::PlanNode, Planner},
        utils::{
            cancellation::CancellationToken,
            parsing::{parse_operation, parse_schema},
        },
    };

    use sonic_rs::JsonValueTrait;

    use super::SubgraphResponse;
    use crate::response::value::Value;

    #[test]
    fn deserialize_response_without_data_with_errors_with_extensions() {
        let json_response = r#"
        {
            "errors": [
                {
                    "message": "Random error from subgraph",
                    "extensions":{
                        "statusCode": 400
                    }
                }
            ]
        }"#;

        let response = SubgraphResponse::deserialize_from_bytes(
            Bytes::from(json_response),
            Some(&super::EMPTY_RESPONSE_SHAPE),
        )
        .expect("Failed to deserialize");

        assert!(response.data.is_null());
        let errors = response.errors.as_ref().unwrap();
        insta::assert_snapshot!(sonic_rs::to_string_pretty(&errors).unwrap(), @r###"
        [
          {
            "message": "Random error from subgraph",
            "extensions": {
              "statusCode": 400
            }
          }
        ]"###);
    }

    #[test]
    fn deserializes_custom_scalar_data_field_as_raw_json() {
        let mut paths = ResponseShape::default();
        paths.insert_raw_path(["labels"]);

        let response = super::SubgraphResponse::deserialize_from_bytes(
            Bytes::from_static(br#"{"data":{"labels":{"generic.learnMore.button\t":"Learn more"}},"extensions":{"statusCode":200}}"#),
            Some(&paths),
        )
        .unwrap();

        assert!(matches!(response.data.slot(0), Some(Value::RawJson(_))));

        let extensions = response.extensions.unwrap();
        assert!(extensions.is_object(), "extensions stay on sonic_rs::Value");
    }

    #[test]
    fn deserializes_mixed_sibling_paths_with_only_marked_path_as_raw_json() {
        let mut shape = ResponseShape::default();
        shape.insert_raw_path(["custom"]);
        // A sibling the shape describes but does not mark: parsed structurally.
        shape.insert_raw_path(["plain", "message"]);

        let response = SubgraphResponse::deserialize_from_bytes(
            Bytes::from_static(
                br#"{
                    "data": {
                        "custom": {
                            "generic.learnMore.button\t": "Learn more"
                        },
                        "plain": {
                            "message": "hello"
                        }
                    }
                }"#,
            ),
            Some(&shape),
        )
        .unwrap();

        let custom = response
            .data
            .slot(0)
            .and_then(Value::as_raw_json)
            .expect("custom path should deserialize as raw json");
        assert!(custom.contains("\"generic.learnMore.button\\t\""));
        assert!(custom.contains("\"Learn more\""));

        let plain = response.data.slot(1).expect("plain stays structured");
        assert_eq!(
            plain.slot(0).and_then(Value::as_raw_json),
            Some(r#""hello""#)
        );
    }

    /// Looks a field up by name through the shape, since the data itself carries no keys.
    fn field_by<'a>(data: &'a Value<'a>, shape: &ResponseShape, key: &str) -> &'a Value<'a> {
        let slot = shape
            .slot_of(key)
            .unwrap_or_else(|| panic!("missing field {key} in shape"));
        data.slot(slot)
            .unwrap_or_else(|| panic!("missing slot {slot} for {key}"))
    }

    fn deserialize(json: &'static str, shape: &ResponseShape) -> SubgraphResponse<'static> {
        SubgraphResponse::deserialize_from_bytes(Bytes::from_static(json.as_bytes()), Some(shape))
            .expect("deserialize")
    }

    #[test]
    fn passthrough_null_becomes_a_real_null() {
        // `RawJson("null")` would defeat null propagation in projection and make merge treat
        // the field as present, so the deserializer has to normalize it back.
        let mut shape = ResponseShape::default();
        shape.insert_raw_path(["user", "nickname"]);

        let response = deserialize(r#"{"data":{"user":{"nickname":null}}}"#, &shape);
        let nickname = response
            .data
            .slot(0)
            .and_then(|user| user.slot(0))
            .expect("nickname slot");
        assert!(
            nickname.is_null(),
            "expected Value::Null, got {nickname:?} — null propagation would break"
        );
    }

    #[test]
    fn passthrough_covers_leaf_lists_verbatim() {
        let mut shape = ResponseShape::default();
        shape.insert_raw_path(["product", "tags"]);

        shape.insert_raw_path(["product", "id"]);

        let response = deserialize(
            r#"{"data":{"product":{"tags":["a","b\t","c"],"id":"p1"}}}"#,
            &shape,
        );
        let product = response.data.slot(0).expect("product slot");
        assert_eq!(
            product.slot(0).and_then(Value::as_raw_json),
            Some(r#"["a","b\t","c"]"#),
            "a leaf list should survive as untouched bytes, escapes included"
        );
        assert_eq!(
            product.slot(1).and_then(Value::as_raw_json),
            Some(r#""p1""#)
        );
    }

    #[test]
    fn out_of_order_keys_still_resolve() {
        // The cursor assumes the subgraph answers in fetch order; the scan has to cover the
        // case where it does not.
        let mut shape = ResponseShape::default();
        shape.insert_raw_path(["a"]);
        shape.insert_raw_path(["b"]);
        shape.insert_raw_path(["c"]);

        let response = deserialize(r#"{"data":{"c":3,"a":1,"b":2}}"#, &shape);
        for (slot, expected) in [(0, "1"), (1, "2"), (2, "3")] {
            assert_eq!(
                response.data.slot(slot).and_then(Value::as_raw_json),
                Some(expected),
                "slot {slot} should hold {expected} regardless of arrival order"
            );
        }
    }

    #[test]
    fn unknown_keys_fall_back_to_structured_parsing() {
        let mut shape = ResponseShape::default();
        shape.insert_raw_path(["known"]);

        let response = deserialize(r#"{"data":{"known":"k","surprise":{"n":1}}}"#, &shape);
        assert_eq!(
            response.data.slot(0).and_then(Value::as_raw_json),
            Some(r#""k""#)
        );
        // A key the plan never asked for has no slot, so it is simply not kept.
        assert_eq!(response.data.as_object().map(<[_]>::len), Some(1));
    }

    #[test]
    fn custom_and_builtin_scalar_sharing_response_path() {
        let schema = parse_schema(
            r#"
            schema
              @link(url: "https://specs.apollo.dev/link/v1.0")
              @link(url: "https://specs.apollo.dev/join/v0.3", for: EXECUTION) {
              query: Query
            }

            directive @join__enumValue(graph: join__Graph!) repeatable on ENUM_VALUE
            directive @join__field(
              graph: join__Graph
              requires: join__FieldSet
              provides: join__FieldSet
              type: String
              external: Boolean
              override: String
              usedOverridden: Boolean
            ) repeatable on FIELD_DEFINITION | INPUT_FIELD_DEFINITION
            directive @join__graph(name: String!, url: String!) on ENUM_VALUE
            directive @join__implements(
              graph: join__Graph!
              interface: String!
            ) repeatable on OBJECT | INTERFACE
            directive @join__type(
              graph: join__Graph!
              key: join__FieldSet
              extension: Boolean! = false
              resolvable: Boolean! = true
              isInterfaceObject: Boolean! = false
            ) repeatable on OBJECT | INTERFACE | UNION | ENUM | INPUT_OBJECT | SCALAR
            directive @join__unionMember(
              graph: join__Graph!
              member: String!
            ) repeatable on UNION
            directive @link(
              url: String
              as: String
              for: link__Purpose
              import: [link__Import]
            ) repeatable on SCHEMA

            scalar join__FieldSet
            scalar link__Import

            enum join__Graph {
              TEST @join__graph(name: "test", url: "http://example.com/graphql")
            }

            enum link__Purpose {
              SECURITY
              EXECUTION
            }

            scalar JSONBlob @join__type(graph: TEST)

            interface InterfaceThing @join__type(graph: TEST) {
              id: ID! @join__field(graph: TEST)
            }

            type JsonInterfaceThing implements InterfaceThing
              @join__type(graph: TEST)
              @join__implements(graph: TEST, interface: "InterfaceThing") {
              id: ID! @join__field(graph: TEST)
              meta: JSONBlob @join__field(graph: TEST)
            }

            type StringInterfaceThing implements InterfaceThing
              @join__type(graph: TEST)
              @join__implements(graph: TEST, interface: "InterfaceThing") {
              id: ID! @join__field(graph: TEST)
              meta: String @join__field(graph: TEST)
            }

            type JsonUnionThing @join__type(graph: TEST) {
              meta: JSONBlob @join__field(graph: TEST)
            }

            type StringUnionThing @join__type(graph: TEST) {
              meta: String @join__field(graph: TEST)
            }

            union UnionThing
              @join__type(graph: TEST)
              @join__unionMember(graph: TEST, member: "JsonUnionThing")
              @join__unionMember(graph: TEST, member: "StringUnionThing") = JsonUnionThing | StringUnionThing

            type Query @join__type(graph: TEST) {
              interfaceThing: InterfaceThing @join__field(graph: TEST)
              unionThing: UnionThing @join__field(graph: TEST)
            }
            "#,
        );
        let planner = Planner::new_from_supergraph(&schema, Default::default()).expect("planner");
        let operation = parse_operation(
            r#"
            {
              interfaceThing {
                __typename
                ... on JsonInterfaceThing {
                  meta
                }
                ... on StringInterfaceThing {
                  meta
                }
              }
              unionThing {
                __typename
                ... on JsonUnionThing {
                  meta
                }
                ... on StringUnionThing {
                  meta
                }
              }
            }
            "#,
        );
        let normalized = hive_router_query_planner::ast::normalization::normalize_operation(
            &planner.supergraph,
            &operation,
            None,
        )
        .expect("normalized operation");
        let plan = planner
            .plan_from_normalized_operation(
                normalized.executable_operation(),
                PlannerOverrideContext::default(),
                &CancellationToken::new(),
            )
            .expect("query plan");

        let response_shape =
            find_fetch_response_shape(plan.node.as_ref(), "test").expect("response shape");

        // The planner aliases the two branches apart, so they get independent decisions:
        // the custom scalar is a passthrough, the plain `String` behind the alias is not.
        let interface = shape_field(response_shape, "interfaceThing");
        assert!(shape_field(interface, "meta").raw);
        assert!(
            !shape_field(interface, "_internal_qp_alias_0").raw,
            "a single builtin scalar is not worth a passthrough"
        );
        assert!(
            !shape_field(interface, "__typename").raw,
            "__typename is read back as a &str and must stay structured"
        );

        let response = SubgraphResponse::deserialize_from_bytes(
            Bytes::from_static(
                br#"{
                    "data": {
                        "interfaceThing": {
                            "__typename": "StringInterfaceThing",
                            "_internal_qp_alias_0": "interface string"
                        },
                        "unionThing": {
                            "__typename": "JsonUnionThing",
                            "meta": {
                                "union.key\t": "union value"
                            }
                        }
                    }
                }"#,
            ),
            Some(response_shape),
        )
        .unwrap();

        let interface_thing = field_by(&response.data, response_shape, "interfaceThing");
        assert_eq!(
            field_by(interface_thing, interface, "_internal_qp_alias_0").as_str(),
            Some("interface string")
        );
        assert_eq!(
            field_by(interface_thing, interface, "__typename").as_str(),
            Some("StringInterfaceThing")
        );

        let union_shape = shape_field(response_shape, "unionThing");
        let union_meta = field_by(
            field_by(&response.data, response_shape, "unionThing"),
            union_shape,
            "meta",
        )
        .as_raw_json()
        .expect("json union branch should deserialize as raw json");
        assert!(union_meta.contains("union.key\\t"));
    }

    fn shape_field<'a>(shape: &'a ResponseShape, key: &str) -> &'a ResponseShape {
        &shape
            .fields
            .iter()
            .find(|f| f.key == key)
            .unwrap_or_else(|| panic!("missing shape field {key} in {shape:?}"))
            .shape
    }

    fn find_fetch_response_shape<'a>(
        node: Option<&'a PlanNode>,
        service_name: &str,
    ) -> Option<&'a ResponseShape> {
        match node? {
            PlanNode::Fetch(fetch) if fetch.service_name == service_name => {
                Some(&fetch.response_shape)
            }
            PlanNode::BatchFetch(fetch) if fetch.service_name == service_name => {
                Some(&fetch.response_shape)
            }
            PlanNode::Sequence(sequence) => sequence
                .nodes
                .iter()
                .find_map(|node| find_fetch_response_shape(Some(node), service_name)),
            PlanNode::Parallel(parallel) => parallel
                .nodes
                .iter()
                .find_map(|node| find_fetch_response_shape(Some(node), service_name)),
            PlanNode::Flatten(flatten) => {
                find_fetch_response_shape(Some(&flatten.node), service_name)
            }
            PlanNode::Condition(condition) => find_fetch_response_shape(
                condition
                    .if_clause
                    .as_deref()
                    .or(condition.else_clause.as_deref()),
                service_name,
            ),
            _ => None,
        }
    }
}
