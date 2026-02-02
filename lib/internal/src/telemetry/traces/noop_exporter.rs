use opentelemetry_sdk::{
    trace::{SpanData, SpanExporter},
    Resource,
};

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct NoopExporter;

impl NoopExporter {
    #[allow(dead_code)] // I keep it to test things
    pub fn new() -> Self {
        NoopExporter
    }
}

impl SpanExporter for NoopExporter {
    async fn export(&self, _batch: Vec<SpanData>) -> opentelemetry_sdk::error::OTelSdkResult {
        Ok(())
    }

    fn shutdown(&mut self) -> opentelemetry_sdk::error::OTelSdkResult {
        Ok(())
    }

    fn force_flush(&mut self) -> opentelemetry_sdk::error::OTelSdkResult {
        Ok(())
    }

    fn set_resource(&mut self, _resource: &Resource) {}
}
