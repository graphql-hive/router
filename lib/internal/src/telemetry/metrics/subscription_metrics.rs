use opentelemetry::{
    metrics::{Counter, Meter, UpDownCounter},
    KeyValue,
};

use crate::telemetry::metrics::catalog::{labels, names, units};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;

#[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
pub enum SubscriptionTransport {
    #[strum(serialize = "websocket")]
    WebSocket,
    #[strum(serialize = "http_multipart")]
    HttpMultipart,
    #[strum(serialize = "http_sse")]
    HttpSse,
    #[strum(serialize = "http_callback")]
    HttpCallback,
}

impl SubscriptionTransport {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
pub enum SubscriptionOperation {
    #[strum(serialize = "subscribe")]
    Subscribe,
    #[strum(serialize = "unsubscribe")]
    Unsubscribe,
}

impl SubscriptionOperation {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

pub struct SubscriptionMetrics {
    subgraphs_active: Option<UpDownCounter<i64>>,
    subgraphs_connections: Option<UpDownCounter<i64>>,
    subgraphs_operations_total: Option<Counter<u64>>,
    subgraphs_dropped_messages_total: Option<Counter<u64>>,
    clients_active: Option<UpDownCounter<i64>>,
    clients_connections: Option<UpDownCounter<i64>>,
    clients_operations_total: Option<Counter<u64>>,
    clients_lagged_messages_total: Option<Counter<u64>>,
}

impl SubscriptionMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let subgraphs_active = meter.map(|m| {
            m.i64_up_down_counter(names::SUBSCRIPTIONS_SUBGRAPHS_ACTIVE)
                .with_description("Active subscribed operations on a subgraph.")
                .with_unit(units::SUBSCRIPTIONS)
                .build()
        });
        let subgraphs_connections = meter.map(|m| {
            m.i64_up_down_counter(names::SUBSCRIPTIONS_SUBGRAPHS_CONNECTIONS)
                .with_description("Active transport connections from router to subgraphs.")
                .with_unit(units::CONNECTIONS)
                .build()
        });
        let subgraphs_operations_total = meter.map(|m| {
            m.u64_counter(names::SUBSCRIPTIONS_SUBGRAPHS_OPERATIONS_TOTAL)
                .with_description("Total subscribe/unsubscribe operations on a subgraph.")
                .with_unit(units::OPERATIONS)
                .build()
        });
        let subgraphs_dropped_messages_total = meter.map(|m| {
            m.u64_counter(names::SUBSCRIPTIONS_SUBGRAPHS_DROPPED_MESSAGES_TOTAL)
                .with_description(
                    "Subgraph messages dropped because consumers were too slow to keep up.",
                )
                .with_unit(units::MESSAGES)
                .build()
        });
        let clients_active = meter.map(|m| {
            m.i64_up_down_counter(names::SUBSCRIPTIONS_CLIENTS_ACTIVE)
                .with_description("Active subscribed operations from clients to the router.")
                .with_unit(units::SUBSCRIPTIONS)
                .build()
        });
        let clients_connections = meter.map(|m| {
            m.i64_up_down_counter(names::SUBSCRIPTIONS_CLIENTS_CONNECTIONS)
                .with_description("Active transport connections from clients to router.")
                .with_unit(units::CONNECTIONS)
                .build()
        });
        let clients_operations_total = meter.map(|m| {
            m.u64_counter(names::SUBSCRIPTIONS_CLIENTS_OPERATIONS_TOTAL)
                .with_description(
                    "Total subscribe/unsubscribe operations from clients to the router.",
                )
                .with_unit(units::OPERATIONS)
                .build()
        });
        let clients_lagged_messages_total = meter.map(|m| {
            m.u64_counter(names::SUBSCRIPTIONS_CLIENTS_LAGGED_MESSAGES_TOTAL)
                .with_description(
                    "Messages skipped by slow client subscribers due to broadcast lag.",
                )
                .with_unit(units::MESSAGES)
                .build()
        });
        Self {
            subgraphs_active,
            subgraphs_connections,
            subgraphs_operations_total,
            subgraphs_dropped_messages_total,
            clients_active,
            clients_connections,
            clients_operations_total,
            clients_lagged_messages_total,
        }
    }

    pub fn active_subgraph_operation(&self, subgraph_name: &str) -> ActiveSubgraphOperationGuard {
        let attrs = [KeyValue::new(
            labels::SUBGRAPH_NAME,
            subgraph_name.to_string(),
        )];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::SUBSCRIPTIONS_SUBGRAPHS_ACTIVE, &attrs);
        if let Some(c) = &self.subgraphs_active {
            c.add(1, &attrs);
        }
        if let Some(c) = &self.subgraphs_operations_total {
            let attrs = [
                KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
                KeyValue::new(
                    labels::SUBSCRIPTION_OPERATION,
                    SubscriptionOperation::Subscribe.as_str(),
                ),
            ];
            #[cfg(debug_assertions)]
            debug_assert_attrs(names::SUBSCRIPTIONS_SUBGRAPHS_OPERATIONS_TOTAL, &attrs);
            c.add(1, &attrs);
        }
        ActiveSubgraphOperationGuard {
            counter: self.subgraphs_active.clone(),
            operations_total: self.subgraphs_operations_total.clone(),
            subgraph_name: subgraph_name.to_string(),
        }
    }

    pub fn active_subgraph_connection(
        &self,
        subgraph_name: &str,
        transport: SubscriptionTransport,
    ) -> ActiveSubgraphConnectionGuard {
        let attrs = [
            KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
            KeyValue::new(labels::SUBSCRIPTION_TRANSPORT, transport.as_str()),
        ];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::SUBSCRIPTIONS_SUBGRAPHS_CONNECTIONS, &attrs);
        if let Some(c) = &self.subgraphs_connections {
            c.add(1, &attrs);
        }
        ActiveSubgraphConnectionGuard {
            counter: self.subgraphs_connections.clone(),
            subgraph_name: subgraph_name.to_string(),
            transport,
        }
    }

    pub fn active_client_operation(
        &self,
        transport: SubscriptionTransport,
    ) -> ActiveClientOperationGuard {
        let attrs = [KeyValue::new(
            labels::SUBSCRIPTION_TRANSPORT,
            transport.as_str(),
        )];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::SUBSCRIPTIONS_CLIENTS_ACTIVE, &attrs);
        if let Some(c) = &self.clients_active {
            c.add(1, &attrs);
        }
        if let Some(c) = &self.clients_operations_total {
            let attrs = [
                KeyValue::new(labels::SUBSCRIPTION_TRANSPORT, transport.as_str()),
                KeyValue::new(
                    labels::SUBSCRIPTION_OPERATION,
                    SubscriptionOperation::Subscribe.as_str(),
                ),
            ];
            #[cfg(debug_assertions)]
            debug_assert_attrs(names::SUBSCRIPTIONS_CLIENTS_OPERATIONS_TOTAL, &attrs);
            c.add(1, &attrs);
        }
        ActiveClientOperationGuard {
            counter: self.clients_active.clone(),
            operations_total: self.clients_operations_total.clone(),
            transport,
        }
    }

    pub fn active_client_connection(
        &self,
        transport: SubscriptionTransport,
    ) -> ActiveClientConnectionGuard {
        let attrs = [KeyValue::new(
            labels::SUBSCRIPTION_TRANSPORT,
            transport.as_str(),
        )];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::SUBSCRIPTIONS_CLIENTS_CONNECTIONS, &attrs);
        if let Some(c) = &self.clients_connections {
            c.add(1, &attrs);
        }
        ActiveClientConnectionGuard {
            counter: self.clients_connections.clone(),
            transport,
        }
    }

    /// Records a single subgraph message dropped because of a slow consumer.
    pub fn record_message_dropped(&self, transport: SubscriptionTransport) {
        let attrs = [KeyValue::new(
            labels::SUBSCRIPTION_TRANSPORT,
            transport.as_str(),
        )];
        #[cfg(debug_assertions)]
        debug_assert_attrs(
            names::SUBSCRIPTIONS_SUBGRAPHS_DROPPED_MESSAGES_TOTAL,
            &attrs,
        );
        if let Some(c) = &self.subgraphs_dropped_messages_total {
            c.add(1, &attrs);
        }
    }

    /// Records `n` messages skipped by a slow client subscriber due to broadcast lag.
    pub fn record_client_lag(&self, transport: SubscriptionTransport, n: u64) {
        let attrs = [KeyValue::new(
            labels::SUBSCRIPTION_TRANSPORT,
            transport.as_str(),
        )];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::SUBSCRIPTIONS_CLIENTS_LAGGED_MESSAGES_TOTAL, &attrs);
        if let Some(c) = &self.clients_lagged_messages_total {
            c.add(n, &attrs);
        }
    }
}

pub struct ActiveSubgraphOperationGuard {
    counter: Option<UpDownCounter<i64>>,
    operations_total: Option<Counter<u64>>,
    subgraph_name: String,
}

impl Drop for ActiveSubgraphOperationGuard {
    fn drop(&mut self) {
        if let Some(c) = &self.counter {
            c.add(
                -1,
                &[KeyValue::new(
                    labels::SUBGRAPH_NAME,
                    self.subgraph_name.clone(),
                )],
            );
        }
        if let Some(c) = &self.operations_total {
            c.add(
                1,
                &[
                    KeyValue::new(labels::SUBGRAPH_NAME, self.subgraph_name.clone()),
                    KeyValue::new(
                        labels::SUBSCRIPTION_OPERATION,
                        SubscriptionOperation::Unsubscribe.as_str(),
                    ),
                ],
            );
        }
    }
}

pub struct ActiveSubgraphConnectionGuard {
    counter: Option<UpDownCounter<i64>>,
    subgraph_name: String,
    transport: SubscriptionTransport,
}

impl Drop for ActiveSubgraphConnectionGuard {
    fn drop(&mut self) {
        if let Some(c) = &self.counter {
            c.add(
                -1,
                &[
                    KeyValue::new(labels::SUBGRAPH_NAME, self.subgraph_name.clone()),
                    KeyValue::new(labels::SUBSCRIPTION_TRANSPORT, self.transport.as_str()),
                ],
            );
        }
    }
}

pub struct ActiveClientOperationGuard {
    counter: Option<UpDownCounter<i64>>,
    operations_total: Option<Counter<u64>>,
    transport: SubscriptionTransport,
}

impl Drop for ActiveClientOperationGuard {
    fn drop(&mut self) {
        if let Some(c) = &self.counter {
            c.add(
                -1,
                &[KeyValue::new(
                    labels::SUBSCRIPTION_TRANSPORT,
                    self.transport.as_str(),
                )],
            );
        }
        if let Some(c) = &self.operations_total {
            c.add(
                1,
                &[
                    KeyValue::new(labels::SUBSCRIPTION_TRANSPORT, self.transport.as_str()),
                    KeyValue::new(
                        labels::SUBSCRIPTION_OPERATION,
                        SubscriptionOperation::Unsubscribe.as_str(),
                    ),
                ],
            );
        }
    }
}

pub struct ActiveClientConnectionGuard {
    counter: Option<UpDownCounter<i64>>,
    transport: SubscriptionTransport,
}

impl Drop for ActiveClientConnectionGuard {
    fn drop(&mut self) {
        if let Some(c) = &self.counter {
            c.add(
                -1,
                &[KeyValue::new(
                    labels::SUBSCRIPTION_TRANSPORT,
                    self.transport.as_str(),
                )],
            );
        }
    }
}
