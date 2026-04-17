---
hive-router-config: minor
hive-router-plan-executor: minor
hive-router: minor
---

# TLS Support

Add TLS support for the router

## TLS Directions

TLS Support has implementations for the following 4 directions:

### Router -> Client - Regular TLS
Router has an `identity` (`cert`, `key`), and client has `cert`, then Client validates the router's `identity`

### Client -> Router - mTLS
Router has the `cert`, client has the `identity`, mTLS/Client Auth then the router validates the client's `identity`

### Subgraph -> Router - Regular TLS
Subgraph has the `identity` (`cert`, `key`), and router has `cert`, then Router validates the subgraph's `identity`.

### Router -> Subgraph - mTLS
Subgraph has the `cert`, router(which is the client this time) has the `identity`, then subgraph validates the router's `identity`.

## Configuration Structure
```yaml
traffic_shaping:
  router:
    key_file:
    cert_file:
    client_auth:
       cert_file:
   all:
      key_file:
      cert_file:
   subgraphs:
      SUBGRAPH_NAME:
          cert_file:
          client_auth:
             cert_file:
             key_file:
```