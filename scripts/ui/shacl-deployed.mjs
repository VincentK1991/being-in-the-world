// Exercise real file inputs, Jena validation, findings, and explicit gate behavior.
// Deliberately leaves the bundled example graphs/report visible for the user.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdir } from 'node:fs/promises';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const base = process.env.RDF_UI_BASE || 'http://127.0.0.1:4321';
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${base}/plugins`);
  const panel = page.getByRole('region', { name: 'SHACL validation' });
  await panel.getByRole('combobox', { name: 'Validation job' }).selectOption('claims-example');
  const validate = panel.getByRole('button', { name: 'Validate · report only' });
  const waitEnabled = () => page.waitForFunction(() => {
    const button = [...document.querySelectorAll('button')].find(b => b.textContent === 'Validate · report only');
    return button && !button.disabled;
  });
  const text = expected => panel.getByRole('status').filter({ hasText: expected }).waitFor();
  async function upload(role, fixture) {
    await waitEnabled();
    await panel.getByLabel(`Replace ${role} graph urn:shacl:example:${role.toLowerCase()}`).setInputFiles(`plugins/shacl/examples/${fixture}.ttl`);
    await text(`Replaced urn:shacl:example:${role.toLowerCase()}`);
    await waitEnabled();
  }
  await upload('Shapes', 'shapes');
  await upload('Data', 'invalid');
  await validate.click();
  await text('Report saved');
  await waitEnabled();
  const status = panel.getByLabel('Validation report status');
  assert.match(await status.innerText(), /2 violations · 1 warnings · 1 info/);
  assert.equal(await panel.getByRole('table', { name: 'SHACL findings' }).locator('tbody tr').count(), 4);
  await panel.getByRole('button', { name: 'Run validation gate' }).click();
  await panel.getByRole('alert').filter({ hasText: 'SHACL gate failed' }).waitFor();
  await waitEnabled();
  const unchanged = await page.request.post(`${base}/query`, {
    headers: { 'Content-Type': 'application/sparql-query', Accept: 'application/sparql-results+json' },
    data: 'ASK { GRAPH <urn:shacl:example:data> { <urn:shacl:example:claim1> <urn:shacl:example:reserve> -10 } }',
  });
  assert.equal((await unchanged.json()).boolean, true);
  await upload('Data', 'valid');
  assert.match(await status.innerText(), /stale/);
  await panel.getByRole('button', { name: 'Run validation gate' }).click();
  await text('Validation gate passed');
  await waitEnabled();
  assert.match(await status.innerText(), /SHACL conforms: true · Gate passes: true/);
  assert.equal(await panel.getByRole('table').count(), 0);
  // Leave the illustrative report available in Studio and through standard SPARQL.
  await upload('Data', 'invalid');
  await validate.click();
  await text('Report saved');
  await waitEnabled();
  await mkdir('target/local-deployment', { recursive: true });
  await page.screenshot({ path: 'target/local-deployment/shacl-validation.png', fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), true);
  assert.deepEqual(errors, []);
  console.log('PASS deployed SHACL Studio: shapes/data file uploads, four findings, failed gate preserves data, stale status, valid gate, mobile layout; example report left visible.');
} finally {
  await browser.close();
}
