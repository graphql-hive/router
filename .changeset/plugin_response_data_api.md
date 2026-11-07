---
hive-router: minor
---

# A data API for the `on_execute` end hook

`OnExecuteEndHookPayload::data` is now a `ResponseData` instead of the executor's internal `Value`. It holds the data as the client will get it: only the fields the operation asked for, under their response keys. A plugin can read any of it (`get`, `to_json`), and change it only in ways that keep the response matching the operation: `set` a scalar or enum to a value of its type, or `set_null` a nullable position. Anything else is refused with a `DataEditError` and the data stays as it was.

```rust
payload.on_end(|mut payload| {
    let path = ["users".into(), 0.into(), "email".into()];
    payload.data.set_null(&path).ok();
    payload.proceed()
})
```

Plugins that read `payload.data` as a `Value`, or replaced it, need to move to these methods.
