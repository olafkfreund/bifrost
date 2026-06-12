# Getting started

## Build and test

Bifrost is a Rust (Cargo) workspace with a React/TS portal.

```bash
cargo build                          # build the workspace
cargo fmt --check                    # formatting gate
cargo clippy -- -D warnings          # lint gate (required CI check)
cargo test                           # run all tests
cargo test -p bifrost-core <name>    # a single crate / test by name
```

CI runs the format check, `clippy -D warnings`, build, and test on every push and
pull request. Tests for the risk model and log parsers run against captured Importer
fixtures.

The portal:

```bash
cd portal
npm install
npm run dev        # standalone with mock data
npm run build      # production build
npm run lint
```

## How work is driven

Work is issue-first and milestone-ordered. Each issue gets one pull request whose
body references the issue (`Closes #<n>`). Milestone order (M0 through M6) is the
build sequence.

## Running a migration

1. **Connect a source.** On the portal's Settings page, add an Azure DevOps
   connection (or Jenkins, GitLab, CircleCI, Travis, Bamboo). Bifrost stores a
   *reference* to the secret — a Key Vault URI, a GitHub App, an Entra federation, or
   an env-var name — never the value.
2. **Audit.** Bifrost runs the Importer across the org and builds the portfolio
   heatmap. Nothing is changed.
3. **Assess.** Download the per-project status report (Markdown or PDF) for the
   change advisory board. It lists what will change and what must be set up in GitHub:
   secrets to create, variables, service connections to re-federate, and the Actions
   allow-list.
4. **Review.** Open a pipeline's proposal. The three-pane view shows the original
   pipeline, the converted workflow with the model's gap-fills highlighted, and the
   rationale, risk flags, verify steps, and manual-task runbook. Approve, request
   changes, or edit inline.
5. **Deliver.** An approved proposal is delivered as a pull request. The base branch
   is never written to directly.

## Air-gap mode

In an enterprise network that can reach in-network model endpoints but not the public
internet, turn on air-gap mode. Bifrost disables every frontier provider, routes all
conversion through a local Ollama model or an in-network endpoint, and asserts that no
pipeline data leaves the box. Live, authenticated, and tenancy paths are all opt-in
behind environment flags; the default posture is mock and open.

## No secrets in code or logs

Secret names discovered during an audit are data; secret values are never fetched or
stored. Connections record names and types only.
