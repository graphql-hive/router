---
internal: minor
router: minor
config: minor
executor: minor
query-planner: minor
---

# Tracing with OpenTelemetry

Introducing comprehensive OpenTelemetry-based tracing to the Hive Router, providing deep visibility into the GraphQL request lifecycle and subgraph communications.

- **OpenTelemetry Integration**: Support for OTLP exporters (gRPC and HTTP) and standard propagation formats (Trace Context, Baggage, Jaeger, B3/Zipkin).
- **GraphQL-Specific Spans**: Detailed spans for every phase of the GraphQL lifecycle
- **Hive Console Tracing**: Native integration with Hive Console for trace visualization and analysis.
- **Semantic Conventions**: Support for both stable and deprecated OpenTelemetry HTTP semantic conventions to ensure compatibility with a wide range of observability tools.
- **Optimized Performance**: Tracing is designed with a "pay only for what you use" approach. Overhead is near-zero when disabled, and allocations/computations are minimized when enabled.
- **Rich Configuration**: New configuration options for telemetry exporters, batching, and resource attributes.
