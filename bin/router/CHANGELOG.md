# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.15](https://github.com/graphql-hive/router/compare/hive-router-v0.0.14...hive-router-v0.0.15) - 2025-10-27

### <!-- 0 -->New Features

- *(router)* added support for label overrides with `@override` ([#518](https://github.com/graphql-hive/router/pull/518))
- *(config)* configuration override using env vars, enable/disable graphiql via config ([#519](https://github.com/graphql-hive/router/pull/519))

### <!-- 1 -->Bug Fixes

- *(query-planner, router)* fix introspection for federation v1 supergraph ([#526](https://github.com/graphql-hive/router/pull/526))

### <!-- 2 -->Refactoring

- *(error-handling)* add context to `PlanExecutionError` ([#513](https://github.com/graphql-hive/router/pull/513))

## [0.0.13](https://github.com/graphql-hive/router/compare/hive-router-v0.0.12...hive-router-v0.0.13) - 2025-10-23

### Added

- *(router)* support `hive` as source for supergraph ([#400](https://github.com/graphql-hive/router/pull/400))

### Fixed

- *(router)* use 503 instead of 500 when router is not ready ([#512](https://github.com/graphql-hive/router/pull/512))
- *(executor)* error logging in HTTP executor ([#498](https://github.com/graphql-hive/router/pull/498))
- *(executor)* handle subgraph errors with extensions correctly ([#494](https://github.com/graphql-hive/router/pull/494))
- *(ci)* fail when audit tests failing ([#495](https://github.com/graphql-hive/router/pull/495))
- *(executor)* project scalars with object values correctly ([#492](https://github.com/graphql-hive/router/pull/492))
- *(query-planner)* inline nested fragment spreads ([#502](https://github.com/graphql-hive/router/pull/502))

### Other

- Remove mimalloc override feature and use v3 ([#497](https://github.com/graphql-hive/router/pull/497))
- Add affectedPath to GraphQLErrorExtensions ([#510](https://github.com/graphql-hive/router/pull/510))
- Handle empty responses from subgraphs and failed entity calls ([#500](https://github.com/graphql-hive/router/pull/500))
- Rename default config file to router.config ([#493](https://github.com/graphql-hive/router/pull/493))

## [0.0.12](https://github.com/graphql-hive/router/compare/hive-router-v0.0.11...hive-router-v0.0.12) - 2025-10-16

### Added

- *(router)* Subgraph endpoint overrides ([#488](https://github.com/graphql-hive/router/pull/488))
- *(router)* jwt auth ([#455](https://github.com/graphql-hive/router/pull/455))
- *(router)* CORS support ([#473](https://github.com/graphql-hive/router/pull/473))
- *(router)* CSRF prevention for browser requests ([#472](https://github.com/graphql-hive/router/pull/472))
- *(executor)* include subgraph name and code to the errors ([#477](https://github.com/graphql-hive/router/pull/477))
- *(executor)* normalize flatten errors for the final response ([#454](https://github.com/graphql-hive/router/pull/454))

### Fixed

- *(router)* fix graphiql autocompletion, and avoid serializing nulls for optional extension fields ([#485](https://github.com/graphql-hive/router/pull/485))
- *(router)* improve csrf and other configs  ([#487](https://github.com/graphql-hive/router/pull/487))
- *(router)* allow null value for nullable scalar types while validating variables ([#483](https://github.com/graphql-hive/router/pull/483))

## [0.0.11](https://github.com/graphql-hive/router/compare/hive-router-v0.0.10...hive-router-v0.0.11) - 2025-10-08

### Added

- *(router)* Advanced Header Management ([#438](https://github.com/graphql-hive/router/pull/438))

### Fixed

- *(executor)* ensure variables passed to subgraph requests ([#464](https://github.com/graphql-hive/router/pull/464))

## [0.0.10](https://github.com/graphql-hive/router/compare/hive-router-v0.0.9...hive-router-v0.0.10) - 2025-10-05

### Other

- *(deps)* update actions-rust-lang/setup-rust-toolchain digest to 1780873 ([#466](https://github.com/graphql-hive/router/pull/466))

## [0.0.9](https://github.com/graphql-hive/router/compare/hive-router-v0.0.8...hive-router-v0.0.9) - 2025-09-09

### Fixed

- *(executor)* handle fragments while resolving the introspection ([#411](https://github.com/graphql-hive/router/pull/411))

### Other

- update Cargo.lock dependencies

## [0.0.8](https://github.com/graphql-hive/router/compare/hive-router-v0.0.7...hive-router-v0.0.8) - 2025-09-04

### Fixed

- *(executor)* added support for https scheme and https connector ([#401](https://github.com/graphql-hive/router/pull/401))

## [0.0.7](https://github.com/graphql-hive/router/compare/hive-router-v0.0.6...hive-router-v0.0.7) - 2025-09-02

### Fixed

- *(config)* use `__` (double underscore) as separator for env vars ([#397](https://github.com/graphql-hive/router/pull/397))

## [0.0.6](https://github.com/graphql-hive/router/compare/hive-router-v0.0.5...hive-router-v0.0.6) - 2025-09-02

### Fixed

- *(hive-router)* fix docker image issues  ([#394](https://github.com/graphql-hive/router/pull/394))

## [0.0.5](https://github.com/graphql-hive/router/compare/hive-router-v0.0.4...hive-router-v0.0.5) - 2025-09-01

### Other

- update Cargo.lock dependencies

## [0.0.4](https://github.com/graphql-hive/router/compare/hive-router-v0.0.3...hive-router-v0.0.4) - 2025-09-01

### Other

- *(deps)* update release-plz/action action to v0.5.113 ([#389](https://github.com/graphql-hive/router/pull/389))
## 0.0.38 (2026-02-11)

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

## 0.0.37 (2026-02-10)

### Features

- request timeout (#753)

#### New configuration option to set a timeout for the router

This update introduces a new configuration option that allows users to set a timeout for the router. This timeout will help prevent long-running requests from consuming resources indefinitely, improving the overall performance and reliability of the router. Users can now specify a timeout duration in their configuration files, and the router will automatically terminate any requests that exceed this duration.

By default, the timeout is set to 60 seconds;

```yaml
traffic_shaping:
    router:
        request_timeout: 60s # Human readable duration format (e.g., "30s", "1m", "2h")
```

### Fixes

#### Hive telemetry (tracing and usage reporting) is now explicitly opt-in.

Two new environment variables are available to control telemetry:
  - `HIVE_TRACING_ENABLED` controls `telemetry.hive.tracing.enabled` config value
  - `HIVE_USAGE_REPORTING_ENABLED` controls `telemetry.hive.usage_reporting.enabled` config value
  
The accepted values are `true` or `false`.

If you only set `HIVE_ACCESS_TOKEN` and `HIVE_TARGET`, usage reporting stays disabled until explicitly enabled with environment variables or configuration file.

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

## 0.0.36 (2026-02-06)

### Features

- Operation Complexity - Limit Aliases (#746)
- Operation Complexity - Limit Aliases (#749)
- configuration to limit the HTTP request body (#729)

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

## 0.0.35 (2026-01-27)

### Features

- support multiple Hive CDN endpoints (#718)

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

#### Fix release issues and conflicts

- Re-export `graphql-tools` from `hive-console-sdk` to make it easier to depend directly on the SDK instead of an external package.

#### Fixed: 4xx client errors are now properly treated as errors and trigger endpoint failover, instead of being returned as successful responses.

This ensures the CDN fallback mechanism works correctly when endpoints return client errors like 403 Forbidden or 404 Not Found.

## 0.0.34 (2026-01-26)

### Fixes

- Render GraphiQL when accepting text/html with highest q-weight (#705)
- avoid `expect` and handle configuration errors better (#715)
- Render GraphiQL when accepting text/html with highest q-weight

#### Better pipeline error handling

- Now there is only one place that logs the pipeline errors instead of many
- Pipeline errors are now mapped automatically from the internal error types using `#[from]` from `thiserror` crate instead of `map_err` everywhere

#### Better error handling for configuration loading

- In case of an invalid environment variables, do not crash with `panic` but provide a clear error message with a proper error type.
- In case of failing to get the current working directory, provide a clear error message instead of crashing with `panic`.
- In case of failing to parse the configuration file path, provide a clear error message instead of crashing with `panic`.

## 0.0.33 (2026-01-22)

### Features

- Query Complexity: Max Depth, Max Directives, Max Tokens (#623)
- Enable/Disable Introspection with `introspection` (#655)

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

#### Refactor Router Initialization Error Handling in `hive-router`

- New `RouterInitError` enum to represent initialization errors in the Hive Router.
- `router_entrypoint` now returns `Result<(), RouterInitError>` instead of a boxed dynamic error(`Box<dyn std::error::Error>`), providing more specific error handling during router initialization.

### Fixes

- Expose query plan with option "dry-run" wont execute the query plan

#### Refactor Parse Error Handling in `graphql-tools`

Breaking;
- `ParseError(String)` is now `ParseError(InternalError<'static>)`.
- - So that the internals of the error can be better structured and more informative, such as including line and column information.
- `ParseError`s are no longer prefixed with "query parse error: " in their Display implementation.

## 0.0.32 (2026-01-16)

### Fixes

#### Add `minify_query_document` for optimized query minification

Implements `minify_query_document` to minify parsed GraphQL operations directly, avoiding the need for an intermediate `Display` step. This new approach uses `itoa` and `ryu` for efficient integer and float formatting.

By minifying the query document representation instead of the query string, we achieve performance improvements: query minification time is reduced from 4μs to 500ns, and unnecessary allocations are eliminated.

Includes benchmarks and tests to validate the performance gains and correctness of the new implementation.

#### Use native TLS instead of vendored

In this release, we've changed the TLS settings to use `native` TLS certificates provided by the OS, instead of using certificates that are bundled (`vendored`) into the router binary. 

This change provides more flexibiliy to `router` users, as you can extend and have full control over the certificates used to make subgraph requests, by extending or changing the certificates installed on your machine, or Docker container.

The `router` is using [AWS-LC](https://aws.amazon.com/security/opensource/cryptography/) as the certificate library.

### If you are using `hive-router` Crate

If you're using the `hive-router` crate as a library, the router provides the `init_rustls_crypto_provider()` function that automatically configures AWS-LC as the default cryptographic provider. You can call this function early in your application startup before initializing the router. Alternatively, you can configure your own `rustls` provider before calling router initialization. See the [`rustls` documentation](https://github.com/rustls/rustls#cryptography-providers) for instructions on setting up a custom provider.

## 0.0.31 (2026-01-15)

### Fixes

- Downgrade `reqwest` to `v0.12` to avoid runtime crash from `rustls` `CryptoProvider` introduced in reqwest `v0.13`.

## 0.0.30 (2026-01-14)

### Fixes

#### Update `reqwest`, `reqwest-retry`, and `reqwest-middleware` dependencies

This change updates the `reqwest` dependency to version `0.13.0`, `reqwest-retry` to version `0.9.0`, and `reqwest-middleware` to version `0.5.0` in the Hive Console SDK and Router packages.

#### Improved Performance for Expressions

This change introduces "lazy evaluation" for contextual information used in expressions (like dynamic timeouts).

Previously, the Router would prepare and clone data (such as request details or subgraph names) every time it performed an operation, even if that data wasn't actually needed.
Now, this work is only performed "on-demand" - for example, only if an expression is actually being executed.
This reduces unnecessary CPU usage and memory allocations during the hot path of request execution.

#### Moves `graphql-tools` to router repository

This change moves the `graphql-tools` package to the Hive Router repository.

## Own GraphQL Parser

This change also introduces our own GraphQL parser (copy of `graphql_parser`), which is now used across all packages in the Hive Router monorepo. This allows us to have better control over parsing and potentially optimize it for our specific use cases.

#### Moves hive-console-sdk to router repository

This change moves the `hive-console-sdk` package to the Hive Router repository.

#### Remove extra `target_id` validation in Router config

This change removes the extra deserialization validation for the `target_id` field in the Router configuration, because it is already done by the Hive Console SDK.

## 0.0.29 (2026-01-12)

### Fixes

#### Bump hive-router-config version

Somehow the `hive-router-internal` crate was published with an older version of the `hive-router-config` dependency.

## 0.0.28 (2026-01-12)

### Features

- allow to customize gql endpoint (#649)

### Fixes

#### Added an option to customize the GraphQL endpoint path

You can now customize the GraphQL endpoint path by adding the following configuration to your router configuration file:

```yaml
http:
  graphql_endpoint: /my-graphql-endpoint
```

#### Improve JSON response serialization

This PR significantly improves JSON response serialization (response projection) performance (50% faster) by replacing the existing character-by-character string escaping logic with a SIMD-accelerated implementation adapted from [sonic-rs](https://github.com/cloudwego/sonic-rs).

#### Fixed response projection for fields on different concrete types of interfaces and unions.

When a query included fragments on an abstract type (interface or union) that selected fields with the same name but different return types, the projection would incorrectly use a single, merged plan for all types. This caused projection to fail when processing responses where different concrete types had different field implementations.

For example, with `... on A { children { id } }` and `... on B { children { id } }` where `A.children` returns `[AChild]` and `B.children` returns `[BChild]`, the projection would fail to correctly distinguish between the types and return incomplete or incorrect data.

The fix introduces type-aware plan merging, which preserves the context of which concrete types a field came from. During response projection, the type is now determined dynamically for each object, ensuring the correct field type is used.

In addition, a refactor of the response projection logic was performed to improve performance.

## 0.0.27 (2026-01-07)

### Features

#### Make JWK algorithm optional

Make the JWK algorithm optional as it is defined as such in the RFC. To handle a missing algorithm, we fall back to reading the algorithm from the user JWT. To protect against forged tokens, we add a validation that the algorithm in the token is part of the `allowed_algorithms`. Since `JwkMissingAlgorithm` is not longer an error, the field is removed.

### Fixes

#### Internal refactoring of JWT handling

Passing mutable request reference around was the unnecessary use of `req.extensions` to pass `JwtContext`. 

Instead, we can directly pass `JwtContext` as-is instead of using `req.extensions`.

## 0.0.26 (2025-12-12)

### Features

#### Support environment variables in expressions

We have added support for using environment variables in expressions within the Hive Router configuration.

Example usage:
```
headers:
  all:
    response:
      - insert:
          name: "x-powered-by"
          expression: env("SERVICE_NAME", "default-value")
```

### Fixes

- bump hive-console-sdk (#617)
- Bump Hive Console SDK to fix the bug where reports are not being sent when client name is provided without a version
- Bump `vrl` dependency to `0.29.0`

## 0.0.25 (2025-12-11)

### Fixes

- chore: Enable publishing of internal crate

## 0.0.24 (2025-12-11)

### Fixes

- strip `@join__directive` and `join__DirectiveArguments` for the public consumer schema (#606)
- Strip `@join__directive` and `join__DirectiveArguments` internal types while creating the consumer/public schema

#### Extract expressions to hive-router-internal crate

The `expressions` module has been extracted from `hive-router-executor` into the `hive-router-internal` crate. This refactoring centralizes expressions handling, making it available to other parts of the project without depending on the executor.

It re-exports the `vrl` crate, ensuring that all consumer crates use the same version and types of VRL.

#### Prevent planner failure when combining conditional directives and interfaces

Fixed a bug where the query planner failed to handle the combination of conditional directives (`@include`/`@skip`) and the automatic `__typename` injection required for abstract types.

## 0.0.23 (2025-12-08)

### Fixes

- Bump dependencies

## 0.0.22 (2025-11-28)

### Features

- Hive Console Usage Reporting (#499)

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

## 0.0.21 (2025-11-28)

### Features

- Subgraph Timeout Configuration (#541)

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
- Fixed an issue where `@skip` and `@include` directives were incorrectly removed from the initial Fetch of the Query Plan.

## 0.0.20 (2025-11-24)

### Features

- support authenticated and requiresScopes directives (#538)

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

#### Breaking

Removed `pool_idle_timeout_seconds` from `traffic_shaping`, instead use `pool_idle_timeout` with duration format.

```diff
traffic_shaping:
-  pool_idle_timeout_seconds: 30
+  pool_idle_timeout: 30s
```

##540 by @ardatan

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

## 0.0.19 (2025-11-19)

### Features

#### Use `hive-console-sdk` to load supergraph from Hive CDN

**Breaking Changes**

The configuration for the `hive` source has been updated to offer more specific timeout controls and support for self-signed certificates. The `timeout` field has been renamed.

```diff
supergraph:
  source: hive
  endpoint: "https://cdn.graphql-hive.com/supergraph"
  key: "YOUR_CDN_KEY"
- timeout: 30s
+ request_timeout: 30s          # `request_timeout` replaces `timeout`
+ connect_timeout: 10s          # new option to control `connect` phase
+ accept_invalid_certs: false   # new option to allow accepting invalid TLS certificates
```

## 0.0.18 (2025-11-18)

### Features

#### JWT claims caching for improved performance

**Performance improvement:** JWT token claims are now cached for up to 5 seconds, reducing the overhead of repeated decoding and verification operations. This optimization increases throughput by approximately 75% in typical workloads.

**What's changed:**
- Decoded JWT payloads are cached with a 5-second time-to-live (TTL), which respects token expiration times
- The cache automatically invalidates based on the token's `exp` claim, ensuring security is maintained

**How it affects you:**
If you're running Hive Router, you'll see significant performance improvements out of the box with no configuration needed. The 5-second cache provides an optimal balance between performance gains and cache freshness without requiring manual tuning.

## 0.0.17 (2025-11-04)

### Fixes

- Trigger release pipeline to fix issues with 0.0.16 release

## 0.0.16 (2025-11-04)

### Fixes

- Internal refactor that prevents the need to create some structs (`ClientRequestDetails`) twice. This change also eliminates the need to have clones

#### Improve the implementation of jwt plugin and expose it to expressions.

The following properties are available in the request object exposed to VRL expressions:
- `request.jwt` will always be an object
- `request.jwt.authenticated` with value of true or false
- `request.jwt.prefix` can either be a string or null (if prefix is not used)
- `request.jwt.token` can be string (when authenticated=true) or null (when authenticated=false)
- `request.jwt.claims` will always be an array (either empty or with values), containing the full JWT token claims payload.
- `request.jwt.scopes` will always be an array (either empty or with values), containing the scopes extracted from the claims

Here are examples on how to use the JWT properties in a VRL expression:

```yaml
## Passes the user-id held in `.sub` claims of the token to the subgraph, or EMPTY
headers:
  all:
    request:
      - insert:
          name: X-User-ID
          expression: |
            if .request.jwt.authenticated == true {
              .request.jwt.claims.sub
            } else {
              "EMPTY"
            }
```

```yaml
## Passes a custom header based on the status of the authentication and the status of the JWT scopes
headers:
 subgraphs:
    accounts:
      request:
        - insert:
            name: X-Can-Read
            expression: |
              if .request.jwt.authenticated == true && includes!(.request.jwt.scopes, "read:accounts") {
                "Yes"
              } else {
                "No"
              }
```
