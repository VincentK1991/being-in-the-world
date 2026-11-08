#!/usr/bin/env bash
# Hot-reload for the running sidecars: Python edits are copied in and the service restarts;
# Java/Maven edits rebuild the Jena image. Start sidecars first (`just inference`, `just semantic-cpu`).
set -uo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-dev.sh
command -v inotifywait >/dev/null || { echo 'inotifywait is required: sudo apt install inotify-tools' >&2; exit 1; }
declare -A COMPOSE=([sidecars/inference]=sidecars/inference/compose.yml [sidecars/jena-rules]=sidecars/jena-rules/compose.yml [sidecars/semantic]=sidecars/semantic/compose.yml)
declare -A SERVICE=([sidecars/inference]=owlrl [sidecars/jena-rules]=jena [sidecars/semantic]=embeddings)
inotifywait -m -r -q -e close_write,moved_to --exclude '(__pycache__|/target/|test_)' --format '%w%f' sidecars |
while read -r path; do
    is_scratch "$path" && continue
    dir=$(echo "$path" | cut -d/ -f1-2); [[ -n ${COMPOSE[$dir]:-} ]] || continue
    C=(-f "${COMPOSE[$dir]}"); svc=${SERVICE[$dir]}
    case $path in
        sidecars/semantic/embeddings/server.py) push_file "${C[@]}" -- "$svc" "$path" /app/server.py ;;
        sidecars/*/*.py) push_file "${C[@]}" -- "$svc" "$path" "/app/$(basename "$path")" ;;
        *) echo "[$(date +%T)] rebuilding $svc…"; bash scripts/docker-compose.sh "${C[@]}" up -d --build "$svc" </dev/null >/dev/null 2>&1; echo "[$(date +%T)] $svc rebuilt"; continue ;;
    esac
    bash scripts/docker-compose.sh "${C[@]}" restart "$svc" </dev/null >/dev/null 2>&1 && echo "[$(date +%T)] $svc reloaded ($(basename "$path"))"
done
