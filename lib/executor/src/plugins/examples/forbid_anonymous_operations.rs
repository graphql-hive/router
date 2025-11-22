// Same with https://github.com/apollographql/router/blob/dev/examples/forbid-anonymous-operations/rust/src/forbid_anonymous_operations.rs

use http::StatusCode;
use sonic_rs::json;

use crate::{
    execution::plan::PlanExecutionOutput,
    hooks::on_graphql_params::{OnGraphQLParamsEndPayload, OnGraphQLParamsStartPayload},
    plugin_trait::{HookResult, RouterPlugin, StartPayload},
};

pub struct ForbidAnonymousOperations {}

#[async_trait::async_trait]
impl RouterPlugin for ForbidAnonymousOperations {
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLParamsStartPayload<'exec>, OnGraphQLParamsEndPayload> {
        let maybe_operation_name = &payload
            .graphql_params
            .as_ref()
            .and_then(|params| params.operation_name.as_ref());

        if maybe_operation_name.is_none()
            || maybe_operation_name
                .expect("is_none() has been checked before; qed")
                .is_empty()
        {
            // let's log the error
            tracing::error!("Operation is not allowed!");

            // Prepare an HTTP 400 response with a GraphQL error message
            let response_body = json!({
                "errors": [
                    {
                        "message": "Anonymous operations are not allowed",
                        "extensions": {
                            "code": "ANONYMOUS_OPERATION"
                        }
                    }
                ]
            });
            return payload.end_response(PlanExecutionOutput {
                body: sonic_rs::to_vec(&response_body).unwrap_or_default(),
                headers: http::HeaderMap::new(),
                status: StatusCode::BAD_REQUEST,
            });
        } else {
            // we're good to go!
            tracing::info!("operation is allowed!");
            return payload.cont();
        }
    }
}
