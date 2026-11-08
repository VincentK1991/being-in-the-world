import { useEffect, useRef, useState } from 'react';
import { createDataset, deleteDataset, listDatasets, nameHint, selectDataset, selectedDataset, validName, type DatasetList } from '../lib/dataset';

const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
const pageName = { graph: 'Graph explorer', datasets: 'Datasets', plugins: 'Plugins', query: 'SPARQL workspace' };
// Top-level context for the whole app: the dataset chosen here scopes the graph, the SPARQL workspace and the
// Datasets page. Switching reloads the page so no state leaks between datasets.
export default function ContextBar({active}: {active: keyof typeof pageName}) {
  const [list, setList] = useState<DatasetList | null>(null), [current, setCurrent] = useState('');
  const [adding, setAdding] = useState(false), [name, setName] = useState(''), [busy, setBusy] = useState(false), [error, setError] = useState('');
  const pop = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let alive = true;
    listDatasets().then(result => {
      if (!alive) return;
      const stored = selectedDataset();
      // A dataset deleted elsewhere falls back to the default instead of failing every request.
      if (stored && !result.datasets.some(d => d.name === stored)) { selectDataset(''); return; }
      setList(result); setCurrent(stored || result.default);
    }).catch(() => { /* a plain `rdftool serve` has one dataset and no dataset API */ });
    return () => { alive = false; };
  }, []);
  useEffect(() => {
    if (!adding) return;
    const close = (e: Event) => { if (e instanceof KeyboardEvent ? e.key === 'Escape' : !pop.current?.contains(e.target as Node)) { setAdding(false); setError(''); } };
    document.addEventListener('keydown', close); document.addEventListener('pointerdown', close);
    return () => { document.removeEventListener('keydown', close); document.removeEventListener('pointerdown', close); };
  }, [adding]);
  async function add() {
    if (!validName(name)) { setError(nameHint); return; }
    setBusy(true); setError('');
    try { await createDataset(name); selectDataset(name); }
    catch (e) { setError(errorText(e)); setBusy(false); }
  }
  async function remove() {
    if (!window.confirm(`Delete dataset "${current}" and all its data? This cannot be undone.`)) return;
    setBusy(true); setError('');
    try {
      const warning = await deleteDataset(current);
      if (warning) window.alert(`Dataset deleted, but cleanup needs attention:\n${warning}`);
      selectDataset('');
    } catch (e) { setError(errorText(e)); setBusy(false); }
  }
  const info = list?.datasets.find(d => d.name === current);
  return <header className="context-bar" aria-label="Dataset context">
    {list ? <>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className="shrink-0 text-emerald-300"><ellipse cx="12" cy="6" rx="7" ry="3" /><path d="M5 6v6c0 1.7 3.1 3 7 3s7-1.3 7-3V6M5 12v6c0 1.7 3.1 3 7 3s7-1.3 7-3v-6" /></svg>
      <label htmlFor="dataset-select" className="ctx-label">Dataset</label>
      <select id="dataset-select" value={current} disabled={busy} onChange={e => selectDataset(e.target.value === list.default ? '' : e.target.value)} className="ctx-select">
        {list.datasets.map(d => <option key={d.name} value={d.name}>{d.name}</option>)}
      </select>
      {info && info.quads !== null && <span className="ctx-stats">{info.quads.toLocaleString()} quads{info.graphs !== null ? ` · ${info.graphs} graphs` : ''}</span>}
      <span className="ctx-page" aria-hidden="true">/ {pageName[active]}</span>
      <div className="relative ml-auto flex items-center gap-2" ref={pop}>
        {current !== list.default && <button type="button" onClick={() => void remove()} disabled={busy} className="ctx-btn text-red-200" aria-label={`Delete dataset ${current}`}>Delete</button>}
        <button type="button" onClick={() => setAdding(a => !a)} disabled={busy} aria-expanded={adding} className="ctx-btn">New dataset</button>
        {adding && <form className="ctx-pop" onSubmit={e => { e.preventDefault(); void add(); }}>
          <input aria-label="New dataset name" autoFocus value={name} onChange={e => setName(e.target.value)} placeholder="tenant-name" maxLength={64} />
          <button type="submit" disabled={busy || !name} className="ctx-create">Create dataset</button>
          {error && <p role="alert">{error}</p>}
        </form>}
      </div>
    </> : <span className="ctx-stats">Single dataset</span>}
  </header>;
}
