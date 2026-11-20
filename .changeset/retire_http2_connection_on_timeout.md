---
hive-router: patch
---

# Stop reusing an HTTP/2 subgraph connection after a request on it times out

Over HTTP/1, a timed-out subgraph request closes its connection. Over HTTP/2 only the request was canceled, and the connection kept getting new requests, likely to the same stuck instance. Now the router stops reusing it: requests already on it finish, and new ones open a fresh connection. Other cancellations, like the client going away, leave the connection alone.
