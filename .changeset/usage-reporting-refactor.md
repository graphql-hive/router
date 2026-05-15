---
hive-router: minor
hive-router-config: minor
hive-console-sdk: minor
hive-apollo-router-plugin: minor
hive-router-plan-executor: patch
hive-router-internal: patch
---

# Usage reporting refactor

**Breaking change.** Usage reporting (`telemetry.hive.usage_reporting`) has been
refactored end-to-end:

- the flat `sample_rate` field is replaced with a tagged `sampler` config (see
  below);
- the runtime sampling/exclusion logic and all the configuration types are now
  defined once in `hive-console-sdk` and consumed by both `hive-router` and the
  Apollo Router plugin;
- a few cross-cutting primitives (`RetryPolicyConfig`, `CircuitBreakerConfig`,
  `TargetId`) are promoted to shared SDK types so they have one shape and one
  JSON schema everywhere they appear.

## New `sampler` config

The new `sampler` field supports two strategies today and can be extended
without further breaking changes:

- `fixed`: probabilistic sampling at a fixed rate (the previous behavior).
- `at_least_once`: guarantees the first occurrence per `key` is reported,
  then applies `rate` to every subsequent occurrence with the same key. The
  key defaults to the GraphQL operation name and can also be derived from a
  VRL expression for arbitrary grouping. The set of "already-seen" keys is
  bounded by `at_least_once.max_seen_keys` (default `1_000`, LRU-evicted),
  so memory use is capped even when the key cardinality is very high.

```yaml
# Before
telemetry:
  hive:
    usage_reporting:
      sample_rate: 90%

# After
telemetry:
  hive:
    usage_reporting:
      sampler:
        type: fixed
        rate: 90%

# Or, to make sure rare operations are still observed:
telemetry:
  hive:
    usage_reporting:
      sampler:
        type: at_least_once
        key: operation_name      # or: { expression: "..." }
        rate: 10%                # rate applied after the first per key
        max_seen_keys: 1000      # optional, defaults to 1_000
```

## Internals: SDK now owns the config types and agent construction

`UsageReportingConfig`, `SamplerConfig`, `AtLeastOnceKey` and
`UsageReportingExclude` now live under `hive_console_sdk::agent::config`.
`hive-router-config` consumes them directly, and the Apollo Router plugin
`#[serde(flatten)]`s `UsageReportingConfig` into its plugin config. Both
routers also drop their hand-rolled `compile_sampler` helpers and the long
`UsageAgent::builder().endpoint(...).buffer_size(...)...` chains: configuration
is fed straight to the SDK with `UsageAgentBuilder::from_config(...)`, which
also performs VRL compilation for `exclude` and `sampler.key.expression`.

### New / shared primitives

A few cross-cutting types have moved into `hive_console_sdk::primitives` so
every consumer deserializes them with the same shape and JSON schema:

- `RetryPolicyConfig` is the single retry config shared by usage reporting,
  persisted documents and the supergraph fetcher.
  `telemetry.hive.usage_reporting.retry_policy.max_retries` is now
  configurable (defaults to `3`).
- `CircuitBreakerConfig` is the single circuit breaker shape used in the
  router. The subgraph traffic shaping config now `#[serde(flatten)]`s it
  (the YAML keys `error_threshold`, `volume_threshold`, `reset_timeout`,
  `half_open_attempts` are unchanged), and
  `telemetry.hive.usage_reporting.circuit_breaker` exposes it as an optional
  override on the usage reporting agent. Omit the field to keep the SDK
  defaults (50% error threshold, rolling sample of 5, 30s reset timeout, 10
  half-open probes), or set any subset of the four fields to override them.
- `TargetId` is a validated newtype with its own JSON schema (`oneOf` of a
  slug pattern `$organizationSlug/$projectSlug/$targetSlug` and a UUID
  pattern). `telemetry.hive.target` and the Apollo plugin's `target` field
  both use it, so misconfigured target ids fail at config-load time and
  surface in YAML editors instead of waiting for the agent to start. Existing
  valid values keep working unchanged.

## `hive-apollo-router-plugin` config: additional breaking changes

To match the unified SDK config shape, several plugin-level field names and
types changed (this is on top of the `sample_rate` -> `sampler` migration):

| Before                                  | After                                                  |
| --------------------------------------- | ------------------------------------------------------ |
| `registry_token: ...`                   | `token: ...`                                           |
| `registry_usage_endpoint: ...`          | `endpoint: ...`                                        |
| `connect_timeout: 5` (seconds, integer) | `connect_timeout: 5s` (humantime string)               |
| `request_timeout: 15`                   | `request_timeout: 15s`                                 |
| `flush_interval: 5`                     | `flush_interval: 5s`                                   |
| `sample_rate: 90%`                      | `sampler: { type: fixed, rate: 90% }`                  |
| `enabled` defaults to `true`            | `enabled` defaults to `false`, set it explicitly to opt in |

Environment variable fallbacks `HIVE_TOKEN`, `HIVE_TARGET_ID` and `HIVE_ENDPOINT`
still work the same way.
