use axum::body::Body;
use axum::response::IntoResponse;
use http::{Request, Response};
use std::convert::Infallible;
use std::sync::Arc;
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
        req: &mut Request<Body>,
    ) -> Result<GatewayPipelineStepDecision, PipelineError>;
}

pub enum GatewayPipelineStepDecision {
    Continue,
    RespondWith(Response<Body>),
}

#[derive(Debug, Clone)]
pub struct ProcessorService<S, P> {
    inner: S,
    processor_arc: Arc<P>,
}

impl<S, P> ProcessorService<S, P> {
    pub fn new_layer(inner: S, processor_arc: Arc<P>) -> Self {
        Self {
            inner,
            processor_arc,
        }
    }
}

impl<S, P> Service<Request<Body>> for ProcessorService<S, P>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + Sync
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

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let processor_arc = self.processor_arc.clone();

        Box::pin(async move {
            let result = processor_arc.process(&mut req).await;

            match result {
                Err(err) => {
                    error!("request pipeline error: {}", err.error);
                    debug!("{:?}", err.error);

                    Ok(err.into_response())
                }
                Ok(GatewayPipelineStepDecision::Continue) => {
                    trace!("Pipeline step decision is to continue");

                    inner.call(req).await
                }
                Ok(GatewayPipelineStepDecision::RespondWith(response)) => {
                    trace!("Pipeline step decision is to short circuit");

                    Ok(response)
                }
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProcessorLayer<P> {
    processor_arc: Arc<P>,
}

impl<P> ProcessorLayer<P> {
    pub fn new(processor: P) -> Self {
        Self {
            processor_arc: Arc::new(processor),
        }
    }
}

impl<S, P> Layer<S> for ProcessorLayer<P>
where
    P: GatewayPipelineLayer,
{
    type Service = ProcessorService<S, P>;

    fn layer(&self, inner: S) -> Self::Service {
        let processor_arc = self.processor_arc.clone();
        ProcessorService::new_layer(inner, processor_arc)
    }
}
