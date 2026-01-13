---
router: patch
config: patch
---

# Remove extra `target_id` validation in Router config

This change removes the extra deserialization validation for the `target_id` field in the Router configuration, because it is already done by the Hive Console SDK.