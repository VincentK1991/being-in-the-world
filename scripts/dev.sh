#!/usr/bin/env bash
# Hot-reload dev loop for the Docker deployment (UI + backend). Sidecars: `just dev-sidecars`.
#  - UI:      Astro dev server in Docker; edits under apps/studio are copied in and Vite HMR refreshes the browser.
#  - Backend: edits under crates/ rebuild the debug binary on the host, copy it into the container and restart it.
set -uo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-dev.sh
C=(-f deploy/local/compose.yml -f deploy/local/compose.dev.yml)
command -v inotifywait >/dev/null || { echo 'inotifywait is required: sudo apt install inotify-tools' >&2; exit 1; }

cargo build --locked -p rdf-cli || exit 1
bash scripts/docker-compose.sh "${C[@]}" up -d --build --wait --wait-timeout 300 backend ui || exit 1

ui_sync() {
    inotifywait -m -r -q -e close_write,moved_to,delete --format '%e|%w%f' \
        apps/studio/src apps/studio/public apps/studio/astro.config.mjs apps/studio/tsconfig.json |
    while IFS='|' read -r event path; do
        is_scratch "$path" && continue
        dest="/app/${path#apps/studio/}"
        if [[ $event == DELETE* ]]; then remove_file "${C[@]}" -- ui "$dest"; elif [[ -f $path ]]; then push_file "${C[@]}" -- ui "$path" "$dest"; fi
        echo "[$(date +%T)] ui synced ${path#apps/studio/}"
    done
}
backend_reload() {
    while inotifywait -r -q -e close_write,moved_to,create,delete \
            --include '\.(rs|toml|lock)$' crates Cargo.toml Cargo.lock >/dev/null; do
        sleep 0.7 # let a burst of saves finish
        echo "[$(date +%T)] rebuilding backend…"
        if cargo build --locked -p rdf-cli; then
            push_file "${C[@]}" -- backend target/debug/rdftool /usr/local/bin/rdftool \
                && bash scripts/docker-compose.sh "${C[@]}" restart backend </dev/null >/dev/null 2>&1 \
                && echo "[$(date +%T)] backend reloaded" || echo "[$(date +%T)] could not update the backend container"
        else echo "[$(date +%T)] build failed; still serving the previous backend"; fi
    done
}
ui_sync & backend_reload & wait
