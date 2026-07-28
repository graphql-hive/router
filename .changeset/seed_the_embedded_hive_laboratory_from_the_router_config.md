---
hive-router-config: patch
hive-router: patch
hive-router-internal: patch
hive-router-plan-executor: patch
---

# Seed the embedded Hive Laboratory from the router config

Adds optional keys under `laboratory`: `operations`, named operations that each open in a pre-filled tab, and `collections`, named groups of operations shown in the Laboratory's sidebar. Each operation may carry `variables`, `headers` and `extensions` as native YAML maps.

Seeded values are embedded in the served page and visible via "view source", so they must not contain secrets. Seeded operations and collections are refreshed from config on every reload; work a user creates themselves is preserved.
