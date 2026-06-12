//! Prompt regression / eval harness for the versioned prompts (#103).
//!
//! Two regressions this guards against:
//!
//! 1. **Prompt drift.** `prompts/gap-fill.v1.md` is referenced by id
//!    (`gap-fill.v1`) for auditability. If its text — or the renderer — changes,
//!    the rendered-prompt snapshot below stops matching the committed golden and
//!    the test fails, forcing a *conscious* prompt-version bump rather than a
//!    silent edit. Regenerate the golden intentionally with
//!    `BLESS_PROMPTS=1 cargo test -p bifrost-llm --test prompt_eval`.
//!
//! 2. **Contract drift.** Recorded model outputs (the messy shapes real models
//!    emit — fenced JSON, camelCase, an object `proposed_yaml`, a stray
//!    `risk_score` the model shouldn't send) are replayed through the same
//!    `parse_gap_fill` the providers use, asserting the structured contract holds:
//!    parses to YAML + rationale + flags + steps + confidence, and **never** a
//!    risk score (the LLM explains; it does not score).

use bifrost_core::{Gap, GapKind};
use bifrost_llm::{build_gap_fill_prompt, parse_gap_fill, GapFillRequest, GAP_FILL_PROMPT_ID};

/// The canned gap the golden prompt snapshot is rendered from. Stable on purpose:
/// changing it is changing the regression baseline.
fn canned_request() -> GapFillRequest {
    GapFillRequest {
        gap: Gap {
            kind: GapKind::UnsupportedStep,
            construct: "DownloadSecureFile@1".into(),
            detail: "no GitHub Actions equivalent".into(),
        },
        source_snippet: "- task: DownloadSecureFile@1\n  inputs:\n    secureFile: app.config"
            .into(),
        converted_yaml: "steps:\n  - uses: actions/checkout@v4".into(),
        importer_message: "secure file download has no built-in GitHub Actions equivalent".into(),
        repo_context: "languages: dotnet; build: msbuild".into(),
    }
}

const GOLDEN_PROMPT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/golden/gap-fill.v1.prompt.txt"
);

#[test]
fn rendered_prompt_matches_the_committed_golden() {
    let rendered = build_gap_fill_prompt(&canned_request());

    if std::env::var("BLESS_PROMPTS").is_ok() {
        let dir = std::path::Path::new(GOLDEN_PROMPT_PATH).parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(GOLDEN_PROMPT_PATH, &rendered).unwrap();
        eprintln!("blessed golden prompt at {GOLDEN_PROMPT_PATH}");
        return;
    }

    let golden = std::fs::read_to_string(GOLDEN_PROMPT_PATH).unwrap_or_else(|_| {
        panic!(
            "missing golden prompt; generate it with \
             BLESS_PROMPTS=1 cargo test -p bifrost-llm --test prompt_eval"
        )
    });
    assert_eq!(
        rendered, golden,
        "the {GAP_FILL_PROMPT_ID} prompt changed. If this is intentional, bump the \
         prompt version and re-bless with BLESS_PROMPTS=1."
    );
}

#[test]
fn prompt_template_enforces_the_non_negotiables() {
    let p = build_gap_fill_prompt(&canned_request());

    // Grounded: every placeholder is substituted and every grounding input present.
    // (Note: `${{ secrets.NAME }}` legitimately contains `{{`, so check the named
    // placeholder tokens specifically rather than any `{{`.)
    for placeholder in [
        "{{source_snippet}}",
        "{{converted_yaml}}",
        "{{importer_message}}",
        "{{repo_context}}",
    ] {
        assert!(
            !p.contains(placeholder),
            "unsubstituted placeholder {placeholder}"
        );
    }
    assert!(!p.contains("{#"), "template comment not stripped");
    assert!(
        p.contains("DownloadSecureFile@1"),
        "source snippet embedded"
    );
    assert!(
        p.contains("actions/checkout@v4"),
        "converted output embedded"
    );
    assert!(
        p.contains("secure file download has no built-in"),
        "importer failure embedded"
    );
    assert!(p.contains("languages: dotnet"), "repo context embedded");

    // Does-not-convert-from-scratch instruction.
    let lower = p.to_lowercase();
    assert!(
        lower.contains("do not convert the pipeline from scratch")
            || lower.contains("fill **only that gap**")
            || lower.contains("fill only that gap"),
        "prompt must constrain the model to the single gap"
    );

    // The LLM explains; it does not score.
    assert!(
        lower.contains("not output a numeric risk score"),
        "prompt must forbid a numeric risk score"
    );

    // JSON-only structured response is demanded.
    assert!(
        lower.contains("respond only with json"),
        "demands JSON output"
    );
    for key in [
        "proposed_yaml",
        "rationale",
        "risk_flags",
        "verify_steps",
        "confidence",
    ] {
        assert!(p.contains(key), "prompt's JSON spec is missing `{key}`");
    }

    // Secret-safety guidance.
    assert!(
        p.contains("secrets.NAME") || lower.contains("never include secret values"),
        "prompt must instruct secret-safe references"
    );
}

/// Every recorded model output, by label. These are the messy shapes real models
/// emit; all must satisfy the structured contract.
fn recorded_outputs() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "clean-snake",
            include_str!("../../../fixtures/prompts/recorded/clean-snake.txt"),
        ),
        (
            "fenced-camel-prose",
            include_str!("../../../fixtures/prompts/recorded/fenced-camel-prose.txt"),
        ),
        (
            "object-yaml-with-stray-score",
            include_str!("../../../fixtures/prompts/recorded/object-yaml-with-stray-score.txt"),
        ),
    ]
}

#[test]
fn recorded_outputs_satisfy_the_structured_contract() {
    for (label, raw) in recorded_outputs() {
        let resp = parse_gap_fill(raw)
            .unwrap_or_else(|e| panic!("[{label}] recorded output failed the contract: {e}"));

        // A reviewable answer: non-empty YAML + rationale, valid confidence.
        assert!(
            !resp.proposed_yaml.trim().is_empty(),
            "[{label}] empty proposed_yaml"
        );
        assert!(
            !resp.rationale.trim().is_empty(),
            "[{label}] empty rationale"
        );
        assert!(
            (0.0..=1.0).contains(&resp.confidence),
            "[{label}] confidence {} out of range",
            resp.confidence
        );

        // The LLM does not score: even when a model emits a stray `risk_score` /
        // `rating`, the parsed contract drops it — no scoring field survives.
        let serialized = serde_json::to_value(&resp).unwrap();
        let obj = serialized.as_object().unwrap();
        for forbidden in ["risk_score", "riskScore", "rating", "score"] {
            assert!(
                !obj.contains_key(forbidden),
                "[{label}] response leaked a scoring field `{forbidden}`"
            );
        }
    }
}

#[test]
fn stray_model_score_is_dropped_not_propagated() {
    // The object-yaml fixture deliberately carries `risk_score: 42` and
    // `rating: amber`. Prove the structured response is score-free regardless.
    let raw = include_str!("../../../fixtures/prompts/recorded/object-yaml-with-stray-score.txt");
    let resp = parse_gap_fill(raw).expect("parses despite stray score fields");
    let serialized = serde_json::to_string(&resp).unwrap();
    assert!(!serialized.contains("42"), "stray numeric score propagated");
    assert!(
        !serialized.to_lowercase().contains("amber"),
        "stray rating propagated"
    );
    // The object proposed_yaml is coerced to a reviewable string.
    assert!(resp.proposed_yaml.contains("dorny/test-reporter@v1"));
}
