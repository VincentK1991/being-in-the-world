#!/usr/bin/env python3
"""End-to-end check of the virtual graph (Postgres -> Ontop) and SERVICE federation across datasets.

Needs the stack from `just dev` plus `sidecars/federation` (Postgres, Ontop, Comunica), the ACME
Postgres seed, and the demo datasets:
  docker compose -f sidecars/federation/compose.yml up -d --build
  python3 scripts/seed/acme_postgres.py && python3 scripts/seed/federation_demo.py

  python3 scripts/test/federation_e2e.py [--store http://127.0.0.1:4321]
"""
import argparse, json, subprocess, sys, time, urllib.error, urllib.request

INS = 'http://data.world/schema/insurance/'
checks = []

def post(url, body, content_type, accept='application/sparql-results+json'):
    request = urllib.request.Request(url, body.encode(), {'Content-Type': content_type, 'Accept': accept})
    try:
        with urllib.request.urlopen(request, timeout=120) as response: return response.status, response.read().decode()
    except urllib.error.HTTPError as error: return error.code, error.read().decode()

def rows(text): return json.loads(text)['results']['bindings']
def value(row, name): return row[name]['value']

def check(name, condition, detail=''):
    checks.append((name, bool(condition)))
    print(('PASS ' if condition else 'FAIL ') + name + ('' if condition else f'  -> {detail}'))

def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--store', default='http://127.0.0.1:4321')
    store = parser.parse_args().store
    def sparql(dataset, query):
        return post(f'{store}{"/d/" + dataset if dataset else ""}/query', query, 'application/sparql-query')
    def op(dataset, plugin, operation, body):
        status, text = post(f'{store}{"/d/" + dataset if dataset else ""}/api/plugins/{plugin}/operations/{operation}', json.dumps(body), 'application/json')
        return status, (json.loads(text) if text.startswith(('{', '[')) else text)
    def federate(dataset, query):
        # The embedding sidecar rate-limits bursts (429) and the dataset server sheds load (503): retry those.
        for attempt in range(6):
            status, result = op(dataset, 'federation', 'query', {'query': query})
            if status == 200 or not any(t in str(result) for t in ('429', 'capacity exhausted', 'sidecar unavailable')): return status, result
            time.sleep(1 + attempt)
        return status, result

    # 1. Plugins appear per dataset, only where configured.
    plugins = lambda d: {p['id'] for p in json.loads(urllib.request.urlopen(f'{store}{"/d/" + d if d else ""}/api/plugins').read())}
    check('default dataset has virtual-graph and federation', {'virtual-graph', 'federation'} <= plugins(''))
    check('claims dataset has federation but no virtual-graph', 'federation' in plugins('claims') and 'virtual-graph' not in plugins('claims'))

    # 2. Virtual graph: Postgres rows through R2RML, checked against the materialised copy in the store.
    status, state = op('', 'virtual-graph', 'status', {})
    check('Ontop reports ready with triples from Postgres', status == 200 and state['state'] == 'ready' and int(state['triples']) > 100, state)
    q = f'SELECT ?p ?n WHERE {{ ?p <{INS}policyNumber> ?n }} ORDER BY ?p'
    status, virtual = op('', 'virtual-graph', 'query', {'query': q})
    _, materialised = sparql('', f'SELECT ?p ?n WHERE {{ GRAPH <urn:rdftool:examples:acme-insurance> {{ ?p <{INS}policyNumber> ?n }} }} ORDER BY ?p')
    v = [(value(r, 'p'), value(r, 'n')) for r in virtual['results']['bindings']]
    m = [(value(r, 'p'), value(r, 'n')) for r in rows(materialised)]
    check('virtual graph policies equal the materialised ACME policies', v and v == m, (v, m))
    status, refused = op('', 'virtual-graph', 'query', {'query': 'SELECT * WHERE { SERVICE <http://ontop:8080/sparql> { ?s ?p ?o } }'})
    check('virtual graph refuses SERVICE', status >= 400, refused)

    # 3. Federation across three sources: claims dataset (local) + Postgres virtual graph + finance dataset.
    query = f'''PREFIX ex: <https://claims.example.org/vocab#>
PREFIX fin: <https://finance.example.org/vocab#>
SELECT ?claim ?handler ?policyNumber ?costCentre WHERE {{
  ?claim ex:handledBy ?handler ; ex:onPolicy ?policy .
  SERVICE <http://ontop:8080/sparql> {{ ?policy <{INS}policyNumber> ?policyNumber }}
  SERVICE <http://host.docker.internal:7878/d/finance/query> {{ ?policy fin:costCentre ?costCentre }}
}} ORDER BY ?claim'''
    status, result = federate('claims', query)
    got = [(value(r, 'handler'), value(r, 'policyNumber'), value(r, 'costCentre')) for r in result['results']['bindings']] if status == 200 else result
    check('claims + Postgres + finance join through SERVICE', got == [('Maria Lopez', '31003000336', 'CC-100'), ('Chen Wei', '31003000337', 'CC-200'), ('Chen Wei', '31003000337', 'CC-200'), ('Maria Lopez', '31003000336', 'CC-100'), ('Ana Ruiz', '31003000337', 'CC-200')], got)
    check('response names the endpoints it used', status == 200 and len(result['services']) == 2, result)

    # Patterns outside SERVICE run on the selected dataset's default graph.
    status, local = federate('claims', f'''PREFIX ex: <https://claims.example.org/vocab#>
SELECT ?handler ?n WHERE {{ ?claim ex:handledBy ?handler ; ex:onPolicy ?policy . SERVICE <http://ontop:8080/sparql> {{ ?policy <{INS}policyNumber> ?n }} }} ORDER BY ?handler''')
    check('local pattern + SERVICE runs on the selected dataset', status == 200 and sorted({value(r, 'handler') for r in local['results']['bindings']}) == ['Ana Ruiz', 'Chen Wei', 'Maria Lopez'], local)

    # 4. The same query from another dataset sees that dataset's own data: isolation holds.
    status, other = federate('finance', query)
    check('finance dataset has no claims, so the same query returns nothing', status == 200 and other['results']['bindings'] == [], other)
    status, reverse = federate('finance', f'''PREFIX fin: <https://finance.example.org/vocab#>
SELECT ?policy ?owner ?handler WHERE {{ ?policy fin:budgetOwner ?owner .
  SERVICE <http://host.docker.internal:7878/d/claims/query> {{ ?claim <https://claims.example.org/vocab#onPolicy> ?policy ; <https://claims.example.org/vocab#handledBy> ?handler }} }} ORDER BY ?policy''')
    check('finance can federate back to claims', status == 200 and sorted({value(r, 'handler') for r in reverse['results']['bindings']}) == ['Ana Ruiz', 'Chen Wei', 'Maria Lopez'], reverse)

    # 5. A query only against the virtual graph, and SERVICE SILENT on a down endpoint behaves per the spec.
    status, count = federate('claims', f'SELECT (COUNT(?p) AS ?n) WHERE {{ SERVICE <http://ontop:8080/sparql> {{ ?p a <{INS}Policy> }} }}')
    check('SERVICE-only query against Ontop', status == 200 and value(count['results']['bindings'][0], 'n') == '2', count)

    # 6. Policy: nothing outside the allowlist, no variable endpoints, no updates.
    for name, bad in [('unregistered endpoint', 'SELECT * WHERE { SERVICE <http://evil.example/sparql> { ?s ?p ?o } }'),
                      ('endpoint hidden in FILTER EXISTS', 'SELECT * WHERE { ?s ?p ?o FILTER EXISTS { SERVICE <http://evil.example/sparql> { ?s ?p ?x } } }'),
                      ('variable endpoint', 'SELECT * WHERE { SERVICE ?e { ?s ?p ?o } }'),
                      ('update', 'INSERT DATA { <a:a> <a:b> <a:c> }')]:
        status, denied = federate('claims', bad)
        check(f'refused: {name}', status >= 400, denied)
    # The sidecar enforces the same list even when the plugin is bypassed.
    status, text = post('http://127.0.0.1:18095/sparql', json.dumps({'query': 'SELECT * WHERE { SERVICE <http://evil.example/sparql> { ?s ?p ?o } }'}), 'application/json')
    check('sidecar refuses unregistered endpoint on its own', status == 403, text)

    # 7. Plugins composed in one federated query (see ADR 0011).
    pre = f'PREFIX ins: <{INS}>\nPREFIX ex: <https://claims.example.org/vocab#>\nPREFIX fin: <https://finance.example.org/vocab#>\nPREFIX search: <urn:rdftool:search:>\nPREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\nPREFIX owl: <http://www.w3.org/2002/07/owl#>\n'
    local, claims, finance, ontop = ('http://host.docker.internal:7878/query', 'http://host.docker.internal:7878/d/claims/query', 'http://host.docker.internal:7878/d/finance/query', 'http://ontop:8080/sparql')
    graph = 'urn:rdftool:examples:acme-insurance'
    status, same = federate('', f'{pre}SELECT ?policy ?stored ?live WHERE {{ SERVICE <{local}> {{ SELECT ?policy ?stored WHERE {{ GRAPH <{graph}> {{ ?policy ins:policyNumber ?stored }} }} }} SERVICE <{ontop}> {{ ?policy ins:policyNumber ?live }} }}')
    pairs = [(value(r, 'stored'), value(r, 'live')) for r in same['results']['bindings']] if status == 200 else same
    check('materialized graph and live Postgres rows agree (federation + virtual graph)', pairs and all(a == b for a, b in pairs) and len(pairs) == 2, pairs)
    status, inferred = federate('', f'{pre}SELECT ?claim ?type WHERE {{ SERVICE <{ontop}> {{ ?claim ins:hasLossReserve ?amount }} SERVICE <{local}> {{ SELECT ?amount ?type WHERE {{ GRAPH <urn:rdftool:derived:inference:acme> {{ ?amount rdf:type ?type }} FILTER(?type != owl:Thing) }} }} }}')
    check('Postgres rows get types inferred by the OWL-RL job', status == 200 and {value(r, 'type') for r in inferred['results']['bindings']} == {INS + 'LossReserve'} and len(inferred['results']['bindings']) == 2, inferred)
    everything = f'''{pre}SELECT ?narrative ?number ?cc ?type WHERE {{
  SERVICE <{claims}> {{ SELECT ?claim ?narrative ?policy WHERE {{ (?claim ?p ?narrative ?score) search:hybrid ("narratives" "burst pipe flooding" 1) . ?claim ex:onPolicy ?policy }} }}
  SERVICE <{ontop}> {{ ?policy ins:policyNumber ?number . ?claim ins:hasLossReserve ?reserve }}
  SERVICE <{finance}> {{ ?policy fin:costCentre ?cc }}
  SERVICE <{local}> {{ SELECT ?reserve ?type WHERE {{ GRAPH <urn:rdftool:derived:inference:acme> {{ ?reserve rdf:type ?type }} FILTER(?type != owl:Thing) }} }} }}'''
    status, allp = federate('claims', everything)
    got = [(value(r, 'narrative')[:22], value(r, 'number'), value(r, 'cc'), value(r, 'type').rsplit('/', 1)[-1]) for r in allp['results']['bindings']] if status == 200 else allp
    check('search + Postgres + finance + inference in one query', got == [('A burst pipe flooded t', '31003000336', 'CC-100', 'LossReserve')], got)
    status, searched = federate('', f'''{pre}SELECT ?claim ?number ?cc ?score WHERE {{
  SERVICE <{claims}> {{ SELECT ?claim ?policy ?score WHERE {{ (?claim ?p ?narrative ?score) search:hybrid ("narratives" "burst pipe flooding the basement" 3) . ?claim ex:onPolicy ?policy }} }}
  SERVICE <{ontop}> {{ ?policy ins:policyNumber ?number }}
  SERVICE <{finance}> {{ ?policy fin:costCentre ?cc }} }} ORDER BY DESC(?score)''')
    top = (value(searched['results']['bindings'][0], 'number'), value(searched['results']['bindings'][0], 'cc')) if status == 200 and searched['results']['bindings'] else searched
    check('hybrid search hit joins to Postgres and finance (Composed example)', top == ('31003000336', 'CC-100'), top)
    modes = lambda method: federate('', f'{pre}SELECT ?narrative WHERE {{ SERVICE <{claims}> {{ SELECT ?narrative WHERE {{ (?claim ?p ?narrative ?score) search:{method} ("narratives" "garage" 5) }} }} }}')
    (_, keyword), (_, hybrid) = modes('lexical'), modes('hybrid')
    kw, hy = [value(r, 'narrative') for r in keyword['results']['bindings']], [value(r, 'narrative') for r in hybrid['results']['bindings']]
    check('keyword search through federation returns only exact-word matches', len(kw) == 2 and all('garage' in n for n in kw), kw)
    check('hybrid search through federation also returns meaning matches', len(hy) == 5 and set(kw) <= set(hy), hy)

    # 8. Inference with outside inputs (ADR 0012).
    inf = lambda job, operation='status': op('', 'inference', operation, {'job': job})
    rebuilt = {job: inf(job, 'rebuild') for job in ('acme', 'acme-live', 'portfolio-risk')}
    check('jobs with inputs rebuild and record what they read', all(st == 200 for st, _ in rebuilt.values()) and rebuilt['acme-live'][1]['metadata']['inputs'][0]['triples'] > 100 and rebuilt['portfolio-risk'][1]['metadata']['inputs'][0]['kind'] == 'federation', rebuilt)
    refreshed = rebuilt['portfolio-risk'][1].get('refreshed', [])
    _, notes_index = op('', 'semantic-search', 'status', {'index': 'risk-notes'})
    check('rebuilding the job refreshes the search index over its output', len(refreshed) == 1 and refreshed[0]['ok'] and notes_index['state'] == 'ready', (refreshed, notes_index))
    guard = 'FILTER(STRSTARTS(STR(?s), "https://myinsurancecompany"))'
    types = lambda graph: {(value(r, 's'), value(r, 't')) for r in rows(sparql('', f'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> SELECT ?s ?t WHERE {{ GRAPH <{graph}> {{ ?s rdf:type ?t {guard} }} }}')[1])}
    virtual_types = lambda: {(value(r, 's'), value(r, 't')) for r in op('', 'virtual-graph', 'query', {'query': f'SELECT ?s ?t WHERE {{ ?s a ?t {guard} }}'})[1]['results']['bindings']}
    # Ontop now reasons over the ontology itself, so the live job only adds what Ontop did not already entail.
    # The fair comparison is closure against closure: what is visible live vs what is visible in the stored copy.
    live_derived, stored_derived = types('urn:rdftool:derived:inference:acme-live'), types('urn:rdftool:derived:inference:acme')
    ontop_types, asserted_types = virtual_types(), types('urn:rdftool:examples:acme-insurance')
    live, stored = live_derived | ontop_types, stored_derived | asserted_types
    # ontology-local.ttl adds Reserve/Payment superclasses that only Ontop knows; the stored copy uses the upstream ontology.
    live = {(s_, t) for s_, t in live if t not in (INS + 'Reserve', INS + 'Payment')}
    _, reserves = op('', 'virtual-graph', 'query', {'query': 'PREFIX ins: <http://data.world/schema/insurance/> SELECT ?x WHERE { ?x a ins:LossReserve }'})
    check('Ontop infers LossReserve from the ontology at query time, with nothing stored', len(reserves['results']['bindings']) == 2, reserves)
    ins = 'PREFIX ins: <http://data.world/schema/insurance/> '
    vq = lambda q: op('', 'virtual-graph', 'query', {'query': ins + q})[1]['results']['bindings']
    reserves_up = {value(r, 'x') for r in vq('SELECT ?x WHERE { ?x a ins:Reserve }')}
    check('Ontop applies subClassOf: loss and expense reserves are both Reserves', len(reserves_up) == 4 and {value(r, 'x') for r in vq('SELECT ?x WHERE { ?x a ins:LossReserve }')} < reserves_up, reserves_up)
    check('Ontop applies subClassOf to payments without a job', len({value(r, 'x') for r in vq('SELECT ?x WHERE { ?x a ins:Payment }')}) == 4)
    check('Ontop applies subPropertyOf: hasLossReserve and hasExpenseReserve answer hasReserve', len(vq('SELECT ?c ?a WHERE { ?c ins:hasReserve ?a }')) == 4)
    check('Ontop reasoning plus OWL-RL over live rows equals OWL-RL over the stored copy', live and live == stored, (len(live), len(stored), sorted(live ^ stored)[:5]))
    check('OWL-RL adds only what Ontop did not already entail', live_derived and not (live_derived & ontop_types), sorted(live_derived & ontop_types)[:3])
    risk = {(value(r, 'policy').rsplit('/', 1)[-1], value(r, 'n')) for r in rows(sparql('', 'PREFIX rsk: <https://risk.example.org/vocab#> SELECT ?policy ?n WHERE { GRAPH <urn:rdftool:derived:inference:portfolio-risk> { ?policy rsk:highSeverityClaims ?n } }')[1])}
    check('rules over Postgres + claims + finance derive per-policy risk', risk == {('Policy-1', '2'), ('Policy-2', '1')}, risk)
    _, notes = federate('', f'''{pre}SELECT ?number ?note ?score WHERE {{
  SERVICE <{local}> {{ SELECT ?policy ?note ?score WHERE {{ (?policy ?p ?note ?score) search:hybrid ("risk-notes" "jewelry stolen in a break-in" 3) }} }}
  SERVICE <{ontop}> {{ ?policy ins:policyNumber ?number }} }} ORDER BY DESC(?score)''')
    first = notes['results']['bindings'][0] if isinstance(notes, dict) and notes['results']['bindings'] else notes
    check('hybrid search over the inferred notes finds the theft claim first, joined to Postgres', isinstance(first, dict) and value(first, 'number') == '31003000336' and 'Jewelry' in value(first, 'note'), first)

    _, fed = op('', 'federation', 'status', {})
    check('dataset and sidecar allowlists agree', fed['state'] == 'ready' and fed['missing_in_sidecar'] == [], fed)
    # Discovery: a query (or a federation engine) can ask each dataset which plugins it has.
    catalog = lambda d: {(value(r, 'plugin'), value(r, 'kind'), value(r, 'name')) for r in rows(sparql(d, 'SELECT ?plugin ?kind ?name WHERE { (?plugin ?kind ?name) <urn:rdftool:plugins:provides> () }')[1])}
    here, claims_catalog, finance_catalog = catalog(''), catalog('claims'), catalog('finance')
    check('catalog lists the default dataset\'s virtual graph and jobs', ('virtual-graph', 'virtual-graph', 'acme-postgres') in here and ('inference', 'job', 'acme-live') in here, sorted(here)[:5])
    check('catalog differs by dataset: claims has its own index and no virtual graph', ('semantic-search', 'index', 'narratives') in claims_catalog and not any(k == 'virtual-graph' for _, k, _ in claims_catalog), sorted(claims_catalog)[:5])
    check('catalog reports the other dataset\'s indexes', ('semantic-search', 'index', 'descriptions-hybrid') in finance_catalog and ('semantic-search', 'index', 'narratives') not in finance_catalog)
    desc = urllib.request.urlopen(f'{store}/d/claims/query').read().decode()
    check('Service Description lists plugins and configured offerings', 'sd:Service' in desc and 'rdftool:index "narratives"' in desc and 'rdftool:propertyFunction <urn:rdftool:plugins:provides>' in desc, desc[:300])
    plugins_pre = 'PREFIX plugins: <urn:rdftool:plugins:>\nPREFIX search: <urn:rdftool:search:>\n'
    claims_url, finance_url = 'http://host.docker.internal:7878/d/claims/query', 'http://host.docker.internal:7878/d/finance/query'
    _, across = federate('', f'''{plugins_pre}SELECT ?dataset ?name WHERE {{
  {{ BIND("claims" AS ?dataset) SERVICE <{claims_url}> {{ (?plugin ?kind ?name) plugins:provides () }} }}
  UNION {{ BIND("finance" AS ?dataset) SERVICE <{finance_url}> {{ (?plugin ?kind ?name) plugins:provides () }} }}
  FILTER(?kind = "index") }}''')
    pairs = {(value(r, 'dataset'), value(r, 'name')) for r in across['results']['bindings']} if isinstance(across, dict) else across
    check('SERVICE asks each remote dataset which plugins it has', pairs == {('claims', 'narratives'), ('finance', 'descriptions'), ('finance', 'descriptions-hybrid')}, pairs)
    def guarded(index):
        return federate('', f'''{plugins_pre}SELECT ?claim WHERE {{
  SERVICE <{claims_url}> {{ SELECT ?plugin WHERE {{ (?plugin ?kind ?name) plugins:provides () FILTER(?kind = "index" && ?name = "{index}") }} }}
  SERVICE <{claims_url}> {{ SELECT ?claim WHERE {{ (?claim ?p ?t ?score) search:hybrid ("narratives" "burst pipe flooding" 2) }} }} }}''')
    present, absent = guarded('narratives'), guarded('missing')
    check('search runs only when the remote dataset advertises the index', present[0] == 200 and len(present[1]['results']['bindings']) == 2 and absent[0] == 200 and absent[1]['results']['bindings'] == [], (present, absent))

    # Reliability: queries that used to fail or silently return nothing.
    _, bare = federate('', 'PREFIX ins: <http://data.world/schema/insurance/> SELECT ?n WHERE { SERVICE <http://host.docker.internal:7878/query> { GRAPH <urn:rdftool:examples:acme-insurance> { ?p ins:policyNumber ?n } } }')
    check('a bare GRAPH block inside SERVICE returns rows', isinstance(bare, dict) and len(bare['results']['bindings']) == 2, bare)
    _, filtered = federate('', f'{plugins_pre}SELECT ?name WHERE {{ SERVICE <{claims_url}> {{ (?p ?kind ?name) plugins:provides () }} FILTER(?kind != "operation" && ?kind != "property-function" && ?kind != "plugin") }}')
    check('richer filters (!=, ||, IN) work on catalog rows through SERVICE', isinstance(filtered, dict) and sorted(value(r, 'name') for r in filtered['results']['bindings'])[:2] == ['narratives', 'totals'], filtered)
    check('federation results carry no process-wide counters', isinstance(filtered, dict) and 'httpRequests' not in json.dumps(filtered), filtered)

    # Rule tests: every job's tests run through its own reasoner and none touches the store.
    quads_before = rows(sparql('', 'SELECT (COUNT(*) AS ?n) WHERE { { ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } } }')[1])[0]['n']['value']
    status, tests = op('', 'inference', 'test', {})
    check('all rule tests pass (SPARQL rules, Jena rules and OWL-RL)', status == 200 and tests['passed'] and sum(j['tests'] for j in tests['jobs']) >= 9, [r for j in tests['jobs'] for r in j['results'] if not r['passed']] if status == 200 else tests)
    quads_after = rows(sparql('', 'SELECT (COUNT(*) AS ?n) WHERE { { ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } } }')[1])[0]['n']['value']
    check('running rule tests wrote nothing to the store', quads_before == quads_after, (quads_before, quads_after))

    # Chained jobs: stage 2 reads stage 1's deductions, stage 3 reads stage 2's.
    chain = ('acme', 'reserved-claims', 'reserve-summary')
    built = op('', 'inference', 'rebuild', {'job': 'reserve-summary', 'cascade': 'upstream'})[1]
    check('cascade rebuilds the upstream jobs a job reads, in order', [s['job'] for s in built['cascade']['steps']] == ['acme', 'reserved-claims'] and built['state'] == 'ready', built.get('cascade'))
    summary = {(value(r, 'p'), value(r, 'o')) for r in rows(sparql('', 'SELECT ?p ?o WHERE { GRAPH <urn:rdftool:derived:inference:reserve-summary> { ?s ?p ?o } }')[1])}
    check('stage 3 counts claims whose reserve type only stage 1 could infer', summary == {('https://risk.example.org/vocab#reservedClaims', '2')}, summary)
    check('all three stages are ready after a cascade', all(inf(j)[1]['state'] == 'ready' for j in chain))
    lineage = op('', 'inference', 'lineage', {'job': 'reserved-claims'})[1]
    check('lineage lists both directions and the rebuild order', lineage['rebuild_order'] == list(chain) and lineage['downstream'][0]['job'] == 'reserve-summary', lineage)
    marker = 'INSERT DATA { GRAPH <urn:rdftool:examples:acme-insurance> { <urn:e2e:marker> <urn:e2e:p> 1 } }'
    unmark = 'DELETE DATA { GRAPH <urn:rdftool:examples:acme-insurance> { <urn:e2e:marker> <urn:e2e:p> 1 } }'
    try:
        post(f'{store}/update', marker, 'application/sparql-update')
        check('changing the data marks every stage stale', all(inf(j)[1]['state'] == 'stale' for j in chain), [inf(j)[1]['state'] for j in chain])
        cascade = op('', 'inference', 'rebuild', {'job': 'acme', 'cascade': 'downstream'})[1]
        check('downstream cascade rebuilds the jobs that read it', [s['job'] for s in cascade['cascade']['steps']] == ['reserved-claims', 'reserve-summary'] and all(s['action'] == 'rebuilt' for s in cascade['cascade']['steps']), cascade.get('cascade'))
        check('and a second cascade skips them because nothing changed', [s['action'] for s in op('', 'inference', 'rebuild', {'job': 'acme', 'cascade': 'downstream'})[1]['cascade']['steps']] == ['skipped', 'skipped'])
    finally:
        post(f'{store}/update', unmark, 'application/sparql-update')
        op('', 'inference', 'rebuild', {'job': 'reserve-summary', 'cascade': 'both'})
    check('stages are ready again after the data is restored', all(inf(j)[1]['state'] == 'ready' for j in chain))
    # Full OWL 2 RL: an inconsistency is a result. The job keeps its output and names the clash.
    full = inf('acme-full', 'rebuild')[1]
    check('the full OWL 2 RL profile rebuilds over the ACME data', full.get('state') == 'ready', full)
    count = lambda: int(value(rows(sparql('', 'SELECT (COUNT(*) AS ?n) WHERE { GRAPH <urn:rdftool:derived:inference:acme-full> { ?s ?p ?o } }')[1])[0], 'n'))
    before = count()
    clash = 'INSERT DATA { GRAPH <urn:rdftool:examples:acme-insurance> { <urn:e2e:Open> <http://www.w3.org/2002/07/owl#disjointWith> <urn:e2e:Closed> . <urn:e2e:claim> a <urn:e2e:Open> , <urn:e2e:Closed> } }'
    try:
        post(f'{store}/update', clash, 'application/sparql-update')
        result = inf('acme-full', 'rebuild')[1]
        check('a clash makes the rebuild report inconsistent, naming the resource', result.get('state') == 'inconsistent' and any('urn:e2e:claim' in c['terms'] for c in result.get('clashes', [])), result)
        status = inf('acme-full')[1]
        check('status reads inconsistent and the last good output stays published', status['state'] == 'inconsistent' and status.get('clashes') and count() == before, (status.get('state'), count(), before))
    finally:
        post(f'{store}/update', clash.replace('INSERT DATA', 'DELETE DATA'), 'application/sparql-update')
    check('removing the clash and rebuilding makes the job ready again', inf('acme-full', 'rebuild')[1].get('state') == 'ready' and 'clashes' not in inf('acme-full')[1])

    # Explanations: why a triple was deduced, traced through chained jobs and outside inputs.
    base = 'https://myinsurancecompany.linked.data.world/d/omg-pc-database/'
    explain = lambda triple, **extra: op('', 'inference', 'explain', {'triple': triple, **extra})
    status, why = explain(f'<{base}Policy-1> <https://risk.example.org/vocab#reservedClaims> "2"^^<http://www.w3.org/2001/XMLSchema#integer> .')
    top = why['explanations'][0] if status == 200 else {}
    derivation = top.get('derivations', [{}])[0]
    check('explain names the rule, that it was an aggregate, and the facts it counted', status == 200 and top.get('job') == 'reserve-summary' and derivation.get('aggregate') and any('hasLossReserve' in p['triple'] or 'ClaimAmount' in p['triple'] for p in derivation['premises']) and len(derivation['premises']) >= 4, why if status != 200 else derivation.get('bindings'))
    middle = [e for p in derivation.get('premises', []) for e in p.get('deduced_by', [])]
    check('explain follows a premise into the earlier job that deduced it', middle and middle[0]['job'] == 'reserved-claims' and middle[0]['supported'], middle[:1])
    deepest = [e for m in middle for p in m.get('derivations', [{}])[0].get('premises', []) for e in p.get('deduced_by', [])]
    check('and says plainly when a reasoner (OWL-RL) records no justification', deepest and deepest[0]['job'] == 'acme' and deepest[0]['supported'] is False, deepest[:1])
    status, why = explain(f'<{base}Policy-2> <https://risk.example.org/vocab#atRisk> "true"^^<http://www.w3.org/2001/XMLSchema#boolean> .')
    premises = why['explanations'][0]['derivations'][0]['premises'] if status == 200 else []
    check('explain traces facts to the outside input they came from (claims joined with Postgres and finance)', status == 200 and premises and all(p.get('outside_input') == ['portfolio'] for p in premises), premises)
    status, plain = explain(f'<{base}Policy-1> <http://data.world/schema/insurance/policyNumber> "31003000336" .')
    check('explain reports an asserted triple as not derived', status == 200 and plain.get('derived') is False, plain)

    # Staleness follows the outside data: change a Postgres row and the job reports it, then rebuilds clean.
    def postgres(sql):
        compose = ['bash', 'scripts/docker-compose.sh', '-f', 'sidecars/federation/compose.yml', 'exec', '-T', 'postgres', 'psql', '-U', 'rdf', '-d', 'acme', '-c', sql]
        run = subprocess.run(compose, stdin=subprocess.DEVNULL, capture_output=True, text=True, cwd=str(__import__('pathlib').Path(__file__).resolve().parents[2]))
        return run.returncode == 0
    check('acme-live is ready before the source changes', inf('acme-live')[1]['state'] == 'ready', inf('acme-live'))
    shift = lambda sign: postgres(f"UPDATE policy SET expiration_date = expiration_date {sign} interval '1 day' WHERE policy_identifier = 1")
    try:
        check('Postgres update applied', shift('+'))
        stale = inf('acme-live')[1]
        check('changing a Postgres row makes the job stale and names the input', stale['state'] == 'stale' and stale.get('stale_inputs') == ['postgres'], stale)
        check('rebuild picks up the change', inf('acme-live', 'rebuild')[1]['state'] == 'ready' and inf('acme-live')[1]['state'] == 'ready')
    finally:
        shift('-')
        inf('acme-live', 'rebuild')
    check('restoring the row and rebuilding leaves the job ready', inf('acme-live')[1]['state'] == 'ready')
    for index in ('risk-notes', 'descriptions', 'descriptions-hybrid'): op('', 'semantic-search', 'rebuild', {'index': index})

    failed = [n for n, ok in checks if not ok]
    print(f'\n{len(checks) - len(failed)}/{len(checks)} checks passed')
    sys.exit(1 if failed else 0)

main()
