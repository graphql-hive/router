use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use opentelemetry_stdout::SpanExporter as StdoutSpanExporter;

#[derive(Debug)]
pub struct StdoutExporter {
    inner: StdoutSpanExporter,
}

impl StdoutExporter {
    pub fn new() -> Self {
        Self {
            inner: StdoutSpanExporter::default(),
        }
    }
}

impl Default for StdoutExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl SpanExporter for StdoutExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        self.inner.export(batch).await
    }

    fn shutdown(&mut self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn force_flush(&mut self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, res: &opentelemetry_sdk::Resource) {
        self.inner.set_resource(res);
    }
}
