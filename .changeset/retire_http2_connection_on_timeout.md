---
hive-router: patch
---

# Stop reusing an HTTP/2 subgraph connection after a request on it times out

When a subgraph request over HTTP/1 times out, its connection is closed with it. Over HTTP/2 only that request was canceled: the connection stayed in the pool and kept getting new requests, most likely to the same stuck subgraph instance.

Now, when a request to a subgraph runs out of `request_timeout`, the router stops reusing the connection it went over. Requests already on that connection finish normally, and new requests open a fresh connection. A request canceled for any other reason, such as the client going away, leaves the connection alone.
