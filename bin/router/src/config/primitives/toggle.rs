use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone)]
pub enum ToggleWith<T: Default> {
    Disabled,
    Enabled(T),
}

impl<T: Default> ToggleWith<T> {
    pub const fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }

    pub const fn enabled_config(&self) -> Option<&T> {
        match self {
            Self::Enabled(config) => Some(config),
            Self::Disabled => None,
        }
    }
}

impl<'de, T> Deserialize<'de> for ToggleWith<T>
where
    T: Default + Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawToggleOr<T> {
            Toggle(bool),
            Config(T),
        }

        match RawToggleOr::deserialize(deserializer)? {
            RawToggleOr::Toggle(false) => Ok(Self::Disabled),
            RawToggleOr::Toggle(true) => Ok(Self::Enabled(T::default())),
            RawToggleOr::Config(config) => Ok(Self::Enabled(config)),
        }
    }
}

impl<T> Serialize for ToggleWith<T>
where
    T: Default + Serialize + PartialEq,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Disabled => serializer.serialize_bool(false),
            Self::Enabled(config) => {
                if *config == T::default() {
                    serializer.serialize_bool(true)
                } else {
                    config.serialize(serializer)
                }
            }
        }
    }
}

#[derive(JsonSchema)]
#[serde(untagged)]
#[allow(dead_code)]
enum ToggleWithSchema<T> {
    Toggle(bool),
    Config(T),
}

impl<T> JsonSchema for ToggleWith<T>
where
    T: Default + JsonSchema,
{
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("ToggleWith_{}", T::schema_name()).into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <ToggleWithSchema<T> as JsonSchema>::json_schema(generator)
    }

    fn inline_schema() -> bool {
        <ToggleWithSchema<T> as JsonSchema>::inline_schema()
    }
}
