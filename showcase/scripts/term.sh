#!/usr/bin/env bash
# Narrated terminal segment for the Bifrost showcase. Runs REAL commands.
cd /mnt/data/Source-home/GitHub/Bifrost
BIN=target/debug/bifrost
cyan=$'\e[36m'; grn=$'\e[32m'; yel=$'\e[33m'; dim=$'\e[2m'; bold=$'\e[1m'; rst=$'\e[0m'
say(){ printf "\n${dim}# %s${rst}\n" "$1"; sleep 1.6; }
prompt(){ printf "${grn}bifrost${rst}:${cyan}~/Bifrost${rst}$ ${bold}%s${rst}\n" "$1"; sleep 0.8; }

clear
printf "${bold}${cyan}  Bifrost — Azure DevOps  ->  GitHub Actions, at portfolio scale${rst}\n"
printf "${dim}  Wrap the official Importer. Review-first. Gap-aware. Attestable.${rst}\n"
sleep 2.2

say "1) Audit a real Azure DevOps organisation (read-only)"
prompt "bifrost audit --project Contoso-Payments"
$BIN audit --project Contoso-Payments
sleep 2.4

say "Every project in the org is discovered. Bifrost classifies each pipeline."
sleep 1.6

say "2) The Importer converts the bulk — Bifrost surfaces what it can't"
prompt "sed -n '17,30p' contoso-payments-ci.yml   # the converted GitHub Actions workflow"
sed -n '17,32p' /tmp/bifrost-show/Contoso-Payments/report/pipelines/Contoso-Payments/Contoso-Payments-CI/.github/workflows/contoso-payments-ci.yml
sleep 2.6
say "Those '# no matching transformer' lines are typed Gaps: DownloadSecureFile, SonarQube."
say "Bifrost routes each Gap to a grounded LLM explanation or a manual-task runbook item."
sleep 1.8

say "3) Reviewed in the portal, then committed as a pull request — never a silent write"
prompt "gh pr list --repo olafkfreund/contoso-payments"
gh pr list --repo olafkfreund/contoso-payments
sleep 2.2

say "Three projects migrated to three public repositories — six reviewed PRs:"
for r in contoso-payments northwind-logistics fabrikam-identity; do
  printf "  ${grn}+${rst} github.com/olafkfreund/%s\n" "$r"; sleep 0.5
done
sleep 1.0
printf "  ${grn}+${rst} board: github.com/users/olafkfreund/projects/8\n"
sleep 2.4
printf "\n${bold}${cyan}  Review it all yourself — every link is public.${rst}\n"
sleep 2.6
