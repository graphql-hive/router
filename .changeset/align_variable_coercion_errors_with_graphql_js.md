---
hive-router-plan-executor: patch
hive-router: patch
---

# Improve variable coercion error messages

Variable coercion errors (invalid scalar/enum/object values, missing required fields, non-null violations) reports clear and informative error messages.

This only changes error text - error codes and HTTP status codes are unchanged.
