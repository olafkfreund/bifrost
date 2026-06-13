# Migration showcase

Artifacts behind the Bifrost migration screencast — a **real** migration of a live Azure DevOps
organisation (`dev.azure.com/olaffreund0455`, 3 projects / 6 YAML pipelines) to GitHub Actions.

- **Video:** [`docs/assets/showcase/bifrost-showcase.mp4`](../docs/assets/showcase/bifrost-showcase.mp4) (also on the [docs site](https://bitfrost.freundcloud.com/showcase.html)).
- **Transcript:** [`transcript.md`](transcript.md) — section-by-section narration, commands, gaps table, links.
- **Voiceover script:** [`voiceover.md`](voiceover.md) — timed narration to record over the video.

## Public artifacts created during the recording

| Project | Repository | PRs (Bifrost-opened) |
|---|---|---|
| Contoso-Payments | https://github.com/olafkfreund/contoso-payments | [#1](https://github.com/olafkfreund/contoso-payments/pull/1) · [#2](https://github.com/olafkfreund/contoso-payments/pull/2) |
| Northwind-Logistics | https://github.com/olafkfreund/northwind-logistics | [#1](https://github.com/olafkfreund/northwind-logistics/pull/1) · [#2](https://github.com/olafkfreund/northwind-logistics/pull/2) |
| Fabrikam-Identity | https://github.com/olafkfreund/fabrikam-identity | [#1](https://github.com/olafkfreund/fabrikam-identity/pull/1) · [#2](https://github.com/olafkfreund/fabrikam-identity/pull/2) |

Program board: https://github.com/users/olafkfreund/projects/8

## How it was produced (`scripts/`)

| Script | Role |
|---|---|
| `term.sh` | The narrated terminal session (real `bifrost audit`, converted-workflow view, `gh pr list`). Recorded with `asciinema rec --window-size 120x34`, rendered to GIF with `agg`. |
| `migrate.sh` | Per project: create a public repo, seed the original ADO source, then run Bifrost convert → approve → commit to open the PRs (via a commit-enabled API on `:8097`, `BIFROST_COMMIT_LIVE`). |
| `showcase-portal.mjs` | Playwright (`recordVideo`) walkthrough of the **live** portal — heatmap, program board (Board/Roadmap/Issues), forecast, readiness, the three-pane diff. |
| `showcase-github.mjs` | Playwright tour of the public GitHub repos, PR diffs, and the board. |
| `stitch.sh` / `fix_outro.sh` | ffmpeg: title/section/outro cards, lower-third captions, concat to the final 1280×720 MP4. |

The portal walkthrough drives the live API serving a merged 3-project portfolio
(`BIFROST_PORTFOLIO`, pipeline ids set to names so converted workflow filenames are clean).

## Reproduce

```bash
# 1. converted workflows + per-project portfolios (Importer via Docker)
bifrost audit --project <Project> --json > <Project>.portfolio.json    # x3, then merge

# 2. live portal (merged portfolio) on :8099, portal dev server with VITE_API=http
BIFROST_PORTFOLIO=merged.json BIFROST_CONVERT_LIVE=1 bifrost-api
cd portal && VITE_API=http BIFROST_API_TARGET=http://127.0.0.1:8099 npm run dev

# 3. create repos + Bifrost PRs
bash scripts/migrate.sh <Project> <repo-slug> <Pipeline-A> <Pipeline-B>

# 4. record + stitch
asciinema rec --window-size 120x34 -c "bash scripts/term.sh" term.cast
CHROME_BIN=$(command -v google-chrome) node scripts/showcase-portal.mjs
CHROME_BIN=$(command -v google-chrome) node scripts/showcase-github.mjs
bash scripts/stitch.sh && bash scripts/fix_outro.sh
```

No secrets are committed; `BIFROST_COMMIT_LIVE` / `BIFROST_BOARD_LIVE` are opt-in, and the terminal
recording never echoes a token (verified: zero `ghp_` matches in the cast).
