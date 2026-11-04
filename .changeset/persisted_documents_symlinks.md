---
hive-router: patch
---

# Persisted documents file watching follows symlinks

The persisted documents file watcher reloads the manifest when its path goes through a symlinked directory, such as the temp directory on macOS. File system events on macOS report resolved paths, so the watcher matches events against both the configured and the canonicalized manifest path.
