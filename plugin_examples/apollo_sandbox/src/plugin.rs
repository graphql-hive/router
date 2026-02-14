use std::collections::HashMap;

use hive_router::{
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    ntex::http::ResponseBuilder,
    plugins::{
        hooks::{
            on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
    sonic_rs,
};

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct ApolloSandboxOptions {
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

pub struct ApolloSandboxPlugin {
    html: String,
}

impl RouterPlugin for ApolloSandboxPlugin {
    type Config = ApolloSandboxOptions;
    fn plugin_name() -> &'static str {
        "apollo_sandbox"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let serialized_options =
            sonic_rs::to_string(&payload.config()?).unwrap_or_else(|_| "{}".to_string());
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
            serialized_options
        );
        payload.initialize_plugin(Self { html })
    }
    fn on_http_request<'req>(
        &'req self,
        payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        if payload.router_http_request.path() == "/apollo-sandbox" {
            return payload.end_with_response(
                ResponseBuilder::new(StatusCode::OK)
                    .header(
                        CONTENT_TYPE,
                        HeaderValue::from_static("text/html; charset=utf-8"),
                    )
                    .body(self.html.clone()),
            );
        }
        payload.proceed()
    }
}
