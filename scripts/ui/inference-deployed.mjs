// Drive the Inference panel against the deployed stack: job states, rebuild, rule tests, "Why?"
// explanations, a real clash and its recovery, and a cascade. Leaves every job ready.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdir } from 'node:fs/promises';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const base = process.env.RDF_UI_BASE || 'http://127.0.0.1:4321';
const clash = 'INSERT DATA { GRAPH <urn:rdftool:examples:acme-insurance> { <urn:e2e:Open> <http://www.w3.org/2002/07/owl#disjointWith> <urn:e2e:Closed> . <urn:e2e:claim> a <urn:e2e:Open> , <urn:e2e:Closed> } }';
const update = async (page, text) => {
  const response = await page.request.post(`${base}/update`, { data: text, headers: { 'Content-Type': 'application/sparql-update' } });
  assert.ok(response.ok(), `update failed: ${response.status()}`);
};
const browser = await chromium.launch({ headless: true });
const checks = [];
const check = (name, ok) => { checks.push([name, ok]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}`); };
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${base}/plugins`);
  const panel = page.getByRole('region', { name: 'Inference jobs' });
  const table = panel.getByRole('table', { name: 'Inference job states' });
  await table.locator('tbody tr').first().waitFor();
  await page.waitForFunction(() => !document.querySelector('[aria-label="Inference job states"] tbody')?.textContent?.includes('…'));
  const rows = await table.locator('tbody tr').count();
  check('the panel lists every job with a state', rows === 10);

  const idle = () => page.waitForFunction(() => !document.querySelector('[aria-label="Inference jobs"]')?.textContent?.includes('Working…'));
  const status = text => panel.getByRole('status').filter({ hasText: text }).first().waitFor();
  const pick = async name => { await table.getByRole('button', { name, exact: true }).click(); await panel.getByRole('region', { name: `Job ${name}` }).waitFor().catch(() => {}); };
  const job = name => panel.getByLabel(`Job ${name}`);
  const row = name => table.locator('tbody tr').filter({ has: page.getByRole('button', { name, exact: true }) });

  await pick('acme');
  await panel.getByRole('button', { name: 'Rebuild', exact: true }).click();
  await status('Rebuilt acme:'); await idle();
  check('rebuilding from the panel makes the job ready', (await job('acme').innerText()).includes('ready'));

  await panel.getByRole('button', { name: /Run 4 rule tests/ }).click();
  await status('All 4 rule tests passed'); await idle();
  check('rule tests run from the panel, including the expected clash', (await panel.getByLabel('Rule test results').innerText()).includes('found inconsistent, as expected'));

  const deduced = panel.getByRole('list', { name: 'Deduced triples' });
  await deduced.locator('li').first().waitFor();
  await deduced.getByRole('button', { name: /^Why / }).first().click(); await idle();
  const explanation = panel.getByLabel('Explanation');
  await explanation.waitFor();
  const text = await explanation.innerText();
  check('“Why?” shows the OWL 2 RL rule and where the facts were asserted', /rule [a-z]+-[a-z0-9-]+/.test(text) && text.includes('Asserted in'));
  await mkdir('target/ui', { recursive: true });
  await page.screenshot({ path: 'target/ui/inference-explain.png', fullPage: true });

  await update(page, clash);
  try {
    await panel.getByRole('button', { name: 'Rebuild', exact: true }).click();
    await status('found the data inconsistent'); await idle();
    const clashes = panel.getByRole('list', { name: 'Clashes' }).first();
    const clashText = await clashes.innerText();
    check('a clash is shown with its message and the resource involved', clashText.includes('disjoint') && clashText.includes('claim'));
    check('the job badge reads inconsistent', (await row('acme').innerText()).includes('inconsistent'));
    await page.screenshot({ path: 'target/ui/inference-clash.png', fullPage: true });
  } finally {
    await update(page, clash.replace('INSERT DATA', 'DELETE DATA'));
  }
  await panel.getByRole('button', { name: 'Rebuild', exact: true }).click();
  await status('Rebuilt acme:'); await idle();
  check('after the clash is removed, a rebuild makes it ready again', (await row('acme').innerText()).includes('ready'));

  await pick('reserve-summary');
  await panel.getByRole('combobox', { name: 'Cascade' }).selectOption('upstream');
  await panel.getByRole('button', { name: 'Rebuild', exact: true }).click();
  await status('Rebuilt reserve-summary'); await idle();
  const steps = await panel.getByLabel('Cascade steps').innerText();
  check('a cascade lists the upstream jobs it rebuilt or skipped', steps.includes('acme') && steps.includes('reserved-claims'));

  // Narrow screens: the panel must not scroll the page sideways.
  await page.setViewportSize({ width: 390, height: 900 });
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  check('no horizontal page scroll at phone width', overflow <= 1);
  await page.screenshot({ path: 'target/ui/inference-mobile.png', fullPage: true });
  check('no page errors', errors.length === 0);
  if (errors.length) console.log(errors);
} finally {
  await browser.close();
}
const failed = checks.filter(([, ok]) => !ok);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
process.exit(failed.length ? 1 : 0);
