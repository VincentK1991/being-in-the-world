"""Explicit live tests: start the Jena sidecar first, then run this script."""
import json
import unittest
from urllib.request import Request,urlopen
from urllib.error import HTTPError
URL='http://127.0.0.1:18083/infer'
def request(rules,source='<urn:s> <urn:p> <urn:o> .\n',profile='jena-forward-v1'):
    data=json.dumps({'profile':profile,'rules':rules,'ntriples':source}).encode()
    try:
        with urlopen(Request(URL,data=data,headers={'Content-Type':'application/json'}),timeout=70) as response:
            return response.status,json.load(response)
    except HTTPError as error:
        return error.code,json.load(error)
class JenaTests(unittest.TestCase):
    def test_recursive_closure(self):
        status,body=request(['[reach: (?a <urn:edge> ?b) (?b <urn:edge> ?c) -> (?a <urn:edge> ?c)]'], '<urn:a> <urn:edge> <urn:b> .\n<urn:b> <urn:edge> <urn:c> .\n<urn:c> <urn:edge> <urn:d> .\n')
        self.assertEqual(status,200,body)
        self.assertIn('<urn:a> <urn:edge> <urn:d>',body['ntriples'])
        self.assertNotIn('<urn:a> <urn:edge> <urn:b>',body['ntriples'])
    def test_literal_and_blank_node_identity(self):
        status,body=request(['[copy: (?s <urn:p> ?o) -> (?s <urn:q> ?o)]'], '_:original <urn:p> "bonjour"@fr .\n<urn:number> <urn:p> "007"^^<http://www.w3.org/2001/XMLSchema#integer> .\n')
        self.assertEqual(status,200,body)
        self.assertIn('_:original <urn:q> "bonjour"@fr',body['ntriples'])
        self.assertIn('"007"',body['ntriples'])
    def test_unsafe_or_unsupported_rules_fail(self):
        for rule in ['@include <http://127.0.0.1:1/remote>','[bad: (?s <urn:p> ?o) -> (?unbound <urn:q> ?o)]','[bad: (?s <urn:p> ?o) -> print(?s)]','[bad: (?s <urn:p> ?o) noValue(?s,<urn:q>) -> (?s <urn:q> ?o)]','[bad: (?s <urn:q> ?o) <- (?s <urn:p> ?o)]','broken syntax']:
            with self.subTest(rule=rule): self.assertEqual(request([rule])[0],422)
    def test_invalid_profile_and_empty_rules(self):
        self.assertEqual(request([])[0],422)
        self.assertEqual(request(['[r: (?s <urn:p> ?o) -> (?s <urn:q> ?o)]'],profile='unknown')[0],422)
if __name__=='__main__': unittest.main()
