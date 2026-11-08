#!/usr/bin/env python3
"""Run a W3C conformance suite and hold it to a committed baseline.

  baseline.py rdf|sparql --engine oxigraph|database [--update]
  baseline.py entailment --reasoner owlrl|jena-rdfs [--update]   (needs `just inference` sidecars)

The baseline (tests/conformance/<suite>-<engine>.json) lists every case that does not pass, with
its status and a cause. A run fails when a case gets worse (a regression) and also when a listed
case now passes, so the baseline only ever shrinks on purpose. `--update` rewrites it, keeping
causes already written and classifying new entries by rule (or "unclassified").

Needs the rdf-tests checkout and OWL 2 export made by scripts/conformance.sh and a built
target/debug/rdftool.
"""
import argparse, hashlib, json, pathlib, re, subprocess, sys
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parents[2]
SUITE_ROOT = ROOT / 'target/w3c-rdf-tests'
RDFTOOL = ROOT / 'target/debug/rdftool'
OWL2 = ROOT / 'target/owl2-tests/profile-RL.rdf'
SUITES = {
    # RDF 1.1 syntaxes, including RDF/XML (not run by the per-format loop in conformance.sh).
    'rdf': [f'rdf/rdf11/{name}/manifest.ttl' for name in ('rdf-n-triples', 'rdf-n-quads', 'rdf-turtle', 'rdf-trig', 'rdf-xml')],
    # Every sub-manifest of SPARQL 1.0 and 1.1; the top-level manifests point at published URLs.
    'sparql': ['sparql/sparql10/*/manifest.ttl', 'sparql/sparql11/*/manifest.ttl'],
    # RDF 1.1 Semantics, SPARQL 1.1 entailment regimes, and the OWL 2 test cases in the RL profile.
    'entailment': ['rdf/rdf11/rdf-mt/manifest.ttl', 'sparql/sparql11/entailment/manifest.ttl', OWL2],
}
SUITE = None
REASONERS = {'owlrl': 'http://127.0.0.1:18082', 'jena-rdfs': 'http://127.0.0.1:18083'}
RULES = [
    (lambda c: c['status'] == 'unsupported' and re.match(r'(Protocol|GraphStoreProtocol|ServiceDescription)Test', c['detail']), 'needs-http-runner'),
    (lambda c: c['status'] == 'unsupported' and 'entailment regime' in c['detail'] and SUITE == 'sparql', 'needs-reasoner'),
    (lambda c: c['status'] == 'value-only', 'oxigraph-canonical-lexical-forms'),
    (lambda c: 'xml-canon' in c['id'], 'oxigraph-no-xml-literal-canonicalisation'),
    (lambda c: re.search(r'(SERVICE|LOAD) is disabled in the local-only profile', c['detail']), 'policy-local-only'),
    (lambda c: re.search(r'The service <[^>]*> is not supported|service name is unbound', c['detail']), 'no-remote-endpoint'),
    (lambda c: 'outside the reasoner profile' in c['detail'], 'outside-declared-profile'),
    (lambda c: re.search(r'needs a reasoner for RDFS|no matching profile|reasoner offers no', c['detail']), 'regime-not-offered'),
    (lambda c: 'Direct Semantics only' in c['detail'], 'owl-direct-semantics-only'),
    (lambda c: 'functional syntax only' in c['detail'], 'owl-functional-syntax-only'),
    (lambda c: 'owl:imports' in c['detail'], 'owl-imports'),
    (lambda c: c['status'] == 'failed' and 'recognized datatypes' in c['detail'], 'datatype-entailment-not-in-profile'),
]


def pinned(name):
    match = re.search(rf'^{name}=(\w+)', (ROOT / 'scripts/conformance.sh').read_text(), re.M)
    if not match:
        sys.exit(f'scripts/conformance.sh pins no {name}')
    return match.group(1)


def pinned_revision():
    return pinned('revision')


def pinned_owl2():
    return pinned('owl2_sha256')


def run(suite, options):
    cases = []
    for pattern in SUITES[suite]:
        manifests = [pattern] if isinstance(pattern, pathlib.Path) else sorted(SUITE_ROOT.glob(pattern))
        for manifest in manifests:
            out = subprocess.run([str(RDFTOOL), 'test-suite', '--manifest', str(manifest), *options],
                                 capture_output=True, text=True, timeout=900)
            try:
                report = json.loads(out.stdout)
            except json.JSONDecodeError:
                sys.exit(f'{manifest.relative_to(SUITE_ROOT)}: runner error: {out.stderr.strip()[:300]}')
            cases += report['cases']
    return cases


def short(case_id):
    return re.sub(r'^<|>$', '', case_id).split('/tests/', 1)[-1].replace('data-sparql11/', 'sparql11/').replace('http://owl.semanticweb.org/id/', 'owl2/')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('suite', choices=SUITES)
    parser.add_argument('--engine', default='oxigraph', choices=['oxigraph', 'database'])
    parser.add_argument('--reasoner', choices=REASONERS)
    parser.add_argument('--update', action='store_true')
    args = parser.parse_args()
    global SUITE
    SUITE = args.suite
    revision = subprocess.run(['git', '-C', str(SUITE_ROOT), 'rev-parse', 'HEAD'], capture_output=True, text=True).stdout.strip()
    if revision != pinned_revision():
        sys.exit(f'rdf-tests checkout is at {revision or "nothing"}, expected {pinned_revision()}: run scripts/conformance.sh')
    if args.suite == 'entailment':
        if not args.reasoner:
            sys.exit('entailment needs --reasoner')
        digest = hashlib.sha256(OWL2.read_bytes()).hexdigest() if OWL2.exists() else 'missing'
        if digest != pinned_owl2():
            sys.exit(f'{OWL2.relative_to(ROOT)} is {digest}, expected {pinned_owl2()}: run scripts/conformance.sh')
        variant, options = args.reasoner, ['--engine', args.engine, '--reasoner', f'{args.reasoner}={REASONERS[args.reasoner]}']
    else:
        variant, options = args.engine, ['--engine', args.engine]
    path = ROOT / f'tests/conformance/{args.suite}-{variant}.json'
    baseline = json.loads(path.read_text()) if path.exists() else {'expected': {}}
    cases = run(args.suite, options)
    totals = Counter(c['status'] for c in cases)
    print(f"{args.suite} via {variant}: {len(cases)} cases, " + ', '.join(f'{k} {v}' for k, v in sorted(totals.items())))

    if args.update:
        expected = {}
        for c in cases:
            if c['status'] == 'passed':
                continue
            old = baseline['expected'].get(c['id'], {})
            cause = old.get('cause') if old.get('cause') not in (None, 'unclassified') else next((name for rule, name in RULES if rule(c)), 'unclassified')
            expected[c['id']] = {'status': c['status'], 'cause': cause, 'detail': c['detail'][:200]}
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({'suite': args.suite, 'variant': variant, 'rdf_tests_revision': revision,
                                    'totals': dict(sorted(totals.items())), 'expected': dict(sorted(expected.items()))},
                                   indent=1, ensure_ascii=False) + '\n')
        print(f'wrote {path.relative_to(ROOT)}: {len(expected)} expected non-passing cases')
        for cause, n in Counter(e['cause'] for e in expected.values()).most_common():
            print(f'  {n:4} {cause}')
        return

    seen, worse, better = set(), [], []
    for c in cases:
        seen.add(c['id'])
        want = baseline['expected'].get(c['id'], {}).get('status', 'passed')
        if c['status'] != want:
            (better if c['status'] == 'passed' else worse).append((c, want))
    gone = [i for i in baseline['expected'] if i not in seen]
    for c, want in worse:
        print(f"REGRESSION {short(c['id'])}: expected {want}, got {c['status']}: {c['detail'][:200]}")
    for c, want in better:
        print(f"FIXED      {short(c['id'])}: was {want}, now passes: remove it with --update")
    for i in gone:
        print(f'MISSING    {short(i)}: in the baseline but not in the suite')
    if worse or better or gone:
        sys.exit(1)
    print(f"matches baseline ({len(baseline['expected'])} known non-passing cases)")


main()
