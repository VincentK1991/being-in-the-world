#!/usr/bin/env python3
"""Run real-model examples and save exact outputs. --openai opts into hosted requests."""
import argparse
import json
from pathlib import Path
import subprocess
import os

root = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser()
parser.add_argument('--openai', action='store_true')
parser.add_argument('--output', type=Path, default=root / 'target/semantic-provider-demo')
args = parser.parse_args()
if args.openai and not os.environ.get('OPENAI_API_KEY'):
    raise SystemExit('Set OPENAI_API_KEY in your environment before using --openai.')
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=True)
db = out / 'db'
if db.exists():
    raise SystemExit('Use a new --output directory; refusing to overwrite an existing demo database.')
config = root / 'plugins/semantic-search/example-providers.json'
binary = root / 'target/debug/rdftool'
def run(*argv):
    return subprocess.check_output([str(binary), '--semantic-config', str(config), *map(str,argv)], text=True)
run('ingest','--from',root/'plugins/semantic-search/examples/catalog.ttl','--to',db)
indexes = ['descriptions'] + (['descriptions_openai'] if args.openai else [])
for index in indexes:
    result = run('plugin','--source',db,'--plugin','semantic-search','--operation','rebuild','--input',json.dumps({'index':index}))
    (out / f'{index}-build.json').write_text(result)
for query in sorted((root/'plugins/semantic-search/examples/queries').glob('*.rq')):
    if query.name.startswith('09') and not args.openai:
        continue
    result = run('query','--source',db,'--query-file',query)
    extension = '.ttl' if query.name.startswith('08') else '.json'
    (out / (query.stem + extension)).write_text(result)
    print(query.name + '\n' + result)
print(f'Actual results and persistent database: {out}')
