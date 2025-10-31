use std::{borrow::Cow, collections::BTreeMap};

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use vrl::{
    compiler::{compile as vrl_compile, Program as VrlProgram, TargetValue as VrlTargetValue},
    core::Value as VrlValue,
    prelude::{
        state::RuntimeState as VrlState, Context as VrlContext, ExpressionError,
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

impl Expression {
    pub fn try_new(expression: String) -> Result<Self, String> {
        let vrl_functions = vrl_build_functions();

        let compilation_result =
            vrl_compile(&expression, &vrl_functions).map_err(|diagnostics| {
                diagnostics
                    .errors()
                    .into_iter()
                    .map(|d| d.code.to_string() + ": " + &d.message)
                    .collect::<Vec<_>>()
                    .join(", ")
            })?;

        Ok(Self {
            expression,
            program: Box::new(compilation_result.program),
        })
    }

    pub fn execute(&self, value: VrlValue) -> Result<VrlValue, ExpressionError> {
        let mut target = VrlTargetValue {
            value,
            metadata: VrlValue::Object(BTreeMap::new()),
            secrets: VrlSecrets::default(),
        };

        let mut state = VrlState::default();
        let timezone = VrlTimeZone::default();
        let mut ctx = VrlContext::new(&mut target, &mut state, &timezone);

        self.program.resolve(&mut ctx)
    }
}

impl<'de> Deserialize<'de> for Expression {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let expression = String::deserialize(deserializer)?;
        Expression::try_new(expression).map_err(serde::de::Error::custom)
    }
}

impl Serialize for Expression {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.expression)
    }
}

impl JsonSchema for Expression {
    fn schema_name() -> Cow<'static, str> {
        "Expression".into()
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        String::json_schema(gen)
    }
}

impl TryFrom<String> for Expression {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Expression::try_new(value)
    }
}
