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
