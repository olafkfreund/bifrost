# Contributing to Bifrost

Thanks for helping build Bifrost — the review-first orchestration layer that turns Azure DevOps →
GitHub Actions migration into a portfolio-scale, semantically-reviewed, attestable workflow. This
guide covers how work is driven and the rules every change follows.

## Ground rule: wrap, don't fork

Bifrost **wraps** GitHub's official tools (`gh actions-importer`, GEI/`ado2gh`) and never
reimplements their conversion logic. Every external tool or API is called behind a **trait** so it
can be mocked in tests. If you find yourself reimplementing a converter, stop — that belongs to the
upstream tool.

See [`CLAUDE.md`](CLAUDE.md) for the full architecture and the non-negotiable hard rules
(review-first, air-gap capable, deterministic risk, grounded generation, attestable, no secrets in
code or logs).

## How work is driven

Work is **issue-first** and **milestone-ordered** (M0 → M6). The backlog is bootstrapped by
[`seed-issues.sh`](seed-issues.sh).

1. Pick the next issue in the current milestone's epic.
2. Read the issue **and** the relevant section of `bifrost-implementation-plan.md`.
3. Implement on a feature branch — **one PR per issue**.
4. Open the PR with `Closes #<n>` in the body, then tick the epic's checklist.

Issue labels say where work lands: `plane:{ingestion,control,portal,llm}`,
`area:{validation,compliance}`, `type:{feature,chore,spike,docs}`, `priority:{p0,p1,p2}`, `epic`.

## Branch + commit conventions

- **Branch per issue**, never commit to `main` directly. Suggested prefixes: `feat/`, `fix/`,
  `docs/`, `chore/`.
- **[Conventional Commits](https://www.conventionalcommits.org/)** for messages, e.g.
  `feat(api): capture converted run result (#59)`.
- **One PR per issue**; the PR body must include `Closes #<n>`.

## Local development

On NixOS or any machine with Nix + flakes, the dev shell provides the pinned toolchain:

```bash
nix develop                       # Rust (pinned), Node 22, gh, azure-cli, docker client
cargo build                       # build the workspace
cargo test                        # run all tests
cd portal && npm ci && npm run dev # portal (mock data) on http://localhost:5173
```

The GitHub Actions Importer is a `gh` extension + Docker image (not in nixpkgs); install once:

```bash
gh extension install github/gh-actions-importer
gh actions-importer configure
```

Secrets (ADO PAT, GitHub token, model keys) live in a **gitignored** `.envrc` — `source .envrc`.
Never commit secret values; record secret *names* only.

## Definition of done (per issue)

A change is done when **all** of these hold:

- [ ] Code + tests + docs updated.
- [ ] `cargo fmt --check` clean.
- [ ] `cargo clippy --all-targets -- -D warnings` clean (a required CI check).
- [ ] `cargo test` green; the portal builds + lints (`npm run build`, `npm run lint`).
- [ ] Where relevant, a captured fixture added under `/fixtures` so behaviour is reproducible.
- [ ] The PR references the issue (`Closes #<n>`) and CI is green.

## Testing rules

- Unit-test the **risk model** and **log parsers** against captured Importer fixtures in
  `/fixtures` — record real `audit_summary.md` / dry-run output so behaviour is reproducible.
- Tests must run **offline** by default. Anything that hits a real external service (Docker,
  GitHub, ADO, a paid LLM, real CI) is gated behind an env flag **and** marked `#[ignore]`.
- LLM calls go through the `LlmProvider` trait — never a vendor SDK from orchestration code.

## Security

- No secrets in code, logs, or commits. Variable groups / service connections record names and
  types only — never values.
- All live/auth/tenancy paths are **opt-in via env** and mock/open by default, so the air-gapped
  single-box path always works.

## License

By contributing, you agree your contributions are licensed under the [MIT License](LICENSE).
