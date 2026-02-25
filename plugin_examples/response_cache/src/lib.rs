use std::collections::HashMap;

use hive_router::http::StatusCode;
use hive_router::plugins::hooks::on_execute::{
    OnExecuteEndHookPayload, OnExecuteStartHookPayload, OnExecuteStartHookResult,
};
use hive_router::plugins::hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult};
use hive_router::plugins::hooks::on_supergraph_load::{
    OnSupergraphLoadStartHookPayload, OnSupergraphLoadStartHookResult,
};
use hive_router::plugins::plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload};
use hive_router::ArcSwap;
use hive_router::{async_trait, graphql_tools, sonic_rs, PlanExecutionOutput};
use redis::Commands;
use serde::Deserialize;

use hive_router::tracing::trace;

#[derive(Deserialize)]
pub struct ResponseCachePluginOptions {
    pub redis_url: String,
    #[serde(default = "default_ttl_seconds")]
    pub default_ttl_seconds: u64,
}

fn default_ttl_seconds() -> u64 {
    5
}

pub struct ResponseCachePlugin {
    redis: r2d2::Pool<redis::Client>,
    ttl_per_type: ArcSwap<HashMap<String, u64>>,
    default_ttl_seconds: u64,
}

#[async_trait]
impl RouterPlugin for ResponseCachePlugin {
    type Config = ResponseCachePluginOptions;
    fn plugin_name() -> &'static str {
        "response_cache_plugin"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        let redis_client = redis::Client::open(config.redis_url.as_str())?;
        let pool = r2d2::Pool::builder().build(redis_client)?;
        payload.initialize_plugin(Self {
            redis: pool,
            ttl_per_type: Default::default(),
            default_ttl_seconds: config.default_ttl_seconds,
        })
    }
    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        let key = format!(
            "response_cache:{}:{:?}",
            payload.query_plan, payload.variable_values
        );
        if let Ok(mut conn) = self.redis.get() {
            trace!("Checking cache for key: {}", key);
            let cache_result: Result<Vec<u8>, redis::RedisError> = conn.get(&key);
            match cache_result {
                Ok(body) => {
                    if body.is_empty() {
                        trace!("Cache miss for key: {}", key);
                    } else {
                        trace!(
                            "Cache hit for key: {} -> {}",
                            key,
                            String::from_utf8_lossy(&body)
                        );
                        return payload.end_with_response(PlanExecutionOutput {
                            body,
                            error_count: 0,
                            response_headers_aggregator: None,
                            status_code: StatusCode::OK,
                        });
                    }
                }
                Err(err) => {
                    trace!("Error accessing cache for key {}: {}", key, err);
                }
            }
            return payload.on_end(move |mut payload: OnExecuteEndHookPayload<'exec>| {
                // Do not cache if there are errors
                if !payload.errors.is_empty() {
                    trace!("Not caching response due to errors");
                    return payload.proceed();
                }

                if let Ok(serialized) = sonic_rs::to_vec(&payload.data) {
                    let ttl_per_type = self.ttl_per_type.load();
                    trace!("Caching response for key: {}", key);
                    // Decide on the ttl somehow
                    // Get the type names
                    let mut max_ttl = 0;

                    // Imagine this code is traversing the response data to find type names
                    if let Some(obj) = payload.data.as_object() {
                        if let Some(typename) = obj
                            .iter()
                            .position(|(k, _)| k == &"__typename")
                            .and_then(|idx| obj[idx].1.as_str())
                        {
                            if let Some(ttl) = ttl_per_type.get(typename) {
                                max_ttl = max_ttl.max(*ttl);
                            }
                        }
                    }

                    // If no ttl found, default
                    if max_ttl == 0 {
                        max_ttl = self.default_ttl_seconds;
                    }
                    trace!("Using TTL of {} seconds for key: {}", max_ttl, key);

                    // Insert the ttl into extensions for client awareness
                    payload.add_extension("response_cache_ttl", max_ttl);

                    // Set the cache with the decided ttl
                    let result = conn.set_ex::<&str, Vec<u8>, ()>(&key, serialized, max_ttl);
                    if let Err(err) = result {
                        trace!("Failed to set cache for key {}: {}", key, err);
                    } else {
                        trace!("Cached response for key: {} with TTL: {}", key, max_ttl);
                    }
                }
                payload.proceed()
            });
        }
        payload.proceed()
    }
    fn on_supergraph_reload<'a>(
        &'a self,
        payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'a> {
        let mut ttl_per_type = HashMap::new();
        // Visit the schema and update ttl_per_type based on some directive
        payload.new_ast.definitions.iter().for_each(|def| {
            if let graphql_tools::parser::schema::Definition::TypeDefinition(
                graphql_tools::parser::schema::TypeDefinition::Object(obj_type),
            ) = def
            {
                for directive in &obj_type.directives {
                    if directive.name == "cacheControl" {
                        for arg in &directive.arguments {
                            if arg.0 == "maxAge" {
                                if let graphql_tools::parser::query::Value::Int(max_age) = &arg.1 {
                                    if let Some(max_age) = max_age.as_i64() {
                                        ttl_per_type.insert(obj_type.name.clone(), max_age as u64);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        self.ttl_per_type.store(ttl_per_type.into());

        payload.proceed()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use hive_router::ntex::rt::tokio;
    use hive_router::tracing::trace;
    use hive_router::{http::StatusCode, ntex};

    use e2e::testkit::{
        init_graphql_request, init_router_from_config_file_with_plugins, wait_for_readiness,
        SubgraphsServer, TestDockerContainer, TestDockerContainerOpts,
    };

    #[ntex::test]
    async fn test_caching_with_default_ttl() {
        let container = TestDockerContainer::async_new(TestDockerContainerOpts {
            name: "redis_resp_caching_test".to_string(),
            image: "redis/redis-stack:latest".to_string(),
            ports: HashMap::from([(6379, 6379)]),
            env: vec!["ALLOW_EMPTY_PASSWORD=yes".to_string()],
            ..Default::default()
        })
        .await
        .expect("failed to start redis container");

        // Redis flush all to ensure clean state
        container
            .exec(vec!["redis-cli", "FLUSHALL"])
            .await
            .expect("Failed to flush redis");
        let subgraphs_server = SubgraphsServer::start().await;

        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/response_cache/router.config.yaml",
            hive_router::PluginRegistry::new().register::<super::ResponseCachePlugin>(),
        )
        .await
        .expect("failed to start router");

        wait_for_readiness(&app.app).await;

        let req = init_graphql_request("{ users { id } }", None);
        let resp = ntex::web::test::call_service(&app.app, req.to_request()).await;
        trace!("First response received");
        assert_eq!(resp.status(), StatusCode::OK);
        let resp_body = ntex::web::test::read_body(resp).await;
        trace!(
            "Response body read: {:?}",
            String::from_utf8_lossy(&resp_body)
        );
        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("Failed to get subgraph requests log");
        assert_eq!(subgraph_requests.len(), 1, "Expected one subgraph request");
        let req = init_graphql_request("{ users { id } }", None);
        let resp2 = ntex::web::test::call_service(&app.app, req.to_request()).await;
        trace!("Second response received");
        assert!(resp2.status().is_success());
        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("Failed to get subgraph requests log");
        assert_eq!(
            subgraph_requests.len(),
            1,
            "Expected only one subgraph request due to caching"
        );
        trace!("Waiting for cache to expire...");
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let req = init_graphql_request("{ users { id } }", None);
        let resp3 = ntex::web::test::call_service(&app.app, req.to_request()).await;
        assert!(resp3.status().is_success());
        let subgraph_requests = subgraphs_server
            .get_subgraph_requests_log("accounts")
            .await
            .expect("Failed to get subgraph requests log");
        assert_eq!(
            subgraph_requests.len(),
            2,
            "Expected a second subgraph request after cache expiry"
        );
        container.stop().await;
    }
}
