---
hive-router: major
hive-console-sdk: major
hive-apollo-router-plugin: major
---

# Breaking Changes: Dynamic Exclusions

## Dynamic Exclusions in Hive Router

Hive Router now supports dynamic exclusions, allowing you to exclude specific requests from usage reporting based on custom logic. This feature is useful for scenarios where you want to skip telemetry for certain requests, such as health checks or specific endpoints.
Previously, it was only operation name based.

### Usage
```diff
- exclude: ['ExcludedOp']
+ exclude: '.request.operation.name == "ExcludedOp"'
```

The details about expression context is documented in the [Hive Router documentation](https://the-guild.dev/graphql/hive/docs/router/configuration/expressions).

## Dynamic Exclusions in Apollo Router

As in Hive Router, Apollo Router used to support only operation name based exclusions. With the new dynamic exclusions feature, you can now specify custom logic to exclude requests from usage reporting.


# New `add_report_with_request` method in Hive Console SDK

In order to support exclusions based on request properties, a new method `add_report_with_request` has been added to the Hive Console SDK. This method allows you to include the request information in the report, which can then be used in the dynamic exclusion logic.
