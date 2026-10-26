---
apollo-router-hive-fork: minor
---

# Add at-least-once sampling for usage reporting

As in Hive Router, the Apollo-Router plugin can now report the first operation for each unique key, even with a low `sample_rate`. Later operations with the same key follow `sample_rate`.

```yaml
plugins:
  hive.usage:
    sample_rate: 0.1 # 10% of operations will be reported
    at_least_once:
      key: # the combination of operation's name and body makes the request unique
        - operation_name
        - operation_body
      max_distinct_keys: 12000 # how many keys to track and hold in memory
```

Possible keys are `operation_name`, `operation_type` and `operation_body`. Keys are tracked in memory, up to `max_distinct_keys` (default: `100_000`). Every key takes approximately 16 bytes of memory.

Sampling and the `exclude` list of operation names are now applied by the Hive Console SDK, the same way as in Hive Router. Excluded operations are dropped before sampling, so they never take a sampling slot. Sampling is decided per report, so with `@defer` or subscriptions each response chunk is sampled on its own, instead of the whole request.

`exclude` expressions can now read the operation type from `.request.operation.type`; it used to be always empty.

Closes https://github.com/graphql-hive/router/issues/1757
