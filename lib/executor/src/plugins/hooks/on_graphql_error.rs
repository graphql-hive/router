use http::StatusCode;

use crate::{
    plugin_context::PluginContext, plugin_trait::RouterPluginBoxed,
    request_context::RequestContextPluginApi, request_context::SharedRequestContext,
    response::graphql_error::GraphQLError,
};

type RequestContextApi = RequestContextPluginApi<super::OnGraphqlError>;

pub type OnGraphQLErrorHookResult<'req> = OnGraphQLErrorHookPayload<'req>;

pub struct OnGraphQLErrorHookPayload<'req> {
    /// The GraphQL error that occurred during the execution of the request.
    /// The plugin can modify the error before proceeding, or it can replace it with a new error.
    /// Example:
    /// ```
    /// fn on_graphql_error<'req>(
    ///     &'req self,
    ///     mut payload: OnGraphQLErrorHookPayload<'req>,
    /// ) -> OnGraphQLErrorHookResult<'req> {
    ///     // Add additional information to the error message
    ///     payload.error.message = format!("{} - Additional info from plugin", payload.error.message);
    ///     payload.proceed()
    /// }
    /// ```
    pub error: GraphQLError,
    /// The HTTP status code that will be sent in the response for this error.
    /// The plugin can modify the status code before proceeding.
    /// Example:
    /// ```
    /// fn on_graphql_error<'req>(
    ///     &'req self,
    ///     mut payload: OnGraphQLErrorHookPayload<'req>,
    /// ) -> OnGraphQLErrorHookResult<'req> {
    ///     // Change the status code to 500 Internal Server Error
    ///     payload.status_code = StatusCode::INTERNAL_SERVER_ERROR;
    ///     payload.proceed()
    /// }
    /// ```
    pub status_code: StatusCode,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'req PluginContext,
    pub request_context: RequestContextApi,
}

impl<'req> OnGraphQLErrorHookPayload<'req> {
    /// Returning this will proceed the hook with `payload.error`, and `payload.status_code`
    pub fn proceed(self) -> OnGraphQLErrorHookResult<'req> {
        self
    }
}

pub fn handle_graphql_errors_with_plugins(
    plugins: &[RouterPluginBoxed],
    context: &PluginContext,
    request_context: &SharedRequestContext,
    errors: Vec<GraphQLError>,
    mut status_code: StatusCode,
) -> (Vec<GraphQLError>, StatusCode) {
    let mut new_errors = Vec::with_capacity(errors.len());
    for error in errors {
        let mut payload = OnGraphQLErrorHookPayload {
            error,
            status_code,
            context,
            request_context: request_context.for_plugin::<super::OnGraphqlError>(),
        };

        for plugin in plugins {
            payload = plugin.on_graphql_error(payload);
        }

        new_errors.push(payload.error);
        status_code = payload.status_code;
    }

    (new_errors, status_code)
}
