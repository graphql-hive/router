# Hive-Router's Plan Executor (`hive-router-plan-executor`)

This crate is a standalone library for performing GraphQL execution for a Federation query plan.

## Installation

Add this to your Cargo.toml:

```toml
[dependencies]
hive-router-plan-executor = "<...>"
```

## Usage

Start by either producing a plan (using [Hive Router query-planner](../query-planner)), or by loading it from a file or any other source.

Once a plan is available, it can be executed and using [the `hive-router-plan-executor` crate](../executor).

For usage example, please follow [the `router` binary hotpath](../../bin/router/src/pipeline/mod.rs). The step involved parsing, processing, planning and preparing the incoming operation.

Once all information is collected, you can use it as follows:

```rust
use hive_router_plan_executor::execute_query_plan;
use hive_router_plan_executor::execution::plan::QueryPlanExecutionContext;

// Result is a Vec<u8> you can send as final response or make into a Bytes buffer.
let result = execute_query_plan(QueryPlanExecutionContext {
    query_plan: query_plan_payload,
    projection_plan: &normalized_payload.projection_plan,
    variable_values: &variable_payload.variables_map,
    extensions,
    introspection_context: &introspection_context,
    operation_type_name: normalized_payload.root_type_name,
    executors: &subgraph_executor_map,
})
.await;
```
