import { useEffect, useMemo, useRef, useState } from 'react';
import { checked, getStatus, importFile, queryStore, type Status } from '../lib/api';
import { buildGraph, graphView, shortName, termKey, typeColor, type Triple } from '../lib/graph';

import AppSidebar from './AppSidebar';
import D3Graph, { type GraphControls } from './D3Graph';

const CAP = 600, DRAWER = 400;
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
// RDF graph names originate in the engine; preserve their lexical form safely.
const iri = (value: string) => `<${value.replace(/[<>"{}|^`\\\x00-\x20]/g, c => `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}`)}>`;
export default function GraphViewer() {
  const [status, setStatus] = useState<Status | null>(null);
  const [triples, setTriples] = useState<Triple[]>([]), [scope, setScope] = useState('all');
  const [expanded, setExpanded] = useState(new Set<string>()), [selected, setSelected] = useState('');
  const [busy, setBusy] = useState(false), [error, setError] = useState(''), [truncated, setTruncated] = useState(false);
  const [search, setSearch] = useState(''), [zoom, setZoom] = useState(1), [revision, setRevision] = useState(0);
  const [uploading, setUploading] = useState(false);
  const request = useRef(0), canvas = useRef<GraphControls>(null);
  const [controlsOpen, setControlsOpen] = useState(true), [fade, setFade] = useState(1);
  useEffect(() => {
    try {
      const saved = localStorage.getItem('rdf-studio:controls'); setControlsOpen(saved ? saved === 'open' : window.innerWidth >= 768);
      const strength = Number(localStorage.getItem('rdf-studio:fade'));
      if (localStorage.getItem('rdf-studio:fade') !== null && Number.isFinite(strength)) setFade(Math.min(2, Math.max(0, strength)));
    } catch { /* storage unavailable */ }
  }, []);
  function changeFade(value: number) {
    setFade(value);
    try { localStorage.setItem('rdf-studio:fade', String(value)); } catch { /* storage unavailable */ }
  }
  function toggleControls() {
    setControlsOpen(open => { try { localStorage.setItem('rdf-studio:controls', open ? 'closed' : 'open'); } catch { /* storage unavailable */ } return !open; });
  }
  useEffect(() => {
    const generation = ++request.current;
    setBusy(true); setError(''); setSelected(''); setTriples([]); setExpanded(new Set());
    const pattern = scope === 'all'
      ? '{ ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } FILTER(!STRENDS(STR(?g), ":metadata") && !STRENDS(STR(?g), ":provenance")) }'
      : scope === 'default' ? '?s ?p ?o' : `GRAPH ${iri(scope)} { ?s ?p ?o } BIND(${iri(scope)} AS ?g)`;
    void Promise.all([getStatus(), queryStore(`SELECT ?s ?p ?o ?g WHERE { ${pattern} } ORDER BY ?s ?p ?o ?g LIMIT ${CAP + 1}`)])
      .then(([nextStatus, result]) => {
        if (generation !== request.current) return;
        const rows = result.data?.results?.bindings;
        if (!rows) throw new Error('The store did not return graph data.');
        setStatus(nextStatus); setTruncated(rows.length > CAP);
        setTriples(rows.slice(0, CAP).map(row => ({s: row.s, p: row.p, o: row.o, g: row.g})));
      }).catch(e => { if (generation === request.current) setError(errorText(e)); })
      .finally(() => { if (generation === request.current) setBusy(false); });
    return () => { ++request.current; };
  }, [scope, revision]);
  const graph = useMemo(() => buildGraph(triples), [triples]);
  const view = useMemo(() => graphView(graph, triples, expanded), [graph, triples, expanded]);
  const node = graph.nodes.get(selected);
  const properties = triples.filter(t => termKey(t.s) === selected);
  const incoming = triples.filter(t => termKey(t.o) === selected);
  const matches = [...graph.nodes.values()].filter(n => `${n.label} ${n.term.value}`.toLowerCase().includes(search.toLowerCase())).slice(0, 15);
  useEffect(() => {
    if (!selected) return;
    const close = (e: KeyboardEvent) => { if (e.key === 'Escape') setSelected(''); };
    window.addEventListener('keydown', close); return () => window.removeEventListener('keydown', close);
  }, [selected]);
  function pick(id: string) {
    if (!selected) setTimeout(() => canvas.current?.focus(id), 80);
    setSelected(id);
    if (graph.nodes.get(id)?.children.length) setExpanded(previous => {
      const next = new Set(previous); if (next.has(id)) next.delete(id); else next.add(id); return next;
    });
  }
  function reveal(id: string) {
    setSelected(id); setExpanded(previous => {
      const next = new Set(previous); let parent = graph.parent.get(id);
      while (parent) { next.add(parent); parent = graph.parent.get(parent); }
      return next;
    });
    setSearch('');
    setTimeout(() => canvas.current?.focus(id), 350);
  }
  async function upload(file?: File) {
    if (!file) return;
    setUploading(true); setError('');
    try { await importFile(file, '', status?.max_upload_bytes ?? 16 * 1024 * 1024); setRevision(r => r + 1); }
    catch (e) { setError(errorText(e)); }
    finally { setUploading(false); }
  }
  async function animalExample() {
    setUploading(true); setError('');
    try {
      const response = await checked(await fetch('/animals.ttl'));
      const target = 'urn:rdftool:examples:animals';
      await importFile(new File([await response.text()], 'animals.ttl'), target, status?.max_upload_bytes ?? 16 * 1024 * 1024);
      setScope(target); setRevision(r => r + 1);
    } catch (e) { setError(errorText(e)); }
    finally { setUploading(false); }
  }
  const connection = busy ? 'Connecting…' : error ? 'Check connection or request' : status ? '● Store connected' : 'Connecting…';
  return <div className="app-frame flex overflow-hidden bg-[#0b1714]">
    <AppSidebar active="graph" connectionId="graph-connection" connection={connection} />
    <main className="graph-canvas relative min-w-0 flex-1 overflow-hidden" aria-label="Graph explorer">
      <section aria-label="RDF graph viewer" className="absolute inset-0">
        <D3Graph ref={canvas} nodes={view.nodes} edges={view.edges} selected={selected} inset={node ? DRAWER : 0} fade={fade} onPick={pick} onZoom={setZoom} />
        {(busy || !graph.nodes.size) && <div className="graph-veil absolute inset-0 flex items-center justify-center pointer-events-none"><p role="status">{busy ? 'Loading graph…' : error ? 'Graph could not be loaded' : 'No resources yet. Import RDF or try the animal example.'}</p></div>}
      </section>

      <button type="button" className="controls-tab graph-panel" aria-expanded={controlsOpen} aria-controls="graph-controls" aria-label={controlsOpen ? 'Hide graph controls' : 'Show graph controls'} onClick={toggleControls}
        style={{left: controlsOpen ? 'calc(var(--controls-w) + 1.5rem)' : '.75rem'}}>
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true"><path d="M4 7h9M17 7h3M4 17h3M11 17h9M13 5v4M7 15v4" /></svg>
        {!controlsOpen && <span>Controls</span>}
      </button>
      <aside id="graph-controls" aria-label="Graph controls" data-open={controlsOpen} {...(controlsOpen ? {} : {inert: true})} className="controls-drawer graph-panel">
        <div className="controls-section"><label htmlFor="graph-scope" className="controls-label">Graph</label>
          <select id="graph-scope" className="gfield w-full" value={scope} onChange={e => setScope(e.target.value)}><option value="all">All data graphs</option><option value="default">Default graph</option>{status?.graphs.map(raw => { const name = raw.startsWith('<') ? raw.slice(1, -1) : raw; return <option key={name} value={name}>{name}</option>; })}</select>
          <p className="graph-stats" id="graph-node-count">{graph.nodes.size} resources · {triples.length} statements</p></div>
        <div className="controls-section"><label htmlFor="graph-find" className="controls-label">Find</label>
          <input id="graph-find" aria-label="Find a node" placeholder="Find a node…" className="gfield w-full" value={search} onChange={e => setSearch(e.target.value)} />
          {search && <div className="controls-results">{matches.length ? matches.map(n => <button key={n.id} className="block w-full truncate rounded-md px-2.5 py-1.5 text-left hover:bg-white/10" onClick={() => reveal(n.id)} title={n.term.value}>{n.label}</button>) : <p className="p-2 muted">No matching loaded nodes.</p>}</div>}</div>
        <div className="controls-section"><span className="controls-label">Layout</span>
          <div className="grid grid-cols-2 gap-2">
            <button onClick={() => { setExpanded(new Set()); setSelected(''); }} className="gbtn">Collapse all</button>
            <button className="gbtn" onClick={() => canvas.current?.relayout()}>Re-layout</button>
            <button onClick={() => setRevision(r => r + 1)} disabled={busy || uploading} className="gbtn">Refresh</button>
            <button aria-label="Fit graph" className="gbtn" onClick={() => canvas.current?.fit()}>Fit · {Math.round(zoom * 100)}%</button></div></div>
        <div className="controls-section"><label htmlFor="fade-strength" className="controls-label flex justify-between">Fade strength<output htmlFor="fade-strength" className="muted">{fade === 0 ? 'Off' : `${fade.toFixed(1)}×`}</output></label>
          <input id="fade-strength" type="range" min="0" max="2" step="0.1" className="fade-range" value={fade} onChange={e => changeFade(Number(e.target.value))} aria-describedby="fade-hint" />
          <p id="fade-hint" className="graph-stats !whitespace-normal !px-0">Titles and edges fade out with distance, by node connectivity. Off shows everything.</p></div>
        <div className="controls-section"><span className="controls-label">Data</span>
          <div className="grid gap-2">
            <button disabled={busy || uploading} onClick={() => void animalExample()} className="gbtn">Try animal example</button>
            <label className={`gbtn accent grid cursor-pointer place-items-center ${uploading ? 'opacity-50' : ''}`}>{uploading ? 'Importing…' : 'Import RDF'}<input aria-label="Upload RDF to graph explorer" type="file" accept=".ttl,.nt,.trig,.nq" className="sr-only" disabled={uploading || busy} onChange={e => { void upload(e.target.files?.[0]); e.target.value = ''; }} /></label></div></div>
        <details className="controls-section controls-list"><summary className="controls-label cursor-pointer">Node list</summary>
          <div className="mt-2 flex max-h-56 flex-wrap gap-2 overflow-auto">{view.nodes.map(item => { const n = graph.nodes.get(item.id)!; return <button key={n.id} className="gbtn" aria-expanded={n.children.length ? expanded.has(n.id) : undefined} onClick={() => pick(n.id)}>{n.label}{n.children.length ? ` (${n.children.length} subclasses)` : ''}</button>; })}</div></details>
        {truncated && <p className="graph-note">Showing the first {CAP} statements. Choose a smaller graph to explore more precisely.</p>}
      </aside>

      <div className="graph-zoom graph-panel" style={{right: node ? `${DRAWER + 12}px` : '.75rem'}}>
        <button aria-label="Zoom out" className="gbtn" onClick={() => canvas.current?.zoomBy(1 / 1.2)}>−</button>
        <span className="graph-stats min-w-12 text-center">{Math.round(zoom * 100)}%</span>
        <button aria-label="Zoom in" className="gbtn" onClick={() => canvas.current?.zoomBy(1.2)}>+</button>
      </div>

      {error && <div role="alert" className="graph-alert">{error}</div>}
      <aside id="node-inspector" aria-label="Node properties" data-open={!!node} {...(node ? {} : {inert: true})} className="graph-drawer">
        {node && <div className="p-5">
          <div className="flex items-start gap-3"><div className="min-w-0 flex-1"><p className="text-xs muted">Selected resource</p><h2 className="mt-1 break-words text-xl font-semibold">{node.label}</h2></div>
            <button type="button" className="gbtn shrink-0" aria-label="Close details" onClick={() => setSelected('')}>✕</button></div>
          <p className="mt-2 break-all font-mono text-xs muted">{node.term.type === 'bnode' ? '_:' : ''}{node.term.value}</p>
          {node.types.length > 0 && <p className="mt-3 flex flex-wrap gap-2 text-xs">{node.types.map(type => <span key={type} title={type} className="inline-flex items-center gap-1.5 rounded-full border border-white/15 px-2.5 py-1"><i className="h-2 w-2 rounded-full" style={{background: typeColor(type)}} />{graph.nodes.get(termKey({type: 'uri', value: type}))?.label ?? shortName(type)}</span>)}</p>}
          {node.children.length > 0 && <div className="mt-5"><h3 className="text-sm font-semibold">Subclasses</h3>{node.children.map(id => <button key={id} className="linky mt-2 block text-sm" onClick={() => reveal(id)}>↳ {graph.nodes.get(id)!.label}</button>)}</div>}
          <h3 className="mt-6 text-sm font-semibold">Properties <span className="muted">{properties.length}</span></h3><p className="mt-1 text-xs muted">From the loaded statements in this view.</p>
          <dl className="mt-3 space-y-4">{properties.map((t, i) => <div key={i} className="rule border-b pb-3"><dt className="break-all text-xs text-emerald-300" title={t.p.value}>{shortName(t.p.value)}</dt><dd className="mt-1 break-words text-sm">{graph.nodes.has(termKey(t.o)) ? <button className="linky break-all" onClick={() => reveal(termKey(t.o))}>{graph.nodes.get(termKey(t.o))!.label}</button> : t.o.value}{t.o['xml:lang'] && <small className="block muted">@{t.o['xml:lang']}</small>}{t.o.datatype && <small className="block break-all muted">{t.o.datatype}</small>}<small className="mt-1 block break-all muted">{t.g?.value || 'Default graph'}</small></dd></div>)}</dl>
          <h3 className="mt-6 text-sm font-semibold">Incoming relationships</h3>{incoming.map((t, i) => <button key={i} className="linky mt-3 block text-xs" onClick={() => reveal(termKey(t.s))}>{shortName(t.s.value)} → {shortName(t.p.value)}</button>)}
        </div>}
      </aside>
    </main>
  </div>;
}
