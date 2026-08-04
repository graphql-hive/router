---
hive-router: patch
---

# Register the state the subscription callback handler extracts

The subscription callback handler could not serve a single request: every call failed the `ntex` `State` extractor with `App state is not configured, to configure use App::state()`, returning a 500 to the subgraph. Because callbacks are how the subgraph delivers events back to the router, any subscription using the callback transport silently stopped producing messages.

`handle_callback` extracts two pieces of state:

```rust
callback_subscriptions: web::types::State<CallbackSubscriptionsMap>,
telemetry_context:      web::types::State<std::sync::Arc<TelemetryContext>>,
```

Neither listener registered a matching pair:

- the **dedicated** callback server (`subscriptions.callback.listen` set) registered the map, but passed `telemetry.context` — a bare `TelemetryContext`, since `Telemetry::context` is not an `Arc` — where the handler asks for `State<Arc<TelemetryContext>>`;
- the **main** server (no `listen`, callback mounted on the main app) registered `Arc<TelemetryContext>` via `shared_state.telemetry_context`, but never registered `CallbackSubscriptionsMap` at all.

`State<T>` is generic, so both mismatches compile; they only fail when the extractor runs. That makes the callback protocol broken in **both** configurations, with no config-level workaround.

This registers an `Arc` on the dedicated server and adds the missing map on the main one.
