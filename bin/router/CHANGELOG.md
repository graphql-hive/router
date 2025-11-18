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
