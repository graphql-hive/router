# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [6.0.0](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v5.0.0...hive-router-plan-executor-v6.0.0) - 2025-10-27

### <!-- 0 -->New Features

- *(router)* added support for label overrides with `@override` ([#518](https://github.com/graphql-hive/router/pull/518))

### <!-- 2 -->Refactoring

- *(error-handling)* add context to `PlanExecutionError` ([#513](https://github.com/graphql-hive/router/pull/513))

## [5.0.0](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v4.0.0...hive-router-plan-executor-v5.0.0) - 2025-10-23

### Added

- *(router)* support `hive` as source for supergraph ([#400](https://github.com/graphql-hive/router/pull/400))

### Fixed

- *(executor)* handle subgraph errors with extensions correctly ([#494](https://github.com/graphql-hive/router/pull/494))
- *(executor)* error logging in HTTP executor ([#498](https://github.com/graphql-hive/router/pull/498))
- *(ci)* fail when audit tests failing ([#495](https://github.com/graphql-hive/router/pull/495))
- *(executor)* project scalars with object values correctly ([#492](https://github.com/graphql-hive/router/pull/492))

### Other

- Add affectedPath to GraphQLErrorExtensions ([#510](https://github.com/graphql-hive/router/pull/510))
- Handle empty responses from subgraphs and failed entity calls ([#500](https://github.com/graphql-hive/router/pull/500))

## [4.0.0](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v3.0.0...hive-router-plan-executor-v4.0.0) - 2025-10-16

### Added

- *(router)* Subgraph endpoint overrides ([#488](https://github.com/graphql-hive/router/pull/488))
- *(router)* jwt auth ([#455](https://github.com/graphql-hive/router/pull/455))
- *(executor)* include subgraph name and code to the errors ([#477](https://github.com/graphql-hive/router/pull/477))
- *(executor)* normalize flatten errors for the final response ([#454](https://github.com/graphql-hive/router/pull/454))

### Fixed

- *(router)* allow null value for nullable scalar types while validating variables ([#483](https://github.com/graphql-hive/router/pull/483))
- *(router)* fix graphiql autocompletion, and avoid serializing nulls for optional extension fields ([#485](https://github.com/graphql-hive/router/pull/485))

## [3.0.0](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v2.0.0...hive-router-plan-executor-v3.0.0) - 2025-10-08

### Added

- *(router)* Advanced Header Management ([#438](https://github.com/graphql-hive/router/pull/438))

### Fixed

- *(executor)* ensure variables passed to subgraph requests ([#464](https://github.com/graphql-hive/router/pull/464))

## [2.0.0](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v1.0.4...hive-router-plan-executor-v2.0.0) - 2025-10-05

### Other

- *(deps)* update actions-rust-lang/setup-rust-toolchain digest to 1780873 ([#466](https://github.com/graphql-hive/router/pull/466))

## [1.0.4](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v1.0.3...hive-router-plan-executor-v1.0.4) - 2025-09-09

### Fixed

- *(executor)* handle fragments while resolving the introspection ([#411](https://github.com/graphql-hive/router/pull/411))

## [1.0.3](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v1.0.2...hive-router-plan-executor-v1.0.3) - 2025-09-04

### Fixed

- *(executor)* added support for https scheme and https connector ([#401](https://github.com/graphql-hive/router/pull/401))

## [1.0.2](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v1.0.1...hive-router-plan-executor-v1.0.2) - 2025-09-02

### Fixed

- *(config)* use `__` (double underscore) as separator for env vars ([#397](https://github.com/graphql-hive/router/pull/397))

## [1.0.1](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v1.0.0...hive-router-plan-executor-v1.0.1) - 2025-09-02

### Other

- updated the following local packages: hive-router-config

## [1.0.0](https://github.com/graphql-hive/router/compare/hive-router-plan-executor-v0.0.1...hive-router-plan-executor-v1.0.0) - 2025-09-01

### Other

- *(deps)* update release-plz/action action to v0.5.113 ([#389](https://github.com/graphql-hive/router/pull/389))
## 6.3.4 (2026-01-16)

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

## 6.3.3 (2026-01-14)

### Fixes

#### Improved Performance for Expressions

This change introduces "lazy evaluation" for contextual information used in expressions (like dynamic timeouts).

Previously, the Router would prepare and clone data (such as request details or subgraph names) every time it performed an operation, even if that data wasn't actually needed.
Now, this work is only performed "on-demand" - for example, only if an expression is actually being executed.
This reduces unnecessary CPU usage and memory allocations during the hot path of request execution.

#### Moves `graphql-tools` to router repository

This change moves the `graphql-tools` package to the Hive Router repository.

## Own GraphQL Parser

This change also introduces our own GraphQL parser (copy of `graphql_parser`), which is now used across all packages in the Hive Router monorepo. This allows us to have better control over parsing and potentially optimize it for our specific use cases.

## 6.3.2 (2026-01-12)

### Fixes

#### Bump hive-router-config version

Somehow the `hive-router-internal` crate was published with an older version of the `hive-router-config` dependency.

## 6.3.1 (2026-01-12)

### Fixes

#### Improve JSON response serialization

This PR significantly improves JSON response serialization (response projection) performance (50% faster) by replacing the existing character-by-character string escaping logic with a SIMD-accelerated implementation adapted from [sonic-rs](https://github.com/cloudwego/sonic-rs).

#### Fixed response projection for fields on different concrete types of interfaces and unions.

When a query included fragments on an abstract type (interface or union) that selected fields with the same name but different return types, the projection would incorrectly use a single, merged plan for all types. This caused projection to fail when processing responses where different concrete types had different field implementations.

For example, with `... on A { children { id } }` and `... on B { children { id } }` where `A.children` returns `[AChild]` and `B.children` returns `[BChild]`, the projection would fail to correctly distinguish between the types and return incomplete or incorrect data.

The fix introduces type-aware plan merging, which preserves the context of which concrete types a field came from. During response projection, the type is now determined dynamically for each object, ensuring the correct field type is used.

In addition, a refactor of the response projection logic was performed to improve performance.

## 6.3.0 (2025-12-12)

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

- Bump `vrl` dependency to `0.29.0`

## 6.2.4 (2025-12-11)

### Fixes

- chore: Enable publishing of internal crate

## 6.2.3 (2025-12-11)

### Fixes

- strip `@join__directive` and `join__DirectiveArguments` for the public consumer schema (#606)
- Strip `@join__directive` and `join__DirectiveArguments` internal types while creating the consumer/public schema

#### Extract expressions to hive-router-internal crate

The `expressions` module has been extracted from `hive-router-executor` into the `hive-router-internal` crate. This refactoring centralizes expressions handling, making it available to other parts of the project without depending on the executor.

It re-exports the `vrl` crate, ensuring that all consumer crates use the same version and types of VRL.

## 6.2.2 (2025-12-08)

### Fixes

- Bump dependencies

## 6.2.1 (2025-11-28)

### Fixes

- make supergraph.{path,key,endpoint} optional (#593)

#### Usage Reporting to Hive Console

Hive Router now supports sending usage reports to the Hive Console. This feature allows you to monitor and analyze the performance and usage of your GraphQL services directly from the Hive Console.
To enable usage reporting, you need to configure the `usage_reporting` section in your Hive Router configuration file.

[Learn more about usage reporting in the documentation.](https://the-guild.dev/graphql/hive/docs/router/configuration/usage_reporting)
```yaml
usage_reporting:
  enabled: true
  access_token: your-hive-console-access-token
```

## 6.2.0 (2025-11-28)

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

## 6.1.0 (2025-11-24)

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

#### Breaking

Removed `pool_idle_timeout_seconds` from `traffic_shaping`, instead use `pool_idle_timeout` with duration format.

```diff
traffic_shaping:
-  pool_idle_timeout_seconds: 30
+  pool_idle_timeout: 30s
```

##540 by @ardatan

## 6.0.1 (2025-11-04)

### Fixes

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
