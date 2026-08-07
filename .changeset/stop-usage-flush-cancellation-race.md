---
hive-console-sdk: patch
---

# Stop cancellation from interrupting an active usage flush

The usage agent now observes cancellation while waiting for the next flush interval instead of checking only after the full interval has elapsed. Cancellation remains pending while an active flush completes, preventing a drained report batch from being lost when its send future is interrupted.
