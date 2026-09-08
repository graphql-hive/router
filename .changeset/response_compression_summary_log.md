---
hive-router: minor
---

# Log response compression details in the request summary

The request access log / summary log line (`router::request`) now reports how the client-facing response was compressed:

- `response_compression` - the algorithm the response was compressed with (`gzip`, `deflate`, `br`, or `zstd`).
- `response_bytes` - the compressed size, in bytes, actually sent over the wire. This sits next to the existing `payload_bytes`, which remains the uncompressed size, so the two make the achieved compression ratio visible.

```json
{
  "target": "router::request",
  "operation_type": "query",
  "status_code": 200,
  "payload_bytes": 4096,
  "response_compression": "gzip",
  "response_bytes": 512
}
```

Both fields are omitted when the response is sent uncompressed (compression disabled, the
client didn't advertise a supported `Accept-Encoding`, or the payload was below
`traffic_shaping.router.compression.response.min_size`).

Related https://github.com/graphql-hive/router/issues/1513
