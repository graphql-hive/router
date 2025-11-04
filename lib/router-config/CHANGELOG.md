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
## 0.0.11 (2025-11-04)

### Fixes

- Bump config crate to fix dependency issues after switching to knope
