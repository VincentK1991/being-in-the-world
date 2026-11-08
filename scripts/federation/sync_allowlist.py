#!/usr/bin/env python3
"""Generate the Comunica sidecar's allowlist from the dataset configs (the single source).

Each dataset's deploy/local/tenants/<name>/federation.json lists the endpoints that dataset may
contact. The sidecar serves every dataset, so its list is their union, plus each local source.
`--check` exits 1 when sidecars/federation/comunica/allow.json is out of date (used by `just ci`).
"""
import argparse, json, pathlib, sys

root = pathlib.Path(__file__).resolve().parents[2]
tenants = sorted((root / 'deploy/local/tenants').glob('*/federation.json'))
target = root / 'sidecars/federation/comunica/allow.json'

def union():
    endpoints = set()
    for path in tenants:
        config = json.loads(path.read_text())
        endpoints.update(config['allow'])
        if config.get('local_source'): endpoints.add(config['local_source'])
    return sorted(endpoints)

def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--check', action='store_true')
    check = parser.parse_args().check
    current = json.loads(target.read_text())
    wanted = {**current, 'allow': union()}
    if current == wanted:
        print(f'{target.relative_to(root)} matches {len(tenants)} dataset configs'); return
    if check:
        print(f'{target.relative_to(root)} is out of date; run: python3 scripts/federation/sync_allowlist.py', file=sys.stderr)
        print('  missing:', sorted(set(wanted['allow']) - set(current['allow'])), 'extra:', sorted(set(current['allow']) - set(wanted['allow'])), file=sys.stderr)
        sys.exit(1)
    target.write_text(json.dumps(wanted, indent=2) + '\n'); print(f'updated {target.relative_to(root)}')

main()
