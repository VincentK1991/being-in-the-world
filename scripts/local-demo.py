#!/usr/bin/env python3
"""Add sample RDF and validate configured providers on the local Docker stack.
Does not clear any graphs. Results are saved under target/local-deployment.
"""
import json
from pathlib import Path
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
BASE = 'http://127.0.0.1:4321'
OUT = ROOT / 'target/local-deployment'
OUT.mkdir(parents=True, exist_ok=True)

def post(path, body, media):
    with urlopen(Request(BASE + path, data=body, headers={'Content-Type': media}), timeout=180) as response:
        return json.load(response)

def operation(plugin, name, arguments):
    return post(f'/api/plugins/{plugin}/operations/{name}', json.dumps(arguments).encode(), 'application/json')

def save(name, value):
    (OUT / f'{name}.json').write_text(json.dumps(value, indent=2) + '\n')
    print(name, json.dumps(value))

for name, media in [
    ('plugins/inference/examples/rules/data.trig', 'application/trig'),
    ('plugins/inference/example.trig', 'application/trig'),
    ('plugins/semantic-search/examples/catalog.ttl', 'text/turtle'),
]:
    post('/api/import', (ROOT / name).read_bytes(), media)

for job in ['work-sparql', 'totals', 'work-jena', 'vehicles']:
    save(job + '-rebuild', operation('inference', 'rebuild', {'job': job}))

for name in ['sparql-countries', 'sparql-no-second-pass', 'sparql-totals', 'jena-continents']:
    folder = ROOT / 'plugins/inference/examples/rules'
    actual = post('/query', (folder / f'{name}.rq').read_bytes(), 'application/sparql-query')
    expected = json.loads((folder / f'{name}.json').read_text())
    assert actual == expected, (name, actual, expected)
    save(name, actual)

save('semantic-rebuild', operation('semantic-search', 'rebuild', {'index': 'descriptions'}))
for path in sorted((ROOT / 'plugins/semantic-search/examples/queries').glob('*.rq')):
    if path.name.startswith(('08', '09')):
        continue  # Graph serialization and optional hosted provider are separate checks.
    save(path.stem, post('/query', path.read_bytes(), 'application/sparql-query'))
print('Sample RDF loaded; inference expected results matched; local semantic queries executed.')
