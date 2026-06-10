//! Types describing what a `SourceAdapter` discovers in a CI source platform.
//!
//! These are platform-agnostic; the Azure DevOps adapter is the first producer.
//! Note the secrecy boundary: variable and service-connection **names** are data
//! and are modelled here, but secret **values** are never fetched or stored.

use serde::{Deserialize, Serialize};

use crate::model::Classification;

/// A project / team-project grouping pipelines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
}

/// A pipeline as discovered in the source platform (pre-conversion).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePipeline {
    pub id: String,
    pub name: String,
    pub project: String,
    pub classification: Classification,
    /// Repository the pipeline builds, if the platform exposes it.
    pub repository: Option<String>,
}

/// A fetched pipeline definition. Classic/designer pipelines have no YAML source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineDefinition {
    pub id: String,
    pub classification: Classification,
    /// The YAML source for YAML pipelines; `None` for classic/designer ones.
    pub yaml: Option<String>,
}

/// A service connection — drives the OIDC-federation risk factor.
/// Recorded by name and type only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConnection {
    pub id: String,
    pub name: String,
    /// e.g. "azurerm", "github", "dockerregistry".
    pub kind: String,
    pub project: String,
}

/// A variable group. Variable **names** are recorded; secret values never are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableGroup {
    pub id: String,
    pub name: String,
    pub project: String,
    pub variables: Vec<VariableRef>,
}

/// A single variable reference — its name and whether it is secret-flagged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableRef {
    pub name: String,
    pub is_secret: bool,
}

/// How a task is sourced — feeds the unsupported-task risk factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    BuiltIn,
    Marketplace,
    Custom,
}

/// Org-wide usage of a single task/extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskUsage {
    /// Task identifier, e.g. "PublishBuildArtifacts@1".
    pub task: String,
    pub kind: TaskKind,
    /// How many pipelines use it across the org.
    pub count: u32,
}
