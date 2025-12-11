## 0.0.3 (2025-12-11)

### Fixes

- chore: Enable publishing of internal crate

## 0.0.2 (2025-12-11)

### Fixes

#### Extract expressions to hive-router-internal crate

The `expressions` module has been extracted from `hive-router-executor` into the `hive-router-internal` crate. This refactoring centralizes expressions handling, making it available to other parts of the project without depending on the executor.

It re-exports the `vrl` crate, ensuring that all consumer crates use the same version and types of VRL.
