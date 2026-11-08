import type { Term } from './api';
export const SUBCLASS = 'http://www.w3.org/2000/01/rdf-schema#subClassOf';
export const RDF_TYPE = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type';
export const LABEL = 'http://www.w3.org/2000/01/rdf-schema#label';
export interface Triple { s: Term; p: Term; o: Term; g?: Term }
export const termKey = (term: Term) => JSON.stringify([term.type, term.value, term.datatype, term['xml:lang']]);
export const shortName = (value: string) => value.split(/[/#:]/).filter(Boolean).at(-1) || value;
export interface GraphNode { id: string; term: Term; label: string; children: string[]; types: string[] }
export function buildGraph(triples: Triple[]) {
  const nodes = new Map<string, GraphNode>();
  for (const triple of triples) for (const term of [triple.s, triple.o]) {
    if (term.type !== 'uri' && term.type !== 'bnode') continue;
    const id = termKey(term);
    if (!nodes.has(id)) nodes.set(id, {id, term, label: shortName(term.value), children: [], types: []});
  }
  for (const t of triples) if (t.p.value === LABEL && t.o.type === 'literal') {
    const node = nodes.get(termKey(t.s));
    if (node) node.label = t.o.value;
  }
  // rdf:type IRIs per node, sorted so the primary type never depends on statement order.
  for (const t of triples) if (t.p.value === RDF_TYPE && t.o.type === 'uri') {
    const node = nodes.get(termKey(t.s));
    if (node && !node.types.includes(t.o.value)) node.types.push(t.o.value);
  }
  for (const node of nodes.values()) node.types.sort();
  const parent = new Map<string, string>();
  // One display parent per node. Keep all original edges, including multiple
  // inheritance and cycles. Never infer a class hierarchy from rdf:type.
  for (const t of triples) {
    if (t.p.value !== SUBCLASS) continue;
    const child = termKey(t.s), candidate = termKey(t.o);
    if (!nodes.has(candidate) || child === candidate || parent.has(child)) continue;
    let ancestor: string | undefined = candidate;
    while (ancestor && ancestor !== child) ancestor = parent.get(ancestor);
    if (ancestor === child) continue;
    parent.set(child, candidate);
    nodes.get(candidate)!.children.push(child);
  }
  return {nodes, parent, roots: [...nodes.keys()].filter(id => !parent.has(id))};
}
// Only RDF-to-view data mapping lives here. D3 owns coordinates, edge geometry,
// hit testing, group hulls and the physics simulation.
export interface ViewNode { id: string; iri: string; label: string; children: number; expanded: boolean; bnode: boolean; type?: string; typeLabel?: string; parent?: string }
export interface ViewEdge { id: string; source: string; target: string; label: string; predicate: string; aggregated: boolean }
export function graphView(graph: ReturnType<typeof buildGraph>, triples: Triple[], expanded: Set<string>) {
  const typeLabels = new Map([...graph.nodes.values()].map(n => [n.term.value, n.label] as const));
  const visible = new Set<string>();
  function visit(id: string, depth: number) {
    visible.add(id);
    if (expanded.has(id) && depth < 8) for (const child of graph.nodes.get(id)!.children) visit(child, depth + 1);
  }
  for (const id of graph.roots) visit(id, 0);
  const nodes: ViewNode[] = [...visible].map(id => {
    const node = graph.nodes.get(id)!;
    const parent = graph.parent.get(id);
    return {id, iri: node.term.value, label: node.label, children: node.children.length,
      expanded: expanded.has(id), bnode: node.term.type === 'bnode',
      ...(node.types[0] ? {type: node.types[0], typeLabel: typeLabels.get(node.types[0]) ?? shortName(node.types[0])} : {}),
      ...(parent && visible.has(parent) ? {parent} : {})};
  });
  function representative(id: string): string | undefined {
    while (!visible.has(id)) {
      const parent = graph.parent.get(id);
      if (!parent) return undefined;
      id = parent;
    }
    return id;
  }
  const edges: ViewEdge[] = triples.flatMap((t, index) => {
    const sourceId = termKey(t.s), targetId = termKey(t.o);
    const source = representative(sourceId), target = representative(targetId);
    if (!source || !target) return [];
    // Primary subclass edges are represented by group containment. Other edges are
    // retained, including multiple inheritance, cycles and actual self-loops.
    if (t.p.value === SUBCLASS && graph.parent.get(sourceId) === targetId) return [];
    if (source === target && sourceId !== targetId) return [];
    const aggregated = source !== sourceId || target !== targetId;
    return [{id: `edge-${index}`, source, target, label: shortName(t.p.value), predicate: t.p.value, aggregated}];
  });
  return {nodes, edges};
}

// Deterministic colour per type IRI: FNV-1a hash -> hue, with hash-derived saturation and
// lightness so neighbouring hues stay apart. Same IRI, same colour, in every session.
export function typeColor(type?: string) {
  if (!type) return 'hsl(165 12% 62%)';
  let h = 2166136261;
  for (let i = 0; i < type.length; i++) { h ^= type.charCodeAt(i); h = Math.imul(h, 16777619) >>> 0; }
  const hue = h % 360, sat = 62 + ((h >>> 9) % 3) * 12, light = 58 + ((h >>> 13) % 3) * 7;
  return `hsl(${hue} ${sat}% ${light}%)`;
}
