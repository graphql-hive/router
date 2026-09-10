use std::{
    collections::HashMap,
    error::Error,
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use ntex::{
    http::body::{Body, BodySize, MessageBody, ResponseBody},
    util::Bytes,
    web::WebResponse,
};
use opentelemetry::trace::{SpanContext, SpanId, TraceContextExt, TraceId};
use tracing::{Span, Subscriber};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::Layer;

#[derive(Default)]
pub(crate) struct HiveTraceDocuments(Mutex<HashMap<(TraceId, SpanId), String>>);

pub(crate) struct RouteGraphqlDocumentsToHive {
    pub(crate) enabled: bool,
}

impl<S: Subscriber> Layer<S> for RouteGraphqlDocumentsToHive {}

tokio::task_local! {
    pub(crate) static HIVE_TRACE_DOCUMENTS: Arc<HiveTraceDocuments>;
}

pub(crate) struct HiveTraceScope {
    documents: Arc<HiveTraceDocuments>,
}

impl HiveTraceScope {
    pub(crate) fn new() -> Self {
        Self {
            documents: Arc::default(),
        }
    }

    pub(crate) fn scope<F: Future>(&self, future: F) -> HiveTraceFuture<F> {
        HiveTraceFuture::new(future, Arc::clone(&self.documents))
    }

    pub(crate) fn attach_to_response(self, response: WebResponse) -> WebResponse {
        let documents = self.documents;
        response.map_body(|_, body| {
            ResponseBody::Body(Body::from_message(HiveTraceBody {
                body: Some(body),
                documents,
            }))
        })
    }
}

pub(crate) struct HiveTraceFuture<F: Future> {
    future: Option<Pin<Box<F>>>,
    documents: Arc<HiveTraceDocuments>,
}

impl<F: Future> HiveTraceFuture<F> {
    pub(crate) fn new(future: F, documents: Arc<HiveTraceDocuments>) -> Self {
        Self {
            future: Some(Box::pin(future)),
            documents,
        }
    }
}

impl<F: Future> Future for HiveTraceFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        HIVE_TRACE_DOCUMENTS.sync_scope(Arc::clone(&this.documents), || {
            this.future.as_mut().unwrap().as_mut().poll(cx)
        })
    }
}

impl<F: Future> Drop for HiveTraceFuture<F> {
    fn drop(&mut self) {
        let future = self.future.take();
        HIVE_TRACE_DOCUMENTS.sync_scope(Arc::clone(&self.documents), || drop(future));
    }
}

pub fn record_graphql_document(span: &Span, document: &str) {
    tracing::dispatcher::get_default(|dispatch| {
        let Some(routing) = dispatch.downcast_ref::<RouteGraphqlDocumentsToHive>() else {
            return;
        };
        if !routing.enabled {
            return;
        }
        let context = span.context();
        let context_span = context.span();
        let span_context = context_span.span_context();
        // only record the document if the span is sampled
        if !span_context.is_sampled() {
            return;
        }
        let _ = HIVE_TRACE_DOCUMENTS.try_with(|documents| {
            documents.0.lock().unwrap().insert(
                (span_context.trace_id(), span_context.span_id()),
                document.to_string(),
            );
        });
    });
}

pub(crate) fn take_graphql_document(span_context: &SpanContext) -> Option<String> {
    HIVE_TRACE_DOCUMENTS
        .try_with(|documents| {
            documents
                .0
                .lock()
                .unwrap()
                .remove(&(span_context.trace_id(), span_context.span_id()))
        })
        .ok()
        .flatten()
}

struct HiveTraceBody<B: MessageBody> {
    body: Option<B>,
    documents: Arc<HiveTraceDocuments>,
}

impl<B: MessageBody> MessageBody for HiveTraceBody<B> {
    fn size(&self) -> BodySize {
        self.body.as_ref().unwrap().size()
    }

    fn poll_next_chunk(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Rc<dyn Error>>>> {
        HIVE_TRACE_DOCUMENTS.sync_scope(Arc::clone(&self.documents), || {
            self.body.as_mut().unwrap().poll_next_chunk(cx)
        })
    }
}

impl<B: MessageBody> Drop for HiveTraceBody<B> {
    fn drop(&mut self) {
        let body = self.body.take();
        HIVE_TRACE_DOCUMENTS.sync_scope(Arc::clone(&self.documents), || drop(body));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct DropProbe(Arc<AtomicBool>);

    struct FutureDropProbe(Arc<AtomicBool>);

    impl Future for FutureDropProbe {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for FutureDropProbe {
        fn drop(&mut self) {
            assert!(HIVE_TRACE_DOCUMENTS.try_with(|_| ()).is_ok());
            self.0.store(true, Ordering::Relaxed);
        }
    }

    impl MessageBody for DropProbe {
        fn size(&self) -> BodySize {
            BodySize::Empty
        }

        fn poll_next_chunk(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Rc<dyn Error>>>> {
            Poll::Ready(None)
        }
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            assert!(HIVE_TRACE_DOCUMENTS.try_with(|_| ()).is_ok());
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn drops_future_inside_hive_trace_scope() {
        let dropped = Arc::new(AtomicBool::new(false));
        drop(HiveTraceFuture::new(
            FutureDropProbe(Arc::clone(&dropped)),
            Arc::default(),
        ));
        assert!(dropped.load(Ordering::Relaxed));
    }

    #[test]
    fn drops_response_body_inside_hive_trace_scope() {
        let dropped = Arc::new(AtomicBool::new(false));
        drop(HiveTraceBody {
            body: Some(DropProbe(Arc::clone(&dropped))),
            documents: Arc::default(),
        });
        assert!(dropped.load(Ordering::Relaxed));
    }
}
