---
hive-router: minor
---

# Log the response compression algorithm in the request summary

The request summary log line (`router::request`) now includes a `response_compression`
field recording which algorithm the router compressed the client-facing response with
(`gzip`, `deflate`, `br`, or `zstd`):

```json
{
  "target": "router::request",
  "operation_type": "query",
  "status_code": 200,
  "payload_bytes": 1234,
  "response_compression": "gzip"
}
```

The field is omitted when the response is sent uncompressed (compression disabled, the
client didn't advertise a supported `Accept-Encoding`, or the payload was below
`traffic_shaping.router.compression.response.min_size`).

Related https://github.com/graphql-hive/router/issues/1513
