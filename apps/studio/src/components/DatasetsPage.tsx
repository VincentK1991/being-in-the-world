import { useCallback, useEffect, useState } from 'react';
import AppSidebar from './AppSidebar';
import { getPlugins, getStatus, queryStore, type Plugin, type Status } from '../lib/api';
import { createDataset, deleteDataset, listDatasets, nameHint, selectDataset, selectedDataset, validName, type DatasetList } from '../lib/dataset';

const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
const short = (iri: string) => iri.split(/[/#]/).filter(Boolean).at(-1) || iri;
interface Count { label: string; title: string; n: number }
const COUNT = (variable: string, where: string, extra = '') =>
  `SELECT ?${variable} (COUNT(*) AS ?n) WHERE { ${where} } GROUP BY ?${variable} ORDER BY DESC(?n) ${extra}`;
async function counts(query: string, variable: string): Promise<Count[]> {
  const rows = (await queryStore(query)).data?.results?.bindings ?? [];
  return rows.map(row => ({ label: short(row[variable].value), title: row[variable].value, n: Number(row.n.value) }));
}
interface Content { defaultQuads: number; graphs: Count[]; classes: Count[]; predicates: Count[] }
async function loadContent(): Promise<Content> {
  const [defaultRows, graphs, classes, predicates] = await Promise.all([
    queryStore('SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }'),
    counts(COUNT('g', 'GRAPH ?g { ?s ?p ?o }'), 'g'),
    counts(COUNT('c', '{ ?s a ?c } UNION { GRAPH ?g { ?s a ?c } }', 'LIMIT 10'), 'c'),
    counts(COUNT('p', '{ ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } }', 'LIMIT 10'), 'p'),
  ]);
  return { defaultQuads: Number(defaultRows.data?.results?.bindings[0]?.n.value ?? 0), graphs, classes, predicates };
}
function Bars({ rows, empty }: { rows: Count[]; empty: string }) {
  const max = Math.max(1, ...rows.map(r => r.n));
  return rows.length ? <ul className="space-y-2">{rows.map(r => <li key={r.title} title={r.title} className="text-sm">
    <div className="flex justify-between gap-3"><span className="truncate">{r.label}</span><span className="tabular-nums text-slate-500">{r.n.toLocaleString()}</span></div>
    <div className="mt-1 h-1.5 rounded-full bg-slate-100"><div className="h-full rounded-full bg-emerald-500" style={{ width: `${Math.max(3, r.n / max * 100)}%` }} /></div>
  </li>)}</ul> : <p className="text-sm text-slate-500">{empty}</p>;
}

export default function DatasetsPage() {
  const [list, setList] = useState<DatasetList | null>(null), [current, setCurrent] = useState('');
  const [status, setStatus] = useState<Status | null>(null), [plugins, setPlugins] = useState<Plugin[] | null>(null), [content, setContent] = useState<Content | null>(null);
  const [error, setError] = useState(''), [name, setName] = useState(''), [busy, setBusy] = useState(false), [sectionError, setSectionError] = useState('');
  const refresh = useCallback(async () => {
    try { setList(await listDatasets()); setError(''); }
    catch (e) { setError(`Dataset management needs the gateway (rdftool gateway). ${errorText(e)}`); }
  }, []);
  useEffect(() => {
    void refresh();
    // The sidebar owns fallback for a deleted selection; here we only read it.
    setCurrent(selectedDataset());
    Promise.all([getStatus(), getPlugins(), loadContent()])
      .then(([s, p, c]) => { setStatus(s); setPlugins(p); setContent(c); })
      .catch(e => setSectionError(errorText(e)));
  }, [refresh]);
  const active = current || list?.default || '';
  async function create() {
    if (!validName(name)) { setError(nameHint); return; }
    setBusy(true); setError('');
    try { await createDataset(name); setName(''); await refresh(); }
    catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  }
  async function remove(target: string) {
    if (!window.confirm(`Delete dataset "${target}" and all its data? This cannot be undone.`)) return;
    setBusy(true); setError('');
    try {
      const warning = await deleteDataset(target);
      if (warning) setError(`Deleted ${target}, but cleanup needs attention: ${warning}`);
      if (target === current) { selectDataset(''); return; }
      await refresh();
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  }
  const connection = error && !list ? 'Gateway unavailable' : list ? '● Store connected' : 'Connecting…';
  return <div className="app-body flex">
    <AppSidebar active="datasets" connectionId="datasets-connection" connection={connection} />
    <main className="min-w-0 flex-1 bg-slate-50">
      <div className="mx-auto max-w-6xl space-y-6 p-4 md:p-10">
        <header><h1 className="text-2xl font-semibold tracking-tight md:text-3xl">Datasets</h1>
          <p className="mt-2 max-w-2xl text-sm text-slate-500">Each dataset is a separate database with its own graphs, plugins and derived data. Pick one to explore it everywhere in Studio.</p></header>
        {error && <div role="alert" className="notice error">{error}</div>}

        <section className="card" aria-label="All datasets">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-100 px-5 py-4"><h2 className="text-sm font-semibold">All datasets</h2>
            <form className="flex gap-2" onSubmit={e => { e.preventDefault(); void create(); }}>
              <input aria-label="New dataset name" value={name} onChange={e => setName(e.target.value)} placeholder="New dataset name" maxLength={64} className="w-48 rounded-lg border border-slate-200 px-3 py-2 text-sm" />
              <button className="primary" disabled={busy || !name}>Create dataset</button></form></div>
          <table className="w-full text-left text-sm"><thead className="text-xs text-slate-500"><tr><th className="px-5 py-3 font-medium">Name</th><th className="px-3 py-3 font-medium">Status</th><th className="px-3 py-3 text-right font-medium">Quads</th><th className="px-3 py-3 text-right font-medium">Named graphs</th><th className="px-5 py-3"><span className="sr-only">Actions</span></th></tr></thead>
            <tbody id="dataset-rows">{list?.datasets.map(d => <tr key={d.name} className="border-t border-slate-100" aria-current={d.name === active ? 'true' : undefined}>
              <td className="px-5 py-3 font-medium">{d.name}{d.default && <span className="ml-2 rounded bg-slate-100 px-1.5 py-0.5 text-[10px] font-normal text-slate-500">default</span>}{d.name === active && <span className="ml-2 rounded bg-emerald-100 px-1.5 py-0.5 text-[10px] font-normal text-emerald-800">in use</span>}</td>
              <td className="px-3 py-3 text-slate-500">{d.running ? 'Running' : 'Starts on first use'}</td>
              <td className="px-3 py-3 text-right tabular-nums">{d.quads?.toLocaleString() ?? '—'}</td>
              <td className="px-3 py-3 text-right tabular-nums">{d.graphs ?? '—'}</td>
              <td className="space-x-2 px-5 py-3 text-right">{d.name !== active && <button className="secondary !py-1.5" onClick={() => selectDataset(d.name === list.default ? '' : d.name)}>Use</button>}{!d.default && <button className="secondary !py-1.5 text-red-700" disabled={busy} onClick={() => void remove(d.name)} aria-label={`Delete dataset ${d.name}`}>Delete</button>}</td>
            </tr>)}</tbody></table>
          {!list && !error && <p className="p-5 text-sm text-slate-500">Loading datasets…</p>}
        </section>

        {sectionError && <div role="alert" className="notice error">{sectionError}</div>}
        <div className="grid gap-6 lg:grid-cols-2">
          <section className="card p-5" aria-label="Dataset content">
            <h2 className="text-sm font-semibold">Content of {active || 'this dataset'}</h2>
            <p className="mt-1 text-xs text-slate-500">{status ? `${status.quads.toLocaleString()} quads in ${status.graphs.length} named graphs` : 'Loading…'}{content && content.defaultQuads > 0 ? ` · ${content.defaultQuads.toLocaleString()} in the default graph` : ''}</p>
            <h3 className="mt-5 text-xs font-medium text-slate-500">Named graphs</h3>
            <div className="mt-2 max-h-72 overflow-auto pr-1" id="dataset-graphs"><Bars rows={content?.graphs ?? []} empty={content ? 'No named graphs yet. Import RDF to add data.' : 'Loading…'} /></div>
            <h3 className="mt-5 text-xs font-medium text-slate-500">Most common types</h3>
            <div className="mt-2"><Bars rows={content?.classes ?? []} empty={content ? 'No typed resources.' : 'Loading…'} /></div>
            <h3 className="mt-5 text-xs font-medium text-slate-500">Most used properties</h3>
            <div className="mt-2"><Bars rows={content?.predicates ?? []} empty={content ? 'No statements yet.' : 'Loading…'} /></div>
          </section>
          <section className="card p-5" aria-label="Plugins">
            <h2 className="text-sm font-semibold">Plugins for {active || 'this dataset'}</h2>
            <p className="mt-1 text-xs text-slate-500">Plugins are configured per dataset; semantic search and inference appear when configured for it.</p>
            <ul id="dataset-plugins" className="mt-4 space-y-4">{plugins?.map(p => <li key={p.id} className="rounded-lg border border-slate-100 p-3">
              <div className="flex items-baseline justify-between"><strong className="text-sm">{p.id}</strong><span className="text-xs text-slate-400">v{p.version}</span></div>
              {p.operations.length > 0 && <p className="mt-2 text-xs text-slate-500">Operations: <span className="font-mono">{p.operations.join(', ')}</span></p>}
              {p.properties.map(prop => <p key={prop.iri} className="mt-1 text-xs text-slate-500" title={prop.description}><code className="break-all">{prop.iri}</code> · {prop.input_arity} in → {prop.output_arity} out</p>)}
            </li>)}</ul>
            {plugins && !plugins.length && <p className="mt-4 text-sm text-slate-500">No plugins loaded.</p>}
            {!plugins && !sectionError && <p className="mt-4 text-sm text-slate-500">Loading…</p>}
          </section>
        </div>
      </div>
    </main>
  </div>;
}
