---
hive-router: major
---

Merge the `hive-router-query-planner`, `hive-router-plan-executor`, `hive-router-config`, and `hive-router-internal` crates into the main `hive-router` crate. They are no longer published as separate crates on crates.io.

**If you just run the router (binary or Docker image), or write plugins against the `hive-router` crate: nothing changes for you.** The plugin API (`hive_router::plugins::*`, `RouterPlugin`, hooks, and everything re-exported at the crate root) is unaffected.

**If you depend directly on one of the four merged crates**, you'll need to switch to depending on `hive-router` instead and update your imports:

| Before                                    | After                              |
| ------------------------------------------ | ----------------------------------- |
| `hive-router-query-planner` (Cargo dep)    | `hive-router`                       |
| `hive_router_query_planner::...`           | `hive_router::query_planner::...`   |
| `hive-router-plan-executor` (Cargo dep)    | `hive-router`                       |
| `hive_router_plan_executor::...`           | `hive_router::executor::...`        |
| `hive-router-config` (Cargo dep)           | `hive-router`                       |
| `hive_router_config::...`                  | `hive_router::config::...`          |

`hive-router-internal` has no public replacement — it was never meant for use outside of the router itself, and is now a private module.

The four crates' existing published versions on crates.io are untouched, but they won't receive any further releases.
