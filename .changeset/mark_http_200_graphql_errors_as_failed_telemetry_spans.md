---
hive-router: patch
node-addon: patch
---

# Mark HTTP 200 GraphQL errors as failed telemetry spans

GraphQL responses can contain errors while still returning an HTTP 200 status. These responses previously left `graphql.operation` spans without an error status and marked the root `http.server` span as successful. Observability platforms could therefore report a 0% service error rate even while the router was returning GraphQL errors.

The router now sets the OpenTelemetry status to `Error` on both the `graphql.operation` span and the root `http.server` span when the GraphQL response contains errors. This allows tracing backends such as Datadog to include these requests in operation-level and service-level error rates without requiring a custom plugin.

Successful 1xx, 2xx, and 3xx root HTTP spans now leave their OpenTelemetry status unset instead of explicitly setting it to `Ok`. This follows the [OpenTelemetry HTTP semantic conventions](https://opentelemetry.io/docs/specs/semconv/http/http-spans/#status) and prevents a successful HTTP status from overwriting an error detected while processing the GraphQL response. HTTP 5xx responses continue to set the span status to `Error` and record the status code as `error.type`.

A GraphQL response can contain multiple errors with different codes and types, so the router does not assign a single aggregate `error.type` to the root or operation span. Individual error details continue to be recorded as span events.

This change does not alter the handling of manually supplied `datadog.error` attributes. The Datadog exporter receives the standard OpenTelemetry error status and maps it to Datadog's native span error flag.
