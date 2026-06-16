---
hive-router: patch
---

# Fix Router's HTTP layer timeout

Hive Router has it's own timeout that's being enforced, but `ntex`'s one was still effective and uses the default settings.  

Instead of fully disabling the low-level timeout, this PR changes the Router implementation to configure `ntex` timeout to `router_timeout+1` so the safe guard is still in place.
