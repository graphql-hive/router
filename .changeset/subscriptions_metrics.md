---
hive-router-plan-executor: minor
hive-router: minor
---

# Add OpenTelemetry metrics for GraphQL subscriptions

Add end-to-end observability into subscription activity between clients, the router, and subgraphs.

## Live state

| Metric                                            | Labels                                    | Unit             | Description                                                 |
| ------------------------------------------------- | ----------------------------------------- | ---------------- | ----------------------------------------------------------- |
| `hive.router.subscriptions.clients.active`        | `subscription.transport`                  | `{subscription}` | Active subscription operations from clients to the router   |
| `hive.router.subscriptions.clients.connections`   | `subscription.transport`                  | `{connection}`   | Active client transport connections carrying subscriptions  |
| `hive.router.subscriptions.subgraphs.active`      | `subgraph.name`                           | `{subscription}` | Active subscription operations from the router to subgraphs |
| `hive.router.subscriptions.subgraphs.connections` | `subgraph.name`, `subscription.transport` | `{connection}`   | Active transport connections from the router to subgraphs   |

Operations and connections are measured separately because one connection can carry multiple operations, while subscription deduplication can fan one subgraph operation out to multiple clients.

## Lifecycle

| Metric                                              | Labels                                              | Unit             | Description                    |
| --------------------------------------------------- | --------------------------------------------------- | ---------------- | ------------------------------ |
| `hive.router.subscriptions.clients.started_total`   | `subscription.transport`                            | `{subscription}` | Client subscriptions started   |
| `hive.router.subscriptions.clients.ended_total`     | `subscription.transport`, `subscription.end_reason` | `{subscription}` | Client subscriptions ended     |
| `hive.router.subscriptions.subgraphs.started_total` | `subgraph.name`                                     | `{subscription}` | Subgraph subscriptions started |
| `hive.router.subscriptions.subgraphs.ended_total`   | `subgraph.name`                                     | `{subscription}` | Subgraph subscriptions ended   |

Every recorded start has exactly one matching end. Client end reasons are `completed` when the source finishes normally, `error` when an error is delivered to the client, and `client_disconnected` when the client disconnects, unsubscribes, or the router otherwise drops the stream.

The counters expose subscription churn and remain meaningful across router restarts. Comparing start and end rates can reveal mass disconnects or reconnect loops even when the active subscription count appears stable.

## Message delivery

| Metric                                                       | Labels                   | Unit        | Description                                                                |
| ------------------------------------------------------------ | ------------------------ | ----------- | -------------------------------------------------------------------------- |
| `hive.router.subscriptions.clients.sent_messages_total`      | `subscription.transport` | `{message}` | Messages successfully sent to client subscribers                           |
| `hive.router.subscriptions.clients.lagged_messages_total`    | `subscription.transport` | `{message}` | Messages skipped for lagging clients on broadcast fan-out                  |
| `hive.router.subscriptions.subgraphs.dropped_messages_total` | `subscription.transport` | `{message}` | Subgraph messages dropped because an internal subscription buffer was full |

Lagged and dropped counters increase by the number of messages skipped without terminating the subscription. `sent_messages_total / (sent_messages_total + lagged_messages_total)` provides the client delivery ratio for each transport.

`subscription.transport` can be `websocket`, `http_multipart`, `http_sse`, or `http_callback`. Units such as `{subscription}`, `{connection}`, and `{message}` are UCUM annotations that identify what each instrument counts.
