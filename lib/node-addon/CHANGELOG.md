# @graphql-hive/router-query-planner changelog
## 0.0.18 (2026-04-01)

### Features

- replace graphiql with laboratory (#791)

## 0.0.17 (2026-04-01)

### Fixes

- This patch includes the fixes in the query planner including the fixes for mismatch handling so conflicting fields are tracked by response key (alias-aware), and internal alias rewrites restore the original client-facing key (alias-or-name) instead of always the schema field name.

## 0.0.16 (2026-03-16)

### Fixes

- Add missing `*.node` binaries to the `dist` folder in the distributed package.

## 0.0.15 (2026-03-16)

### Features

- progressive override (#856)

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

## 0.0.14 (2026-03-12)

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

## 0.0.13 (2026-03-05)

### Features

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

## 0.0.12 (2026-02-06)

### Features

- Operation Complexity - Limit Aliases (#746)
- Operation Complexity - Limit Aliases (#749)

## 0.0.11 (2026-01-22)

### Fixes

#### Refactor Parse Error Handling in `graphql-tools`

Breaking;
- `ParseError(String)` is now `ParseError(InternalError<'static>)`.
- - So that the internals of the error can be better structured and more informative, such as including line and column information.
- `ParseError`s are no longer prefixed with "query parse error: " in their Display implementation.

## 0.0.10 (2026-01-14)

### Fixes

#### Moves `graphql-tools` to router repository

This change moves the `graphql-tools` package to the Hive Router repository.

## Own GraphQL Parser

This change also introduces our own GraphQL parser (copy of `graphql_parser`), which is now used across all packages in the Hive Router monorepo. This allows us to have better control over parsing and potentially optimize it for our specific use cases.

## 0.0.9 (2025-12-11)

### Fixes

- chore: Enable publishing of internal crate

## 0.0.8 (2025-12-11)

### Fixes

#### Prevent planner failure when combining conditional directives and interfaces

Fixed a bug where the query planner failed to handle the combination of conditional directives (`@include`/`@skip`) and the automatic `__typename` injection required for abstract types.

## 0.0.7 (2025-12-08)

### Fixes

- Bump dependencies

## 0.0.6 (2025-11-28)

### Fixes

- make supergraph.{path,key,endpoint} optional (#593)

## 0.0.5 (2025-11-28)

### Fixes

- support `@include` and `@skip` in initial fetch node (#591)
- Fixed an issue where `@skip` and `@include` directives were incorrectly removed from the initial Fetch of the Query Plan.

## 0.0.4 (2025-11-24)

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

## 0.0.3 (2025-11-06)

### Fixes

#### CommonJS bindings

Adding support for CJS.

## 0.0.2 (2025-11-05)

### Features

#### A node addon containing the query planner of hive router

To use in TypeScript, you would go ahead and do something like:

```ts
import {
  QueryPlanner,
  type QueryPlan,
} from "@graphql-hive/router-query-planner";

const supergraphSdl = "<your sdl from file or in code>";

const qp = new QueryPlanner(supergraphSdl);

const plan: QueryPlan = qp.plan(/* GraphQL */ `
  {
    posts {
      title
      author {
        name
      }
    }
  }
`);

// see QueryPlan types in lib/node-addon/src/query-plan.d.ts
```

which will generate you a [Query Plan used in Apollo Federation](https://www.apollographql.com/docs/graphos/schema-design/federated-schemas/reference/query-plans).

Hive Gateway uses it as an alternative federation query planner in the [`@graphql-hive/router-runtime`](https://github.com/graphql-hive/gateway/blob/main/packages/router-runtime).

To use in with Hive Gateway, you first install the runtime

```sh
npm i @graphql-hive/router-runtime
```

```ts
// gateway.config.ts
import { defineConfig } from "@graphql-hive/gateway";
import { unifiedGraphHandler } from "@graphql-hive/router-runtime";

export const gatewayConfig = defineConfig({
  unifiedGraphHandler,
});
```
