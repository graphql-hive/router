---
hive-router-config: patch
hive-router: patch
hive-router-internal: patch
hive-router-plan-executor: patch
---

# Seed the embedded Hive Laboratory from the router config

Adds two optional keys under `laboratory`: `preflight`, a script that runs before every operation in the Laboratory (and can prompt the user for a token at runtime), and `operations`, named operations that each open in a pre-filled tab. `LABORATORY_PREFLIGHT_ENABLED` toggles the script via env var.

Seeded values are embedded in the served page and visible via "view source", so they must not contain secrets; use `lab.prompt` for anything sensitive. Preflight is a Laboratory convenience, not a router auth mechanism.
