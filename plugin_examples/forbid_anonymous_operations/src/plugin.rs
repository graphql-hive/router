// Same with https://github.com/apollographql/router/blob/dev/examples/forbid-anonymous-operations/rust/src/forbid_anonymous_operations.rs

use hive_router::http::StatusCode;
use hive_router::{async_trait, tracing, GraphQLError};

use hive_router::plugins::{
    hooks::{
        on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
        on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
    },
    plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
};

#[derive(Default)]
pub struct ForbidAnonymousOperationsPlugin;

#[async_trait]
impl RouterPlugin for ForbidAnonymousOperationsPlugin {
    type Config = ();
    fn plugin_name() -> &'static str {
        "forbid_anonymous_operations"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        // After the GraphQL parameters have been parsed, we can check if the operation is anonymous
        // So we use `on_end`
        payload.on_end(|payload| {
            let maybe_operation_name = &payload.graphql_params.operation_name.as_ref();

            if maybe_operation_name.is_none_or(|operation_name| operation_name.is_empty()) {
                // let's log the error
                tracing::error!("Operation is not allowed!");

                // Prepare an HTTP 400 response with a GraphQL error message
                return payload.end_with_graphql_error(
                    GraphQLError::from_message_and_code(
                        "Anonymous operations are not allowed",
                        "ANONYMOUS_OPERATION",
                    ),
                    StatusCode::BAD_REQUEST,
                );
            }
            // we're good to go!
            tracing::info!("operation is allowed!");
            payload.proceed()
        })
    }
}
