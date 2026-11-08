import { useEffect, useState } from 'react';
import AppSidebar from './AppSidebar';
import InferencePanel from './InferencePanel';
import { getPlugins, type Plugin, type Property } from '../lib/api';
import { selectedDataset } from '../lib/dataset';

const failure = (error: unknown) => error instanceof Error ? error.message : String(error);
const local = (iri: string) => iri.slice(Math.max(iri.lastIndexOf(':'), iri.lastIndexOf('/'), iri.lastIndexOf('#')) + 1);
// A property function is used as `(?out1 ?out2) <iri> (in1 in2)`; show that shape with the arities the plugin declares.
const usage = (p: Property) => {
  const out = Array.from({length: p.output_arity}, (_, i) => `?out${i + 1}`), inputs = Array.from({length: p.input_arity}, (_, i) => `input${i + 1}`);
  return `${out.length > 1 ? `(${out.join(' ')})` : out[0] ?? '?s'} <${p.iri}> ${inputs.length > 1 ? `(${inputs.join(' ')})` : inputs[0] ?? '""'} .`;
};
const plural = (n: number, word: string) => `${n} ${n === 1 ? word : word.replace(/y$/, 'ie') + 's'}`;

// Lists the plugins loaded for the selected dataset, with the operations and property-function parameters each one offers.
export default function PluginsPage() {
  const [plugins, setPlugins] = useState<Plugin[] | null>(null), [error, setError] = useState(''), [dataset, setDataset] = useState('default');
  useEffect(() => {
    setDataset(selectedDataset() || 'default');
    getPlugins().then(setPlugins).catch(e => setError(failure(e)));
  }, []);
  const connection = error ? 'Connection unavailable' : plugins ? '● Store connected' : 'Connecting…';
  return <div className="app-body flex">
    <AppSidebar active="plugins" connectionId="plugins-connection" connection={connection} />
    <main className="min-w-0 flex-1 bg-slate-50">
      <div className="mx-auto max-w-5xl space-y-6 p-4 md:p-10">
        <header>
          <h1 className="text-3xl font-semibold tracking-tight">Plugins</h1>
          <p className="mt-2 max-w-2xl text-sm text-slate-500">Loaded for dataset <strong>{dataset}</strong>. Operations run through the API; properties are called from SPARQL with the inputs and outputs listed below.</p>
        </header>
        {error && <p role="alert" className="notice error">{error}</p>}
        {!plugins && !error && <p className="text-sm text-slate-500">Loading…</p>}
        {plugins?.some(p => p.id === 'inference' && p.operations.includes('jobs')) && <InferencePanel />}
        {plugins && !plugins.length && <p className="card p-5 text-sm text-slate-500">No plugins are loaded for this dataset.</p>}
        <ul id="plugin-list" className="space-y-5">{plugins?.map(p => <li key={p.id} className="card p-5" aria-label={`Plugin ${p.id}`}>
          <div className="flex flex-wrap items-baseline gap-x-3">
            <h2 className="text-lg font-semibold">{p.id}</h2><span className="text-xs text-slate-400">v{p.version} · SDK {p.sdk_version}</span>
            <span className="ml-auto text-xs text-slate-500">{plural(p.operations.length, 'operation')} · {plural(p.properties.length, 'property')}</span>
          </div>
          {p.operations.length > 0 && <div className="mt-4"><h3 className="text-sm font-semibold">Operations</h3>
            <ul className="mt-2 space-y-1">{p.operations.map(op => <li key={op} className="text-sm"><code className="rounded bg-slate-100 px-1.5 py-0.5 text-xs">POST /api/plugins/{p.id}/operations/{op}</code></li>)}</ul></div>}
          {p.properties.length > 0 && <div className="mt-4"><h3 className="text-sm font-semibold">Properties</h3>
            <ul className="mt-2 divide-y divide-slate-100">{p.properties.map(prop => <li key={prop.iri} className="py-3">
              <div className="flex flex-wrap items-baseline gap-x-3"><strong className="text-sm">{local(prop.iri)}</strong><span className="text-xs text-slate-500">{plural(prop.input_arity, 'input')} → {plural(prop.output_arity, 'output')}</span></div>
              <code className="mt-1 block break-all text-xs text-slate-500">{prop.iri}</code>
              <p className="mt-1 text-sm text-slate-600">{prop.description}</p>
              <pre className="mt-2 overflow-x-auto rounded-lg bg-slate-900 p-3 text-xs text-emerald-100">{usage(prop)}</pre>
            </li>)}</ul></div>}
          {!p.operations.length && !p.properties.length && <p className="mt-3 text-sm text-slate-500">This plugin exposes no operations or properties.</p>}
        </li>)}</ul>
      </div>
    </main>
  </div>;
}
