use hive_router::{
    ntex::http::header::{HeaderName, HeaderValue},
    plugins::{
        hooks::{
            on_http_request::{OnHttpRequestHookFuture, OnHttpRequestHookPayload},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
    },
};
use serde::Deserialize;

const FETCHED_GREETING_HEADER: HeaderName = HeaderName::from_static("x-fetched-greeting");

#[derive(Deserialize)]
pub struct AsyncHttpFetchConfig {
    pub upstream_url: String,
}

pub struct AsyncHttpFetchPlugin {
    client: reqwest::Client,
    upstream_url: String,
}

#[derive(Deserialize)]
struct GreetingResponse {
    greeting: String,
}

struct FetchedGreeting(String);

impl RouterPlugin for AsyncHttpFetchPlugin {
    type Config = AsyncHttpFetchConfig;

    fn plugin_name() -> &'static str {
        "async_http_fetch"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        payload.initialize_plugin(Self {
            client: reqwest::Client::new(),
            upstream_url: config.upstream_url,
        })
    }

    // Fetches data from an upstream HTTP service and awaits it here, before the rest of the
    // pipeline (parsing, validation, planning, execution) runs.
    fn on_http_request<'req>(
        &'req self,
        payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookFuture<'req> {
        Box::pin(async move {
            let greeting = fetch_greeting(&self.client, &self.upstream_url).await;
            payload.context.insert(FetchedGreeting(greeting));

            payload.on_end(|payload| {
                let greeting = payload
                    .context
                    .get_ref::<FetchedGreeting>()
                    .map(|entry| entry.0.clone())
                    .unwrap_or_default();
                payload
                    .map_response(move |mut response| {
                        if let Ok(header_value) = HeaderValue::from_str(&greeting) {
                            response
                                .response_mut()
                                .headers_mut()
                                .insert(FETCHED_GREETING_HEADER, header_value);
                        }
                        response
                    })
                    .proceed()
            })
        })
    }
}

async fn fetch_greeting(client: &reqwest::Client, upstream_url: &str) -> String {
    match client.get(upstream_url).send().await {
        Ok(response) => response
            .json::<GreetingResponse>()
            .await
            .map(|body| body.greeting)
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}
