---
hive-router: patch
---

Fix JWT audience validation rejecting tokens when `audiences` is not configured.

Previously, any token containing an `aud` claim was rejected with a 403 error even though audience validation is documented to be skipped when `audiences` is left empty.
