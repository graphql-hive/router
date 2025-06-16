## Gateway

To run the gateway, use the following command:

```bash
cargo gateway SUPERGRAPH_PATH
```

## Env Vars

* `LOG_FORMAT`: Specifies the format of the logs. Can be `json`, `tree`, `compact`. Defaults to `compact`.
* `RUST_LOG`: Specifies the level of the logs. Can be `trace`, `debug`, `info`, `warn`, or `error`. Defaults to `info`. Uses `EnvFilter` to parse the log level and components.
* `EXPOSE_QUERY_PLAN`: Specifies whether to expose the query plan in the response. Can be `true` or `false`. Defaults to `false`.
