import { invoke } from "@tauri-apps/api/core";
import type {
  ActionExecution,
  BatchActionResult,
  DoctorReport,
  EnvTemplateField,
  ImportedRepo,
  LogEntry,
  ManagedProject,
  PortLease,
  ProjectAction,
  RouteBinding,
  RuntimeNode,
  WorkspaceSession,
} from "../shared/types";

export const api = {
  listWorkspaceRoots: () => invoke<string[]>("list_workspace_roots"),
  setWorkspaceRoots: (roots: string[]) => invoke<string[]>("set_workspace_roots", { roots }),
  listProjects: () => invoke<ManagedProject[]>("list_projects"),
  scanLocalProjects: (roots?: string[]) =>
    invoke<ImportedRepo[]>("scan_local_projects", { roots }),
  registerLocalProject: (path: string, gitUrl?: string | null) =>
    invoke<ManagedProject>("register_local_project", { path, gitUrl }),
  importRepoFromGit: (url: string, workspaceRoot?: string | null) =>
    invoke<ManagedProject>("import_repo_from_git", { url, workspaceRoot }),
  listProjectActions: (projectId: string) =>
    invoke<ProjectAction[]>("list_project_actions", { projectId }),
  getEnvTemplate: (projectId: string) =>
    invoke<EnvTemplateField[]>("get_env_template", { projectId }),
  getDoctorReport: (projectId: string) =>
    invoke<DoctorReport>("get_doctor_report", { projectId }),
  saveEnvProfile: (
    projectId: string,
    values: Record<string, string>,
    rawEditorText?: string | null,
  ) => invoke<ManagedProject>("save_env_profile", { projectId, values, rawEditorText }),
  listActionExecutions: () => invoke<ActionExecution[]>("list_action_executions"),
  listWorkspaceSessions: () => invoke<WorkspaceSession[]>("list_workspace_sessions"),
  saveWorkspaceSession: (
    name: string,
    projectIds: string[],
    runActionOverrides?: Record<string, string> | null,
  ) =>
    invoke<WorkspaceSession>("save_workspace_session", {
      name,
      projectIds,
      runActionOverrides,
    }),
  deleteWorkspaceSession: (sessionId: string) =>
    invoke<WorkspaceSession[]>("delete_workspace_session", { sessionId }),
  restoreWorkspaceSession: (sessionId: string) =>
    invoke<BatchActionResult>("restore_workspace_session", { sessionId }),
  runBatchAction: (projectIds: string[]) =>
    invoke<BatchActionResult>("run_batch_action", { projectIds }),
  listRuntimeNodes: () => invoke<RuntimeNode[]>("list_runtime_nodes"),
  stopProjects: (projectIds: string[]) =>
    invoke<BatchActionResult>("stop_projects", { projectIds }),
  restartProjects: (projectIds: string[]) =>
    invoke<BatchActionResult>("restart_projects", { projectIds }),
  getProjectLogs: (projectId?: string | null) =>
    invoke<LogEntry[]>("get_project_logs", { projectId }),
  listPorts: () => invoke<PortLease[]>("list_ports"),
  listRoutes: () => invoke<RouteBinding[]>("list_routes"),
  runProjectAction: (projectId: string, actionId: string) =>
    invoke<ActionExecution>("run_project_action", { projectId, actionId }),
  stopActionExecution: (executionId: string) =>
    invoke<ActionExecution | null>("stop_action_execution", { executionId }),
  restartProject: (projectId: string, actionId: string) =>
    invoke<ActionExecution>("restart_project", { projectId, actionId }),
};
