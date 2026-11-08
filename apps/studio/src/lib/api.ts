import { datasetBase } from './dataset';
export interface Status { connected: boolean; quads: number; graphs: string[]; max_upload_bytes: number; engine: string }
export interface Term { type: string; value: string; datatype?: string; 'xml:lang'?: string }
export interface Results { head?: { vars: string[] }; results?: { bindings: Record<string, Term>[] }; boolean?: boolean }
export interface QueryResponse { text: string; mime: string; data: Results | null }
export interface Property { iri: string; description: string; input_arity: number; output_arity: number }
export interface Plugin { id: string; version: string; sdk_version: number; operations: string[]; properties: Property[] }
export async function checked(response: Response): Promise<Response> {
  if (!response.ok) throw new Error(`HTTP ${response.status} · ${await response.text() || response.statusText}`);
  return response;
}
export async function getStatus(): Promise<Status> { return (await checked(await fetch(`${datasetBase()}/api/status`, { cache: 'no-store' }))).json(); }
export async function getPlugins(): Promise<Plugin[]> { return (await checked(await fetch(`${datasetBase()}/api/plugins`))).json(); }
export async function queryStore(query: string): Promise<QueryResponse> {
  const response = await checked(await fetch(`${datasetBase()}/query`, { method: 'POST', headers: { 'Content-Type': 'application/sparql-query', Accept: 'application/sparql-results+json, text/turtle' }, body: query }));
  const text = await response.text(), mime = response.headers.get('content-type') || 'text/plain';
  return { text, mime, data: mime.includes('json') ? JSON.parse(text) as Results : null };
}
/** Runs a read query through a plugin (`federation` for SERVICE, `virtual-graph` for Ontop). */
export async function queryVia(plugin: string, query: string): Promise<QueryResponse> {
  const json = await (await checked(await fetch(`${datasetBase()}/api/plugins/${plugin}/operations/query`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ query }) }))).json() as Record<string, unknown>;
  if (typeof json.turtle === 'string') return { text: json.turtle, mime: 'text/turtle', data: null };
  return { text: JSON.stringify(json, null, 2), mime: 'application/json', data: json as Results };
}
export async function pluginStatus(plugin: string): Promise<Record<string, unknown>> {
  return (await checked(await fetch(`${datasetBase()}/api/plugins/${plugin}/operations/status`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }))).json();
}
export async function importFile(file: File, graph: string, limit: number): Promise<{ parsed_quads: number; total_quads: number }> {
  if (file.size > limit) throw new Error(`File exceeds the ${limit / 1024 / 1024} MiB upload limit.`);
  const extension = file.name.split('.').pop()?.toLowerCase() || '';
  const types: Record<string, string> = { ttl: 'text/turtle', nt: 'application/n-triples', trig: 'application/trig', nq: 'application/n-quads' };
  if (!types[extension]) throw new Error('Choose a .ttl, .nt, .trig, or .nq file.');
  if (graph && ['trig', 'nq'].includes(extension)) throw new Error('TriG and N-Quads define their own graphs. Leave target graph empty.');
  return (await checked(await fetch(`${datasetBase()}/api/import${graph ? `?${new URLSearchParams({ graph })}` : ''}`, { method: 'POST', headers: { 'Content-Type': types[extension] }, body: file }))).json();
}
