import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import {
  curveCatmullRomClosed, drag, easeCubicOut, forceCollide, forceLink, forceManyBody, forceSimulation, forceX, forceY,
  line, polygonHull, select, zoom, zoomIdentity,
  type D3DragEvent, type D3ZoomEvent, type Selection, type Simulation, type SimulationLinkDatum, type SimulationNodeDatum, type ZoomBehavior,
} from 'd3';
import { typeColor, type ViewEdge, type ViewNode } from '../lib/graph';

export interface GraphControls { zoomBy(factor: number): void; fit(): void; relayout(): void; focus(id: string): void }
interface Props { nodes: ViewNode[]; edges: ViewEdge[]; selected: string; inset?: number; fade?: number; onPick(id: string): void; onZoom(value: number): void }

interface SimNode extends ViewNode, SimulationNodeDatum { r: number; degree: number; depth: number }
interface SimLink extends SimulationLinkDatum<SimNode> { id: string; label: string; aggregated: boolean; curve: number; flip: boolean; contain: boolean; edge?: ViewEdge }
interface Group { id: string; members: SimNode[]; pad: number; depth: number }

const MIN_ZOOM = .15, MAX_ZOOM = 3, FIT_PAD = 70;
const LABEL_FADE = [7, 14] as const; // on-screen node radius (px) where a title starts / finishes appearing
const reduced = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;
const clip = (text: string, max = 24) => text.length > max ? `${text.slice(0, max - 1)}…` : text;
const radius = (degree: number, children: number) => Math.min(28, 6 + Math.sqrt(degree) * 3.2 + Math.sqrt(children) * 2.6 + (children ? 3 : 0));

const D3Graph = forwardRef<GraphControls, Props>(function D3Graph({nodes, edges, selected, inset = 0, fade = 1, onPick, onZoom}, ref) {
  const host = useRef<HTMLDivElement>(null), svgEl = useRef<SVGSVGElement>(null);
  const handlers = useRef({onPick, onZoom}); handlers.current = {onPick, onZoom};
  const insetRef = useRef(0); insetRef.current = inset;
  const fadeRef = useRef(1); fadeRef.current = fade;
  const [running, setRunning] = useState(false);
  // Everything D3 owns lives in one mutable bag so React renders never touch it.
  const d3s = useRef({
    sim: null as Simulation<SimNode, SimLink> | null,
    nodes: [] as SimNode[], links: [] as SimLink[], groups: [] as Group[],
    svg: null as Selection<SVGSVGElement, unknown, null, undefined> | null,
    behavior: null as ZoomBehavior<SVGSVGElement, unknown> | null,
    layers: null as null | Record<'viewport' | 'hulls' | 'edges' | 'edgeLabels' | 'nodes', Selection<SVGGElement, unknown, null, undefined>>,
    size: {w: 800, h: 520}, k: 1, autoFit: true, ticks: 0, hover: '', selected: '',
    draw: () => {}, focus: () => {}, labels: () => {}, fit: (_animate = true) => {},
  });

  useImperativeHandle(ref, () => ({
    zoomBy(factor) { const {svg, behavior} = d3s.current; if (svg && behavior) { d3s.current.autoFit = false; svg.transition().duration(reduced() ? 0 : 240).call(behavior.scaleBy, factor); } },
    fit() { d3s.current.autoFit = true; d3s.current.fit(); },
    relayout() {
      const s = d3s.current; if (!s.sim) return;
      s.nodes.forEach((n, i) => { const a = i * 2.399963, d = 40 + 22 * Math.sqrt(i); n.x = Math.cos(a) * d; n.y = Math.sin(a) * d; n.vx = n.vy = 0; n.fx = n.fy = null; });
      s.autoFit = true; s.ticks = 0; settle();
    },
    focus(id) {
      const {svg, behavior, nodes: list, size, k} = d3s.current, n = list.find(item => item.id === id);
      if (!svg || !behavior || !n) return;
      svg.transition().duration(reduced() ? 0 : 500).ease(easeCubicOut)
        .call(behavior.transform, zoomIdentity.translate((size.w - Math.min(insetRef.current, size.w * .5)) / 2, size.h / 2).scale(Math.max(k, .9)).translate(-(n.x ?? 0), -(n.y ?? 0)));
    },
  }));

  function settle() {
    const s = d3s.current; if (!s.sim) return;
    if (reduced()) {
      s.sim.alpha(1).stop(); for (let i = 0; i < 320; i++) s.sim.tick();
      s.draw(); s.fit(false); setRunning(false); return;
    }
    setRunning(true); s.sim.alpha(Math.max(s.sim.alpha(), .9)).restart();
  }

  // One-time scaffolding: layers, zoom, simulation, resize observer.
  useEffect(() => {
    const s = d3s.current, svg = select(svgEl.current!), root = svg;
    s.svg = svg;
    svg.selectAll('*').remove();
    const defs = svg.append('defs');
    for (const [id, cls] of [['rdf-arrow', 'arrow'], ['rdf-arrow-hot', 'arrow hot']] as const)
      defs.append('marker').attr('id', id).attr('viewBox', '0 0 10 10').attr('refX', 9).attr('refY', 5).attr('markerWidth', 8).attr('markerHeight', 8)
        .attr('markerUnits', 'userSpaceOnUse').attr('orient', 'auto').append('path').attr('d', 'M0,1 L10,5 L0,9 L2.6,5 Z').attr('class', cls);
    const glow = defs.append('filter').attr('id', 'rdf-glow').attr('x', '-60%').attr('y', '-60%').attr('width', '220%').attr('height', '220%');
    glow.append('feGaussianBlur').attr('stdDeviation', 5);
    const viewport = root.append('g').attr('class', 'viewport');
    s.layers = {
      viewport,
      hulls: viewport.append('g').attr('class', 'hulls'),
      edges: viewport.append('g').attr('class', 'edges'),
      edgeLabels: viewport.append('g').attr('class', 'edge-labels'),
      nodes: viewport.append('g').attr('class', 'nodes'),
    };

    s.behavior = zoom<SVGSVGElement, unknown>().scaleExtent([MIN_ZOOM, MAX_ZOOM]).clickDistance(4)
      .on('start', (e: D3ZoomEvent<SVGSVGElement, unknown>) => { if (e.sourceEvent) { s.autoFit = false; svg.classed('panning', e.sourceEvent.type === 'mousedown'); } })
      .on('zoom', (e: D3ZoomEvent<SVGSVGElement, unknown>) => {
        viewport.attr('transform', e.transform.toString());
        if (e.transform.k !== s.k) { s.k = e.transform.k; svg.classed('zoomed-in', s.k >= 1.7); s.labels(); svg.style('--lk', String(1 / Math.sqrt(s.k))); handlers.current.onZoom(s.k); }
      })
      .on('end', () => svg.classed('panning', false));
    svg.call(s.behavior).on('dblclick.zoom', null);

    const link = forceLink<SimNode, SimLink>([]).id(n => n.id)
      .distance(l => l.contain ? 46 + (l.target as SimNode).r + (l.source as SimNode).r : 120 + (l.curve ? 24 : 0))
      .strength(l => l.contain ? .55 : .22);
    s.sim = forceSimulation<SimNode, SimLink>([])
      .force('link', link)
      .force('charge', forceManyBody<SimNode>().strength(n => -(150 + n.r * 14)).distanceMax(520).theta(.9))
      .force('collide', forceCollide<SimNode>().radius(n => n.r + 16).strength(.85).iterations(2))
      .force('x', forceX<SimNode>(0).strength(.045)).force('y', forceY<SimNode>(0).strength(.06))
      .velocityDecay(.34).alphaDecay(.017).alphaMin(.004).stop();
    s.sim.on('tick', () => {
      s.draw(); s.ticks++;
      if (s.autoFit && (s.ticks === 24 || s.ticks === 70 || s.ticks === 140)) s.fit();
    }).on('end', () => { setRunning(false); if (s.autoFit) s.fit(); });

    const place = () => {
      const box = host.current!.getBoundingClientRect();
      s.size = {w: Math.max(200, box.width), h: Math.max(200, box.height)};
      if (s.autoFit) s.fit(false);
    };
    place();
    svg.call(s.behavior.transform, zoomIdentity.translate(s.size.w / 2, s.size.h / 2));
    const observer = new ResizeObserver(place); observer.observe(host.current!);

    // Curves, hulls and positions: everything that depends on x/y lives here.
    const hullPath = line<[number, number]>().curve(curveCatmullRomClosed.alpha(.6));
    s.draw = () => {
      const L = s.layers!;
      L.nodes.selectAll<SVGGElement, SimNode>('g.node').attr('transform', n => `translate(${n.x},${n.y})`);
      L.edges.selectAll<SVGPathElement, SimLink>('path.edge').attr('d', l => geometry(l).d);
      L.edgeLabels.selectAll<SVGTextElement, SimLink>('text.edge-label').each(function (l) { const g = geometry(l); select(this).attr('x', g.lx).attr('y', g.ly); });
      L.hulls.selectAll<SVGPathElement, Group>('path.hull').attr('d', g => {
        const pts: [number, number][] = [];
        for (const m of g.members) for (let i = 0; i < 12; i++) {
          const a = (i / 12) * Math.PI * 2, rr = m.r + g.pad;
          pts.push([(m.x ?? 0) + Math.cos(a) * rr, (m.y ?? 0) + Math.sin(a) * rr]);
        }
        const hull = polygonHull(pts);
        return hull ? hullPath(hull) : null;
      });
    };
    function geometry(l: SimLink) {
      const a = l.source as SimNode, b = l.target as SimNode, ax = a.x ?? 0, ay = a.y ?? 0, bx = b.x ?? 0, by = b.y ?? 0;
      if (a === b) {
        const r = a.r;
        return {d: `M${ax + r * .75},${ay - r * .75} C${ax + r * 3.4},${ay - r * 3.6} ${ax - r * 3.4},${ay - r * 3.6} ${ax - r * .75},${ay - r * .75}`, lx: ax, ly: ay - r * 3.1};
      }
      const dx = bx - ax, dy = by - ay, dist = Math.hypot(dx, dy) || 1;
      let nx = -dy / dist, ny = dx / dist; if (l.flip) { nx = -nx; ny = -ny; }
      const cx = (ax + bx) / 2 + nx * l.curve * dist, cy = (ay + by) / 2 + ny * l.curve * dist;
      const sd = Math.hypot(cx - ax, cy - ay) || 1, td = Math.hypot(cx - bx, cy - by) || 1;
      const sx = ax + (cx - ax) / sd * (a.r + 2), sy = ay + (cy - ay) / sd * (a.r + 2);
      const ex = bx + (cx - bx) / td * (b.r + 7), ey = by + (cy - by) / td * (b.r + 7);
      return {d: `M${sx},${sy} Q${cx},${cy} ${ex},${ey}`, lx: .25 * sx + .5 * cx + .25 * ex, ly: .25 * sy + .5 * cy + .25 * ey - 4};
    }

    // Titles and edges fade in as the on-screen radius of the node (or, for an edge, its larger
    // end) grows: hubs name themselves while zoomed out, small nodes only once you zoom in close.
    // `fade` scales the thresholds: 0 shows everything, 1 is the default, 2 is aggressive.
    const smooth = (value: number, lo: number, hi: number) => {
      const t = Math.min(1, Math.max(0, (value - lo) / (hi - lo)));
      return t * t * (3 - 2 * t);
    };
    s.labels = () => {
      const strength = fadeRef.current, show = (screenRadius: number, scale = 1) =>
        strength <= 0 ? 1 : smooth(screenRadius, LABEL_FADE[0] * strength * scale, LABEL_FADE[1] * strength * scale);
      const L = s.layers!, size = (l: SimLink) => Math.max((l.source as SimNode).r, (l.target as SimNode).r) * s.k;
      L.nodes.selectAll<SVGGElement, SimNode>('g.node').style('--lf', n => String(show(n.r * s.k)));
      L.edges.selectAll<SVGPathElement, SimLink>('path.edge').style('--ef', l => String(strength <= 0 ? 1 : .12 + .88 * show(size(l), .8)));
      L.edgeLabels.selectAll<SVGTextElement, SimLink>('text.edge-label').style('--ef', l => String(show(size(l), 2.4)));
    };

    s.fit = (animate = true) => {
      if (!s.behavior || !s.nodes.length) return;
      let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
      for (const n of s.nodes) { x0 = Math.min(x0, (n.x ?? 0) - n.r); x1 = Math.max(x1, (n.x ?? 0) + n.r); y0 = Math.min(y0, (n.y ?? 0) - n.r); y1 = Math.max(y1, (n.y ?? 0) + n.r + 18); }
      const w = Math.max(x1 - x0, 1), h = Math.max(y1 - y0, 1);
      const k = Math.max(MIN_ZOOM, Math.min(1.2, (s.size.w - FIT_PAD * 2) / w, (s.size.h - FIT_PAD * 2) / h));
      const t = zoomIdentity.translate(s.size.w / 2, s.size.h / 2).scale(k).translate(-(x0 + x1) / 2, -(y0 + y1) / 2);
      const target = animate && !reduced() ? svg.transition().duration(850).ease(easeCubicOut) : svg;
      (target as typeof svg).call(s.behavior.transform, t);
    };

    // Emphasis follows hover first, then selection; everything else recedes.
    s.focus = () => {
      const id = s.hover || s.selected;
      const near = new Set<string>();
      if (id) { near.add(id); for (const l of s.links) if (!l.contain) { const a = (l.source as SimNode).id, b = (l.target as SimNode).id; if (a === id) near.add(b); if (b === id) near.add(a); } }
      const L = s.layers!;
      svg.classed('has-focus', !!id);
      L.nodes.selectAll<SVGGElement, SimNode>('g.node').classed('near', n => near.has(n.id)).classed('selected', n => n.id === s.selected).classed('hovered', n => n.id === s.hover);
      const hot = (l: SimLink) => !l.contain && !!id && ((l.source as SimNode).id === id || (l.target as SimNode).id === id);
      L.edges.selectAll<SVGPathElement, SimLink>('path.edge').classed('hot', hot);
      L.edgeLabels.selectAll<SVGTextElement, SimLink>('text.edge-label').classed('hot', hot);
    };

    return () => { observer.disconnect(); s.sim?.stop(); s.sim = null; svg.on('.zoom', null); svg.selectAll('*').remove(); };
  }, []);

  // Data sync: preserve positions of surviving nodes, spawn new ones at a neighbour.
  useEffect(() => {
    const s = d3s.current; if (!s.sim || !s.layers) return;
    const previous = new Map(s.nodes.map(n => [n.id, n]));
    const degree = new Map<string, number>();
    for (const e of edges) { degree.set(e.source, (degree.get(e.source) ?? 0) + 1); degree.set(e.target, (degree.get(e.target) ?? 0) + 1); }
    const byId = new Map<string, SimNode>();
    const depthOf = (n: ViewNode): number => { let d = 0, p = n.parent; while (p && d < 12) { d++; p = nodes.find(x => x.id === p)?.parent; } return d; };
    const fresh: SimNode[] = nodes.map((n, i) => {
      const old = previous.get(n.id), deg = degree.get(n.id) ?? 0;
      const sim: SimNode = Object.assign(old ?? {} as SimNode, n, {degree: deg, r: radius(deg, n.children), depth: depthOf(n)});
      if (!old) { const a = i * 2.399963, d = 30 + 20 * Math.sqrt(i); sim.x = Math.cos(a) * d; sim.y = Math.sin(a) * d; }
      byId.set(n.id, sim); return sim;
    });
    // New nodes burst out of their parent or first neighbour instead of the origin.
    for (const n of fresh) if (!previous.has(n.id)) {
      const anchor = (n.parent && previous.get(n.parent)) || previous.get(edges.find(e => e.source === n.id && previous.has(e.target))?.target ?? '')
        || previous.get(edges.find(e => e.target === n.id && previous.has(e.source))?.source ?? '');
      if (anchor) { n.x = (anchor.x ?? 0) + (Math.random() - .5) * 30; n.y = (anchor.y ?? 0) + (Math.random() - .5) * 30; }
    }
    // Parallel edges between one pair fan out as arcs.
    const pairs = new Map<string, ViewEdge[]>();
    for (const e of edges) { const key = e.source < e.target ? `${e.source}\u0000${e.target}` : `${e.target}\u0000${e.source}`; (pairs.get(key) ?? pairs.set(key, []).get(key)!).push(e); }
    const links: SimLink[] = [];
    for (const group of pairs.values()) group.forEach((e, i) => links.push({
      id: e.id, source: e.source, target: e.target, label: e.label, aggregated: e.aggregated, edge: e, contain: false,
      curve: (i - (group.length - 1) / 2) * .3, flip: e.source > e.target,
    }));
    const groups = new Map<string, Group>();
    for (const n of fresh) {
      links.push(...(n.parent && byId.has(n.parent) ? [{id: `in-${n.id}`, source: n.parent, target: n.id, label: '', aggregated: false, curve: 0, flip: false, contain: true} as SimLink] : []));
      for (let p = n.parent; p && byId.has(p); p = byId.get(p)!.parent) {
        const g = groups.get(p) ?? groups.set(p, {id: p, members: [byId.get(p)!], pad: 0, depth: byId.get(p)!.depth}).get(p)!;
        g.members.push(n); g.pad = Math.max(g.pad, 14 + 11 * (n.depth - g.depth - 1));
      }
    }
    s.nodes = fresh; s.links = links; s.groups = [...groups.values()].sort((a, b) => a.depth - b.depth);
    const firstLoad = previous.size === 0;
    const grew = fresh.some(n => !previous.has(n.id)) || links.length !== s.sim.force<ReturnType<typeof forceLink<SimNode, SimLink>>>('link')!.links().length;

    s.sim.nodes(fresh);
    s.sim.force<ReturnType<typeof forceLink<SimNode, SimLink>>>('link')!.links(links);

    // ---- DOM join ----
    const L = s.layers;
    L.hulls.selectAll<SVGPathElement, Group>('path.hull').data(s.groups, g => g.id).join(
      enter => enter.append('path').attr('class', 'hull').style('opacity', 0).call(e => e.transition().duration(600).style('opacity', 1)),
      update => update, exit => exit.remove());
    const drawn = links.filter(l => !l.contain);
    L.edges.selectAll<SVGPathElement, SimLink>('path.edge').data(drawn, l => l.id).join(
      enter => enter.append('path').attr('class', 'edge').style('opacity', 0).call(e => e.transition().duration(500).style('opacity', null)),
      update => update, exit => exit.remove())
      .classed('aggregated', l => l.aggregated);
    L.edgeLabels.selectAll<SVGTextElement, SimLink>('text.edge-label').data(drawn, l => l.id).join('text')
      .attr('class', 'edge-label').attr('text-anchor', 'middle').text(l => clip(l.label, 22));
    s.labels();

    const nodeSel = L.nodes.selectAll<SVGGElement, SimNode>('g.node').data(fresh, n => n.id).join(
      enter => {
        const g = enter.append('g').attr('class', 'node');
        g.append('circle').attr('class', 'halo').attr('r', 0);
        g.append('circle').attr('class', 'core').attr('r', 0);
        g.append('path').attr('class', 'mark');
        g.append('text').attr('class', 'label').attr('text-anchor', 'middle');
        g.append('title');
        return g;
      },
      // A node re-matched while it is still fading out must keep living: cancel the pending removal.
      update => update.interrupt().style('opacity', null),
      exit => exit.transition().duration(220).style('opacity', 0).remove());
    nodeSel.style('--c', n => typeColor(n.type)).attr('data-kind', n => n.bnode ? 'blank' : n.children ? 'class' : 'resource').classed('expandable', n => n.children > 0);
    const dur = reduced() ? 0 : 520;
    nodeSel.select<SVGCircleElement>('circle.core').transition().duration(dur).ease(easeCubicOut).attr('r', n => n.r);
    nodeSel.select<SVGCircleElement>('circle.halo').transition().duration(dur).ease(easeCubicOut).attr('r', n => n.r + 7);
    nodeSel.select<SVGPathElement>('path.mark').attr('d', n => {
      if (!n.children) return null;
      const m = Math.max(3, n.r * .38);
      return n.expanded ? `M${-m},0 H${m}` : `M${-m},0 H${m} M0,${-m} V${m}`;
    });
    nodeSel.select<SVGTextElement>('text.label').attr('y', n => n.r + 15).text(n => clip(n.label));
    nodeSel.select('title').text(n => `${n.label}\n${n.iri}`);
    s.labels();

    nodeSel.on('pointerenter', (_e, n) => { s.hover = n.id; s.focus(); })
      .on('pointerleave', () => { s.hover = ''; s.focus(); })
      .on('click', (e, n) => { if (!e.defaultPrevented) handlers.current.onPick(n.id); })
      .call(drag<SVGGElement, SimNode>().container(L.viewport.node()!).clickDistance(4)
        .on('start', (e: D3DragEvent<SVGGElement, SimNode, SimNode>, n) => { if (!e.active) { s.sim!.alphaTarget(.22).restart(); setRunning(true); } n.fx = n.x; n.fy = n.y; })
        .on('drag', (e, n) => { n.fx = e.x; n.fy = e.y; })
        .on('end', (e, n) => { if (!e.active) s.sim!.alphaTarget(0); n.fx = n.fy = null; }));

    s.selected = selected; s.focus();
    if (firstLoad) { s.autoFit = true; s.ticks = 0; }
    s.draw();
    if (!fresh.length) { setRunning(false); return; }
    if (firstLoad || grew) settle();
  }, [nodes, edges]);

  useEffect(() => { d3s.current.selected = selected; d3s.current.focus(); }, [selected]);
  useEffect(() => { d3s.current.labels(); }, [fade]);

  return <div ref={host} className="d3-graph relative h-full w-full">
    <svg ref={svgEl} id="d3-graph" role="img" className="h-full w-full touch-none select-none"
      aria-label="Interactive RDF graph. Drag nodes to rearrange, drag the background to pan, scroll to zoom. Use the node list below for keyboard access." />
    <span role="status" id="layout-status" className="graph-chip pointer-events-none absolute bottom-3 left-3">
      <i className={running ? 'pulse' : ''} aria-hidden="true" />{running ? 'Settling graph…' : 'Drag nodes · Pan · Scroll to zoom'}
    </span>
  </div>;
});
export default D3Graph;
