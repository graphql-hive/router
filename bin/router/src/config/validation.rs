use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ValidationConfig {
    /// Accepts input object variables that hold fields their type doesn't define, instead of
    /// rejecting the request. The value goes to the subgraphs as sent, unknown fields included.
    ///
    /// Handy when clients send back objects they got from the API, e.g. with `__typename` still
    /// in them. Only variables are affected, values written inline in the operation are still
    /// validated.
    ///
    /// Off by default, so unknown fields are rejected.
    ///
    /// ```yaml
    /// validation:
    ///   disable_unknown_input_fields_validation: true
    /// ```
    #[serde(default)]
    pub disable_unknown_input_fields_validation: bool,

    /// Accepts `Int` values outside the 32-bit signed range, instead of rejecting them. Applies
    /// to both variables and values written inline in the operation.
    ///
    /// Off by default. You can override this setting by setting the
    /// `ROUTER_DISABLE_INT_VALUE_RANGE_VALIDATION` environment variable to `true` or `1`.
    ///
    /// ```yaml
    /// validation:
    ///   disable_int_value_range_validation: true
    /// ```
    #[serde(default)]
    pub disable_int_value_range_validation: bool,
}
