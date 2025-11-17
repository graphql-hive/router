use dashmap::DashMap;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};

use crate::{
    hooks::on_deserialization::{OnDeserializationEndPayload, OnDeserializationStartPayload},
    plugin_trait::{EndPayload, HookResult, RouterPlugin, StartPayload},
};

pub struct APQPlugin {
    cache: DashMap<String, String>,
}

impl RouterPlugin for APQPlugin {
    fn on_deserialization<'exec>(
        &'exec self,
        start_payload: OnDeserializationStartPayload<'exec>,
    ) -> HookResult<'exec, OnDeserializationStartPayload<'exec>, OnDeserializationEndPayload<'exec>>
    {
        start_payload.on_end(|mut end_payload| {
            let persisted_query_ext = end_payload.graphql_params.extensions.as_ref()
                .and_then(|ext| ext.get("persistedQuery"))
                .and_then(|pq| pq.as_object());
            if let Some(persisted_query_ext) = persisted_query_ext {
                match persisted_query_ext.get(&"version").and_then(|v| v.as_str()) {
                    Some("1") => {}
                    _ => {
                        // TODO: Error for unsupported version
                        return end_payload.cont();
                    }
                }
                let sha256_hash = match persisted_query_ext.get(&"sha256Hash").and_then(|h| h.as_str()) {
                    Some(h) => h,
                    None => {
                        return end_payload.cont();
                    }
                };
                if let Some(query_param) = &end_payload.graphql_params.query {
                    // Store the query in the cache
                    self.cache.insert(sha256_hash.to_string(), query_param.to_string());
                } else {
                    // Try to get the query from the cache
                    if let Some(cached_query) = self.cache.get(sha256_hash) {
                        // Update the graphql_params with the cached query
                        end_payload.graphql_params.query = Some(cached_query.value().to_string());
                    } else {
                        // Error
                        return end_payload.cont();
                    }
                }
            }

            end_payload.cont()
        })
    }
}
