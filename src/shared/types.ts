export type ProjectKind = "repo" | "compose";
export type RuntimeKind = "node" | "python" | "rust" | "go" | "compose" | "unknown";
export type RuntimeStatus =
  | "stopped"
  | "starting"
  | "running"
  | "unhealthy"
  | "port_conflict"
  | "error";
export type ActionKind =
  | "install"
  | "run"
  | "stop"
  | "restart"
  | "build"
  | "deploy"
  | "open"
  | "logs";
export type ActionSource = "inferred" | "user_defined";
export type EnvFieldType = "text" | "secret" | "boolean" | "multiline";
export type ExecutionStatus = "running" | "success" | "failed" | "stopped";
export type DoctorStatus = "ok" | "warn" | "error" | "info";
export type BatchItemStatus = "success" | "failed" | "skipped";
export type RunPhase =
  | "installing"
  | "starting"
  | "waiting_for_port"
  | "healthy"
  | "failed"
  | "stopped";

export interface ImportedRepo {
  name: string;
  root_path: string;
  git_url: string | null;
  project_kind: ProjectKind;
  runtime_kind: RuntimeKind;
  suggested_port: number | null;
  has_env_template: boolean;
  has_docker_compose: boolean;
  has_dockerfile: boolean;
  detected_files: string[];
  action_count: number;
  workspace_target_count: number;
  readme_hints: string[];
}

export interface EnvTemplateField {
  key: string;
  default_value: string | null;
  description: string | null;
  field_type: EnvFieldType;
}

export interface EnvProfile {
  values: Record<string, string>;
  raw_editor_text: string | null;
}

export interface ProjectAction {
  id: string;
  label: string;
  kind: ActionKind;
  command: string;
  workdir: string;
  env_profile: string | null;
  port_hint: number | null;
  healthcheck_url: string | null;
  source: ActionSource;
}

export interface ManagedProject {
  id: string;
  name: string;
  slug: string;
  root_path: string;
  git_url: string | null;
  project_kind: ProjectKind;
  runtime_kind: RuntimeKind;
  status: RuntimeStatus;
  last_error: string | null;
  preferred_port: number | null;
  resolved_port: number | null;
  route_subdomain_url: string;
  route_path_url: string;
  has_docker_compose: boolean;
  has_dockerfile: boolean;
  detected_files: string[];
  primary_target_id: string | null;
  workspace_targets: DetectedAppTarget[];
  readme_hints: string[];
  env_template: EnvTemplateField[];
  env_profile: EnvProfile;
  actions: ProjectAction[];
  created_at: string;
  updated_at: string;
}

export interface DetectedAppTarget {
  id: string;
  name: string;
  relative_path: string;
  root_path: string;
  runtime_kind: RuntimeKind;
  suggested_port: number | null;
  priority: number;
  available_actions: string[];
}

export interface DoctorCheck {
  id: string;
  label: string;
  status: DoctorStatus;
  summary: string;
  detail: string | null;
  fix_label: string | null;
  fix_command: string | null;
}

export interface DoctorReport {
  project_id: string;
  generated_at: string;
  missing_env_keys: string[];
  install_action_id: string | null;
  run_action_id: string | null;
  open_action_id: string | null;
  recommended_next_step: string | null;
  checks: DoctorCheck[];
}

export interface WorkspaceSessionProject {
  project_id: string;
  project_name: string;
  auto_start: boolean;
  run_action_id: string | null;
  env_profile_name: string | null;
}

export interface WorkspaceSession {
  id: string;
  name: string;
  projects: WorkspaceSessionProject[];
  created_at: string;
  updated_at: string;
}

export interface BatchActionItemResult {
  project_id: string;
  project_name: string;
  status: BatchItemStatus;
  message: string;
  execution_id: string | null;
}

export interface BatchActionResult {
  kind: string;
  total: number;
  success_count: number;
  failure_count: number;
  skipped_count: number;
  items: BatchActionItemResult[];
}

export interface ActionExecution {
  id: string;
  project_id: string;
  action_id: string;
  label: string;
  command: string;
  status: ExecutionStatus;
  pid: number | null;
  port_hint: number | null;
  resolved_port: number | null;
  started_at: string;
  finished_at: string | null;
  last_log: string | null;
}

export interface HealthProbeResult {
  url: string | null;
  ready: boolean;
  last_checked_at: string | null;
  summary: string | null;
}

export interface RuntimeNode {
  project_id: string;
  project_name: string;
  runtime_kind: RuntimeKind;
  status: RuntimeStatus;
  execution_id: string | null;
  execution_label: string | null;
  execution_status: ExecutionStatus | null;
  run_phase: RunPhase | null;
  route_url: string;
  port: number | null;
  last_log: string | null;
  health: HealthProbeResult | null;
  compose_services: string[];
}

export interface ProjectRecipeTarget {
  id: string;
  relative_path: string;
  runtime_kind: RuntimeKind | null;
  priority: number | null;
  suggested_port: number | null;
}

export interface ProjectRecipe {
  version: number;
  project_name: string | null;
  primary_target_id: string | null;
  preferred_port: number | null;
  install_action_id: string | null;
  run_action_id: string | null;
  open_action_id: string | null;
  readme_hints: string[];
  env_keys: string[];
  targets: ProjectRecipeTarget[];
}

export interface LogEntry {
  execution_id: string;
  stream: string;
  message: string;
  timestamp: string;
}

export interface PortLease {
  project_id: string;
  project_name: string;
  action_id: string;
  action_label: string;
  port: number;
  pid: number | null;
  status: RuntimeStatus;
}

export interface RouteBinding {
  project_id: string;
  project_name: string;
  slug: string;
  target_port: number | null;
  subdomain_url: string;
  path_url: string;
  status: RuntimeStatus;
}
