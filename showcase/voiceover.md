# Bifrost showcase — voiceover script

Timed narration for `bifrost-showcase.mp4` (~2:02). No synthetic TTS was used; record this in your
own voice (or a VO artist). Timings are approximate cue points; pace to the on-screen action.

| Cue | On screen | Narration |
|---|---|---|
| 0:00 | Title card | "Migrating CI/CD from Azure DevOps to GitHub Actions is usually one pipeline at a time, by hand. Bifrost does it at portfolio scale — wrapping GitHub's own Importer, keeping a human in the loop." |
| 0:07 | CLI — audit | "It starts with a read-only audit of a real Azure DevOps organisation. Bifrost discovers every project and classifies every pipeline." |
| 0:18 | CLI — converted workflow + gaps | "The Importer converts the bulk automatically. What it can't convert — a secure-file download, a SonarQube scan — Bifrost captures as a typed gap, right here in the diff." |
| 0:28 | CLI — gh pr list | "Each gap is routed to a grounded explanation or a manual-task checklist. Nothing is guessed, and nothing is silently rewritten." |
| 0:35 | Portal — heatmap | "In the portal, the whole estate is one heatmap: six pipelines across three projects, colour-coded by migration risk." |
| 0:48 | Portal — program board | "A program board mirrors the GitHub Project Bifrost would stand up — one issue per pipeline, planned in waves, with KPIs Bifrost computes itself. Nothing is created on GitHub until you approve it." |
| 1:00 | Portal — 3-pane diff | "Open any pipeline and you get the three-pane review: the original Azure DevOps YAML, the converted GitHub Actions workflow, the deterministic risk score, and the runbook for the gaps." |
| 1:08 | GitHub — repo | "Approve, and Bifrost commits — as a pull request. Here are the real, public repositories it created." |
| 1:20 | GitHub — PR diff | "Every converted workflow arrives as a reviewable PR, with the gap comments preserved inline and the risk and manual tasks in the description." |
| 1:35 | GitHub — board | "And the program board, live on GitHub — the whole migration, tracked on GitHub's own features." |
| 2:00 | Outro | "Three projects, six pipelines, six pull requests, one board — all public. Review it yourself." |
