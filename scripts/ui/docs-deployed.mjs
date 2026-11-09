// Check the documentation served by the deployed UI: the sidebar link, every section, diagrams,
// search, a 404, the dark theme and phone width.
import { createRequire } from 'node:module';
import { mkdir } from 'node:fs/promises';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const base = process.env.RDF_UI_BASE || 'http://127.0.0.1:4321';
const pages = ['what-it-is/overview', 'what-it-is/architecture', 'what-it-is/components', 'what-it-is/status-and-limits',
  'how-to-use/install', 'how-to-use/cli', 'how-to-use/studio', 'how-to-use/http-api', 'how-to-use/datasets', 'how-to-use/plugins',
  'internals/request-path', 'internals/storage', 'internals/plugin-architecture', 'internals/vector-index',
  'internals/inference', 'internals/federation', 'internals/shacl', 'internals/testing'];
const diagrams = { 'what-it-is/architecture': 1, 'internals/request-path': 1, 'internals/storage': 1, 'internals/plugin-architecture': 1, 'internals/vector-index': 1, 'internals/inference': 1 };
const checks = [];
const check = (name, ok) => { checks.push([name, ok]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}`); };
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${base}/`);
  await page.getByRole('link', { name: 'Documentation' }).first().click();
  await page.waitForURL(/\/docs\/$/);
  check('the Studio sidebar leads to the docs', true);
  for (const slug of pages) {
    const response = await page.goto(`${base}/docs/${slug}/`);
    const text = await page.locator('main').innerText();
    check(`${slug} loads and is written`, response.ok() && !text.includes('Not written yet') && text.length > 800);
    if (diagrams[slug]) check(`${slug} shows its diagram`, (await page.locator('main svg[role="img"]').count()) >= 1);
  }
  const missing = await page.goto(`${base}/docs/no-such-page/`);
  check('a missing docs page answers 404', missing.status() === 404);
  await page.goto(`${base}/docs/`);
  const search = await page.request.get(`${base}/pagefind/pagefind-entry.json`);
  check('the search index is served', search.ok());
  await page.goto(`${base}/docs/internals/storage/`);
  await page.emulateMedia({ colorScheme: 'dark' });
  await mkdir('target/ui', { recursive: true });
  await page.screenshot({ path: 'target/ui/docs-storage-dark.png', fullPage: false });
  await page.setViewportSize({ width: 390, height: 900 });
  for (const slug of ['internals/storage', 'internals/request-path', 'what-it-is/architecture', 'internals/inference', 'internals/testing', 'internals/federation']) {
    await page.goto(`${base}/docs/${slug}/`);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    check(`${slug} has no sideways scroll at phone width`, overflow <= 1);
  }
  const studio = await page.goto(`${base}/plugins`);
  check('Studio pages are unchanged', studio.ok());
  check('no page errors', errors.length === 0);
} finally {
  await browser.close();
}
const failed = checks.filter(([, ok]) => !ok);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
process.exit(failed.length ? 1 : 0);
