// Validate the running Docker deployment. Uses one temporary named graph.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdir } from 'node:fs/promises';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const base = 'http://127.0.0.1:4321';
const graph = `urn:local:browser-check:${crypto.randomUUID()}`;
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const text = (selector, expected) => page.waitForFunction(
    ({selector, expected}) => document.querySelector(selector)?.textContent.includes(expected),
    {selector, expected});
  await page.goto(`${base}/workspace`);
  await text('#connection-label', 'Store connected');
  await text('#plugins', 'inference');
  await text('#plugins', 'semantic-search');
  await page.locator('#open-upload').click();
  await page.locator('#target-graph').fill(graph);
  await page.locator('#rdf-file').setInputFiles({
    name: 'deployment-check.ttl', mimeType: 'text/turtle',
    buffer: Buffer.from('<urn:deployment:sample> <urn:deployment:label> "Docker upload verified" .')
  });
  await page.locator('#submit-upload').click();
  await text('#upload-message', 'Imported deployment-check.ttl');
  await page.locator('#cancel-upload').click();
  await page.locator('#query').fill(`SELECT ?label WHERE { GRAPH <${graph}> { <urn:deployment:sample> <urn:deployment:label> ?label } }`);
  await page.locator('#run-query').click();
  await text('#result-status', 'completed successfully');
  await text('tbody', 'Docker upload verified');
  await page.locator('#query').fill('SELECT ?person ?continent WHERE { GRAPH <urn:rdftool:derived:inference:work-jena> { ?person <https://example.org/worksOnContinent> ?continent } } ORDER BY ?person');
  await page.locator('#run-query').click();
  await text('tbody', 'Europe');
  await text('tbody', 'NorthAmerica');
  assert.equal(await page.locator('tbody tr').count(), 2);
  assert.deepEqual(errors, []);
  await mkdir('target/local-deployment', {recursive: true});
  await page.screenshot({ path: 'target/local-deployment/studio.png', fullPage: true });
  console.log('PASS: deployed Studio connected, uploaded Turtle, queried it, and displayed Jena deductions; no browser errors.');
} finally {
  await browser.close();
  const result = await fetch(`${base}/update`, {
    method: 'POST', headers: {'Content-Type': 'application/sparql-update'},
    body: `DROP SILENT GRAPH <${graph}>`
  });
  assert.equal(result.ok, true, 'temporary test graph cleanup');
}
