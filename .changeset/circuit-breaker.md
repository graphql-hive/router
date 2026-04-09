---
hive-router: patch
hive-router-plan-executor: patch
hive-router-config: patch
---

# Implement Circuit Breaker for Subgraph Requests

This change introduces a circuit breaker mechanism for subgraph requests in the Hive Router. The circuit breaker will monitor the success and failure rates of requests to each subgraph and will prevent future requests if the failure rate exceeds a certain threshold. When the circuit breaker is opened, subsequent requests to that subgraph will fail immediately without attempting to send the request.

This implementation helps improve the resilience and stability of the Hive Router when dealing with unreliable subgraphs.