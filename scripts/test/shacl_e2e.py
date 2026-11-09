#!/usr/bin/env python3
"""Real HTTP/Jena checks, confined to a disposable dataset. No demo data overwritten."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

BASE = os.environ.get('RDF_BASE', 'http://127.0.0.1:7878').rstrip('/')
ROOT = Path(__file__).resolve().parents[2]


def request(path, value=None, method='POST', content_type='application/json', allow_error=False):
    body = json.dumps(value).encode() if content_type == 'application/json' else value
    req = urllib.request.Request(BASE + path, data=body, method=method,
                                 headers={'Content-Type': content_type})
    try:
        with urllib.request.urlopen(req, timeout=90) as response:
            text = response.read().decode()
            return response.status, json.loads(text) if 'json' in response.headers.get('Content-Type', '') else text
    except urllib.error.HTTPError as error:
        if not allow_error:
            raise AssertionError(error.read().decode()) from error
        return error.code, error.read().decode()


def gateway_isolation():
    name = 'shacl-test-' + uuid.uuid4().hex[:12]
    request('/api/datasets', {'name': name})
    prefix = '/d/' + name
    # A new dataset has no SHACL job by default: tenant policy does not leak.
    try:
        _, plugins = request(prefix + '/api/plugins', method='GET')
        assert all(p['id'] != 'shacl' for p in plugins)
        print('PASS deployed gateway: validation configuration remains dataset-scoped')
    finally:
        request('/api/datasets/' + name, method='DELETE')


def standalone():
    global BASE
    previous = BASE
    with tempfile.TemporaryDirectory(prefix='rdf-shacl-http-') as temp:
        temp = Path(temp)
        config = temp / 'shacl.json'
        config.write_text((ROOT / 'deploy/local/tenants/default/shacl.json').read_text())
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        BASE = f'http://127.0.0.1:{port}'
        env = {k: v for k, v in os.environ.items() if not k.startswith('RDF_')}
        env['RDF_SHACL_CONFIG'] = str(config)
        args = [str(ROOT / 'target/debug/rdftool'), 'serve', '--db', str(temp / 'db'), '--bind', f'127.0.0.1:{port}']
        log = (temp / 'server.log').open('w+')

        def start():
            process = subprocess.Popen(args, env=env, stdout=log, stderr=log)
            for _ in range(100):
                if process.poll() is not None:
                    log.seek(0)
                    raise AssertionError(log.read())
                try:
                    request('/health', method='GET')
                    return process
                except (OSError, urllib.error.URLError):
                    time.sleep(.1)
            process.terminate()
            process.wait(timeout=15)
            raise AssertionError('server did not become ready')

        def op(name, allow_error=False):
            return request('/api/plugins/shacl/operations/' + name, {'job': 'claims-example'}, allow_error=allow_error)

        def replace(graph, text):
            return request('/store?' + urllib.parse.urlencode({'graph': graph}), text,
                           method='PUT', content_type='text/turtle')

        process = start()
        try:
            shapes = (ROOT / 'plugins/shacl/examples/shapes.ttl').read_bytes()
            bad = (ROOT / 'plugins/shacl/examples/invalid.ttl').read_bytes()
            good = (ROOT / 'plugins/shacl/examples/valid.ttl').read_bytes()
            replace('urn:shacl:example:shapes', shapes)
            replace('urn:shacl:example:data', bad)
            _, report = op('validate')
            assert report['summary']['counts'] == {'violation': 2, 'warning': 1, 'info': 1}, report
            assert report['mode'] == 'report-only'
            code, error = op('check', allow_error=True)
            assert code == 400 and 'gate failed' in error and 'report saved' in error, (code, error)
            _, unchanged = request('/store?' + urllib.parse.urlencode({'graph': 'urn:shacl:example:data'}), method='GET')
            assert '-10' in unchanged
            # Malformed replacement rolls back; it cannot corrupt shapes or make a stale report current.
            code, _ = request('/store?' + urllib.parse.urlencode({'graph': 'urn:shacl:example:shapes'}),
                              b'invalid turtle', method='PUT', content_type='text/turtle', allow_error=True)
            assert code == 400
            assert op('status')[1]['state'] == 'ready'
            replace('urn:shacl:example:data', good)
            assert op('status')[1]['state'] == 'stale'
            assert op('check')[1]['summary']['conforms'] is True
            # Empty shapes fail explicitly; previous report remains, visibly stale.
            replace('urn:shacl:example:shapes', b'')
            assert op('validate', allow_error=True)[0] == 400
            assert op('status')[1]['state'] == 'stale'
            replace('urn:shacl:example:shapes', shapes)
            assert op('check')[1]['summary']['gate_passed'] is True
            process.terminate()
            process.wait(timeout=15)
            process = start()
            assert op('status')[1]['state'] == 'ready'
            _, answer = request('/query', b'PREFIX sh: <http://www.w3.org/ns/shacl#> ASK { GRAPH <urn:rdftool:derived:shacl:claims-example> { ?r a sh:ValidationReport; sh:conforms true } }', content_type='application/sparql-query')
            assert answer['boolean'] is True
            print('PASS real HTTP/Jena: report-only, failed gate, unchanged data, queryable reports, replacement rollback, stale shapes/data, empty-shapes rejection, shutdown/reopen')
        finally:
            process.terminate()
            process.wait(timeout=15)
            log.close()
            BASE = previous


if __name__ == '__main__':
    standalone()
    gateway_isolation()
