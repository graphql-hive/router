---
node-addon: patch
---

# Javascript bindings only look for local addons

No more attemtpting to load an npm package that's deployed specifically for the binary.

This helps with the release process by not requiring rebuilds after version bumps from knope. Previously, the binding would import exact versions of addons and would therefore need to be updated on every release - to keep knope simple and not require it to rebuild, we simply always want the local addons.
