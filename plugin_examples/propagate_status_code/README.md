# Status Code Propagation

This example plugin demonstrates how to propagate status codes from subgraph responses to the client response. The plugin intercepts subgraph responses in the `on_subgraph_response` hook, and saves the status code in the context. Then in the `on_end` hook of `on_http_request`, it reads the status code from the context and sets it as the final response status code.

## How to run?

```bash
cargo run --package propagate-status-code-plugin-example
```

## What does the plugin do?

- Creates a context object `PropagateStatusCodeCtx` with `status_code` field to store the status code from subgraph responses.
- In `on_subgraph_response` hook, it checks the status code of the subgraph response and if it is a code defined in the configuration, it saves that status code in the context. If the existing code in the context is lower than the new one, it updates it.
- In `on_end` hook of `on_http_request`, it reads the status code from the context and if it is set, it uses `payload.map_response` to set the final response status code.

## Context usage

The plugin defines a context struct `PropagateStatusCodeCtx` to store the status code. This context is inserted and updated in the `on_subgraph_http_request` hook and is available throughout the request lifecycle.

```rust
struct PropagateStatusCodeCtx {
    status_code: StatusCode,
}
```

In `on_subgraph_http_request`;

```rust
// Checking if there is already a context entry
let ctx = payload.context.get_mut::<PropagateStatusCodeCtx>();
if let Some(mut ctx) = ctx {
    // Update the status code if the new one is more severe (higher)
    if status_code.as_u16() > ctx.status_code.as_u16() {
        ctx.status_code = status_code;
    }
} else {
    // Insert a new context entry
    let new_ctx = PropagateStatusCodeCtx { status_code };
    payload.context.insert(new_ctx);
}
```
