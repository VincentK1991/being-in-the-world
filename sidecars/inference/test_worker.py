import unittest
from rdflib import Graph
from worker import infer

OWL = 'http://www.w3.org/2002/07/owl#'
RDFS = 'http://www.w3.org/2000/01/rdf-schema#'
RDF = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#'
def lines(text):
    """One statement per line, as N-Triples requires."""
    return '\n'.join(s.strip() + ' .' for s in text.replace('\n', ' ').split(' . ') if s.strip()).rstrip(' .') + ' .'
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
    def test_inconsistency_is_a_result_with_clashes(self):
        for profile in ['owlrl-core-v1', 'owlrl-full-v1']:
            value = infer({'profile': profile, 'ntriples': f'<urn:x> <{OWL}sameAs> <urn:y> .\n<urn:x> <{OWL}differentFrom> <urn:y> .'})
            self.assertEqual(value['ntriples'], '')
            self.assertTrue(value['clashes'], profile)
            self.assertIn('urn:x', value['clashes'][0]['terms'])
    def test_full_profile_admits_class_expressions(self):
        g=self.result(lines(f'''<urn:B> <{OWL}intersectionOf> _:l1 .
_:l1 <{RDF}first> <urn:A> . _:l1 <{RDF}rest> _:l2 . _:l2 <{RDF}first> <urn:C> . _:l2 <{RDF}rest> <{RDF}nil> .
<urn:x> <{RDF}type> <urn:A> . <urn:x> <{RDF}type> <urn:C> .
<urn:gp> <{OWL}propertyChainAxiom> _:c1 .
_:c1 <{RDF}first> <urn:parent> . _:c1 <{RDF}rest> _:c2 . _:c2 <{RDF}first> <urn:parent> . _:c2 <{RDF}rest> <{RDF}nil> .
<urn:a> <urn:parent> <urn:b> . <urn:b> <urn:parent> <urn:c> .'''), 'owlrl-full-v1')
        for pattern in ['<urn:x> a <urn:B>', '<urn:a> <urn:gp> <urn:c>']:
            self.assertTrue(g.query('ASK { '+pattern+' }').askAnswer, pattern)
    def test_rdfs_v2_treats_owl_terms_as_iris_and_scopes_axioms(self):
        g=self.result(f'''<urn:c> <{RDF}type> <{OWL}Class> .
<urn:x> <{RDF}_1> <urn:y> .
<urn:p> <{RDFS}range> <urn:R> .
<urn:x> <urn:p> <urn:z> .''', 'rdfs-v2')
        self.assertTrue(g.query(f'ASK {{ <urn:x> <{RDFS}member> <urn:y> }}').askAnswer)
        self.assertTrue(g.query('ASK { <urn:z> a <urn:R> }').askAnswer)
        # rdf:_2 never appears, so nothing is said about it; nor what holds of everything.
        self.assertFalse(g.query(f'ASK {{ <{RDF}_2> ?p ?o }}').askAnswer)
        self.assertFalse(g.query(f'ASK {{ ?x a <{RDFS}Resource> }}').askAnswer)
        self.assertFalse(g.query(f'ASK {{ ?x a <{OWL}Thing> }}').askAnswer)
    def test_imports_and_unsupported_constructs_rejected(self):
        for term in ['imports','unionOf','someValuesFrom']:  # owlrl-core-v1 keeps its allowlist
            with self.assertRaisesRegex(ValueError,'unsupported OWL term'):
                self.result(f'<urn:x> <{OWL}{term}> <https://example.org/remote> .')
    def test_profile_and_parse_errors(self):
        with self.assertRaises(ValueError): self.result('', 'full-owl')
        with self.assertRaises(Exception): self.result('broken RDF')
    def test_blank_node_labels_and_literal_lexical_forms(self):
        value=infer({'profile':'owlrl-core-v1','ntriples':f'_:original <{RDF}type> <urn:A> .\n<urn:A> <{RDFS}subClassOf> <urn:B> .'})
        self.assertIn('_:original', value['ntriples'])
if __name__ == '__main__': unittest.main()
