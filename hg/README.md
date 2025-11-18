1. Pull Hive Gateway binary
1. `$ npm install`
1. `$ cargo run -p subgraphs `
1. `$ ./hive-gateway supergraph ../bench/supergraph.graphql`

```graphql
query MyQuery {
  topProducts {
    reviews {
      id
      favProduct {
        name
      }
    }
  }
}
```
