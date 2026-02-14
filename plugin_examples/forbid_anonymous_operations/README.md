# Forbid Anonymous Operations

This example plugin forbids anonymous GraphQL operations. It checks if the incoming GraphQL request has an operation name, and if not, it returns an error response in `on_graphql_params` hook.

## How to run?

```bash
cargo run --package forbid-anonymous-operations-plugin-example
```

## How to short circuit

Using `payload.end_with_graphql_error`, we can short circuit the request and return an error response immediately without executing the GraphQL operation. In this example, we check if the operation name is empty or not provided, and if so, we return a 400 Bad Request response with a GraphQL error message.

```rust
if maybe_operation_name.is_none_or(|operation_name| operation_name.is_empty()) {
    // let's log the error
    tracing::error!("Operation is not allowed!");

    // Prepare an HTTP 400 response with a GraphQL error message
    return payload.end_with_graphql_error(
        GraphQLError::from_message_and_code(
            "Anonymous operations are not allowed",
            "ANONYMOUS_OPERATION",
        ),
        StatusCode::BAD_REQUEST,
    );
}
```