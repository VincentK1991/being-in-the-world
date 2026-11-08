import assert from 'node:assert/strict';
import { buildGraph, graphView, SUBCLASS, termKey } from '../../apps/studio/src/lib/graph.ts';
const uri = value => ({type: 'uri', value});
const triple = (s, p, o) => ({s: uri(s), p: uri(p), o: uri(o)});
const rows = [triple('Elephant', SUBCLASS, 'Mammal'), triple('Whale', SUBCLASS, 'Mammal'), triple('Whale', SUBCLASS, 'Aquatic'), triple('Mammal', SUBCLASS, 'Whale'), triple('Mammal', SUBCLASS, 'Mammal'), triple('Elephant','habitat','Savanna')];
const graph = buildGraph(rows), mammal = termKey(uri('Mammal'));
assert.equal(graph.nodes.get(mammal).children.length, 2);
const collapsed = graphView(graph,rows,new Set());
assert.equal(collapsed.nodes.length,3);
assert.ok(collapsed.edges.some(edge => edge.aggregated && edge.label === 'habitat'));
const expanded = graphView(graph,rows,new Set([mammal]));
assert.equal(expanded.nodes.filter(node => node.parent === mammal).length,2);
assert.equal(expanded.nodes.find(node => node.id === termKey(uri('Whale'))).parent, mammal);
assert.equal(expanded.edges.length,4); // Additional parent, cyclic, self, habitat.
console.log('PASS: RDF-to-D3 view model hierarchy, collapse, redirected edges, multiple inheritance, cycles.');
