import { checked } from './api';

// The selected dataset lives in localStorage so every page and reload agrees on it. Nothing selected
// means the server's default dataset, reached through the unprefixed API paths.
const KEY = 'rdf-studio:dataset';
export interface DatasetInfo { name: string; id: string; running: boolean; default: boolean; quads: number | null; graphs: number | null }
export interface DatasetList { default: string; datasets: DatasetInfo[] }

export function selectedDataset(): string { try { return localStorage.getItem(KEY) ?? ''; } catch { return ''; } }
/** Path prefix for dataset-scoped API calls: `/d/<name>` or empty for the default dataset. */
export function datasetBase(): string { const name = selectedDataset(); return name ? `/d/${encodeURIComponent(name)}` : ''; }
/** Switches dataset and reloads, so every component starts clean against the new dataset. */
export function selectDataset(name: string) {
  try { if (name) localStorage.setItem(KEY, name); else localStorage.removeItem(KEY); } catch { /* storage unavailable */ }
  location.reload();
}
export async function listDatasets(): Promise<DatasetList> { return (await checked(await fetch('/api/datasets', { cache: 'no-store' }))).json(); }
export async function createDataset(name: string): Promise<DatasetInfo> {
  return (await checked(await fetch('/api/datasets', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name }) }))).json();
}
/** Returns a warning when the dataset was deleted but plugin cleanup (e.g. Qdrant) failed. */
export async function deleteDataset(name: string): Promise<string | null> {
  const result = await (await checked(await fetch(`/api/datasets/${encodeURIComponent(name)}`, { method: 'DELETE' }))).json() as { cleanup_warning: string | null };
  return result.cleanup_warning;
}
export const nameHint = 'Use 1–64 letters, digits, "-", "_" or ".", not starting with ".".';
export const validName = (name: string) => /^[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}$/.test(name);
