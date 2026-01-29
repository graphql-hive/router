---
hive-router: patch
hive-router-config: patch
--- 

New configuration flag to limit the incoming HTTP request body size in the router before parsing the request(JSON etc).

```yaml
limits:
  max_request_body_size: 2MB # Human readable size format
```

By default, this limit is set to 2MB.