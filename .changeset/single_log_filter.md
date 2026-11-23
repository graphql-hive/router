---
hive-router: patch
---

# Less CPU spent on disabled log levels

The router's log filters are combined into one, so `debug!` and `trace!` calls are skipped early when the log level is higher. About 3% less router CPU per request at `log.level: error`.
