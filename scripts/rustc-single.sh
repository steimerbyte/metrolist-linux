#!/bin/sh
# Cap rustc at 2 concurrent processes project-wide: cargo's `jobs = 2` covers a
# single build, this 2-slot semaphore also covers concurrent builds/probes with
# separate target dirs. Each slot is a flock(2) on a lock file, so the kernel
# drops it on any exit including SIGKILL - no stale state to sweep.
# Do not add a third slot.
dir="$HOME/.cache/metrolist-rustc-slots"
mkdir -p "$dir" || exit 1

while :; do
	for s in 1 2; do
		exec 9>"$dir/$s.lock"
		if flock -n 9; then
			exec "$@"
		fi
		exec 9>&-
	done
	sleep 0.05
done
