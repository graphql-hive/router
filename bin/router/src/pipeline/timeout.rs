use std::future::Future;

use ntex::{
    time::sleep,
    util::{select, Either},
    web,
};

use crate::{pipeline::error::PipelineError, RouterSharedState};

#[inline]
pub async fn handle_timeout<TFuture: Future<Output = Result<web::HttpResponse, PipelineError>>>(
    res_fut: TFuture,
    shared_state: &RouterSharedState,
) -> TFuture::Output {
    let timeout_limit = shared_state
        .router_config
        .traffic_shaping
        .router
        .request_timeout;

    let timeout = sleep(timeout_limit);

    match select(timeout, res_fut).await {
        // If the timeout future completes first, return a timeout error response.
        Either::Left(_) => {
            tracing::error!(
                limit_ms = timeout_limit.as_millis(),
                "request between client and router has timed out"
            );

            Err(PipelineError::TimeoutError)
        }
        // If the request handler future completes first, return its response.
        Either::Right(res) => res,
    }
}
