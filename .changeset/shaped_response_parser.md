---
hive-router: patch
---

# Faster reading of subgraph responses

The router now reads each subgraph response with the shape of the query it sent. It knows which fields to expect and in what order, so it skips most of the work a general JSON parser does. Responses it isn't sure about are read the old way, so the results and errors stay the same. In our benchmark the router uses about 7% less CPU per request.
