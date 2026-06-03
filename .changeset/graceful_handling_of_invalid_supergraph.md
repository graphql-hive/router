---
hive-router: patch
---

# Graceful handling of invalid Supergraph while polling

As described in [issue #1089](https://github.com/graphql-hive/router/issues/1089), when the Supergraph fails to parse, the internal mpsc channel should not panic or collapse. 

This fix prevents the Router from crashing when the Supergraph fails to parse, and keeps the channel alive for future updates. 

In case of an invalid Supergraph, the channel is not closed, allowing the Router to continue receiving updates, and the error is being logged. Also, a telemetry (metrics) event is being emitted to track the error.
