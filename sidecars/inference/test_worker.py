import unittest
from rdflib import Graph
from worker import infer

OWL = 'http://www.w3.org/2002/07/owl#'
RDFS = 'http://www.w3.org/2000/01/rdf-schema#'
RDF = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#'
class Rules(unittest.TestCase):
    def result(self, text, profile='owlrl-core-v1'):
        return Graph().parse(data=infer({'profile':profile, 'ntriples':text})['ntriples'], format='nt')
    def test_rdfs(self):
        g=self.result(f'<urn:A> <{RDFS}subClassOf> <urn:B> .\n<urn:x> <{RDF}type> <urn:A> .', 'rdfs-v1')
        self.assertTrue(g.query('ASK { <urn:x> a <urn:B> }').askAnswer)
    def test_equivalence_symmetry_domain_and_same_as(self):
        g=self.result(f'''<urn:A> <{OWL}equivalentClass> <urn:B> .
<urn:x> <{RDF}type> <urn:A> .
<urn:p> <{RDF}type> <{OWL}SymmetricProperty> .
<urn:p> <{RDFS}domain> <urn:D> .
<urn:x> <urn:p> <urn:y> .
<urn:x> <{OWL}sameAs> <urn:z> .''')
        for pattern in ['<urn:x> a <urn:B>', '<urn:y> <urn:p> <urn:x>', '<urn:x> a <urn:D>', '<urn:z> <urn:p> <urn:y>']:
            self.assertTrue(g.query('ASK { '+pattern+' }').askAnswer,pattern)
    def test_inconsistency_rejected(self):
        with self.assertRaisesRegex(ValueError, 'inconsistency'):
            self.result(f'<urn:x> <{OWL}sameAs> <urn:y> .\n<urn:x> <{OWL}differentFrom> <urn:y> .')
    def test_imports_and_unsupported_constructs_rejected(self):
        for term in ['imports','unionOf','someValuesFrom']:
            with self.assertRaisesRegex(ValueError,'unsupported OWL term'):
                self.result(f'<urn:x> <{OWL}{term}> <https://example.org/remote> .')
    def test_profile_and_parse_errors(self):
        with self.assertRaises(ValueError): self.result('', 'full-owl')
        with self.assertRaises(Exception): self.result('broken RDF')
    def test_blank_node_labels_and_literal_lexical_forms(self):
        value=infer({'profile':'owlrl-core-v1','ntriples':f'_:original <{RDF}type> <urn:A> .\n<urn:A> <{RDFS}subClassOf> <urn:B> .'})
        self.assertIn('_:original', value['ntriples'])
if __name__ == '__main__': unittest.main()
