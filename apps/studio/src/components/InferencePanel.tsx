import { useEffect, useState } from 'react';
import { checked, queryStore, type Term } from '../lib/api';
import { datasetBase } from '../lib/dataset';

interface Job {
  name: string; provider: string; profile: string; source_graphs: string[]; depends_on: string[];
  inputs: string[]; rules: number; tests: number; explains: boolean; graph: string;
}
interface Clash { message: string; terms?: string[]; rule?: string; premises?: string[] }
interface Status {
  generation?: string;
  metadata?: { consistency?: string; diagnostics?: { datatype_checks?: string; issues?: string[]; unchecked_datatypes?: string[] } };
  state: string; triples?: number; clashes?: Clash[]; inconsistent_since?: number;
  stale_upstream?: string[]; stale_inputs?: string[]; input_error?: string; inputs_checked?: boolean;
}
interface Step { job: string; action: string; blocked_by?: string }
interface Rebuilt { state: string; triples?: number; clashes?: Clash[]; blocked_by?: string; cascade?: { steps: Step[] } }
interface TestResult {
  name: string; passed: boolean; error?: string; inconsistent?: boolean; clashes?: Clash[];
  missing?: string[]; present_but_forbidden?: string[]; unexpected?: string[];
}
interface Premise {
  triple: string; asserted_in?: string[]; outside_input?: string[]; origin?: string;
  deduced_by?: Explanation[]; deduced_in_job?: { rule: string; premises?: Premise[]; truncated?: boolean };
}
interface Derivation { rule: string | number; rule_text?: string; aggregate?: boolean; incomplete?: boolean; premises: Premise[] }
interface Explanation {
  generation?: string; explanation_mode?: string;
  job: string; supported?: boolean; stale?: boolean; note?: string; explained?: boolean; repeated?: boolean;
  derivations?: Derivation[]; source_graphs?: string[];
}
interface Why { triple: string; derived: boolean; asserted_in?: string[]; explanations: Explanation[]; truncated?: boolean }
type Cascade = '' | 'upstream' | 'downstream' | 'both';

const failure = (e: unknown) => e instanceof Error ? e.message : String(e);
async function invoke<T>(operation: string, input: object): Promise<T> {
  return (await checked(await fetch(`${datasetBase()}/api/plugins/inference/operations/${operation}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(input),
  }))).json();
}
const STATE_STYLE: Record<string, string> = {
  ready: 'bg-emerald-50 text-emerald-800 ring-emerald-200',
  stale: 'bg-amber-50 text-amber-900 ring-amber-200',
  inconsistent: 'bg-red-50 text-red-800 ring-red-200',
  blocked: 'bg-orange-50 text-orange-900 ring-orange-200',
  missing: 'bg-slate-100 text-slate-600 ring-slate-200',
};
const STATE_HELP: Record<string, string> = {
  ready: 'Up to date with its sources; this does not prove consistency or complete OWL support.',
  stale: 'Its sources, inputs or configuration changed since the last rebuild.',
  missing: 'Never built.',
  inconsistent: 'The last rebuild found the current data inconsistent. The previous output is still published.',
  blocked: 'An upstream job is not ready, so this job was not rebuilt.',
};
function StateBadge({ state }: { state: string }) {
  return <span className={`inline-block rounded-full px-2 py-0.5 text-xs font-medium ring-1 ${STATE_STYLE[state] ?? STATE_STYLE.missing}`}>{state}</span>;
}
/** One RDF term as N-Triples, so a result row can be handed back to `explain`. */
function nt(term: Term): string | null {
  if (term.type === 'uri') return `<${term.value}>`;
  if (term.type === 'bnode') return null;
  const text = term.value.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\r/g, '\\r');
  if (term['xml:lang']) return `"${text}"@${term['xml:lang']}`;
  if (term.datatype && term.datatype !== 'http://www.w3.org/2001/XMLSchema#string') return `"${text}"^^<${term.datatype}>`;
  return `"${text}"`;
}
const short = (iri: string) => iri.replace(/^<|>$/g, '').split(/[#/]/).filter(Boolean).pop() ?? iri;
/** `<s> <p> <o> .` shortened for reading; the full line stays in the title. */
function Triple({ line }: { line: string }) {
  const parts = line.replace(/ \.$/, '').match(/<[^>]*>|"(?:[^"\\]|\\.)*"(?:\^\^<[^>]*>|@[\w-]+)?|_:\S+/g) ?? [line];
  return <code title={line} className="break-all text-xs">{parts.map((p, i) => <span key={i} className={i === 1 ? 'text-slate-500' : ''}>{i > 0 && ' '}{p.startsWith('<') ? short(p) : p.replace(/\^\^<[^>]*>/, '')}</span>)}</code>;
}

function ClashList({ clashes }: { clashes: Clash[] }) {
  return <ul className="space-y-2" aria-label="Clashes">{clashes.map((c, i) => <li key={i} className="rounded-lg border border-red-200 bg-white p-3 text-sm">
    <p className="text-red-900">{c.message}</p>
    {c.terms && c.terms.length > 0 && <p className="mt-1 text-xs text-slate-600">Involves {c.terms.map(t => <code key={t} className="term uri mr-1">{short(t)}</code>)}</p>}
    {c.premises && c.premises.length > 0 && <details className="mt-2"><summary className="cursor-pointer text-xs text-slate-600">{c.rule ? <>Rule <code>{c.rule}</code>, </> : null}{c.premises.length} triples</summary>
      <ul className="mt-1 space-y-1 pl-3">{c.premises.map(p => <li key={p}><Triple line={p} /></li>)}</ul></details>}
  </li>)}</ul>;
}

function PremiseItem({ premise }: { premise: Premise }) {
  const inJob = premise.deduced_in_job;
  return <li className="border-l-2 border-slate-200 pl-3">
    <Triple line={premise.triple} />
    <div className="mt-0.5 text-xs text-slate-500">
      {premise.asserted_in && <>Asserted in {premise.asserted_in.map(g => <code key={g} className="mr-1">{g}</code>)}</>}
      {premise.outside_input && <>From outside input {premise.outside_input.join(', ')} (not stored)</>}
      {inJob && <>Deduced first by this job, rule <code>{inJob.rule}</code>{inJob.truncated && ' (proof cut off here)'}</>}
      {premise.deduced_by && <>Deduced by an earlier job</>}
      {premise.origin && <>{premise.origin}</>}
    </div>
    {inJob?.premises && <ul className="mt-2 space-y-2">{inJob.premises.map((p, i) => <PremiseItem key={i} premise={p} />)}</ul>}
    {premise.deduced_by && <div className="mt-2 space-y-2">{premise.deduced_by.map((e, i) => <ExplanationView key={i} explanation={e} />)}</div>}
  </li>;
}
function ExplanationView({ explanation }: { explanation: Explanation }) {
  if (explanation.repeated) return <p className="text-xs text-slate-500">Job {explanation.job}: already shown above.</p>;
  if (explanation.explained === false) return <p className="text-xs text-slate-500">Job {explanation.job}: not followed further (depth limit).</p>;
  if (!explanation.supported) return <p className="text-xs text-slate-600">Job <strong>{explanation.job}</strong> deduced it, but its reasoner does not record why. It read {explanation.source_graphs?.join(', ')}.</p>;
  return <div className="space-y-2"><p className="break-all text-xs text-slate-500">Generation {explanation.generation ?? 'legacy'} · Reconstructed from current data; not a recorded historical proof.</p>{explanation.derivations?.map((d, i) => <div key={i} className="rounded-lg bg-slate-50 p-3">
    <p className="text-sm">Job <strong>{explanation.job}</strong>, rule <code className="rounded bg-white px-1">{typeof d.rule === 'number' ? `#${d.rule + 1}` : d.rule}</code>
      {d.rule_text && <span className="text-slate-500"> · {d.rule_text}</span>}
      {explanation.stale && <span className="ml-2 text-xs text-amber-800">job is stale; recomputed from current data</span>}</p>
    {d.aggregate && <p className="text-xs text-slate-500">An aggregate: every member counted is listed.</p>}
    {d.incomplete && <p className="text-xs text-amber-800">Some premises could not be traced.</p>}
    <ul className="mt-2 space-y-2">{d.premises.map((p, j) => <PremiseItem key={j} premise={p} />)}</ul>
  </div>)}</div>;
}

export default function InferencePanel() {
  const [jobs, setJobs] = useState<Job[]>([]), [states, setStates] = useState<Record<string, Status>>({});
  const [name, setName] = useState(''), [cascade, setCascade] = useState<Cascade>('');
  const [busy, setBusy] = useState(false), [error, setError] = useState(''), [notice, setNotice] = useState('');
  const [rebuilt, setRebuilt] = useState<Rebuilt | null>(null), [tests, setTests] = useState<TestResult[] | null>(null);
  const [rows, setRows] = useState<Record<string, Term>[]>([]), [why, setWhy] = useState<Why | null>(null);
  const [asked, setAsked] = useState(''), [typed, setTyped] = useState('');
  // `x sameAs x`, `C subClassOf C` and the like are true but rarely what someone is looking for.
  const [reflexive, setReflexive] = useState(false);
  const job = jobs.find(j => j.name === name), status = states[name];

  async function loadStates() {
    // One call for every job (the server limits concurrent operations), without re-reading outside
    // inputs, which can take seconds each. `checkInputs` does that for the selected job.
    const result = await invoke<{ jobs: (Status & { job: string })[] }>('status', { inputs: false });
    setStates(Object.fromEntries(result.jobs.map(s => [s.job, s])));
  }
  async function checkInputs(selected: Job) {
    if (!selected.inputs.length) return;
    const full = await invoke<Status>('status', { job: selected.name });
    setStates(current => ({ ...current, [selected.name]: full }));
  }
  async function loadTriples(selected: Job, withReflexive = reflexive) {
    // The graph name comes from the plugin and is built from a validated job name.
    const filter = withReflexive ? '' : 'FILTER(?s != ?o)';
    const result = await queryStore(`SELECT ?s ?p ?o WHERE { GRAPH <${selected.graph}> { ?s ?p ?o ${filter} } } ORDER BY ?s ?p ?o LIMIT 25`);
    setRows(result.data?.results?.bindings ?? []);
  }
  useEffect(() => {
    invoke<{ jobs: Job[] }>('jobs', {}).then(async result => {
      setJobs(result.jobs); setName(result.jobs[0]?.name ?? '');
      await loadStates();
    }).catch(e => setError(failure(e)));
  }, []);
  useEffect(() => {
    setRebuilt(null); setTests(null); setWhy(null); setAsked(''); setNotice(''); setRows([]);
    if (job) {
      loadTriples(job).catch(e => setError(failure(e)));
      checkInputs(job).catch(e => setError(failure(e)));
    }
  }, [name, jobs.length]);

  async function act(work: () => Promise<void>) {
    setBusy(true); setError(''); setNotice('');
    try { await work(); } catch (e) { setError(failure(e)); }
    finally { setBusy(false); }
  }
  const rebuild = () => act(async () => {
    const result = await invoke<Rebuilt>('rebuild', cascade ? { job: name, cascade } : { job: name });
    setRebuilt(result);
    setNotice(result.state === 'inconsistent' ? `Rebuild found the data inconsistent. The previous output of ${name} is unchanged.`
      : result.state === 'blocked' ? `Not rebuilt: upstream job ${result.blocked_by} is inconsistent.`
      : `Rebuilt ${name}: ${result.triples ?? 0} triples.`);
    await loadStates();
    if (job) { await checkInputs(job); await loadTriples(job); }
  });
  const runTests = () => act(async () => {
    const result = await invoke<{ passed: boolean; results: TestResult[] }>('test', { job: name });
    setTests(result.results);
    setNotice(result.passed ? `All ${result.results.length} rule tests passed.` : 'Some rule tests failed.');
  });
  const explain = (line: string) => act(async () => {
    setAsked(line); setWhy(null);
    setWhy(await invoke<Why>('explain', { triple: line }));
  });

  return <section className="card space-y-5 p-5" aria-label="Inference jobs">
    <div><h2 className="text-xl font-semibold">Inference</h2>
      <p className="mt-2 text-sm text-slate-600">Each job reads its source graphs, applies a reasoner, and publishes what it deduces to its own graph. Pick a job to rebuild it, run its rule tests, or ask why a deduced triple holds.</p></div>
    {error && <p role="alert" className="notice error break-words">{error}</p>}
    {notice && <p role="status" className="notice">{notice}</p>}
    <div className="overflow-x-auto"><table className="w-full text-left text-sm" aria-label="Inference job states">
      <thead className="text-xs text-slate-500"><tr><th className="p-2">Job</th><th className="p-2">State</th><th className="p-2">Reasoner</th><th className="p-2">Reads</th></tr></thead>
      <tbody>{jobs.map(j => <tr key={j.name} className={`cursor-pointer border-t border-slate-100 ${j.name === name ? 'bg-slate-50' : ''}`} onClick={() => setName(j.name)}>
        <td className="p-2"><button className="text-left font-medium underline-offset-2 hover:underline" aria-pressed={j.name === name} onClick={() => setName(j.name)}>{j.name}</button></td>
        <td className="p-2">{states[j.name] ? <StateBadge state={states[j.name].state} /> : <span className="text-xs text-slate-400">…</span>}</td>
        <td className="p-2 text-xs text-slate-600">{j.profile}</td>
        <td className="p-2 text-xs text-slate-600">{j.depends_on.length ? <>jobs {j.depends_on.join(', ')}</> : `${j.source_graphs.length} graph${j.source_graphs.length === 1 ? '' : 's'}`}{j.inputs.length > 0 && ` + ${j.inputs.join(', ')}`}</td>
      </tr>)}</tbody>
    </table></div>

    {job && <div className="space-y-4 rounded-lg border border-slate-200 p-4" aria-label={`Job ${job.name}`}>
      <div className="flex flex-wrap items-center gap-3">
        <h3 className="text-lg font-semibold">{job.name}</h3>
        {status && <StateBadge state={status.state} />}
        {status && <span className="text-sm text-slate-500">{STATE_HELP[status.state]}</span>}
      </div>
      <dl className="grid gap-x-6 gap-y-1 text-sm md:grid-cols-[auto_1fr]">
        <dt className="text-slate-500">Reasoner</dt><dd>{job.provider} · <code>{job.profile}</code>{job.rules > 0 && ` · ${job.rules} rules`}{job.explains ? ' · explains its deductions' : ''}</dd>
        <dt className="text-slate-500">Reads</dt><dd className="break-all">{job.source_graphs.join(', ')}{job.inputs.length > 0 && <> and outside input {job.inputs.join(', ')}</>}</dd>
        <dt className="text-slate-500">Publishes</dt><dd className="break-all"><code>{job.graph}</code>{status?.triples !== undefined && ` · ${status.triples} triples`}</dd>
        {status?.stale_upstream && status.stale_upstream.length > 0 && <><dt className="text-slate-500">Waiting on</dt><dd>{status.stale_upstream.join(', ')}</dd></>}
        {status?.stale_inputs && <><dt className="text-slate-500">Changed inputs</dt><dd>{status.stale_inputs.join(', ')}</dd></>}
        {status?.input_error && <><dt className="text-slate-500">Input error</dt><dd className="text-red-800">{status.input_error}</dd></>}
        {status?.inputs_checked === false && <><dt className="text-slate-500">Outside inputs</dt><dd className="text-slate-600">Not re-read yet, so a change there would not show. Checking…</dd></>}
      </dl>
      <p className="text-xs text-slate-500">Ready means current output. No clash found is not proof of consistency; missing facts are not validation failures. Use SHACL for required fields.</p>
      {status?.generation && <p className="break-all text-xs text-slate-500">Generation: {status.generation}</p>}
      {status?.metadata?.diagnostics && <div className="rounded-lg bg-amber-50 p-3 text-xs text-amber-900" aria-label="Inference diagnostics">
        <p>Datatype checks: {status.metadata.diagnostics.datatype_checks ?? 'provider-defined'}</p>
        {status.metadata.diagnostics.issues?.map(issue => <p key={issue}>{issue}</p>)}
        {Boolean(status.metadata.diagnostics.unchecked_datatypes?.length) && <p className="break-all">Unchecked datatypes: {status.metadata.diagnostics.unchecked_datatypes?.join(', ')}</p>}
      </div>}
      {status?.clashes && <div><h4 className="mb-2 text-sm font-semibold">What clashed</h4><ClashList clashes={status.clashes} /></div>}

      <div className="flex flex-wrap items-center gap-2">
        <button className="btn primary" disabled={busy} onClick={() => void rebuild()}>Rebuild</button>
        <label className="text-sm text-slate-600">with
          <select aria-label="Cascade" value={cascade} disabled={busy} onChange={e => setCascade(e.target.value as Cascade)} className="ml-2 rounded-lg border border-slate-300 bg-white p-1.5 text-sm">
            <option value="">this job only</option><option value="upstream">jobs it reads first</option>
            <option value="downstream">jobs that read it after</option><option value="both">both</option>
          </select></label>
        {job.tests > 0 && <button className="btn" disabled={busy} onClick={() => void runTests()}>Run {job.tests} rule test{job.tests === 1 ? '' : 's'}</button>}
        <button className="btn" disabled={busy} onClick={() => void act(async () => { await loadStates(); await checkInputs(job); await loadTriples(job); })}>Refresh</button>
        {busy && <span className="text-sm text-slate-500" role="status">Working…</span>}
      </div>

      {rebuilt?.clashes && <div><h4 className="mb-2 text-sm font-semibold">What clashed</h4><ClashList clashes={rebuilt.clashes} /></div>}
      {rebuilt?.cascade && rebuilt.cascade.steps.length > 0 && <ol className="space-y-1 text-sm" aria-label="Cascade steps">{rebuilt.cascade.steps.map(s => <li key={s.job}>
        <strong>{s.job}</strong>: {s.action}{s.blocked_by && ` by ${s.blocked_by}`}</li>)}</ol>}

      {tests && <ul className="space-y-2" aria-label="Rule test results">{tests.map(t => <li key={t.name} className={`rounded-lg p-3 text-sm ${t.passed ? 'bg-emerald-50' : 'bg-red-50'}`}>
        <strong>{t.passed ? 'Passed' : 'Failed'}</strong>: {t.name}{t.inconsistent && ' (found inconsistent, as expected)'}
        {t.error && <p className="mt-1 text-red-800">{t.error}</p>}
        {[['Missing', t.missing], ['Present but forbidden', t.present_but_forbidden], ['Unexpected', t.unexpected]].map(([label, lines]) =>
          (lines as string[] | undefined)?.length ? <div key={label as string} className="mt-1"><span className="text-xs text-slate-600">{label as string}</span>
            <ul className="pl-3">{(lines as string[]).map(l => <li key={l}><Triple line={l} /></li>)}</ul></div> : null)}
      </li>)}</ul>}

      <div>
        <h4 className="text-sm font-semibold">Deduced triples</h4>
        <p className="text-xs text-slate-500">The first 25 in {job.graph}.{job.explains ? ' Choose “Why?” to see the rule and the facts it used.' : ' This reasoner does not record why; “Why?” shows which job and graphs.'}</p>
        <label className="mt-1 flex items-center gap-2 text-xs text-slate-600"><input type="checkbox" checked={reflexive} disabled={busy}
          onChange={e => { setReflexive(e.target.checked); void act(() => loadTriples(job, e.target.checked)); }} />
          Include triples that relate something to itself (x sameAs x, C subClassOf C)</label>
        {rows.length === 0 && <p className="mt-2 text-sm text-slate-500">Nothing published yet. Rebuild the job.</p>}
        <ul className="mt-2 divide-y divide-slate-100" aria-label="Deduced triples">{rows.map((row, i) => {
          const [s, p, o] = [nt(row.s), nt(row.p), nt(row.o)];
          const line = s && p && o ? `${s} ${p} ${o} .` : null;
          return <li key={i} className="flex items-start gap-3 py-1.5">
            <span className="min-w-0 flex-1">{line ? <Triple line={line} /> : <span className="text-xs text-slate-400">a triple with a blank node</span>}</span>
            {line && <button className="btn shrink-0 py-0.5 text-xs" disabled={busy} onClick={() => void explain(line)} aria-label={`Why ${short(row.s.value)} ${short(row.p.value)} ${short(row.o.value)}`}>Why?</button>}
          </li>;
        })}</ul>
      </div>
      <form className="flex flex-wrap gap-2" onSubmit={e => { e.preventDefault(); if (typed.trim()) void explain(typed.trim()); }}>
        <label className="min-w-0 flex-1 text-sm">Explain any triple (one N-Triples line)
          <input aria-label="Triple to explain" value={typed} onChange={e => setTyped(e.target.value)} placeholder="<https://…/subject> <https://…/predicate> <https://…/object> ."
            className="mt-1 w-full rounded-lg border border-slate-300 p-2 font-mono text-xs" /></label>
        <button className="btn self-end" disabled={busy || !typed.trim()}>Explain</button>
      </form>
      {why && <div className="space-y-3 rounded-lg border border-slate-200 p-3" aria-label="Explanation">
        <p className="text-sm font-semibold">Why <Triple line={asked} /></p>
        {!why.derived && <p className="text-sm text-slate-600">{why.asserted_in?.length ? <>Not deduced: it is asserted in {why.asserted_in.join(', ')}.</> : 'No job deduced this triple, and no graph holds it.'}</p>}
        {why.explanations.map((e, i) => <ExplanationView key={i} explanation={e} />)}
        {why.truncated && <p className="text-xs text-amber-800">The trail was cut off at the lookup limit.</p>}
      </div>}
    </div>}
  </section>;
}
