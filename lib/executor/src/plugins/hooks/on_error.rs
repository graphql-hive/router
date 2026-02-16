use http::StatusCode;

use crate::{plugin_trait::RouterPluginBoxed, response::graphql_error::GraphQLError};

pub type OnErrorHookResult = OnErrorHookPayload;

pub struct OnErrorHookPayload {
    pub error: GraphQLError,
    pub status_code: StatusCode,
}

impl OnErrorHookPayload {
    pub fn proceed(self) -> OnErrorHookResult {
        self
    }
}

pub fn handle_errors_with_plugins(
    plugins: &[RouterPluginBoxed],
    errors: Vec<GraphQLError>,
    mut status_code: StatusCode,
) -> (Vec<GraphQLError>, StatusCode) {
    let mut new_errors = Vec::with_capacity(errors.len());
    for error in errors {
        let mut payload = OnErrorHookPayload { error, status_code };

        for plugin in plugins {
            payload = plugin.on_error(payload);
        }

        new_errors.push(payload.error);
        status_code = payload.status_code;
    }

    (new_errors, status_code)
}
