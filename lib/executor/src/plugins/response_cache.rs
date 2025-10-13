use dashmap::DashMap;
use ntex::web::HttpResponse;
use redis::Commands;
use sonic_rs::json;

use crate::{
    plugins::traits::{
        ControlFlow, OnExecuteEnd, OnExecuteEndPayload, OnExecuteStart, OnExecuteStartPayload,
        OnSchemaReload, OnSchemaReloadPayload,
    },
    utils::consts::TYPENAME_FIELD_NAME,
};

pub struct ResponseCachePlugin {
    redis_client: redis::Client,
    ttl_per_type: DashMap<String, u64>,
}

impl ResponseCachePlugin {
    pub fn try_new(redis_url: &str) -> Result<Self, redis::RedisError> {
        let redis_client = redis::Client::open(redis_url)?;
        Ok(Self {
            redis_client,
            ttl_per_type: DashMap::new(),
        })
    }
}

pub struct ResponseCacheContext {
    key: String,
}

impl OnExecuteStart for ResponseCachePlugin {
    fn on_execute_start(&self, payload: OnExecuteStartPayload) -> ControlFlow {
        let key = format!(
            "response_cache:{}:{:?}",
            payload.query_plan, payload.variable_values
        );
        payload
            .router_http_request
            .extensions_mut()
            .insert(ResponseCacheContext { key: key.clone() });
        if let Ok(mut conn) = self.redis_client.get_connection() {
            let cached_response: Option<Vec<u8>> = conn.get(&key).ok();
            if let Some(cached_response) = cached_response {
                return ControlFlow::Break(
                    HttpResponse::Ok()
                        .header("Content-Type", "application/json")
                        .body(cached_response),
                );
            }
        }
        ControlFlow::Continue
    }
}

impl OnExecuteEnd for ResponseCachePlugin {
    fn on_execute_end(&self, payload: OnExecuteEndPayload) -> ControlFlow {
        // Do not cache if there are errors
        if !payload.errors.is_empty() {
            return ControlFlow::Continue;
        }
        if let Some(key) = payload
            .router_http_request
            .extensions()
            .get::<ResponseCacheContext>()
            .map(|ctx| &ctx.key)
        {
            if let Ok(mut conn) = self.redis_client.get_connection() {
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
                        .insert("response_cache_ttl".to_string(), json!(max_ttl));

                    // Set the cache with the decided ttl
                    let _: () = conn.set_ex(key, serialized, max_ttl).unwrap_or(());
                }
            }
        }
        ControlFlow::Continue
    }
}

impl OnSchemaReload for ResponseCachePlugin {
    fn on_schema_reload(&self, payload: OnSchemaReloadPayload) {
        // Visit the schema and update ttl_per_type based on some directive
        payload
            .new_schema
            .document
            .definitions
            .iter()
            .for_each(|def| {
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
    }
}
