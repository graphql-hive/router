use config_rs::{Map, Value, ValueKind};

const FROM_ENV_KEY: &str = "from_env";
const DEFAULT_KEY: &str = "default";

/// Recursively resolves `{ from_env: "VAR" }` (optionally with `default: <value>`) placeholders
/// anywhere in a parsed config value tree, in place, before it gets deserialized into typed
/// config structs.
///
/// A placeholder is replaced by the referenced environment variable's value (as a string,
/// letting the normal field deserialization coerce it into the target type). If the variable is
/// not set:
/// - with a `default` given, the placeholder is replaced by that value instead;
/// - otherwise, the placeholder is dropped entirely, so the field is treated as if it was never
///   present in the config file: `#[serde(default = ...)]` kicks in, or deserialization fails
///   with the usual "missing field" error for required fields.
///
/// Returns a human-readable warning per unset variable. These aren't logged here directly:
/// this runs before the tracing subscriber exists (its own format/level comes from this same
/// config), so callers should log them once that subscriber is set up.
pub fn resolve_from_env_placeholders(root: &mut Value) -> Vec<String> {
    let mut warnings = Vec::new();
    if let ValueKind::Table(table) = &mut root.kind {
        resolve_table(table, &mut warnings);
    }
    warnings
}

fn resolve_table(table: &mut Map<String, Value>, warnings: &mut Vec<String>) {
    table.retain(|key, value| resolve_and_keep(value, key, warnings));
}

fn resolve_array(array: &mut Vec<Value>, warnings: &mut Vec<String>) {
    array.retain_mut(|value| resolve_and_keep(value, "<array item>", warnings));
}

fn resolve_and_keep(value: &mut Value, label: &str, warnings: &mut Vec<String>) -> bool {
    let Some(marker) = take_from_env_marker(value) else {
        match &mut value.kind {
            ValueKind::Table(table) => resolve_table(table, warnings),
            ValueKind::Array(array) => resolve_array(array, warnings),
            _ => {}
        }
        return true;
    };

    if let Ok(resolved) = std::env::var(&marker.var_name) {
        value.kind = ValueKind::String(resolved);
        return true;
    }

    match marker.default {
        Some(default_value) => {
            warnings.push(format!(
                "environment variable `{}` referenced by `from_env` for '{label}' is not set; using its default value instead",
                marker.var_name
            ));
            *value = default_value;
            true
        }
        None => {
            warnings.push(format!(
                "environment variable `{}` referenced by `from_env` for '{label}' is not set; treating '{label}' as unset",
                marker.var_name
            ));
            false
        }
    }
}

struct FromEnvMarker {
    var_name: String,
    default: Option<Value>,
}

/// Matches `{ from_env: <string> }` or `{ from_env: <string>, default: <anything> }` exactly -
/// no other keys allowed, so a genuine multi-key config table is never mistaken for a marker.
/// On a match, consumes both keys out of the table so nothing marker-shaped leaks downstream.
fn take_from_env_marker(value: &mut Value) -> Option<FromEnvMarker> {
    let ValueKind::Table(table) = &value.kind else {
        return None;
    };
    let is_marker = matches!(table.get(FROM_ENV_KEY), Some(v) if matches!(v.kind, ValueKind::String(_)))
        && table.keys().all(|k| k == FROM_ENV_KEY || k == DEFAULT_KEY);
    if !is_marker {
        return None;
    }

    let ValueKind::Table(table) = &mut value.kind else {
        unreachable!("checked above")
    };
    let var_name = match table.remove(FROM_ENV_KEY).map(|v| v.kind) {
        Some(ValueKind::String(var_name)) => var_name,
        _ => unreachable!("checked above"),
    };
    let default = table.remove(DEFAULT_KEY);
    Some(FromEnvMarker { var_name, default })
}

#[cfg(test)]
mod tests {
    use super::*;
    use config_rs::{Config, File, FileFormat};
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq, Default)]
    struct Nested {
        #[serde(default)]
        values: Vec<String>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Sample {
        #[serde(default = "default_greeting")]
        greeting: String,
        #[serde(default)]
        enabled: bool,
        #[serde(default)]
        nested: Nested,
        required: String,
    }

    fn default_greeting() -> String {
        "hi".to_string()
    }

    fn build(yaml: &str) -> Result<Sample, config_rs::ConfigError> {
        build_with_warnings(yaml).0
    }

    fn build_with_warnings(yaml: &str) -> (Result<Sample, config_rs::ConfigError>, Vec<String>) {
        let mut built = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap();
        let warnings = resolve_from_env_placeholders(&mut built.cache);
        (built.try_deserialize::<Sample>(), warnings)
    }

    #[test]
    fn resolves_scalar_from_env() {
        unsafe { std::env::set_var("FROM_ENV_TEST_GREETING", "hello there") };
        let sample =
            build("required: r\ngreeting: { from_env: FROM_ENV_TEST_GREETING }\n").unwrap();
        assert_eq!(sample.greeting, "hello there");
        unsafe { std::env::remove_var("FROM_ENV_TEST_GREETING") };
    }

    #[test]
    fn coerces_bool_from_env_string() {
        unsafe { std::env::set_var("FROM_ENV_TEST_ENABLED", "true") };
        let sample = build("required: r\nenabled: { from_env: FROM_ENV_TEST_ENABLED }\n").unwrap();
        assert!(sample.enabled);
        unsafe { std::env::remove_var("FROM_ENV_TEST_ENABLED") };
    }

    #[test]
    fn missing_env_var_falls_back_to_default() {
        unsafe { std::env::remove_var("FROM_ENV_TEST_MISSING") };
        let sample = build("required: r\ngreeting: { from_env: FROM_ENV_TEST_MISSING }\n").unwrap();
        assert_eq!(sample.greeting, "hi");
    }

    #[test]
    fn missing_env_var_produces_a_warning_instead_of_logging_directly() {
        unsafe { std::env::remove_var("FROM_ENV_TEST_MISSING_WARN") };
        let (result, warnings) = build_with_warnings(
            "required: r\ngreeting: { from_env: FROM_ENV_TEST_MISSING_WARN }\n",
        );
        result.unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("FROM_ENV_TEST_MISSING_WARN"));
        assert!(warnings[0].contains("greeting"));
    }

    #[test]
    fn missing_env_var_on_required_field_fails() {
        unsafe { std::env::remove_var("FROM_ENV_TEST_MISSING_REQUIRED") };
        let err = build("required: { from_env: FROM_ENV_TEST_MISSING_REQUIRED }\n").unwrap_err();
        assert!(err.to_string().contains("required"));
    }

    #[test]
    fn resolves_inside_array_elements() {
        unsafe { std::env::set_var("FROM_ENV_TEST_ITEM", "b") };
        let sample = build(
            "required: r\nnested:\n  values:\n    - a\n    - { from_env: FROM_ENV_TEST_ITEM }\n",
        )
        .unwrap();
        assert_eq!(sample.nested.values, vec!["a".to_string(), "b".to_string()]);
        unsafe { std::env::remove_var("FROM_ENV_TEST_ITEM") };
    }

    #[test]
    fn missing_env_var_uses_the_inline_default_instead_of_the_field_default() {
        unsafe { std::env::remove_var("FROM_ENV_TEST_MISSING_WITH_DEFAULT") };
        let sample = build(
            "required: r\ngreeting: { from_env: FROM_ENV_TEST_MISSING_WITH_DEFAULT, default: bonjour }\n",
        )
        .unwrap();
        assert_eq!(
            sample.greeting, "bonjour",
            "not the field's own default (\"hi\")"
        );
    }

    #[test]
    fn missing_env_var_with_default_still_warns_but_mentions_the_default_was_used() {
        unsafe { std::env::remove_var("FROM_ENV_TEST_MISSING_WITH_DEFAULT_WARN") };
        let (result, warnings) = build_with_warnings(
            "required: r\ngreeting: { from_env: FROM_ENV_TEST_MISSING_WITH_DEFAULT_WARN, default: bonjour }\n",
        );
        result.unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("FROM_ENV_TEST_MISSING_WITH_DEFAULT_WARN"));
        assert!(warnings[0].contains("default"));
    }

    #[test]
    fn present_env_var_wins_over_the_inline_default() {
        unsafe { std::env::set_var("FROM_ENV_TEST_PRESENT_WITH_DEFAULT", "hello there") };
        let sample = build(
            "required: r\ngreeting: { from_env: FROM_ENV_TEST_PRESENT_WITH_DEFAULT, default: bonjour }\n",
        )
        .unwrap();
        assert_eq!(sample.greeting, "hello there");
        unsafe { std::env::remove_var("FROM_ENV_TEST_PRESENT_WITH_DEFAULT") };
    }

    #[test]
    fn missing_env_var_with_a_non_string_default_still_coerces_correctly() {
        unsafe { std::env::remove_var("FROM_ENV_TEST_MISSING_BOOL_DEFAULT") };
        let sample = build(
            "required: r\nenabled: { from_env: FROM_ENV_TEST_MISSING_BOOL_DEFAULT, default: true }\n",
        )
        .unwrap();
        assert!(sample.enabled);
    }

    #[test]
    fn does_not_confuse_a_real_multi_key_table() {
        let sample = build("required: r\ngreeting: hi\nnested: { values: [] }\n").unwrap();
        assert_eq!(
            sample,
            Sample {
                greeting: "hi".into(),
                enabled: false,
                nested: Nested { values: vec![] },
                required: "r".into(),
            }
        );
    }

    #[test]
    fn a_table_with_from_env_plus_an_unrelated_key_is_not_a_marker() {
        let mut value = Value::new(
            None,
            ValueKind::Table(Map::from_iter([
                (
                    "from_env".to_string(),
                    Value::new(None, ValueKind::String("X".to_string())),
                ),
                (
                    "something_else".to_string(),
                    Value::new(None, ValueKind::String("y".to_string())),
                ),
            ])),
        );
        assert!(take_from_env_marker(&mut value).is_none());
    }
}
