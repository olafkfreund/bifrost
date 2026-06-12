//! Importer/ADO fixture harness (#17).
//!
//! One place to load the captured `audit_summary.md`, dry-run YAML/logs, the
//! converted workflow, and the ADO REST JSON under `/fixtures` — so tests across
//! the crate reference fixtures by name instead of repeating brittle
//! `include_str!("../../../fixtures/…")` paths. Re-capture a fixture and every
//! test that uses it picks up the new data.
//!
//! Test-only: gated behind `#[cfg(test)]` so nothing ships in the binary.

/// The captured Importer/ADO fixtures, embedded at compile time.
pub mod fixtures {
    pub const AUDIT_SUMMARY: &str = include_str!("../../../fixtures/audit_summary.md");
    pub const DRY_RUN_LOG: &str = include_str!("../../../fixtures/dry_run.log");
    pub const DRY_RUN_CONVERTED_YAML: &str =
        include_str!("../../../fixtures/dry_run_converted.yml");
    pub const SOURCE_PIPELINE_YAML: &str = include_str!("../../../fixtures/source_pipeline.yml");
    pub const CONVERTED_WORKFLOW_YAML: &str =
        include_str!("../../../fixtures/importer_converted_workflow.yml");
    pub const FORECAST_REPORT: &str = include_str!("../../../fixtures/forecast_report.md");

    pub const ADO_PROJECTS_JSON: &str = include_str!("../../../fixtures/ado/projects.json");
    pub const ADO_DEFINITION_JSON: &str = include_str!("../../../fixtures/ado/definition.json");
    pub const ADO_SERVICE_ENDPOINTS_JSON: &str =
        include_str!("../../../fixtures/ado/serviceendpoints.json");
    pub const ADO_VARIABLE_GROUPS_JSON: &str =
        include_str!("../../../fixtures/ado/variablegroups.json");
}

/// Parse an embedded ADO JSON fixture into a `serde_json::Value`.
pub fn ado_json(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).expect("fixture is valid JSON")
}

#[cfg(test)]
mod tests {
    use super::ado_json;
    use super::fixtures::*;
    use crate::azure_devops::{parse_projects, parse_service_connections, parse_variable_groups};
    use crate::importer::{
        parse_audit_summary, parse_converted_workflow, parse_dry_run, parse_forecast,
    };

    /// The harness loads every fixture and each feeds its parser without panic —
    /// the reproducibility guarantee the parser issues rely on (#17 AC).
    #[test]
    fn every_fixture_loads_and_parses() {
        // Importer outputs.
        let summary = parse_audit_summary(AUDIT_SUMMARY);
        assert!(
            summary.pipelines.total > 0,
            "audit_summary fixture should describe pipelines"
        );
        let dry = parse_dry_run(DRY_RUN_LOG);
        assert!(
            !dry.pipeline_id.is_empty(),
            "dry-run fixture should carry a pipeline id"
        );
        assert!(
            !parse_converted_workflow(CONVERTED_WORKFLOW_YAML).is_empty(),
            "converted-workflow fixture should surface gaps"
        );
        let forecast = parse_forecast(FORECAST_REPORT);
        assert!(
            forecast.total_minutes > 0 && !forecast.per_pipeline.is_empty(),
            "forecast fixture should yield a total + per-pipeline estimates"
        );
        // Capacity (#248): the Total section's Execution/Queue/Concurrent sub-sections.
        let cap = forecast
            .capacity
            .expect("forecast fixture should yield capacity");
        assert_eq!(cap.peak_concurrency, 9); // Concurrent jobs Max
        assert_eq!(cap.p50_job_minutes, 4.5); // Execution Median
        assert_eq!(cap.p90_job_minutes, 12.0); // Execution P90
        assert_eq!(cap.max_job_minutes, 38.0); // Execution Max
        assert_eq!(cap.median_queue_minutes, 0.8); // Queue Median
        assert!(!SOURCE_PIPELINE_YAML.is_empty());
        assert!(!DRY_RUN_CONVERTED_YAML.is_empty());

        // ADO REST captures.
        assert!(!parse_projects(&ado_json(ADO_PROJECTS_JSON)).is_empty());
        let def = ado_json(ADO_DEFINITION_JSON);
        assert!(def.is_object(), "definition fixture is a JSON object");
        // Connections/variable groups record names only (never secret values).
        let _ = parse_service_connections(&ado_json(ADO_SERVICE_ENDPOINTS_JSON), "proj");
        let _ = parse_variable_groups(&ado_json(ADO_VARIABLE_GROUPS_JSON), "proj");
    }
}
