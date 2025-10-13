# Context Data

This example demonstrates how to pass the data between different hooks using the context.

## How to run?

```bash
cargo run --package context-data-plugin-example
```

## What does the plugin do?

1. Place some information in context in `on_graphql_params`. (world!)
2. Pick up and print it out at subgraph request. (Hello world!)
3. For each subgraph response merge some information into the `ContextData`. (response_count)
4. Pick up and print it out in `on_end` of `on_graphql_params`. (response_count)

## Inserting data into context

`payload.context` can accept any type but that type will be a singleton in the scope of a request.

```rust
struct ContextData {
    incoming_data: String,
    response_count: u64,
}
```

In order to add this data to the context, we can use `payload.context.insert` in `on_graphql_params` hook. This will make the data available in the context for the rest of the request lifecycle.

```rust
payload.context.insert(ContextData {
    incoming_data: "world".to_string(),
    response_count: 0,
});
```

### Mutating data in context

In order to mutate the data in the context, we can use `payload.context.get_mut` to get a mutable reference to the data and then mutate it.

```rust
if let Some(context_data) = payload.context.get_mut::<ContextData>() {
    context_data.response_count += 1;
}
```

### Reading data in context

In order to read the data in the context, we can use `payload.context.get` to get a reference to the data and then read it.

```rust
if let Some(context_data) = payload.context.get_ref::<ContextData>() {
    println!("incoming_data: {}", context_data.incoming_data);
    println!("response_count: {}", context_data.response_count);
}
```

