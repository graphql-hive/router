use std::collections::HashMap;

use hive_router_plan_executor::{
    execution::plan::PlanExecutionOutput,
    hooks::on_http_request::{OnHttpRequestPayload, OnHttpResponsePayload},
    plugin_trait::{HookResult, RouterPlugin, RouterPluginWithConfig, StartPayload},
};
use http::HeaderMap;
use reqwest::StatusCode;
pub(crate) use sonic_rs::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct ApolloSandboxOptions {
    pub enabled: bool,
    pub initial_endpoint: String,
    /**
     * By default, the embedded Sandbox does not show the **Include cookies** toggle in its connection settings.Set `hideCookieToggle` to `false` to enable users of your embedded Sandbox instance to toggle the **Include cookies** setting.
     */
    pub hide_cookie_toggle: bool,
    /**
     * By default, the embedded Sandbox has a URL input box that is editable by users.Set endpointIsEditable to false to prevent users of your embedded Sandbox instance from changing the endpoint URL.
     */
    pub endpoint_is_editable: bool,
    /**
     * You can set `includeCookies` to `true` if you instead want Sandbox to pass `{ credentials: 'include' }` for its requests.If you pass the `handleRequest` option, this option is ignored.Read more about the `fetch` API and credentials [here](https://developer.mozilla.org/en-US/docs/Web/API/fetch#credentials).This config option is deprecated in favor of using the connection settings cookie toggle in Sandbox and setting the default value via `initialState.includeCookies`.
     */
    pub include_cookies: bool,
    /**
     * An object containing additional options related to the state of the embedded Sandbox on page load.
     */
    pub initial_state: ApolloSandboxInitialStateOptions,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApolloSandboxInitialStateOptions {
    pub enabled: bool,
    /**
     * Set this value to `true` if you want Sandbox to pass `{ credentials: 'include' }` for its requests by default.If you set `hideCookieToggle` to `false`, users can override this default setting with the **Include cookies** toggle. (By default, the embedded Sandbox does not show the **Include cookies** toggle in its connection settings.)If you also pass the `handleRequest` option, this option is ignored.Read more about the `fetch` API and credentials [here](https://developer.mozilla.org/en-US/docs/Web/API/fetch#credentials).
     */
    pub include_cookies: bool,
    /**
     * A URI-encoded operation to populate in Sandbox's editor on load.If you omit this, Sandbox initially loads an example query based on your schema.Example:
     * ```js
     * initialState: {
     *   document: `
     *    query ExampleQuery {
     *      books {
     *        title
     *      }
     *    }
     *  `
     * }
     * ```
     */
    pub document: Option<String>,
    /**
     * A URI-encoded, serialized object containing initial variable values to populate in Sandbox on load.If provided, these variables should apply to the initial query you provide for [`document`](https://www.apollographql.com/docs/apollo-sandbox#document).Example:
     *
     * ```js
     * initialState: {
     *   variables: {
     *     userID: "abc123"
     *   },
     * }
     * ```
     */
    pub variables: Option<String>,
    /**
     * A URI-encoded, serialized object containing initial HTTP header values to populate in Sandbox on load.Example:
     *
     *
     * ```js
     * initialState: {
     *   headers: {
     *     authorization: "Bearer abc123";
     *   }
     * }
     * ```
     */
    pub headers: Option<String>,
    /**
     * The ID of a collection, paired with an operation ID to populate in Sandbox on load. You can find these values from a registered graph in Studio by clicking the **...** menu next to an operation in the Explorer of that graph and selecting **View  operation  details**.Example:
     *
     * ```js
     * initialState: {
     *   collectionId: 'abc1234',
     *   operationId: 'xyz1234'
     * }
     * ```
     */
    pub collection_id: Option<String>,
    pub operation_id: Option<String>,
    /**
     * If `true`, the embedded Sandbox periodically polls your `initialEndpoint` for schema updates.The default value is `true`.Example:
     *
     * ```js
     * initialState: {
     *   pollForSchemaUpdates: false;
     * }
     * ```
     */
    pub poll_for_schema_updates: bool,
    /**
     * Headers that are applied by default to every operation executed by the embedded Sandbox. Users can turn off the application of these headers, but they can't modify their values.The embedded Sandbox always includes these headers in its introspection queries to your `initialEndpoint`.Example:
     *
     * ```js
     * initialState: {
     *   sharedHeaders: {
     *     authorization: "Bearer abc123";
     *   }
     * }
     * ```
     */
    pub shared_headers: HashMap<String, String>,
}

impl RouterPluginWithConfig for ApolloSandboxPlugin {
    type Config = ApolloSandboxOptions;
    fn plugin_name() -> &'static str {
        "apollo_sandbox"
    }
    fn from_config(config: ApolloSandboxOptions) -> Option<Self> {
        if config.enabled {
            Some(ApolloSandboxPlugin {
                serialized_options: sonic_rs::to_string(&config)
                    .unwrap_or_else(|_| "{}".to_string()),
            })
        } else {
            None
        }
    }
}

pub struct ApolloSandboxPlugin {
    serialized_options: String,
}

impl RouterPlugin for ApolloSandboxPlugin {
    fn on_http_request<'req>(
        &'req self,
        payload: OnHttpRequestPayload<'req>,
    ) -> HookResult<'req, OnHttpRequestPayload<'req>, OnHttpResponsePayload<'req>> {
        if payload.router_http_request.path() == "/apollo-sandbox" {
            let config =
                sonic_rs::to_string(&self.serialized_options).unwrap_or_else(|_| "{}".to_string());
            let html = format!(
                r#"
                    <div style=\"width: 100%; height: 100%;\" id=\"embedded-sandbox\"></div>
                    <script src=\"https://embeddable-sandbox.cdn.apollographql.com/_latest/embeddable-sandbox.umd.production.min.js\"></script>
                    <script>
                        const opts = {};
                        opts.initialEndpoint ||= new URL(location.pathname, location.href).toString();
                        new window.EmbeddedSandbox(opts);
                    </script>
                "#,
                config
            );
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", "text/html".parse().unwrap());
            return payload.end_response(PlanExecutionOutput {
                body: html.into_bytes(),
                headers,
                status: StatusCode::OK,
            });
        }
        payload.cont()
    }
}

#[cfg(test)]
mod apollo_sandbox_tests {
    use hive_router::PluginRegistry;

    #[ntex::test]
    async fn renders_apollo_sandbox_page() {
        use crate::testkit::init_router_from_config_inline;
        use ntex::web::test;

        let app = init_router_from_config_inline(
            r#"
            plugins:
              apollo_sandbox:
                enabled: true
        "#,
            Some(PluginRegistry::new().register::<super::ApolloSandboxPlugin>()),
        )
        .await
        .expect("failed to start router");

        let req = test::TestRequest::get().uri("/apollo-sandbox").to_request();
        let response = app.call(req).await.expect("failed to call /apollo-sandbox");
        let status = response.status();

        let body_bytes = test::read_body(response).await;
        let body_str = std::str::from_utf8(&body_bytes).expect("response body is not valid UTF-8");

        assert_eq!(status, 200);
        assert!(body_str.contains("EmbeddedSandbox"));
    }
}
