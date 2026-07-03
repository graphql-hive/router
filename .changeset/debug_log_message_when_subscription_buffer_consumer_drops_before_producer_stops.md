---
hive-router-plan-executor: patch
hive-router: patch
---

# Debug log message when subscription buffer consumer drops before producer stops

"Consumer for subgraph {} at {} dropped the receiver" was logged at `error` level, but it fires on the normal teardown path: once all clients unsubscribe/disconnect from a subscription, the broadcast channel closes, the pump task stops draining, and the mpsc receiver drops. This is expected cleanup, not a failure, so users were seeing noisy error logs for routine subscribe/unsubscribe churn.

Changed to `debug` level with a clearer message explaining it's expected shutdown of the upstream drain, not an actual error.
