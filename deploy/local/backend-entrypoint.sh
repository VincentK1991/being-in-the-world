#!/usr/bin/env bash
# Container transport only. RDF logic and remote-query policy remain in rdftool.
set -uo pipefail
pids=()
shutdown() {
    trap '' TERM INT
    kill -TERM "${pids[@]}" 2>/dev/null || true
    wait || true
}
trap 'shutdown; exit 0' TERM INT
# Docker publishes this bridge listener only on host 127.0.0.1.
socat TCP-LISTEN:7878,bind=0.0.0.0,reuseaddr,fork TCP:127.0.0.1:7879 &
pids+=("$!")
# Existing optional sidecars are published on Docker Desktop's localhost.
for port in 16333 18081 18082 18083 18090 18095; do
    socat "TCP-LISTEN:${port},bind=127.0.0.1,reuseaddr,fork" "TCP:host.docker.internal:${port}" &
    pids+=("$!")
done
rdftool "$@" &
pids+=("$!")
wait -n
status=$?
shutdown
exit "$status"
