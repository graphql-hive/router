use once_cell::sync::Lazy;
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{ser::SerializeStruct, Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap};
use vrl::{
    compiler::{compile as vrl_compile, Program as VrlProgram, TargetValue as VrlTargetValue},
    core::Value as VrlValue,
    prelude::{
        state::RuntimeState as VrlState, Context as VrlContext, ExpressionError, Function,
        TimeZone as VrlTimeZone,
    },
    stdlib::all as vrl_build_functions,
    value::Secrets as VrlSecrets,
};

#[derive(Debug, Clone)]
pub struct Expression {
    expression: String,
    program: Box<VrlProgram>,
}

static VRL_FUNCTIONS: Lazy<Vec<Box<dyn Function>>> = Lazy::new(vrl_build_functions);
static VRL_TIMEZONE: Lazy<VrlTimeZone> = Lazy::new(VrlTimeZone::default);

impl Expression {
    pub fn try_new(expression: String) -> Result<Self, String> {
        let compilation_result =
            vrl_compile(&expression, &VRL_FUNCTIONS).map_err(|diagnostics| {
                diagnostics
                    .errors()
                    .iter()
                    .map(|d| format!("{}: {}", d.code, d.message))
                    .collect::<Vec<_>>()
                    .join(", ")
            })?;

        Ok(Self {
            expression,
            program: Box::new(compilation_result.program),
        })
    }

    pub fn execute_with_value(&self, value: VrlValue) -> Result<VrlValue, ExpressionError> {
        let mut target = VrlTargetValue {
            value,
            metadata: VrlValue::Object(BTreeMap::new()),
            secrets: VrlSecrets::default(),
        };

        let mut state = VrlState::default();
        let mut ctx = VrlContext::new(&mut target, &mut state, &VRL_TIMEZONE);

        self.execute_with_context(&mut ctx)
    }

    pub fn execute_with_context(&self, ctx: &mut VrlContext) -> Result<VrlValue, ExpressionError> {
        self.program.resolve(ctx)
    }
}

impl<'de> Deserialize<'de> for Expression {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ExpressionVisitor;
        impl<'de> serde::de::Visitor<'de> for ExpressionVisitor {
            type Value = Expression;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a map for Expression")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut expression_str: Option<String> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "expression" => {
                            if expression_str.is_some() {
                                return Err(serde::de::Error::duplicate_field("expression"));
                            }
                            expression_str = Some(map.next_value()?);
                        }
                        other_key => {
                            return Err(serde::de::Error::unknown_field(
                                other_key,
                                &["expression"],
                            ));
                        }
                    }
                }

                let expression_str =
                    expression_str.ok_or_else(|| serde::de::Error::missing_field("expression"))?;

                Expression::try_new(expression_str).map_err(serde::de::Error::custom)
            }
        }
        deserializer.deserialize_map(ExpressionVisitor)
    }
}

impl Serialize for Expression {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("Expression", 1)?;
        state.serialize_field("expression", &self.expression)?;
        state.end()
    }
}

impl JsonSchema for Expression {
    fn schema_name() -> Cow<'static, str> {
        "Expression".into()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "description": "A VRL expression used for dynamic evaluations.",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "The VRL expression string."
                }
            },
            "required": ["expression"],
            "additionalProperties": false
        })
    }
}

impl TryFrom<String> for Expression {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Expression::try_new(value)
    }
}
