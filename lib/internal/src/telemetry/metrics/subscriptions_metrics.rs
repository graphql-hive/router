use opentelemetry::{
    metrics::{Counter, Meter, UpDownCounter},
    KeyValue,
};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names, values::SubscriptionOperation};

#[derive(Clone)]
struct SubscriptionsInstruments {
    // Client -> Router (the router's server side).
    /// Active client subscriptions across all transports (WS + HTTP streaming).
    active_subscriptions: Option<UpDownCounter<i64>>,
    /// Active accepted WebSocket connections (clients).
    active_connections: Option<UpDownCounter<i64>>,
    /// Count of client subscribe/unsubscribe operations.
    operations_total: Option<Counter<u64>>,
    // Router -> Subgraph (the router's client side), labeled by subgraph.
    /// Active upstream subscriptions to subgraphs.
    subgraph_active_subscriptions: Option<UpDownCounter<i64>>,
    /// Count of upstream subscribe/unsubscribe operations against subgraphs.
    subgraph_operations_total: Option<Counter<u64>>,
    /// Messages dropped because the downstream consumer fell behind the subgraph.
    subgraph_dropped_messages_total: Option<Counter<u64>>,
}

pub struct SubscriptionsMetrics {
    instruments: SubscriptionsInstruments,
}

impl SubscriptionsMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let active_subscriptions = meter.map(|meter| {
            meter
                .i64_up_down_counter(names::SUBSCRIPTIONS_ACTIVE)
                .with_unit("{subscription}")
                .with_description("Number of active GraphQL subscriptions")
                .build()
        });

        let active_connections = meter.map(|meter| {
            meter
                .i64_up_down_counter(names::WEBSOCKET_CONNECTIONS_ACTIVE)
                .with_unit("{connection}")
                .with_description("Number of active WebSocket connections")
                .build()
        });

        let operations_total = meter.map(|meter| {
            meter
                .u64_counter(names::SUBSCRIPTIONS_OPERATIONS_TOTAL)
                .with_unit("{operation}")
                .with_description("Total number of subscribe/unsubscribe operations")
                .build()
        });

        let subgraph_active_subscriptions = meter.map(|meter| {
            meter
                .i64_up_down_counter(names::SUBGRAPH_SUBSCRIPTIONS_ACTIVE)
                .with_unit("{subscription}")
                .with_description("Number of active subscriptions to subgraphs")
                .build()
        });

        let subgraph_operations_total = meter.map(|meter| {
            meter
                .u64_counter(names::SUBGRAPH_SUBSCRIPTIONS_OPERATIONS_TOTAL)
                .with_unit("{operation}")
                .with_description(
                    "Total number of subscribe/unsubscribe operations against subgraphs",
                )
                .build()
        });

        let subgraph_dropped_messages_total = meter.map(|meter| {
            meter
                .u64_counter(names::SUBGRAPH_SUBSCRIPTIONS_DROPPED_MESSAGES_TOTAL)
                .with_unit("{message}")
                .with_description(
                    "Total number of subscription messages dropped because the consumer \
                     could not keep up with the subgraph",
                )
                .build()
        });

        Self {
            instruments: SubscriptionsInstruments {
                active_subscriptions,
                active_connections,
                operations_total,
                subgraph_active_subscriptions,
                subgraph_operations_total,
                subgraph_dropped_messages_total,
            },
        }
    }

    /// Track an active WebSocket connection. Increments the active-connections gauge now and
    /// decrements it when the returned guard is dropped (connection closed).
    pub fn track_connection(&self) -> WsConnectionGuard {
        if let Some(gauge) = &self.instruments.active_connections {
            gauge.add(1, &[]);
        }
        WsConnectionGuard {
            gauge: self.instruments.active_connections.clone(),
        }
    }

    /// Track an active client subscription. Increments the active-subscriptions gauge and the
    /// `subscribe` operation counter now; decrements the gauge and increments the `unsubscribe`
    /// operation counter when the returned guard is dropped (subscription ended).
    pub fn track_subscription(&self) -> SubscriptionMetricsGuard {
        if let Some(gauge) = &self.instruments.active_subscriptions {
            gauge.add(1, &[]);
        }
        self.record_operation(SubscriptionOperation::Subscribe);
        SubscriptionMetricsGuard {
            gauge: self.instruments.active_subscriptions.clone(),
            operations_total: self.instruments.operations_total.clone(),
        }
    }

    /// Track an active upstream subscription to a subgraph. Increments the active-subscriptions
    /// gauge and the `subscribe` operation counter (both tagged with the subgraph name) now;
    /// decrements the gauge and increments the `unsubscribe` operation counter when the returned
    /// guard is dropped (upstream subscription ended).
    pub fn track_subgraph_subscription(&self, subgraph_name: &str) -> SubgraphSubscriptionGuard {
        if let Some(gauge) = &self.instruments.subgraph_active_subscriptions {
            gauge.add(
                1,
                &[KeyValue::new(
                    labels::SUBGRAPH_NAME,
                    subgraph_name.to_string(),
                )],
            );
        }
        if let Some(counter) = &self.instruments.subgraph_operations_total {
            record_subgraph_operation_on(counter, subgraph_name, SubscriptionOperation::Subscribe);
        }
        SubgraphSubscriptionGuard {
            gauge: self.instruments.subgraph_active_subscriptions.clone(),
            operations_total: self.instruments.subgraph_operations_total.clone(),
            subgraph_name: subgraph_name.to_string(),
        }
    }

    /// Record a subscription message dropped because the consumer fell behind the subgraph.
    pub fn record_dropped_message(&self, subgraph_name: &str) {
        if let Some(counter) = &self.instruments.subgraph_dropped_messages_total {
            let attributes = [KeyValue::new(
                labels::SUBGRAPH_NAME,
                subgraph_name.to_string(),
            )];
            #[cfg(debug_assertions)]
            debug_assert_attrs(
                names::SUBGRAPH_SUBSCRIPTIONS_DROPPED_MESSAGES_TOTAL,
                &attributes,
            );
            counter.add(1, &attributes);
        }
    }

    fn record_operation(&self, operation: SubscriptionOperation) {
        if let Some(counter) = &self.instruments.operations_total {
            record_operation_on(counter, operation);
        }
    }
}

fn record_operation_on(counter: &Counter<u64>, operation: SubscriptionOperation) {
    let attributes = [KeyValue::new(
        labels::SUBSCRIPTION_OPERATION,
        operation.as_str(),
    )];
    #[cfg(debug_assertions)]
    debug_assert_attrs(names::SUBSCRIPTIONS_OPERATIONS_TOTAL, &attributes);
    counter.add(1, &attributes);
}

fn record_subgraph_operation_on(
    counter: &Counter<u64>,
    subgraph_name: &str,
    operation: SubscriptionOperation,
) {
    let attributes = [
        KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
        KeyValue::new(labels::SUBSCRIPTION_OPERATION, operation.as_str()),
    ];
    #[cfg(debug_assertions)]
    debug_assert_attrs(names::SUBGRAPH_SUBSCRIPTIONS_OPERATIONS_TOTAL, &attributes);
    counter.add(1, &attributes);
}

/// Decrements the active-connections gauge when dropped.
pub struct WsConnectionGuard {
    gauge: Option<UpDownCounter<i64>>,
}

impl Drop for WsConnectionGuard {
    fn drop(&mut self) {
        if let Some(gauge) = &self.gauge {
            gauge.add(-1, &[]);
        }
    }
}

/// Decrements the active-subscriptions gauge and records an `unsubscribe` operation when dropped.
pub struct SubscriptionMetricsGuard {
    gauge: Option<UpDownCounter<i64>>,
    operations_total: Option<Counter<u64>>,
}

impl Drop for SubscriptionMetricsGuard {
    fn drop(&mut self) {
        if let Some(gauge) = &self.gauge {
            gauge.add(-1, &[]);
        }
        if let Some(counter) = &self.operations_total {
            record_operation_on(counter, SubscriptionOperation::Unsubscribe);
        }
    }
}

/// Decrements the subgraph active-subscriptions gauge and records an `unsubscribe` operation
/// (both tagged with the subgraph name) when dropped.
pub struct SubgraphSubscriptionGuard {
    gauge: Option<UpDownCounter<i64>>,
    operations_total: Option<Counter<u64>>,
    subgraph_name: String,
}

impl Drop for SubgraphSubscriptionGuard {
    fn drop(&mut self) {
        if let Some(gauge) = &self.gauge {
            gauge.add(
                -1,
                &[KeyValue::new(
                    labels::SUBGRAPH_NAME,
                    self.subgraph_name.clone(),
                )],
            );
        }
        if let Some(counter) = &self.operations_total {
            record_subgraph_operation_on(
                counter,
                &self.subgraph_name,
                SubscriptionOperation::Unsubscribe,
            );
        }
    }
}
