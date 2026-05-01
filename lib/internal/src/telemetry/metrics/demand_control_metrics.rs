use opentelemetry::{
    metrics::{Histogram, Meter},
    KeyValue,
};
use sonic_rs::Serialize;
use strum::IntoStaticStr;

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names};

struct DemandControlInstruments {
    estimated_cost: Option<Histogram<u64>>,
    actual_cost: Option<Histogram<u64>>,
    delta: Option<Histogram<f64>>,
}

pub struct DemandControlMetrics {
    instruments: DemandControlInstruments,
}

#[derive(Debug, Clone)]
pub struct DemandControlMetricsRecorder {
    actual_cost: Histogram<u64>,
    delta: Histogram<f64>,
}

impl DemandControlMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let estimated_cost = meter.map(|meter| {
            meter
                .u64_histogram(names::COST_ESTIMATED)
                .with_unit("By")
                .with_description("The estimated cost of an operation before execution")
                .build()
        });

        let actual_cost = meter.map(|meter| {
            meter
                .u64_histogram(names::COST_ACTUAL)
                .with_unit("By")
                .with_description("The actual cost of an operation, measured after execution")
                .build()
        });

        let delta = meter.map(|meter| {
            meter
                .f64_histogram(names::COST_DELTA)
                .with_unit("By")
                .with_description("The difference between actual and estimated operation cost")
                .build()
        });

        Self {
            instruments: DemandControlInstruments {
                estimated_cost,
                actual_cost,
                delta,
            },
        }
    }

    pub fn record_estimated_cost(
        &self,
        cost: u64,
        result: &DemandControlResultCode,
        operation_name: Option<&str>,
    ) {
        let result_code: &'static str = result.into();
        let Some(histogram) = self.instruments.estimated_cost.as_ref() else {
            return;
        };

        let mut attrs = vec![KeyValue::new(labels::COST_RESULT, result_code.to_string())];
        if let Some(operation_name) = operation_name {
            attrs.push(KeyValue::new(
                labels::GRAPHQL_OPERATION_NAME,
                operation_name.to_string(),
            ));
        }
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::COST_ESTIMATED, &attrs);
        histogram.record(cost, &attrs);
    }

    pub fn recorder(&self) -> Option<DemandControlMetricsRecorder> {
        let actual_cost = self.instruments.actual_cost.as_ref()?.clone();
        let delta = self.instruments.delta.as_ref()?.clone();

        Some(DemandControlMetricsRecorder { actual_cost, delta })
    }
}

#[derive(Debug, IntoStaticStr, Serialize, Clone)]
pub enum DemandControlResultCode {
    #[serde(rename = "COST_OK")]
    #[strum(serialize = "COST_OK")]
    CostOk,
    #[serde(rename = "COST_ESTIMATED_TOO_EXPENSIVE")]
    #[strum(serialize = "COST_ESTIMATED_TOO_EXPENSIVE")]
    CostEstimatedTooExpensive,
    #[serde(rename = "COST_ACTUAL_TOO_EXPENSIVE")]
    #[strum(serialize = "COST_ACTUAL_TOO_EXPENSIVE")]
    CostActualTooExpensive,
}

impl DemandControlMetricsRecorder {
    pub fn record_actual_cost(
        &self,
        cost: u64,
        result: &DemandControlResultCode,
        operation_name: Option<&str>,
    ) {
        let result_code: &'static str = result.into();
        let mut attrs = vec![KeyValue::new(labels::COST_RESULT, result_code.to_string())];
        if let Some(operation_name) = operation_name {
            attrs.push(KeyValue::new(
                labels::GRAPHQL_OPERATION_NAME,
                operation_name.to_string(),
            ));
        }
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::COST_ACTUAL, &attrs);
        self.actual_cost.record(cost, &attrs);
    }

    pub fn record_delta(
        &self,
        delta: i64,
        result: &DemandControlResultCode,
        operation_name: Option<&str>,
    ) {
        let result_code: &'static str = result.into();
        let mut attrs = vec![KeyValue::new(labels::COST_RESULT, result_code.to_string())];
        if let Some(operation_name) = operation_name {
            attrs.push(KeyValue::new(
                labels::GRAPHQL_OPERATION_NAME,
                operation_name.to_string(),
            ));
        }
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::COST_DELTA, &attrs);
        self.delta.record(delta as f64, &attrs);
    }
}
