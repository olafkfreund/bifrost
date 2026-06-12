type Page =
  | 'portfolio'
  | 'assessment'
  | 'forecast'
  | 'completeness'
  | 'program'
  | 'board'
  | 'review'
  | 'connections'
  | 'routing'
  | 'readiness'
  | 'docs'

/** Left-rail navigation. Primary work (Workspace) and configuration (Settings)
 * are grouped separately so the change-management surfaces — connections,
 * routing — read as administration, not day-to-day review. The pinned importer
 * version at the foot is attestation metadata (pinned tool version per audit). */
const ICONS: Record<Page, React.ReactNode> = {
  portfolio: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  ),
  assessment: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <path d="M14 2v6h6M8 13h8M8 17h6" />
    </svg>
  ),
  forecast: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 3v18h18" />
      <path d="M7 14l4-4 3 3 5-6" />
    </svg>
  ),
  completeness: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 11l3 3 8-8" />
      <path d="M20 12v6a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h9" />
    </svg>
  ),
  program: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2 2 7l10 5 10-5-10-5z" />
      <path d="M2 17l10 5 10-5M2 12l10 5 10-5" />
    </svg>
  ),
  board: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M9 3v18M15 3v18" />
    </svg>
  ),
  review: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 11l3 3L22 4" />
      <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11" />
    </svg>
  ),
  connections: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 2v6M15 2v6M7 8h10v3a5 5 0 0 1-10 0z" />
      <path d="M12 16v6" />
    </svg>
  ),
  routing: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="6" cy="19" r="2" />
      <circle cx="18" cy="5" r="2" />
      <path d="M8 19h6a4 4 0 0 0 4-4V7M6 17V9a4 4 0 0 1 4-4h6" />
    </svg>
  ),
  readiness: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      <path d="M9 12l2 2 4-4" />
    </svg>
  ),
  docs: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
      <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" />
    </svg>
  ),
}

const GROUPS: { label: string; items: { id: Page; label: string }[] }[] = [
  { label: 'Workspace', items: [
    { id: 'portfolio', label: 'Portfolio' },
    { id: 'assessment', label: 'Assessment' },
    { id: 'forecast', label: 'Forecast' },
    { id: 'completeness', label: 'Coverage' },
    { id: 'program', label: 'Program' },
    { id: 'board', label: 'Board' },
    { id: 'review', label: 'Review' },
  ] },
  { label: 'Settings', items: [
    { id: 'connections', label: 'Connections' },
    { id: 'routing', label: 'Routing' },
    { id: 'readiness', label: 'Readiness' },
  ] },
]

function NavItem({
  id,
  label,
  active,
  onNavigate,
}: {
  id: Page
  label: string
  active: boolean
  onNavigate: (p: Page) => void
}) {
  return (
    <button
      onClick={() => onNavigate(id)}
      aria-current={active ? 'page' : undefined}
      className={`group relative flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm transition ${
        active
          ? 'bg-ink-800 font-medium text-ink-100'
          : 'text-ink-300 hover:bg-ink-850 hover:text-ink-100'
      }`}
    >
      {/* active accent bar */}
      <span
        className={`absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full bg-brand-400 transition-opacity ${
          active ? 'opacity-100' : 'opacity-0'
        }`}
      />
      <span className={`h-4 w-4 shrink-0 ${active ? 'text-brand-400' : 'text-ink-400 group-hover:text-ink-200'}`}>
        {ICONS[id]}
      </span>
      {label}
    </button>
  )
}

export function Sidebar({
  page,
  onNavigate,
  importerVersion,
}: {
  page: Page
  onNavigate: (p: Page) => void
  importerVersion?: string
}) {
  return (
    <aside className="flex w-56 shrink-0 flex-col border-r border-ink-800 bg-ink-900/40">
      <nav className="flex-1 space-y-6 p-3">
        {GROUPS.map((g) => (
          <div key={g.label}>
            <div className="mb-1.5 px-2.5 text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-400">
              {g.label}
            </div>
            <div className="space-y-0.5">
              {g.items.map((it) => (
                <NavItem key={it.id} id={it.id} label={it.label} active={page === it.id} onNavigate={onNavigate} />
              ))}
            </div>
          </div>
        ))}

        <div className="border-t border-ink-800 pt-4">
          <NavItem id="docs" label="Docs & Help" active={page === 'docs'} onNavigate={onNavigate} />
        </div>
      </nav>

      {importerVersion && (
        <div
          title="Pinned Importer version for this audit run — recorded for attestation"
          className="border-t border-ink-800 px-4 py-3"
        >
          <div className="text-[10px] font-medium uppercase tracking-[0.1em] text-ink-400">Importer</div>
          <div className="tnum mt-0.5 font-mono text-xs text-ink-300">{importerVersion}</div>
        </div>
      )}
    </aside>
  )
}
