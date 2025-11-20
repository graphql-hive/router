use dashmap::DashMap;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};

use crate::{
    hooks::on_graphql_params::{OnGraphQLParamsEndPayload, OnGraphQLParamsStartPayload},
    plugin_trait::{EndPayload, HookResult, RouterPlugin, StartPayload},
};

pub struct APQPlugin {
    cache: DashMap<String, String>,
}

#[async_trait::async_trait]
impl RouterPlugin for APQPlugin {
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLParamsStartPayload<'exec>, OnGraphQLParamsEndPayload> {
        payload.on_end(|mut payload| {
            let persisted_query_ext = payload
                .graphql_params
                .extensions
                .as_ref()
                .and_then(|ext| ext.get("persistedQuery"))
                .and_then(|pq| pq.as_object());
            if let Some(persisted_query_ext) = persisted_query_ext {
                match persisted_query_ext.get(&"version").and_then(|v| v.as_str()) {
                    Some("1") => {}
                    _ => {
                        // TODO: Error for unsupported version
                        return payload.cont();
                    }
                }
                let sha256_hash = match persisted_query_ext
                    .get(&"sha256Hash")
                    .and_then(|h| h.as_str())
                {
                    Some(h) => h,
                    None => {
                        return payload.cont();
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
                        // Error
                        return payload.cont();
                    }
                }
            }

            payload.cont()
        })
    }
}
