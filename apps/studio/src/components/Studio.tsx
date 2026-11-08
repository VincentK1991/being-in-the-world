import { useEffect, useRef, useState, type SyntheticEvent } from 'react';
import AppSidebar from './AppSidebar';
import { selectedDataset } from '../lib/dataset';
import { available, groupOf, templates } from '../lib/queries';
import { checked, getPlugins, getStatus, importFile, pluginStatus, queryStore, queryVia, type Plugin, type QueryResponse, type Status, type Term } from '../lib/api';

const byId = (id: string) => templates.find(t => t.id === id)?.query ?? '';
const routeOf = (id: string) => templates.find(t => t.id === id)?.via ?? '';
const routeNote: Record<string, string> = {
  federation: 'Runs through the federation plugin. Patterns outside SERVICE use this dataset; each SERVICE clause goes to its own endpoint. Only registered endpoints are allowed. Read a named graph from a SERVICE block with a sub-select (SERVICE <endpoint> { SELECT ... { GRAPH <g> { ... } } }); a bare GRAPH block can silently return no rows.',
  'virtual-graph': 'Runs on the Ontop virtual graph. The triples are generated from Postgres rows by an R2RML mapping when you query, not stored here.',
};
const failure = (error: unknown) => error instanceof Error ? error.message : String(error);
function TermValue({ term }: { term?: Term }) {
  if (!term) return <span className="unbound" title="Unbound variable">—</span>;
  return <><span className={`term ${term.type === 'uri' ? 'uri' : term.type === 'bnode' ? 'bnode' : 'literal'}`} title={term.value}>{term.type === 'bnode' ? '_:' : ''}{term.value}</span>{term['xml:lang'] ? <small className="block text-slate-500">@{term['xml:lang']}</small> : term.datatype && term.datatype !== 'http://www.w3.org/2001/XMLSchema#string' ? <small className="block break-all text-slate-500">{term.datatype}</small> : null}</>;
}

export default function Studio() {
  const [status, setStatus] = useState<Status | null>(null), [connectionError, setConnectionError] = useState('');
  const [plugins, setPlugins] = useState<Plugin[]>([]), [pluginError, setPluginError] = useState('');
  const [query, setQuery] = useState(templates[0].query), [template, setTemplate] = useState('browse'), [route, setRoute] = useState(''), [services, setServices] = useState<Record<string, Record<string, unknown>>>({});
  const [result, setResult] = useState<QueryResponse | null>(null), [queryError, setQueryError] = useState('');
  const [busy, setBusy] = useState(''), [resultStatus, setResultStatus] = useState(''), [elapsed, setElapsed] = useState('');
  const [view, setView] = useState('table'), [page, setPage] = useState(0), [history, setHistory] = useState<string[]>([]);
  const [file, setFile] = useState<File | null>(null), [graph, setGraph] = useState(''), [uploadMessage, setUploadMessage] = useState(''), [uploadError, setUploadError] = useState(false);
  const dialog = useRef<HTMLDialogElement>(null), gutter = useRef<HTMLPreElement>(null), running = useRef(false);
  const [dataset, setDataset] = useState('default');
  useEffect(() => setDataset(selectedDataset() || 'default'), []);
  const limit = status?.max_upload_bytes ?? 16 * 1024 * 1024;
  const rows = result?.data?.results?.bindings, boolean = result?.data?.boolean;
  async function refresh() { try { setStatus(await getStatus()); setConnectionError(''); } catch (error) { setConnectionError(failure(error)); } }
  useEffect(() => {
    void refresh();
    void getPlugins().then(setPlugins).catch(error => setPluginError(failure(error)));
    try { const saved: unknown = JSON.parse(localStorage.getItem('rdf-studio-history') || '[]'); if (Array.isArray(saved)) setHistory(saved.filter((item): item is string => typeof item === 'string' && item.length <= 50000).slice(0, 8)); } catch { /* Optional browser storage. */ }
  }, []);
  useEffect(() => {
    for (const id of ['federation', 'virtual-graph']) if (plugins.some(p => p.id === id)) void pluginStatus(id).then(status => setServices(current => ({ ...current, [id]: status }))).catch(() => { /* status is informational */ });
  }, [plugins]);
  function saveHistory(next: string[]) { setHistory(next); try { localStorage.setItem('rdf-studio-history', JSON.stringify(next)); } catch { /* Optional browser storage. */ } }
  async function run() {
    if (running.current) return;
    if (!query.trim()) { setQueryError('Enter a SPARQL query first.'); return; }
    running.current = true; setBusy('query'); setQueryError(''); setResult(null); setElapsed(''); setResultStatus('Executing against the selected dataset…');
    const start = performance.now();
    try { setResult(route ? await queryVia(route, query) : await queryStore(query)); setPage(0); setElapsed(`${Math.round(performance.now() - start)} ms`); setResultStatus('Query completed successfully.'); saveHistory([query, ...history.filter(item => item !== query)].slice(0, 8)); }
    catch (error) { setQueryError(failure(error)); setResultStatus(''); }
    finally { running.current = false; setBusy(''); }
  }
  async function upload(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault(); if (!file || running.current) return;
    running.current = true; setBusy('upload'); setUploadError(false); setUploadMessage('Parsing and importing…');
    try { const data = await importFile(file, graph.trim(), limit); setUploadMessage(`Imported ${file.name}: ${data.parsed_quads} parsed quads. Store now contains ${data.total_quads} quads.`); await refresh(); }
    catch (error) { setUploadError(true); setUploadMessage(`Import failed. ${failure(error)}`); }
    finally { running.current = false; setBusy(''); }
  }
  async function loadExample() {
    if (running.current) return; running.current = true; setBusy('upload'); setQueryError('');
    try { const response = await checked(await fetch('/example.ttl')); await importFile(new File([await response.text()], 'example.ttl'), '', limit); await refresh(); setQuery(byId('people')); setTemplate('people'); setRoute(''); setResultStatus('Example imported. Run the people & projects query to inspect it.'); }
    catch (error) { setQueryError(failure(error)); }
    finally { running.current = false; setBusy(''); }
  }
  function download() { if (!result) return; const url = URL.createObjectURL(new Blob([result.text], { type: result.mime })); const a = document.createElement('a'); a.href = url; a.download = result.data ? 'results.json' : 'results.ttl'; a.click(); setTimeout(() => URL.revokeObjectURL(url), 1000); }
  function openUpload() { setUploadMessage(''); dialog.current?.showModal(); }

  const connection = connectionError ? 'Connection unavailable' : status ? '● Store connected' : 'Connecting…';
  return <div className="app-body flex">
    <AppSidebar active="query" connectionId="connection-label" connection={connection}>

        <p className="mb-3 text-[10px] tracking-widest text-emerald-100/50">WORKSPACE</p>
        <div className="rounded-lg border border-white/15 p-3 text-sm"><strong>Dataset: {dataset}</strong><small className="block text-emerald-100/50 mt-1">Queries and imports run here</small></div>
        <button id="upload-nav" className="mt-4 w-full rounded-lg bg-white/10 p-3 text-left text-sm" onClick={openUpload}>↥ Import data</button>
        <div className="mt-8 flex justify-between text-[10px] tracking-widest text-emerald-100/50">DATA OVERVIEW<button id="refresh" aria-label="Refresh dataset statistics" disabled={!!busy} onClick={() => void refresh()}>↻</button></div>
        <div className="mt-4 flex justify-between text-sm"><span>Quads</span><strong id="quad-count">{status?.quads.toLocaleString() ?? '—'}</strong></div>
        <div className="mt-3 flex justify-between text-sm"><span>Named graphs</span><strong id="graph-count">{status?.graphs.length ?? '—'}</strong></div>
        <div id="graph-list" className="mt-4 space-y-2">{status?.graphs.map(name => <button key={name} className="block w-full truncate text-xs font-mono text-emerald-100/60 text-left" title={name} onClick={() => setQuery(`SELECT ?subject ?predicate ?object\nWHERE { GRAPH ${name} { ?subject ?predicate ?object } }\nLIMIT 100`)}>{name}</button>)}</div>
        <div className="mt-8 border-t border-white/10 pt-6 flex justify-between text-[10px] tracking-widest text-emerald-100/50">RECENT QUERIES<button id="clear-history" aria-label="Clear query history" onClick={() => saveHistory([])}>×</button></div>
        <div id="history" className="mt-3 space-y-3">{history.length ? history.map(item => <button key={item} title={item} className="block truncate w-full text-left text-xs text-emerald-100/60" onClick={() => setQuery(item)}>{item}</button>) : <p className="text-xs text-emerald-100/50">No queries yet.</p>}</div>
        <div className="mt-8 border-t border-white/10 pt-6 text-[10px] tracking-widest text-emerald-100/50">ENGINE PLUGINS</div>
        <div id="plugins" className="mt-3 space-y-3">{plugins.map(plugin => <div key={plugin.id} className="text-xs" title={plugin.operations.join(', ')}>{plugin.id}<span className="ml-2 text-emerald-100/50">{plugin.version}</span>{plugin.properties.map(property => <div key={property.iri} className="mt-2 text-emerald-100/60" title={property.description}><code className="block truncate">{property.iri}</code><small>{property.input_arity} inputs → {property.output_arity} outputs</small></div>)}</div>)}{pluginError && <p role="alert">{pluginError}</p>}</div>
    </AppSidebar>
    <main className="min-w-0 flex-1">
      <header className="h-16 border-b border-slate-200 bg-white px-6 md:px-10 flex items-center justify-between text-xs"><span className="text-slate-500">Workspace / <strong className="text-slate-700">Query</strong></span></header>
      <div className="max-w-7xl mx-auto p-4 md:p-10 space-y-6">
        <div className="flex items-center justify-between gap-3"><div><p className="text-[9px] md:text-[10px] tracking-[.2em] text-emerald-800/60 font-semibold">EXPLORE YOUR KNOWLEDGE GRAPH</p><h1 className="text-2xl md:text-3xl font-semibold tracking-tight mt-2">Query workspace<span className="text-accent">.</span></h1><p className="text-xs text-slate-500 mt-2">Bring your RDF data. Ask better questions.</p></div><button id="open-upload" className="primary shrink-0" onClick={openUpload}>↥ Import RDF</button></div>
        {connectionError && <div id="connection-error" role="alert" className="notice error">{connectionError}</div>}
        <section className="card" aria-label="SPARQL query editor">
          <div className="px-5 py-4 border-b border-slate-100 flex justify-between text-sm"><strong>● SPARQL editor</strong><span className="text-xs text-slate-400">SPARQL 1.1</span></div>
          <div className="p-4 flex items-center gap-3 text-xs"><label htmlFor="template" className="text-slate-500 shrink-0">Start with</label><select id="template" className="border border-slate-200 rounded p-2 min-w-0 max-w-72" value={template} onChange={event => { setTemplate(event.target.value); setQuery(byId(event.target.value)); setRoute(routeOf(event.target.value)); }}>{groupOf(available(plugins.map(p => p.id))).map(g => <optgroup key={g.group} label={g.group}>{g.items.map(t => <option key={t.id} value={t.id}>{t.label}</option>)}</optgroup>)}</select><label htmlFor="route" className="ml-2 text-slate-500 shrink-0">Run on</label><select id="route" className="border border-slate-200 rounded p-2 min-w-0 max-w-48" value={route} onChange={event => setRoute(event.target.value)}><option value="">This dataset</option>{plugins.some(p => p.id === 'federation') && <option value="federation">Federation (SERVICE)</option>}{plugins.some(p => p.id === 'virtual-graph') && <option value="virtual-graph">Virtual graph (Ontop)</option>}</select><button className="ml-auto text-slate-500" onClick={() => { setQuery(byId(template)); setRoute(routeOf(template)); }}>Reset editor</button></div>
          {route && <div id="route-help" className="mx-4 mb-3 rounded-lg bg-emerald-50 p-3 text-xs text-slate-700"><p>{routeNote[route]}</p>
            {route === 'federation' && Array.isArray(services.federation?.allow) && <details className="mt-2"><summary className="cursor-pointer font-medium">Registered SERVICE endpoints ({services.federation.allow.length})</summary><ul className="mt-1 space-y-0.5">{(services.federation.allow as string[]).map(url => <li key={url}><code className="break-all">{url}</code></li>)}</ul><p className="mt-1 text-slate-500">Other datasets are reached as <code>/d/&lt;name&gt;/query</code>. The demo datasets <strong>claims</strong> and <strong>finance</strong> come from <code>just federation</code>.</p></details>}
            {route === 'virtual-graph' && services['virtual-graph'] && <p className="mt-1 text-slate-500">{String(services['virtual-graph'].name)}: {String(services['virtual-graph'].state)}{services['virtual-graph'].triples ? ` · ${String(services['virtual-graph'].triples)} triples` : ''}{services['virtual-graph'].description ? ` · ${String(services['virtual-graph'].description)}` : ''}</p>}
          </div>}
          <div className="flex py-4"><pre ref={gutter} aria-hidden="true" className="text-right w-12 shrink-0 pr-3 text-slate-300 text-xs leading-6 max-h-56 overflow-hidden">{query.split('\n').map((_, i) => i + 1).join('\n')}</pre><label htmlFor="query" className="sr-only">SPARQL query</label><textarea id="query" spellCheck={false} className="w-full min-w-0 h-56 resize-y pr-5 font-mono text-xs leading-6 text-emerald-900 whitespace-pre overflow-auto" value={query} onChange={event => setQuery(event.target.value)} onScroll={event => { if (gutter.current) gutter.current.scrollTop = event.currentTarget.scrollTop; }} onKeyDown={event => { if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); void run(); } }} /></div>
          <div className="flex items-center justify-between gap-3 px-5 py-3 border-t border-slate-100"><span className="text-[10px] text-slate-400">SELECT, ASK, CONSTRUCT, DESCRIBE <span className="hidden md:inline"> · Ctrl / ⌘ + Enter</span></span><button id="run-query" className="primary shrink-0" disabled={!!busy} onClick={() => void run()}>{busy === 'query' ? 'Running…' : '▶ Run query'}</button></div>
        </section>
        <section id="results-panel" className="card" aria-label="Query results" aria-busy={busy === 'query'}>
          <div className="px-4 md:px-5 py-4 flex items-center justify-between gap-2 border-b border-slate-100"><div className="flex gap-2 items-center"><strong className="text-sm">Results</strong><span id="result-count" className="text-[10px] px-2 py-1 rounded bg-emerald-50 text-emerald-800">{rows ? `${rows.length} rows` : boolean !== undefined ? 'ASK' : result ? 'Graph' : '—'}</span><span id="elapsed" className="hidden md:inline text-xs text-slate-400">{elapsed}</span></div><div className="flex gap-3 items-center"><div role="tablist" aria-label="Result view" className="bg-slate-100 p-1 rounded text-xs">{['table', 'raw'].map(tab => <button key={tab} id={`${tab}-tab`} role="tab" aria-selected={view === tab} aria-controls={`${tab}-view`} className={`px-2 py-1 rounded capitalize ${view === tab ? 'bg-white shadow-sm' : ''}`} onClick={() => setView(tab)}>{tab}</button>)}</div><button id="download" className="text-xs" disabled={!result} onClick={download}>↓ Download</button></div></div>
          {queryError && <div id="query-error" role="alert" className="notice error m-3">{queryError}</div>}
          <div id="result-status" role="status" className={resultStatus ? 'px-5 py-3 text-xs text-emerald-800 bg-emerald-50/50' : ''}>{resultStatus}</div>
          <div id="table-view" role="tabpanel" aria-labelledby="table-tab" hidden={view !== 'table'} className="overflow-auto max-h-[520px]">
            {boolean !== undefined ? <div className="boolean-result p-10 text-3xl font-semibold text-accent">{String(boolean).toUpperCase()}<p className="text-sm font-normal mt-3">{boolean ? 'The pattern has a match.' : 'The pattern has no match in this dataset.'}</p></div> : rows ? rows.length ? <table className="w-full min-w-[680px] text-left text-xs"><thead className="sticky top-0 bg-slate-50"><tr><th className="p-4">#</th>{result?.data?.head?.vars.map(name => <th key={name} scope="col" className="p-4">?{name}</th>)}</tr></thead><tbody>{rows.slice(page * 50, (page + 1) * 50).map((row, i) => <tr key={i} className="border-t border-slate-100 hover:bg-slate-50"><td className="p-4 text-slate-400 whitespace-nowrap">{page * 50 + i + 1}</td>{result?.data?.head?.vars.map(name => <td key={name} className="p-4 max-w-96 align-top"><TermValue term={row[name]} /></td>)}</tr>)}</tbody></table> : <div className="p-12 text-center"><h2>No matching results</h2><p className="text-sm text-slate-500 mt-2">Check the graph selection or try a broader pattern.</p></div> : result ? <pre className="graph-result p-5 text-xs overflow-auto">{result.text}</pre> : !queryError && !busy ? <div className="p-10 md:p-14 text-center"><div className="text-4xl text-accent mb-4">⌘</div><h2 className="font-semibold">Your next insight starts with a query</h2><p className="text-sm text-slate-500 mt-3 mb-5">Import a Turtle file, or load the example dataset to explore.</p><button id="load-example" className="secondary" disabled={!!busy} onClick={() => void loadExample()}>Load example data ↗</button><small className="block text-slate-400 mt-3">Appends 14 triples to the default graph.</small></div> : null}
          </div>
          <pre id="raw-view" role="tabpanel" aria-labelledby="raw-tab" hidden={view !== 'raw'} className="p-5 overflow-auto max-h-[520px] text-xs">{result?.data ? JSON.stringify(result.data, null, 2) : result?.text}</pre>
          <div className="p-4 border-t border-slate-100 flex justify-between gap-3 text-[10px] text-slate-500"><span>{rows ? `${rows.length} returned rows · 50 per page · Add LIMIT to bound large queries` : queryError ? 'Query failed · no results' : 'Results from your persistent RDF store'}</span><div id="pagination" hidden={!rows || rows.length <= 50} className="flex gap-3"><button id="previous" aria-label="Previous result page" disabled={page === 0} onClick={() => setPage(page - 1)}>←</button><span>{page + 1} / {Math.ceil((rows?.length ?? 0) / 50)}</span><button id="next" aria-label="Next result page" disabled={(page + 1) * 50 >= (rows?.length ?? 0)} onClick={() => setPage(page + 1)}>→</button></div></div>
        </section>
        <footer className="flex flex-wrap justify-between gap-3 text-[10px] text-slate-400"><span>RDF Studio / Observe your database behavior</span><a href="/example.ttl" download>Download example Turtle ↗</a></footer>
      </div>
    </main>
    <dialog id="upload-dialog" ref={dialog} aria-labelledby="upload-title" onCancel={event => { if (busy) event.preventDefault(); }} className="m-auto w-[min(500px,calc(100vw-32px))] rounded-xl p-6 shadow-2xl backdrop:bg-forest/50">
      <form id="upload-form" onSubmit={event => void upload(event)} className="space-y-4">
        <div className="flex justify-between"><h2 id="upload-title" className="text-xl font-semibold">Import RDF</h2><button id="close-upload" type="button" aria-label="Close import dialog" disabled={!!busy} onClick={() => dialog.current?.close()}>×</button></div>
        <p className="text-sm text-slate-500">Append RDF data to the connected store. Invalid files roll back completely.</p>
        <label className="block rounded-lg border-2 border-dashed border-emerald-200 bg-emerald-50/40 p-5 text-center space-y-3" onDragOver={event => event.preventDefault()} onDrop={event => { event.preventDefault(); if (!busy) setFile(event.dataTransfer.files[0] ?? null); }}><strong className="block text-sm">Choose a file or drop it here</strong><span className="block text-xs text-slate-500">Turtle (.ttl), N-Triples, TriG, or N-Quads</span><input id="rdf-file" type="file" accept=".ttl,.nt,.trig,.nq" disabled={!!busy} className="text-xs max-w-full" onChange={event => setFile(event.target.files?.[0] ?? null)} /><span className="block text-xs text-slate-500">{file?.name ?? 'No file selected'}</span></label>
        <label htmlFor="target-graph" className="block text-sm">Target graph <span className="text-slate-400">optional</span></label><input id="target-graph" className="border border-slate-200 rounded-lg p-3 w-full text-sm" value={graph} disabled={!!busy} onChange={event => setGraph(event.target.value)} placeholder="https://example.org/my-graph" />
        <p className="text-xs text-slate-500">Leave empty for the default graph. TriG and N-Quads preserve their own graphs.</p><p className="text-xs text-slate-500">Maximum upload: {limit / 1024 / 1024} MiB · Changes persist on disk</p>
        <div id="upload-message" role="status" hidden={!uploadMessage} className={`notice ${uploadError ? 'error' : ''}`}>{uploadMessage}</div>
        <div className="flex justify-end gap-3"><button id="cancel-upload" type="button" className="secondary" disabled={!!busy} onClick={() => dialog.current?.close()}>Cancel</button><button id="submit-upload" type="submit" className="primary" disabled={!!busy || !file}>{busy === 'upload' ? 'Importing…' : 'Import file ↥'}</button></div>
      </form>
    </dialog>
  </div>;
}
