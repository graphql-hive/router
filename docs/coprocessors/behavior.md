# Coprocessor Behavior & Error Handling

This document details the strict behaviors, data exchange rules, and failure handling mechanisms of Coprocessors within the Hive Router.

## 1. Communication Protocol & Payload Format

The Router communicates with the Coprocessor by sending an HTTP POST request containing a JSON payload. The coprocessor must respond with a well-formed JSON object.

### Expected Coprocessor Response
A valid response from the coprocessor must include the following:
- **`version`**: Must be `1`. Any unsupported version (e.g., `999`) will cause the router to reject the response and fail the request.
- **`control`**: Defines the flow. Can be `"continue"` (default behavior if omitted) or `{"break": <status_code>}` to short-circuit the request. Invalid values (e.g., `"jump"`) will result in an immediate failure.

### Applying Mutations
When a coprocessor responds, the Router inspects the payload to apply mutations.
- **Selective Updates**: The Router only updates fields that are explicitly present in the response. If a property is missing from the coprocessor's response, the Router retains the existing value.
- **Top-Level Replacements**: Properties like `headers` or `context` are treated as complete state updates if provided. Setting them to `{}` replaces the current state with an empty object.

## 2. Inclusions vs. Mutations

The router's configuration allows you to conditionally include or exclude data sent to the coprocessor to optimize payload size (e.g., `include: { body: false, headers: false }`).

**Crucial Behavior:** `include` settings *only control the outbound payload* sent from the Router to the Coprocessor. They do not restrict what the coprocessor is allowed to mutate. 
- Even if `include.headers: false` is set, a coprocessor can still return a `headers` object in its response, and the Router will successfully apply those header mutations.
- Even if `include.body: false` is set, the coprocessor can return a modified `body` to patch the request/response (subject to stage constraints).

## 3. Stage-Specific Constraints

Different stages have distinct rules regarding what can and cannot be mutated.

### `graphql.request`
- **Body Structure**: The GraphQL inputs are nested inside a `body` object containing fields like `query`, `variables`, `operation_name`, and `extensions`.
- **Selected Fields**: If configured as `body: [query, extensions]`, only those specific fields are sent to the coprocessor.
- **Safe Patching**: The coprocessor can send back partial updates to the `body` (e.g., injecting variables without modifying the `query`). However, returning a malformed body (like an empty query where one is required) will cause the GraphQL execution to fail.

### `graphql.analysis`
- **Strictly Read-Only Body**: Because the `graphql.analysis` stage runs *after* normalization but before query planning and authorization, **the GraphQL body is strictly read-only**.
- If a coprocessor attempts to return a `body` mutation during the `graphql.analysis` stage, the Router will reject it and immediately fail the request with a `COPROCESSOR_FAILURE`.
- *Note:* The coprocessor is still free to mutate `headers` and `context` during this stage.

## 4. Short-Circuiting (Early Return)

A coprocessor can halt the execution pipeline and immediately return a response to the client by issuing a `break` control flow.

```json
{
  "version": 1,
  "control": { "break": 401 },
  "headers": {
    "content-type": "application/json",
    "x-custom-error": "unauthorized"
  },
  "body": "{\"error\": \"Unauthorized from coprocessor\"}"
}
```

**Behavior on `break`:**
- The router immediately halts downstream execution (e.g., no subgraphs are called).
- The HTTP status code is set to the value provided in the `break` directive (e.g., `401`).
- The router applies any `headers` and `body` provided alongside the `break` control and sends them directly to the client.

## 5. Failure Modes and Resilience

The Router is designed to protect downstream subgraphs and clients from misbehaving coprocessors. If the coprocessor fails to fulfill the protocol contract, the Router aborts the current request phase, preventing it from reaching the subgraphs, and returns a standard HTTP 500 error to the client with the GraphQL error code `COPROCESSOR_FAILURE`.

The Router will trigger a `COPROCESSOR_FAILURE` under the following conditions:

1. **HTTP Error Responses**: The coprocessor returns a non-200 HTTP status code (e.g., `500 Internal Server Error`).
2. **Malformed JSON**: The coprocessor returns an incomplete or syntactically invalid JSON body.
3. **Unsupported Version**: The coprocessor returns a `version` other than `1`.
4. **Invalid Control Flow**: The coprocessor returns an unrecognized `control` value (e.g., `"control": "jump"`).
5. **Timeouts**: The coprocessor takes longer than the configured `timeout` duration to respond. The router refuses to hang indefinitely and forcefully drops the connection.
