import { useEffect, useMemo, useState } from 'react'
import type { BifrostApi } from '../api/client'
import type { PlannedIssue, ProgramBoardPlan, RiskBand } from '../types'
import { minutes } from '../lib/format'

const num = (n: number) => n.toLocaleString()

type Tab = 'board' | 'roadmap' | 'issues'

/** The seven lifecycle states, in order, used for the board columns. The plan's
 * `fields[0]` (Status) carries the same labels from the core model. */
const STATUS_ORDER = [
  'Not started',
  'Draft',
  'In review',
  'Changes requested',
  'Approved',
  'Committed',
  'Validated',
] as const

/** Wave metadata — the rollout cohorts, in sequence (mirrors bifrost-core). */
const WAVES: { wave: number; name: string; blurb: string }[] = [
  { wave: 1, name: 'Pilot', blurb: 'Low-risk YAML — migrate first to prove the process.' },
  { wave: 2, name: 'Early majority', blurb: 'Amber YAML — standard conversions once the pilot lands.' },
  { wave: 3, name: 'Late majority', blurb: 'Classic / high-risk — the hard tail; most review.' },
]

/** Map the plan's risk string (Green/Amber/Red) onto the shared RiskBand. */
function bandOf(risk: string): RiskBand {
  const r = risk.toLowerCase()
  return r === 'green' || r === 'red' ? (r as RiskBand) : 'amber'
}

const RISK_META: Record<RiskBand, { text: string; bg: string; dot: string }> = {
  green: {
    text: 'text-[var(--color-risk-green)]',
    bg: 'bg-[var(--color-risk-green-dim)]',
    dot: 'bg-[var(--color-risk-green)]',
  },
  amber: {
    text: 'text-[var(--color-risk-amber)]',
    bg: 'bg-[var(--color-risk-amber-dim)]',
    dot: 'bg-[var(--color-risk-amber)]',
  },
  red: {
    text: 'text-[var(--color-risk-red)]',
    bg: 'bg-[var(--color-risk-red-dim)]',
    dot: 'bg-[var(--color-risk-red)]',
  },
}

/** Compact risk pill sized for cards and dense table rows. */
function RiskPill({ risk }: { risk: string }) {
  const m = RISK_META[bandOf(risk)]
  return (
    <span className={`inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium ${m.bg} ${m.text}`}>
      <span className={`h-1.5 w-1.5 rounded-full ${m.dot}`} />
      {risk}
    </span>
  )
}

/** A small external-link glyph that trails a deep-link. */
function ExternalGlyph() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="h-3 w-3">
      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
      <path d="M15 3h6v6M10 14 21 3" />
    </svg>
  )
}

/** Build a GitHub search deep-link for the planned issue. The board may not be
 * provisioned yet, so we link to a search rather than a fixed issue URL — it
 * lands somewhere sensible (the program repo, or org-wide) either way. */
function issueSearchUrl(issue: PlannedIssue): string {
  const q = encodeURIComponent(`${issue.title} in:title`)
  return `https://github.com/search?q=${q}&type=issues`
}

/** The org/repo deep-links the board mirrors. Derived from the plan; honest about
 * being the *planned* location until provisioning runs. */
function deepLinks(plan: ProgramBoardPlan) {
  // `repo` is the bare repo name (e.g. "contoso-migration-program"); the org isn't
  // in the plan, so we link to a repo search scoped by name and the Projects docs —
  // both work pre- and post-provisioning.
  const repoSearch = `https://github.com/search?q=${encodeURIComponent(plan.repo)}&type=repositories`
  const projectsDocs = 'https://docs.github.com/issues/planning-and-tracking-with-projects'
  return { repoSearch, projectsDocs }
}

export function ProgramBoard({ api }: { api: BifrostApi }) {
  const [plan, setPlan] = useState<ProgramBoardPlan | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [tab, setTab] = useState<Tab>('board')

  useEffect(() => {
    api.getProgramBoardPlan().then(setPlan).catch((e) => setError(String(e)))
  }, [api])

  if (error) {
    return (
      <div className="flex flex-1 items-center justify-center text-[var(--color-risk-red)]">
        Failed to load program board: {error}
      </div>
    )
  }
  if (!plan) {
    return (
      <div className="flex flex-1 items-center justify-center text-ink-300">
        <div className="animate-pulse">Planning the program board…</div>
      </div>
    )
  }

  const links = deepLinks(plan)

  return (
    <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
      <div className="mb-5 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold text-ink-100">Program board</h1>
          <p className="text-sm text-ink-300">
            An API-backed mirror of the GitHub Project Bifrost would stand up for{' '}
            <span className="text-ink-200">{plan.projectTitle}</span>. Bifrost computes its own KPIs;
            views and Insights are configured once in GitHub.
          </p>
        </div>
        <a
          href={links.repoSearch}
          target="_blank"
          rel="noreferrer"
          title="Open GitHub to find the program repository (planned name)"
          className="inline-flex items-center gap-1.5 rounded-lg border border-ink-800 px-3 py-1.5 text-xs text-ink-200 transition hover:bg-ink-850 hover:text-ink-100"
        >
          <span className="font-mono">{plan.repo}</span>
          <ExternalGlyph />
        </a>
      </div>

      <KpiHeader plan={plan} />

      {/* tab switcher — mirrors the portfolio view toggle */}
      <div className="mt-6 flex items-center justify-between gap-3">
        <div className="flex overflow-hidden rounded-lg border border-ink-800 text-xs">
          {(['board', 'roadmap', 'issues'] as Tab[]).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={`px-3.5 py-1.5 capitalize transition ${
                tab === t ? 'bg-ink-800 text-ink-100' : 'text-ink-300 hover:bg-ink-850'
              }`}
            >
              {t}
            </button>
          ))}
        </div>
        <span className="hidden text-xs text-ink-400 sm:block">
          Not yet provisioned — this is the planned board. Nothing is created on GitHub until you approve provisioning.
        </span>
      </div>

      <div className="mt-4">
        {tab === 'board' ? (
          <BoardView plan={plan} />
        ) : tab === 'roadmap' ? (
          <RoadmapView plan={plan} />
        ) : (
          <IssuesView plan={plan} />
        )}
      </div>

      <ProvisionNote plan={plan} projectsDocs={links.projectsDocs} />
    </main>
  )
}

/** KPI / velocity header for management reporting. */
function KpiHeader({ plan }: { plan: ProgramBoardPlan }) {
  const k = plan.kpis
  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <div className="bf-card rounded-xl p-4">
        <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-300">Progress</div>
        <div className="tnum mt-1.5 font-display text-2xl font-semibold tracking-tight text-ink-100">
          {k.percentDone}%
        </div>
        <div className="mt-2 flex h-2 w-full overflow-hidden rounded-full bg-ink-800">
          <div
            className="bg-[var(--color-risk-green)]"
            style={{ width: `${Math.min(k.percentDone, 100)}%` }}
            title={`${k.percentDone}% migrated`}
          />
        </div>
      </div>
      <div className="bf-card rounded-xl p-4">
        <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-300">Migrated</div>
        <div className="tnum mt-1.5 font-display text-2xl font-semibold tracking-tight text-ink-100">
          {num(k.migrated)}
          <span className="text-base font-normal text-ink-400"> / {num(k.total)}</span>
        </div>
        <div className="mt-1 text-xs text-ink-300">{num(k.validated)} validated · committed or validated</div>
      </div>
      <div className="bf-card rounded-xl p-4">
        <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-300">In flight</div>
        <div className="tnum mt-1.5 flex items-baseline gap-3 text-sm font-medium">
          <span className="text-[var(--color-brand-400)]">{num(k.inProgress)} in progress</span>
          <span className="text-ink-300">{num(k.notStarted)} not started</span>
        </div>
        <div className="mt-2 flex h-2 w-full overflow-hidden rounded-full bg-ink-800">
          <Velocity migrated={k.migrated} inProgress={k.inProgress} notStarted={k.notStarted} />
        </div>
      </div>
      <div className="bf-card rounded-xl p-4">
        <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-300">Forecast runner-minutes</div>
        <div className="tnum mt-1.5 font-display text-2xl font-semibold tracking-tight text-ink-100">
          {minutes(k.forecastMinutes)}/mo
        </div>
        <div className="mt-1 text-xs text-ink-300">projected GitHub Actions usage</div>
      </div>
    </div>
  )
}

/** Migrated / in-progress / not-started velocity bar. */
function Velocity({ migrated, inProgress, notStarted }: { migrated: number; inProgress: number; notStarted: number }) {
  const total = Math.max(migrated + inProgress + notStarted, 1)
  const seg = (n: number, color: string, title: string) =>
    n > 0 && <div key={title} className={color} style={{ width: `${(n / total) * 100}%` }} title={`${title}: ${n}`} />
  return (
    <>
      {seg(migrated, 'bg-[var(--color-risk-green)]', 'migrated')}
      {seg(inProgress, 'bg-[var(--color-brand-400)]', 'in progress')}
      {seg(notStarted, 'bg-ink-700', 'not started')}
    </>
  )
}

/** Board — a kanban mirror of the Project, columns by Status. */
function BoardView({ plan }: { plan: ProgramBoardPlan }) {
  const byStatus = useMemo(() => {
    const m = new Map<string, PlannedIssue[]>()
    for (const s of STATUS_ORDER) m.set(s, [])
    for (const issue of plan.issues) {
      const bucket = m.get(issue.status)
      if (bucket) bucket.push(issue)
      else m.set(issue.status, [issue])
    }
    return m
  }, [plan])

  return (
    <div className="flex gap-3 overflow-x-auto pb-2">
      {STATUS_ORDER.map((status) => {
        const issues = byStatus.get(status) ?? []
        return (
          <div key={status} className="flex w-72 shrink-0 flex-col">
            <div className="mb-2 flex items-center justify-between px-1">
              <span className="text-xs font-semibold uppercase tracking-[0.08em] text-ink-300">{status}</span>
              <span className="tnum rounded-full bg-ink-800 px-2 py-0.5 text-[11px] font-medium text-ink-300">
                {issues.length}
              </span>
            </div>
            <div className="flex flex-col gap-2 rounded-xl bg-ink-900/40 p-2">
              {issues.length === 0 ? (
                <div className="rounded-lg border border-dashed border-ink-800 px-3 py-6 text-center text-xs text-ink-500">
                  No issues
                </div>
              ) : (
                issues.map((issue, i) => <BoardCard key={`${issue.title}-${i}`} issue={issue} />)
              )}
            </div>
          </div>
        )
      })}
    </div>
  )
}

/** One pipeline card. Deep-links to where the issue would live on GitHub. */
function BoardCard({ issue }: { issue: PlannedIssue }) {
  const wave = WAVES.find((w) => w.wave === issue.wave)
  return (
    <a
      href={issueSearchUrl(issue)}
      target="_blank"
      rel="noreferrer"
      title="Find this issue on GitHub (opens once the board is provisioned)"
      className="group block rounded-lg border border-ink-800 bg-ink-900 p-3 transition hover:border-ink-700 hover:bg-ink-850"
    >
      <div className="flex items-start justify-between gap-2">
        <div className="text-sm font-medium leading-snug text-ink-100">{issue.title}</div>
        <span className="mt-0.5 shrink-0 text-ink-500 opacity-0 transition group-hover:opacity-100">
          <ExternalGlyph />
        </span>
      </div>
      <div className="mt-2.5 flex flex-wrap items-center gap-2">
        <RiskPill risk={issue.risk} />
        <span className="rounded bg-ink-800 px-1.5 py-0.5 text-[11px] text-ink-300">
          Wave {issue.wave}
          {wave ? ` · ${wave.name}` : ''}
        </span>
        <span className="tnum ml-auto font-mono text-[11px] text-ink-400">{minutes(issue.forecastMinutes)} min/mo</span>
      </div>
    </a>
  )
}

/** Roadmap — waves as horizontal lanes, management-readable. */
function RoadmapView({ plan }: { plan: ProgramBoardPlan }) {
  const lanes = useMemo(() => {
    return WAVES.map((w) => {
      const issues = plan.issues.filter((i) => i.wave === w.wave)
      const forecast = issues.reduce((s, i) => s + i.forecastMinutes, 0)
      return { ...w, issues, forecast }
    })
  }, [plan])

  const maxCount = Math.max(...lanes.map((l) => l.issues.length), 1)

  return (
    <div className="space-y-3">
      {/* timeline header */}
      <div className="flex items-center gap-2 px-1 text-[11px] font-medium uppercase tracking-[0.08em] text-ink-400">
        <span>Sequence</span>
        <span className="h-px flex-1 bg-gradient-to-r from-ink-800 via-ink-700 to-ink-800" />
        <span>Pilot first · hard tail last</span>
      </div>

      {lanes.map((lane) => (
        <div key={lane.wave} className="bf-card rounded-xl p-5">
          <div className="flex flex-wrap items-baseline justify-between gap-2">
            <div className="flex items-baseline gap-3">
              <span className="tnum font-display text-lg font-semibold text-ink-100">Wave {lane.wave}</span>
              <span className="font-display text-lg font-semibold text-ink-100">{lane.name}</span>
            </div>
            <div className="tnum text-sm text-ink-300">
              {num(lane.issues.length)} {lane.issues.length === 1 ? 'pipeline' : 'pipelines'} ·{' '}
              {minutes(lane.forecast)} min/mo
            </div>
          </div>
          <p className="mt-1 text-xs text-ink-300">{lane.blurb}</p>

          {/* lane volume bar, scaled across waves so they read comparatively */}
          <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-ink-800">
            <div
              className="h-full rounded-full bg-[var(--color-brand-400)]"
              style={{ width: `${(lane.issues.length / maxCount) * 100}%` }}
            />
          </div>

          {/* issue chips */}
          <div className="mt-4 flex flex-wrap gap-1.5">
            {lane.issues.length === 0 ? (
              <span className="text-xs text-ink-500">No pipelines in this wave.</span>
            ) : (
              lane.issues.map((issue, i) => (
                <span
                  key={`${issue.title}-${i}`}
                  className="inline-flex items-center gap-1.5 rounded-full border border-ink-800 bg-ink-900 px-2.5 py-1 text-xs text-ink-200"
                  title={`${issue.status} · ${minutes(issue.forecastMinutes)} min/mo`}
                >
                  <span className={`h-1.5 w-1.5 rounded-full ${RISK_META[bandOf(issue.risk)].dot}`} />
                  {issue.title.replace(/^Migrate /, '')}
                </span>
              ))
            )}
          </div>
        </div>
      ))}
    </div>
  )
}

type SortKey = 'title' | 'wave' | 'risk' | 'status' | 'forecast'

/** Issues — a per-pipeline table, sortable + filterable, matching PipelineTable. */
function IssuesView({ plan }: { plan: ProgramBoardPlan }) {
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const [waveFilter, setWaveFilter] = useState<string>('all')
  const [sort, setSort] = useState<{ key: SortKey; dir: 1 | -1 }>({ key: 'wave', dir: 1 })

  const rows = useMemo(() => {
    const riskRank: Record<string, number> = { Red: 3, Amber: 2, Green: 1 }
    const statusRank = new Map<string, number>(STATUS_ORDER.map((s, i) => [s, i]))
    const filtered = plan.issues
      .filter((i) => statusFilter === 'all' || i.status === statusFilter)
      .filter((i) => waveFilter === 'all' || String(i.wave) === waveFilter)
    const cmp = (a: PlannedIssue, b: PlannedIssue): number => {
      switch (sort.key) {
        case 'title':
          return a.title.localeCompare(b.title)
        case 'wave':
          return a.wave - b.wave
        case 'risk':
          return (riskRank[a.risk] ?? 0) - (riskRank[b.risk] ?? 0)
        case 'status':
          return (statusRank.get(a.status) ?? 99) - (statusRank.get(b.status) ?? 99)
        case 'forecast':
          return a.forecastMinutes - b.forecastMinutes
      }
    }
    return [...filtered].sort((a, b) => cmp(a, b) * sort.dir)
  }, [plan, statusFilter, waveFilter, sort])

  const toggleSort = (key: SortKey) =>
    setSort((s) => (s.key === key ? { key, dir: (s.dir * -1) as 1 | -1 } : { key, dir: 1 }))

  // Sortable header cell. A plain render helper (not a component) so the active
  // sort state stays in this view without re-creating a component each render.
  const th = (label: string, k: SortKey, align?: 'right') => (
    <th className={`px-4 py-2.5 font-medium ${align === 'right' ? 'text-right' : ''}`}>
      <button
        onClick={() => toggleSort(k)}
        className={`inline-flex items-center gap-1 transition hover:text-ink-100 ${sort.key === k ? 'text-ink-100' : ''}`}
      >
        {label}
        <span className={`text-[9px] ${sort.key === k ? 'opacity-100' : 'opacity-30'}`}>
          {sort.key === k && sort.dir === -1 ? '▲' : '▼'}
        </span>
      </button>
    </th>
  )

  const waveName = (w: number) => WAVES.find((x) => x.wave === w)?.name ?? ''

  return (
    <div>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <select
          value={waveFilter}
          onChange={(e) => setWaveFilter(e.target.value)}
          className="bf-field px-3 py-1.5 text-xs"
          title="Filter by wave"
        >
          <option value="all">All waves</option>
          {WAVES.map((w) => (
            <option key={w.wave} value={String(w.wave)}>
              Wave {w.wave} · {w.name}
            </option>
          ))}
        </select>
        <select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value)}
          className="bf-field px-3 py-1.5 text-xs"
          title="Filter by status"
        >
          <option value="all">All statuses</option>
          {STATUS_ORDER.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        <span className="tnum ml-auto text-xs text-ink-400">
          {num(rows.length)} of {num(plan.issues.length)} pipelines
        </span>
      </div>

      <div className="overflow-hidden rounded-xl border border-ink-800">
        <table className="w-full text-left text-sm">
          <thead className="bg-ink-900 text-xs uppercase tracking-wide text-ink-300">
            <tr>
              {th('Pipeline', 'title')}
              {th('Wave', 'wave')}
              {th('Risk', 'risk')}
              {th('Status', 'status')}
              <th className="hidden px-4 py-2.5 font-medium sm:table-cell">Checklist</th>
              {th('min/mo', 'forecast', 'right')}
              <th className="px-4 py-2.5 text-right font-medium">GitHub</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-ink-800">
            {rows.map((issue, i) => (
              <tr key={`${issue.title}-${i}`} className="bg-ink-900/30 hover:bg-ink-850">
                <td className="px-4 py-2.5 font-medium text-ink-100">{issue.title.replace(/^Migrate /, '')}</td>
                <td className="px-4 py-2.5 text-ink-300">
                  <span className="rounded bg-ink-800 px-1.5 py-0.5 text-xs">
                    {issue.wave} · {waveName(issue.wave)}
                  </span>
                </td>
                <td className="px-4 py-2.5">
                  <RiskPill risk={issue.risk} />
                </td>
                <td className="px-4 py-2.5 text-ink-300">{issue.status}</td>
                <td className="hidden px-4 py-2.5 text-ink-300 sm:table-cell">
                  <span className="tnum font-mono">{issue.subIssues.length}</span> tasks
                </td>
                <td className="tnum px-4 py-2.5 text-right font-mono text-ink-300">{minutes(issue.forecastMinutes)}</td>
                <td className="px-4 py-2.5 text-right">
                  <a
                    href={issueSearchUrl(issue)}
                    target="_blank"
                    rel="noreferrer"
                    title="Find this issue on GitHub"
                    className="inline-flex items-center gap-1 text-ink-400 transition hover:text-ink-100"
                  >
                    Open
                    <ExternalGlyph />
                  </a>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

/** Honest note about provisioning — review-first, no auto-run. The fields list
 * makes the planned Project schema legible to a buyer. */
function ProvisionNote({ plan, projectsDocs }: { plan: ProgramBoardPlan; projectsDocs: string }) {
  return (
    <div className="mt-8 bf-card rounded-xl p-5">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="max-w-2xl">
          <h2 className="text-sm font-semibold text-ink-100">Planned GitHub Project</h2>
          <p className="mt-0.5 text-xs text-ink-300">
            Provisioning creates an org-level Project and a dedicated repo (<span className="font-mono">{plan.repo}</span>),
            sets the custom fields below, and opens one issue per pipeline with the migration checklist as sub-issues.
            It is review-first and idempotent — an administrator approves it, and every run is appended to the
            attestation log. Nothing here triggers that action.
          </p>
          <div className="mt-3 flex flex-wrap gap-1.5">
            {plan.fields.map((f) => (
              <span
                key={f.name}
                className="inline-flex items-center gap-1.5 rounded-full border border-ink-800 bg-ink-900 px-2.5 py-1 text-xs text-ink-200"
                title={f.options.length > 0 ? f.options.join(' · ') : f.dataType}
              >
                {f.name}
                <span className="text-[10px] uppercase tracking-wide text-ink-500">{f.dataType}</span>
              </span>
            ))}
          </div>
        </div>
        <a
          href={projectsDocs}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1.5 rounded-lg border border-ink-800 px-3 py-1.5 text-xs text-ink-200 transition hover:bg-ink-850 hover:text-ink-100"
          title="Provisioning is an administrator action approved outside this view — read how Projects views are configured"
        >
          How provisioning works
          <ExternalGlyph />
        </a>
      </div>

      {plan.notes.length > 0 && (
        <ul className="mt-4 space-y-1 border-t border-ink-800 pt-3 text-xs text-ink-400">
          {plan.notes.map((n, i) => (
            <li key={i} className="flex gap-2">
              <span className="mt-1.5 h-1 w-1 shrink-0 rounded-full bg-ink-600" />
              {n}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
