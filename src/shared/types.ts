export type ProjectKind = "repo" | "compose";
export type RuntimeKind = "node" | "python" | "rust" | "go" | "compose" | "unknown";
export type ProjectProfileKind =
  | "web_app"
  | "ai_ui"
  | "gateway_stack"
  | "compose_stack"
  | "fullstack_mixed"
  | "unknown";
export type RouteStrategy = "gateway_path" | "localhost_direct" | "compose_service" | "hybrid";
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
  | "waiting_for_service"
  | "healthy"
  | "failed"
  | "stopped";
export type LocalServiceStatus =
  | "ready"
  | "stopped"
  | "failed"
  | "unmanaged_already_running"
  | "unmanaged";
export type LocalHttpsCertificateState =
  | "trusted"
  | "needs_install"
  | "needs_trust"
  | "fallback_self_signed"
  | "error";

export interface ProjectProfile {
  kind: ProjectProfileKind;
  preferred_entrypoint: string | null;
  required_services: string[];
  required_env_groups: string[];
  known_ports: number[];
  route_strategy: RouteStrategy | null;
  summary: string | null;
}

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
  project_profile: ProjectProfile;
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
  project_profile: ProjectProfile;
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
  blockers: DoctorBlocker[];
  port_conflicts: DoctorPortConflict[];
  compose_requirements: ComposeRequirement[];
  service_requirements: ComposeRequirement[];
  checks: DoctorCheck[];
}

export interface DoctorBlocker {
  id: string;
  label: string;
  summary: string;
  fix_label: string | null;
  fix_command: string | null;
}

export interface DoctorPortConflict {
  port: number;
  occupied: boolean;
  can_auto_reassign: boolean;
  detail: string;
}

export interface ComposeRequirement {
  kind: string;
  name: string;
  ready: boolean;
  detail: string | null;
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
  readiness_reason: string | null;
}

export interface LocalUrl {
  kind: string;
  url: string;
  recommended: boolean;
}

export interface ComposeServiceStatus {
  name: string;
  state: string | null;
  health: string | null;
  container_name: string | null;
  published_ports: string[];
}

export interface RuntimeNode {
  project_id: string;
  project_name: string;
  kind: ProjectProfileKind;
  runtime_kind: RuntimeKind;
  status: RuntimeStatus;
  execution_id: string | null;
  execution_label: string | null;
  execution_status: ExecutionStatus | null;
  run_phase: RunPhase | null;
  route_url: string;
  port: number | null;
  local_urls: LocalUrl[];
  last_log: string | null;
  health: HealthProbeResult | null;
  services: ComposeServiceStatus[];
  dependencies_ready: boolean;
  recommended_action: string | null;
}

export interface LocalServicePreset {
  name: string;
  label: string;
  port: number | null;
  ready: boolean;
  auto_started: boolean;
  status: LocalServiceStatus;
  ready_detail: string | null;
  hint: string | null;
  setup_command: string | null;
  start_command: string | null;
  stop_command: string | null;
  managed: boolean;
  management_kind: string | null;
  used_by_projects: string[];
}

export interface LocalHttpsStatus {
  enabled: boolean;
  http_port: number;
  https_port: number | null;
  provider: string | null;
  certificate_state: LocalHttpsCertificateState;
  restart_required: boolean;
  detail: string | null;
}

export interface EnvGroupPreset {
  id: string;
  label: string;
  description: string;
  values: Record<string, string>;
  manual_keys: string[];
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
  kind: ProjectProfileKind | null;
  preferred_entrypoint: string | null;
  required_services: string[];
  required_env_groups: string[];
  known_ports: number[];
  route_strategy: RouteStrategy | null;
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
