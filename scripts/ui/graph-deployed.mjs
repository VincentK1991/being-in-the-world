import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdir } from 'node:fs/promises';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const base = 'http://127.0.0.1:4321', graph = `urn:viewer-test:${crypto.randomUUID()}`;
const data = `@prefix ex: <urn:animals:> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Mammal rdfs:label "Mammals" ; rdfs:comment "Warm-blooded animals" .
ex:Elephant rdfs:subClassOf ex:Mammal ; rdfs:label "Elephant" ; ex:habitat ex:Savanna .
ex:Whale rdfs:subClassOf ex:Mammal ; rdfs:label "Whale" .
ex:BlueWhale rdfs:subClassOf ex:Whale ; rdfs:label "Blue whale" .
ex:Savanna rdfs:label "Savanna" .`;
const upload = await fetch(`${base}/api/import?${new URLSearchParams({graph})}`, {method: 'POST',headers:{'Content-Type':'text/turtle'},body:data});
assert.equal(upload.ok, true);
const browser = await chromium.launch({headless:true});
try {
 const page = await browser.newPage({viewport:{width:1500,height:1050}}), errors=[];
 page.on('pageerror', e => errors.push(e.message));
 await page.goto(base);
 await page.locator('#graph-scope option', {hasText:graph}).waitFor({state:'attached'});
 await page.locator('#graph-scope').selectOption(graph);
 const nodeCount = () => page.evaluate(() => document.querySelectorAll('#d3-graph g.node').length);
 await page.waitForFunction(() => { const nodes = [...document.querySelectorAll('#d3-graph g.node title')]; return nodes.length === 2 && nodes.every(t => t.textContent.split('\n')[1].startsWith('urn:animals:')); });
 const cyNode = async name => page.evaluate(name => {
   const g = [...document.querySelectorAll('#d3-graph g.node')].find(g => g.querySelector('title').textContent.split('\n')[1] === `urn:animals:${name}`);
   if (!g) return null;
   const box = g.querySelector('circle.core').getBoundingClientRect();
   const hulls = [...document.querySelectorAll('#d3-graph path.hull')].map(h => h.getBoundingClientRect());
   return {x:box.x+box.width/2, y:box.y+box.height/2, box:{x1:box.x,y1:box.y,x2:box.right,y2:box.bottom}, hulls:hulls.map(h => ({x1:h.x,y1:h.y,x2:h.right,y2:h.bottom}))};
 }, name);
 const settled = async () => { await page.waitForFunction(() => document.querySelector('#layout-status')?.textContent.includes('Drag nodes')); await page.waitForTimeout(300); await page.waitForFunction(() => document.querySelector('#layout-status')?.textContent.includes('Drag nodes')); };
 const click = async name => { await page.keyboard.press('Escape'); await page.waitForTimeout(350); await settled(); await page.getByRole('button',{name:'Fit graph',exact:true}).click(); await page.waitForTimeout(100); const node = await cyNode(name); assert.ok(node); await page.mouse.click(node.x,node.y); await page.waitForTimeout(100); };
 await settled();
 assert.equal(await cyNode('Whale'),null);
 await click('Mammal');
 await page.locator('#node-inspector').getByText('Warm-blooded animals').waitFor();
 await settled();
 const child = await cyNode('Whale');
 assert.ok(child.hulls.some(h => child.box.x1 > h.x1 && child.box.x2 < h.x2 && child.box.y1 > h.y1 && child.box.y2 < h.y2), 'subclass sits inside the shaded group');
 await click('Whale');
 await settled();
 assert.ok(await cyNode('BlueWhale'));
 await click('Mammal');
 await settled();
 assert.equal(await cyNode('Whale'),null);
 await page.getByRole('textbox',{name:'Find a node'}).fill('Blue whale');
 await page.getByRole('button',{name:'Blue whale',exact:true}).click();
 await settled();
 assert.ok(await cyNode('BlueWhale'));
 await click('Elephant');
 await page.locator('#node-inspector').getByText('habitat',{exact:true}).waitFor();
 const before = await cyNode('Elephant');
 await page.mouse.move(before.x,before.y); await page.mouse.down();
 await page.mouse.move(before.x+65,before.y+40,{steps:12});
 const during = await cyNode('Elephant');
 assert.ok(Math.abs(during.x-before.x)>20,'D3 drag changes position');
 await page.mouse.up(); await settled();
 const zoomOf = () => page.evaluate(() => new DOMMatrix(getComputedStyle(document.querySelector('#d3-graph .viewport')).transform).a);
 const zoomBefore = await zoomOf();
 await page.getByRole('button',{name:'Zoom in',exact:true}).click();
 await page.waitForTimeout(400);
 assert.ok(await zoomOf()>zoomBefore);
 await page.getByRole('button',{name:'Fit graph',exact:true}).click();
 const edgeCount = await page.locator('#d3-graph path.edge').count();
 assert.ok(edgeCount>0);
 await page.getByRole('button',{name:'Re-layout',exact:true}).click(); await settled();
 await mkdir('target/local-deployment',{recursive:true});
 await page.screenshot({path:'target/local-deployment/graph-viewer.png',fullPage:true});
 // Datasets: list, create, switch (isolated, empty), plugins and content, switch back, delete.
 const tenant = `ui-test-${crypto.randomUUID().slice(0,8)}`;
 await page.goto(`${base}/datasets`);
 await page.locator('#dataset-rows').getByText('default',{exact:true}).waitFor();
 await page.locator('#dataset-plugins').getByText('dataset-summary').waitFor();
 await page.locator('#dataset-graphs').getByText(graph).waitFor();
 await page.getByRole('textbox',{name:'New dataset name'}).fill(tenant);
 await page.getByRole('button',{name:'Create dataset'}).click();
 const row = page.locator('#dataset-rows tr',{hasText:tenant});
 await row.waitFor();
 await row.getByRole('button',{name:'Use',exact:true}).click();
 await page.locator('#dataset-rows tr[aria-current="true"]',{hasText:tenant}).waitFor();
 assert.equal(await page.locator('#dataset-select').inputValue(), tenant);
 await page.goto(base);
 await page.waitForFunction(() => document.querySelector('#graph-node-count')?.textContent.startsWith('0 resources'));
 await page.locator('#dataset-select').selectOption('default');
 await page.waitForFunction(() => document.querySelector('#graph-node-count') && !document.querySelector('#graph-node-count').textContent.startsWith('0 resources'));
 await page.goto(`${base}/datasets`);
 page.once('dialog', dialog => dialog.accept());
 await page.locator('#dataset-rows tr',{hasText:tenant}).getByRole('button',{name:`Delete dataset ${tenant}`}).click();
 await page.waitForFunction(name => ![...document.querySelectorAll('#dataset-rows tr')].some(r => r.textContent.includes(name)), tenant);
 await page.evaluate(() => localStorage.removeItem('rdf-studio:dataset'));
 await page.goto(base);
 await page.setViewportSize({width:390,height:844});
 assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
 await page.locator('#query-workspace-link').click();
 await page.locator('#query').waitFor();
 assert.deepEqual(errors,[]);
 console.log('PASS: dataset list/create/switch/delete, D3 canvas, group hulls, properties, collapse/search, real pointer drag, zoom, force re-layout, edges, mobile and SPARQL navigation.');
} finally {
 await browser.close();
 const result = await fetch(`${base}/update`,{method:'POST',headers:{'Content-Type':'application/sparql-update'},body:`DROP GRAPH <${graph}>`});
 assert.equal(result.ok,true);
}
