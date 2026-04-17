use opentelemetry::metrics::{Counter, Meter};

use crate::telemetry::metrics::catalog::names;

struct PersistedDocumentsInstruments {
    storage_failures_total: Option<Counter<u64>>,
    extract_missing_id_total: Option<Counter<u64>>,
}

pub struct PersistedDocumentsMetrics {
    instruments: PersistedDocumentsInstruments,
}

impl PersistedDocumentsMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let storage_failures_total = meter.map(|meter| {
            meter
                .u64_counter(names::PERSISTED_DOCUMENTS_STORAGE_FAILURES_TOTAL)
                .with_unit("{failure}")
                .with_description("Total number of failed persisted document resolutions")
                .build()
        });

        let extract_missing_id_total = meter.map(|meter| {
            meter
                .u64_counter(names::PERSISTED_DOCUMENTS_EXTRACT_MISSING_ID_TOTAL)
                .with_unit("{request}")
                .with_description("Total number of requests without persisted document id")
                .build()
        });

        Self {
            instruments: PersistedDocumentsInstruments {
                storage_failures_total,
                extract_missing_id_total,
            },
        }
    }

    pub fn record_resolution_failure(&self) {
        if let Some(counter) = &self.instruments.storage_failures_total {
            counter.add(1, &[]);
        }
    }

    pub fn record_missing_id(&self) {
        if let Some(counter) = &self.instruments.extract_missing_id_total {
            counter.add(1, &[]);
        }
    }
}
