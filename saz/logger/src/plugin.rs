use hive_router::{
    async_trait,
    plugins::{
        hooks::{
            on_execute::{OnExecuteEndHookPayload, OnExecuteStartHookPayload, OnExecuteStartHookResult},
            on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult, OnGraphQLParamsEndHookPayload},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload, EndHookPayload},
    },
};

use serde_json::{json, Value};

#[derive(Default)]
pub struct GraphQLLoggerLayer;

/// Per-request log entry, stored in the request's PluginContext
/// (a field on the plugin would be shared by all concurrent requests).
struct LogMessage(Value);

fn normalize_header_name(name: &str) -> String {
    name.strip_prefix("x-")
        .unwrap_or(name)
        .split('-')
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part.to_string()
            } else {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                    None => String::new(),
                }
            }
        })
        .collect::<String>()
}

#[async_trait]
impl RouterPlugin for GraphQLLoggerLayer {
    type Config = ();
    fn plugin_name() -> &'static str {
        "logger"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }

    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        let request = payload.router_http_request;
        let headers = request.headers.clone();

        return payload.on_end(move |payload: OnGraphQLParamsEndHookPayload<'exec>| {
            let params = &payload.graphql_params;

            let query = params
                .query
                .clone()
                .unwrap_or_default();

            let variables = params.variables.clone();

            let extra_fields = [
                "x-correlation-id",
                "x-session-id",
                "x-app",
                "x-version",
                "x-device",
                "referer",
                "user-agent",
            ];

            let mut extra_fields_json = serde_json::Map::new();

            for name in extra_fields {
                if let Some(value) = headers.get(name) {
                    if let Ok(value_str) = value.to_str() {
                        if !value_str.is_empty() {
                            let normalized_name = normalize_header_name(name);
                            extra_fields_json.insert(normalized_name, json!(value_str));
                        }
                    }
                }
            }

            let ip = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    headers
                        .get("x-real-ip")
                        .and_then(|v| v.to_str().ok())
                        .filter(|v| !v.is_empty())
                });

            if let Some(ip) = ip {
                extra_fields_json.insert("ip".to_string(), json!(ip));
            }

            let mut log_message = serde_json::Map::new();

            if !query.is_empty() {
                log_message.insert("query".to_string(), json!(query));
            }

            if !variables.is_empty() {
                log_message.insert("variables".to_string(), json!(variables));
            }

            if !extra_fields_json.is_empty() {
                log_message.insert("extra".to_string(), Value::Object(extra_fields_json));
            }

            payload.context.insert(LogMessage(Value::Object(log_message)));

            return payload.proceed();
        });
    }

    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        let mut log_message = payload
            .context
            .get_mut::<LogMessage>()
            .map(|mut entry| entry.0.take())
            .filter(|v| !v.is_null())
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

        if let Some(operation_name) = payload
            .operation_for_plan
            .name
            .as_ref()
            .filter(|name| !name.is_empty())
        {
            if operation_name == "IntrospectionQuery" {
                return payload.proceed();
            }

            if let Value::Object(ref mut map) = log_message {
                map.insert("operationName".to_string(), json!(operation_name));
            }
        }

        return payload.on_end(move |payload: OnExecuteEndHookPayload<'exec>| {
            if !payload.errors.is_empty() {
                let errors: Vec<String> = payload.errors.clone()
                    .into_iter()
                    .map(|e| e.message.to_string())
                    .collect();

                if let Value::Object(ref mut map) = log_message {
                    map.insert("errors".to_string(), json!(errors));
                }

                if let Value::Object(ref mut map) = log_message {
                    map.insert("severity".to_string(), json!("ERROR"));
                    println!("{}", json!(map));
                }

                return payload.proceed();
            }

            if let Value::Object(ref mut map) = log_message {
                map.insert("severity".to_string(), json!("INFO"));
                println!("{}", json!(map));
            }

            return payload.proceed();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_header_name;

    #[test]
    fn normalizes_header_names_to_camel_case() {
        assert_eq!(normalize_header_name("x-correlation-id"), "correlationId");
        assert_eq!(normalize_header_name("x-app"), "app");
        assert_eq!(normalize_header_name("user-agent"), "userAgent");
        assert_eq!(normalize_header_name("referer"), "referer");
    }
}
