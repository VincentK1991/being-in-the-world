// Real browser + real persistent database. No mocked API responses.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const directory = await mkdtemp(resolve(tmpdir(), 'rdf-studio-test-'));
const socket = createServer();
await new Promise(resolve => socket.listen(0, '127.0.0.1', resolve));
const port = socket.address().port;
await new Promise(resolve => socket.close(resolve));
const base = `http://127.0.0.1:${port}`;
let server, frontend, browser, logs = '', uiBase = base;
const semanticCollections = [];
const semantic = process.argv.includes('--semantic');
const inference = process.argv.includes('--inference');
const rules = process.argv.includes('--rules');
async function start() {
  server = spawn(resolve(root, 'target/debug/rdftool'), ['serve', '--db', resolve(directory, 'db'), '--bind', `127.0.0.1:${port}`, ...(semantic ? ['--semantic-config', resolve(root, 'plugins/semantic-search/example-config.json')] : []), ...((inference || rules) ? ['--inference-config', resolve(root, rules ? 'plugins/inference/example-rules-config.json' : 'plugins/inference/example-config.json')] : [])], { stdio: ['ignore', 'pipe', 'pipe'] });
  server.stderr.on('data', data => { logs += data; });
  let error;
  server.on('error', value => { error = value; });
  for (let i = 0; i < 100; i++) {
    if (error) throw error;
    if (server.exitCode !== null) throw new Error(logs);
    try { if ((await fetch(`${base}/health`)).ok) return; } catch { /* Wait for bind. */ }
    await delay(100);
  }
  throw new Error(`Server did not start: ${logs}`);
}
async function stop() {
  if (!server || server.exitCode !== null) return;
  await new Promise(resolve => { server.once('exit', resolve); server.kill('SIGTERM'); });
}
try {
  await start();
  if (process.argv.includes('--dev')) {
    const reservation = createServer();
    await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve));
    const frontendPort = reservation.address().port;
    await new Promise(resolve => reservation.close(resolve));
    uiBase = `http://127.0.0.1:${frontendPort}`;
    frontend = spawn(process.execPath, ['node_modules/astro/bin/astro.mjs', 'dev', '--host', '127.0.0.1', '--port', String(frontendPort)], { cwd: resolve(root, 'apps/studio'), env: { ...process.env, RDF_BACKEND_URL: base }, stdio: ['ignore', 'pipe', 'pipe'] });
    frontend.stdout.on('data', data => { logs += data; });
    frontend.stderr.on('data', data => { logs += data; });
    for (let i = 0; i < 100; i++) {
      if (frontend.exitCode !== null) throw new Error(logs);
      try { if ((await fetch(`${uiBase}/health`)).ok) break; } catch { /* Wait for proxy. */ }
      if (i === 99) throw new Error(`Frontend did not start: ${logs}`);
      await delay(100);
    }
  }
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const text = async (selector, expected) => {
    await page.waitForFunction(({ selector, expected }) => document.querySelector(selector)?.textContent.includes(expected), { selector, expected });
  };
  const query = async sparql => {
    await page.locator('#query').fill(sparql);
    await page.locator('#run-query').click();
    await text('#result-status', 'completed successfully');
    assert.equal(await page.locator('#run-query').isEnabled(), true);
  };
  await page.goto(`${uiBase}/workspace`);
  await text('#connection-label', 'Store connected');
  await text('#plugins', 'dataset-summary');
  await page.locator('#load-example').click();
  await text('#quad-count', '14');
  await page.locator('#run-query').click();
  await text('#result-count', '3 rows');
  assert.equal(await page.locator('tbody tr').count(), 3);
  await text('tbody', 'Alice');
  await page.locator('#raw-tab').click();
  await text('#raw-view', 'bindings');
  const downloadPromise = page.waitForEvent('download');
  await page.locator('#download').click();
  assert.equal((await downloadPromise).suggestedFilename(), 'results.json');
  await page.locator('#table-tab').click();
  await text('#plugins', 'custom-properties');
  await text('#plugins', 'urn:rdftool:property:tokens');
  await page.locator('#template').selectOption('properties');
  await page.locator('#run-query').click();
  await text('#result-status', 'completed successfully');
  await text('tbody', 'semantic');
  assert.equal(await page.locator('tbody tr').count(), 3);

  await page.locator('#open-upload').click();
  await page.locator('#target-graph').fill('https://example.org/upload');
  await page.locator('#rdf-file').setInputFiles({ name: 'people.ttl', mimeType: 'text/turtle', buffer: Buffer.from('<urn:person> <urn:name> "Uploaded person" .') });
  await page.locator('#submit-upload').click();
  await text('#upload-message', 'Imported people.ttl');
  await text('#quad-count', '15');
  await page.locator('#rdf-file').setInputFiles({ name: 'broken.ttl', mimeType: 'text/turtle', buffer: Buffer.from('<urn:new> <urn:p> "must roll back" .\n<broken') });
  await page.locator('#submit-upload').click();
  await text('#upload-message', 'Import failed');
  assert.equal((await (await fetch(`${base}/api/status`)).json()).quads, 15);
  await page.locator('#cancel-upload').click();
  await page.locator('#graph-list button').click();
  await page.locator('#run-query').click();
  await text('#result-count', '1 rows');
  await text('tbody', 'Uploaded person');
  await query('ASK { <urn:new> ?p ?o }');
  await text('.boolean-result', 'FALSE');
  await query('CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o } LIMIT 2');
  await text('#result-count', 'Graph');
  assert.ok((await page.locator('.graph-result').textContent()).length > 10);
  await query('SELECT ?value ?unbound WHERE { VALUES ?value { "<img src=x onerror=alert(1)>" "duplicate" "duplicate" } OPTIONAL { <urn:absent> <urn:p> ?unbound } }');
  assert.equal(await page.locator('tbody tr').count(), 3);
  assert.equal(await page.locator('tbody img').count(), 0);
  assert.equal(await page.locator('.unbound').count(), 3);
  await query(`SELECT ?n WHERE { VALUES ?n { ${Array.from({ length: 55 }, (_, i) => i).join(' ')} } } ORDER BY ?n`);
  assert.equal(await page.locator('tbody tr').count(), 50);
  await page.locator('#next').click();
  assert.equal(await page.locator('tbody tr').count(), 5);
  await page.locator('#query').fill('SELECT broken syntax');
  await page.locator('#run-query').click();
  await text('#query-error', 'HTTP 400');
  assert.equal(await page.locator('#download').isDisabled(), true);
  await query('SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10');
  await mkdir(resolve(root, 'target/ui'), { recursive: true });
  await page.screenshot({ path: resolve(root, 'target/ui/desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: resolve(root, 'target/ui/mobile.png'), fullPage: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'no page-wide mobile overflow');
  await stop();
  await start();
  await page.reload();
  await text('#quad-count', '15');
  assert.ok(await page.locator('#history button').count() > 0);
  await query('SELECT ?name WHERE { GRAPH <https://example.org/upload> { ?s <urn:name> ?name } }');
  await text('tbody', 'Uploaded person');
  assert.deepEqual(errors, []);
  if (semantic) {
    await page.setViewportSize({ width: 1440, height: 1100 });
    await text('#plugins', 'semantic-search');
    await page.locator('#open-upload').click();
    await page.locator('#target-graph').fill('');
    await page.locator('#rdf-file').setInputFiles(resolve(root, 'plugins/semantic-search/example.ttl'));
    await page.locator('#submit-upload').click();
    await text('#upload-message', 'Imported example.ttl');
    await page.locator('#cancel-upload').click();
    const response = await fetch(`${base}/api/plugins/semantic-search/operations/rebuild`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ index: 'descriptions' }) });
    assert.equal(response.status, 200, await response.clone().text());
    const built = await response.json();
    semanticCollections.push(built.collection);
    if (process.env.REQUIRE_CUDA === '1') assert.equal(built.device, 'cuda');
    await page.locator('#template').selectOption('semantic');
    await page.locator('#run-query').click();
    await text('#result-status', 'completed successfully');
    await text('tbody tr:first-child', 'https://example.org/car');
    await page.screenshot({ path: resolve(root, 'target/ui/semantic.png'), fullPage: true });
    console.log(`PASS: real semantic upload/rebuild/property query through UI and HTTP; embeddings device=${built.device}.`);
  }
  if (inference) {
    await page.setViewportSize({ width: 1440, height: 1100 });
    await text('#plugins', 'inference');
    await page.locator('#open-upload').click();
    await page.locator('#target-graph').fill('');
    await page.locator('#rdf-file').setInputFiles(resolve(root, 'plugins/inference/example.trig'));
    await page.locator('#submit-upload').click();
    await text('#upload-message', 'Imported example.trig');
    await page.locator('#cancel-upload').click();
    const response = await fetch(`${base}/api/plugins/inference/operations/rebuild`, {method:'POST', headers:{'Content-Type':'application/json'},body:JSON.stringify({job:'vehicles'})});
    assert.equal(response.status, 200, await response.clone().text());
    await query('SELECT ?vehicle WHERE { GRAPH <urn:rdftool:derived:inference:vehicles> { ?vehicle a <https://example.org/Vehicle> } }');
    await text('tbody', 'https://example.org/car1');
    assert.equal(await page.locator('tbody tr').count(), 1);
    await page.locator('#refresh').click();
    await text('#graph-list', 'urn:rdftool:derived:inference:vehicles');
    await page.screenshot({path:resolve(root,'target/ui/inference.png'),fullPage:true});
    assert.deepEqual(errors, []);
    console.log('PASS: real OWL-RL TriG upload, HTTP rebuild and inferred SPARQL results through Studio.');
  }
  if (rules) {
    await page.setViewportSize({width:1440,height:1100});
    await text('#plugins','inference');
    await page.locator('#open-upload').click();
    await page.locator('#target-graph').fill('');
    await page.locator('#rdf-file').setInputFiles(resolve(root,'plugins/inference/examples/rules/data.trig'));
    await page.locator('#submit-upload').click();
    await text('#upload-message','Imported data.trig');
    await page.locator('#cancel-upload').click();
    for (const job of ['work-sparql','work-jena','totals']) {
      const response = await fetch(`${base}/api/plugins/inference/operations/rebuild`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({job})});
      assert.equal(response.status,200,await response.clone().text());
    }
    await query('SELECT ?person ?country WHERE { GRAPH <urn:rdftool:derived:inference:work-sparql> { ?person <https://example.org/worksInCountry> ?country } } ORDER BY ?person');
    await text('tbody','https://example.org/France');
    assert.equal(await page.locator('tbody tr').count(),2);
    await query('SELECT ?person ?continent WHERE { GRAPH <urn:rdftool:derived:inference:work-jena> { ?person <https://example.org/worksOnContinent> ?continent } } ORDER BY ?person');
    await text('tbody','https://example.org/Europe');
    assert.equal(await page.locator('tbody tr').count(),2);
    await page.locator('#refresh').click();
    await text('#graph-list','urn:rdftool:derived:inference:work-jena');
    await page.setViewportSize({width:1440,height:1100});
    await page.screenshot({path:resolve(root,'target/ui/rules.png'),fullPage:true});
    assert.deepEqual(errors,[]);
    console.log('PASS: SPARQL and real Jena rule providers through upload, generic HTTP rebuild and Studio queries.');
  }
  console.log('PASS: example, upload, named graph, rollback, SELECT/ASK/CONSTRUCT, repeated queries, raw/download, safe RDF rendering, unbound/duplicate rows, pagination, errors, mobile layout, history, shutdown/reopen.');
} finally {
  await browser?.close();
  if (frontend && frontend.exitCode === null) await new Promise(resolve => { frontend.once('exit', resolve); frontend.kill('SIGTERM'); });
  await stop();
  await rm(directory, { recursive: true, force: true });
  if (semanticCollections.length) {
    const qdrant = 'http://127.0.0.1:16333';
    const aliases = await (await fetch(`${qdrant}/aliases`)).json();
    for (const alias of aliases.result.aliases) {
      if (semanticCollections.includes(alias.collection_name)) await fetch(`${qdrant}/collections/aliases`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ actions: [{ delete_alias: { alias_name: alias.alias_name } }] }) });
    }
    for (const collection of semanticCollections) await fetch(`${qdrant}/collections/${collection}`, { method: 'DELETE' });
  }
}
