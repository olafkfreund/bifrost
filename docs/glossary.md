---
title: Glossary
layout: default
nav_order: 4
permalink: /glossary
---

# Domain Glossary

| Term | Meaning |
|---|---|
| **Importer** | `gh actions-importer` — the official GitHub CLI extension (audit / forecast / dry-run / migrate). |
| **GEI / ado2gh** | Repo migration tooling (history, branches, metadata). Out of scope for conversion; Bifrost orchestrates and tracks it only. |
| **Gap** | A construct the Importer marked partial or unsupported, routed to the LLM for a grounded fill. |
| **Proposal** | An augmented workflow + rationale + risk, awaiting human review. Moves through `draft → in_review → approved/changes_requested → committed → validated`. |
| **Parity report** | A smoke-validation result comparing a converted workflow run to the ADO baseline (status, artifact set, declared outputs). **Smoke parity, not full equivalence.** |
| **Classic pipeline** | An ADO designer pipeline — the hard tail; defaults to Amber/Red and is flagged for manual work. |
| **YAML pipeline** | An ADO YAML pipeline — the easy path. |
| **Air-gap mode** | A configuration where only a local model (Ollama / llama.cpp) is used and all frontier providers are disabled, so no pipeline data leaves the network. |
| **Attestation** | A signed, exportable record of a migration decision (who approved what, what changed, why, and the validation result). |
| **SourceAdapter** | The trait abstracting a CI source platform (`discover`, `enumerate_pipelines`, `fetch_definition`, …). ADO is the first implementation. |
| **LlmProvider** | The trait abstracting an LLM backend, returning structured JSON `{ proposed_yaml, rationale, risk_flags[], verify_steps[], confidence }`. Orchestration calls only this trait. |
