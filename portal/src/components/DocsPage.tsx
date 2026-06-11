// In-portal Docs / Help. Curated JSX (no markdown-runtime dependency) using the
// shared Gruvbox tokens. Distinct from the external Jekyll docs site.

function Step({ n, title, children }: { n: number; title: string; children: React.ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-ink-800 text-xs font-semibold text-brand-400">
        {n}
      </span>
      <div>
        <div className="text-sm font-medium text-ink-100">{title}</div>
        <div className="text-sm text-ink-300">{children}</div>
      </div>
    </li>
  )
}

function Code({ children }: { children: React.ReactNode }) {
  return <code className="rounded bg-ink-800 px-1.5 py-0.5 font-mono text-[0.85em] text-ink-100">{children}</code>
}

function Fix({ symptom, children }: { symptom: string; children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-ink-800 bg-ink-850 p-4">
      <div className="text-sm font-semibold text-ink-100">{symptom}</div>
      <div className="mt-1 text-sm text-ink-300">{children}</div>
    </div>
  )
}

const SECTIONS = [
  { id: 'getting-started', title: 'Getting started' },
  { id: 'using', title: 'Using the portal' },
  { id: 'backend', title: 'Connecting to live data' },
  { id: 'troubleshooting', title: 'Troubleshooting' },
] as const

export function DocsPage() {
  return (
    <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
      <div className="mb-5">
        <h1 className="text-xl font-semibold text-ink-100">Docs &amp; Help</h1>
        <p className="text-sm text-ink-300">
          How to get started, drive a migration review, and get unstuck. Bifrost wraps the official
          GitHub migration tools — it never reimplements their conversion logic.
        </p>
      </div>

      <div className="grid gap-8 lg:grid-cols-[200px_1fr]">
        {/* sticky table of contents */}
        <nav className="hidden lg:block">
          <ul className="sticky top-6 space-y-1 text-sm">
            {SECTIONS.map((s) => (
              <li key={s.id}>
                <a href={`#${s.id}`} className="block rounded-md px-3 py-1.5 text-ink-300 hover:bg-ink-850 hover:text-ink-100">
                  {s.title}
                </a>
              </li>
            ))}
          </ul>
        </nav>

        <div className="max-w-3xl space-y-10">
          <section id="getting-started" className="scroll-mt-6">
            <h2 className="text-lg font-semibold text-ink-100">Getting started</h2>
            <p className="mt-2 text-sm text-ink-300">
              Bifrost turns one-at-a-time, CLI-only Azure&nbsp;DevOps → GitHub&nbsp;Actions conversion into
              a portfolio-scale, semantically-reviewed, human-approved migration. It is{' '}
              <span className="font-medium text-ink-100">review-first</span>: nothing is silently
              committed to your CI — every change is recommended, explained, and waits for your approval.
            </p>
            <ol className="mt-4 space-y-3">
              <Step n={1} title="Audit the org">
                The portfolio heatmap scores every pipeline Green / Amber / Red from the Importer audit
                + source inventory.
              </Step>
              <Step n={2} title="Drill into a pipeline">
                Click a tile (or table row) to open the drawer: converted %, unsupported steps, manual
                tasks, and the deterministic risk-factor breakdown.
              </Step>
              <Step n={3} title="Open the proposal">
                <Code>Open proposal →</Code> runs the conversion loop and shows the assembled workflow,
                the model's rationale, risk flags, verify steps, and the manual-task runbook.
              </Step>
              <Step n={4} title="Review &amp; approve">
                Approve, edit, or request changes (landing next). Approval drives the proposal state
                machine and is recorded in the append-only audit log.
              </Step>
              <Step n={5} title="Commit &amp; validate">
                The approved workflow is pushed / PR'd and smoke-validated against the ADO baseline
                (opt-in, never silent).
              </Step>
            </ol>
          </section>

          <section id="using" className="scroll-mt-6">
            <h2 className="text-lg font-semibold text-ink-100">Using the portal</h2>
            <ul className="mt-3 space-y-2 text-sm text-ink-300">
              <li>
                <span className="font-medium text-ink-100">Heatmap / Table</span> — toggle top-right.
                The <span className="font-medium text-ink-100">risk filter</span> (All / Green / Amber /
                Red) narrows both views.
              </li>
              <li>
                <span className="font-medium text-ink-100">Risk is deterministic</span> — Green&nbsp;&lt;&nbsp;34
                ≤ Amber &lt;&nbsp;67 ≤ Red, computed from weighted factors. The LLM explains and flags;
                it never produces the score.
              </li>
              <li>
                <span className="font-medium text-ink-100">Proposal panel</span> — the proposed workflow
                keeps the Importer's output above the <Code>REVIEW BEFORE USE</Code> banner and the
                model's gap-fills below it, each tagged with its source construct and prompt id
                (provenance).
              </li>
              <li>
                <span className="font-medium text-ink-100">Runbook</span> — the things the Importer can't
                do for you: secrets to provision, service connections to federate via OIDC, runners,
                and approval gates to recreate as Environments.
              </li>
            </ul>
          </section>

          <section id="backend" className="scroll-mt-6">
            <h2 className="text-lg font-semibold text-ink-100">Connecting to live data</h2>
            <p className="mt-2 text-sm text-ink-300">
              By default the portal runs standalone on mock fixtures. Point it at the Rust control plane
              to see live data:
            </p>
            <ol className="mt-3 space-y-2 text-sm text-ink-300">
              <li>
                1. Start the API: <Code>cargo run -p bifrost-api</Code> (defaults to{' '}
                <Code>127.0.0.1:8080</Code>).
              </li>
              <li>
                2. Run the portal against it: <Code>VITE_API=http npm run dev</Code>. The dev server
                proxies <Code>/api</Code> → <Code>BIFROST_API_TARGET</Code>.
              </li>
              <li>
                3. Portfolio source resolves in order: live ADO audit (<Code>BIFROST_PROJECT</Code> +{' '}
                <Code>AZDO_*</Code> creds) → <Code>BIFROST_PORTFOLIO</Code> file → built-in sample.
              </li>
              <li>
                4. The proposal panel calls <Code>POST /api/pipelines/:id/convert</Code>.
              </li>
            </ol>
            <p className="mt-3 text-sm text-ink-300">
              <span className="font-medium text-ink-100">Air-gap mode</span> runs with a local model only
              — frontier providers are disabled and no pipeline data leaves the box. When it's on, the
              header shows an <span className="font-medium" style={{ color: 'var(--color-accent-aqua)' }}>Air-gap</span> badge.
            </p>
          </section>

          <section id="troubleshooting" className="scroll-mt-6">
            <h2 className="text-lg font-semibold text-ink-100">Troubleshooting</h2>
            <div className="mt-3 space-y-3">
              <Fix symptom="Blank page, or “_jsxDEV is not a function” in the console">
                A global <Code>NODE_ENV=production</Code> makes Vite bundle the production React build in
                dev. Start with <Code>npm run dev</Code> (it sets <Code>NODE_ENV=development</Code>), or
                unset the global <Code>NODE_ENV</Code>.
              </Fix>
              <Fix symptom="Dev server unreachable on 127.0.0.1">
                Vite binds IPv6 <Code>[::1]</Code> — open <Code>http://localhost:PORT</Code> instead of{' '}
                <Code>127.0.0.1</Code>.
              </Fix>
              <Fix symptom="“Failed to load portfolio”">
                The backend isn't running (or <Code>VITE_API=http</Code> with no API up). The portal
                defaults to mock mode and runs with zero backend — only set <Code>VITE_API=http</Code>{' '}
                once the API is listening.
              </Fix>
              <Fix symptom="Proposal panel says “Conversion failed”">
                The API is an older build without the convert route. Rebuild and restart{' '}
                <Code>bifrost-api</Code>, then retry <Code>Open proposal →</Code>.
              </Fix>
              <Fix symptom="Importer image not found">
                The official image is <Code>ghcr.io/actions-importer/cli:latest</Code>. Bifrost shells
                out to it; it is never reimplemented.
              </Fix>
            </div>
          </section>
        </div>
      </div>
    </main>
  )
}
