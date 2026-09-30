---
hive-router: patch
---

# Configure the plugin-selected supergraph runtime cache

Set the maximum number of plugin-selected supergraph runtimes retained in the router's shared FIFO cache:

```yaml
cache:
  router:
    plugin_supergraph_runtimes: 20
```

The default is 10. Set `plugin_supergraph_runtimes: 0` to disable caching for plugin-selected runtimes; each selection then builds a new runtime. The configured supergraph runtime is separate and does not count toward this limit.

You can also configure the value with `ROUTER_CACHE_PLUGIN_SUPERGRAPH_RUNTIMES=0` (or another non-negative integer). The environment override takes precedence over the config file.

The cache grows as runtimes are added rather than allocating space for the configured limit at startup, so a large limit does not reserve memory before it is needed.
