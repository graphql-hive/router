---
hive-router-internal: patch
hive-router: patch
hive-router-plan-executor: patch
---

# Fix `http.route` lable value in traces

The `http.route` span attribute was set to the effective route path, instead of the route template. 

The [OTEL specification for HTTP spans](https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-server) describes the `http.router` field as:

> The matched route template for the request. This MUST be low-cardinality and include all static path segments, with dynamic path segments represented with placeholders.

Thanks [praguevara](https://github.com/praguevara) for contributing.
