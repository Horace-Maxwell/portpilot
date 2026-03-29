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
  env_template: EnvTemplateField[];
  env_profile: EnvProfile;
  actions: ProjectAction[];
  created_at: string;
  updated_at: string;
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
