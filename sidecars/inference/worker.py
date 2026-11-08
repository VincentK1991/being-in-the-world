"""Declared reasoning profiles; no imports/network fetching or arbitrary custom rules.

Profiles (never changed once published; a new behaviour gets a new name):
  rdfs-v1        RDFS; rejects any OWL term.
  owlrl-core-v1  OWL 2 RL over an allowlist of OWL terms; class expressions must be named IRIs.
  rdfs-v2        RDFS; OWL terms are plain IRIs. Axiomatic triples scoped to the vocabulary in use;
                 nothing that holds of every resource (`x a rdfs:Resource`, `x a owl:Thing`).
  owlrl-full-v1  All of OWL 2 RL (as implemented by owlrl), scoped and filtered likewise.

An inconsistent input is a result, not an error: the reply carries `clashes` and no triples.
"""
import json
import re
import sys
import rdflib
from rdflib import Graph, BNode, URIRef, Literal
from owlrl import DeductiveClosure, OWLRL_Semantics, RDFS_Semantics

OWL = 'http://www.w3.org/2002/07/owl#'
VOCABULARIES = (OWL, 'http://www.w3.org/1999/02/22-rdf-syntax-ns#', 'http://www.w3.org/2000/01/rdf-schema#',
                'http://www.w3.org/2001/XMLSchema#')
ERROR = 'http://www.daml.org/2002/03/agents/agent-ont#'
ALLOWED = {'Class', 'Ontology', 'NamedIndividual', 'ObjectProperty', 'DatatypeProperty',
           'Thing', 'Nothing', 'inverseOf', 'TransitiveProperty', 'SymmetricProperty',
           'equivalentClass', 'equivalentProperty', 'sameAs', 'FunctionalProperty',
           'InverseFunctionalProperty', 'disjointWith', 'differentFrom', 'propertyDisjointWith'}
# name: (semantics, OWL terms allowed (None = any), class expressions may be blank, scoped axioms)
PROFILES = {
    'rdfs-v1': (RDFS_Semantics, set(), False, False),
    'owlrl-core-v1': (OWLRL_Semantics, ALLOWED, False, False),
    'rdfs-v2': (RDFS_Semantics, None, True, True),
    'owlrl-full-v1': (OWLRL_Semantics, None, True, True),
}
RDF_TYPE = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
SUBCLASS = 'http://www.w3.org/2000/01/rdf-schema#subClassOf'
DOMAIN_RANGE = ('http://www.w3.org/2000/01/rdf-schema#domain', 'http://www.w3.org/2000/01/rdf-schema#range')
EVERYTHING = {'http://www.w3.org/2000/01/rdf-schema#Resource', OWL + 'Thing'}

def trivial(s, p, o):
    """True of every resource, so saying it adds nothing: `x a rdfs:Resource`, `x a owl:Thing`,
    `C rdfs:subClassOf owl:Thing`, `p rdfs:domain owl:Thing`. Reflexive `owl:sameAs` stays: queries
    that walk sameAs chains rely on it (eq-ref)."""
    return str(p) in (RDF_TYPE, SUBCLASS) + DOMAIN_RANGE and str(o) in EVERYTHING

IRI = re.compile(r'(?:https?|urn|file|tag):[^\s,;()\'"<>]+')

def clash(message):
    """owlrl reports clashes as prose; keep it, and pull out the resources it names."""
    terms = [t.rstrip('.') for t in IRI.findall(message)]
    return {'message': message, 'terms': list(dict.fromkeys(terms))}

def infer(request):
    profile = request['profile']
    if profile not in PROFILES:
        raise ValueError('unsupported reasoning profile')
    semantics, allowed, blank_expressions, scoped_axioms = PROFILES[profile]
    text = request['ntriples']
    if not isinstance(text, str) or len(text.encode()) > 16 * 1024 * 1024:
        raise ValueError('source exceeds 16 MiB')
    # Preserve literal lexical forms and dataset blank-node identity on round trip.
    rdflib.NORMALIZE_LITERALS = False
    graph, mapping = Graph(), {}
    graph.parse(data=text, format='nt', bnode_context=mapping)
    if len(graph) > 100000:
        raise ValueError('source exceeds 100000 triples')
    if allowed is not None:
        for triple in graph:
            for term in triple:
                if isinstance(term, URIRef) and str(term).startswith(OWL) and str(term)[len(OWL):] not in allowed:
                    raise ValueError('unsupported OWL term: ' + str(term))
    if not blank_expressions:
        # Class/property expressions are intentionally outside the v1 profiles.
        for s, p, o in graph:
            if str(p) in (OWL+'equivalentClass', OWL+'equivalentProperty',
                          'http://www.w3.org/2000/01/rdf-schema#subClassOf',
                          'http://www.w3.org/2000/01/rdf-schema#subPropertyOf'):
                if not isinstance(s, URIRef) or not isinstance(o, URIRef):
                    raise ValueError('class/property expressions must be named IRIs')
    original = set(graph)
    used = {term for triple in original for term in triple if isinstance(term, URIRef)}
    closure = DeductiveClosure(semantics, axiomatic_triples=scoped_axioms, datatype_axioms=False)
    closure.expand(graph)
    errors = sorted(str(m) for m in graph.objects(None, URIRef(ERROR + 'error')))
    if errors:
        return {'ntriples': '', 'triples': 0, 'clashes': [clash(m) for m in errors[:100]]}
    reverse = {value: BNode(label) for label, value in mapping.items()}
    derived = Graph()
    for s, p, o in set(graph) - original:
        if isinstance(s, Literal) or str(p).startswith(ERROR):
            continue
        # Scoped axioms: say nothing about RDF/RDFS/OWL/XSD terms the input never mentions,
        # and nothing that holds of every resource.
        if scoped_axioms and (trivial(s, p, o) or
                              isinstance(s, URIRef) and str(s).startswith(VOCABULARIES) and s not in used):
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
