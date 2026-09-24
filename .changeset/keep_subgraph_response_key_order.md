---
hive-router: patch
node-addon: patch
---

# Faster Processing of Large Subgraph Responses

The Router no longer sorts the fields of every object in a subgraph response. Objects now keep the order the subgraph sent them in, and the Router finds each field by checking the next one first, since responses follow the order of the query.

This removes a sort per object and a search per field. In our test with a single subgraph returning a 2.8MB response, the time the Router spends on each request dropped by about 10%, and throughput increased by about 5%.
