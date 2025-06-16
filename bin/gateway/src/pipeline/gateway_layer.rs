use axum::body::Body;
use axum::response::IntoResponse;
use http::{Request, Response};
use std::convert::Infallible;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tower::{Layer, Service};
use tracing::{debug, error, trace};

use crate::pipeline::error::PipelineError;

#[async_trait::async_trait]
pub trait GatewayPipelineLayer: Clone + Send + Sync + 'static {
    async fn process(
        &self,
        req: Request<Body>,
    ) -> Result<(Request<Body>, GatewayPipelineStepDecision), PipelineError>;
}

pub enum GatewayPipelineStepDecision {
    Continue,
    RespondWith(Response<Body>),
}

#[derive(Debug, Clone)]
pub struct ProcessorService<S, P> {
    inner: S,
    processor: P,
}

impl<S, P> ProcessorService<S, P> {
    pub fn new_layer(inner: S, processor: P) -> Self {
        Self { inner, processor }
    }
}

impl<S, P> Service<Request<Body>> for ProcessorService<S, P>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    P: GatewayPipelineLayer,
{
    type Response = S::Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let processor = self.processor.clone();

        Box::pin(async move {
            let result = processor.process(req).await;

            match result {
                Err(err) => {
                    error!("request pipeline error: {}", err.error);
                    debug!("{:?}", err.error);

                    Ok(err.into_response())
                }
                Ok((req, GatewayPipelineStepDecision::Continue)) => {
                    trace!("Pipeline step decision is to continue");

                    inner.call(req).await
                }
                Ok((_req, GatewayPipelineStepDecision::RespondWith(response))) => {
                    trace!("Pipeline step decision is to short circuit");

                    Ok(response)
                }
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProcessorLayer<P> {
    processor: P,
}

impl<P> ProcessorLayer<P> {
    pub fn new(processor: P) -> Self {
        Self { processor }
    }
}

impl<S, P> Layer<S> for ProcessorLayer<P>
where
    P: GatewayPipelineLayer,
{
    type Service = ProcessorService<S, P>;

    fn layer(&self, inner: S) -> Self::Service {
        ProcessorService::new_layer(inner, self.processor.clone())
    }
}
