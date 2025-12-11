use std::env as std_env;
use vrl::compiler::prelude::{
    kind, ArgumentList, Compiled, Context, Example, Expression, ExpressionError, Function,
    FunctionCompileContext, FunctionExpression, Parameter, Resolved, TypeDef, TypeState, Value,
};

#[derive(Clone, Copy, Debug)]
pub struct Env;

impl Function for Env {
    fn identifier(&self) -> &'static str {
        "env"
    }

    fn parameters(&self) -> &'static [Parameter] {
        &[
            Parameter {
                // The name of the environment variable.
                keyword: "name",
                kind: kind::BYTES, // VRL strings are bytes under the hood
                required: true,
            },
            Parameter {
                // A fallback value if the environment variable is not set.
                keyword: "default",
                kind: kind::BYTES,
                required: false,
            },
            Parameter {
                // If `true`, treats an empty environment variable as unset.
                keyword: "treat_empty_as_unset",
                kind: kind::BOOLEAN,
                required: false,
            },
        ]
    }

    fn examples(&self) -> &'static [Example] {
        &[
            Example {
                title: "Get an environment variable",
                source: r#"env("OTEL_EXPORTER_OTLP_ENDPOINT")"#,
                result: Ok("http://collector:4317"),
            },
            Example {
                title: "Default when unset",
                source: r#"env("MISSING_VAR", "fallback")"#,
                result: Ok("fallback"),
            },
            Example {
                title: "Default when unset or empty",
                source: r#"env("MAYBE_EMPTY", "fallback", true)"#,
                result: Ok("fallback"),
            },
        ]
    }

    fn compile(
        &self,
        _state: &TypeState,
        _ctx: &mut FunctionCompileContext,
        arguments: ArgumentList,
    ) -> Compiled {
        let name = arguments.required("name");
        let default = arguments.optional("default");
        let treat_empty_as_unset = arguments.optional("treat_empty_as_unset");

        Ok(EnvFn {
            name,
            default,
            treat_empty_as_unset,
        }
        .as_expr())
    }
}

#[derive(Debug, Clone)]
struct EnvFn {
    name: Box<dyn Expression>,
    default: Option<Box<dyn Expression>>,
    treat_empty_as_unset: Option<Box<dyn Expression>>,
}

fn resolve_env(
    env_opt: Option<String>,
    default_val: Option<Value>,
    treat_empty_as_unset: bool,
) -> Value {
    if let Some(v) = env_opt {
        // The variable is set. Return its value unless it's empty and we should treat it as unset.
        if !v.is_empty() || !treat_empty_as_unset {
            return Value::from(v);
        }
    }

    // The environment variable is not set, or it is empty and `treat_empty_as_unset` is true.
    default_val.unwrap_or(Value::Null)
}

impl FunctionExpression for EnvFn {
    fn resolve(&self, ctx: &mut Context) -> Resolved {
        let name_val = self.name.resolve(ctx)?;
        let name_str = name_val.as_str().ok_or_else(|| ExpressionError::Error {
            message: "env(:name) parameter must be a string".to_string(),
            labels: vec![],
            notes: vec![],
        })?;

        let default_val = self
            .default
            .as_ref()
            .map(|expr| expr.resolve(ctx))
            .transpose()?;

        let treat_empty_as_unset = self
            .treat_empty_as_unset
            .as_ref()
            .map(|expr| {
                expr.resolve(ctx)?
                    .as_boolean()
                    .ok_or_else(|| ExpressionError::Error {
                        message: "env(?, ?, :treat_empty_as_unset) parameter must be a boolean"
                            .to_string(),
                        labels: vec![],
                        notes: vec![],
                    })
            })
            .transpose()?
            .unwrap_or(false);

        // Read env var
        let env_opt = std_env::var(name_str.as_ref()).ok();

        Ok(resolve_env(env_opt, default_val, treat_empty_as_unset))
    }

    fn type_def(&self, _: &TypeState) -> TypeDef {
        // env() returns bytes or null, never throws at runtime
        TypeDef::bytes().add_null().infallible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_empty_false_var_set_with_value() {
        let result = resolve_env(Some("value".to_string()), None, false);
        assert_eq!(result, Value::from("value"));
    }

    #[test]
    fn test_non_empty_false_var_set_empty() {
        let default = Some(Value::from("default"));
        let result = resolve_env(Some("".to_string()), default.clone(), false);
        // With treat_empty_as_unset=false, empty string should be returned as-is
        assert_eq!(result, Value::from(""));
    }

    #[test]
    fn test_non_empty_false_var_unset_no_default() {
        let result = resolve_env(None, None, false);
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_non_empty_false_var_unset_with_default() {
        let default = Some(Value::from("default"));
        let result = resolve_env(None, default.clone(), false);
        assert_eq!(result, default.unwrap());
    }

    #[test]
    fn test_non_empty_true_var_set_with_value() {
        let result = resolve_env(Some("value".to_string()), None, true);
        assert_eq!(result, Value::from("value"));
    }

    #[test]
    fn test_non_empty_true_var_set_empty_with_default() {
        let default = Some(Value::from("default"));
        let result = resolve_env(Some("".to_string()), default.clone(), true);
        // With treat_empty_as_unset=true, empty string should be treated as unset
        assert_eq!(result, default.unwrap());
    }

    #[test]
    fn test_non_empty_true_var_set_empty_no_default() {
        let result = resolve_env(Some("".to_string()), None, true);
        // With treat_empty_as_unset=true and no default, empty string returns null
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_non_empty_true_var_unset_no_default() {
        let result = resolve_env(None, None, true);
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_non_empty_true_var_unset_with_default() {
        let default = Some(Value::from("default"));
        let result = resolve_env(None, default.clone(), true);
        assert_eq!(result, default.unwrap());
    }
}
