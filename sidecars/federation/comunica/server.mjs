// SPARQL federation sidecar. Comunica evaluates standard SERVICE clauses; this wrapper only decides
// what may be contacted. Every SERVICE target and the local source must appear in the allowlist,
// `SERVICE ?var`, updates and anything that is not a read query are refused, and redirects are not followed.
import http from 'node:http';
import { readFileSync } from 'node:fs';
import { QueryEngine } from '@comunica/query-sparql';
import sparqljs from 'sparqljs';
import { Store } from 'n3';

const config = JSON.parse(readFileSync(process.env.FEDERATION_CONFIG ?? '/config/federation.json', 'utf8'));
const allow = new Set(config.allow);
const maxBody = config.max_body_bytes ?? 1 << 20, timeout = config.timeout_ms ?? 60000;
const debug = process.env.FEDERATION_DEBUG === '1';
const engine = new QueryEngine(), parser = new sparqljs.Parser();

class Refused extends Error { constructor(status, message) { super(message); this.status = status; } }

function serviceTargets(node, found = []) {
  if (Array.isArray(node)) node.forEach(child => serviceTargets(child, found));
  else if (node && typeof node === 'object') {
    if (node.type === 'service') found.push(node.name);
    Object.values(node).forEach(child => serviceTargets(child, found));
  }
  return found;
}
// Comunica ignores a GRAPH block inside SERVICE (it counts the default graph instead), so rows silently
// vanish. The same pattern inside a sub-select is evaluated correctly, so wrap such SERVICE bodies in one.
const hasGraph = node => Array.isArray(node) ? node.some(hasGraph)
  : node && typeof node === 'object' ? node.type === 'graph' || Object.values(node).some(hasGraph) : false;
function wrapGraphs(node) {
  let changed = false;
  const visit = value => {
    if (Array.isArray(value)) value.forEach(visit);
    else if (value && typeof value === 'object') {
      if (value.type === 'service' && hasGraph(value.patterns) && !(value.patterns.length === 1 && value.patterns[0].type === 'query')) {
        value.patterns = [{ type: 'query', queryType: 'SELECT', variables: [new sparqljs.Wildcard()], where: value.patterns, prefixes: {} }];
        changed = true;
      } else Object.values(value).forEach(visit);
    }
  };
  visit(node);
  return changed;
}
function checked(query, source) {
  let parsed;
  try { parsed = parser.parse(query); } catch (error) { throw new Refused(400, `SPARQL syntax error: ${error.message}`); }
  if (parsed.type !== 'query') throw new Refused(400, 'only read queries are accepted');
  if (parsed.from?.default?.length || parsed.from?.named?.length) throw new Refused(400, 'FROM and FROM NAMED are not accepted');
  const targets = serviceTargets(parsed);
  for (const name of targets) {
    if (name.termType !== 'NamedNode') throw new Refused(403, 'SERVICE needs a registered endpoint IRI, not a variable');
    if (!allow.has(name.value)) throw new Refused(403, `endpoint is not registered for federation: ${name.value}`);
  }
  if (source && !allow.has(source)) throw new Refused(403, `local source is not registered for federation: ${source}`);
  const text = wrapGraphs(parsed) ? new sparqljs.Generator().stringify(parsed) : query;
  return { parsed, text, services: targets.map(t => t.value) };
}
const read = request => new Promise((resolve, reject) => {
  const chunks = []; let size = 0;
  request.on('data', chunk => { size += chunk.length; if (size > maxBody) { reject(new Refused(413, 'request too large')); request.destroy(); } else chunks.push(chunk); });
  request.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
  request.on('error', reject);
});
const send = (response, status, body, type = 'application/json') => { response.writeHead(status, { 'Content-Type': type }); response.end(typeof body === 'string' ? body : JSON.stringify(body)); };

// Comunica's `httpRequests` is a counter for the whole process, so it mixes every dataset's queries.
function withoutCounters(text) {
  try { const json = JSON.parse(text); delete json.metadata?.httpRequests; if (json.metadata && !Object.keys(json.metadata).length) delete json.metadata; return JSON.stringify(json); } catch { return text; }
}
const GRAPH_TYPES = new Set(['text/turtle', 'application/n-triples']);
async function run(query, source, accept) {
  const { parsed, text: rewritten, services } = checked(query, source);
  // Comunica pushes the whole query to a lone SPARQL source, which would send SERVICE clauses to the
  // local endpoint (it rejects them). An always-empty second source keeps evaluation in Comunica:
  // patterns outside SERVICE see only `source`, and SERVICE clauses go to their own endpoints.
  const sources = [new Store()];
  if (source) sources.unshift({ type: 'sparql', value: source });
  else if (!services.length) throw new Refused(400, 'query has no SERVICE clause and no local source');
  const signal = AbortSignal.timeout(timeout);
  const context = { sources, httpTimeout: timeout, httpRetryCount: 0, httpRetryOnServerError: false, lenient: false, httpAbortSignal: signal, fetch: (input, init) => {
    if (debug) console.log('outbound', init?.method ?? 'GET', String(input?.url ?? input).slice(0, 600), decodeURIComponent(String(init?.body ?? '').replace(/\+/g, ' ')).slice(0, 700));
    return fetch(input, { ...init, redirect: 'error', signal });
  } };
  const result = await engine.query(rewritten, context);
  const kind = parsed.queryType;
  const media = kind === 'CONSTRUCT' || kind === 'DESCRIBE' ? (GRAPH_TYPES.has(accept) ? accept : 'text/turtle') : 'application/sparql-results+json';
  const { data } = await engine.resultToString(result, media);
  let text = ''; for await (const chunk of data) text += chunk;
  if (media === 'application/sparql-results+json') text = withoutCounters(text);
  return { text, media, services };
}

const server = http.createServer(async (request, response) => {
  try {
    const url = new URL(request.url, 'http://x');
    if (url.pathname === '/health') return send(response, 200, { status: 'ok', engine: 'comunica', allowed: allow.size });
    if (url.pathname === '/allowed') return send(response, 200, { allow: [...allow] });
    if (url.pathname !== '/sparql' || request.method !== 'POST') throw new Refused(404, 'POST /sparql');
    const body = JSON.parse(await read(request));
    if (typeof body.query !== 'string') throw new Refused(400, 'expected {"query": "...", "source": "..."?}');
    const { text, media, services } = await run(body.query, body.source, body.accept);
    response.setHeader('X-Federation-Services', services.join(' '));
    send(response, 200, text, media);
  } catch (error) {
    const status = error instanceof Refused ? error.status : error.name === 'TimeoutError' ? 504 : 502;
    send(response, status, { error: String(error.message ?? error) });
  }
});
// Clients pool connections; Node's 5 s default would close them while a client is about to reuse one.
server.keepAliveTimeout = 65000; server.headersTimeout = 66000;
server.listen(8095, '0.0.0.0', () => console.log('federation sidecar listening on :8095'));
