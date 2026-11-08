"""Bounded declared profile; no imports/network fetching or arbitrary custom rules."""
import json
import sys
import rdflib
from rdflib import Graph, BNode, URIRef, Literal
from owlrl import DeductiveClosure, OWLRL_Semantics, RDFS_Semantics

OWL = 'http://www.w3.org/2002/07/owl#'
ALLOWED = {'Class', 'Ontology', 'NamedIndividual', 'ObjectProperty', 'DatatypeProperty',
           'Thing', 'Nothing', 'inverseOf', 'TransitiveProperty', 'SymmetricProperty',
           'equivalentClass', 'equivalentProperty', 'sameAs', 'FunctionalProperty',
           'InverseFunctionalProperty', 'disjointWith', 'differentFrom', 'propertyDisjointWith'}

def infer(request):
    profile = request['profile']
    if profile not in ('rdfs-v1', 'owlrl-core-v1'):
        raise ValueError('unsupported reasoning profile')
    text = request['ntriples']
    if not isinstance(text, str) or len(text.encode()) > 16 * 1024 * 1024:
        raise ValueError('source exceeds 16 MiB')
    # Preserve literal lexical forms and dataset blank-node identity on round trip.
    rdflib.NORMALIZE_LITERALS = False
    graph, mapping = Graph(), {}
    graph.parse(data=text, format='nt', bnode_context=mapping)
    if len(graph) > 100000:
        raise ValueError('source exceeds 100000 triples')
    for triple in graph:
        for term in triple:
            if isinstance(term, URIRef) and str(term).startswith(OWL):
                if profile == 'rdfs-v1' or str(term)[len(OWL):] not in ALLOWED:
                    raise ValueError('unsupported OWL term: ' + str(term))
    # Class/property expressions are intentionally outside this initial profile.
    for s, p, o in graph:
        if str(p) in (OWL+'equivalentClass', OWL+'equivalentProperty',
                      'http://www.w3.org/2000/01/rdf-schema#subClassOf',
                      'http://www.w3.org/2000/01/rdf-schema#subPropertyOf'):
            if not isinstance(s, URIRef) or not isinstance(o, URIRef):
                raise ValueError('class/property expressions must be named IRIs')
    original = set(graph)
    DeductiveClosure(OWLRL_Semantics if profile == 'owlrl-core-v1' else RDFS_Semantics,
                     axiomatic_triples=False, datatype_axioms=False).expand(graph)
    errors = list(graph.objects(None, URIRef('http://www.daml.org/2002/03/agents/agent-ont#error')))
    if errors:
        raise ValueError('reasoner reported inconsistency: ' + '; '.join(map(str, errors))[:2000])
    reverse = {value: BNode(label) for label, value in mapping.items()}
    derived = Graph()
    for s, p, o in set(graph) - original:
        if isinstance(s, Literal):
            continue
        derived.add(tuple(reverse.get(term, term) for term in (s, p, o)))
    if len(derived) > 200000:
        raise ValueError('derived output exceeds 200000 triples')
    result = derived.serialize(format='nt')
    if len(result.encode()) > 16 * 1024 * 1024:
        raise ValueError('derived output exceeds 16 MiB')
    return {'ntriples': result, 'triples': len(derived)}

if __name__ == '__main__':
    try:
        print(json.dumps(infer(json.load(sys.stdin))))
    except Exception as error:
        print(json.dumps({'error': str(error)}))
        sys.exit(1)
