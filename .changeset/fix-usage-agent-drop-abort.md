---
hive-router: patch
hive-console-sdk: patch
hive-apollo-router-plugin: patch
---

Fix the router process aborting on supergraph hot-reload when `telemetry.hive.usage_reporting` is enabled.

Retiring a supergraph drops the previous generation's Hive usage-reporting agent.

That agent used to flush on drop via a blocking bridge (`block_in_place`) that is only valid on a multi-threaded `tokio` runtime.
The router runs on ntex's current-thread runtime, so this always panicked, and since the panic happened inside a `Drop` impl, it escalated to a full process abort (`panic in a destructor during cleanup`) instead of just failing the reload.

The agent now uses a non-blocking flush, best-effort operation instead.

Fixes https://github.com/graphql-hive/router/issues/1439
