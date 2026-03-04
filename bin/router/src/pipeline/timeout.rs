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
    let timeout = sleep(
        shared_state
            .router_config
            .traffic_shaping
            .router
            .request_timeout,
    );

    match select(timeout, res_fut).await {
        // If the timeout future completes first, return a timeout error response.
        Either::Left(_) => Err(PipelineError::TimeoutError),
        // If the request handler future completes first, return its response.
        Either::Right(res) => res,
    }
}
