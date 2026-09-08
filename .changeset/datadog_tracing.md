---
hive-router: minor
---

# Add native Datadog tracing

Adds native Datadog tracing through the Datadog Agent while preserving the Router's existing OpenTelemetry instrumentation, propagation, and lifecycle handling. The Datadog-backed provider supports Router resources and span limits, native sampling rules and rate limits, all-request APM statistics, and mixed OTLP, stdout, and Hive exporters.

Configure the integration with the new `datadog` tracing exporter.

```yaml
telemetry:
  tracing:
    collect:
      # detailed traces are sampled, but request, error, and latency
      # statistics still cover all recorded requests
      sampling: 0.01
    exporters:
      - kind: datadog
        endpoint: http://datadog-agent:8126
```
