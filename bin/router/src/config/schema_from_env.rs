use schemars::{json_schema, Schema};
use serde_json::{json, Map, Value};

/// Walks a generated JSON schema and adds a `{ "from_env": "VAR" }` alternative everywhere a
/// bare object or array isn't strictly required, mirroring what
/// [`crate::config::from_env::resolve_from_env_placeholders`] accepts at runtime: any scalar field, and
/// any branch of an existing "either/or" union (e.g. [`crate::config::primitives::toggle::ToggleWith`],
/// [`crate::config::primitives::single_or_multiple::SingleOrMultiple`],
/// [`crate::config::primitives::value_or_expression::ValueOrExpression`]).
pub fn augment_with_from_env(schema: &mut Schema) {
    if let Some(map) = schema.as_object_mut() {
        augment_field_map(map);
    }
}

/// `default_schema` is the field's own original schema (before it grew a `from_env`
/// alternative) - it's reused here so `default: <value>` is validated against the same rules
/// as if `<value>` had been written directly in place of the `from_env` object.
fn from_env_alternative(default_schema: Value) -> Value {
    json_schema!({
        "type": "object",
        "properties": {
          "from_env": {
            "type": "string",
            "description": "Reads the value from an environment variable. In case of a missing value, a warning is logged and the field default/validation rules are applied as usual.",
          },
          "default": default_schema,
        },
        "required": ["from_env"],
        "additionalProperties": false
    })
    .to_value()
}

fn augment_field_value(value: &mut Value) {
    if let Value::Object(map) = value {
        augment_field_map(map);
    }
}

/// Full treatment for an actual value position (a struct field, an array item, the root): recurse
/// into its own nested structure, then decide whether this position itself gets a `from_env` alt.
fn augment_field_map(map: &mut Map<String, Value>) {
    recurse_into_properties_and_items(map);
    recurse_into_union_branches(map);

    if !is_object_or_array_only(map) {
        add_from_env_alternative(map);
    }
}

fn recurse_into_properties_and_items(map: &mut Map<String, Value>) {
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for value in properties.values_mut() {
            augment_field_value(value);
        }
    }
    if let Some(items) = map.get_mut("items") {
        augment_field_value(items);
    }
}

/// A union's branches are not themselves value positions - only the union as a whole is - so we
/// only descend into each branch's *own* nested fields, without deciding on the branch itself.
/// Otherwise a plain scalar branch (e.g. `ToggleWith<T>`'s `bool` branch) would grow its own
/// nested `from_env` alternative on top of the one added for the union as a whole.
fn recurse_into_union_branches(map: &mut Map<String, Value>) {
    for key in ["oneOf", "anyOf"] {
        if let Some(Value::Array(variants)) = map.get_mut(key) {
            for variant in variants.iter_mut() {
                if let Value::Object(variant_map) = variant {
                    recurse_into_properties_and_items(variant_map);
                    recurse_into_union_branches(variant_map);
                }
            }
        }
    }
}

/// Keys that tools read directly off a property's own schema object (for hover text,
/// autocomplete, form generation, ...) rather than by inspecting its `oneOf` branches.
const METADATA_KEYS: [&str; 5] = ["description", "title", "default", "deprecated", "examples"];

fn add_from_env_alternative(map: &mut Map<String, Value>) {
    if let Some(Value::Array(variants)) = map.get_mut("oneOf") {
        let default_schema = json!({ "oneOf": Value::Array(variants.clone()) });
        variants.push(from_env_alternative(default_schema));
        return;
    }
    if let Some(Value::Array(variants)) = map.get_mut("anyOf") {
        let default_schema = json!({ "anyOf": Value::Array(variants.clone()) });
        variants.push(from_env_alternative(default_schema));
        return;
    }

    let mut original = std::mem::take(map);
    for key in METADATA_KEYS {
        if let Some(value) = original.remove(key) {
            map.insert(key.to_string(), value);
        }
    }
    // `type` itself isn't hoisted (removed) from the branch - it's still needed there for the
    // branch's own validation - but a summary is copied to the top level too. Tools like
    // jsonschema2mk render a property's "Type" column straight from its own top-level `type`
    // and don't look inside `oneOf`, so without this the column would just go blank. This is
    // validation-neutral: each branch already enforces its own `type`, so this top-level `type`
    // is a strict superset and constrains nothing further.
    if let Some(summary) = summarize_types(original.get("type")) {
        map.insert("type".to_string(), summary);
    }
    // The branch keeps the field's original type-shape (minus the metadata just hoisted above);
    // that's exactly the schema `default: <value>` should also be validated against.
    let default_schema = Value::Object(original.clone());
    map.insert(
        "oneOf".to_string(),
        Value::Array(vec![
            Value::Object(original),
            from_env_alternative(default_schema),
        ]),
    );
}

fn summarize_types(original_type: Option<&Value>) -> Option<Value> {
    let mut types = match original_type {
        Some(Value::String(t)) => vec![t.clone()],
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect(),
        _ => return None,
    };
    if !types.iter().any(|t| t == "object") {
        types.push("object".to_string());
    }
    Some(Value::Array(types.into_iter().map(Value::String).collect()))
}

/// A schema is ineligible for a `from_env` alternative if it can only ever be satisfied by a
/// bare object or array - there's no plain string an env var could resolve to that would match.
fn is_object_or_array_only(map: &Map<String, Value>) -> bool {
    if map.contains_key("properties") || map.contains_key("additionalProperties") {
        return true;
    }
    if map.contains_key("items") || map.contains_key("prefixItems") {
        return true;
    }

    match map.get("type") {
        Some(Value::String(t)) => t == "object" || t == "array",
        Some(Value::Array(types)) => types
            .iter()
            .all(|t| matches!(t.as_str(), Some("object") | Some("array"))),
        _ => ["oneOf", "anyOf"]
            .into_iter()
            .find_map(|key| match map.get(key) {
                Some(Value::Array(variants)) => Some(variants),
                _ => None,
            })
            .is_some_and(|variants| {
                !variants
                    .iter()
                    .any(|v| matches!(v, Value::Object(m) if !is_object_or_array_only(m)))
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn augmented(schema: Value) -> Value {
        let Value::Object(mut map) = schema else {
            panic!("expected an object schema")
        };
        augment_field_map(&mut map);
        Value::Object(map)
    }

    #[test]
    fn wraps_a_plain_scalar() {
        let result = augmented(json!({ "type": "string" }));
        assert_eq!(
            result,
            json!({
                "type": ["string", "object"],
                "oneOf": [
                    { "type": "string" },
                    {
                        "type": "object",
                        "properties": {
                            "from_env": {
                                "type": "string",
                                "description": "Reads the value from an environment variable. In case of a missing value, a warning is logged and the field default/validation rules are applied as usual.",
                            },
                            "default": { "type": "string" },
                        },
                        "required": ["from_env"],
                        "additionalProperties": false
                    }
                ]
            })
        );
    }

    #[test]
    fn the_default_property_is_validated_against_the_fields_original_type() {
        let result = augmented(json!({
            "type": "integer",
            "minimum": 0,
            "maximum": 65535
        }));

        let from_env_branch = &result["oneOf"][1];
        assert_eq!(
            from_env_branch["properties"]["default"],
            json!({ "type": "integer", "minimum": 0, "maximum": 65535 })
        );
        // "default" is optional - only "from_env" is required.
        assert_eq!(from_env_branch["required"], json!(["from_env"]));
    }

    #[test]
    fn the_default_property_is_validated_against_the_original_union_for_wrapper_types() {
        // Mirrors ToggleWith<T>'s generated shape: oneOf[bool, T].
        let result = augmented(json!({
            "oneOf": [
                { "type": "boolean" },
                { "type": "string" }
            ]
        }));

        let from_env_branch = &result["oneOf"][2];
        assert_eq!(
            from_env_branch["properties"]["default"],
            json!({ "oneOf": [{ "type": "boolean" }, { "type": "string" }] })
        );
    }

    #[test]
    fn hoists_description_and_default_to_the_wrapper_top_level() {
        let result = augmented(json!({
            "description": "The port to bind to.",
            "type": "integer",
            "default": 4000,
            "minimum": 0
        }));

        assert_eq!(result["description"], json!("The port to bind to."));
        assert_eq!(result["default"], json!(4000));

        let variants = result["oneOf"].as_array().unwrap();
        assert_eq!(
            variants[0],
            json!({ "type": "integer", "minimum": 0 }),
            "the original branch keeps its type-shape keys but not the hoisted metadata"
        );
        assert_eq!(variants[1]["required"], json!(["from_env"]));
    }

    #[test]
    fn summarizes_a_nullable_types_array_instead_of_dropping_it() {
        let result = augmented(json!({ "type": ["string", "null"] }));
        assert_eq!(result["type"], json!(["string", "null", "object"]));
    }

    #[test]
    fn does_not_duplicate_object_in_the_type_summary() {
        let value = json!("object");
        assert_eq!(summarize_types(Some(&value)), Some(json!(["object"])));
    }

    #[test]
    fn omits_the_type_summary_when_there_is_nothing_to_summarize() {
        assert_eq!(summarize_types(None), None);
    }

    #[test]
    fn leaves_a_plain_object_alone_but_recurses_into_its_properties() {
        let result = augmented(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string" }
            }
        }));

        assert_eq!(result["type"], json!("object"));
        assert!(result.get("oneOf").is_none());
        assert!(result["properties"]["name"]["oneOf"].is_array());
    }

    #[test]
    fn leaves_a_hashmap_alone() {
        let result = augmented(json!({
            "type": "object",
            "additionalProperties": { "type": "string" }
        }));
        assert!(result.get("oneOf").is_none());
    }

    #[test]
    fn appends_a_single_from_env_alternative_to_an_existing_union_without_duplicating() {
        // Mirrors ToggleWith<T>'s generated shape: oneOf[bool, T].
        let result = augmented(json!({
            "oneOf": [
                { "type": "boolean" },
                { "type": "string" }
            ]
        }));

        let variants = result["oneOf"].as_array().unwrap();
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0], json!({ "type": "boolean" }));
        assert_eq!(variants[1], json!({ "type": "string" }));
        assert_eq!(variants[2]["required"], json!(["from_env"]));
    }

    #[test]
    fn does_not_add_an_alternative_to_an_all_object_union() {
        // Mirrors a tag-discriminated enum where every variant is a full object.
        let result = augmented(json!({
            "oneOf": [
                { "type": "object", "properties": { "type": { "const": "a" } } },
                { "type": "object", "properties": { "type": { "const": "b" } } }
            ]
        }));

        let variants = result["oneOf"].as_array().unwrap();
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn wraps_array_items_individually() {
        let result = augmented(json!({
            "type": "array",
            "items": { "type": "string" }
        }));

        assert!(result.get("oneOf").is_none());
        assert!(result["items"]["oneOf"].is_array());
    }
}
