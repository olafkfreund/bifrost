{# Bifrost grounded gap-fill prompt — version v1.
   Referenced by id ("gap-fill.v1") so prompt changes are auditable.
   Placeholders ({{name}}) are substituted by build_gap_fill_prompt. #}
You are assisting a review-first Azure DevOps → GitHub Actions migration.

The official GitHub Actions Importer has already converted most of a pipeline. A
single construct could not be fully converted. Fill **only that gap**, working
from the diff below — do NOT convert the pipeline from scratch, and do NOT invent
steps beyond what the source requires.

## Source construct (Azure DevOps)
```
{{source_snippet}}
```

## Importer's converted output so far (GitHub Actions)
```
{{converted_yaml}}
```

## What the Importer could not handle
{{importer_message}}

## Repository context
{{repo_context}}

## Your task
Produce the minimal GitHub Actions YAML that closes this specific gap, plus a
short rationale and concrete verification steps. Respond ONLY with JSON matching:

```json
{
  "proposed_yaml": "<the YAML fragment that fills the gap>",
  "rationale": "<why this is equivalent to the source construct>",
  "risk_flags": ["<things a human reviewer must check>"],
  "verify_steps": ["<how to confirm parity before approving>"],
  "confidence": 0.0
}
```

Rules:
- Stay grounded in the diff above; if the source intent is ambiguous, say so in
  `risk_flags` rather than guessing.
- Do NOT output a numeric risk score or rating — risk is scored deterministically
  elsewhere. `confidence` (0.0–1.0) is only your certainty in the proposed YAML.
- Never include secret values; reference secrets as `${{ secrets.NAME }}`.
