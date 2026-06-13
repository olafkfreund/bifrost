#!/usr/bin/env bash
# Create a public repo, seed the ADO source, then run Bifrost convert->approve->commit
# for each pipeline (real PRs). One project per invocation.
set -uo pipefail
cd /mnt/data/Source-home/GitHub/Bifrost
SHOW=/tmp/bifrost-show
P="$1"; SLUG="$2"; shift 2; PIPES=("$@")
REPO="olafkfreund/$SLUG"
echo "############ $P -> $REPO ############"

# 1. public repo + README
gh repo create "$REPO" --public --add-readme \
  --description "$P CI/CD — migrated from Azure DevOps to GitHub Actions by Bifrost (review-first, gap-aware)" >/dev/null 2>&1
sleep 2
# 2. seed original ADO source onto main
for d in "$SHOW/$P"/report/pipelines/"$P"/*/; do
  pl=$(basename "$d"); src="$d/source.yml"; [ -f "$src" ] || continue
  b64=$(base64 -w0 "$src")
  curl -s -X PUT -H "authorization: token $GITHUB_TOKEN" -H "user-agent: bifrost" \
    "https://api.github.com/repos/$REPO/contents/azure-pipelines/$pl.yml" \
    -d "{\"message\":\"chore: import original ADO pipeline $pl (pre-migration baseline)\",\"content\":\"$b64\"}" \
    -o /dev/null -w "  seeded azure-pipelines/$pl.yml HTTP %{http_code}\n"
done

# 3. launch commit-API targeting this repo
env -u BIFROST_PROJECT -u BIFROST_AUTH \
  BIFROST_PORTFOLIO="$SHOW/merged.portfolio.json" \
  BIFROST_CONVERT_LIVE=1 BIFROST_COMMIT_LIVE=1 \
  BIFROST_GH_REPO="$REPO" BIFROST_GH_BASE=main \
  BIFROST_DB="sqlite:///tmp/bifrost-commit-$SLUG.db" \
  BIFROST_API_ADDR=127.0.0.1:8097 RUST_LOG=warn \
  target/debug/bifrost-api >"$SHOW/commit-$SLUG.log" 2>&1 &
APIPID=$!
# poll health
for i in $(seq 1 20); do
  [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8097/api/health)" = "200" ] && break
  sleep 1
done

C=http://127.0.0.1:8097
for PL in "${PIPES[@]}"; do
  echo "== $PL =="
  curl -s -X POST "$C/api/pipelines/$PL/convert" -o /dev/null -w "  convert %{http_code}\n"
  for to in in_review approved; do
    curl -s -o /dev/null -w "  ->$to %{http_code}\n" -X POST "$C/api/proposals/prop-$PL/transition" \
      -H 'content-type: application/json' -d "{\"to\":\"$to\",\"actor\":\"olaf@portal\"}"
  done
  curl -s -X POST "$C/api/proposals/prop-$PL/commit" -o /tmp/commit.json -w "  commit %{http_code}\n"
  pr=$(python3 -c "import json;d=json.load(open('/tmp/commit.json'));p=d.get('proposal',d);print(p.get('prUrl',p.get('pr_url','?')))" 2>/dev/null)
  echo "  PR=$pr"; echo "$pr" >> "$SHOW/prs.txt"
done

kill "$APIPID" 2>/dev/null; wait "$APIPID" 2>/dev/null
echo "done $P"
exit 0
