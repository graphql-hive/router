---
hive-router: patch
---

# Datadog tracing now honors `parent_based_sampler`

The Datadog tracing exporter now respects `telemetry.tracing.collect.parent_based_sampler` when deciding whether to inherit the sampling decision from an incoming trace.

Previously, this setting only affected the OpenTelemetry sampling path. Datadog always inherited the sampling decision carried by a remote parent context, which meant that `parent_based_sampler: false` was effectively ignored when Datadog tracing was enabled. For example, an incoming W3C `traceparent` marked as sampled could cause Datadog to retain the trace even when the router was configured with `sampling: 0.0`.

With this change, the two settings now have consistent and explicit behavior for Datadog:

- When `parent_based_sampler: true`, Datadog continues to honor the incoming parent's sampling decision. A sampled remote parent remains sampled.
- When `parent_based_sampler: false`, the router does not allow the incoming sampled flag to decide whether Datadog retains the trace. Datadog instead evaluates the trace using its configured sampling policy, including `telemetry.tracing.collect.sampling`, Datadog sampling rules, and Datadog rate limits.

For example:

```yaml
telemetry:
  tracing:
    collect:
      sampling: 0.0
      parent_based_sampler: false
    exporters:
      - kind: datadog
```

If a request arrives with a sampled parent:

```text
traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
```

Datadog now ignores only the inherited sampling decision and applies the configured `0.0` sampling rate. The detailed trace is therefore not retained. Datadog still receives the request statistics needed for accurate all-request metrics, including hit and error counts.

Trace continuity is preserved while making the new decision. The incoming trace ID, parent span ID, remote-parent relationship, and trace state continue to propagate through the router. Only the inherited sampled state is deferred so that Datadog's own sampler can make the final decision.

This behavior applies only when an enabled Datadog exporter is present and `parent_based_sampler` is false. OpenTelemetry exporters and Datadog configurations that enable parent-based sampling retain their existing behavior.
