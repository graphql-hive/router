use http::StatusCode;

use crate::{plugin_trait::RouterPluginBoxed, response::graphql_error::GraphQLError};

pub type OnGraphQLErrorHookResult = OnGraphQLErrorHookPayload;

pub struct OnGraphQLErrorHookPayload {
    /// The GraphQL error that occurred during the execution of the request.
    /// The plugin can modify the error before proceeding, or it can replace it with a new error.
    /// Example:
    /// ```
    /// fn on_graphql_error(mut payload: OnGraphQLErrorHookPayload) -> OnGraphQLErrorHookResult {
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
    /// fn on_graphql_error(mut payload: OnGraphQLErrorHookPayload) -> OnGraphQLErrorHookResult {
    ///     // Change the status code to 500 Internal Server Error
    ///     payload.status_code = StatusCode::INTERNAL_SERVER_ERROR;
    ///     payload.proceed()
    /// }
    /// ```
    pub status_code: StatusCode,
}

impl OnGraphQLErrorHookPayload {
    /// Returning this will proceed the hook with `payload.error`, and `payload.status_code`
    pub fn proceed(self) -> OnGraphQLErrorHookResult {
        self
    }
}

pub fn handle_graphql_errors_with_plugins(
    plugins: &[RouterPluginBoxed],
    errors: Vec<GraphQLError>,
    mut status_code: StatusCode,
) -> (Vec<GraphQLError>, StatusCode) {
    let mut new_errors = Vec::with_capacity(errors.len());
    for error in errors {
        let mut payload = OnGraphQLErrorHookPayload { error, status_code };

        for plugin in plugins {
            payload = plugin.on_graphql_error(payload);
        }

        new_errors.push(payload.error);
        status_code = payload.status_code;
    }

    (new_errors, status_code)
}
