---
node-addon: minor
hive-router-query-planner: patch
---

# Public API Changes

## Progressive Override support in `QueryPlanner.plan`

Now `QueryPlanner.plan` accepts two additional parameters: `activeLabels` and `percentageValue`. These parameters are used to determine which overrides should be applied when generating the query plan. The `activeLabels` parameter is a set of labels that are currently active, and the `percentageValue` parameter is a number between 0 and 100 that represents the percentage of traffic that should be routed to the overrides.

## `AbortSignal` support in `QueryPlanner.plan`

The `QueryPlanner.plan` method now also accepts an optional `signal` parameter of type `AbortSignal`. This allows the caller to abort the query planning process if it takes too long or if the user cancels the operation. If the signal is aborted, the `plan` method will throw an error.

## `overrideLabels` and `overridePercentages` getters

Two new getters have been added to the `QueryPlanner` class: `overrideLabels` and `overridePercentages`. The `overrideLabels` getter returns a set of all the labels that are defined in the planner's supergraph, while the `overridePercentages` getter returns an array of all the percentage values that are defined in the planner's supergraph. These getters can be used by the caller to determine which overrides are available and how they are configured.

## `QueryPlanner.plan` is no longer a `Promise`

The `QueryPlanner.plan` method is now a synchronous method that returns a `QueryPlan` directly, instead of returning a `Promise`. This change was made to simplify the API and to allow for better error handling. If the query planning process encounters an error, it will throw an exception that can be caught by the caller.

## `QueryPlanner` constructor now uses `safe_parse_schema`

The `QueryPlanner` constructor now uses the `safe_parse_schema` function to parse the supergraph SDL. This function is a safer alternative to the previous parsing method, as it returns a `Result` that can be handled gracefully in case of parsing errors. If the SDL cannot be parsed, the constructor will return an error instead of panicking.

# Implementation changes

- The `QueryPlanner` struct now holds a `Planner` instance directly, instead of an `Arc<Planner>`. This change was made to simplify the internal implementation and to avoid unnecessary reference counting. Since the `QueryPlanner` is not designed to be shared across threads, there is no need for the additional overhead of an `Arc`.

- `AbortSignal` and `CancellationToken` integration to give the ability to cancel the query planning process to the Node addon consumer.