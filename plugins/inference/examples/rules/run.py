#!/usr/bin/env python3
"""Execute actual one-pass SPARQL and optionally Jena custom-rule examples."""
from pathlib import Path
import argparse,json,subprocess
root=Path(__file__).resolve().parents[4]
parser=argparse.ArgumentParser()
parser.add_argument('--output',type=Path,default=root/'target/rule-provider-demo')
parser.add_argument('--sparql-only',action='store_true')
args=parser.parse_args()
out=args.output.resolve()
if (out/'db').exists(): raise SystemExit('Choose a new output directory; refusing to overwrite the demo database.')
out.mkdir(parents=True,exist_ok=True)
def run(*argv):
    return subprocess.check_output([str(root/'target/debug/rdftool'),'--inference-config',str(root/'plugins/inference/example-rules-config.json'),*map(str,argv)],text=True)
run('ingest','--from',root/'plugins/inference/examples/rules/data.trig','--to',out/'db')
jobs=['work-sparql','totals']+([] if args.sparql_only else ['work-jena'])
for job in jobs:
    (out/(job+'-build.json')).write_text(run('plugin','--source',out/'db','--plugin','inference','--operation','rebuild','--input',json.dumps({'job':job})))
queries={
'sparql-countries':'SELECT ?person ?country WHERE { GRAPH <urn:rdftool:derived:inference:work-sparql> { ?person <https://example.org/worksInCountry> ?country } } ORDER BY ?person',
'sparql-no-second-pass':'ASK { GRAPH <urn:rdftool:derived:inference:work-sparql> { ?person <https://example.org/worksOnContinent> ?continent } }',
'sparql-totals':'SELECT ?line ?total WHERE { GRAPH <urn:rdftool:derived:inference:totals> { ?line <https://example.org/total> ?total } }',
'jena-continents':'SELECT ?person ?continent WHERE { GRAPH <urn:rdftool:derived:inference:work-jena> { ?person <https://example.org/worksOnContinent> ?continent } } ORDER BY ?person'}
for name,query in queries.items():
    if name.startswith('jena') and args.sparql_only: continue
    result=run('query','--source',out/'db',query)
    (out/(name+'.json')).write_text(result);(out/(name+'.rq')).write_text(query+'\n')
    print(name,result)
assert json.loads((out/'sparql-no-second-pass.json').read_text())['boolean'] is False
assert json.loads((out/'sparql-totals.json').read_text())['results']['bindings'][0]['total']['value']=='21'
print('Persistent database and exact results:',out)
