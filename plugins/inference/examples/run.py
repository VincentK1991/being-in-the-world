#!/usr/bin/env python3
"""Build the real sample inferred graph and retain exact SPARQL outputs."""
from pathlib import Path
import json
import subprocess
import argparse
root=Path(__file__).resolve().parents[3]
parser=argparse.ArgumentParser()
parser.add_argument('--output',type=Path,default=root/'target/inference-demo')
args=parser.parse_args()
out=args.output.resolve()
if (out/'db').exists(): raise SystemExit('Choose a new output directory; database already exists.')
out.mkdir(parents=True,exist_ok=True)
def run(*args):
    return subprocess.check_output([str(root/'target/debug/rdftool'),'--inference-config',str(root/'plugins/inference/example-config.json'),*map(str,args)],text=True)
run('ingest','--from',root/'plugins/inference/example.trig','--to',out/'db')
result=run('plugin','--source',out/'db','--plugin','inference','--operation','rebuild','--input','{"job":"vehicles"}')
(out/'build.json').write_text(result)
queries={
'classes':'SELECT ?type WHERE { GRAPH <urn:rdftool:derived:inference:vehicles> { <https://example.org/car1> a ?type } FILTER(?type IN (<https://example.org/Vehicle>, <https://example.org/Asset>)) } ORDER BY ?type',
'inverse':'SELECT ?parent ?child WHERE { GRAPH <urn:rdftool:derived:inference:vehicles> { ?parent <https://example.org/hasChild> ?child } }',
'transitive':'ASK { GRAPH <urn:rdftool:derived:inference:vehicles> { <https://example.org/office> <https://example.org/locatedIn> <https://example.org/country> } }',
'isolation':'ASK { GRAPH <urn:rdftool:derived:inference:vehicles> { <https://example.org/secret> a <https://example.org/Vehicle> } }'}
for name,query in queries.items():
    result=run('query','--source',out/'db',query)
    (out/(name+'.json')).write_text(result)
    (out/(name+'.rq')).write_text(query+'\n')
    print(name, result)
assert json.loads((out/'transitive.json').read_text())['boolean'] is True
assert json.loads((out/'isolation.json').read_text())['boolean'] is False
print('Database and results:',out)
