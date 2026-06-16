# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.10](https://github.com/graphql-hive/router/compare/hive-router-config-v0.0.9...hive-router-config-v0.0.10) - 2025-10-27

### <!-- 0 -->New Features

- *(router)* added support for label overrides with `@override` ([#518](https://github.com/graphql-hive/router/pull/518))
- *(config)* configuration override using env vars, enable/disable graphiql via config ([#519](https://github.com/graphql-hive/router/pull/519))

## [0.0.8](https://github.com/graphql-hive/router/compare/hive-router-config-v0.0.7...hive-router-config-v0.0.8) - 2025-10-23

### Added

- *(router)* support `hive` as source for supergraph ([#400](https://github.com/graphql-hive/router/pull/400))

### Other

- Rename default config file to router.config ([#493](https://github.com/graphql-hive/router/pull/493))

## [0.0.7](https://github.com/graphql-hive/router/compare/hive-router-config-v0.0.6...hive-router-config-v0.0.7) - 2025-10-16

### Added

- *(router)* Subgraph endpoint overrides ([#488](https://github.com/graphql-hive/router/pull/488))
- *(router)* jwt auth ([#455](https://github.com/graphql-hive/router/pull/455))
- *(router)* CORS support ([#473](https://github.com/graphql-hive/router/pull/473))
- *(router)* CSRF prevention for browser requests ([#472](https://github.com/graphql-hive/router/pull/472))

### Fixed

- *(router)* improve csrf and other configs  ([#487](https://github.com/graphql-hive/router/pull/487))

## [0.0.6](https://github.com/graphql-hive/router/compare/hive-router-config-v0.0.5...hive-router-config-v0.0.6) - 2025-10-08

### Added

- *(router)* Advanced Header Management ([#438](https://github.com/graphql-hive/router/pull/438))

## [0.0.5](https://github.com/graphql-hive/router/compare/hive-router-config-v0.0.4...hive-router-config-v0.0.5) - 2025-10-05

### Other

- *(deps)* update actions-rust-lang/setup-rust-toolchain digest to 1780873 ([#466](https://github.com/graphql-hive/router/pull/466))

## [0.0.4](https://github.com/graphql-hive/router/compare/hive-router-config-v0.0.3...hive-router-config-v0.0.4) - 2025-09-09

### Other

- update Cargo.lock dependencies

## [0.0.3](https://github.com/graphql-hive/router/compare/hive-router-config-v0.0.2...hive-router-config-v0.0.3) - 2025-09-02

### Fixed

- *(config)* use `__` (double underscore) as separator for env vars ([#397](https://github.com/graphql-hive/router/pull/397))

## [0.0.2](https://github.com/graphql-hive/router/compare/hive-router-config-v0.0.1...hive-router-config-v0.0.2) - 2025-09-02

### Fixed

- *(hive-router)* fix docker image issues  ([#394](https://github.com/graphql-hive/router/pull/394))
## 0.1.2 (2026-06-16)

### Features

#### Support VRL expression for subscription callback `public_url`

The `subscriptions.callback.public_url` config field now accepts either a static URL string or a VRL expression, in addition to the previously supported plain URL value.

This is useful in horizontally scaled deployments where the public callback URL is not known at build time and must be resolved at runtime - for example, from an environment variable set by the orchestrator per instance.

### Configuration

```yaml
subscriptions:
  enabled: true
  callback:
    # static URL (existing behavior, unchanged)
    public_url: "https://my-router.example.com/callback"
    subgraphs:
      - reviews
```

```yaml
subscriptions:
  enabled: true
  callback:
    # VRL expression - resolved at runtime
    public_url:
      expression: 'env("ROUTER_CALLBACK_PUBLIC_URL")'
    subgraphs:
      - reviews
```

## 0.1.1 (2026-06-15)

### Features

#### Add at-least-once sampling for Usage Reporting

Hive Router now supports at-least-once sampling for Usage Reporting.

This feature is useful when you want to keep a low sampling rate, but still make sure all operations are visible in Hive at least once.

The first request for each unique key is always reported. Later requests for the same key follow the configured sampling `rate`.

Example configuration:

```yaml
telemetry:
  hive:
    usage_reporting:
      enabled: true
      sampling:
        rate: "10%" # 10% of operations will be reported
        at_least_once:
          key: # the combination of operation's name and body makes the request unique
            - operation_name
            - operation_body
          max_distinct_keys: 12000 # how many keys to track and hold in memory
```

Keys are tracked in memory, up to `max_distinct_keys` (default: `100_000`). Every key takes approximately 16 bytes of memory.

#### Demand Control with `@cost` and `@listSize` directives

Add support for the [Demand Control specification](https://ibm.github.io/graphql-specs/cost-spec.html), allowing operators to limit the cost of incoming GraphQL operations using the `@cost` and `@listSize` directives.

The router now calculates the cost of incoming operations based on directive-driven type, field, and argument costs (with list-size estimation) and can reject operations that exceed a configured maximum. Both static (request) and actual (response) cost can be measured, and the behavior is configurable via the new `demand_control` section in the router configuration.

Telemetry is included: new metrics under `demand_control_metrics` and additional span attributes expose estimated/actual cost and rejection reasons for observability.

[Documentation for the feature is available here](https://the-guild.dev/graphql/hive/docs/router/security/demand-control)

#### Move `sample_rate` into `sampling.rate`

**Breaking change** The sampling configuration of Usage Reporting has been reorganized.

```diff
telemetry:
  hive:
    usage_reporting:
-      sample_rate: "10%"
+      sampling:
+        rate: "10%"
```


The old top-level `sample_rate` field has been replaced by `sampling.rate`.

## 0.1.0 (2026-06-03)

### Breaking Changes

#### BREAKING: `override_subgraph_urls.subgraphs` and global `all`

In `override_subgraph_urls` the per-subgraph overrides now live under a `subgraphs` field, alongside a new optional `all` override.

```yaml
## Before
override_subgraph_urls:
  accounts:
    url: "https://accounts.example.com/graphql"
  products:
    url:
      expression: |
        if .request.headers."x-region" == "us-east" {
          "https://products-us-east.example.com/graphql"
        } else {
          .default
        }

## After
override_subgraph_urls:
  subgraphs:
    accounts:
      url: "https://accounts.example.com/graphql"
    products:
      url:
        expression: |
          if .request.headers."x-region" == "us-east" {
            "https://products-us-east.example.com/graphql"
          } else {
            .default
          }
  all:
    url:
      expression: |
        if .subgraph.name == "reviews" {
          "https://reviews.example.com/graphql"
        } else {
          .default
        }
```

A single override under `override_subgraph_urls.all.url` is now applied to every subgraph that does not have its own per-subgraph override. This is useful when the override logic is the same for all subgraphs or depends on the current subgraph name.

The expression has access to:

- `.request`: the incoming HTTP request
- `.default`: the original subgraph URL from the supergraph SDL
- `.subgraph.name`: the name of the subgraph the URL is being resolved for

Per-subgraph entries under `subgraphs.<name>` always take precedence over `all`.

### Features

#### Path parameters from `http.graphql_endpoint`

Any path parameters captured from the configured pattern are now exposed:

- in expressions as `.request.path_params`
- in plugins through the existing `RouterHttpRequest.match_info`

```yaml
http:
  graphql_endpoint: /{tenant}/graphql
override_subgraph_urls:
  all:
    url:
      expression: |
        tenant = string!(.request.path_params.tenant)
        replace(string!(.default), "/api/", "/api/" + tenant + "/")
```

A request to `/acme/graphql` resolves `tenant` to `"acme"` before the expression runs.

### Fixes

#### Forward operation name to subgraphs

Added the `traffic_shaping.all.forward_operation_name` and `traffic_shaping.subgraphs.<name>.forward_operation_name` options. The option defaults to `false`.

The operation name is injected (opt-in) into the query document and the `operationName` JSON field, formatted as `<client_operation_name>__<fetch_step_id>`, when sending requests to subgraphs.

Global opt-in:

```yaml
traffic_shaping:
  all:
    forward_operation_name: true
```

Per-subgraph opt-in:

```yaml
traffic_shaping:
  subgraphs:
    products:
      # Overrides global setting for this subgraph
      forward_operation_name: true
```

#### Improve parsing of Router configuration

Improve error messages when parsing Router configuration, in cases where `SingleOrMultiple<T>` is used and parsing of `T` fails. 

The error is now visible to the user, instead of being swallowed and reported as a generic error.

## 0.0.37 (2026-05-27)

### Fixes

#### Add tracing sampling rate environment override

The tracing sampling rate can now be overridden without editing the router config file:

```shell
TELEMETRY_TRACING_SAMPLING_RATE=0.1
```

This sets the same value as the following YAML configuration:

```yaml
telemetry:
  tracing:
    collect:
      sampling: 0.1
```

## 0.0.36 (2026-05-26)

### Features

#### External storage support (e.g S3)

[documentation](http://the-guild.dev/graphql/hive/docs/router/configuration/storages)

This release introduces a new top-level `storages` configuration and the first storage backend, s3, so the router can load external artifacts from object storage.

With this change, both the `supergraph` source and `persisted_documents` manifest can be resolved from a configured storage by reference. It also adds optional polling support so the router can reload updated content from storage without restarting.

Start by configuring the storage in your router config:

```yaml
storages: 
  my-s3: # this is the storage id 
    type: s3
    bucket: my-bucket
    region: eu-west-1
    # .. additional S3 configurations 
```

Then, you can use the storage id in your `supergraph` source:

```yaml
supergraph:
  source: storage
  storage_id: my-s3
  location: supergraphs/current.graphql
  poll_interval: 30s
```

Or, you can use the storage id in your `persisted_documents` manifest:

```yaml
persisted_documents:
  enabled: true
  require_id: true
  storage:
    type: storage
    storage_id: my-s3
    location: persisted/manifest.json
    poll_interval: 30s
```

## 0.0.35 (2026-05-17)

### Fixes

#### Implement Circuit Breaker for Subgraph Requests

This change introduces a circuit breaker mechanism for subgraph requests in the Hive Router. The circuit breaker will monitor the success and failure rates of requests to each subgraph and will prevent future requests if the failure rate exceeds a certain threshold. When the circuit breaker is opened, subsequent requests to that subgraph will fail immediately without attempting to send the request.

This implementation helps improve the resilience and stability of the Hive Router when dealing with unreliable subgraphs.

## 0.0.34 (2026-05-13)

### Features

#### Allow overriding number of HTTP server workers

Adds a new `http.workers` configuration option (and `ROUTER_HTTP_WORKERS` environment variable) to control the number of HTTP server worker threads.

By default, the router spawns one worker per physical CPU core. In containerized environments such as Kubernetes the number of physical cores reported by the OS is often higher than the CPU limit assigned to the container, which leads to oversubscribed worker threads. Set `http.workers` (or `ROUTER_HTTP_WORKERS`) to match the container's CPU limit to avoid this.

```yaml
http:
  workers: 4
```

#### Add `cors.preflight_response_headers` to attach headers to CORS preflight (OPTIONS) responses

Adds a new optional `preflight_response_headers` map to the `cors` configuration block, and to each entry under `cors.policies`. The map allows attaching arbitrary headers (e.g. `Cache-Control`, `Server-Timing`, custom `X-*` headers) to CORS preflight (OPTIONS) responses.

This is useful because the `headers` configuration block does not affect preflight responses (they are returned early by the CORS layer), so there was previously no way to control headers like `Cache-Control` for `OPTIONS` requests.

Example:

```yaml
cors:
  enabled: true
  allow_any_origin: true
  max_age: 86400
  preflight_response_headers:
    Cache-Control: "public, max-age=86400"
```

## 0.0.33 (2026-05-05)

### Features

#### Coprocessors

Introduces Coprocessors as language agnostic way to extend Hive Router.

**Supports coprocessor stages:**
- `router.request`
- `router.response`
- `graphql.request`
- `graphql.analysis`
- `graphql.response`

**Stage capabilities:**
- include selected request/response fields in stage payloads (headers, body, context, and optional SDL depending on stage config)
- mutate request body/headers/context for downstream pipeline execution
- short-circuit and return an immediate HTTP response from a stage

**Transport and endpoint support:**
- `http://` and `unix://` (unix socket domain) endpoints
- http/1, http/2 and h2c protocols

**Error handling:**
- coprocessor failures map to server-side failures (500)
- client-facing GraphQL errors are masked as Internal server error
- structured error codes are preserved in GraphQL extensions.code
- detailed coprocessor failure reasons remain in server logs/telemetry only

**Adds coprocessor metrics:**
- hive.router.coprocessor.requests_total
- hive.router.coprocessor.duration
- hive.router.coprocessor.errors_total

## 0.0.32 (2026-04-27)

### Fixes

#### HTTP/2 Cleartext (h2c) Support for Subgraph Connections

Adds support for HTTP/2 cleartext (h2c) connections between the router and subgraphs via the new `allow_only_http2` configuration flag. When enabled, the router uses HTTP/2 prior knowledge to communicate with subgraphs over plain HTTP without TLS.

This is useful in environments where subgraphs support HTTP/2 but TLS is not required, such as service meshes, internal networks, or sidecar proxies.

### Configuration

The flag can be set globally for all subgraphs or per-subgraph. Per-subgraph settings override the global default.

#### Global (all subgraphs)

```yaml
traffic_shaping:
  all:
    allow_only_http2: true
```

#### Per-subgraph

```yaml
traffic_shaping:
  subgraphs:
    accounts:
      allow_only_http2: true
```

The default value is `false`, preserving the existing behavior of using HTTP/1.1 for plain HTTP connections and negotiating HTTP/2 via ALPN for TLS connections.

## 0.0.31 (2026-04-20)

### Features

#### Persisted Documents

Introduces persisted documents support in Hive Router with configurable extraction and storage backends.

Supports extracting persisted document IDs from:
- `documentId` in request body (default)
- `documentId` in URL query params (default)
- Apollo-style `extensions.persistedQuery.sha256Hash` (default)
- custom `json_path` (for example `doc_id` or `extensions.anything.id`)
- custom `url_query_param` (for example `?doc_id=123`)
- custom `url_path_param` (for example `/graphql/:id`)

Order is configurable and evaluated top-to-bottom.

Supports persisted document resolution from:
- file manifests (Apollo and Relay KV styles)
- Hive CDN (via `hive-console-sdk`)

File storage includes watch mode by default (with 150ms debounce) to reload manifests after file changes.
Hive storage validates document ID syntax before generating CDN paths to avoid silent invalid-path behavior.

Adds persisted-documents metrics:

- `hive.router.persisted_documents.extract.missing_id_total`
- `hive.router.persisted_documents.storage.failures_total`

These help track migration progress and resolution failures in production

#### TLS Support

Adds TLS support to Hive Router for both client and subgraph connections, including mutual TLS (mTLS) authentication. This allows secure communication between clients, the router, and subgraphs by encrypting data in transit and optionally verifying identities.

#### TLS Directions

TLS Support has implementations for the following 4 directions:

##### Router -> Client - Regular TLS
Router has an `identity` (`cert`, `key`), and client has `cert`, then Client validates the router's `identity`

##### Client -> Router - mTLS
Router has the `cert`, client has the `identity`, mTLS/Client Auth then the router validates the client's `identity`

##### Subgraph -> Router - Regular TLS
Subgraph has the `identity` (`cert`, `key`), and router has `cert`, then Router validates the subgraph's `identity`.

##### Router -> Subgraph - mTLS
Subgraph has the `cert`, router(which is the client this time) has the `identity`, then subgraph validates the router's `identity`.

#### TLS Directions Diagram

```mermaid
flowchart LR
    Client["Client"]
    Router["Router"]
    Subgraph["Subgraph"]

    %% Router -> Client: Regular TLS
    Router -- "TLS\n(cert_file + key_file)" --> Client
    Client -. "validates router identity\n(cert_file)" .-> Router

    %% Client -> Router: mTLS / Client Auth
    Client -- "mTLS\n(client identity)" --> Router
    Router -. "validates client identity\n(client_auth.cert_file)" .-> Client

    %% Subgraph -> Router: Regular TLS
    Subgraph -- "TLS\n(cert_file)" --> Router
    Router -. "validates subgraph identity\n(all/subgraphs.cert_file)" .-> Subgraph

    %% Router -> Subgraph: mTLS
    Router -- "mTLS\n(client_auth.cert_file + key_file)" --> Subgraph
    Subgraph -. "validates router identity\n(cert_file)" .-> Router
```

#### Configuration Structure
```yaml
traffic_shaping:
  router:
    key_file:          # Router server private key
    cert_file:         # Router server certificate(s)
    client_auth:       # mTLS: Client -> Router
       cert_file:      # Trusted client CA certificate(s)
  all:                 # Default TLS for all subgraph connections
    cert_file:         # Trusted subgraph CA certificate(s)
    client_auth:       # mTLS: Router -> Subgraph
       cert_file:      # Router client certificate(s)
       key_file:       # Router client private key
  subgraphs:
    SUBGRAPH_NAME:     # Per-subgraph TLS override
      cert_file:       # Trusted subgraph CA certificate(s)
      client_auth:     # mTLS: Router -> Subgraph
         cert_file:    # Router client certificate(s)
         key_file:     # Router client private key
```

## 0.0.30 (2026-04-15)

### Features

#### Federated GraphQL Subscriptions

Hive Router now supports federated GraphQL subscriptions with full protocol coverage across [SSE](https://the-guild.dev/graphql/hive/docs/router/subscriptions/sse), [WebSockets](https://the-guild.dev/graphql/hive/docs/router/subscriptions/websockets), [Multipart HTTP](https://the-guild.dev/graphql/hive/docs/router/subscriptions/multipart-http), [Incremental Delivery](https://the-guild.dev/graphql/hive/docs/router/subscriptions/incremental-delivery), and [HTTP Callback](https://the-guild.dev/graphql/hive/docs/router/subscriptions/http-callback) - for both client-to-router and router-to-subgraph communication. Subscription events spanning multiple subgraphs are resolved automatically: when a subscription field lives in one subgraph but the response includes entity fields owned by others, the router fetches those on every event with no extra configuration.

- [Read the product update](https://the-guild.dev/graphql/hive/product-updates/2026-04-14-hive-router-subscriptions)
- [Subscriptions overview](https://the-guild.dev/graphql/hive/docs/router/subscriptions)
- [Server-Sent Events](https://the-guild.dev/graphql/hive/docs/router/subscriptions/sse)
- [Incremental Delivery over HTTP](https://the-guild.dev/graphql/hive/docs/router/subscriptions/incremental-delivery)
- [Multipart HTTP](https://the-guild.dev/graphql/hive/docs/router/subscriptions/multipart-http)
- [WebSockets](https://the-guild.dev/graphql/hive/docs/router/subscriptions/websockets)
- [HTTP Callback](https://the-guild.dev/graphql/hive/docs/router/subscriptions/http-callback)

## 0.0.29 (2026-04-15)

### Features

#### Federated GraphQL Subscriptions

Hive Router now supports federated GraphQL subscriptions with full protocol coverage across [SSE](https://the-guild.dev/graphql/hive/docs/router/subscriptions/sse), [WebSockets](https://the-guild.dev/graphql/hive/docs/router/subscriptions/websockets), [Multipart HTTP](https://the-guild.dev/graphql/hive/docs/router/subscriptions/multipart-http), [Incremental Delivery](https://the-guild.dev/graphql/hive/docs/router/subscriptions/incremental-delivery), and [HTTP Callback](https://the-guild.dev/graphql/hive/docs/router/subscriptions/http-callback) - for both client-to-router and router-to-subgraph communication. Subscription events spanning multiple subgraphs are resolved automatically: when a subscription field lives in one subgraph but the response includes entity fields owned by others, the router fetches those on every event with no extra configuration.

- [Read the product update](https://the-guild.dev/graphql/hive/product-updates/2026-04-14-hive-router-subscriptions)
- [Subscriptions overview](https://the-guild.dev/graphql/hive/docs/router/subscriptions)
- [Server-Sent Events](https://the-guild.dev/graphql/hive/docs/router/subscriptions/sse)
- [Incremental Delivery over HTTP](https://the-guild.dev/graphql/hive/docs/router/subscriptions/incremental-delivery)
- [Multipart HTTP](https://the-guild.dev/graphql/hive/docs/router/subscriptions/multipart-http)
- [WebSockets](https://the-guild.dev/graphql/hive/docs/router/subscriptions/websockets)
- [HTTP Callback](https://the-guild.dev/graphql/hive/docs/router/subscriptions/http-callback)

## 0.0.28 (2026-04-13)

### Fixes

- Version bump to fix release issues

#### Replace GraphiQL with Hive Laboratory

The Laboratory is Hive's powerful GraphQL playground that provides a comprehensive environment for exploring, testing, and experimenting with your GraphQL APIs. Whether you're developing new queries, debugging issues, or sharing operations with your team, the Laboratory offers all the tools you need.

Read more about Hive Laboratory in [the introduction blog post](https://the-guild.dev/graphql/hive/product-updates/2026-01-28-new-laboratory) or [the documentation](https://the-guild.dev/graphql/hive/docs/new-laboratory).

#### Breaking Changes:

The top-level config option has been renamed.

```diff
- graphiql:
+ laboratory:
    enabled: true
```

So was the environment variable override.

```diff
- GRAPHIQL_ENABLED=true
+ LABORATORY_ENABLED=true
```

## 0.0.27 (2026-04-12)

### Fixes

#### Replace GraphiQL with Hive Laboratory

The Laboratory is Hive's powerful GraphQL playground that provides a comprehensive environment for exploring, testing, and experimenting with your GraphQL APIs. Whether you're developing new queries, debugging issues, or sharing operations with your team, the Laboratory offers all the tools you need.

Read more about Hive Laboratory in [the introduction blog post](https://the-guild.dev/graphql/hive/product-updates/2026-01-28-new-laboratory) or [the documentation](https://the-guild.dev/graphql/hive/docs/new-laboratory).

#### Breaking Changes:

The top-level config option has been renamed.

```diff
- graphiql:
+ laboratory:
    enabled: true
```

So was the environment variable override.

```diff
- GRAPHIQL_ENABLED=true
+ LABORATORY_ENABLED=true
```

## 0.0.26 (2026-03-26)

### Features

#### Add router-level in-flight request deduplication for GraphQL queries

The router now supports deduplicating identical incoming GraphQL query requests while they are in flight, so concurrent duplicates can share one execution result.

### Configuration

A new router traffic-shaping section is available:

- `traffic_shaping.router.dedupe.enabled` (default: `false`)
- `traffic_shaping.router.dedupe.headers` as `all`, `none`, or `{ include: [...] }` (default: `all`)

Supported header config shapes:

```yaml
headers: all
```

```yaml
headers: none
```

```yaml
headers:
  include:
    - authorization
    - cookie
```

Header names are validated and normalized as standard HTTP header names.

### Deduplication key behavior

The router dedupe fingerprint includes:

- request method and path
- selected request headers (based on dedupe header policy)
- normalized operation hash
- GraphQL variables hash
- schema checksum
- GraphQL extensions

## 0.0.25 (2026-03-12)

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

### Fixes

#### Support multiple endpoints for Hive Console CDN source for Supergraph.

So you can pass endpoints separated by comma in the env var `HIVE_CDN_ENDPOINT`, so that if one CDN endpoint is not available, the router can fallback to the next one in the list.

```
HIVE_CDN_ENDPOINT=https://cdn.graphql-hive.com/***,https://cdn-mirror.graphql-hive.com/***
```

[Learn more about CDN mirrors](https://the-guild.dev/graphql/hive/docs/schema-registry/high-availability-cdn#cdn-mirrors)

## 0.0.24 (2026-03-05)

### Features

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

#### Dependencies Updates

- Update `rustls`, `aws-lc-rs` and `aws-lc-sys` dependencies to address `PKCS7` CVE in `aws-lc` crates.
- Update `rand` to latest version.

## 0.0.23 (2026-02-11)

### Features

#### Move `telemetry.hive.endpoint` to `telemetry.hive.tracing.endpoint`.

The endpoint is tracing-specific, but its current placement at `telemetry.hive.endpoint` suggests it applies globally to all Hive telemetry features. This becomes misleading now that usage reporting also defines its own endpoint configuration (`telemetry.hive.usage_reporting.endpoint`).

```diff
telemetry:
  hive:
-   endpoint: "<value>"
+   tracing:
+     endpoint: "<value>"
```

## 0.0.22 (2026-02-10)

### Features

#### Hive telemetry (tracing and usage reporting) is now explicitly opt-in.

Two new environment variables are available to control telemetry:
  - `HIVE_TRACING_ENABLED` controls `telemetry.hive.tracing.enabled` config value
  - `HIVE_USAGE_REPORTING_ENABLED` controls `telemetry.hive.usage_reporting.enabled` config value
  
The accepted values are `true` or `false`.

If you only set `HIVE_ACCESS_TOKEN` and `HIVE_TARGET`, usage reporting stays disabled until explicitly enabled with environment variables or configuration file.

#### New configuration option to set a timeout for the router

This update introduces a new configuration option that allows users to set a timeout for the router. This timeout will help prevent long-running requests from consuming resources indefinitely, improving the overall performance and reliability of the router. Users can now specify a timeout duration in their configuration files, and the router will automatically terminate any requests that exceed this duration.

By default, the timeout is set to 60 seconds;

```yaml
traffic_shaping:
    router:
        request_timeout: 60s # Human readable duration format (e.g., "30s", "1m", "2h")
```

### Fixes

#### Tracing with OpenTelemetry

Introducing comprehensive OpenTelemetry-based tracing to the Hive Router, providing deep visibility into the GraphQL request lifecycle and subgraph communications.

- **OpenTelemetry Integration**: Support for OTLP exporters (gRPC and HTTP) and standard propagation formats (Trace Context, Baggage, Jaeger, B3/Zipkin).
- **GraphQL-Specific Spans**: Detailed spans for every phase of the GraphQL lifecycle
- **Hive Console Tracing**: Native integration with Hive Console for trace visualization and analysis.
- **Semantic Conventions**: Support for both stable and deprecated OpenTelemetry HTTP semantic conventions to ensure compatibility with a wide range of observability tools.
- **Optimized Performance**: Tracing is designed with a "pay only for what you use" approach. Overhead is near-zero when disabled, and allocations/computations are minimized when enabled.
- **Rich Configuration**: New configuration options for telemetry exporters, batching, and resource attributes.

#### Unified Hive Telemetry Configuration

Refactored the configuration structure to unify Hive-specific telemetry (tracing and usage reporting) and centralize client identification.

- **Unified Hive Config**: Moved `usage_reporting` under `telemetry.hive.usage_reporting`. Usage reporting now shares the `token` and `target` configuration with Hive tracing, eliminating redundant settings.
- **Centralized Client Identification**: Introduced `telemetry.client_identification` to define client name and version headers once. These are now propagated to both OpenTelemetry spans and Hive usage reports.
- **Enhanced Expression Support**: Both Hive token and target ID now support VRL expressions for usage reporting, matching the existing behavior of tracing.

#### Breaking Changes:

The top-level `usage_reporting` block has been moved. 

**Before:**
```yaml
usage_reporting:
  enabled: true
  access_token: "..."
  target_id: "..."
  client_name_header: "..."
  client_version_header: "..."
```

**After:**
```yaml
telemetry:
  client_identification:
    name_header: "..."
    version_header: "..."
  hive:
    token: "..."
    target: "..."
    usage_reporting:
      enabled: true
```

## 0.0.21 (2026-02-06)

### Features

- Operation Complexity - Limit Aliases (#746)
- Operation Complexity - Limit Aliases (#749)

### Fixes

#### New Operation Complexity Option: Max Aliases

We've introduced a new configuration option, `max_aliases` that allows you to limit the number of aliases in the incoming GraphQL operations. This helps to prevent overly complex queries that could impact performance, or any potential DOS attack or heap overflow via excessive aliases.

```yaml
limits:
  max_aliases:
    n: 3  # Set the maximum number of aliases allowed in a query
```

#### New configuration flag to limit the incoming HTTP request body size in the router before parsing the request(JSON etc).

```yaml
limits:
  max_request_body_size: 2MB # Human readable size format
```

By default, this limit is set to 2MB.

## 0.0.20 (2026-01-27)

### Fixes

- Bump version to fix release and dependencies issues

#### Support multiple endpoints in Hive CDN Supergraph config

In order to support a Secondary CDN endpoint for better reliability, the Hive CDN Supergraph configuration has been updated to allow specifying either a single endpoint or multiple endpoints.
This change enables users to provide a list of CDN endpoints, enhancing the robustness of their supergraph setup.

[Learn more about it in the relevant Hive Console documentation here](https://the-guild.dev/graphql/hive/docs/schema-registry/high-availability-resilence).

```diff
supergraph:
    source: hive
-    endpoint: https://cdn-primary.example.com/supergraph
+    endpoint:
+       - https://cdn-primary.example.com/supergraph
+       - https://cdn-secondary.example.com/supergraph
```

## 0.0.19 (2026-01-26)

### Fixes

#### Better error handling for configuration loading

- In case of an invalid environment variables, do not crash with `panic` but provide a clear error message with a proper error type.
- In case of failing to get the current working directory, provide a clear error message instead of crashing with `panic`.
- In case of failing to parse the configuration file path, provide a clear error message instead of crashing with `panic`.

## 0.0.18 (2026-01-22)

### Features

#### New Query Complexity Configuration in `hive-router` and `hive-router-config`

We have introduced a new configuration module for query complexity in the Hive Router. 

This includes new validation rules to enforce maximum query depth, maximum number of directives in the incoming GraphQL operation, helping to prevent overly complex queries that could impact performance.

### Max Depth

By default, it is disabled, but you can enable and configure it in your router configuration as follows:

```yaml
limits:
  max_depth:
    n: 10  # Set the maximum allowed depth for queries
```

This configuration allows you to set a maximum depth for incoming GraphQL queries, enhancing the robustness of your API by mitigating the risk of deep-nested queries.

### Max Directives

You can also limit the number of directives in incoming GraphQL operations. This is also disabled by default. You can enable and configure it as follows:

```yaml
limits:
  max_directives:
    n: 5  # Set the maximum allowed number of directives
```

This configuration helps to prevent excessive use of directives in queries, which can lead to performance issues.

### Max Tokens

Additionally, we have introduced a new configuration option to limit the maximum number of tokens in incoming GraphQL operations. This feature is designed to prevent excessively large queries that could impact server performance.

By default, this limit is disabled. You can enable and configure it in your router configuration as follows:

```yaml
limits:
  max_tokens:
    n: 1000  # Set the maximum allowed number of tokens
```

This configuration allows you to set a maximum token count for incoming GraphQL queries, helping to ensure that queries remain manageable and do not overwhelm the server.

With these new configurations, you can better manage the complexity of incoming GraphQL queries and ensure the stability and performance of your API.

## 0.0.17 (2026-01-14)

### Fixes

#### Remove extra `target_id` validation in Router config

This change removes the extra deserialization validation for the `target_id` field in the Router configuration, because it is already done by the Hive Console SDK.

## 0.0.16 (2026-01-12)

### Fixes

#### Added an option to customize the GraphQL endpoint path

You can now customize the GraphQL endpoint path by adding the following configuration to your router configuration file:

```yaml
http:
  graphql_endpoint: /my-graphql-endpoint
```

## 0.0.15 (2025-12-08)

### Fixes

- Bump dependencies

## 0.0.14 (2025-11-28)

### Fixes

- make supergraph.{path,key,endpoint} optional (#593)

#### Improve error messages and fix environment variable support for supergraph configuration

- **Fix:** Previously, `supergraph.path` (for file source), and `supergraph.endpoint`/`supergraph.key` (for Hive CDN source) were mandatory in the configuration file. This prevented users from relying solely on environment variables (`SUPERGRAPH_FILE_PATH`, `HIVE_CDN_ENDPOINT`, `HIVE_CDN_KEY`). This has been fixed, and these fields are now optional in the configuration file if the corresponding environment variables are provided.
- **Improved Error Reporting:** If the supergraph file path or Hive CDN endpoint/key are missing from both configuration and environment variables, the error message now explicitly guides you to set the required environment variable or the corresponding configuration option.

This change ensures that misconfigurations are easier to diagnose and fix during startup.

#### Usage Reporting to Hive Console

Hive Router now supports sending usage reports to the Hive Console. This feature allows you to monitor and analyze the performance and usage of your GraphQL services directly from the Hive Console.
To enable usage reporting, you need to configure the `usage_reporting` section in your Hive Router configuration file.

[Learn more about usage reporting in the documentation.](https://the-guild.dev/graphql/hive/docs/router/configuration/usage_reporting)
```yaml
usage_reporting:
  enabled: true
  access_token: your-hive-console-access-token
```

## 0.0.13 (2025-11-28)

### Features

#### Subgraph Request Timeout Feature

Adds support for configurable subgraph request timeouts via the `traffic_shaping` configuration. The `request_timeout` option allows you to specify the maximum time the router will wait for a response from a subgraph before timing out the request. You can set a static timeout (e.g., `30s`) globally or per-subgraph, or use dynamic timeouts with VRL expressions to vary timeout values based on request characteristics. This helps protect your router from hanging requests and enables fine-grained control over how long requests to different subgraphs should be allowed to run.

#### Rename `original_url` variable to `default` in subgraph URL override expressions.

This change aligns the variable naming with other configuration expressions, such as timeout configuration.

When using expressions to override subgraph URLs, use `.default` to refer to the original URL defined in the subgraph definition.

Example:

```yaml
url:
  expression: |
    if .request.headers."x-region" == "us-east" {
      "https://products-us-east.example.com/graphql"
    } else {
      .default
    }
```

### Fixes

- support `@include` and `@skip` in initial fetch node (#591)

## 0.0.12 (2025-11-24)

### Features

#### Breaking

Removed `pool_idle_timeout_seconds` from `traffic_shaping`, instead use `pool_idle_timeout` with duration format.

```diff
traffic_shaping:
-  pool_idle_timeout_seconds: 30
+  pool_idle_timeout: 30s
```

##540 by @ardatan

## 0.0.11 (2025-11-04)

### Fixes

- Bump config crate to fix dependency issues after switching to knope
