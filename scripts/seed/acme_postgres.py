#!/usr/bin/env python3
"""Load the ACME Insurance CSV tables into the Postgres sidecar and write the Ontop R2RML mapping.

Source: https://github.com/datadotworld/cwd-benchmark-data/tree/main/ACME_Insurance. Column types come
from the dataset's DDL. The mapping is the dataset's own R2RML file with the qualified table names
(`myinsurancecompany.omg-pc-database.claim`) reduced to the Postgres table name. Pure standard library:
SQL is piped to `psql` inside the compose service.

  python3 scripts/seed/acme_postgres.py
"""
import csv, io, json, os, re, subprocess, sys, tempfile, urllib.parse, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RAW = 'https://raw.githubusercontent.com/datadotworld/cwd-benchmark-data/main/ACME_Insurance'
API = 'https://api.github.com/repos/datadotworld/cwd-benchmark-data/contents/ACME_Insurance'
COMPOSE = ['bash', str(ROOT / 'scripts/docker-compose.sh'), '-f', str(ROOT / 'sidecars/federation/compose.yml')]
SQL_TYPES = {'int': 'integer', 'bigint': 'bigint', 'decimal': 'numeric', 'datetime': 'timestamp'}

def fetch(url):
    with urllib.request.urlopen(url, timeout=60) as r: return r.read().decode('utf-8-sig')

def column_types(ddl):
    types = {}
    for line in ddl.splitlines():
        m = re.match(r'\s*(\w+)\s+(int|bigint|decimal|datetime|varchar|char)\b', line, re.I)
        if m: types[m[1].lower()] = SQL_TYPES.get(m[2].lower(), 'text')
    return types

def quote(value):
    return 'NULL' if value == '' else "'" + value.replace("'", "''") + "'"

def main():
    types = column_types(fetch(f'{RAW}/DDL/ACME_small.ddl'))
    script = ['DROP SCHEMA public CASCADE;', 'CREATE SCHEMA public;']
    for entry in json.loads(fetch(f'{API}/data')):
        if not entry['name'].endswith('.csv'): continue
        rows = list(csv.reader(io.StringIO(fetch(f"{RAW}/data/{urllib.parse.quote(entry['name'])}"))))
        table, header = entry['name'][:-4].lower(), [re.sub(r'\W+', '_', h.strip().lower()) for h in rows[0]]
        # Source quirk: Agreement.csv repeats `Agreement_Type_Code` where the key column belongs.
        if len(set(header)) < len(header) and f'{table}_identifier' not in header:
            header[header.index(next(h for h in header if header.count(h) > 1))] = f'{table}_identifier'
        columns = ', '.join(f"{h} {types.get(h, 'text')}" for h in header)
        script.append(f'CREATE TABLE {table} ({columns});')
        for i in range(1, len(rows), 500):
            values = ',\n'.join('(' + ','.join(quote(v) for v in r + [''] * (len(header) - len(r))) + ')' for r in rows[i:i + 500])
            script.append(f'INSERT INTO {table} VALUES {values};')
    roles = (ROOT / 'sidecars/federation/postgres/roles.sql').read_text()
    # Roles are cluster-wide, so re-running must not fail on the existing one.
    script.append(roles.replace('CREATE ROLE ontop_ro LOGIN PASSWORD', "DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='ontop_ro') THEN CREATE ROLE ontop_ro LOGIN PASSWORD").replace("'ontop-read-only';", "'ontop-read-only'; END IF; END $$;", 1))
    # Docker Desktop's Windows CLI does not forward stdin, so copy the script in and run it there.
    with tempfile.NamedTemporaryFile('w', suffix='.sql', delete=False) as handle: handle.write('\n'.join(script))
    try:
        path = subprocess.run(['wslpath', '-w', handle.name], capture_output=True, text=True).stdout.strip() if not os.access('/var/run/docker.sock', os.W_OK) else handle.name
        for command in (['cp', path, 'postgres:/tmp/seed.sql'], ['exec', '-T', 'postgres', 'psql', '-v', 'ON_ERROR_STOP=1', '-q', '-U', 'rdf', '-d', 'acme', '--file=/tmp/seed.sql']):
            run = subprocess.run(COMPOSE + command, stdin=subprocess.DEVNULL, capture_output=True, text=True)
            if run.returncode: sys.exit(run.stderr[-2000:])
    finally: os.unlink(handle.name)
    mapping = fetch(f'{RAW}/data/data.world_P%26C_Insurance_Ontology_V1.r2rml')
    mapping = re.sub(r'"myinsurancecompany\.omg-pc-database\.([^"]+)"', r'"\1"', mapping)
    out = ROOT / 'sidecars/federation/ontop/mapping.ttl'; out.write_text(mapping)
    # The ontology lets Ontop answer subclass/range questions over the live rows (query-time reasoning).
    ontology = ROOT / 'sidecars/federation/ontop/ontology.ttl'; ontology.write_text(fetch(f'{RAW}/ontology/insurance.ttl'))
    print(f'loaded {len([s for s in script if s.startswith("CREATE TABLE")])} tables; wrote {out.relative_to(ROOT)} and {ontology.relative_to(ROOT)}')

main()
