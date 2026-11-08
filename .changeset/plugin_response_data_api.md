---
hive-router: minor
---

# A data API for the `on_execute` end hook

`OnExecuteEndHookPayload::data` is now a `ResponseData` instead of the executor's internal `Value`. A plugin walks the data value by value with `walk`, seeing only what the operation asked for, and can change it only in ways that keep the response matching the operation and the schema:

- `set` a scalar or enum to a value of its type. Anything else is refused with a `DataEditError` and the value stays as it was.
- `set_null` any value, or `set_null_with_error` to also add an error at its path. In a Non-Null position the null bubbles up to the closest nullable parent, like a null from a subgraph.

```rust
payload.on_end(|mut payload| {
    payload.data.walk(|value| {
        if value.is("User", "email") {
            value.set_null_with_error("email is private");
        }
    });
    payload.proceed()
})
```

Each value knows its field (`field_name`, `response_key`, `parent_type`, `is("Type", "field")`), its position (`index`, `path`, `is_non_null`) and can be read (`as_str`, `as_i64`, `as_f64`, `as_bool`, `is_null`, `list_len`). `skip_children` leaves out what's under a value. `to_json` returns the data as the client will get it.

The walk reads and changes the router's own storage in place, on both execution pipelines. Walking allocates nothing, and on the slot pipeline neither does setting values: they go into a buffer reused across requests.

Plugins that read `payload.data` as a `Value`, or replaced it, need to move to `walk`.
