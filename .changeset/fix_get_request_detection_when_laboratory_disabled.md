---
hive-router: patch
---

# Fix GET request detection when Laboratory disabled

Hitting the router with a `GET` request from a browser was returning 404 when Laboratory is disabled. 

This change adds a check to only negotiate for Laboratory when Laboratory is enabled, and fallbacks to GET request handling when Laboratory is disabled.
