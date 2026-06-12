use http::StatusCode;
use ntex::http::Response;

use crate::{
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::FromGraphQLErrorToResponse,
    response::graphql_error::GraphQLError,
};

/// Payload for `on_schema_resolve`, which runs once per GraphQL request (queries,
/// mutations, subscriptions) before the pipeline, letting a plugin pick the schema
/// the request is validated and planned against.
///
/// Select a schema by inserting the router's schema type into
/// [`context`](Self::context) (`hive_router::RequestSchema`) — it flows through the
/// type-erased [`PluginContext`] because that type lives in the router crate, which
/// this crate can't name. No insert ([`proceed`](Self::proceed)) keeps the default
/// schema; [`end_with_response`](Self::end_with_response) /
/// [`end_with_graphql_error`](Self::end_with_graphql_error) short-circuit. There is
/// no end callback — the response doesn't exist yet.
///
/// # Example
///
/// ```ignore
/// async fn on_schema_resolve<'exec>(
///     &'exec self,
///     payload: OnSchemaResolveHookPayload<'exec>,
/// ) -> OnSchemaResolveHookResult {
///     match self.schema_for(payload.router_http_request) {
///         Some(schema_state) => {
///             payload.context.insert(RequestSchema::new(schema_state));
///             payload.proceed()
///         }
///         None => payload.proceed(), // fall back to the router's default schema
///     }
/// }
/// ```
pub struct OnSchemaResolveHookPayload<'exec> {
    /// The incoming request; inspect its path, params, or headers to pick a schema.
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    /// Per-request context; insert the router's schema type here to select a schema.
    pub context: &'exec PluginContext,
}

impl<'exec> OnSchemaResolveHookPayload<'exec> {
    /// Continue with the inserted schema, or the default if none was inserted.
    pub fn proceed(self) -> OnSchemaResolveHookResult {
        OnSchemaResolveHookResult {
            control_flow: SchemaResolveControlFlow::Proceed,
        }
    }

    /// Short-circuit the request with `response`, skipping the pipeline.
    pub fn end_with_response<TResponse: Into<Response>>(
        self,
        response: TResponse,
    ) -> OnSchemaResolveHookResult {
        OnSchemaResolveHookResult {
            control_flow: SchemaResolveControlFlow::EndWithResponse(response.into()),
        }
    }

    /// Short-circuit with a GraphQL error response (e.g. no schema for the request).
    pub fn end_with_graphql_error(
        self,
        error: GraphQLError,
        status_code: StatusCode,
    ) -> OnSchemaResolveHookResult {
        self.end_with_response(Response::from_graphql_error_to_response(error, status_code))
    }
}

/// Outcome of [`OnSchemaResolveHookPayload`]: continue into the pipeline, or
/// short-circuit with a response.
pub struct OnSchemaResolveHookResult {
    pub control_flow: SchemaResolveControlFlow,
}

pub enum SchemaResolveControlFlow {
    /// Use the schema inserted into the context (if any), else the default.
    Proceed,
    /// Return this response immediately, skipping the pipeline.
    EndWithResponse(Response),
}

#[cfg(test)]
mod tests {
    use http::Uri;
    use ntex::router::Path;

    use crate::{
        hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        plugin_context::{PluginContext, RouterHttpRequest},
        plugin_trait::{RouterPlugin, RouterPluginBoxed},
    };

    use super::*;

    /// What a real plugin would insert into the context to select a schema; the
    /// router defines the concrete type, here we just prove the mechanism.
    struct SelectedSchemaMarker(&'static str);

    fn fake_request<'a>(
        uri: &'a Uri,
        path: &'a Path<Uri>,
        headers: &'a ntex::http::HeaderMap,
    ) -> RouterHttpRequest<'a> {
        RouterHttpRequest {
            uri,
            method: &http::Method::POST,
            version: http::Version::HTTP_11,
            headers,
            path: "/graphql",
            query_string: "",
            match_info: path,
        }
    }

    #[derive(Default)]
    struct SelectingPlugin;
    #[async_trait::async_trait]
    impl RouterPlugin for SelectingPlugin {
        type Config = ();
        fn plugin_name() -> &'static str {
            "selecting_plugin"
        }
        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }
        async fn on_schema_resolve<'exec>(
            &'exec self,
            payload: OnSchemaResolveHookPayload<'exec>,
        ) -> OnSchemaResolveHookResult {
            payload.context.insert(SelectedSchemaMarker("schema-a"));
            payload.proceed()
        }
    }

    #[derive(Default)]
    struct RejectingPlugin;
    #[async_trait::async_trait]
    impl RouterPlugin for RejectingPlugin {
        type Config = ();
        fn plugin_name() -> &'static str {
            "rejecting_plugin"
        }
        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }
        async fn on_schema_resolve<'exec>(
            &'exec self,
            payload: OnSchemaResolveHookPayload<'exec>,
        ) -> OnSchemaResolveHookResult {
            payload.end_with_graphql_error(
                GraphQLError::from_message_and_code("no schema", "SCHEMA_RESOLUTION_FAILED"),
                StatusCode::BAD_GATEWAY,
            )
        }
    }

    /// The common default path: a plugin that selects nothing leaves the context
    /// untouched, so the router falls back to its single schema.
    #[derive(Default)]
    struct NoopPlugin;
    #[async_trait::async_trait]
    impl RouterPlugin for NoopPlugin {
        type Config = ();
        fn plugin_name() -> &'static str {
            "noop_plugin"
        }
        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }
        async fn on_schema_resolve<'exec>(
            &'exec self,
            payload: OnSchemaResolveHookPayload<'exec>,
        ) -> OnSchemaResolveHookResult {
            payload.proceed()
        }
    }

    #[ntex::test]
    async fn selects_schema_via_context_through_dyn_dispatch() {
        let uri: Uri = "http://example.com/graphql".parse().unwrap();
        let path: Path<Uri> = Path::new(uri.clone());
        let headers = ntex::http::HeaderMap::new();
        let router_http_request = fake_request(&uri, &path, &headers);
        let context = PluginContext::default();
        let plugin: RouterPluginBoxed = Box::new(SelectingPlugin);

        let result = plugin
            .on_schema_resolve(OnSchemaResolveHookPayload {
                router_http_request: &router_http_request,
                context: &context,
            })
            .await;

        assert!(matches!(
            result.control_flow,
            SchemaResolveControlFlow::Proceed
        ));
        assert!(context.contains::<SelectedSchemaMarker>());
        assert_eq!(
            context.get_ref::<SelectedSchemaMarker>().unwrap().0,
            "schema-a"
        );
    }

    #[ntex::test]
    async fn proceed_without_selecting_leaves_context_empty() {
        let uri: Uri = "http://example.com/graphql".parse().unwrap();
        let path: Path<Uri> = Path::new(uri.clone());
        let headers = ntex::http::HeaderMap::new();
        let router_http_request = fake_request(&uri, &path, &headers);
        let context = PluginContext::default();
        let plugin: RouterPluginBoxed = Box::new(NoopPlugin);

        let result = plugin
            .on_schema_resolve(OnSchemaResolveHookPayload {
                router_http_request: &router_http_request,
                context: &context,
            })
            .await;

        assert!(matches!(
            result.control_flow,
            SchemaResolveControlFlow::Proceed
        ));
        // No selection inserted → the router uses its default schema.
        assert!(!context.contains::<SelectedSchemaMarker>());
    }

    #[ntex::test]
    async fn can_short_circuit_with_a_response() {
        let uri: Uri = "http://example.com/graphql".parse().unwrap();
        let path: Path<Uri> = Path::new(uri.clone());
        let headers = ntex::http::HeaderMap::new();
        let router_http_request = fake_request(&uri, &path, &headers);
        let context = PluginContext::default();
        let plugin: RouterPluginBoxed = Box::new(RejectingPlugin);

        let result = plugin
            .on_schema_resolve(OnSchemaResolveHookPayload {
                router_http_request: &router_http_request,
                context: &context,
            })
            .await;

        match result.control_flow {
            SchemaResolveControlFlow::EndWithResponse(response) => {
                assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
            }
            _ => panic!("expected the rejecting plugin to short-circuit with a response"),
        }
        assert!(!context.contains::<SelectedSchemaMarker>());
    }
}
