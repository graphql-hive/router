use schemars::{json_schema, JsonSchema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePath(pub String);

impl JsonSchema for FilePath {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FilePath".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "format": "path"
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

impl From<String> for FilePath {
    fn from(value: String) -> Self {
        FilePath(value)
    }
}

impl From<&str> for FilePath {
    fn from(value: &str) -> Self {
        FilePath(value.to_string())
    }
}
