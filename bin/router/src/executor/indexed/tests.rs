//! The indexed pipeline against the `Value` pipeline, one root fetch at a time: the same body goes
//! through `deserialize_with_shape` + `deep_merge` + `project_by_operation` and through
//! `parse_body` + `merge_object` + `write_response`, and the client outputs must be JSON-equal
//! (same keys in the same order, numbers by value). Bodies cover what a subgraph can get wrong
//! or write differently: key order, missing and extra keys, nulls where the type is non-null,
//! invalid enum values, escapes, whitespace, number forms.

use std::sync::Arc;

use bytes::Bytes;

use super::compile::IndexedResponsePlan;
use super::parse::parse_body;
use super::store::{Arena, ValueCell};
use super::write::write_response;
use crate::executor::execution::plan::CoerceVariablesPayload;
use crate::executor::introspection::partition::partition_operation;
use crate::executor::plugins::hooks::on_supergraph_load::Supergraph;
use crate::executor::plugins::response_data::{DataEditError, DataValue, ResponseData};
use crate::executor::projection::plan::ProjectionPlan;
use crate::executor::projection::response::project_by_operation;
use crate::executor::response::graphql_error::GraphQLErrorPathSegment;
use crate::executor::response::merge::deep_merge;
use crate::executor::response::subgraph_response::SubgraphResponse;
use crate::executor::response::value::Value;
use crate::query_planner::ast::normalization::normalize_operation;
use crate::query_planner::planner::plan_nodes::PlanNode;
use crate::query_planner::state::supergraph_state::OperationKind;
use crate::query_planner::utils::cancellation::CancellationToken;
use crate::query_planner::utils::parsing::parse_operation;

fn no_variables() -> Arc<CoerceVariablesPayload> {
    Arc::new(CoerceVariablesPayload {
        variables_map: None,
    })
}

pub(crate) const HEADER: &str = r#"
schema @link(url: "https://specs.apollo.dev/link/v1.0") @link(url: "https://specs.apollo.dev/join/v0.3", for: EXECUTION) { query: Query }
directive @join__enumValue(graph: join__Graph!) repeatable on ENUM_VALUE
directive @join__graph(name: String!, url: String!) on ENUM_VALUE
directive @join__field(graph: join__Graph, requires: join__FieldSet, provides: join__FieldSet, type: String, external: Boolean, override: String, usedOverridden: Boolean) repeatable on FIELD_DEFINITION | INPUT_FIELD_DEFINITION
directive @join__implements(graph: join__Graph!, interface: String!) repeatable on OBJECT | INTERFACE
directive @join__type(graph: join__Graph!, key: join__FieldSet, extension: Boolean! = false, resolvable: Boolean! = true, isInterfaceObject: Boolean! = false) repeatable on OBJECT | INTERFACE | UNION | ENUM | INPUT_OBJECT | SCALAR
directive @join__unionMember(graph: join__Graph!, member: String!) repeatable on UNION
scalar join__FieldSet
directive @link(url: String, as: String, for: link__Purpose, import: [link__Import]) repeatable on SCHEMA
scalar link__Import
enum link__Purpose { SECURITY EXECUTION }
enum join__Graph { A @join__graph(name: "a", url: "http://localhost/a") }
"#;

const TYPES: &str = r#"
type Query @join__type(graph: A) {
  products: [Product!]!
  product: Product
}
type Product @join__type(graph: A) {
  id: ID!
  name: String
  price: Float!
  state: State
  tags: [String!]
  variants: [Variant!]!
  meta: Meta
}
type Variant @join__type(graph: A) {
  sku: String!
  size: Int
}
type Meta @join__type(graph: A) {
  label: String
  states: [State]
}
enum State @join__type(graph: A) {
  ACTIVE @join__enumValue(graph: A)
  RETIRED @join__enumValue(graph: A)
}
"#;

/// Runs `body` as the response of the only fetch of `query`, through both pipelines. Also
/// returns how many subtrees the indexed plan may pass through, and which root fields of `data`
/// the parser kept as one raw range (passed through).
fn both_with(types: &str, query: &str, body: &str) -> (String, String, usize, Vec<String>) {
    both_edited(types, query, body, None)
}

/// [`both_with`], with the data walked by `edit` (see `ResponseData::walk`) before it's written
fn both_edited(
    types: &str,
    query: &str,
    body: &str,
    mut edit: Option<&mut dyn FnMut(&mut DataValue<'_, '_>)>,
) -> (String, String, usize, Vec<String>) {
    let header = if types.contains("type Mutation") {
        HEADER.replace("{ query: Query }", "{ query: Query mutation: Mutation }")
    } else {
        HEADER.to_string()
    };
    let sdl = format!("{header}{types}");
    let supergraph = Supergraph::from_sdl("test", &sdl, Default::default()).expect("builds");
    let planner = &supergraph.planner;
    let metadata = Arc::clone(&supergraph.metadata);
    let document = parse_operation(query);
    let normalized = normalize_operation(&planner.supergraph, &document, None).expect("normalizes");
    let (root, projection) = ProjectionPlan::from_operation(&normalized.operation, &metadata);
    let projection = Arc::new(projection);
    let operation = partition_operation(normalized.operation).downstream_operation;
    let plan = planner
        .plan_from_normalized_operation(&operation, Default::default(), &CancellationToken::new())
        .expect("plans")
        .into_executable(&planner.supergraph);
    let Some(PlanNode::Fetch(fetch)) = &plan.node else {
        panic!("one root fetch");
    };
    let bytes = Bytes::from(body.to_string());

    let old = match SubgraphResponse::deserialize_with_shape(
        bytes.clone(),
        fetch.operation.response_shape.as_deref().expect("a shape"),
        fetch.custom_scalar_paths.as_ref(),
    ) {
        Ok(mut response) => {
            // like the executor: output rewrites before the merge
            for rewrite in fetch.output_rewrites.iter().flatten() {
                use crate::executor::execution::rewrites::FetchRewriteExt;
                rewrite.rewrite(&metadata.possible_types, &mut response.data);
            }
            let mut data = Value::Object(Vec::new());
            deep_merge(&mut data, response.data);
            let mut errors = response.errors.unwrap_or_default();
            if let Some(edit) = edit.as_mut() {
                let mut walked = ResponseData::from_value(
                    data,
                    projection.clone(),
                    metadata.clone(),
                    no_variables(),
                    OperationKind::Query,
                );
                walked.walk(&mut **edit);
                let (edited, plugin_errors) = walked.into_value();
                data = edited;
                errors.extend(plugin_errors);
            }
            let output = project_by_operation(
                &data,
                errors,
                &Default::default(),
                root,
                &projection,
                &None,
                0,
                &metadata,
            )
            .expect("projects");
            String::from_utf8(output).expect("utf-8")
        }
        Err(_) => "invalid".to_string(),
    };

    let indexed = IndexedResponsePlan::compile(
        &plan,
        &projection,
        &metadata,
        root,
        super::compile::Seed::None,
        false,
    )
    .expect("compiles");
    let indexed = Arc::new(indexed);
    let program = &indexed.fetches[&fetch.id];
    let passthroughs = program.shape.verifies.len();
    let mut arena = Arena::default();
    let width = indexed.layouts[indexed.root_layout as usize].width();
    let first = arena.object(width);
    let mut raw = Vec::new();
    // a plugin walks the data value by value, nothing is passed through then
    let passthrough = edit.is_none();
    let new = match parse_body(
        &mut arena,
        &indexed.layouts,
        &program.shape,
        &bytes,
        passthrough,
    ) {
        Some(parsed) => {
            if let ValueCell::Object {
                layout,
                first: data,
            } = parsed.data
            {
                for (field_index, key) in indexed.layouts[layout as usize].keys.iter().enumerate() {
                    if let ValueCell::RawJson(_) = arena.cells[data as usize + field_index] {
                        raw.push(key.to_string());
                    }
                }
                arena.merge_object(&indexed.layouts, layout, first, data);
            }
            let mut root_data = ValueCell::Object {
                layout: indexed.root_layout,
                first,
            };
            let mut errors = parsed.errors.unwrap_or_default();
            if let Some(edit) = edit.as_mut() {
                let mut walked = ResponseData::from_cells(
                    std::mem::take(&mut arena),
                    root_data,
                    indexed.clone(),
                    metadata.clone(),
                    no_variables(),
                    OperationKind::Query,
                );
                walked.walk(&mut **edit);
                let (edited, data, plugin_errors) = walked.into_cells();
                arena = edited;
                root_data = data;
                errors.extend(plugin_errors);
            }
            let output = write_response(
                &arena,
                &indexed,
                root_data,
                errors,
                &Default::default(),
                root,
                &None,
                0,
                &metadata,
            )
            .expect("writes");
            String::from_utf8(output).expect("utf-8")
        }
        None => "invalid".to_string(),
    };
    (old, new, passthroughs, raw)
}

fn json_equal(a: &sonic_rs::Value, b: &sonic_rs::Value) -> bool {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};
    if let (Some(a), Some(b)) = (a.as_object(), b.as_object()) {
        return a.len() == b.len()
            && a.iter()
                .zip(b.iter())
                .all(|((ka, va), (kb, vb))| ka == kb && json_equal(va, vb));
    }
    if let (Some(a), Some(b)) = (a.as_array(), b.as_array()) {
        return a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| json_equal(a, b));
    }
    if a.is_number() && b.is_number() {
        return a.as_f64() == b.as_f64();
    }
    a == b
}

#[track_caller]
fn same(query: &str, body: &str) -> usize {
    same_with(TYPES, query, body).0
}

/// [`same`] for other types. Also returns the root fields that were passed through.
#[track_caller]
fn same_with(types: &str, query: &str, body: &str) -> (usize, Vec<String>) {
    let (old, new, passthroughs, raw) = both_with(types, query, body);
    if old == "invalid" || new == "invalid" {
        assert_eq!(old, new, "one pipeline rejected the body\nbody: {body}");
        return (passthroughs, raw);
    }
    let a: sonic_rs::Value = sonic_rs::from_str(&old).expect("old output is JSON");
    let b: sonic_rs::Value = sonic_rs::from_str(&new).expect("new output is JSON");
    assert!(
        json_equal(&a, &b),
        "outputs differ\nbody: {body}\nold:  {old}\nnew:  {new}"
    );
    (passthroughs, raw)
}

const QUERY: &str = "{ products { id name price state tags variants { sku size } meta { label states } } product { id } }";

#[test]
fn passthrough_and_fallback_match_the_value_pipeline() {
    let product =
        |fields: &str| format!(r#"{{"data":{{"products":[{fields}],"product":{{"id":"x"}}}}}}"#);
    let bodies = [
        // the order the fetch asks for
        product(r#"{"id":"1","name":"a","price":1.5,"state":"ACTIVE","tags":["t"],"variants":[{"sku":"s","size":1}],"meta":{"label":"l","states":["RETIRED",null]}}"#),
        // keys in another order
        product(r#"{"name":"a","id":"1","state":"ACTIVE","price":1.5,"variants":[{"size":1,"sku":"s"}],"tags":["t"],"meta":{"states":[],"label":"l"}}"#),
        // a nullable key missing
        product(r#"{"id":"1","price":1.5,"state":"ACTIVE","tags":null,"variants":[],"meta":null}"#),
        // an extra key
        product(r#"{"id":"1","name":"a","price":1.5,"state":"ACTIVE","tags":[],"variants":[{"sku":"s","size":1,"extra":true}],"meta":{"label":"l","states":null,"x":1}}"#),
        // null where non-null: a variant's sku, an item of [String!], a product's price
        product(r#"{"id":"1","name":"a","price":1.5,"state":"ACTIVE","tags":["t"],"variants":[{"sku":null,"size":1}],"meta":null}"#),
        product(r#"{"id":"1","name":"a","price":1.5,"state":"ACTIVE","tags":["t",null],"variants":[],"meta":null}"#),
        product(r#"{"id":"1","name":"a","price":null,"state":"ACTIVE","tags":[],"variants":[],"meta":null}"#),
        // invalid enum values, at the top and in a list
        product(r#"{"id":"1","name":"a","price":1.5,"state":"NOPE","tags":[],"variants":[],"meta":{"label":"l","states":["NOPE"]}}"#),
        // an escaped enum value, escapes and unicode in strings
        product(r#"{"id":"1","name":"a\"b\\cé ż \n","price":1.5,"state":"ACTIVE","tags":["A"],"variants":[],"meta":{"label":"\/x","states":["ACTIVE"]}}"#),
        // whitespace everywhere
        "{ \"data\" : { \"products\" : [ { \"id\" : \"1\" , \"name\" : null , \"price\" : 2 , \"state\" : null , \"tags\" : [ ] , \"variants\" : [ { \"sku\" : \"s\" , \"size\" : null } ] , \"meta\" : { \"label\" : \"l\" , \"states\" : [ \"ACTIVE\" ] } } ] , \"product\" : null } }\n".to_string(),
        // number forms
        product(r#"{"id":"1","name":"a","price":1e2,"state":"ACTIVE","tags":[],"variants":[{"sku":"s","size":-0}],"meta":null},{"id":"2","name":"b","price":1.50,"state":"RETIRED","tags":[],"variants":[{"sku":"s","size":12345678901}],"meta":null}"#),
        // empty object and wrong kinds
        product(r#"{}"#),
        product(r#"{"id":"1","name":"a","price":1.5,"state":"ACTIVE","tags":"oops","variants":[],"meta":[]}"#),
        // data null with errors, and an error next to data
        r#"{"data":null,"errors":[{"message":"boom","path":["products"]}]}"#.to_string(),
        r#"{"errors":[{"message":"boom"}],"data":{"products":[],"product":null}}"#.to_string(),
        // not JSON
        r#"{"data":{"products":[{"id":"1","name":"a","price":01}]}}"#.to_string(),
        r#"{"data":{"products":[{"id":"1","name":"a	b"}]}}"#.to_string(),
    ];
    for body in &bodies {
        // `products`, `variants`, `meta` and `product` all qualify
        assert_eq!(same(QUERY, body), 4);
    }
}

#[test]
fn conditions_and_aliases_match_the_value_pipeline() {
    let query = "{ products { id n: name s: state ... on Product { tags } variants { sku } } }";
    let bodies = [
        r#"{"data":{"products":[{"id":"1","n":"a","s":"ACTIVE","tags":["x"],"variants":[{"sku":"s"}]}]}}"#,
        r#"{"data":{"products":[{"id":"1","n":"a","s":"BAD","tags":["x"],"variants":[{"sku":"s"}]}]}}"#,
        r#"{"data":{"products":[{"id":"1","s":"ACTIVE","n":"a","variants":[{"sku":null}],"tags":null}]}}"#,
    ];
    for body in bodies {
        // `... on Product` inside a Product is normalized away, so `products` and `variants`
        // both qualify
        assert_eq!(same(query, body), 2);
    }
}

const ABSTRACT: &str = r#"
type Query @join__type(graph: A) { things: [Thing] }
interface Thing @join__type(graph: A) { detail: Detail }
type Gadget implements Thing @join__type(graph: A) @join__implements(graph: A, interface: "Thing") { detail: Detail }
type Widget implements Thing @join__type(graph: A) @join__implements(graph: A, interface: "Thing") { detail: Detail }
interface Detail @join__type(graph: A) { meta: Meta }
type RichDetail implements Detail @join__type(graph: A) @join__implements(graph: A, interface: "Detail") { meta: Meta }
type BasicDetail implements Detail @join__type(graph: A) @join__implements(graph: A, interface: "Detail") { meta: Meta }
type Meta @join__type(graph: A) { a: String b: String }
"#;

#[test]
fn a_position_two_fields_read_is_never_passed_through() {
    // the Widget's `meta { a }` and the Gadget's `meta { a b }` read one position, and the
    // fetch selects `meta { a b }`: passing it through would give the Widget a `b`
    let query = "{ things { detail { meta { a } } ... on Gadget { detail { ... on RichDetail { meta { b } } } } } }";
    let body = r#"{"data":{"things":[
        {"__typename":"Gadget","detail":{"__typename":"RichDetail","meta":{"a":"A1","b":"B1"}}},
        {"__typename":"Gadget","detail":{"__typename":"BasicDetail","meta":{"a":"A2"}}},
        {"__typename":"Widget","detail":{"__typename":"RichDetail","meta":{"a":"A3","b":"B3"}}}
    ]}}"#;
    let (old, new, _, _) = both_with(ABSTRACT, query, body);
    let a: sonic_rs::Value = sonic_rs::from_str(&old).expect("JSON");
    let b: sonic_rs::Value = sonic_rs::from_str(&new).expect("JSON");
    assert!(json_equal(&a, &b), "old: {old}\nnew: {new}");
}

const JSON_SCALAR: &str = r#"
scalar JSON @join__type(graph: A)
type Query @join__type(graph: A) { products: [Product!]! }
type Product @join__type(graph: A) { id: ID! attrs: [Attr!]! }
type Attr @join__type(graph: A) { name: String! value: JSON! }
"#;

#[test]
fn a_custom_scalar_passes_through_whatever_json_it_holds() {
    let query = "{ products { id attrs { name value } } }";
    let attrs = |value: &str| {
        format!(
            r#"{{"data":{{"products":[{{"id":"1","attrs":[{{"name":"a","value":{value}}}]}}]}}}}"#
        )
    };
    for value in [
        r#""text""#,
        "1.5",
        r#"["a",{"b":[1]}]"#,
        r#"{"x":null}"#,
        "true",
    ] {
        let (_, raw) = same_with(JSON_SCALAR, query, &attrs(value));
        assert_eq!(raw, ["products"], "passed through with value {value}");
    }
    // null where the scalar is non-null bubbles, so it can't be passed through
    let (_, raw) = same_with(JSON_SCALAR, query, &attrs("null"));
    assert!(raw.is_empty());
}

#[test]
fn a_list_keeps_its_items_right_after_a_failed_check() {
    // the first item's keys come in another order: the check fails, and the parser reads the
    // rest of the list the normal way without checking again; the output is the same
    let body = r#"{"data":{"products":[
        {"name":"a","id":"1","price":1,"state":"ACTIVE","tags":[],"variants":[],"meta":null},
        {"id":"2","name":"b","price":2,"state":"RETIRED","tags":["t"],"variants":[{"sku":"s","size":1}],"meta":{"label":"l","states":[]}},
        {"id":"3","name":"c","price":3,"state":"NOPE","tags":null,"variants":[],"meta":null}
    ],"product":null}}"#;
    assert_eq!(same(QUERY, body), 4);
}

#[test]
fn a_rewritten_projection_plan_gets_an_indexed_plan_of_its_own() {
    use crate::executor::indexed::IndexedPlanCache;
    use crate::executor::operation_filter::PathSegment;
    use crate::pipeline::trie::Trie;

    let sdl = format!("{HEADER}{TYPES}");
    let supergraph = Supergraph::from_sdl("test", &sdl, Default::default()).expect("builds");
    let planner = &supergraph.planner;
    let metadata = Arc::clone(&supergraph.metadata);
    let document = parse_operation("{ products { id name } }");
    let normalized = normalize_operation(&planner.supergraph, &document, None).expect("normalizes");
    let (root, projection) = ProjectionPlan::from_operation(&normalized.operation, &metadata);
    let projection = Arc::new(projection);
    let operation = partition_operation(normalized.operation).downstream_operation;
    let plan = planner
        .plan_from_normalized_operation(&operation, Default::default(), &CancellationToken::new())
        .expect("plans")
        .into_executable(&planner.supergraph);
    let Some(PlanNode::Fetch(fetch)) = &plan.node else {
        panic!("one root fetch");
    };
    // what authorization does when the client may not see `products.name`: a new plan with
    // the field forced to null, for every request
    let rewrite = || {
        projection.rewrite(&Trie::from_paths(&[vec![
            PathSegment::Field("products"),
            PathSegment::Field("name"),
        ]]))
    };

    let cell = IndexedPlanCache::default();
    let original = cell
        .get(&plan, &projection, &metadata, root, None, false)
        .expect("compiles");
    let rewritten = rewrite();
    let indexed = cell
        .get(&plan, &rewritten, &metadata, root, None, false)
        .expect("compiles");
    assert!(
        !Arc::ptr_eq(&indexed, &original),
        "another projection, another indexed plan"
    );
    assert!(
        Arc::ptr_eq(
            &cell
                .get(&plan, &rewrite(), &metadata, root, None, false)
                .expect("compiles"),
            &indexed
        ),
        "the next request with the same rewrite reuses it"
    );
    assert!(Arc::ptr_eq(
        &cell
            .get(&plan, &projection, &metadata, root, None, false)
            .expect("compiles"),
        &original
    ));

    // and it writes what the other pipeline writes: `name` null
    let body = Bytes::from_static(br#"{"data":{"products":[{"id":"1","name":"a"}]}}"#);
    let response = SubgraphResponse::deserialize_with_shape(
        body.clone(),
        fetch.operation.response_shape.as_deref().expect("a shape"),
        fetch.custom_scalar_paths.as_ref(),
    )
    .expect("parses");
    let mut data = Value::Object(Vec::new());
    deep_merge(&mut data, response.data);
    let old = project_by_operation(
        &data,
        Vec::new(),
        &Default::default(),
        root,
        &rewritten,
        &None,
        0,
        &metadata,
    )
    .expect("projects");

    let program = &indexed.fetches[&fetch.id];
    let mut arena = Arena::default();
    let first = arena.object(indexed.layouts[indexed.root_layout as usize].width());
    let parsed =
        parse_body(&mut arena, &indexed.layouts, &program.shape, &body, true).expect("parses");
    if let ValueCell::Object {
        layout,
        first: data,
    } = parsed.data
    {
        arena.merge_object(&indexed.layouts, layout, first, data);
    }
    let new = write_response(
        &arena,
        &indexed,
        ValueCell::Object {
            layout: indexed.root_layout,
            first,
        },
        Vec::new(),
        &Default::default(),
        root,
        &None,
        0,
        &metadata,
    )
    .expect("writes");
    assert_eq!(String::from_utf8_lossy(&new), String::from_utf8_lossy(&old));
    assert_eq!(
        String::from_utf8_lossy(&new),
        r#"{"data":{"products":[{"id":"1","name":null}]}}"#
    );
}

#[test]
fn a_mutation_matches_the_value_pipeline() {
    let types = format!(
        "{TYPES}{}",
        r#"
type Mutation @join__type(graph: A) {
  addProduct(name: String!): Product!
}
"#
    );
    let query = r#"mutation { addProduct(name: "a") { id name price variants { sku } } }"#;
    for body in [
        r#"{"data":{"addProduct":{"id":"1","name":"a","price":1.5,"variants":[{"sku":"s"}]}}}"#,
        // a null in a non-null field bubbles up to `data`
        r#"{"data":{"addProduct":{"id":"1","name":"a","price":null,"variants":[]}}}"#,
        r#"{"data":null,"errors":[{"message":"boom","path":["addProduct"]}]}"#,
    ] {
        same_with(&types, query, body);
    }
}

const MISMATCH: &str = r#"
type Query @join__type(graph: A) { i: [I] }
interface I @join__type(graph: A) { id: ID }
type TypeA implements I @join__type(graph: A) @join__implements(graph: A, interface: "I") {
  id: ID
  strField: String
}
type TypeB implements I @join__type(graph: A) @join__implements(graph: A, interface: "I") {
  id: ID
  strField: [String]
}
"#;

#[test]
fn an_aliased_field_is_renamed_back_like_the_value_pipeline() {
    // `strField` has two types, so the planner aliases it for `TypeB` and renames it back in the
    // response (an output rewrite)
    let query = "{ i { ... on TypeA { strField } ... on TypeB { strField } } }";
    for body in [
        r#"{"data":{"i":[{"__typename":"TypeA","strField":"a"},{"__typename":"TypeB","_internal_qp_alias_0":["b","c"]}]}}"#,
        r#"{"data":{"i":[{"__typename":"TypeB","_internal_qp_alias_0":null},{"__typename":"TypeA","strField":null}]}}"#,
        r#"{"data":{"i":null}}"#,
    ] {
        let (old, new, _, _) = both_with(MISMATCH, query, body);
        assert_eq!(new, old, "body: {body}");
    }
}

#[test]
fn a_wide_object_runs_on_the_indexed_pipeline() {
    // past the 512 keys that used to send a plan to the other pipeline
    let fields: Vec<String> = (0..600).map(|i| format!("f{i}")).collect();
    let types = format!(
        "type Query @join__type(graph: A) {{ w: Wide }}\ntype Wide @join__type(graph: A) {{ {} }}\n",
        fields
            .iter()
            .map(|field| format!("{field}: Int"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let query = format!("{{ w {{ {} }} }}", fields.join(" "));
    let body = format!(
        r#"{{"data":{{"w":{{{}}}}}}}"#,
        fields
            .iter()
            .enumerate()
            .map(|(i, field)| format!(r#""{field}":{i}"#))
            .collect::<Vec<_>>()
            .join(",")
    );
    same_with(&types, &query, &body);
}

#[test]
fn plugin_edits_match_on_both_pipelines() {
    let query =
        "{ products { id name price state tags variants { sku size } } product { id name } }";
    let body = r#"{"data":{"products":[{"id":"1","name":"a\"b","price":1.5,"state":"ACTIVE","tags":["x","y"],"variants":[{"sku":"s","size":1}]}],"product":{"id":"2","name":"p"}}}"#;
    let mut outcomes = Vec::new();
    let mut edit = |value: &mut DataValue<'_, '_>| {
        let mut note = |what: &str, result: Result<(), DataEditError>| {
            outcomes.push(format!("{what}: {result:?}"))
        };
        if value.is("Product", "name") {
            let upper = value.as_str().map(|name| name.to_uppercase());
            note("name", value.set(upper));
        } else if value.is("Product", "price") {
            note("price string", value.set("x"));
            note("price", value.set(2));
        } else if value.is("Product", "state") {
            note("state unknown", value.set("NOPE"));
            note("state", value.set("RETIRED"));
        } else if value.is("Product", "tags") && value.index().is_none() {
            note("tags", value.set("z"));
        } else if value.is("Product", "tags") && value.index() == Some(1) {
            // a Non-Null item: the list becomes null
            value.set_null();
        } else if value.is("Variant", "size") {
            note("size too big", value.set(1u64 << 40));
            note("size", value.set(7));
        } else if value.is("Product", "variants") && value.index().is_none() {
            note("variants", value.set(1));
        } else if value.is("Product", "id") && value.path().segments.len() == 2 {
            // `product.id` is Non-Null: `product` becomes null
            value.set_null_with_error("hidden");
        }
    };
    let (old, new, _, _) = both_edited(TYPES, query, body, Some(&mut edit));
    assert_eq!(old, new, "both pipelines write the same response");
    assert_eq!(
        new,
        r#"{"data":{"products":[{"id":"1","name":"A\"B","price":2,"state":"RETIRED","tags":null,"variants":[{"sku":"s","size":7}]}],"product":null},"errors":[{"message":"hidden","path":["product","id"]}]}"#
    );
    let per_run = outcomes.len() / 2;
    assert_eq!(
        outcomes[..per_run],
        outcomes[per_run..],
        "the same outcomes"
    );
    assert_eq!(
        outcomes[..per_run].join("\n"),
        [
            "name: Ok(())",
            "price string: Err(WrongType)",
            "price: Ok(())",
            "state unknown: Err(WrongType)",
            "state: Ok(())",
            "tags: Err(NotALeaf)",
            "variants: Err(NotALeaf)",
            "size too big: Err(WrongType)",
            "size: Ok(())",
            "name: Ok(())",
        ]
        .join("\n")
    );
}

#[test]
fn a_plugin_can_fix_an_invalid_enum_value_on_both_pipelines() {
    let query = "{ products { id state } }";
    let body = r#"{"data":{"products":[{"id":"1","state":"LEGACY"},{"id":"2","state":"LEGACY"}]}}"#;
    let mut edit = |value: &mut DataValue<'_, '_>| {
        // the first is fixed, the second is left to the writer
        if value.is("Product", "state")
            && value.path().segments[1] == GraphQLErrorPathSegment::Index(0)
        {
            value.set("RETIRED").expect("a valid enum value");
        }
    };
    let (old, new, _, _) = both_edited(TYPES, query, body, Some(&mut edit));
    assert_eq!(old, new, "both pipelines write the same response");
    assert_eq!(
        new,
        r#"{"data":{"products":[{"id":"1","state":"RETIRED"},{"id":"2","state":null}]},"errors":[{"message":"Value is not a valid enum value"}]}"#
    );
}
