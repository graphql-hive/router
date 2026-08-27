use std::hash::{Hash, Hasher};

use hive_router::{
    async_trait,
    ntex::http::HeaderMap,
    plugins::{
        hooks::{
            on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::Deserialize;
use xxhash_rust::xxh3::Xxh3;

#[derive(Deserialize)]
pub struct DedupePartitionPluginConfig {
    /// Name of the cookie carrying the JWT, e.g. "session"
    pub cookie_name: String,
    /// HMAC secret used to validate the JWT. A real deployment would likely use JWKS instead.
    pub secret: String,
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
    #[allow(dead_code)]
    exp: usize,
}

pub struct DedupePartitionPlugin {
    cookie_name: String,
    decoding_key: DecodingKey,
    validation: Validation,
}

fn extract_cookie<'a>(headers: &'a HeaderMap, cookie_name: &str) -> Option<&'a str> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    cookie_header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == cookie_name).then_some(value)
    })
}

#[async_trait]
impl RouterPlugin for DedupePartitionPlugin {
    type Config = DedupePartitionPluginConfig;

    fn plugin_name() -> &'static str {
        "dedupe_partition"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        payload.initialize_plugin(Self {
            cookie_name: config.cookie_name,
            decoding_key: DecodingKey::from_secret(config.secret.as_bytes()),
            validation: Validation::default(),
        })
    }

    // Runs before the router computes the inbound dedupe fingerprint, so a partition
    // set here is picked up by the claim that follows.
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        let token = extract_cookie(payload.router_http_request.headers, &self.cookie_name);

        // A missing cookie and an invalid/expired token are treated identically: leave the
        // context untouched, so the request stays in the shared "unauthenticated" partition.
        if let Some(token) = token {
            if let Ok(token_data) = decode::<Claims>(token, &self.decoding_key, &self.validation) {
                // Partition by stable user identity: requests for the same `sub` coalesce,
                // requests for different users never do.
                let mut hasher = Xxh3::new();
                token_data.claims.sub.hash(&mut hasher);
                payload.add_inbound_dedupe_partition(hasher.finish());
            }
        }

        payload.proceed()
    }
}
