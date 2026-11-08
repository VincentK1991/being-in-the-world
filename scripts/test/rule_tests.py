#!/usr/bin/env python3
"""Run every inference job's rule tests on a running stack; exit 1 if any fail or none exist.

Tests live in each job's `tests` list (deploy/local/tenants/<dataset>/inference.json) and run through the
job's own reasoner on the data each test supplies. Nothing is read from or written to the store.
"""
import argparse, json, sys, urllib.request

parser = argparse.ArgumentParser()
parser.add_argument('--store', default='http://127.0.0.1:4321')
parser.add_argument('--dataset', default='', help='dataset name (default dataset when empty)')
args = parser.parse_args()
prefix = f'/d/{args.dataset}' if args.dataset else ''
request = urllib.request.Request(f'{args.store}{prefix}/api/plugins/inference/operations/test', b'{}', {'Content-Type': 'application/json'})
report = json.load(urllib.request.urlopen(request, timeout=300))
total = 0
for job in report['jobs']:
    for result in job['results']:
        total += 1
        print('PASS' if result['passed'] else 'FAIL', f"{job['job']}: {result['name']}")
        if not result['passed']:
            for key in ('error', 'missing', 'present_but_forbidden', 'unexpected'):
                if result.get(key): print(f'    {key}: {result[key]}')
print(f'\n{total} rule tests, {"all passed" if report["passed"] else "FAILURES"}')
sys.exit(0 if report['passed'] and total else 1)
