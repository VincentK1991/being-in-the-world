#!/usr/bin/env python3
"""Create two small business-unit datasets that join to the Postgres virtual graph (Ontop).

  claims   claim handling data about ACME claims and the policy each one is on
  finance  cost centres per ACME policy
It also builds the claims narrative index (hybrid semantic search) and the `acme` inference job on default.

Both use the ACME instance IRIs, so a federated query can join claims -> policy (Postgres via Ontop)
-> cost centre (finance dataset). Safe to re-run: it replaces the data in both datasets.

  python3 scripts/seed/federation_demo.py [--store http://127.0.0.1:4321]
"""
import argparse, json, urllib.error, urllib.request

INST = 'https://myinsurancecompany.linked.data.world/d/omg-pc-database/'
CLAIMS = f'''@prefix ex: <https://claims.example.org/vocab#> .
@prefix inst: <{INST}> .
inst:Claim-1 ex:handledBy "Maria Lopez" ; ex:severity "high" ; ex:onPolicy inst:Policy-1 ;
  ex:narrative "A burst pipe flooded the basement and ruined stored furniture and electronics." .
inst:Claim-2 ex:handledBy "Chen Wei" ; ex:severity "low" ; ex:onPolicy inst:Policy-2 ;
  ex:narrative "A cracked windshield was replaced after a stone hit the car on the highway." .
# Further claims exist only in this business unit: they have no row in the Postgres source.
inst:Claim-3 ex:handledBy "Maria Lopez" ; ex:severity "high" ; ex:narrative "Kitchen fire spread to the garage and caused heavy smoke damage." .
inst:Claim-4 ex:handledBy "Chen Wei" ; ex:severity "medium" ; ex:narrative "Storm winds tore roof shingles off and rain leaked into the attic." .
inst:Claim-5 ex:handledBy "Ana Ruiz" ; ex:severity "low" ; ex:narrative "Bicycle stolen from a locked garage overnight." .
inst:Claim-6 ex:handledBy "Ana Ruiz" ; ex:severity "high" ; ex:narrative "Water leaked from the washing machine hose and damaged the floor." .
# Claims 7-9 are on the two policies that exist in Postgres; they feed the portfolio-risk inference job.
inst:Claim-7 ex:handledBy "Chen Wei" ; ex:severity "high" ; ex:onPolicy inst:Policy-2 ; ex:narrative "Hail broke several windows and dented the roof of the insured warehouse." .
inst:Claim-8 ex:handledBy "Maria Lopez" ; ex:severity "high" ; ex:onPolicy inst:Policy-1 ; ex:narrative "Jewelry was stolen from a bedroom safe during a break-in." .
inst:Claim-9 ex:handledBy "Ana Ruiz" ; ex:severity "low" ; ex:onPolicy inst:Policy-2 ; ex:narrative "Minor scratches on a delivery van were repaired at a body shop." .
'''
FINANCE = f'''@prefix fin: <https://finance.example.org/vocab#> .
@prefix inst: <{INST}> .
inst:Policy-1 fin:costCentre "CC-100" ; fin:budgetOwner "Underwriting East" .
inst:Policy-2 fin:costCentre "CC-200" ; fin:budgetOwner "Underwriting West" .
'''

def call(url, method='GET', body=None, content_type='application/json'):
    request = urllib.request.Request(url, data=body, method=method, headers={'Content-Type': content_type})
    try:
        with urllib.request.urlopen(request, timeout=60) as response: return response.status
    except urllib.error.HTTPError as error: return error.code

def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--store', default='http://127.0.0.1:4321')
    store = parser.parse_args().store
    for name, turtle in (('claims', CLAIMS), ('finance', FINANCE)):
        status = call(f'{store}/api/datasets', 'POST', json.dumps({'name': name}).encode())
        if status not in (201, 409): raise SystemExit(f'could not create {name}: HTTP {status}')
        # PUT to the default graph replaces it, so re-runs do not accumulate.
        status = call(f'{store}/d/{name}/store?default', 'PUT', turtle.encode(), 'text/turtle')
        if status not in (200, 201, 204): raise SystemExit(f'could not load {name}: HTTP {status}')
        print(f'{name}: loaded')
    # claims has a hybrid semantic index over claim narratives (deploy/local/tenants/claims/semantic.json).
    status = call(f'{store}/d/claims/api/plugins/semantic-search/operations/rebuild', 'POST', json.dumps({'index': 'narratives'}).encode())
    print(f'claims: narratives index rebuild -> HTTP {status}')
    # default: the ACME ontology on its own (so inference jobs can pair it with live Postgres rows) ...
    ontology = ('INSERT { GRAPH <urn:rdftool:examples:acme-ontology> { ?s ?p ?o } } WHERE { GRAPH <urn:rdftool:examples:acme-insurance> '
                '{ ?s ?p ?o . FILTER(STRSTARTS(STR(?s), "http://data.world/schema/insurance/")) } }')
    status = call(f'{store}/update', 'POST', ontology.encode(), 'application/sparql-update')
    print(f'default: ontology graph -> HTTP {status}')
    # ... then three inference jobs and a search index over their output (deploy/local/tenants/default/*.json).
    for job in ('acme', 'acme-live', 'portfolio-risk'):
        status = call(f'{store}/api/plugins/inference/operations/rebuild', 'POST', json.dumps({'job': job}).encode())
        print(f'default: {job} inference rebuild -> HTTP {status}')
    # Rebuild every index last: new graphs (the ontology copy, derived graphs) change what the indexes cover.
    for index in ('risk-notes', 'descriptions', 'descriptions-hybrid'):
        status = call(f'{store}/api/plugins/semantic-search/operations/rebuild', 'POST', json.dumps({'index': index}).encode())
        print(f'default: {index} index rebuild -> HTTP {status}')


main()
