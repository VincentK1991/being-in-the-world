import { useEffect, useId, useState, type ReactNode } from 'react';
import ContextBar from './ContextBar';

const KEY = 'rdf-studio:sidebar';
const icons = {
  graph: <><circle cx="6" cy="7" r="2.4" /><circle cx="18" cy="6" r="2.4" /><circle cx="13" cy="18" r="2.4" /><path d="M8 8l3.5 8M16.2 7.4l-2.4 8.4M8.4 7h7.2" /></>,
  datasets: <><ellipse cx="12" cy="6" rx="7" ry="3" /><path d="M5 6v6c0 1.7 3.1 3 7 3s7-1.3 7-3V6M5 12v6c0 1.7 3.1 3 7 3s7-1.3 7-3v-6" /></>,
  plugins: <><path d="M9 3v4M15 3v4M7 7h10v5a5 5 0 0 1-10 0V7zM12 17v4" /></>,
  query: <><rect x="3" y="4" width="18" height="16" rx="3" /><path d="M7 10l3 2.5L7 15M12.5 15H17" /></>,
  docs: <><path d="M5 4.5A1.5 1.5 0 0 1 6.5 3H19v15H6.5A1.5 1.5 0 0 0 5 19.5v-15z" /><path d="M5 19.5A1.5 1.5 0 0 0 6.5 21H19v-3M9 7.5h6M9 11h6" /></>,
};
const Icon = ({name}: {name: keyof typeof icons}) => <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className="shrink-0">{icons[name]}</svg>;

interface Props { active: 'graph' | 'query' | 'datasets' | 'plugins'; connectionId: string; connection: string; children?: ReactNode }
// Left navigation shared by every page. Collapses to an icon rail; the choice is remembered per browser.
export default function AppSidebar({active, connectionId, connection, children}: Props) {
  const [collapsed, setCollapsed] = useState(false), panel = useId();
  useEffect(() => { try { setCollapsed(localStorage.getItem(KEY) === 'collapsed'); } catch { /* storage unavailable */ } }, []);
  function toggle() {
    setCollapsed(value => { try { localStorage.setItem(KEY, value ? 'open' : 'collapsed'); } catch { /* storage unavailable */ } return !value; });
  }
  const item = (name: keyof typeof icons, href: string, label: string, id?: string) =>
    <a id={id} href={href} title={collapsed ? label : undefined} aria-current={active === name ? 'page' : undefined} className="side-item">
      <Icon name={name} /><span className={collapsed ? 'sr-only' : 'truncate'}>{label}</span>
    </a>;
  return <>
  <ContextBar active={active} />
  <div className="side-slot" data-collapsed={collapsed}>
    <aside id={panel} aria-label="Primary" className="app-sidebar" data-collapsed={collapsed}>
      <div className="flex items-center gap-2 px-3 pt-4 pb-3">
        <a href="/" className="flex min-w-0 items-center gap-3 text-lg font-semibold tracking-tight" aria-label="RDF Studio home">
          <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-white/10 text-emerald-300">◈</span>
          {!collapsed && <span className="truncate">RDF Studio</span>}
        </a>
        <button type="button" className="side-toggle ml-auto" aria-expanded={!collapsed} aria-controls={panel} aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'} onClick={toggle}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={{transform: collapsed ? 'rotate(180deg)' : undefined}}><path d="M14.5 6l-6 6 6 6" /></svg>
        </button>
      </div>
      <nav aria-label="Workspace" className="space-y-1 px-2">
        {item('graph', '/', 'Graph explorer')}
        {item('datasets', '/datasets', 'Datasets')}
        {item('plugins', '/plugins', 'Plugins')}
        {item('query', '/workspace', 'SPARQL workspace', 'query-workspace-link')}
        {/* The documentation is a separate Starlight site in this app, so this is a plain link, never the current page. */}
        {item('docs', '/docs/', 'Documentation', 'docs-link')}
      </nav>
      {!collapsed && children && <div className="mt-6 min-h-0 flex-1 overflow-y-auto px-4 pb-4">{children}</div>}
      <div className="mt-auto px-4 py-3 text-xs text-emerald-100/70" title={collapsed ? connection : undefined}><span id={connectionId} className={collapsed ? 'sr-only' : undefined}>{connection}</span>{collapsed && <span aria-hidden="true" className={`block text-center ${connection.startsWith('●') ? 'text-emerald-300' : 'text-amber-300'}`}>●</span>}</div>
    </aside>
  </div>
  </>;
}
