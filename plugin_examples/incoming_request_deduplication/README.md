# Deduplicate Incoming Requests Plugin Example

This example demonstrates how to implement a plugin that deduplicates incoming requests based on their fingerprint. When multiple identical requests are received while the first one is still being processed, the plugin ensures that only one request is sent to the downstream service, and all other requests receive the same response once it's available.

The plugin maintains a message broker to manage in-flight requests and their responses. When a request is received, the plugin checks if there is already an in-flight request with the same fingerprint. If there is, it subscribes to the response of that request instead of sending a new request downstream. Once the response is available, it is sent to all subscribers.

The test case simulates multiple parallel requests to the same endpoint and verifies that only one request is sent to the downstream service, while all requests receive the same response. The number of parallel requests can be adjusted to test the deduplication logic under different loads.

Implementation details can be found in the `src/plugin.rs` file, and the test case is located in the `src/test.rs` file.