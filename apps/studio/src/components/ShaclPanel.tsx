import { useEffect, useState } from 'react';
import { checked, queryStore, type Term } from '../lib/api';
import { datasetBase } from '../lib/dataset';

interface Job { name: string; data_graphs: string[]; shapes_graphs: string[]; fail_on: string }
interface Summary { conforms: boolean; gate_passed: boolean; counts: Record<string, number> }
interface Status { state: string; graph?: string; metadata?: Summary }
type Row = Record<string, Term>;
const failure = (e: unknown) => e instanceof Error ? e.message : String(e);
async function invoke(operation: string, input: object) {
  return (await checked(await fetch(`${datasetBase()}/api/plugins/shacl/operations/${operation}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(input),
  }))).json();
}

export default function ShaclPanel() {
  const [jobs, setJobs] = useState<Job[]>([]), [name, setName] = useState('');
  const [busy, setBusy] = useState(false), [error, setError] = useState(''), [notice, setNotice] = useState('');
  const [status, setStatus] = useState<Status | null>(null), [rows, setRows] = useState<Row[]>([]);
  const job = jobs.find(j => j.name === name);
  async function refresh(selected: string) {
    const next: Status = await invoke('status', { job: selected });
    setStatus(next);
    // Job names are host-validated identifiers, never arbitrary query fragments.
    const results = await queryStore(`PREFIX sh: <http://www.w3.org/ns/shacl#>
SELECT ?severity ?focus ?path ?value ?message WHERE {
  GRAPH <urn:rdftool:derived:shacl:${selected}> {
    ?report a sh:ValidationReport ; sh:result ?finding .
    ?finding sh:resultSeverity ?severity ; sh:focusNode ?focus .
    OPTIONAL { ?finding sh:resultPath ?path }
    OPTIONAL { ?finding sh:value ?value }
    OPTIONAL { ?finding sh:resultMessage ?message }
  }
} ORDER BY ?severity ?focus ?path LIMIT 1001`);
    setRows(results.data?.results?.bindings ?? []);
  }
  useEffect(() => {
    invoke('jobs', {}).then((result: { jobs: Job[] }) => { setJobs(result.jobs); setName(result.jobs[0]?.name ?? ''); })
      .catch(e => setError(failure(e)));
  }, []);
  useEffect(() => {
    setStatus(null); setRows([]); setError(''); setNotice('');
    if (name) { setBusy(true); refresh(name).catch(e => setError(failure(e))).finally(() => setBusy(false)); }
  }, [name]);
  async function run(operation: 'validate' | 'check') {
    setBusy(true); setError(''); setNotice('');
    try {
      const result: { summary: Summary } = await invoke(operation, { job: name });
      setNotice(operation === 'check' ? 'Validation gate passed.' : result.summary.conforms ? 'Validation completed: data conforms.' : 'Report saved. Findings did not block or change your data.');
    } catch (e) { setError(failure(e)); }
    finally {
      try { await refresh(name); } catch (e) { setError(failure(e)); }
      setBusy(false);
    }
  }
  async function upload(file: File, graph: string) {
    setBusy(true); setError(''); setNotice('');
    try {
      if (!/\.(ttl|nt)$/i.test(file.name)) throw new Error('Choose a Turtle (.ttl) or N-Triples (.nt) file.');
      if (file.size > 16 * 1024 * 1024) throw new Error('File exceeds the 16 MiB limit.');
      await checked(await fetch(`${datasetBase()}/store?${new URLSearchParams({ graph })}`, {
        method: 'PUT', headers: { 'Content-Type': file.name.toLowerCase().endsWith('.nt') ? 'application/n-triples' : 'text/turtle' }, body: file,
      }));
      setNotice(`Replaced ${graph}. Run validation to refresh the report.`);
      await refresh(name);
    } catch (e) { setError(failure(e)); }
    finally { setBusy(false); }
  }
  const summary = status?.metadata;
  return <section className="card space-y-5 p-5" aria-label="SHACL validation">
    <div><h2 className="text-xl font-semibold">SHACL validation</h2>
      <p className="mt-2 text-sm text-slate-600">Check RDF against shapes using Apache Jena. Report-only is the default. An explicit gate fails at its configured severity; ordinary uploads and updates remain independent.</p></div>
    {error && <p role="alert" className="notice error break-words">{error}</p>}
    {notice && <p role="status" className="notice">{notice}</p>}
    <label className="block text-sm font-medium">Validation job
      <select aria-label="Validation job" value={name} disabled={busy} onChange={e => setName(e.target.value)} className="mt-1 w-full rounded-lg border border-slate-300 bg-white p-2">
        {jobs.map(j => <option key={j.name} value={j.name}>{j.name}</option>)}
      </select>
    </label>
    {job && <>
      <div className="grid gap-4 md:grid-cols-2">{[
        ['Data', job.data_graphs], ['Shapes', job.shapes_graphs],
      ].map(([label, graphs]) => <div key={label as string} className="min-w-0 rounded-lg border border-slate-200 p-3">
        <h3 className="text-sm font-semibold">{label}</h3>
        {(graphs as string[]).map(graph => <label key={graph} className="mt-2 block text-xs text-slate-600">
          <span className="block break-all">{graph}</span>
          <span className="mt-1 block">Replace this graph with a .ttl or .nt file</span>
          <input aria-label={`Replace ${label} graph ${graph}`} type="file" accept=".ttl,.nt" disabled={busy} className="mt-2 block w-full text-xs"
            onChange={e => { const file = e.target.files?.[0]; if (file) void upload(file, graph); e.target.value = ''; }} />
        </label>)}
      </div>)}</div>
      <p className="text-sm text-slate-500">Gate threshold: <strong>{job.fail_on}</strong> and higher. Shapes choose each finding’s severity with <code>sh:severity</code>; the default is Violation.</p>
      <div className="flex flex-wrap gap-2">
        <button className="btn primary" disabled={busy} onClick={() => void run('validate')}>Validate · report only</button>
        <button className="btn" disabled={busy} onClick={() => void run('check')}>Run validation gate</button>
        <button className="btn" disabled={busy} onClick={() => { setBusy(true); refresh(name).catch(e => setError(failure(e))).finally(() => setBusy(false)); }}>Refresh report status</button>
        {busy && <span className="self-center text-sm text-slate-500" role="status">Working…</span>}
      </div>
    </>}
    {status && <div className="rounded-lg bg-slate-100 p-3 text-sm" aria-label="Validation report status">
      <strong>Report: {status.state}</strong>
      {summary && <p className="mt-1">SHACL conforms: {String(summary.conforms)} · Gate passes: {String(summary.gate_passed)} · {summary.counts.violation} violations · {summary.counts.warning} warnings · {summary.counts.info} info</p>}
      {status.state === 'stale' && <p className="mt-1 text-amber-800">Data, shapes, configuration, or report changed. Findings below belong to the previous run.</p>}
      {status.graph && <code className="mt-1 block break-all text-xs">{status.graph}</code>}
    </div>}
    {rows.length > 0 && <div className="overflow-x-auto"><table className="w-full text-left text-xs" aria-label="SHACL findings">
      <thead><tr>{['Severity', 'Focus node', 'Path', 'Value', 'Message'].map(h => <th className="p-2" key={h}>{h}</th>)}</tr></thead>
      <tbody>{rows.map((row, i) => <tr key={i} className="border-t border-slate-200">{['severity', 'focus', 'path', 'value', 'message'].map(key => <td key={key} className="max-w-xs break-words p-2 align-top">{key === 'severity' ? row[key]?.value.split('#').pop() : row[key]?.value ?? '—'}</td>)}</tr>)}</tbody>
    </table>{rows.length > 1000 && <p>Showing the first 1001 report rows. Query the report graph for all details.</p>}</div>}
    <p className="text-xs text-slate-500">Validation uses the union of the configured data graphs, with no automatic inference. Reports are ordinary RDF and can be queried in the SPARQL workspace. A successful check describes that snapshot; it does not lock a subsequent write.</p>
  </section>;
}
