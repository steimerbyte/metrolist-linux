#!/bin/sh
# Serialize every rustc invocation project-wide; cargo jobs=1 covers one build,
# flock covers concurrent builds/probes with separate target dirs.
exec flock "$HOME/.cache/metrolist-rustc.lock" "$@"
