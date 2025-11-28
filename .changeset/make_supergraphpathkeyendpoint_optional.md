---
router: patch
config: patch
---

# Improve error messages and fix environment variable support for supergraph configuratio

- **Fix:** Previously, `supergraph.path` (for file source), and `supergraph.endpoint`/`supergraph.key` (for Hive CDN source) were mandatory in the configuration file. This prevented users from relying solely on environment variables (`SUPERGRAPH_FILE_PATH`, `HIVE_CDN_ENDPOINT`, `HIVE_CDN_KEY`). This has been fixed, and these fields are now optional in the configuration file if the corresponding environment variables are provided.
- **Improved Error Reporting:** If the supergraph file path or Hive CDN endpoint/key are missing from both configuration and environment variables, the error message now explicitly guides you to set the required environment variable or the corresponding configuration option.

This change ensures that misconfigurations are easier to diagnose and fix during startup.
