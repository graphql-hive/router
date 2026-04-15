# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.0.2](https://github.com/graphql-hive/router/compare/hive-router-query-planner-v2.0.1...hive-router-query-planner-v2.0.2) - 2025-10-27

### <!-- 1 -->Bug Fixes

- *(query-planner, router)* fix introspection for federation v1 supergraph ([#526](https://github.com/graphql-hive/router/pull/526))

## [2.0.1](https://github.com/graphql-hive/router/compare/hive-router-query-planner-v2.0.0...hive-router-query-planner-v2.0.1) - 2025-10-23

### Fixed

- *(query-planner)* inline nested fragment spreads ([#502](https://github.com/graphql-hive/router/pull/502))

## [2.0.0](https://github.com/graphql-hive/router/compare/hive-router-query-planner-v1.0.0...hive-router-query-planner-v2.0.0) - 2025-10-05

### Other

- *(deps)* update actions-rust-lang/setup-rust-toolchain digest to 1780873 ([#466](https://github.com/graphql-hive/router/pull/466))

## [1.0.0](https://github.com/graphql-hive/router/compare/hive-router-query-planner-v0.0.1...hive-router-query-planner-v1.0.0) - 2025-09-01

### Other

- *(deps)* update release-plz/action action to v0.5.113 ([#389](https://github.com/graphql-hive/router/pull/389))
## 2.6.0 (2026-04-15)

### Features

#### Query Plan Subscriptions Node

The query planner now emits a `Subscription` node when planning a subscription operation. The `Subscription` node contains a `primary` fetch that is sent to the subgraph owning the subscription field.

## 2.5.2 (2026-04-13)

### Fixes

- planning for conditional inline fragments and field conditions (#894)
- Version bump to fix release issues

#### Fix planning for conditional inline fragments and field conditions

Fixed a query-planner bug where directive-only inline fragments (using `@include`/`@skip` without an explicit type condition) could fail during normalization/planning for deeply nested operations.

This update improves planner handling for conditional selections and adds regression tests to prevent these failures in the future.

## 2.5.1 (2026-03-31)

### Fixes

- preserve client aliases in mismatch output rewrites (#870)

#### Preserve client aliases in mismatch rewrites

Fixed query planner mismatch handling so conflicting fields are tracked by response key (alias-aware), and internal alias rewrites restore the original client-facing key (alias-or-name) instead of always the schema field name.

## 2.5.0 (2026-03-16)

### Features

#### Introduce BatchFetch for compatible entity fetches to improve query performance

When multiple `Flatten(Fetch)` steps target the same subgraph and have compatible shape, the planner can group them into one batched fetch operation with aliases.

Batching keeps execution depth the same, but **reduces request fanout**.
In our benchmark query, **downstream requests drop from `13` to `7`** while the number of execution waves stays unchanged.
This should also reduce pressure on subgraphs, because entities are resolved in one batched subgraph call instead of being resolved across multiple incoming GraphQL requests, where the lack of DataLoader or another caching layer could otherwise cause duplicate resolution work.

Before: 

```graphql
Parallel {
  Flatten(path: "products.@") {
    Fetch(service: "inventory") {
      {
        ... on Product {
          upc
        }
      } =>
      {
        ... on Product {
          shippingEstimate
        }
      }
    }
  }
  Flatten(path: "topProducts.@") {
    Fetch(service: "inventory") {
      {
        ... on Product {
          upc
        }
      } =>
      {
        ... on Product {
          shippingEstimate
        }
      }
    }
  }
}
```

After:

```graphql
BatchFetch(service: "inventory") {
  {
    _e0 {
      paths: [
        "products.@"
        "topProducts.@"
      ]
      {
        ... on Product {
          upc
        }
      }
    }
  }
  {
    _e0: _entities(representations: $__batch_reps_0) {
      ... on Product {
        shippingEstimate
      }
    }
  }
}
```

When two entity fetches go to the same subgraph but request different output fields, they are batched into one `BatchFetch` node with two aliases, but share the same variables, to reduce the payload size.

```
BatchFetch(service: "inventory") {
  {
    _e0 {
      paths: [
        "products.@"
      ]
      {
        ... on Product {
          upc
        }
      }
    }
    _e1 {
      paths: [
        "products.@"
      ]
      {
        ... on Product {
          upc
        }
      }
    }
  }
  {
    _e0: _entities(representations: $__batch_reps_0) {
      ... on Product {
        shippingEstimate
      }
    }
    _e1: _entities(representations: $__batch_reps_0) {
      ... on Product {
        inStock
      }
    }
  }
}
```

### Fixes

- Implements `AsRef` trait for `graphql_tools::parser::query::ast::TypeCondition`

#### Public API Changes

### Progressive Override support in `QueryPlanner.plan`

Now `QueryPlanner.plan` accepts two additional parameters: `activeLabels` and `percentageValue`. These parameters are used to determine which overrides should be applied when generating the query plan. The `activeLabels` parameter is a set of labels that are currently active, and the `percentageValue` parameter is a number between 0 and 100 that represents the percentage of traffic that should be routed to the overrides.

### `AbortSignal` support in `QueryPlanner.plan`

The `QueryPlanner.plan` method now also accepts an optional `signal` parameter of type `AbortSignal`. This allows the caller to abort the query planning process if it takes too long or if the user cancels the operation. If the signal is aborted, the `plan` method will throw an error.

### `overrideLabels` and `overridePercentages` getters

Two new getters have been added to the `QueryPlanner` class: `overrideLabels` and `overridePercentages`. The `overrideLabels` getter returns a set of all the labels that are defined in the planner's supergraph, while the `overridePercentages` getter returns an array of all the percentage values that are defined in the planner's supergraph. These getters can be used by the caller to determine which overrides are available and how they are configured.

### `QueryPlanner.plan` is no longer a `Promise`

The `QueryPlanner.plan` method is now a synchronous method that returns a `QueryPlan` directly, instead of returning a `Promise`. This change was made to simplify the API and to allow for better error handling. If the query planning process encounters an error, it will throw an exception that can be caught by the caller.

### `QueryPlanner.planAsync` is now a `Promise`

The `QueryPlanner.planAsync` method is now an asynchronous method that returns a `Promise` that resolves to a `QueryPlan`. This method is intended for use cases where the query planning process may take a long time, and the caller wants to avoid blocking the main thread. The `planAsync` method accepts the same parameters as the `plan` method, including the new `activeLabels`, `percentageValue`, and `signal` parameters.

### `QueryPlanner` constructor now uses `safe_parse_schema`

The `QueryPlanner` constructor now uses the `safe_parse_schema` function to parse the supergraph SDL. This function is a safer alternative to the previous parsing method, as it returns a `Result` that can be handled gracefully in case of parsing errors. If the SDL cannot be parsed, the constructor will return an error instead of panicking.

## Implementation changes

- The `QueryPlanner` struct now holds a `Planner` instance directly, instead of an `Arc<Planner>`. This change was made to simplify the internal implementation and to avoid unnecessary reference counting. Since the `QueryPlanner` is not designed to be shared across threads, there is no need for the additional overhead of an `Arc`.

- `AbortSignal` and `CancellationToken` integration to give the ability to cancel the query planning process to the Node addon consumer.

- `QueryPlanner.planAsync` is introduced with [`AsyncTask`](https://napi.rs/docs/concepts/async-tasks) to allow for non-blocking query planning in the Node addon.

## 2.4.0 (2026-03-12)

### Features

#### Metrics with OpenTelemetry and Prometheus

This release adds support for OpenTelemetry metrics. In addition to existing tracing support, the router can now collect detailed metrics about HTTP and GraphQL activity and export them to a Prometheus endpoint or to an OTLP collector.

- Telemetry configuration now has a `metrics` section. Users can enable metrics exporters and tune histogram buckets under `telemetry.metrics` in `router.config.yaml`. By default metrics are disabled, so existing configurations continue to work unchanged.
- **Prometheus exporter** exposes a `/metrics` endpoint that follows the standard Prometheus text format. It can be attached to Router's http server or run on its own port. 
- **OTLP exporter** is available for sending metrics to an OpenTelemetry collector via gRPC or HTTP.
- **Instrumentation for every stage of the pipeline** - parsing, normalization, validation, planning and execution.
- **HTTP client/server metrics** - Router records metrics for incoming HTTP requests (latencies, sizes and status codes) and for outbound subgraph requests. These instruments follow the OpenTelemetry HTTP semantic conventions, making them usable out‑of‑the‑box with observability backends.
- **Supergraph reload metrics** - polling and reloading the supergraph is measured with poll counts, durations and errors, giving visibility into slow or failed schema reloads.

**Example configuration**

```yaml
telemetry:
  metrics:
    exporters:
      - prometheus:
          enabled: true
          # optional custom path (default `/metrics`)
          path: /metrics
          # serve on this port
          port: 9090
      - otlp:
          enabled: true
          # An absolute path to the OpenTelemetry collector
          endpoint: "http://otel-collector:4317"
          # protocol can be `grpc` or `http`
          protocol: http
    instrumentation:
      instruments:
        # Disable HTTP server request duration metric
        http.server.request.duration: false
        http.client.request.duration:
          attributes:
            # Disable the label
            graphql.operation.name: false
```

Visit ["OpenTelemetry Metrics" documentation](https://the-guild.dev/graphql/hive/docs/router/observability/metrics) for more details on configuring metrics and exporters.

## 2.3.0 (2026-03-05)

### Features

- use multi-type TypeCondition paths and batch sibling entity fetches (#280)

#### Improve Query Plans for abstract types

The query planner now combines fetches for multiple matching types into a single fetch step.
Before, the planner could create one fetch per type.
Now, it can fetch many types together when possible, which reduces duplicate fetches and makes query plans more efficient.

#### Rename internal query-plan path segment from `Cast(String)` to `TypeCondition(Vec<String>)`

Query Plan shape changed from `Cast(String)` to `TypeCondition(Vec<String>)`.
The `TypeCondition` name better reflects GraphQL semantics (`... on Type`) and avoids string encoding/decoding like `"A|B"` in planner/executor code.

**What changed**
- Query planner path model now uses `TypeCondition` terminology instead of `Cast`.
- Type conditions are represented as a list of type names, not a pipe-delimited string.
- Node addon query-plan typings were updated accordingly:
  - `FetchNodePathSegment.TypenameEquals` now uses `string[]`
  - `FlattenNodePathSegment` now uses `TypeCondition: string[]` (instead of `Cast: string`)

#### Plugin System

This release introduces a Plugin System that allows users to extend the functionality of Hive Router by creating custom plugins.

```rust
use hive_router::plugins::plugin_trait::RouterPlugin;
use hive_router::async_trait;
 
struct MyPlugin;
 
##[async_trait]
impl RouterPlugin for MyPlugin {
    type Config = ();
 
    fn plugin_name() -> &'static str {
        "my_plugin"
    }
}
```

You can learn more about the plugin system in the [technical documentation](https://the-guild.dev/graphql/hive/docs/router/plugin-system) and in [Extending the Router guide](https://the-guild.dev/graphql/hive/docs/router/guides/extending-the-router).

This new feature also exposes many of the Router's internals through the [`hive-router` crate](https://crates.io/crates/hive-router).

### Fixes

#### Internal GraphQL Validation Cache Key

- `ConsumerSchema` and `ValidationPlan` now implement `hash` property, which is calculated based on the SDL string of the consumer schema and the validation rules when the struct is created or when a new rule is added to the validation plan.
- Validation cache key is generated by hashing the SDL string of the consumer schema, and the validation rules together with the operation itself.
- All schema AST nodes now implement `Hash` trait, which allows us to hash the entire schema AST when generating the validation cache key.

## 2.2.1 (2026-02-10)

### Fixes

#### Tracing with OpenTelemetry

Introducing comprehensive OpenTelemetry-based tracing to the Hive Router, providing deep visibility into the GraphQL request lifecycle and subgraph communications.

- **OpenTelemetry Integration**: Support for OTLP exporters (gRPC and HTTP) and standard propagation formats (Trace Context, Baggage, Jaeger, B3/Zipkin).
- **GraphQL-Specific Spans**: Detailed spans for every phase of the GraphQL lifecycle
- **Hive Console Tracing**: Native integration with Hive Console for trace visualization and analysis.
- **Semantic Conventions**: Support for both stable and deprecated OpenTelemetry HTTP semantic conventions to ensure compatibility with a wide range of observability tools.
- **Optimized Performance**: Tracing is designed with a "pay only for what you use" approach. Overhead is near-zero when disabled, and allocations/computations are minimized when enabled.
- **Rich Configuration**: New configuration options for telemetry exporters, batching, and resource attributes.

## 2.2.0 (2026-02-06)

### Features

- Operation Complexity - Limit Aliases (#746)
- Operation Complexity - Limit Aliases (#749)

## 2.1.7 (2026-01-22)

### Fixes

#### Refactor Parse Error Handling in `graphql-tools`

Breaking;
- `ParseError(String)` is now `ParseError(InternalError<'static>)`.
- - So that the internals of the error can be better structured and more informative, such as including line and column information.
- `ParseError`s are no longer prefixed with "query parse error: " in their Display implementation.

## 2.1.6 (2026-01-14)

### Fixes

#### Moves `graphql-tools` to router repository

This change moves the `graphql-tools` package to the Hive Router repository.

## Own GraphQL Parser

This change also introduces our own GraphQL parser (copy of `graphql_parser`), which is now used across all packages in the Hive Router monorepo. This allows us to have better control over parsing and potentially optimize it for our specific use cases.

## 2.1.5 (2025-12-11)

### Fixes

- chore: Enable publishing of internal crate

## 2.1.4 (2025-12-11)

### Fixes

- prevent planner failure when combining conditional directives and interfaces (#602)
- Strip `@join__directive` and `join__DirectiveArguments` internal types while creating the consumer/public schema

#### Prevent planner failure when combining conditional directives and interfaces

Fixed a bug where the query planner failed to handle the combination of conditional directives (`@include`/`@skip`) and the automatic `__typename` injection required for abstract types.

## 2.1.3 (2025-12-08)

### Fixes

- Bump dependencies

## 2.1.2 (2025-11-28)

### Fixes

- make supergraph.{path,key,endpoint} optional (#593)

## 2.1.1 (2025-11-28)

### Fixes

- support `@include` and `@skip` in initial fetch node (#591)
- Fixed an issue where `@skip` and `@include` directives were incorrectly removed from the initial Fetch of the Query Plan.

## 2.1.0 (2025-11-24)

### Features

#### Directive-Based Authorization

Introducing directive-based authorization. This allows you to enforce fine-grained access control directly from your subgraph schemas using the `@authenticated` and `@requiresScopes` directives.

This new authorization layer runs before the query planner, ensuring that unauthorized requests are handled efficiently without reaching your subgraphs.

#### Configuration

You can configure how the router handles unauthorized requests with two modes:

- **`filter`** (default): Silently removes any fields the user is not authorized to see from the query. The response will contain `null` for the removed fields and an error in the `errors` array.
- **`reject`**: Rejects the entire GraphQL operation if it requests any field the user is not authorized to access.

To configure this, add the following to your `router.yaml` configuration file:

```yaml
authentication:
  directives:
    unauthorized:
      # "filter" (default): Removes unauthorized fields from the query and returns errors.
      # "reject": Rejects the entire request if any unauthorized field is requested.
      mode: reject
```

If this section is omitted, the router will use `filter` mode by default.

#### JWT Scope Requirements

When using the `@requiresScopes` directive, the router expects the user's granted scopes to be present in the JWT payload. The scopes should be in an array of strings or a string (scopes separated by space), within a claim named `scope`.

Here is an example of a JWT payload with the correct format:

```json
{
  "sub": "user-123",
  "scope": [
    "read:products",
    "write:reviews"
  ],
  "iat": 1516239022
}
```

### Fixes

#### Avoid extra `query` prefix for anonymous queries

When there is no variable definitions and no operation name, GraphQL queries can be sent without the `query` prefix. For example, instead of sending:

```diff
- query {
+ {
  user(id: "1") {
    name
  }
}
```
