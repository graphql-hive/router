use opentelemetry::{
    metrics::{Counter, Meter, UpDownCounter},
    KeyValue,
};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names, units};

#[derive(Clone, Copy, strum::IntoStaticStr)]
pub enum WebSocketPoolOperationType {
    #[strum(serialize = "execute")]
    Execute,
    #[strum(serialize = "subscribe")]
    Subscribe,
}

impl WebSocketPoolOperationType {
    fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Copy, strum::IntoStaticStr)]
pub enum WebSocketPoolConnectionCloseReason {
    #[strum(serialize = "idle")]
    Idle,
    #[strum(serialize = "dispatcher")]
    Dispatcher,
    #[strum(serialize = "pool_dropped")]
    PoolDropped,
}

impl WebSocketPoolConnectionCloseReason {
    fn as_str(self) -> &'static str {
        self.into()
    }
}

pub struct WebSocketPoolMetrics {
    connections_active: Option<UpDownCounter<i64>>,
    connection_initializations_total: Option<Counter<u64>>,
    connection_initialization_waiters_total: Option<Counter<u64>>,
    connection_lookups_total: Option<Counter<u64>>,
    connections_closed_total: Option<Counter<u64>>,
    operations_active: Option<UpDownCounter<i64>>,
    operations_started_total: Option<Counter<u64>>,
}

impl WebSocketPoolMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let connections_active = meter.map(|meter| {
            meter
                .i64_up_down_counter(names::WEBSOCKET_POOL_CONNECTIONS_ACTIVE)
                .with_description("Active connections in the WebSocket pool.")
                .with_unit(units::CONNECTIONS)
                .build()
        });
        let connection_initializations_total = meter.map(|meter| {
            meter
                .u64_counter(names::WEBSOCKET_POOL_CONNECTION_INITIALIZATIONS_TOTAL)
                .with_description("Completed WebSocket pool connection initialization attempts.")
                .with_unit(units::CONNECTIONS)
                .build()
        });
        let connection_initialization_waiters_total = meter.map(|meter| {
            meter
                .u64_counter(names::WEBSOCKET_POOL_CONNECTION_INITIALIZATION_WAITERS_TOTAL)
                .with_description(
                    "Requests that waited for an in-progress connection initialization.",
                )
                .build()
        });
        let connection_lookups_total = meter.map(|meter| {
            meter
                .u64_counter(names::WEBSOCKET_POOL_CONNECTION_LOOKUPS_TOTAL)
                .with_description("Attempts to reuse an existing WebSocket pool connection.")
                .build()
        });
        let connections_closed_total = meter.map(|meter| {
            meter
                .u64_counter(names::WEBSOCKET_POOL_CONNECTIONS_CLOSED_TOTAL)
                .with_description("WebSocket pool connections closed.")
                .with_unit(units::CONNECTIONS)
                .build()
        });
        let operations_active = meter.map(|meter| {
            meter
                .i64_up_down_counter(names::WEBSOCKET_POOL_OPERATIONS_ACTIVE)
                .with_description("Active operations using WebSocket pool connections.")
                .build()
        });
        let operations_started_total = meter.map(|meter| {
            meter
                .u64_counter(names::WEBSOCKET_POOL_OPERATIONS_STARTED_TOTAL)
                .with_description("Operations started through the WebSocket pool.")
                .build()
        });

        Self {
            connections_active,
            connection_initializations_total,
            connection_initialization_waiters_total,
            connection_lookups_total,
            connections_closed_total,
            operations_active,
            operations_started_total,
        }
    }

    pub fn record_connection_initialization(&self, subgraph_name: &str, succeeded: bool) {
        let attrs = [
            KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
            KeyValue::new(labels::RESULT, if succeeded { "success" } else { "error" }),
        ];
        #[cfg(debug_assertions)]
        debug_assert_attrs(
            names::WEBSOCKET_POOL_CONNECTION_INITIALIZATIONS_TOTAL,
            &attrs,
        );
        if let Some(counter) = &self.connection_initializations_total {
            counter.add(1, &attrs);
        }
    }

    pub fn record_connection_initialization_waiter(&self, subgraph_name: &str) {
        let attrs = [KeyValue::new(
            labels::SUBGRAPH_NAME,
            subgraph_name.to_string(),
        )];
        #[cfg(debug_assertions)]
        debug_assert_attrs(
            names::WEBSOCKET_POOL_CONNECTION_INITIALIZATION_WAITERS_TOTAL,
            &attrs,
        );
        if let Some(counter) = &self.connection_initialization_waiters_total {
            counter.add(1, &attrs);
        }
    }

    pub fn record_connection_lookup(&self, subgraph_name: &str, found: bool) {
        let attrs = [
            KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
            KeyValue::new(labels::RESULT, if found { "hit" } else { "miss" }),
        ];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::WEBSOCKET_POOL_CONNECTION_LOOKUPS_TOTAL, &attrs);
        if let Some(counter) = &self.connection_lookups_total {
            counter.add(1, &attrs);
        }
    }

    pub fn active_connection(&self, subgraph_name: &str) -> ActiveConnectionGuard {
        let attrs = [KeyValue::new(
            labels::SUBGRAPH_NAME,
            subgraph_name.to_string(),
        )];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::WEBSOCKET_POOL_CONNECTIONS_ACTIVE, &attrs);
        if let Some(counter) = &self.connections_active {
            counter.add(1, &attrs);
        }
        ActiveConnectionGuard {
            counter: self.connections_active.clone(),
            subgraph_name: subgraph_name.to_string(),
        }
    }

    pub fn record_connection_closed(
        &self,
        subgraph_name: &str,
        reason: WebSocketPoolConnectionCloseReason,
    ) {
        let attrs = [
            KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
            KeyValue::new(
                labels::WEBSOCKET_POOL_CONNECTION_CLOSE_REASON,
                reason.as_str(),
            ),
        ];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::WEBSOCKET_POOL_CONNECTIONS_CLOSED_TOTAL, &attrs);
        if let Some(counter) = &self.connections_closed_total {
            counter.add(1, &attrs);
        }
    }

    pub fn active_operation(
        &self,
        subgraph_name: &str,
        operation_type: WebSocketPoolOperationType,
    ) -> ActiveOperationGuard {
        let attrs = operation_attrs(subgraph_name, operation_type);
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::WEBSOCKET_POOL_OPERATIONS_ACTIVE, &attrs);
        if let Some(counter) = &self.operations_active {
            counter.add(1, &attrs);
        }
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::WEBSOCKET_POOL_OPERATIONS_STARTED_TOTAL, &attrs);
        if let Some(counter) = &self.operations_started_total {
            counter.add(1, &attrs);
        }
        ActiveOperationGuard {
            counter: self.operations_active.clone(),
            subgraph_name: subgraph_name.to_string(),
            operation_type,
        }
    }
}

fn operation_attrs(
    subgraph_name: &str,
    operation_type: WebSocketPoolOperationType,
) -> [KeyValue; 2] {
    [
        KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
        KeyValue::new(
            labels::WEBSOCKET_POOL_OPERATION_TYPE,
            operation_type.as_str(),
        ),
    ]
}

pub struct ActiveConnectionGuard {
    counter: Option<UpDownCounter<i64>>,
    subgraph_name: String,
}

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        if let Some(counter) = &self.counter {
            counter.add(
                -1,
                &[KeyValue::new(
                    labels::SUBGRAPH_NAME,
                    self.subgraph_name.clone(),
                )],
            );
        }
    }
}

pub struct ActiveOperationGuard {
    counter: Option<UpDownCounter<i64>>,
    subgraph_name: String,
    operation_type: WebSocketPoolOperationType,
}

impl Drop for ActiveOperationGuard {
    fn drop(&mut self) {
        if let Some(counter) = &self.counter {
            counter.add(
                -1,
                &operation_attrs(&self.subgraph_name, self.operation_type),
            );
        }
    }
}
