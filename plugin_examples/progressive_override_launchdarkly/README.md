# Progressive Override + LaunchDarkly plugin example

This example shows how to resolve `hive::progressive_override::unresolved_labels`
with LaunchDarkly and set `hive::progressive_override::labels_to_override` from a plugin.

## Configure

Set your LaunchDarkly server-side SDK key:

```bash
export LD_SDK_KEY="your-launchdarkly-server-sdk-key"
```

## Run

```bash
cargo run -p progressive-override-launchdarkly-plugin-example -- \
  --config ./plugin_examples/progressive_override_launchdarkly/router.config.yaml
```

The plugin reads the context key from the `x-user-id` header by default
(configurable via `context_key_header`) and evaluates each unresolved override
label as a LaunchDarkly boolean flag key.
