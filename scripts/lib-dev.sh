# Shared helpers for the hot-reload scripts. Source this; do not execute.
# Docker Desktop's Windows CLI (used when the WSL socket is down) needs Windows paths for `cp`.
if timeout 5 docker info </dev/null >/dev/null 2>&1; then dpath() { realpath "$1"; }; else dpath() { wslpath -w "$(realpath "$1")"; }; fi

# push_file <compose-args...> -- <service> <local-file> <container-path>
push_file() {
    local args=() ; while [[ "$1" != -- ]]; do args+=("$1"); shift; done; shift
    local service=$1 file=$2 dest=$3
    bash scripts/docker-compose.sh "${args[@]}" cp "$(dpath "$file")" "$service:$dest" </dev/null >/dev/null 2>&1 && return
    # New directory inside the container: create it, then retry once.
    bash scripts/docker-compose.sh "${args[@]}" exec -T -u root "$service" mkdir -p "$(dirname "$dest")" </dev/null >/dev/null 2>&1
    bash scripts/docker-compose.sh "${args[@]}" cp "$(dpath "$file")" "$service:$dest" </dev/null >/dev/null 2>&1
}
# Editor scratch files (sed -i, vim swap, backups) are never worth syncing.
is_scratch() { [[ $(basename "$1") =~ ^(\.|sed[A-Za-z0-9]{6}$|4913$)|~$|\.tmp\.[0-9]+\.|\.sw[a-z]$ ]]; }
remove_file() {
    local args=() ; while [[ "$1" != -- ]]; do args+=("$1"); shift; done; shift
    bash scripts/docker-compose.sh "${args[@]}" exec -T -u root "$1" rm -rf "$2" </dev/null >/dev/null 2>&1
}
