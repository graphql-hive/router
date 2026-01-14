## 0.0.6 (2026-01-14)

### Fixes

#### Improved Performance for Expressions

This change introduces "lazy evaluation" for contextual information used in expressions (like dynamic timeouts).

Previously, the Router would prepare and clone data (such as request details or subgraph names) every time it performed an operation, even if that data wasn't actually needed.
Now, this work is only performed "on-demand" - for example, only if an expression is actually being executed.
This reduces unnecessary CPU usage and memory allocations during the hot path of request execution.

## 0.0.5 (2026-01-12)

### Fixes

#### Bump hive-router-config version

Somehow the `hive-router-internal` crate was published with an older version of the `hive-router-config` dependency.

## 0.0.4 (2025-12-12)

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

## 0.0.3 (2025-12-11)

### Fixes

- chore: Enable publishing of internal crate

## 0.0.2 (2025-12-11)

### Fixes

#### Extract expressions to hive-router-internal crate

The `expressions` module has been extracted from `hive-router-executor` into the `hive-router-internal` crate. This refactoring centralizes expressions handling, making it available to other parts of the project without depending on the executor.

It re-exports the `vrl` crate, ensuring that all consumer crates use the same version and types of VRL.
