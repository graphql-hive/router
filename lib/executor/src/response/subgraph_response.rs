use core::fmt;
use std::sync::Arc;

use bytes::Bytes;
use hive_router_query_planner::planner::plan_nodes::CustomScalarPaths;
use http::{HeaderMap, StatusCode};
use serde::{
    de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor},
    Deserialize,
};
use sonic_rs::LazyValue;

use crate::{
    executors::error::SubgraphExecutorError,
    response::{graphql_error::GraphQLError, value::Value},
};

#[derive(Debug, Default)]
pub struct SubgraphResponse<'a> {
    pub data: Value<'a>,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<Value<'a>>,
    pub headers: Option<Arc<HeaderMap>>,
    pub bytes: Option<Bytes>,
    pub status: Option<StatusCode>,
}

impl<'de> de::Deserialize<'de> for SubgraphResponse<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_subgraph_response_with_paths(deserializer, &EMPTY_CUSTOM_SCALAR_PATHS)
    }
}

static EMPTY_CUSTOM_SCALAR_PATHS: CustomScalarPaths = CustomScalarPaths {
    children: std::collections::BTreeMap::new(),
    terminal: false,
};

struct SubgraphResponseSeed<'a> {
    custom_scalar_paths: &'a CustomScalarPaths,
}

impl<'a, 'de> DeserializeSeed<'de> for SubgraphResponseSeed<'a> {
    type Value = SubgraphResponse<'de>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_subgraph_response_with_paths(deserializer, self.custom_scalar_paths)
    }
}

fn deserialize_subgraph_response_with_paths<'a, 'de, D>(
    deserializer: D,
    custom_scalar_paths: &'a CustomScalarPaths,
) -> Result<SubgraphResponse<'de>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_map(SubgraphResponseVisitor {
        custom_scalar_paths,
    })
}

struct SubgraphResponseVisitor<'a> {
    custom_scalar_paths: &'a CustomScalarPaths,
}

impl<'a, 'de> Visitor<'de> for SubgraphResponseVisitor<'a> {
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
                        custom_scalar_paths: self.custom_scalar_paths,
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
            status: None,
        })
    }
}

#[derive(Clone, Copy)]
struct ValueSeed<'a> {
    custom_scalar_paths: &'a CustomScalarPaths,
}

impl<'a, 'de> DeserializeSeed<'de> for ValueSeed<'a> {
    type Value = Value<'de>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_value_with_paths(deserializer, self.custom_scalar_paths)
    }
}

fn deserialize_value_with_paths<'a, 'de, D>(
    deserializer: D,
    custom_scalar_paths: &'a CustomScalarPaths,
) -> Result<Value<'de>, D::Error>
where
    D: Deserializer<'de>,
{
    if custom_scalar_paths.is_empty() {
        return Value::deserialize(deserializer);
    }

    if custom_scalar_paths.terminal {
        let raw = LazyValue::deserialize(deserializer)?;
        return Ok(Value::RawJson(raw.as_raw_cow()));
    }

    deserializer.deserialize_any(PathAwareValueVisitor {
        custom_scalar_paths,
    })
}

struct PathAwareValueVisitor<'a> {
    custom_scalar_paths: &'a CustomScalarPaths,
}

impl<'a, 'de> Visitor<'de> for PathAwareValueVisitor<'a> {
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
        Ok(Value::String(value.into()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned().into()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.into()))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut elements = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(elem) = seq.next_element_seed(ValueSeed {
            custom_scalar_paths: self.custom_scalar_paths,
        })? {
            elements.push(elem);
        }
        Ok(Value::Array(elements))
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some(key) = map.next_key::<&'de str>()? {
            let value = match self.custom_scalar_paths.children.get(key) {
                Some(child_paths) if !child_paths.is_empty() => map.next_value_seed(ValueSeed {
                    custom_scalar_paths: child_paths,
                })?,
                _ => map.next_value()?,
            };
            entries.push((key, value));
        }
        entries.sort_unstable_by_key(|(key, _)| *key);
        Ok(Value::Object(entries))
    }
}

impl<'a> SubgraphResponse<'a> {
    pub fn deserialize_from_bytes(
        bytes: Bytes,
        custom_scalar_paths: Option<&CustomScalarPaths>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        let bytes_ref: &[u8] = &bytes;

        // SAFETY: The byte slice `bytes_ref` is transmuted to `'static`.
        // This is safe because the returned `SubgraphResponse` stores the `bytes` (Arc-backed
        // reference-counted buffer) in its `bytes` field, keeping the underlying data alive as
        // long as the `SubgraphResponse` does. The `data` field of `SubgraphResponse` contains
        // values that borrow from this buffer, creating a self-referential struct, which is why
        // `unsafe` is required.
        let bytes_ref: &'static [u8] = unsafe { std::mem::transmute(bytes_ref) };
        let mut deserializer = sonic_rs::Deserializer::from_slice(bytes_ref);

        SubgraphResponseSeed {
            custom_scalar_paths: custom_scalar_paths.unwrap_or(&EMPTY_CUSTOM_SCALAR_PATHS),
        }
        .deserialize(&mut deserializer)
        .map_err(|e| SubgraphExecutorError::ResponseDeserializationFailure(e, None))
        .and_then(|mut resp: SubgraphResponse<'static>| {
            deserializer
                .end()
                .map_err(|e| SubgraphExecutorError::ResponseDeserializationFailure(e, None))?;
            resp.bytes = Some(bytes);
            Ok(resp)
        })
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use hive_router_query_planner::planner::plan_nodes::CustomScalarPaths;
    use hive_router_query_planner::{
        graph::PlannerOverrideContext,
        planner::{plan_nodes::PlanNode, Planner},
        utils::{
            cancellation::CancellationToken,
            parsing::{parse_operation, parse_schema},
        },
    };

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

        let response: super::SubgraphResponse =
            sonic_rs::from_str(json_response).expect("Failed to deserialize");

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
        let mut paths = CustomScalarPaths::default();
        paths.insert_path(["labels"]);

        let response = super::SubgraphResponse::deserialize_from_bytes(
            Bytes::from_static(br#"{"data":{"labels":{"generic.learnMore.button\t":"Learn more"}},"extensions":{"statusCode":200}}"#),
            Some(&paths),
        )
        .unwrap();

        let data = response.data.as_object().unwrap();
        assert!(matches!(data[0].1, Value::RawJson(_)));

        let extensions = response.extensions.unwrap();
        assert!(matches!(extensions, Value::Object(_)));
    }

    #[test]
    fn deserializes_mixed_sibling_paths_with_only_marked_path_as_raw_json() {
        let mut paths = CustomScalarPaths::default();
        paths.insert_path(["custom"]);

        let response = super::SubgraphResponse::deserialize_from_bytes(
            Bytes::from_static(
                br#"{
                    "data": {
                        "custom": {
                            "generic.learnMore.button\t": "Learn more"
                        },
                        "plain": {
                            "message": "hello",
                            "nested": {
                                "count": 1
                            }
                        }
                    }
                }"#,
            ),
            Some(&paths),
        )
        .unwrap();

        let data = response.data.as_object().unwrap();

        let custom = data
            .iter()
            .find(|(key, _)| *key == "custom")
            .unwrap()
            .1
            .as_raw_json()
            .expect("custom path should deserialize as raw json");
        assert!(custom.contains("\"generic.learnMore.button\\t\""));
        assert!(custom.contains("\"Learn more\""));

        let plain = data
            .iter()
            .find(|(key, _)| *key == "plain")
            .unwrap()
            .1
            .as_object()
            .expect("plain path should stay structured");
        let message = plain
            .iter()
            .find(|(key, _)| *key == "message")
            .unwrap()
            .1
            .as_str();
        assert_eq!(message, Some("hello"));

        let nested = plain
            .iter()
            .find(|(key, _)| *key == "nested")
            .unwrap()
            .1
            .as_object()
            .expect("nested object should stay structured");
        assert!(matches!(nested[0].1, Value::U64(1)));
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
        let planner = Planner::new_from_supergraph(&schema).expect("planner");
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

        insta::assert_snapshot!(format!("{}", plan), @r#"
        QueryPlan {
          Fetch(service: "test") {
            {
              interfaceThing {
                __typename
                ... on JsonInterfaceThing {
                  meta
                }
                ... on StringInterfaceThing {
                  _internal_qp_alias_0: meta
                }
              }
              unionThing {
                __typename
                ... on JsonUnionThing {
                  meta
                }
                ... on StringUnionThing {
                  _internal_qp_alias_0: meta
                }
              }
            }
          },
        },
        "#);

        let custom_scalar_paths = find_fetch_custom_scalar_paths(plan.node.as_ref(), "test")
            .expect("custom scalar paths");

        let interface_paths = custom_scalar_paths
            .children
            .get("interfaceThing")
            .expect("interfaceThing paths");
        assert!(
            interface_paths
                .children
                .get("meta")
                .is_some_and(|path| path.terminal),
            "json interface branch should keep a terminal custom-scalar path"
        );
        assert!(!interface_paths
            .children
            .contains_key("_internal_qp_alias_0"));

        let union_paths = custom_scalar_paths
            .children
            .get("unionThing")
            .expect("unionThing paths");
        assert!(
            union_paths
                .children
                .get("meta")
                .is_some_and(|path| path.terminal),
            "json union branch should keep a terminal custom-scalar path"
        );
        assert!(!union_paths.children.contains_key("_internal_qp_alias_0"));

        let response = super::SubgraphResponse::deserialize_from_bytes(
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
            Some(custom_scalar_paths),
        )
        .unwrap();

        let data = response.data.as_object().unwrap();

        let interface_thing = data
            .iter()
            .find(|(key, _)| *key == "interfaceThing")
            .unwrap()
            .1
            .as_object()
            .expect("interfaceThing should stay structured");
        let interface_meta = interface_thing
            .iter()
            .find(|(key, _)| *key == "_internal_qp_alias_0")
            .unwrap()
            .1
            .as_str();
        assert_eq!(interface_meta, Some("interface string"));

        let union_thing = data
            .iter()
            .find(|(key, _)| *key == "unionThing")
            .unwrap()
            .1
            .as_object()
            .expect("unionThing should stay structured");
        let union_meta = union_thing
            .iter()
            .find(|(key, _)| *key == "meta")
            .unwrap()
            .1
            .as_raw_json()
            .expect("json union branch should deserialize as raw json");
        assert!(union_meta.contains("union.key\\t"));
    }

    fn find_fetch_custom_scalar_paths<'a>(
        node: Option<&'a PlanNode>,
        service_name: &str,
    ) -> Option<&'a CustomScalarPaths> {
        match node? {
            PlanNode::Fetch(fetch) if fetch.service_name == service_name => {
                fetch.custom_scalar_paths.as_ref()
            }
            PlanNode::BatchFetch(fetch) if fetch.service_name == service_name => {
                fetch.custom_scalar_paths.as_ref()
            }
            PlanNode::Sequence(sequence) => sequence
                .nodes
                .iter()
                .find_map(|node| find_fetch_custom_scalar_paths(Some(node), service_name)),
            PlanNode::Parallel(parallel) => parallel
                .nodes
                .iter()
                .find_map(|node| find_fetch_custom_scalar_paths(Some(node), service_name)),
            PlanNode::Flatten(flatten) => {
                find_fetch_custom_scalar_paths(Some(&flatten.node), service_name)
            }
            PlanNode::Condition(condition) => find_fetch_custom_scalar_paths(
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
