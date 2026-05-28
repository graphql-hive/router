---
hive-router: minor
hive-router-query-planner: minor
hive-router-plan-executor: minor
hive-router-config: minor
hive-router-internal: minor
---

# Demand Control with `@cost` and `@listSize` directives

Add support for the [Demand Control specification](https://ibm.github.io/graphql-specs/cost-spec.html), allowing operators to limit the cost of incoming GraphQL operations using the `@cost` and `@listSize` directives.

The router now calculates the cost of incoming operations based on directive-driven type, field, and argument costs (with list-size estimation) and can reject operations that exceed a configured maximum. Both static (request) and actual (response) cost can be measured, and the behavior is configurable via the new `demand_control` section in the router configuration.

Telemetry is included: new metrics under `demand_control_metrics` and additional span attributes expose estimated/actual cost and rejection reasons for observability.

[Documentation for the feature is available here](https://the-guild.dev/graphql/hive/docs/router/security/demand-control)