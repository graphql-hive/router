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
