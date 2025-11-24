use dashmap::DashMap;
use http::{HeaderMap, StatusCode};
use redis::Commands;
use serde::Deserialize;

use crate::{
    execution::plan::PlanExecutionOutput,
    hooks::{
        on_execute::{OnExecuteEndPayload, OnExecuteStartPayload},
        on_supergraph_load::{OnSupergraphLoadEndPayload, OnSupergraphLoadStartPayload},
    },
    plugin_trait::{EndPayload, HookResult, RouterPluginWithConfig, StartPayload},
    plugins::plugin_trait::RouterPlugin,
    utils::consts::TYPENAME_FIELD_NAME,
};

#[derive(Deserialize)]
pub struct ResponseCachePluginOptions {
    pub redis_url: String,
}

impl RouterPluginWithConfig for ResponseCachePlugin {
    type Config = ResponseCachePluginOptions;
    fn plugin_name() -> &'static str {
        "response_cache_plugin"
    }
    fn new(config: ResponseCachePluginOptions) -> Self {
        let redis_client = redis::Client::open(config.redis_url)
            .expect("Failed to create Redis client");
        Self {
            redis_client,
            ttl_per_type: DashMap::new(),
        }
    }
}

pub struct ResponseCachePlugin {
    redis_client: redis::Client,
    ttl_per_type: DashMap<String, u64>,
}

#[async_trait::async_trait]
impl RouterPlugin for ResponseCachePlugin {
    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartPayload<'exec>,
    ) -> HookResult<'exec, OnExecuteStartPayload<'exec>, OnExecuteEndPayload<'exec>> {
        let key = format!(
            "response_cache:{}:{:?}",
            payload.query_plan, payload.variable_values
        );
        if let Ok(mut conn) = self.redis_client.get_connection() {
            let cached_response: Option<Vec<u8>> = conn.get(&key).ok();
            if let Some(cached_response) = cached_response {
                return payload.end_response(PlanExecutionOutput {
                    body: cached_response,
                    headers: HeaderMap::new(),
                    status: StatusCode::OK,
                });
            }
            return payload.on_end(move |mut payload: OnExecuteEndPayload<'exec>| {
                // Do not cache if there are errors
                if !payload.errors.is_empty() {
                    return payload.cont();
                }

                if let Ok(serialized) = sonic_rs::to_vec(&payload.data) {
                    // Decide on the ttl somehow
                    // Get the type names
                    let mut max_ttl = 0;

                    // Imagine this code is traversing the response data to find type names
                    if let Some(obj) = payload.data.as_object() {
                        if let Some(typename) = obj
                            .iter()
                            .position(|(k, _)| k == &TYPENAME_FIELD_NAME)
                            .and_then(|idx| obj[idx].1.as_str())
                        {
                            if let Some(ttl) = self.ttl_per_type.get(typename).map(|v| *v) {
                                max_ttl = max_ttl.max(ttl);
                            }
                        }
                    }

                    // If no ttl found, default to 60 seconds
                    if max_ttl == 0 {
                        max_ttl = 60;
                    }

                    // Insert the ttl into extensions for client awareness
                    payload
                        .extensions
                        .get_or_insert_default()
                        .insert("response_cache_ttl".to_string(), sonic_rs::json!(max_ttl));

                    // Set the cache with the decided ttl
                    let _: () = conn.set_ex(key, serialized, max_ttl).unwrap_or(());
                }
                payload.cont()
            });
        }
        payload.cont()
    }
    fn on_supergraph_reload<'a>(
        &'a self,
        payload: OnSupergraphLoadStartPayload,
    ) -> HookResult<'a, OnSupergraphLoadStartPayload, OnSupergraphLoadEndPayload> {
        // Visit the schema and update ttl_per_type based on some directive
        payload.new_ast.definitions.iter().for_each(|def| {
            if let graphql_parser::schema::Definition::TypeDefinition(type_def) = def {
                if let graphql_parser::schema::TypeDefinition::Object(obj_type) = type_def {
                    for directive in &obj_type.directives {
                        if directive.name == "cacheControl" {
                            for arg in &directive.arguments {
                                if arg.0 == "maxAge" {
                                    if let graphql_parser::query::Value::Int(max_age) = &arg.1 {
                                        if let Some(max_age) = max_age.as_i64() {
                                            self.ttl_per_type
                                                .insert(obj_type.name.clone(), max_age as u64);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        payload.cont()
    }
}
