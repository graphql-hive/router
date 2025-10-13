use hive_router::{
    async_trait,
    http::StatusCode,
    plugins::{
        hooks::{
            on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
    },
    sonic_rs::{JsonContainerTrait, JsonValueTrait},
    DashMap, GraphQLError,
};

#[derive(Default)]
pub struct APQPlugin {
    cache: DashMap<String, String>,
}

#[async_trait]
impl RouterPlugin for APQPlugin {
    type Config = ();
    fn plugin_name() -> &'static str {
        "apq"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        payload.on_end(|mut payload| {
            let persisted_query_ext = payload
                .graphql_params
                .extensions
                .as_ref()
                .and_then(|ext| ext.get("persistedQuery"))
                .and_then(|pq| pq.as_object());
            if let Some(persisted_query_ext) = persisted_query_ext {
                match persisted_query_ext.get(&"version").and_then(|v| v.as_i64()) {
                    Some(1) => {}
                    _ => {
                        return payload.end_with_graphql_error(
                            GraphQLError::from_message_and_code(
                                "Unsupported persisted query version",
                                "UNSUPPORTED_PERSISTED_QUERY_VERSION",
                            ),
                            StatusCode::BAD_REQUEST,
                        );
                    }
                }
                let sha256_hash = match persisted_query_ext
                    .get(&"sha256Hash")
                    .and_then(|h| h.as_str())
                {
                    Some(h) => h,
                    None => {
                        return payload.end_with_graphql_error(
                            GraphQLError::from_message_and_code(
                                "Missing sha256Hash in persisted query",
                                "MISSING_PERSISTED_QUERY_HASH",
                            ),
                            StatusCode::BAD_REQUEST,
                        );
                    }
                };
                if let Some(query_param) = &payload.graphql_params.query {
                    // Store the query in the cache
                    self.cache
                        .insert(sha256_hash.to_string(), query_param.to_string());
                } else {
                    // Try to get the query from the cache
                    if let Some(cached_query) = self.cache.get(sha256_hash) {
                        // Update the graphql_params with the cached query
                        payload.graphql_params.query = Some(cached_query.value().to_string());
                    } else {
                        return payload.end_with_graphql_error(
                            GraphQLError::from_message_and_code(
                                "PersistedQueryNotFound",
                                "PERSISTED_QUERY_NOT_FOUND",
                            ),
                            StatusCode::BAD_REQUEST,
                        );
                    }
                }
            }

            payload.proceed()
        })
    }
}
