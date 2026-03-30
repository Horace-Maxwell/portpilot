use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    Repo,
    Compose,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Node,
    Python,
    Rust,
    Go,
    Compose,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectProfileKind {
    WebApp,
    AiUi,
    GatewayStack,
    ComposeStack,
    FullstackMixed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteStrategy {
    GatewayPath,
    LocalhostDirect,
    ComposeService,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Stopped,
    Starting,
    Running,
    Unhealthy,
    PortConflict,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Install,
    Run,
    Stop,
    Restart,
    Build,
    Deploy,
    Open,
    Logs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionSource {
    Inferred,
    UserDefined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvFieldType {
    Text,
    Secret,
    Boolean,
    Multiline,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Running,
    Success,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchItemStatus {
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Installing,
    Starting,
    WaitingForPort,
    WaitingForService,
    Healthy,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProfile {
    #[serde(default)]
    pub kind: ProjectProfileKind,
    pub preferred_entrypoint: Option<String>,
    #[serde(default)]
    pub required_services: Vec<String>,
    #[serde(default)]
    pub required_env_groups: Vec<String>,
    #[serde(default)]
    pub known_ports: Vec<u16>,
    pub route_strategy: Option<RouteStrategy>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedRepo {
    pub name: String,
    pub root_path: String,
    pub git_url: Option<String>,
    pub project_kind: ProjectKind,
    pub runtime_kind: RuntimeKind,
    pub suggested_port: Option<u16>,
    pub has_env_template: bool,
    pub has_docker_compose: bool,
    pub has_dockerfile: bool,
    pub detected_files: Vec<String>,
    pub action_count: usize,
    #[serde(default)]
    pub workspace_target_count: usize,
    #[serde(default)]
    pub readme_hints: Vec<String>,
    #[serde(default)]
    pub project_profile: ProjectProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvTemplateField {
    pub key: String,
    pub default_value: Option<String>,
    pub description: Option<String>,
    pub field_type: EnvFieldType,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvProfile {
    pub values: HashMap<String, String>,
    pub raw_editor_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectAction {
    pub id: String,
    pub label: String,
    pub kind: ActionKind,
    pub command: String,
    pub workdir: String,
    pub env_profile: Option<String>,
    pub port_hint: Option<u16>,
    pub healthcheck_url: Option<String>,
    pub source: ActionSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedProject {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub root_path: String,
    pub git_url: Option<String>,
    pub project_kind: ProjectKind,
    pub runtime_kind: RuntimeKind,
    pub status: RuntimeStatus,
    pub last_error: Option<String>,
    pub preferred_port: Option<u16>,
    pub resolved_port: Option<u16>,
    pub route_subdomain_url: String,
    pub route_path_url: String,
    pub has_docker_compose: bool,
    pub has_dockerfile: bool,
    pub detected_files: Vec<String>,
    #[serde(default)]
    pub primary_target_id: Option<String>,
    #[serde(default)]
    pub workspace_targets: Vec<DetectedAppTarget>,
    #[serde(default)]
    pub readme_hints: Vec<String>,
    #[serde(default)]
    pub project_profile: ProjectProfile,
    pub env_template: Vec<EnvTemplateField>,
    pub env_profile: EnvProfile,
    pub actions: Vec<ProjectAction>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedAppTarget {
    pub id: String,
    pub name: String,
    pub relative_path: String,
    pub root_path: String,
    pub runtime_kind: RuntimeKind,
    pub suggested_port: Option<u16>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub available_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    Ok,
    Warn,
    Error,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub id: String,
    pub label: String,
    pub status: DoctorStatus,
    pub summary: String,
    pub detail: Option<String>,
    pub fix_label: Option<String>,
    pub fix_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorBlocker {
    pub id: String,
    pub label: String,
    pub summary: String,
    pub fix_label: Option<String>,
    pub fix_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorPortConflict {
    pub port: u16,
    pub occupied: bool,
    pub can_auto_reassign: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeRequirement {
    pub kind: String,
    pub name: String,
    pub ready: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub project_id: String,
    pub generated_at: String,
    #[serde(default)]
    pub missing_env_keys: Vec<String>,
    pub install_action_id: Option<String>,
    pub run_action_id: Option<String>,
    pub open_action_id: Option<String>,
    #[serde(default)]
    pub recommended_next_step: Option<String>,
    #[serde(default)]
    pub blockers: Vec<DoctorBlocker>,
    #[serde(default)]
    pub port_conflicts: Vec<DoctorPortConflict>,
    #[serde(default)]
    pub compose_requirements: Vec<ComposeRequirement>,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSessionProject {
    pub project_id: String,
    pub project_name: String,
    pub auto_start: bool,
    pub run_action_id: Option<String>,
    pub env_profile_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSession {
    pub id: String,
    pub name: String,
    pub projects: Vec<WorkspaceSessionProject>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchActionItemResult {
    pub project_id: String,
    pub project_name: String,
    pub status: BatchItemStatus,
    pub message: String,
    pub execution_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchActionResult {
    pub kind: String,
    pub total: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub skipped_count: usize,
    pub items: Vec<BatchActionItemResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionExecution {
    pub id: String,
    pub project_id: String,
    pub action_id: String,
    pub label: String,
    pub command: String,
    pub status: ExecutionStatus,
    pub pid: Option<u32>,
    pub port_hint: Option<u16>,
    pub resolved_port: Option<u16>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub last_log: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthProbeResult {
    pub url: Option<String>,
    pub ready: bool,
    pub last_checked_at: Option<String>,
    pub summary: Option<String>,
    pub readiness_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComposeServiceStatus {
    pub name: String,
    pub state: Option<String>,
    pub health: Option<String>,
    pub container_name: Option<String>,
    #[serde(default)]
    pub published_ports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeNode {
    pub project_id: String,
    pub project_name: String,
    #[serde(default)]
    pub kind: ProjectProfileKind,
    pub runtime_kind: RuntimeKind,
    pub status: RuntimeStatus,
    pub execution_id: Option<String>,
    pub execution_label: Option<String>,
    pub execution_status: Option<ExecutionStatus>,
    pub run_phase: Option<RunPhase>,
    pub route_url: String,
    pub port: Option<u16>,
    pub last_log: Option<String>,
    pub health: Option<HealthProbeResult>,
    #[serde(default)]
    pub services: Vec<ComposeServiceStatus>,
    #[serde(default)]
    pub dependencies_ready: bool,
    pub recommended_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalServicePreset {
    pub name: String,
    pub label: String,
    pub port: Option<u16>,
    pub ready: bool,
    pub hint: Option<String>,
    pub start_command: Option<String>,
    pub managed: bool,
    pub management_kind: Option<String>,
    #[serde(default)]
    pub used_by_projects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvGroupPreset {
    pub id: String,
    pub label: String,
    pub description: String,
    #[serde(default)]
    pub values: HashMap<String, String>,
    #[serde(default)]
    pub manual_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecipeTarget {
    pub id: String,
    pub relative_path: String,
    pub runtime_kind: Option<RuntimeKind>,
    pub priority: Option<i32>,
    pub suggested_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecipe {
    #[serde(default = "default_recipe_version")]
    pub version: u32,
    pub project_name: Option<String>,
    pub primary_target_id: Option<String>,
    pub preferred_port: Option<u16>,
    pub install_action_id: Option<String>,
    pub run_action_id: Option<String>,
    pub open_action_id: Option<String>,
    #[serde(default)]
    pub readme_hints: Vec<String>,
    #[serde(default)]
    pub env_keys: Vec<String>,
    #[serde(default)]
    pub kind: Option<ProjectProfileKind>,
    #[serde(default)]
    pub preferred_entrypoint: Option<String>,
    #[serde(default)]
    pub required_services: Vec<String>,
    #[serde(default)]
    pub required_env_groups: Vec<String>,
    #[serde(default)]
    pub known_ports: Vec<u16>,
    #[serde(default)]
    pub route_strategy: Option<RouteStrategy>,
    #[serde(default)]
    pub targets: Vec<ProjectRecipeTarget>,
}

fn default_recipe_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub execution_id: String,
    pub stream: String,
    pub message: String,
    pub timestamp: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectHealth {
    pub project_id: String,
    pub url: Option<String>,
    pub ok: bool,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortLease {
    pub project_id: String,
    pub project_name: String,
    pub action_id: String,
    pub action_label: String,
    pub port: u16,
    pub pid: Option<u32>,
    pub status: RuntimeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteBinding {
    pub project_id: String,
    pub project_name: String,
    pub slug: String,
    pub target_port: Option<u16>,
    pub subdomain_url: String,
    pub path_url: String,
    pub status: RuntimeStatus,
}
