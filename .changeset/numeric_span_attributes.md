---
hive-router: patch
---

Export numeric span attributes as integers

Some span attributes were exported as strings, even though the OpenTelemetry semantic conventions define them as integers. Filters that compare numbers, such as tail-sampling policies or TraceQL queries like `span.http.response.status_code >= 500`, did not match these attributes.

The following attributes are now exported as integers:

- `http.response.status_code`
- `http.request.body.size` and `http.response.body.size`
- `server.port`, `client.port` and `network.peer.port`
- `hive.graphql.error.count`
- `cost.estimated` and `cost.actual`

`error.type` on HTTP spans is now always the bare status code as a string, such as `"500"`, as the semantic conventions require. Previously it was sometimes an integer, and 5xx responses recorded the full reason phrase, such as `"500 Internal Server Error"`.
