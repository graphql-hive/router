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
