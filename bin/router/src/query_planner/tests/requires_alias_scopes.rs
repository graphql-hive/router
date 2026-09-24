//! A `@requires` reads `price(currency: "EUR")` while the client asks for
//! `price(currency: "GBP")` on the same objects, so one of them gets an internal alias. These
//! tests check what the plan does with it, under interfaces, type conditions and `@include`.
//!
//! All fixtures are composed from real subgraphs:
//! - `requires-alias-entity-interface`: `shop` has `interface Node { price(currency:) }` with
//!   `Cat` and `Dog`, and `pricing` has `Cat.eur` / `Dog.eur` requiring the EUR price.
//! - `requires-alias-interface-object`: the same `shop`, with `pricing` seeing `Node` as an
//!   `@interfaceObject` with `eur` on it. Composed with Apollo, the Guild composer gets this one
//!   wrong, see `composition-guild.md`.
//! - `requires-alias-two-requirements`: the same `shop` and `pricing`, plus `tax` with
//!   `Cat.gbp` requiring the GBP price. Composed with Apollo, for the same reason.
//! - `requires-alias-object-field`: `users` has `User.team(role:)`, `teams` has `Team.name`,
//!   and `perms` has `User.label` requiring `team(role: "admin") { name }`.

use std::error::Error;

use graphql_tools::{
    parser::query::{Definition, OperationDefinition, Selection, SelectionSet, TypeCondition},
    validation::{
        rules::OverlappingFieldsCanBeMerged,
        validate::{validate, ValidationPlan},
    },
};
use serde_json::Value;

use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::{parse_operation, parse_schema},
};

const INTERFACE_OBJECT: &str = "fixture/tests/requires-alias-interface-object.supergraph.graphql";
const ENTITY_INTERFACE: &str = "fixture/tests/requires-alias-entity-interface.supergraph.graphql";
const TWO_REQUIREMENTS: &str = "fixture/tests/requires-alias-two-requirements.supergraph.graphql";
const OBJECT_FIELD: &str = "fixture/tests/requires-alias-object-field.supergraph.graphql";

/// What `shop` serves, without the federation parts. Enough to validate its root fetches.
const SHOP_SCHEMA: &str = r#"
  type Query { things: [Node!]! }
  interface Node { id: ID! price(currency: String!): Int! }
  type Cat implements Node { id: ID! price(currency: String!): Int! meow: String! }
  type Dog implements Node { id: ID! price(currency: String!): Int! bark: String! }
"#;

/// `eur` of `Node`s under `$x` and of `Cat`s under `$y`. Under `$x`, `shop` fetches the EUR
/// price as `_internal_qp_alias_0`, next to the client's GBP one.
///
/// The `Cat` representations are sent when `$y` is true, so they have to read a value that's
/// fetched when `$y` is true, not `_internal_qp_alias_0`, which only exists when `$x` is. With
/// `$x: false, $y: true`, `pricing` would get no price.
#[test]
fn requires_alias_is_read_only_where_it_is_fetched() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = plan(
        INTERFACE_OBJECT,
        r#"
        query($x: Boolean!, $y: Boolean!) {
          things {
            ... on Node @include(if: $x) { price(currency: "GBP") eur }
            ... on Node @include(if: $y) { ... on Cat { eur } }
          }
        }
        "#,
    )?;

    let shop: Vec<_> = fetches(&plan)
        .into_iter()
        .filter(|f| f.service == "shop")
        .collect();
    let readers: Vec<_> = fetches(&plan)
        .into_iter()
        .filter(|f| f.service == "pricing")
        .collect();
    assert_eq!(
        readers.len(),
        2,
        "expected a pricing fetch under $x and one under $y"
    );

    // The one under `$x` reads the alias fetched under `$x`, which is right.
    let mut wrong = Vec::new();
    for reader in readers {
        let key = read_keys(&reader.requires, "price")
            .into_iter()
            .next()
            .expect("the representation reads a price")
            .1;
        // Any `shop` fetch sent whenever the reader is, with the EUR price under that key.
        let sent_with_reader =
            |conditions: &[String]| conditions.iter().all(|v| reader.conditions.contains(v));
        let fetched_when_sent = shop
            .iter()
            .filter(|fetch| sent_with_reader(&fetch.conditions))
            .flat_map(|fetch| selections_with_key(&fetch.operation, &key))
            .filter(|s| s.arguments == r#"currency: "EUR""#)
            .any(|s| sent_with_reader(&s.includes));
        if !fetched_when_sent {
            wrong.push(format!(
                "representations sent under {:?} read `{key}`, but `shop` fetches it under other conditions",
                reader.conditions
            ));
        }
    }
    let operations: Vec<_> = shop.iter().map(|fetch| fetch.operation.as_str()).collect();
    assert!(wrong.is_empty(), "{wrong:#?}\n{operations:#?}");

    Ok(())
}

/// `shop` gets `price(currency: "GBP")` under `... on Node` and `price(currency: "EUR")`
/// under `... on Cat`, with the same response key. `Node` is an interface, so those two can
/// meet on one object, and GraphQL rejects the operation.
#[test]
fn requires_alias_keeps_shop_operation_valid_across_interface_and_object(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    for query in [
        r#"
        query($x: Boolean!) {
          things {
            ... on Node @include(if: $x) { price(currency: "GBP") eur }
            ... on Cat { eur }
          }
        }
        "#,
        r#"
        query($x: Boolean!, $y: Boolean!) {
          things {
            ... on Node @include(if: $x) { price(currency: "GBP") eur }
            ... on Node @include(if: $y) { ... on Cat { eur } }
          }
        }
        "#,
    ] {
        let plan = plan(INTERFACE_OBJECT, query)?;
        let shop = root_fetch(&plan, "shop");
        let errors = overlapping_fields_errors(SHOP_SCHEMA, &shop.operation);
        assert!(
            errors.is_empty(),
            "`shop` operation doesn't validate: {errors:?}\n{}",
            shop.operation
        );
    }

    Ok(())
}

/// `Cat.eur` and `Dog.eur` both need the EUR price, and the client asks for the GBP one on
/// every `Node`. The two `pricing` fetches get batched into one step for `Cat` and `Dog`, and
/// planning fails in the alias pass with "Expected single input type ... but found multi-type
/// input".
///
/// Both kinds of representations have to read the EUR value, not the client's GBP one.
#[test]
fn requires_alias_reaches_readers_batched_for_many_types() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = plan(
        ENTITY_INTERFACE,
        r#"
        {
          things {
            price(currency: "GBP")
            ... on Cat { eur }
            ... on Dog { eur }
          }
        }
        "#,
    )?;

    let shop = root_fetch(&plan, "shop");
    let readers: Vec<_> = fetches(&plan)
        .into_iter()
        .filter(|f| f.service == "pricing")
        .collect();
    assert!(!readers.is_empty(), "expected pricing fetches");

    for reader in readers {
        for (type_name, key) in read_keys(&reader.requires, "price") {
            let values: Vec<_> = selections_with_key(&shop.operation, &key)
                .into_iter()
                .filter(|s| s.applies_to(&type_name))
                .map(|s| s.arguments)
                .collect();
            assert!(
                !values.is_empty() && values.iter().all(|a| a == r#"currency: "EUR""#),
                "`{type_name}` representations read `{key}`, which `shop` fetches as {values:?}:\n{}",
                shop.operation
            );
        }
    }

    Ok(())
}

/// `pricing` needs the EUR price of every `Node` and `tax` the GBP price of `Cat`s, next to
/// the client's own price. `shop` fetches them per type, under different aliases, like
/// `_internal_qp_alias_1: price(currency: "EUR")` for `Cat` and `_internal_qp_alias_2` for
/// `Dog`.
///
/// The `pricing` input is `... on Node { ... on Cat { price } ... on Dog { price } }`. The
/// alias pass only looks for `price` right in the input, not inside those fragments, so it
/// stays plain `price` and `pricing` gets the client's price instead of the EUR one.
#[test]
fn requires_alias_reaches_readers_inside_type_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    for query in [
        r#"{ things { price(currency: "USD") eur ... on Cat { gbp } } }"#,
        r#"{ things { ... on Cat { gbp } eur } }"#,
        r#"{ things { ... on Cat { price(currency: "USD") gbp } eur } }"#,
    ] {
        let plan = plan(TWO_REQUIREMENTS, query)?;
        let shop = root_fetch(&plan, "shop");
        let mut wrong = Vec::new();
        for reader in fetches(&plan) {
            let expected = match reader.service.as_str() {
                "pricing" => r#"currency: "EUR""#,
                "tax" => r#"currency: "GBP""#,
                _ => continue,
            };
            for (type_name, key) in read_keys(&reader.requires, "price") {
                let values: Vec<_> = selections_with_key(&shop.operation, &key)
                    .into_iter()
                    .filter(|s| s.applies_to(&type_name))
                    .map(|s| s.arguments)
                    .collect();
                if values.is_empty() || values.iter().any(|a| a != expected) {
                    wrong.push(format!(
                        "`{}` reads `{key}` of `{type_name}`, which `shop` fetches as {values:?}",
                        reader.service
                    ));
                }
            }
        }
        assert!(wrong.is_empty(), "{query}\n{wrong:#?}\n{}", shop.operation);
    }

    Ok(())
}

/// `perms` needs `team(role: "admin") { name }` and the client asks for
/// `team(role: "user") { name }` on the same `me`. When they end up in one fetch, the admin one
/// gets an alias. When they don't, the admin team still comes back under the plain `team` key,
/// lands on the same `me` as the client's team, and both `teams` fetches write `name` into
/// that one object. The client can get the admin team's name.
///
/// So no two fetches can write the same key at the same place with different arguments.
#[test]
fn requires_alias_keeps_response_keys_apart_across_fetches() -> Result<(), Box<dyn Error>> {
    init_logger();
    for query in [
        // Both in one fetch, the admin team is aliased. This one is right.
        r#"{ me { team(role: "user") { name } label } }"#,
        r#"query($x: Boolean!) { me { team(role: "user") { name } label @include(if: $x) } }"#,
        r#"query($x: Boolean!) { me { team(role: "user") @include(if: $x) { name } label } }"#,
        r#"
        query($x: Boolean!, $y: Boolean!) {
          me {
            ... on User @include(if: $x) { team(role: "user") { name } label }
            ... on User @include(if: $y) { label }
          }
        }
        "#,
    ] {
        let plan = plan(OBJECT_FIELD, query)?;
        let writes: Vec<_> = fetches(&plan).iter().flat_map(writes).collect();
        let mut clashes = Vec::new();
        for (i, a) in writes.iter().enumerate() {
            for b in &writes[i + 1..] {
                if a.location == b.location && a.key == b.key && a.arguments != b.arguments {
                    clashes.push(format!(
                        "`{}` at `{}` is `{}({})` from {} and `{}({})` from {}",
                        a.key,
                        a.location.join("."),
                        a.name,
                        a.arguments,
                        a.service,
                        b.name,
                        b.arguments,
                        b.service
                    ));
                }
            }
        }
        assert!(clashes.is_empty(), "{query}\n{clashes:#?}");
    }

    Ok(())
}

fn plan(fixture: &str, query: &str) -> Result<Value, Box<dyn Error>> {
    let plan = build_query_plan_with_defaults(fixture, parse_operation(query))?;
    Ok(serde_json::to_value(&plan)?)
}

struct Fetch {
    service: String,
    operation: String,
    requires: Value,
    /// Variables of the `Condition` nodes around the fetch, `!x` for a skip.
    conditions: Vec<String>,
    /// Where the fetched objects go, as response keys, without lists and type conditions.
    paths: Vec<Vec<String>>,
    /// In a batch, the alias of this entity call, like `_e0`.
    entity_alias: Option<String>,
}

/// A `Flatten` or batch path, as response keys.
fn response_keys(path: &Value) -> Vec<String> {
    path.as_array()
        .into_iter()
        .flatten()
        .filter_map(|segment| segment.get("Field").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn fetches(plan: &Value) -> Vec<Fetch> {
    fn walk(node: &Value, conditions: &[String], path: &[String], out: &mut Vec<Fetch>) {
        match node {
            Value::Object(map) => {
                if map.get("kind").and_then(Value::as_str) == Some("Fetch") {
                    out.push(Fetch {
                        service: map["serviceName"].as_str().unwrap_or_default().to_string(),
                        operation: map["operation"].as_str().unwrap_or_default().to_string(),
                        requires: map.get("requires").cloned().unwrap_or(Value::Null),
                        conditions: conditions.to_vec(),
                        paths: vec![path.to_vec()],
                        entity_alias: None,
                    });
                    return;
                }
                if map.get("kind").and_then(Value::as_str) == Some("Flatten") {
                    walk(&map["node"], conditions, &response_keys(&map["path"]), out);
                    return;
                }
                // Each entity call in a batch has its own representations.
                if map.get("kind").and_then(Value::as_str) == Some("BatchFetch") {
                    let aliases = map["entityBatch"]["aliases"].as_array();
                    for alias in aliases.into_iter().flatten() {
                        out.push(Fetch {
                            service: map["serviceName"].as_str().unwrap_or_default().to_string(),
                            operation: map["operation"].as_str().unwrap_or_default().to_string(),
                            requires: alias["requires"].clone(),
                            conditions: conditions.to_vec(),
                            paths: alias["paths"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(response_keys)
                                .collect(),
                            entity_alias: alias["alias"].as_str().map(str::to_string),
                        });
                    }
                    return;
                }
                if map.get("kind").and_then(Value::as_str) == Some("Condition") {
                    let variable = map["condition"].as_str().unwrap_or_default();
                    for (branch, prefix) in [("ifClause", ""), ("elseClause", "!")] {
                        if let Some(branch) = map.get(branch) {
                            let mut inner = conditions.to_vec();
                            inner.push(format!("{prefix}{variable}"));
                            walk(branch, &inner, path, out);
                        }
                    }
                    return;
                }
                for value in map.values() {
                    walk(value, conditions, path, out);
                }
            }
            Value::Array(items) => items
                .iter()
                .for_each(|item| walk(item, conditions, path, out)),
            _ => {}
        }
    }

    let mut out = Vec::new();
    walk(plan, &[], &[], &mut out);
    out
}

/// The first fetch to `service` that isn't an entity call.
fn root_fetch(plan: &Value, service: &str) -> Fetch {
    fetches(plan)
        .into_iter()
        .find(|f| f.service == service && !f.operation.contains("_entities"))
        .unwrap_or_else(|| panic!("expected a root fetch to {service}"))
}

/// For each type in a representation, the response key it reads `field` from.
/// Looks into nested fragments too, and takes the innermost type.
fn read_keys(requires: &Value, field: &str) -> Vec<(String, String)> {
    fn walk(selections: &Value, type_name: &str, field: &str, out: &mut Vec<(String, String)>) {
        for selection in selections.as_array().into_iter().flatten() {
            if selection["kind"].as_str() == Some("InlineFragment") {
                let inner = selection["typeCondition"].as_str().unwrap_or(type_name);
                walk(&selection["selections"], inner, field, out);
                continue;
            }
            let name = selection["name"].as_str().unwrap_or_default();
            let alias = selection.get("alias").and_then(Value::as_str);
            if alias.unwrap_or(name) == field {
                out.push((type_name.to_string(), name.to_string()));
            }
        }
    }

    let mut out = Vec::new();
    walk(requires, "", field, &mut out);
    out
}

struct Write {
    service: String,
    /// Where the object holding the field sits, as response keys.
    location: Vec<String>,
    key: String,
    name: String,
    arguments: String,
}

/// Every field a fetch puts into the response, and where.
fn writes(fetch: &Fetch) -> Vec<Write> {
    fn collect(
        set: &SelectionSet<'_, String>,
        location: &[String],
        fetch: &Fetch,
        out: &mut Vec<Write>,
    ) {
        for item in &set.items {
            match item {
                Selection::Field(field) => {
                    let key = field.alias.clone().unwrap_or_else(|| field.name.clone());
                    out.push(Write {
                        service: fetch.service.clone(),
                        location: location.to_vec(),
                        key: key.clone(),
                        name: field.name.clone(),
                        arguments: arguments_of(field),
                    });
                    let mut inner = location.to_vec();
                    inner.push(key);
                    collect(&field.selection_set, &inner, fetch, out);
                }
                Selection::InlineFragment(fragment) => {
                    collect(&fragment.selection_set, location, fetch, out)
                }
                Selection::FragmentSpread(_) => {}
            }
        }
    }

    let mut out = Vec::new();
    let document = parse_operation(&fetch.operation);
    for root in operation_selection_sets(&document) {
        let entities_key = fetch.entity_alias.as_deref().unwrap_or("_entities");
        let entities = root.items.iter().find_map(|item| match item {
            Selection::Field(field)
                if field.name == "_entities"
                    && field.alias.as_deref().unwrap_or(&field.name) == entities_key =>
            {
                Some(&field.selection_set)
            }
            _ => None,
        });
        match entities {
            Some(set) => {
                for path in &fetch.paths {
                    collect(set, path, fetch, &mut out);
                }
            }
            None => collect(root, &[], fetch, &mut out),
        }
    }
    out
}

fn arguments_of(field: &graphql_tools::parser::query::Field<'_, String>) -> String {
    field
        .arguments
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Operations without variables are printed as a bare `{ ... }`.
fn operation_selection_sets<'a>(
    document: &'a graphql_tools::parser::query::Document<'static, String>,
) -> Vec<&'a SelectionSet<'static, String>> {
    document
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            Definition::Operation(OperationDefinition::Query(query)) => Some(&query.selection_set),
            Definition::Operation(OperationDefinition::SelectionSet(set)) => Some(set),
            _ => None,
        })
        .collect()
}

struct Selected {
    arguments: String,
    /// Type conditions on the way to the field, outermost first.
    types: Vec<String>,
    /// `@include(if:)` variables on the way to the field, including its own.
    includes: Vec<String>,
}

impl Selected {
    /// Whether objects of `type_name` get this field. An interface in the way counts, as the
    /// objects are all `Node`s here.
    fn applies_to(&self, type_name: &str) -> bool {
        self.types.iter().all(|t| t == type_name || t == "Node")
    }
}

/// Fields of the operation with the response key `key`.
fn selections_with_key(operation: &str, key: &str) -> Vec<Selected> {
    fn includes_of(
        directives: &[graphql_tools::parser::query::Directive<'_, String>],
    ) -> Vec<String> {
        directives
            .iter()
            .filter(|d| d.name == "include")
            .flat_map(|d| d.arguments.iter())
            .filter_map(|(_, value)| match value {
                graphql_tools::parser::query::Value::Variable(name) => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    fn walk(
        set: &SelectionSet<'_, String>,
        key: &str,
        types: &[String],
        includes: &[String],
        out: &mut Vec<Selected>,
    ) {
        for item in &set.items {
            match item {
                Selection::Field(field) => {
                    let mut includes = includes.to_vec();
                    includes.extend(includes_of(&field.directives));
                    if field.alias.as_deref().unwrap_or(&field.name) == key {
                        out.push(Selected {
                            arguments: field
                                .arguments
                                .iter()
                                .map(|(name, value)| format!("{name}: {value}"))
                                .collect::<Vec<_>>()
                                .join(", "),
                            types: types.to_vec(),
                            includes: includes.clone(),
                        });
                    }
                    walk(&field.selection_set, key, &[], &includes, out);
                }
                Selection::InlineFragment(fragment) => {
                    let mut types = types.to_vec();
                    if let Some(TypeCondition::On(name)) = &fragment.type_condition {
                        types.push(name.clone());
                    }
                    let mut includes = includes.to_vec();
                    includes.extend(includes_of(&fragment.directives));
                    walk(&fragment.selection_set, key, &types, &includes, out);
                }
                Selection::FragmentSpread(_) => {}
            }
        }
    }

    let document = parse_operation(operation);
    let mut out = Vec::new();
    for definition in &document.definitions {
        // Operations without variables are printed as a bare `{ ... }`.
        let selection_set = match definition {
            Definition::Operation(OperationDefinition::Query(query)) => &query.selection_set,
            Definition::Operation(OperationDefinition::SelectionSet(set)) => set,
            _ => continue,
        };
        walk(selection_set, key, &[], &[], &mut out);
    }
    out
}

fn overlapping_fields_errors(schema: &str, operation: &str) -> Vec<String> {
    let schema = parse_schema(schema);
    let operation = parse_operation(operation);
    let plan = ValidationPlan::from(vec![Box::new(OverlappingFieldsCanBeMerged::new())]);
    validate(&schema, &operation, &plan)
        .into_iter()
        .map(|error| error.message)
        .collect()
}
