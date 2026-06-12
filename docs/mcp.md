---
title: Bifrost in your editor (MCP)
layout: default
nav_order: 5
permalink: /mcp
description: "Use the Bifrost MCP server from VS Code, Cursor or any MCP client — query the migration portfolio and (next) convert a pipeline to a GitHub Actions workflow without leaving the editor."
---

# Bifrost in your editor
{: .fs-9 }

Bifrost ships a Model Context Protocol (MCP) server. Point VS Code agent mode (or any
MCP client) at it and your AI assistant can read the whole migration picture — and,
on the roadmap, convert a pipeline to a GitHub Actions workflow — grounded in the
official GitHub Importer, never hallucinated.
{: .fs-6 .fw-300 }

---

## What MCP gives you here

[MCP](https://modelcontextprotocol.io) is a standard way for an AI assistant to call
external tools. VS Code, Cursor, Claude Desktop and the GitHub Copilot coding agent all
speak it. The Bifrost MCP server (`bifrost-mcp`) is a small stdio JSON-RPC process that
proxies the Bifrost control-plane API, so an assistant in your editor gets the same
deterministic answers the portal shows — risk, forecast, coverage, readiness, the program
board — as structured tool results it can reason over.

The point is **grounding**. Ask a generic assistant "rewrite this Azure Pipelines YAML as a
GitHub workflow" and it guesses from training data. Ask it through Bifrost and it runs the
official `gh actions-importer`, reads the real conversion output and the specific
unsupported steps, and explains the gap from that — the same review-first, attestable path
the portal uses.

---

## Tools available today

The server is **read-only** today by design: it gives an assistant full situational
awareness without the power to change anything. Every tool is a deterministic read.

| Tool | What it answers |
|------|-----------------|
| `bifrost_portfolio` | Every pipeline with project, classification (YAML/classic), risk band + score, converted ratio, proposal status. |
| `bifrost_assessment` | Source (Azure DevOps) assessment: pipeline mix, risk distribution, inventory density (service connections, variable groups, secrets, runners, custom tasks). |
| `bifrost_coverage` | The completeness matrix: each ADO moving-part category mapped to its GitHub equivalent and a status (auto / review / manual / not-inventoried). |
| `bifrost_forecast` | Deterministic GitHub Actions cost + capacity forecast for the target org. |
| `bifrost_readiness` | Target GitHub pre-flight checklist: runners, Actions policy, OIDC federation, secrets, rulesets, ownership, rollback. |
| `bifrost_program_board` | The dry-run plan of the GitHub Projects program board: dedicated repo, org Project, fields, one issue per pipeline, KPIs. |
| `bifrost_report` | The pre-migration status report (Markdown), optionally scoped to one project. |
| `validate_workflow` | A quick structural sanity check on a workflow YAML (has `on:` and `jobs:`). Local; no API call. |

{: .note }
> Read-only is a feature, not a limitation. An assistant can plan, explain, prioritise and
> report all day without ever touching production CI. Writes (convert, commit, PR) stay on
> the gated path described below.

---

## Set it up in VS Code

VS Code reads MCP servers from `.vscode/mcp.json` in your workspace (committable, so the
whole team gets it) or your user profile. The root key is **`servers`** — note that Cursor
and Claude Desktop use `mcpServers` instead; copy-pasting their config is the number-one
setup mistake.

1. Build the server once:

   ```bash
   cargo build -p bifrost-mcp
   ```

2. Start the Bifrost API the server proxies (in another terminal):

   ```bash
   cargo run -p bifrost-api
   ```

3. Add `.vscode/mcp.json`:

   ```json
   {
     "servers": {
       "bifrost": {
         "command": "${workspaceFolder}/target/debug/bifrost-mcp",
         "env": {
           "BIFROST_API_URL": "http://127.0.0.1:8080"
         }
       }
     }
   }
   ```

   `stdio` is implicit for a local command. `BIFROST_API_URL` defaults to
   `http://127.0.0.1:8080`, so you can omit `env` if you run the API there.

4. Open Copilot Chat, switch the mode dropdown to **Agent** (MCP tools are invisible in Ask
   or Edit mode), and the Bifrost tools appear in the tools picker. VS Code asks you to
   confirm each tool call before it runs — that confirmation is your human gate.

The same server works in Cursor and Claude Desktop; only the config key differs
(`mcpServers`).

---

## A real day with it

> **Priya, a platform engineer, is three weeks into migrating 180 Azure DevOps pipelines.**
> She works entirely in VS Code agent mode.

**"Which pipelines should my team take this sprint, and why?"**
The assistant calls `bifrost_portfolio` and `bifrost_program_board`, sees the deterministic
wave assignment and risk bands, and answers: the twelve green YAML pipelines in Wave 1 —
low risk, no classic-designer tail — with the two amber ones flagged for a closer look. No
guessing; the waves come from Bifrost's risk model.

**"What's going to bite us on the `Payments-API` pipeline?"**
It calls `bifrost_coverage` and `bifrost_assessment` scoped to that project and reports the
real gaps: a `DownloadSecureFile@1` task with no automatic equivalent, one service
connection to federate via OIDC, two secrets to recreate. These are the Importer's findings,
surfaced as a checklist — not a generic "you may need to adjust secrets."

**"Draft the management update."**
`bifrost_report` returns the Markdown status report; the assistant trims it into a Slack
post. The numbers match the portal because they came from the same API.

Every one of those is possible **today**, read-only, inside the editor.

---

## The headline: convert a pipeline in the editor

Here is the workflow you asked about — *open a pipeline, ask Bifrost to migrate it, see the
proposed GitHub workflow* — and exactly how it fits the design.

The conversion engine already exists in the Bifrost API:
`POST /api/pipelines/:id/convert` runs the Importer dry-run, detects the gaps, fills them
with grounded LLM context, and returns a **Proposal** (the augmented workflow + rationale +
deterministic risk) plus a **Runbook** (the manual tasks the Importer cannot do for you). The
portal already drives it through the three-pane review.

What is missing is one MCP tool that exposes that endpoint — `bifrost_convert` — so the same
flow runs from your editor:

```text
You (in VS Code, Payments-API azure-pipelines.yml open):
  "Migrate this pipeline to a GitHub Actions workflow."

Assistant → bifrost_convert { pipelineId: "payments-api" }
  Bifrost runs gh actions-importer dry-run, detects gaps, fills them
  grounded in the source + the Importer output + the failure log.

Assistant ← Proposal:
  - proposed .github/workflows/payments-api.yml
  - rationale: what changed and why (e.g. DownloadSecureFile@1 → a
    checkout of a secrets repo gated by OIDC)
  - risk: Amber — one unsupported task gap-filled, verify manually
  - runbook: [ federate azure-prod via OIDC, recreate NUGET_API_KEY,
    label the self-hosted runner ]

You: review the diff in the editor, edit if needed, then approve.
```

Crucially, **`convert` produces a proposal, not a merged PR.** Committing the workflow and
opening the pull request stays a separate, explicitly-approved step
(`POST /api/proposals/:id/commit`), exactly as in the portal. The editor flow makes the
*review* faster; it does not remove the human from the loop.

{: .highlight }
> This is the "perfect migration tool" idea, made real: the engineer opens the legacy
> pipeline, asks once, and gets a reviewable GitHub workflow with the conversion's risks and
> manual follow-ups spelled out — without a hallucinated rewrite, because Bifrost wraps the
> official tool and explains the diff.

---

## How far does the automation go?

Bifrost's hard rules set the ceiling deliberately, and they are what make the automation
trustworthy at portfolio scale:

- **Review-first.** The assistant can take you from legacy YAML to a proposed workflow with
  risk and a runbook autonomously. The commit and the pull request remain human-approved.
  Auto-commit is opt-in and gated behind approval plus validation.
- **Grounded generation only.** Every conversion carries the source snippet, the Importer's
  converted output and the specific failure from the log. The model fills the gap from that
  diff — it never converts a pipeline from scratch.
- **The LLM explains; it does not score.** Risk and cost are deterministic. The assistant can
  explain an Amber rating but cannot move it.
- **Everything is attestable.** Each state transition and human action is appended to an
  immutable audit log, whether it originated in the portal or the editor.
- **Air-gap capable.** Routing can force every model call to a local provider (Ollama), so the
  whole editor flow runs with no pipeline data leaving the box.

A realistic end state: a Copilot agent in a `<org>-migration-program` repo opens the next
Wave-1 pipeline, calls `bifrost_convert`, opens a draft PR with the proposed workflow plus the
runbook as a checklist, and stops — a human reviews, ticks off the manual tasks, and merges.
Bifrost turns "convert 180 pipelines by hand" into "review 180 grounded proposals," in the
tools the team already uses.

---

## Roadmap for the editor flow

| State | Status |
|-------|--------|
| Read-only context tools (portfolio, assessment, coverage, forecast, readiness, program board, report) | Shipped |
| `validate_workflow` structural check | Shipped |
| `bifrost_convert` — convert a pipeline to a proposed workflow from the editor | Planned (wraps the existing `/convert` endpoint) |
| `bifrost_runbook` — read a proposal's manual-task checklist | Planned |
| Gated `bifrost_commit` — open the PR after approval | Planned (approval + live-flag gated) |

The engine is built; extending the editor surface is mostly exposing it through MCP, one
review-first tool at a time.
