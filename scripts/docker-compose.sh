#!/usr/bin/env bash
# Prefer the Linux Docker CLI. Recover when Desktop runs but its WSL socket is stale.
set -euo pipefail
if timeout 5 docker info >/dev/null 2>&1; then
    exec docker compose "$@"
fi
desktop='/mnt/c/Program Files/Docker/Docker/resources/bin/docker.exe'
if [[ ! -x "$desktop" ]] || ! "$desktop" info >/dev/null 2>&1; then
    echo 'Docker is unavailable. Start Docker Desktop and enable this WSL distro integration.' >&2
    exit 1
fi
echo 'Using Docker Desktop Windows CLI because the WSL Docker socket is unavailable.' >&2
args=()
while (($#)); do
    if [[ "$1" == '-f' || "$1" == '--file' ]]; then
        args+=("$1" "$(wslpath -w "$(realpath "$2")")")
        shift 2
    else
        args+=("$1")
        shift
    fi
done
exec "$desktop" compose "${args[@]}"
