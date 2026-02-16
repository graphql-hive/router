use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use fibre::spmc;
use hive_router::{
    async_trait,
    http::StatusCode,
    ntex::{
        http::{
            body::{Body, ResponseBody},
            HeaderMap,
        },
        util::Bytes,
        web::{self},
    },
    plugins::{
        hooks::{
            on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
            on_http_request::{
                OnHttpRequestHookPayload, OnHttpRequestHookResult, OnHttpResponseHookPayload,
            },
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_context::RouterHttpRequest,
        plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
    },
    DashMap,
};
use serde::Deserialize;
use xxhash_rust::xxh3::Xxh3;

pub struct PluginContext {
    fingerprint: u64,
}

#[derive(Clone)]
struct SharedResponse {
    body: Bytes,
    headers: Arc<HeaderMap>,
    status: StatusCode,
}

pub struct IncomingRequestDeduplicationPlugin {
    receiver: spmc::topic::AsyncTopicReceiver<u64, SharedResponse>,
    sender: spmc::topic::AsyncTopicSender<u64, SharedResponse>,
    in_flight_requests: DashMap<u64, bool>,
    deduplicate_headers: Vec<String>,
}

#[derive(Deserialize)]
pub struct IncomingRequestDeduplicationPluginConfig {
    deduplicate_headers: Vec<String>,
}

impl IncomingRequestDeduplicationPlugin {
    pub fn get_fingerprint(&self, request: &RouterHttpRequest<'_>, body_bytes: &[u8]) -> u64 {
        let mut hasher = Xxh3::new();

        let mut headers = BTreeMap::new();
        for header_name in &self.deduplicate_headers {
            if let Some(header_value) = request.headers.get(header_name) {
                if let Ok(value_str) = header_value.to_str() {
                    headers.insert(header_name.as_str(), value_str);
                }
            }
        }

        request.method.hash(&mut hasher);
        request.path.hash(&mut hasher);
        headers.hash(&mut hasher);
        body_bytes.hash(&mut hasher);

        hasher.finish()
    }
}

#[async_trait]
impl RouterPlugin for IncomingRequestDeduplicationPlugin {
    type Config = IncomingRequestDeduplicationPluginConfig;
    fn plugin_name() -> &'static str {
        "incoming_request_deduplication"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        let (sender, receiver) = spmc::topic::channel_async(1000);
        payload.initialize_plugin(Self {
            sender,
            receiver,
            in_flight_requests: DashMap::new(),
            deduplicate_headers: config.deduplicate_headers,
        })
    }
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        let fingerprint = self.get_fingerprint(payload.router_http_request, &payload.body);

        if self.in_flight_requests.contains_key(&fingerprint) {
            let receiver = self.receiver.clone();
            receiver.subscribe(fingerprint);
            let (response_key, shared_response) = receiver.recv().await.unwrap();
            if fingerprint == response_key {
                // Remove from in-flight requests to allow future identical requests to proceed
                self.in_flight_requests.remove(&fingerprint);
                let mut response = web::HttpResponse::Ok();
                response.status(shared_response.status);
                for (header_name, header_value) in shared_response.headers.iter() {
                    response.set_header(header_name, header_value);
                }
                let response = response.body(shared_response.body);
                receiver.unsubscribe(&fingerprint);
                receiver
                    .close()
                    .expect("Expected to successfully close receiver");
                return payload.end_with_response(response);
            }
        } else {
            payload.context.insert(PluginContext { fingerprint });
            self.in_flight_requests.insert(fingerprint, true);
        }
        payload.proceed()
    }
    fn on_http_request<'req>(
        &'req self,
        payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        payload.on_end(|payload: OnHttpResponseHookPayload| {
            if let Some(context) = payload.context.get_ref::<PluginContext>() {
                return payload
                    .map_response(|web_response| {
                        // Take body, headers and status
                        let response = web_response.response();
                        if let ResponseBody::Body(Body::Bytes(bytes)) = response.body() {
                            let shared_response = SharedResponse {
                                body: bytes.clone(),
                                headers: response.headers().clone().into(),
                                status: response.status(),
                            };
                            // Send to all waiting receivers
                            self.sender
                                .send(context.fingerprint, shared_response)
                                .expect("Failed to send response to waiting receivers");
                        }
                        web_response
                    })
                    .proceed();
            }
            payload.proceed()
        })
    }
}
