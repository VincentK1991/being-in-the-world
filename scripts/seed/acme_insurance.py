#!/usr/bin/env python3
"""Seed the local store with the ACME Insurance benchmark (ontology + R2RML-materialised instances).

Source: https://github.com/datadotworld/cwd-benchmark-data/tree/main/ACME_Insurance
Instances are the CSV tables run through the dataset's own R2RML mapping (templates, columns,
classes, table names and SQL queries; evaluated with SQLite), so IRIs and predicates match the
dataset. Requires `rdflib`.

  python3 scripts/seed/acme_insurance.py                # download, convert, import
  python3 scripts/seed/acme_insurance.py --out acme.ttl # also write the Turtle file
  python3 scripts/seed/acme_insurance.py --no-import --out acme.ttl
"""
import argparse, csv, io, json, re, sqlite3, urllib.parse, urllib.request
from rdflib import Graph, Literal, URIRef, RDF, Namespace
from rdflib.namespace import XSD

RAW = 'https://raw.githubusercontent.com/datadotworld/cwd-benchmark-data/main/ACME_Insurance'
API = 'https://api.github.com/repos/datadotworld/cwd-benchmark-data/contents/ACME_Insurance'
RR = Namespace('http://www.w3.org/ns/r2rml#')
DWO = Namespace('https://ontology.data.world/v0#')
GRAPH = 'urn:rdftool:examples:acme-insurance'

def fetch(url): 
    with urllib.request.urlopen(url, timeout=60) as r: return r.read().decode('utf-8-sig')

def column_types(ddl):
    """column name (lower) -> xsd datatype from the DDL; unknown columns fall back to value sniffing."""
    types = {}
    for line in ddl.splitlines():
        m = re.match(r'\s*(\w+)\s+(int|bigint|decimal|datetime|varchar|char)\b', line, re.I)
        if m: types[m[1].lower()] = {'int': XSD.integer, 'bigint': XSD.integer, 'decimal': XSD.decimal, 'datetime': XSD.dateTime}.get(m[2].lower())
    return types

def literal(value, datatype):
    if datatype == XSD.dateTime:  # CSV dates carry no time part
        return Literal(value, datatype=XSD.date) if re.fullmatch(r'\d{4}-\d\d-\d\d', value) else Literal(value, datatype=XSD.dateTime)
    return Literal(value, datatype=datatype) if datatype else Literal(value)

def expand(template, row):
    def sub(match):
        value = row[match[1].lower()]
        return None if value is None else urllib.parse.quote(str(value), safe='')
    parts = [sub(m) for m in re.finditer(r'\{([^}]+)\}', template)]
    return None if None in parts else re.sub(r'\{([^}]+)\}', lambda m: sub(m), template)

def main():
    ap = argparse.ArgumentParser(); ap.add_argument('--out'); ap.add_argument('--no-import', action='store_true')
    ap.add_argument('--store', default='http://127.0.0.1:4321'); ap.add_argument('--graph', default=GRAPH)
    args = ap.parse_args()

    ontology = Graph().parse(data=fetch(f'{RAW}/ontology/insurance.ttl'), format='turtle')
    mapping = Graph().parse(data=fetch(f'{RAW}/data/data.world_P%26C_Insurance_Ontology_V1.r2rml'), format='turtle')
    types = column_types(fetch(f'{RAW}/DDL/ACME_small.ddl'))

    db = sqlite3.connect(':memory:'); db.row_factory = sqlite3.Row
    for entry in json.loads(fetch(f'{API}/data')):
        if not entry['name'].endswith('.csv'): continue
        rows = list(csv.reader(io.StringIO(fetch(f"{RAW}/data/{urllib.parse.quote(entry['name'])}"))))
        table, header = entry['name'][:-4].lower(), [h.lower() for h in rows[0]]
        # Source quirk: Agreement.csv repeats `Agreement_Type_Code` where the key column
        # `Agreement_Identifier` belongs. Restore the key so the mapping's joins resolve.
        if len(set(header)) < len(header) and f'{table}_identifier' not in header:
            header[header.index(next(h for h in header if header.count(h) > 1))] = f'{table}_identifier'
        db.execute(f'create table "{table}" ({",".join(f"{chr(34)}{h}{chr(34)}" for h in header)})')
        db.executemany(f'insert into "{table}" values ({",".join("?" * len(header))})', [[v if v != '' else None for v in r] for r in rows[1:]])

    instances = Graph()
    for tm in mapping.subjects(RDF.type, RR.TriplesMap):
        logical = mapping.value(tm, RR.logicalTable)
        sql = mapping.value(logical, RR.sqlQuery)
        query = str(sql) if sql else f'select * from "{str(mapping.value(logical, RR.tableName)).split(".")[-1]}"'
        rows = db.execute(query).fetchall()
        subject_map = mapping.value(tm, RR.subjectMap)
        template = mapping.value(subject_map, RR.template)
        classes = list(mapping.objects(subject_map, RR['class']))
        for row in rows:
            r = {k.lower(): row[k] for k in row.keys()}
            iri = expand(str(template), r)
            if iri is None: continue
            s = URIRef(iri)
            for c in classes: instances.add((s, RDF.type, c))
            for pom in mapping.objects(tm, RR.predicateObjectMap):
                p, om = mapping.value(pom, RR.predicate), mapping.value(pom, RR.objectMap)
                if om is None: continue
                if (t := mapping.value(om, RR.template)) is not None:
                    o = expand(str(t), r)
                    if o is not None: instances.add((s, p, URIRef(o)))
                elif (col := mapping.value(om, RR.column)) is not None:
                    v = r.get(str(col).lower())
                    if v is not None: instances.add((s, p, literal(str(v), types.get(str(col).lower()))))

    combined = Graph()
    for prefix, ns in ontology.namespaces(): combined.bind(prefix, ns)
    combined.bind('inst', 'https://myinsurancecompany.linked.data.world/d/omg-pc-database/')
    for triple in ontology: combined.add(triple)
    for triple in instances: combined.add(triple)
    turtle = combined.serialize(format='turtle')
    print(f'ontology: {len(ontology)} triples, instances: {len(instances)} triples, total: {len(combined)}')
    if args.out: open(args.out, 'w').write(turtle)
    if not args.no_import:
        request = urllib.request.Request(f'{args.store}/api/import?{urllib.parse.urlencode({"graph": args.graph})}',
                                         data=turtle.encode(), headers={'Content-Type': 'text/turtle'}, method='POST')
        with urllib.request.urlopen(request, timeout=120) as r: print('imported into', args.graph, '->', r.status, r.read().decode()[:200])

if __name__ == '__main__': main()
