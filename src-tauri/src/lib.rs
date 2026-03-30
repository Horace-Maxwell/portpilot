mod core {
    pub mod inference;
    pub mod models;
}
mod gateway;
mod runtime {
    pub mod manager;
}
mod storage {
    pub mod store;
}

use std::collections::HashMap;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::core::inference::{
    find_compose_file, infer_project_from_path, now_iso, parse_env_template,
    repo_name_from_git_url, scan_workspace_roots, slugify, DEFAULT_WORKSPACE_ROOT,
};
use crate::core::models::{
    ActionExecution, ActionKind, BatchActionItemResult, BatchActionResult, BatchItemStatus,
    ComposeRequirement, ComposeServiceStatus, DoctorBlocker, DoctorCheck, DoctorPortConflict,
    DoctorReport, DoctorStatus, EnvGroupPreset, EnvProfile, EnvTemplateField, HealthProbeResult,
    ImportedRepo, LocalHttpsCertificateState, LocalHttpsStatus, LocalServicePreset,
    LocalServiceStatus, LocalUrl, LogEntry, ManagedProject, PortLease, ProjectAction,
    ProjectRecipe, ProjectRecipeTarget, RouteBinding, RunPhase, RuntimeKind, RuntimeNode,
    RuntimeStatus, WorkspaceSession, WorkspaceSessionProject,
};
use crate::runtime::manager::RuntimeManager;
use crate::storage::store::ProjectStore;

#[derive(Clone)]
struct AppState {
    store: Arc<ProjectStore>,
    runtime: Arc<RuntimeManager>,
    gateway_port: Arc<Mutex<u16>>,
    local_https_status: Arc<Mutex<LocalHttpsStatus>>,
    data_dir: PathBuf,
}

#[tauri::command]
fn list_workspace_roots(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let roots = state.store.list_workspace_roots()?;
    if roots.is_empty() {
        return Ok(vec![DEFAULT_WORKSPACE_ROOT.to_string()]);
    }
    Ok(roots)
}

#[tauri::command]
fn set_workspace_roots(
    state: State<'_, AppState>,
    roots: Vec<String>,
) -> Result<Vec<String>, String> {
    let normalized = if roots.is_empty() {
        vec![DEFAULT_WORKSPACE_ROOT.to_string()]
    } else {
        roots
    };
    state.store.replace_workspace_roots(&normalized)?;
    Ok(normalized)
}

#[tauri::command]
fn list_projects(state: State<'_, AppState>) -> Result<Vec<ManagedProject>, String> {
    let gateway_port = *state.gateway_port.lock();
    let mut projects = Vec::new();
    for project in state.store.list()? {
        let refreshed = refresh_project_metadata(&state.store, project, gateway_port)?;
        projects.push(refreshed);
    }
    projects.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(projects)
}

#[tauri::command]
fn scan_local_projects(
    state: State<'_, AppState>,
    roots: Option<Vec<String>>,
) -> Result<Vec<ImportedRepo>, String> {
    let roots = roots.unwrap_or_else(|| {
        state
            .store
            .list_workspace_roots()
            .unwrap_or_else(|_| vec![DEFAULT_WORKSPACE_ROOT.to_string()])
    });
    Ok(scan_workspace_roots(&roots, *state.gateway_port.lock()))
}

#[tauri::command]
fn register_local_project(
    state: State<'_, AppState>,
    path: String,
    git_url: Option<String>,
) -> Result<ManagedProject, String> {
    let gateway_port = *state.gateway_port.lock();
    let project = infer_project_from_path(Path::new(&path), git_url, gateway_port)
        .ok_or_else(|| "Could not infer a managed project from that path.".to_string())?;
    state.store.upsert(project.clone())?;
    Ok(project)
}

#[tauri::command]
fn list_project_actions(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<ProjectAction>, String> {
    let project = fresh_project(&state.store, &project_id, *state.gateway_port.lock())?;
    Ok(project.actions)
}

#[tauri::command]
fn get_env_template(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<EnvTemplateField>, String> {
    let project = fresh_project(&state.store, &project_id, *state.gateway_port.lock())?;
    if !project.env_template.is_empty() {
        return Ok(project.env_template);
    }
    Ok(parse_env_template(Path::new(&project.root_path)))
}

#[tauri::command]
fn get_doctor_report(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<DoctorReport, String> {
    let project = fresh_project(&state.store, &project_id, *state.gateway_port.lock())?;
    let https_status = state.local_https_status.lock().clone();
    Ok(build_doctor_report(&project, &https_status))
}

#[tauri::command]
fn get_project_recipe(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ProjectRecipe, String> {
    let project = fresh_project(&state.store, &project_id, *state.gateway_port.lock())?;
    Ok(build_project_recipe(&project))
}

#[tauri::command]
fn write_project_recipe(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ManagedProject, String> {
    let gateway_port = *state.gateway_port.lock();
    let project = state
        .store
        .get(&project_id)?
        .ok_or_else(|| "Project not found.".to_string())?;
    let latest = fresh_project(&state.store, &project_id, gateway_port)?;
    let recipe = build_project_recipe(&latest);
    let path = Path::new(&latest.root_path).join(".portpilot.json");
    let contents = serde_json::to_string_pretty(&recipe).map_err(|error| error.to_string())?;
    fs::write(path, contents).map_err(|error| error.to_string())?;
    refresh_project_metadata(&state.store, project, gateway_port)
}

#[tauri::command]
fn save_env_profile(
    state: State<'_, AppState>,
    project_id: String,
    values: HashMap<String, String>,
    raw_editor_text: Option<String>,
) -> Result<ManagedProject, String> {
    let project = state
        .store
        .get(&project_id)?
        .ok_or_else(|| "Project not found.".to_string())?;

    let env_path = Path::new(&project.root_path).join(".env");
    let profile = EnvProfile {
        values,
        raw_editor_text,
    };
    let content = render_env_file(&project.env_template, &profile);
    fs::write(&env_path, content).map_err(|error| error.to_string())?;

    state
        .store
        .update(&project_id, |item| {
            item.env_profile = profile.clone();
            item.updated_at = now_iso();
        })?
        .ok_or_else(|| "Project disappeared while saving the environment.".to_string())
}

#[tauri::command]
fn list_workspace_sessions(state: State<'_, AppState>) -> Result<Vec<WorkspaceSession>, String> {
    state.store.list_sessions()
}

#[tauri::command]
fn save_workspace_session(
    state: State<'_, AppState>,
    name: String,
    project_ids: Vec<String>,
    run_action_overrides: Option<HashMap<String, String>>,
) -> Result<WorkspaceSession, String> {
    let run_action_overrides = run_action_overrides.unwrap_or_default();
    let projects = state.store.list()?;
    let selected = projects
        .into_iter()
        .filter(|project| project_ids.iter().any(|id| id == &project.id))
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return Err("Select at least one managed project before saving a session.".to_string());
    }

    let session_projects = selected
        .into_iter()
        .map(|project| WorkspaceSessionProject {
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            auto_start: true,
            run_action_id: run_action_overrides
                .get(&project.id)
                .cloned()
                .or_else(|| primary_run_action(&project).map(|action| action.id.clone())),
            env_profile_name: Some("default".to_string()),
        })
        .collect::<Vec<_>>();

    let timestamp = now_iso();
    let session = WorkspaceSession {
        id: Uuid::new_v4().to_string(),
        name: if name.trim().is_empty() {
            format!("Workspace {}", timestamp)
        } else {
            name.trim().to_string()
        },
        projects: session_projects,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    };
    state.store.upsert_session(&session)?;
    Ok(session)
}

#[tauri::command]
fn delete_workspace_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<WorkspaceSession>, String> {
    state.store.delete_session(&session_id)?;
    state.store.list_sessions()
}

#[tauri::command]
fn list_action_executions(state: State<'_, AppState>) -> Result<Vec<ActionExecution>, String> {
    Ok(state.runtime.list_executions())
}

#[tauri::command]
fn list_runtime_nodes(state: State<'_, AppState>) -> Result<Vec<RuntimeNode>, String> {
    let projects = state.store.list()?;
    let executions = state.runtime.list_executions();
    let logs = state.runtime.list_logs(None);
    let https_status = state.local_https_status.lock().clone();
    let gateway_port = *state.gateway_port.lock();

    Ok(projects
        .into_iter()
        .map(|project| build_runtime_node(&project, &executions, &logs, &https_status, gateway_port))
        .collect())
}

#[tauri::command]
fn get_local_https_status(state: State<'_, AppState>) -> Result<LocalHttpsStatus, String> {
    Ok(state.local_https_status.lock().clone())
}

#[tauri::command]
fn refresh_local_https_status(state: State<'_, AppState>) -> Result<LocalHttpsStatus, String> {
    let current = state.local_https_status.lock().clone();
    let refreshed = gateway::refresh_local_https_status(&state.data_dir, &current)?;
    *state.local_https_status.lock() = refreshed.clone();
    Ok(refreshed)
}

#[tauri::command]
fn install_local_https(state: State<'_, AppState>) -> Result<LocalHttpsStatus, String> {
    let current = state.local_https_status.lock().clone();
    let refreshed = gateway::install_local_https(&state.data_dir, &current)?;
    *state.local_https_status.lock() = refreshed.clone();
    Ok(refreshed)
}

#[tauri::command]
fn list_local_service_presets(state: State<'_, AppState>) -> Result<Vec<LocalServicePreset>, String> {
    let projects = state.store.list()?;
    Ok(collect_local_service_presets(&projects))
}

#[tauri::command]
fn inspect_local_service(
    state: State<'_, AppState>,
    service_name: String,
) -> Result<LocalServicePreset, String> {
    let projects = state.store.list()?;
    collect_local_service_presets(&projects)
        .into_iter()
        .find(|preset| preset.name == service_name.to_ascii_lowercase())
        .ok_or_else(|| "Service preset not found".to_string())
}

#[tauri::command]
fn start_local_service(
    state: State<'_, AppState>,
    service_name: String,
) -> Result<LocalServicePreset, String> {
    ensure_local_service_running(&service_name)?;
    let projects = state.store.list()?;
    collect_local_service_presets(&projects)
        .into_iter()
        .find(|preset| preset.name == service_name.to_ascii_lowercase())
        .ok_or_else(|| "Service preset not found after start".to_string())
}

#[tauri::command]
fn restart_local_service(
    state: State<'_, AppState>,
    service_name: String,
) -> Result<LocalServicePreset, String> {
    if matches!(
        local_service_status(&service_name),
        LocalServiceStatus::UnmanagedAlreadyRunning
    ) {
        return Err(format!(
            "{service_name} is already running outside PortPilot, so it cannot be restarted from here."
        ));
    }
    let _ = ensure_local_service_stopped(&service_name);
    ensure_local_service_running(&service_name)?;
    let projects = state.store.list()?;
    collect_local_service_presets(&projects)
        .into_iter()
        .find(|preset| preset.name == service_name.to_ascii_lowercase())
        .ok_or_else(|| "Service preset not found after restart".to_string())
}

#[tauri::command]
fn stop_local_service(
    state: State<'_, AppState>,
    service_name: String,
) -> Result<LocalServicePreset, String> {
    ensure_local_service_stopped(&service_name)?;
    let projects = state.store.list()?;
    collect_local_service_presets(&projects)
        .into_iter()
        .find(|preset| preset.name == service_name.to_ascii_lowercase())
        .ok_or_else(|| "Service preset not found after stop".to_string())
}

#[tauri::command]
fn list_env_group_presets(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<EnvGroupPreset>, String> {
    let project = state
        .store
        .get(&project_id)?
        .ok_or_else(|| "Project not found".to_string())?;
    Ok(build_env_group_presets(&project))
}

#[tauri::command]
fn get_project_logs(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<Vec<LogEntry>, String> {
    Ok(state.runtime.list_logs(project_id.as_deref()))
}

#[tauri::command]
fn list_ports(state: State<'_, AppState>) -> Result<Vec<PortLease>, String> {
    let executions = state.runtime.list_executions();
    let mut leases = Vec::new();
    for project in state.store.list()? {
        if let Some(port) = project.resolved_port.or(project.preferred_port) {
            let current = executions.iter().find(|execution| {
                execution.project_id == project.id
                    && execution.status == crate::core::models::ExecutionStatus::Running
            });
            leases.push(PortLease {
                project_id: project.id.clone(),
                project_name: project.name.clone(),
                action_id: current
                    .map(|item| item.action_id.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                action_label: current
                    .map(|item| item.label.clone())
                    .unwrap_or_else(|| "Idle".to_string()),
                port,
                pid: current.and_then(|item| item.pid),
                status: project.status.clone(),
            });
        }
    }
    Ok(leases)
}

#[tauri::command]
fn run_batch_action(
    app: AppHandle,
    state: State<'_, AppState>,
    project_ids: Vec<String>,
) -> Result<BatchActionResult, String> {
    execute_batch_action(app, state, "run", project_ids)
}

#[tauri::command]
fn stop_projects(
    app: AppHandle,
    state: State<'_, AppState>,
    project_ids: Vec<String>,
) -> Result<BatchActionResult, String> {
    execute_batch_action(app, state, "stop", project_ids)
}

#[tauri::command]
fn restart_projects(
    app: AppHandle,
    state: State<'_, AppState>,
    project_ids: Vec<String>,
) -> Result<BatchActionResult, String> {
    execute_batch_action(app, state, "restart", project_ids)
}

#[tauri::command]
fn restore_workspace_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<BatchActionResult, String> {
    let session = state
        .store
        .get_session(&session_id)?
        .ok_or_else(|| "Workspace session not found.".to_string())?;

    let mut items = Vec::new();
    for session_project in &session.projects {
        let Some(project) = state.store.get(&session_project.project_id)? else {
            items.push(BatchActionItemResult {
                project_id: session_project.project_id.clone(),
                project_name: session_project.project_name.clone(),
                status: BatchItemStatus::Skipped,
                message: "Project is no longer registered in PortPilot.".to_string(),
                execution_id: None,
            });
            continue;
        };

        if !session_project.auto_start {
            items.push(BatchActionItemResult {
                project_id: project.id.clone(),
                project_name: project.name.clone(),
                status: BatchItemStatus::Skipped,
                message: "Session kept this project in a stopped state.".to_string(),
                execution_id: None,
            });
            continue;
        }

        let Some(action_id) = session_project
            .run_action_id
            .clone()
            .or_else(|| primary_run_action(&project).map(|action| action.id.clone()))
        else {
            items.push(BatchActionItemResult {
                project_id: project.id.clone(),
                project_name: project.name.clone(),
                status: BatchItemStatus::Skipped,
                message: "No primary run action is available for this project.".to_string(),
                execution_id: None,
            });
            continue;
        };

        let Some(action) = project
            .actions
            .iter()
            .find(|action| action.id == action_id)
            .cloned()
        else {
            items.push(BatchActionItemResult {
                project_id: project.id.clone(),
                project_name: project.name.clone(),
                status: BatchItemStatus::Skipped,
                message: "Saved run action is no longer available.".to_string(),
                execution_id: None,
            });
            continue;
        };

        match state.runtime.run_action(
            app.clone(),
            Arc::clone(&state.store),
            project.clone(),
            action,
        ) {
            Ok(execution) => items.push(BatchActionItemResult {
                project_id: project.id.clone(),
                project_name: project.name.clone(),
                status: BatchItemStatus::Success,
                message: format!("Started {}.", execution.label),
                execution_id: Some(execution.id),
            }),
            Err(error) => items.push(BatchActionItemResult {
                project_id: project.id.clone(),
                project_name: project.name.clone(),
                status: BatchItemStatus::Failed,
                message: error,
                execution_id: None,
            }),
        }
    }

    Ok(summarize_batch_result("restore_session", items))
}

#[tauri::command]
fn list_routes(state: State<'_, AppState>) -> Result<Vec<RouteBinding>, String> {
    Ok(state
        .store
        .list()?
        .into_iter()
        .map(|project| RouteBinding {
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            slug: project.slug.clone(),
            target_port: project.resolved_port.or(project.preferred_port),
            subdomain_url: project.route_subdomain_url.clone(),
            path_url: project.route_path_url.clone(),
            status: project.status.clone(),
        })
        .collect())
}

#[tauri::command]
fn stop_action_execution(
    app: AppHandle,
    state: State<'_, AppState>,
    execution_id: String,
) -> Result<Option<ActionExecution>, String> {
    state
        .runtime
        .stop_execution(app, Arc::clone(&state.store), &execution_id)
}

#[tauri::command]
fn restart_project(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    action_id: String,
) -> Result<ActionExecution, String> {
    let active = state
        .runtime
        .list_executions()
        .into_iter()
        .find(|execution| {
            execution.project_id == project_id
                && execution.status == crate::core::models::ExecutionStatus::Running
        });
    if let Some(active) = active {
        let _ = state
            .runtime
            .stop_execution(app.clone(), Arc::clone(&state.store), &active.id)?;
    }
    run_project_action(app, state, project_id, action_id)
}

#[tauri::command]
fn run_project_action(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    action_id: String,
) -> Result<ActionExecution, String> {
    let project = state
        .store
        .get(&project_id)?
        .ok_or_else(|| "Project not found.".to_string())?;
    let action = project
        .actions
        .iter()
        .find(|action| action.id == action_id)
        .cloned()
        .ok_or_else(|| "Action not found.".to_string())?;

    if matches!(action.kind, crate::core::models::ActionKind::Open) {
        return Err("Open actions should be handled by the UI.".to_string());
    }

    state
        .runtime
        .run_action(app, Arc::clone(&state.store), project, action)
}

#[tauri::command]
fn import_repo_from_git(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    workspace_root: Option<String>,
) -> Result<ManagedProject, String> {
    let roots = state.store.list_workspace_roots()?;
    let target_root = workspace_root
        .or_else(|| roots.first().cloned())
        .unwrap_or_else(|| DEFAULT_WORKSPACE_ROOT.to_string());

    fs::create_dir_all(&target_root).map_err(|error| error.to_string())?;
    let repo_name = repo_name_from_git_url(&url)?;
    let destination = unique_destination(Path::new(&target_root), &repo_name);
    app.emit(
        "repo-import-progress",
        serde_json::json!({ "stage": "cloning", "url": url, "destination": destination }),
    )
    .map_err(|error| error.to_string())?;

    let output = Command::new("git")
        .arg("clone")
        .arg(&url)
        .arg(&destination)
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    app.emit(
        "repo-import-progress",
        serde_json::json!({ "stage": "scanning", "destination": destination }),
    )
    .map_err(|error| error.to_string())?;

    let gateway_port = *state.gateway_port.lock();
    let project = infer_project_from_path(Path::new(&destination), Some(url), gateway_port)
        .ok_or_else(|| {
            "PortPilot cloned the repo, but could not infer a supported project.".to_string()
        })?;
    state.store.upsert(project.clone())?;

    app.emit(
        "repo-import-progress",
        serde_json::json!({ "stage": "finished", "projectId": project.id }),
    )
    .map_err(|error| error.to_string())?;

    Ok(project)
}

fn execute_batch_action(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: &str,
    project_ids: Vec<String>,
) -> Result<BatchActionResult, String> {
    if project_ids.is_empty() {
        return Err("Select at least one project first.".to_string());
    }

    let mut items = Vec::new();
    for project_id in project_ids {
        let Some(project) = state.store.get(&project_id)? else {
            items.push(BatchActionItemResult {
                project_id,
                project_name: "Unknown Project".to_string(),
                status: BatchItemStatus::Skipped,
                message: "Project is no longer registered.".to_string(),
                execution_id: None,
            });
            continue;
        };

        let active_execution = state
            .runtime
            .list_executions()
            .into_iter()
            .find(|execution| {
                execution.project_id == project.id
                    && execution.status == crate::core::models::ExecutionStatus::Running
            });

        match kind {
            "run" => {
                let Some(action) = primary_run_action(&project) else {
                    items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Skipped,
                        message: "No primary run action was found.".to_string(),
                        execution_id: None,
                    });
                    continue;
                };

                if active_execution.is_some() {
                    items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Skipped,
                        message: "Project is already running.".to_string(),
                        execution_id: None,
                    });
                    continue;
                }

                match state.runtime.run_action(
                    app.clone(),
                    Arc::clone(&state.store),
                    project.clone(),
                    action.clone(),
                ) {
                    Ok(execution) => items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Success,
                        message: format!("Started {}.", action.label),
                        execution_id: Some(execution.id),
                    }),
                    Err(error) => items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Failed,
                        message: error,
                        execution_id: None,
                    }),
                }
            }
            "stop" => {
                let Some(active) = active_execution else {
                    items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Skipped,
                        message: "No running execution to stop.".to_string(),
                        execution_id: None,
                    });
                    continue;
                };

                match state.runtime.stop_execution(
                    app.clone(),
                    Arc::clone(&state.store),
                    &active.id,
                ) {
                    Ok(Some(execution)) => items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Success,
                        message: "Stopped active execution.".to_string(),
                        execution_id: Some(execution.id),
                    }),
                    Ok(None) => items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Skipped,
                        message: "No running execution to stop.".to_string(),
                        execution_id: None,
                    }),
                    Err(error) => items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Failed,
                        message: error,
                        execution_id: None,
                    }),
                }
            }
            "restart" => {
                let Some(action) = primary_run_action(&project) else {
                    items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Skipped,
                        message: "No primary run action was found.".to_string(),
                        execution_id: None,
                    });
                    continue;
                };

                if let Some(active) = active_execution {
                    let _ = state.runtime.stop_execution(
                        app.clone(),
                        Arc::clone(&state.store),
                        &active.id,
                    );
                }

                match state.runtime.run_action(
                    app.clone(),
                    Arc::clone(&state.store),
                    project.clone(),
                    action.clone(),
                ) {
                    Ok(execution) => items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Success,
                        message: format!("Restarted {}.", action.label),
                        execution_id: Some(execution.id),
                    }),
                    Err(error) => items.push(BatchActionItemResult {
                        project_id: project.id.clone(),
                        project_name: project.name.clone(),
                        status: BatchItemStatus::Failed,
                        message: error,
                        execution_id: None,
                    }),
                }
            }
            _ => {}
        }
    }

    Ok(summarize_batch_result(kind, items))
}

fn unique_destination(root: &Path, repo_name: &str) -> String {
    let mut suffix = 0;
    loop {
        let candidate = if suffix == 0 {
            root.join(repo_name)
        } else {
            root.join(format!("{repo_name}-{suffix}"))
        };
        if !candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
        suffix += 1;
    }
}

fn render_env_file(template: &[EnvTemplateField], profile: &EnvProfile) -> String {
    if let Some(raw) = &profile.raw_editor_text {
        if !raw.trim().is_empty() {
            return raw.clone();
        }
    }

    let mut lines = Vec::new();
    let mut seen = HashMap::new();
    for field in template {
        if let Some(description) = &field.description {
            lines.push(format!("# {description}"));
        }
        let value = profile
            .values
            .get(&field.key)
            .cloned()
            .or_else(|| field.default_value.clone())
            .unwrap_or_default();
        lines.push(format!("{}={}", field.key, value));
        lines.push(String::new());
        seen.insert(field.key.clone(), true);
    }

    for (key, value) in &profile.values {
        if seen.contains_key(key) {
            continue;
        }
        lines.push(format!("{key}={value}"));
    }

    lines.join("\n")
}

fn summarize_batch_result(kind: &str, items: Vec<BatchActionItemResult>) -> BatchActionResult {
    let success_count = items
        .iter()
        .filter(|item| matches!(item.status, BatchItemStatus::Success))
        .count();
    let failure_count = items
        .iter()
        .filter(|item| matches!(item.status, BatchItemStatus::Failed))
        .count();
    let skipped_count = items
        .iter()
        .filter(|item| matches!(item.status, BatchItemStatus::Skipped))
        .count();

    BatchActionResult {
        kind: kind.to_string(),
        total: items.len(),
        success_count,
        failure_count,
        skipped_count,
        items,
    }
}

fn refresh_routes(store: &Arc<ProjectStore>, gateway_port: u16) -> Result<(), String> {
    for project in store.list()? {
        let slug = slugify(&project.name);
        let _ = store.update(&project.id, |item| {
            item.slug = slug.clone();
            item.route_subdomain_url = format!("http://{}.localhost:{}", slug, gateway_port);
            item.route_path_url = format!("http://gateway.localhost:{}/p/{}/", gateway_port, slug);
            item.updated_at = now_iso();
        })?;
    }
    Ok(())
}

fn primary_run_action(project: &ManagedProject) -> Option<&ProjectAction> {
    project
        .actions
        .iter()
        .find(|action| matches!(action.kind, ActionKind::Run))
}

fn fresh_project(
    store: &Arc<ProjectStore>,
    project_id: &str,
    gateway_port: u16,
) -> Result<ManagedProject, String> {
    let project = store
        .get(project_id)?
        .ok_or_else(|| "Project not found.".to_string())?;
    refresh_project_metadata(store, project, gateway_port)
}

fn refresh_project_metadata(
    store: &Arc<ProjectStore>,
    project: ManagedProject,
    gateway_port: u16,
) -> Result<ManagedProject, String> {
    let inferred = infer_project_from_path(
        Path::new(&project.root_path),
        project.git_url.clone(),
        gateway_port,
    )
    .unwrap_or_else(|| project.clone());

    let mut merged = inferred;
    merged.id = project.id.clone();
    merged.status = project.status.clone();
    merged.last_error = project.last_error.clone();
    merged.resolved_port = project.resolved_port;
    merged.env_profile = project.env_profile.clone();
    merged.created_at = project.created_at.clone();
    merged.updated_at = now_iso();
    store.upsert(merged.clone())?;
    Ok(merged)
}

fn build_project_recipe(project: &ManagedProject) -> ProjectRecipe {
    ProjectRecipe {
        version: 1,
        project_name: Some(project.name.clone()),
        primary_target_id: project.primary_target_id.clone(),
        preferred_port: project.preferred_port,
        install_action_id: project
            .actions
            .iter()
            .find(|action| matches!(action.kind, ActionKind::Install))
            .map(|action| action.id.clone()),
        run_action_id: project
            .actions
            .iter()
            .find(|action| matches!(action.kind, ActionKind::Run))
            .map(|action| action.id.clone()),
        open_action_id: project
            .actions
            .iter()
            .find(|action| matches!(action.kind, ActionKind::Open))
            .map(|action| action.id.clone()),
        readme_hints: project.readme_hints.clone(),
        env_keys: project
            .env_template
            .iter()
            .map(|field| field.key.clone())
            .collect(),
        kind: Some(project.project_profile.kind.clone()),
        preferred_entrypoint: project.project_profile.preferred_entrypoint.clone(),
        required_services: project.project_profile.required_services.clone(),
        required_env_groups: project.project_profile.required_env_groups.clone(),
        known_ports: project.project_profile.known_ports.clone(),
        route_strategy: project.project_profile.route_strategy.clone(),
        targets: project
            .workspace_targets
            .iter()
            .map(|target| ProjectRecipeTarget {
                id: target.id.clone(),
                relative_path: target.relative_path.clone(),
                runtime_kind: Some(target.runtime_kind.clone()),
                priority: Some(target.priority),
                suggested_port: target.suggested_port,
            })
            .collect(),
    }
}

fn build_doctor_report(project: &ManagedProject, https_status: &LocalHttpsStatus) -> DoctorReport {
    let install_action_id = project
        .actions
        .iter()
        .find(|action| matches!(action.kind, ActionKind::Install))
        .map(|action| action.id.clone());
    let run_action_id = project
        .actions
        .iter()
        .find(|action| matches!(action.kind, ActionKind::Run))
        .map(|action| action.id.clone());
    let open_action_id = project
        .actions
        .iter()
        .find(|action| matches!(action.kind, ActionKind::Open))
        .map(|action| action.id.clone());
    let primary_target = project.primary_target_id.as_ref().and_then(|target_id| {
        project
            .workspace_targets
            .iter()
            .find(|target| &target.id == target_id)
    });

    let env_values = merged_env_values(project);
    let missing_env_keys = project
        .env_template
        .iter()
        .filter_map(|field| {
            let value = env_values
                .get(&field.key)
                .map(|value| value.trim())
                .unwrap_or("");
            if value.is_empty() {
                Some(field.key.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let tooling = tooling_check(project);
    let env = env_check(project, &missing_env_keys);
    let install = install_check(project);
    let port_conflicts = project_port_conflicts(project);
    let compose_requirements = build_compose_requirements(project, &env_values);
    let service_requirements = compose_requirements
        .iter()
        .filter(|item| item.kind == "service" || item.kind == "local-service")
        .cloned()
        .collect::<Vec<_>>();
    let port = port_check(project, &port_conflicts);
    let blockers = build_doctor_blockers(
        &tooling,
        &env,
        &install,
        &port_conflicts,
        &compose_requirements,
        &project.project_profile.required_env_groups,
        https_status,
    );

    let mut checks = Vec::new();
    checks.push(tooling);
    checks.push(env);
    checks.push(install);
    checks.push(port);
    if !project.workspace_targets.is_empty() {
        checks.push(DoctorCheck {
            id: "workspace-targets".to_string(),
            label: "Monorepo Targets".to_string(),
            status: if primary_target.is_some() {
                DoctorStatus::Ok
            } else {
                DoctorStatus::Info
            },
            summary: format!(
                "Detected {} runnable app target{} inside this repo.",
                project.workspace_targets.len(),
                if project.workspace_targets.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ),
            detail: Some(if let Some(target) = primary_target {
                format!(
                    "Recommended target: {} ({}) | Other targets: {}",
                    target.name,
                    target.relative_path,
                    project
                        .workspace_targets
                        .iter()
                        .filter(|item| item.id != target.id)
                        .map(|item| format!("{} ({})", item.name, item.relative_path))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                project
                    .workspace_targets
                    .iter()
                    .map(|target| format!("{} ({})", target.name, target.relative_path))
                    .collect::<Vec<_>>()
                    .join(", ")
            }),
            fix_label: None,
            fix_command: None,
        });
    }
    if !project.readme_hints.is_empty() {
        checks.push(DoctorCheck {
            id: "readme-hints".to_string(),
            label: "README Hints".to_string(),
            status: DoctorStatus::Info,
            summary: "PortPilot found likely setup commands in the repository README.".to_string(),
            detail: Some(project.readme_hints.join(" | ")),
            fix_label: None,
            fix_command: None,
        });
    }
    checks.push(https_check(https_status));

    DoctorReport {
        project_id: project.id.clone(),
        generated_at: now_iso(),
        missing_env_keys: missing_env_keys.clone(),
        install_action_id,
        run_action_id,
        open_action_id,
        recommended_next_step: recommend_next_step(
            project,
            &missing_env_keys,
            &port_conflicts,
            &compose_requirements,
        ),
        blockers,
        port_conflicts,
        compose_requirements,
        service_requirements,
        checks,
    }
}

fn recommend_next_step(
    project: &ManagedProject,
    missing_env_keys: &[String],
    port_conflicts: &[DoctorPortConflict],
    compose_requirements: &[ComposeRequirement],
) -> Option<String> {
    if tooling_check(project).status == DoctorStatus::Error {
        return Some("Install the required local tooling first.".to_string());
    }

    if !missing_env_keys.is_empty() {
        if compose_requirements
            .iter()
            .any(|item| item.kind == "env" && !item.ready)
        {
            let groups = if project.project_profile.required_env_groups.is_empty() {
                None
            } else {
                Some(project.project_profile.required_env_groups.join(", "))
            };
            return Some(
                match groups {
                    Some(groups) => format!(
                        "Fill in the required compose env values for {groups} before starting this stack."
                    ),
                    None => {
                        "Fill in the required compose env values before starting this stack."
                            .to_string()
                    }
                },
            );
        }
        return Some(format!(
            "Fill in {} missing environment value{} before running.",
            missing_env_keys.len(),
            if missing_env_keys.len() == 1 { "" } else { "s" }
        ));
    }

    if port_conflicts
        .iter()
        .any(|conflict| conflict.occupied && !conflict.can_auto_reassign)
    {
        let port = port_conflicts
            .iter()
            .find(|conflict| conflict.occupied && !conflict.can_auto_reassign)
            .map(|conflict| conflict.port)
            .unwrap_or_default();
        return Some(format!(
            "Free fixed port {port} or change the command arguments before starting this project."
        ));
    }

    if compose_requirements
        .iter()
        .any(|item| item.kind == "service" && !item.ready)
    {
        return Some(
            "Start the required compose services first, then run the recommended entrypoint."
                .to_string(),
        );
    }

    if compose_requirements
        .iter()
        .any(|item| item.kind == "local-service" && !item.ready)
    {
        return missing_service_action_hint(project)
            .or_else(|| Some("Start the required local services first.".to_string()));
    }

    if matches!(project.status, RuntimeStatus::Running) {
        return Some("Open the live route or inspect the runtime panel.".to_string());
    }

    if let Some(target) = project.primary_target_id.as_ref().and_then(|target_id| {
        project
            .workspace_targets
            .iter()
            .find(|item| &item.id == target_id)
    }) {
        return Some(format!(
            "Start the recommended target {} in {}.",
            target.name, target.relative_path
        ));
    }

    if project
        .actions
        .iter()
        .any(|action| matches!(action.kind, ActionKind::Install))
    {
        return Some("Run install first, then start the primary action.".to_string());
    }

    if project
        .actions
        .iter()
        .any(|action| matches!(action.kind, ActionKind::Run))
    {
        return Some("Start the primary run action to bring this repo online.".to_string());
    }

    None
}

fn build_runtime_node(
    project: &ManagedProject,
    executions: &[ActionExecution],
    logs: &[LogEntry],
    https_status: &LocalHttpsStatus,
    gateway_port: u16,
) -> RuntimeNode {
    let current_execution = executions
        .iter()
        .filter(|execution| execution.project_id == project.id)
        .max_by(|left, right| left.started_at.cmp(&right.started_at));

    let execution_logs = current_execution
        .map(|execution| {
            logs.iter()
                .filter(|entry| entry.execution_id == execution.id)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let services = runtime_services_for_project(project);
    let dependencies_ready = dependencies_ready(project, &services);
    let run_phase = current_execution
        .map(|execution| infer_run_phase(execution, &execution_logs, dependencies_ready));
    let health = current_execution.and_then(|execution| {
        let port = execution
            .resolved_port
            .or(project.resolved_port)
            .or(project.preferred_port);
        let url = port.map(|value| format!("http://127.0.0.1:{value}/"));
        let ready_from_logs = execution_logs
            .iter()
            .rev()
            .any(|entry| is_ready_signal(&entry.message));
        let ready = port.map(port_is_open).unwrap_or(false) || ready_from_logs;
        let readiness_reason = if ready {
            Some("Port opened or the process emitted a ready signal.".to_string())
        } else if !dependencies_ready {
            Some("Required local services are not ready yet.".to_string())
        } else if execution.status == crate::core::models::ExecutionStatus::Running {
            Some(
                "The process is still booting and has not exposed a healthy route yet.".to_string(),
            )
        } else {
            None
        };
        let summary = if ready {
            Some("Route is reachable and the process looks ready.".to_string())
        } else if !dependencies_ready {
            Some(
                "Waiting for supporting services before this project can be considered healthy."
                    .to_string(),
            )
        } else if execution.status == crate::core::models::ExecutionStatus::Running {
            Some("Waiting for the project to bind a port or emit a ready signal.".to_string())
        } else {
            None
        };
        Some(HealthProbeResult {
            url,
            ready,
            last_checked_at: Some(now_iso()),
            summary,
            readiness_reason,
        })
    });

    RuntimeNode {
        project_id: project.id.clone(),
        project_name: project.name.clone(),
        kind: project.project_profile.kind.clone(),
        runtime_kind: project.runtime_kind.clone(),
        status: project.status.clone(),
        execution_id: current_execution.map(|execution| execution.id.clone()),
        execution_label: current_execution.map(|execution| execution.label.clone()),
        execution_status: current_execution.map(|execution| execution.status.clone()),
        run_phase,
        route_url: project.route_path_url.clone(),
        port: project.resolved_port.or(project.preferred_port),
        local_urls: project_local_urls(project, https_status, gateway_port),
        last_log: execution_logs.last().map(|entry| entry.message.clone()),
        health,
        services,
        dependencies_ready,
        recommended_action: runtime_recommended_action(project, dependencies_ready),
    }
}

fn runtime_recommended_action(
    project: &ManagedProject,
    dependencies_ready: bool,
) -> Option<String> {
    if !dependencies_ready {
        if let Some(message) = missing_service_action_hint(project) {
            return Some(message);
        }
        if project.has_docker_compose {
            return Some("Start the required compose services first.".to_string());
        }
    }

    if matches!(project.status, RuntimeStatus::Running) {
        return Some("Open the live route or inspect recent logs.".to_string());
    }

    if let Some(action_id) = &project.project_profile.preferred_entrypoint {
        if let Some(action) = project
            .actions
            .iter()
            .find(|action| &action.id == action_id)
        {
            return Some(format!(
                "Run {} to bring this project online.",
                action.label
            ));
        }
    }

    primary_run_action(project)
        .map(|action| format!("Run {} to bring this project online.", action.label))
}

fn dependencies_ready(project: &ManagedProject, services: &[ComposeServiceStatus]) -> bool {
    if !project.has_docker_compose {
        if project.project_profile.required_services.is_empty() {
            return true;
        }
        return project
            .project_profile
            .required_services
            .iter()
            .all(|required| {
                known_local_service_port(required)
                    .map(port_is_open)
                    .unwrap_or(true)
            });
    }

    if project.project_profile.required_services.is_empty() {
        return true;
    }

    project
        .project_profile
        .required_services
        .iter()
        .all(|required| service_dependency_ready(required, services))
}

fn build_compose_requirements(
    project: &ManagedProject,
    env_values: &HashMap<String, String>,
) -> Vec<ComposeRequirement> {
    let mut requirements = Vec::new();
    let services = if project.has_docker_compose {
        collect_compose_services(project)
    } else {
        Vec::new()
    };
    let required_services = if !project.project_profile.required_services.is_empty() {
        project.project_profile.required_services.clone()
    } else if project.has_docker_compose {
        services
            .iter()
            .map(|service| service.name.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    for service_name in required_services {
        let service = services.iter().find(|item| item.name == service_name);
        let known_local_port = known_local_service_port(&service_name);
        let ready = service_dependency_ready(&service_name, &services);
        let detail = service
            .map(|item| {
                let mut details = Vec::new();
                if let Some(state) = &item.state {
                    details.push(format!("state: {state}"));
                }
                if let Some(health) = &item.health {
                    details.push(format!("health: {health}"));
                }
                if !item.published_ports.is_empty() {
                    details.push(format!("ports: {}", item.published_ports.join(", ")));
                }
                details.join(" | ")
            })
            .or_else(|| {
                known_local_port.map(|port| {
                    let hint = known_local_service_hint(&service_name)
                        .unwrap_or("Start the dependency before launching the main app.");
                    let status = match local_service_status(&service_name) {
                        LocalServiceStatus::Ready => "Status: ready.",
                        LocalServiceStatus::Stopped => "Status: stopped but manageable from PortPilot.",
                        LocalServiceStatus::Failed => "Status: failed.",
                        LocalServiceStatus::UnmanagedAlreadyRunning => {
                            "Status: already running outside PortPilot."
                        }
                        LocalServiceStatus::Unmanaged => {
                            "Status: unavailable until you install or start it manually."
                        }
                    };
                    let start = local_service_start_command(&service_name)
                        .map(|command| format!(" Suggested start: {command}"))
                        .unwrap_or_default();
                    format!(
                        "Expected local service on 127.0.0.1:{port}. {status} {hint}{start}"
                    )
                })
            });

        requirements.push(ComposeRequirement {
            kind: if service.is_some() {
                "service".to_string()
            } else if known_local_port.is_some() {
                "local-service".to_string()
            } else if project.has_docker_compose {
                "service".to_string()
            } else {
                "local-service".to_string()
            },
            name: service_name,
            ready,
            detail: detail.filter(|value| !value.is_empty()),
        });
    }

    if project.has_docker_compose {
        for field in project.env_template.iter().filter(|field| {
            field
                .description
                .as_deref()
                .map(|detail| detail.contains("docker-compose"))
                .unwrap_or(false)
        }) {
            let ready = env_values
                .get(&field.key)
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
            requirements.push(ComposeRequirement {
                kind: "env".to_string(),
                name: field.key.clone(),
                ready,
                detail: Some(
                    "Required for docker compose configuration or volume mapping.".to_string(),
                ),
            });
        }
    }

    requirements
}

fn build_doctor_blockers(
    tooling: &DoctorCheck,
    env: &DoctorCheck,
    install: &DoctorCheck,
    port_conflicts: &[DoctorPortConflict],
    compose_requirements: &[ComposeRequirement],
    required_env_groups: &[String],
    https_status: &LocalHttpsStatus,
) -> Vec<DoctorBlocker> {
    let mut blockers = Vec::new();

    if matches!(tooling.status, DoctorStatus::Error) {
        blockers.push(DoctorBlocker {
            id: tooling.id.clone(),
            label: tooling.label.clone(),
            summary: tooling.summary.clone(),
            fix_label: tooling.fix_label.clone(),
            fix_command: tooling.fix_command.clone(),
        });
    }

    if matches!(env.status, DoctorStatus::Warn | DoctorStatus::Error) {
        blockers.push(DoctorBlocker {
            id: env.id.clone(),
            label: env.label.clone(),
            summary: env.summary.clone(),
            fix_label: env.fix_label.clone(),
            fix_command: env.fix_command.clone(),
        });
    }

    if matches!(install.status, DoctorStatus::Warn | DoctorStatus::Error) {
        blockers.push(DoctorBlocker {
            id: install.id.clone(),
            label: install.label.clone(),
            summary: install.summary.clone(),
            fix_label: install.fix_label.clone(),
            fix_command: install.fix_command.clone(),
        });
    }

    if let Some(conflict) = port_conflicts
        .iter()
        .find(|conflict| conflict.occupied && !conflict.can_auto_reassign)
    {
        blockers.push(DoctorBlocker {
            id: "fixed-port-conflict".to_string(),
            label: "Fixed Port Conflict".to_string(),
            summary: format!(
                "Port {} is busy and this command cannot be auto-reassigned.",
                conflict.port
            ),
            fix_label: Some("Free the port".to_string()),
            fix_command: None,
        });
    }

    let missing_compose_env = compose_requirements
        .iter()
        .filter(|item| item.kind == "env" && !item.ready)
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    if !missing_compose_env.is_empty() {
        let group_suffix = if required_env_groups.is_empty() {
            String::new()
        } else {
            format!(
                " Related groups: {}.",
                required_env_groups.join(", ")
            )
        };
        blockers.push(DoctorBlocker {
            id: "compose-env".to_string(),
            label: "Compose Env".to_string(),
            summary: format!(
                "Compose is missing {} required env value{}: {}.{}",
                missing_compose_env.len(),
                if missing_compose_env.len() == 1 {
                    ""
                } else {
                    "s"
                },
                missing_compose_env.join(", "),
                group_suffix
            ),
            fix_label: Some("Fill env values".to_string()),
            fix_command: None,
        });
    }

    let missing_local_services = compose_requirements
        .iter()
        .filter(|item| item.kind == "local-service" && !item.ready)
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    if !missing_local_services.is_empty() {
        let primary_service = missing_local_services.first().cloned().unwrap_or_default();
        blockers.push(DoctorBlocker {
            id: "local-services".to_string(),
            label: "Local Services".to_string(),
            summary: format!(
                "Start the required local service{} first: {}.",
                if missing_local_services.len() == 1 {
                    ""
                } else {
                    "s"
                },
                missing_local_services.join(", ")
            ),
            fix_label: Some("Suggested start".to_string()),
            fix_command: local_service_start_command(&primary_service).map(ToString::to_string),
        });
    }

    match https_status.certificate_state {
        LocalHttpsCertificateState::Trusted => {}
        LocalHttpsCertificateState::NeedsInstall => blockers.push(DoctorBlocker {
            id: "localhost-https".to_string(),
            label: "Local HTTPS".to_string(),
            summary: "PortPilot could not find mkcert yet. HTTPS can only fall back to a self-signed certificate until mkcert is installed.".to_string(),
            fix_label: Some("Install trusted HTTPS".to_string()),
            fix_command: Some("brew install mkcert nss && mkcert -install".to_string()),
        }),
        LocalHttpsCertificateState::NeedsTrust => blockers.push(DoctorBlocker {
            id: "localhost-https".to_string(),
            label: "Local HTTPS".to_string(),
            summary: "HTTPS is available, but the current localhost certificate still needs browser trust. Install or trust the mkcert CA to make HTTPS the default local route.".to_string(),
            fix_label: Some("Suggested fix".to_string()),
            fix_command: Some("brew install mkcert nss && mkcert -install".to_string()),
        }),
        LocalHttpsCertificateState::FallbackSelfSigned => blockers.push(DoctorBlocker {
            id: "localhost-https".to_string(),
            label: "Local HTTPS".to_string(),
            summary: "HTTPS is running with a self-signed localhost certificate. Browsers will warn until mkcert is installed and PortPilot reloads a trusted cert.".to_string(),
            fix_label: Some("Install trusted HTTPS".to_string()),
            fix_command: Some("brew install mkcert nss && mkcert -install".to_string()),
        }),
        LocalHttpsCertificateState::Error => blockers.push(DoctorBlocker {
            id: "localhost-https".to_string(),
            label: "Local HTTPS".to_string(),
            summary: https_status
                .detail
                .clone()
                .unwrap_or_else(|| "PortPilot hit an HTTPS setup error.".to_string()),
            fix_label: Some("Retry setup".to_string()),
            fix_command: None,
        }),
    }

    blockers
}

fn https_check(https_status: &LocalHttpsStatus) -> DoctorCheck {
    match https_status.certificate_state {
        LocalHttpsCertificateState::Trusted => DoctorCheck {
            id: "local-https".to_string(),
            label: "Local HTTPS".to_string(),
            status: DoctorStatus::Ok,
            summary: "Trusted localhost HTTPS is available.".to_string(),
            detail: https_status
                .https_port
                .map(|port| format!("HTTPS gateway is listening on gateway.localhost:{port}."))
                .or_else(|| https_status.detail.clone()),
            fix_label: None,
            fix_command: None,
        },
        LocalHttpsCertificateState::NeedsInstall => DoctorCheck {
            id: "local-https".to_string(),
            label: "Local HTTPS".to_string(),
            status: DoctorStatus::Info,
            summary: "Trusted localhost HTTPS is not installed yet.".to_string(),
            detail: Some(
                "Install mkcert to move PortPilot from a self-signed fallback to trusted localhost HTTPS."
                    .to_string(),
            ),
            fix_label: Some("Install trusted HTTPS".to_string()),
            fix_command: Some("brew install mkcert nss && mkcert -install".to_string()),
        },
        LocalHttpsCertificateState::NeedsTrust => DoctorCheck {
            id: "local-https".to_string(),
            label: "Local HTTPS".to_string(),
            status: DoctorStatus::Warn,
            summary: "Local HTTPS is available with a certificate that still needs trust."
                .to_string(),
            detail: https_status.detail.clone(),
            fix_label: Some("Suggested fix".to_string()),
            fix_command: Some("brew install mkcert nss && mkcert -install".to_string()),
        },
        LocalHttpsCertificateState::FallbackSelfSigned => DoctorCheck {
            id: "local-https".to_string(),
            label: "Local HTTPS".to_string(),
            status: DoctorStatus::Warn,
            summary: "Local HTTPS is using a self-signed fallback certificate.".to_string(),
            detail: https_status.detail.clone(),
            fix_label: Some("Install trusted HTTPS".to_string()),
            fix_command: Some("brew install mkcert nss && mkcert -install".to_string()),
        },
        LocalHttpsCertificateState::Error => DoctorCheck {
            id: "local-https".to_string(),
            label: "Local HTTPS".to_string(),
            status: DoctorStatus::Error,
            summary: "PortPilot could not finish the localhost HTTPS setup.".to_string(),
            detail: https_status.detail.clone(),
            fix_label: Some("Retry setup".to_string()),
            fix_command: None,
        },
    }
}

fn project_local_urls(
    project: &ManagedProject,
    https_status: &LocalHttpsStatus,
    gateway_port: u16,
) -> Vec<LocalUrl> {
    let slug = &project.slug;
    let http_subdomain = format!("http://{}.localhost:{}", slug, gateway_port);
    let http_path = format!("http://gateway.localhost:{}/p/{}/", gateway_port, slug);
    let mut urls = Vec::new();

    if let Some(https_port) = https_status.https_port {
        let https_subdomain = format!("https://{}.localhost:{}", slug, https_port);
        let https_path = format!("https://gateway.localhost:{}/p/{}/", https_port, slug);
        let https_recommended = matches!(
            https_status.certificate_state,
            LocalHttpsCertificateState::Trusted
        );
        urls.push(LocalUrl {
            kind: "https_subdomain".to_string(),
            url: https_subdomain,
            recommended: https_recommended,
        });
        urls.push(LocalUrl {
            kind: "https_path".to_string(),
            url: https_path,
            recommended: false,
        });
        urls.push(LocalUrl {
            kind: "http_subdomain".to_string(),
            url: http_subdomain,
            recommended: !https_recommended,
        });
        urls.push(LocalUrl {
            kind: "http_path".to_string(),
            url: http_path,
            recommended: false,
        });
        return urls;
    }

    urls.push(LocalUrl {
        kind: "http_subdomain".to_string(),
        url: http_subdomain,
        recommended: true,
    });
    urls.push(LocalUrl {
        kind: "http_path".to_string(),
        url: http_path,
        recommended: false,
    });
    urls
}

fn project_port_conflicts(project: &ManagedProject) -> Vec<DoctorPortConflict> {
    let Some(port) = project.preferred_port else {
        return Vec::new();
    };

    let occupied = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().expect("socket addr"),
        Duration::from_millis(250),
    )
    .is_ok();
    let fixed_from_config = fixed_port_from_project_config(project);
    let can_auto_reassign = primary_run_action(project)
        .map(|action| fixed_port_from_command(&action.command).is_none())
        .unwrap_or(false)
        && fixed_from_config.is_none();

    vec![DoctorPortConflict {
        port,
        occupied,
        can_auto_reassign,
        detail: if occupied && !can_auto_reassign {
            if let Some(config_path) = fixed_from_config {
                format!(
                    "This project hardcodes its port in {config_path}, so PortPilot cannot move it automatically."
                )
            } else {
                "This project hardcodes its port, so PortPilot cannot move it automatically."
                    .to_string()
            }
        } else if occupied {
            "PortPilot can reassign this port when the primary run action starts.".to_string()
        } else {
            "The preferred port is currently free.".to_string()
        },
    }]
}

fn collect_compose_services(project: &ManagedProject) -> Vec<ComposeServiceStatus> {
    let root = Path::new(&project.root_path);
    let Some(compose_file) = find_compose_file(root) else {
        return Vec::new();
    };

    let running = query_compose_ps(&project.root_path, &compose_file)
        .into_iter()
        .map(|service| (service.name.clone(), service))
        .collect::<HashMap<_, _>>();
    let configured = query_compose_service_names(&project.root_path, &compose_file);

    if configured.is_empty() {
        return running.into_values().collect();
    }

    configured
        .into_iter()
        .map(|name| {
            running.get(&name).cloned().unwrap_or(ComposeServiceStatus {
                name,
                state: Some("stopped".to_string()),
                health: None,
                container_name: None,
                published_ports: Vec::new(),
            })
        })
        .collect()
}

fn query_compose_service_names(workdir: &str, compose_file: &Path) -> Vec<String> {
    let compose_file_str = compose_file.to_string_lossy().to_string();
    let commands = [
        (
            "docker",
            vec![
                "compose",
                "-f",
                compose_file_str.as_str(),
                "config",
                "--services",
            ],
        ),
        (
            "docker-compose",
            vec!["-f", compose_file_str.as_str(), "config", "--services"],
        ),
    ];

    for (bin, args) in commands {
        let output = Command::new(bin).args(args).current_dir(workdir).output();
        let Ok(output) = output else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let names = stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !names.is_empty() {
            return names;
        }
    }

    parse_compose_service_names_from_file(compose_file)
}

fn query_compose_ps(workdir: &str, compose_file: &Path) -> Vec<ComposeServiceStatus> {
    let compose_file_str = compose_file.to_string_lossy().to_string();
    let commands = [
        (
            "docker",
            vec![
                "compose",
                "-f",
                compose_file_str.as_str(),
                "ps",
                "--format",
                "json",
            ],
        ),
        (
            "docker-compose",
            vec!["-f", compose_file_str.as_str(), "ps", "--format", "json"],
        ),
    ];

    for (bin, args) in commands {
        let output = Command::new(bin).args(args).current_dir(workdir).output();
        let Ok(output) = output else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            continue;
        }
        if let Some(items) = parse_compose_ps_json(&stdout) {
            return items;
        }
    }

    Vec::new()
}

fn parse_compose_ps_json(contents: &str) -> Option<Vec<ComposeServiceStatus>> {
    let value = if contents.trim_start().starts_with('[') {
        serde_json::from_str::<serde_json::Value>(contents).ok()
    } else {
        let rows = contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        Some(serde_json::Value::Array(rows))
    }?;

    let items = value.as_array()?;
    let mut output = Vec::new();
    for item in items {
        let published_ports = item
            .get("Publishers")
            .and_then(|value| value.as_array())
            .map(|publishers| {
                publishers
                    .iter()
                    .map(|publisher| {
                        let url = publisher
                            .get("URL")
                            .and_then(|value| value.as_str())
                            .unwrap_or("127.0.0.1");
                        let published = publisher
                            .get("PublishedPort")
                            .and_then(|value| value.as_u64())
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        let target = publisher
                            .get("TargetPort")
                            .and_then(|value| value.as_u64())
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        format!("{url}:{published}->{target}")
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        output.push(ComposeServiceStatus {
            name: item
                .get("Service")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown")
                .to_string(),
            state: item
                .get("State")
                .and_then(|value| value.as_str())
                .map(ToString::to_string),
            health: item
                .get("Health")
                .and_then(|value| value.as_str())
                .map(ToString::to_string),
            container_name: item
                .get("Name")
                .and_then(|value| value.as_str())
                .map(ToString::to_string),
            published_ports,
        });
    }
    Some(output)
}

fn parse_compose_service_names_from_file(compose_file: &Path) -> Vec<String> {
    let Ok(contents) = fs::read_to_string(compose_file) else {
        return Vec::new();
    };

    let mut services = Vec::new();
    let mut in_services = false;
    for line in contents.lines() {
        let raw = line.trim_end();
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if raw.starts_with("services:") {
            in_services = true;
            continue;
        }
        if !in_services {
            continue;
        }
        if !raw.starts_with("  ") {
            break;
        }
        let candidate = raw.trim_start();
        if candidate.starts_with('-') || candidate.starts_with('#') || !candidate.ends_with(':') {
            continue;
        }
        if candidate.contains(' ') {
            continue;
        }
        services.push(candidate.trim_end_matches(':').to_string());
    }

    services
}

fn infer_run_phase(
    execution: &ActionExecution,
    logs: &[LogEntry],
    dependencies_ready: bool,
) -> RunPhase {
    use crate::core::models::ExecutionStatus;

    match execution.status {
        ExecutionStatus::Failed => return RunPhase::Failed,
        ExecutionStatus::Stopped | ExecutionStatus::Success => return RunPhase::Stopped,
        ExecutionStatus::Running => {}
    }

    if execution.command.contains(" install") || execution.command.starts_with("uv sync") {
        return RunPhase::Installing;
    }

    if logs
        .iter()
        .rev()
        .any(|entry| is_ready_signal(&entry.message))
    {
        return RunPhase::Healthy;
    }

    if !dependencies_ready {
        return RunPhase::WaitingForService;
    }

    if execution.resolved_port.is_some() {
        if execution.resolved_port.map(port_is_open).unwrap_or(false) {
            return RunPhase::Healthy;
        }
        return RunPhase::WaitingForPort;
    }

    if logs
        .iter()
        .rev()
        .any(|entry| is_failure_signal(&entry.message))
    {
        return RunPhase::Failed;
    }

    RunPhase::Starting
}

fn is_ready_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("ready in")
        || normalized.contains("server ready")
        || normalized.contains("listening on")
        || normalized.contains("listening at")
        || normalized.contains("started on")
        || normalized.contains("healthcheck passed")
        || normalized.contains("local:")
}

fn is_failure_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("error")
        || normalized.contains("failed")
        || normalized.contains("exception")
        || normalized.contains("panic")
}

fn port_is_open(port: u16) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok()
}

fn known_local_service_port(service_name: &str) -> Option<u16> {
    match service_name.to_ascii_lowercase().as_str() {
        "ollama" => Some(11434),
        "mongodb" => Some(27017),
        "meilisearch" => Some(7700),
        "redis" => Some(6379),
        "postgres" | "postgresql" | "db" => Some(5432),
        "qdrant" => Some(6333),
        "weaviate" => Some(8080),
        "chroma" | "vectordb" => Some(8000),
        "rag_api" => Some(8000),
        _ => None,
    }
}

fn core_local_services() -> &'static [&'static str] {
    &["ollama", "mongodb", "redis", "postgres", "meilisearch"]
}

fn known_local_service_hint(service_name: &str) -> Option<&'static str> {
    match service_name.to_ascii_lowercase().as_str() {
        "ollama" => Some("Ollama is the common local model provider for this stack."),
        "mongodb" => Some("MongoDB must be available before the app can boot cleanly."),
        "meilisearch" => Some("Meilisearch is used for local indexing and search."),
        "redis" => Some("Redis is required for queue, cache, or worker coordination."),
        "postgres" | "postgresql" | "db" => {
            Some("Postgres is required for the primary app database.")
        }
        "qdrant" | "weaviate" | "chroma" | "vectordb" => {
            Some("Vector storage should be online before the app is considered ready.")
        }
        "rag_api" => Some("The RAG sidecar should be available before opening the main route."),
        _ => None,
    }
}

fn local_service_label(service_name: &str) -> String {
    match service_name.to_ascii_lowercase().as_str() {
        "ollama" => "Ollama".to_string(),
        "mongodb" => "MongoDB".to_string(),
        "meilisearch" => "Meilisearch".to_string(),
        "redis" => "Redis".to_string(),
        "postgres" | "postgresql" | "db" => "Postgres".to_string(),
        "qdrant" => "Qdrant".to_string(),
        "weaviate" => "Weaviate".to_string(),
        "chroma" | "vectordb" => "Chroma / Vector DB".to_string(),
        "rag_api" => "RAG API".to_string(),
        other => other.to_string(),
    }
}

fn collect_local_service_presets(projects: &[ManagedProject]) -> Vec<LocalServicePreset> {
    let mut by_service: HashMap<String, LocalServicePreset> = HashMap::new();

    for service_name in core_local_services() {
        let normalized = service_name.to_ascii_lowercase();
        let status = local_service_status(&normalized);
        let port = known_local_service_port(&normalized);
        by_service.insert(
            normalized.clone(),
            LocalServicePreset {
                name: normalized.clone(),
                label: local_service_label(&normalized),
                port,
                ready: matches!(
                    status,
                    LocalServiceStatus::Ready | LocalServiceStatus::UnmanagedAlreadyRunning
                ),
                status,
                ready_detail: local_service_ready_detail(&normalized),
                hint: known_local_service_hint(&normalized).map(ToString::to_string),
                start_command: local_service_start_command(&normalized).map(ToString::to_string),
                stop_command: local_service_stop_command(&normalized).map(ToString::to_string),
                managed: can_manage_local_service(&normalized),
                management_kind: local_service_management_kind(&normalized).map(ToString::to_string),
                used_by_projects: Vec::new(),
            },
        );
    }

    for project in projects {
        for service_name in &project.project_profile.required_services {
            let normalized = service_name.to_ascii_lowercase();
            let Some(port) = known_local_service_port(&normalized) else {
                continue;
            };
            let status = local_service_status(&normalized);

            let entry = by_service.entry(normalized.clone()).or_insert_with(|| {
                LocalServicePreset {
                    name: normalized.clone(),
                    label: local_service_label(&normalized),
                    port: Some(port),
                    ready: matches!(
                        status,
                        LocalServiceStatus::Ready | LocalServiceStatus::UnmanagedAlreadyRunning
                    ),
                    status: status.clone(),
                    ready_detail: local_service_ready_detail(&normalized),
                    hint: known_local_service_hint(&normalized).map(ToString::to_string),
                    start_command: local_service_start_command(&normalized)
                        .map(ToString::to_string),
                    stop_command: local_service_stop_command(&normalized)
                        .map(ToString::to_string),
                    managed: can_manage_local_service(&normalized),
                    management_kind: local_service_management_kind(&normalized)
                        .map(ToString::to_string),
                    used_by_projects: Vec::new(),
                }
            });

            if !entry.used_by_projects.iter().any(|name| name == &project.name) {
                entry.used_by_projects.push(project.name.clone());
            }
            entry.status = local_service_status(&normalized);
            entry.ready = matches!(
                entry.status,
                LocalServiceStatus::Ready | LocalServiceStatus::UnmanagedAlreadyRunning
            );
            entry.ready_detail = local_service_ready_detail(&normalized);
            entry.managed = can_manage_local_service(&normalized);
            entry.management_kind = local_service_management_kind(&normalized).map(ToString::to_string);
        }
    }

    let mut presets = by_service.into_values().collect::<Vec<_>>();
    presets.sort_by(|left, right| left.label.cmp(&right.label));
    presets
}

fn ensure_local_service_running(service_name: &str) -> Result<(), String> {
    let normalized = service_name.to_ascii_lowercase();
    if let Some(port) = known_local_service_port(&normalized) {
        if port_is_open(port) {
            return Ok(());
        }
    }

    match normalized.as_str() {
        "ollama" => {
            Command::new("sh")
                .args(["-lc", "nohup ollama serve >/tmp/portpilot-ollama.log 2>&1 &"])
                .output()
                .map_err(|error| format!("Failed to launch Ollama: {error}"))?;
        }
        "mongodb" | "meilisearch" | "redis" | "postgres" | "postgresql" | "db" | "qdrant"
        | "chroma" | "vectordb" => ensure_docker_service_running(&normalized)?,
        _ => {
            return Err(format!(
                "PortPilot cannot auto-start {service_name} yet. Copy the suggested start command instead."
            ))
        }
    }

    if let Some(port) = known_local_service_port(&normalized) {
        for _ in 0..20 {
            if port_is_open(port) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        return Err(format!(
            "{service_name} was started, but localhost:{port} is still not ready."
        ));
    }

    Ok(())
}

fn ensure_local_service_stopped(service_name: &str) -> Result<(), String> {
    let normalized = service_name.to_ascii_lowercase();
    if matches!(
        local_service_status(&normalized),
        LocalServiceStatus::UnmanagedAlreadyRunning
    ) {
        return Err(format!(
            "{service_name} is running outside PortPilot, so it cannot be stopped from here."
        ));
    }
    match normalized.as_str() {
        "ollama" => {
            Command::new("pkill")
                .args(["-f", "ollama serve"])
                .output()
                .map_err(|error| format!("Failed to stop Ollama: {error}"))?;
        }
        "mongodb" | "meilisearch" | "redis" | "postgres" | "postgresql" | "db" | "qdrant"
        | "chroma" | "vectordb" => stop_docker_service(&normalized)?,
        _ => {
            return Err(format!(
                "PortPilot cannot stop {service_name} yet. Stop it manually if needed."
            ))
        }
    }

    if let Some(port) = known_local_service_port(&normalized) {
        for _ in 0..20 {
            if !port_is_open(port) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        return Err(format!(
            "{service_name} stop request finished, but localhost:{port} still looks open."
        ));
    }

    Ok(())
}

fn local_service_status(service_name: &str) -> LocalServiceStatus {
    let normalized = service_name.to_ascii_lowercase();
    let port_open = known_local_service_port(&normalized)
        .map(port_is_open)
        .unwrap_or(false);

    match normalized.as_str() {
        "ollama" => {
            if port_open {
                if binary_exists("ollama") {
                    LocalServiceStatus::Ready
                } else {
                    LocalServiceStatus::UnmanagedAlreadyRunning
                }
            } else if binary_exists("ollama") {
                LocalServiceStatus::Stopped
            } else {
                LocalServiceStatus::Unmanaged
            }
        }
        "mongodb" | "meilisearch" | "redis" | "postgres" | "postgresql" | "db" | "qdrant"
        | "chroma" | "vectordb" => {
            let container_name = docker_service_container_name(&normalized);
            let container_exists = container_name.map(docker_container_exists).unwrap_or(false);
            let container_running = container_name
                .and_then(docker_container_state)
                .map(|state| state == "running")
                .unwrap_or(false);
            let docker_available = binary_exists("docker");
            if port_open && container_exists && container_running {
                LocalServiceStatus::Ready
            } else if container_exists {
                LocalServiceStatus::Failed
            } else if port_open {
                LocalServiceStatus::UnmanagedAlreadyRunning
            } else if docker_available {
                LocalServiceStatus::Stopped
            } else {
                LocalServiceStatus::Unmanaged
            }
        }
        _ => LocalServiceStatus::Unmanaged,
    }
}

fn local_service_ready_detail(service_name: &str) -> Option<String> {
    let normalized = service_name.to_ascii_lowercase();
    let status = local_service_status(&normalized);
    let label = local_service_label(&normalized);
    let port = known_local_service_port(&normalized)
        .map(|value| format!("localhost:{value}"))
        .unwrap_or_else(|| "localhost".to_string());

    Some(match status {
        LocalServiceStatus::Ready => format!("{label} is ready on {port}."),
        LocalServiceStatus::Stopped => match local_service_management_kind(&normalized) {
            Some("docker") => format!(
                "{label} is stopped right now, but PortPilot can start the managed Docker service."
            ),
            Some("native") => format!(
                "{label} is stopped right now, but PortPilot can start the native service."
            ),
            _ => format!("{label} is stopped right now."),
        },
        LocalServiceStatus::Failed => format!(
            "{label} has a managed instance, but it is not healthy or did not bind {port}."
        ),
        LocalServiceStatus::UnmanagedAlreadyRunning => format!(
            "{label} is already running on {port} outside PortPilot. It will be reused without taking ownership."
        ),
        LocalServiceStatus::Unmanaged => format!(
            "{label} is not installed or PortPilot cannot manage it automatically on this machine."
        ),
    })
}

fn ensure_docker_service_running(service_name: &str) -> Result<(), String> {
    let Some(container_name) = docker_service_container_name(service_name) else {
        return Err(format!("No managed Docker preset found for {service_name}."));
    };

    let existing = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name=^{}$", container_name),
            "--format",
            "{{.Names}}",
        ])
        .output()
        .map_err(|error| format!("Failed to inspect Docker containers: {error}"))?;

    let existing_name = String::from_utf8_lossy(&existing.stdout).trim().to_string();
    if existing_name == container_name {
        let started = Command::new("docker")
            .args(["start", container_name])
            .output()
            .map_err(|error| format!("Failed to start {service_name}: {error}"))?;
        if !started.status.success() {
            return Err(String::from_utf8_lossy(&started.stderr).trim().to_string());
        }
        return Ok(());
    }

    let args = docker_service_run_args(service_name)
        .ok_or_else(|| format!("No run arguments found for {service_name}."))?;
    let started = Command::new("docker")
        .args(args)
        .output()
        .map_err(|error| format!("Failed to run {service_name}: {error}"))?;
    if !started.status.success() {
        return Err(String::from_utf8_lossy(&started.stderr).trim().to_string());
    }
    Ok(())
}

fn stop_docker_service(service_name: &str) -> Result<(), String> {
    let Some(container_name) = docker_service_container_name(service_name) else {
        return Err(format!("No managed Docker preset found for {service_name}."));
    };
    let stopped = Command::new("docker")
        .args(["rm", "-f", container_name])
        .output()
        .map_err(|error| format!("Failed to stop {service_name}: {error}"))?;
    if !stopped.status.success() {
        let stderr = String::from_utf8_lossy(&stopped.stderr).trim().to_string();
        if stderr.contains("No such container") {
            return Ok(());
        }
        return Err(stderr);
    }
    Ok(())
}

fn docker_service_container_name(service_name: &str) -> Option<&'static str> {
    match service_name {
        "mongodb" => Some("portpilot-mongodb"),
        "meilisearch" => Some("portpilot-meilisearch"),
        "redis" => Some("portpilot-redis"),
        "postgres" | "postgresql" | "db" => Some("portpilot-postgres"),
        "qdrant" => Some("portpilot-qdrant"),
        "chroma" | "vectordb" => Some("portpilot-chroma"),
        _ => None,
    }
}

fn local_service_management_kind(service_name: &str) -> Option<&'static str> {
    match service_name {
        "ollama" => Some("native"),
        "mongodb" | "meilisearch" | "redis" | "postgres" | "postgresql" | "db" | "qdrant"
        | "chroma" | "vectordb" => Some("docker"),
        _ => None,
    }
}

fn local_service_stop_command(service_name: &str) -> Option<&'static str> {
    match service_name {
        "ollama" => Some("pkill -f 'ollama serve'"),
        "mongodb" => Some("docker rm -f portpilot-mongodb"),
        "meilisearch" => Some("docker rm -f portpilot-meilisearch"),
        "redis" => Some("docker rm -f portpilot-redis"),
        "postgres" | "postgresql" | "db" => Some("docker rm -f portpilot-postgres"),
        "qdrant" => Some("docker rm -f portpilot-qdrant"),
        "chroma" | "vectordb" => Some("docker rm -f portpilot-chroma"),
        _ => None,
    }
}

fn can_manage_local_service(service_name: &str) -> bool {
    match service_name {
        "ollama" => binary_exists("ollama"),
        "mongodb" | "meilisearch" | "redis" | "postgres" | "postgresql" | "db" | "qdrant"
        | "chroma" | "vectordb" => binary_exists("docker"),
        _ => false,
    }
}

fn docker_container_exists(container_name: &str) -> bool {
    let Ok(output) = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name=^{}$", container_name),
            "--format",
            "{{.Names}}",
        ])
        .output()
    else {
        return false;
    };

    String::from_utf8_lossy(&output.stdout).trim() == container_name
}

fn docker_container_state(container_name: &str) -> Option<String> {
    let Ok(output) = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name=^{}$", container_name),
            "--format",
            "{{.State}}",
        ])
        .output()
    else {
        return None;
    };

    let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if state.is_empty() {
        None
    } else {
        Some(state)
    }
}

fn docker_service_run_args(service_name: &str) -> Option<Vec<&'static str>> {
    match service_name {
        "mongodb" => Some(vec![
            "run", "-d", "--name", "portpilot-mongodb", "-p", "27017:27017", "mongo:7",
        ]),
        "meilisearch" => Some(vec![
            "run", "-d", "--name", "portpilot-meilisearch", "-e", "MEILI_NO_ANALYTICS=true",
            "-p", "7700:7700", "getmeili/meilisearch:v1.12",
        ]),
        "redis" => Some(vec![
            "run", "-d", "--name", "portpilot-redis", "-p", "6379:6379", "redis:7",
        ]),
        "postgres" | "postgresql" | "db" => Some(vec![
            "run", "-d", "--name", "portpilot-postgres", "-e", "POSTGRES_PASSWORD=postgres",
            "-p", "5432:5432", "postgres:16",
        ]),
        "qdrant" => Some(vec![
            "run", "-d", "--name", "portpilot-qdrant", "-p", "6333:6333", "qdrant/qdrant",
        ]),
        "chroma" | "vectordb" => Some(vec![
            "run", "-d", "--name", "portpilot-chroma", "-p", "8000:8000", "chromadb/chroma:latest",
        ]),
        _ => None,
    }
}

fn build_env_group_presets(project: &ManagedProject) -> Vec<EnvGroupPreset> {
    let mut presets = Vec::new();
    for group in &project.project_profile.required_env_groups {
        let preset = build_env_group_preset(project, group);
        if !preset.values.is_empty() || !preset.manual_keys.is_empty() {
            presets.push(preset);
        }
    }
    presets
}

fn build_env_group_preset(project: &ManagedProject, group: &str) -> EnvGroupPreset {
    let group_id = group.to_ascii_lowercase();
    let project_slug = slugify(&project.name);
    let project_root = Path::new(&project.root_path);
    let default_port = project.preferred_port.unwrap_or_else(|| {
        project
            .project_profile
            .known_ports
            .first()
            .copied()
            .unwrap_or(3000)
    });
    let mut values = HashMap::new();
    let mut manual_keys = Vec::new();

    for field in &project.env_template {
        let key = field.key.as_str();
        let upper = key.to_ascii_uppercase();
        let key_matches = match group_id.as_str() {
            "app" => matches!(
                upper.as_str(),
                "PORT" | "HOST" | "APP_URL" | "WEB_URL" | "SERVER_URL" | "API_URL"
            ),
            "database" => {
                upper.contains("MONGO")
                    || upper.contains("DATABASE")
                    || upper.contains("POSTGRES")
                    || upper.contains("PG")
            }
            "search" => {
                upper.contains("MEILI")
                    || upper.contains("SEARCH")
                    || upper.contains("QDRANT")
                    || upper.contains("WEAVIATE")
                    || upper.contains("CHROMA")
                    || upper.contains("VECTOR")
            }
            "rag" => upper.contains("RAG"),
            "queue" => upper.contains("REDIS") || upper.contains("QUEUE"),
            "workspace" => upper.contains("WORKSPACE") || upper.contains("CONFIG_DIR"),
            "gateway" => upper.contains("GATEWAY") || upper.contains("WEBCHAT"),
            "credentials" | "model-providers" | "llm-provider" | "models" => {
                upper.contains("API_KEY")
                    || upper.contains("TOKEN")
                    || upper.contains("SECRET")
                    || upper.contains("MODEL")
                    || upper.contains("PROVIDER")
            }
            "frontend" => upper == "PORT" || upper.contains("FRONTEND") || upper.contains("WEB"),
            "server" => upper == "PORT" || upper.contains("SERVER") || upper.contains("API"),
            _ => false,
        };

        if !key_matches {
            continue;
        }

        let suggested = suggested_env_value(project, &group_id, key, default_port, &project_slug, project_root);
        if let Some(value) = suggested {
            values.insert(field.key.clone(), value);
        } else if !manual_keys.iter().any(|item| item == key) {
            manual_keys.push(field.key.clone());
        }
    }

    EnvGroupPreset {
        id: group_id.clone(),
        label: env_group_label(&group_id).to_string(),
        description: env_group_description(&group_id).to_string(),
        values,
        manual_keys,
    }
}

fn suggested_env_value(
    project: &ManagedProject,
    group: &str,
    key: &str,
    default_port: u16,
    project_slug: &str,
    project_root: &Path,
) -> Option<String> {
    let upper = key.to_ascii_uppercase();
    let localhost = "127.0.0.1";
    let app_url = format!("http://{localhost}:{default_port}");
    let project_name = project.name.to_ascii_lowercase();
    let uid = current_unix_id("-u").unwrap_or_else(|| "1000".to_string());
    let gid = current_unix_id("-g").unwrap_or_else(|| "1000".to_string());

    if project_name.contains("flowise") {
        match upper.as_str() {
            "DATABASE_TYPE" => return Some("sqlite".to_string()),
            "DATABASE_PATH" => {
                return Some(
                    project_root
                        .join(".portpilot")
                        .join("flowise.sqlite")
                        .to_string_lossy()
                        .to_string(),
                )
            }
            "SECRETKEY_STORAGE_TYPE" => return Some("local".to_string()),
            "SECRETKEY_PATH" => {
                return Some(
                    project_root
                        .join(".portpilot")
                        .join("flowise-secret.key")
                        .to_string_lossy()
                        .to_string(),
                )
            }
            "APP_URL" => return Some(app_url.clone()),
            "DATABASE_HOST" => return Some(localhost.to_string()),
            "DATABASE_PORT" => return Some("5432".to_string()),
            "DATABASE_NAME" => return Some(project_slug.to_string()),
            "DATABASE_USER" => return Some("postgres".to_string()),
            "DATABASE_PASSWORD" => return Some("postgres".to_string()),
            "DATABASE_SSL" => return Some("false".to_string()),
            "DATABASE_SSL_KEY_BASE64" => return Some("".to_string()),
            "FLOWISE_SECRETKEY_OVERWRITE" => {
                return Some(format!("{project_slug}-flowise-dev-secret"))
            }
            "DEBUG" => return Some("false".to_string()),
            "LOG_PATH" => {
                return Some(
                    project_root
                        .join(".portpilot")
                        .join("flowise.log")
                        .to_string_lossy()
                        .to_string(),
                )
            }
            "LOG_LEVEL" => return Some("info".to_string()),
            "LOG_SANITIZE_BODY_FIELDS" | "LOG_SANITIZE_HEADER_FIELDS" => {
                return Some("authorization,password,token".to_string())
            }
            "TOOL_FUNCTION_BUILTIN_DEP" | "ALLOW_BUILTIN_DEP" => {
                return Some("true".to_string())
            }
            "TOOL_FUNCTION_EXTERNAL_DEP" => return Some("false".to_string()),
            "STORAGE_TYPE" => return Some("local".to_string()),
            "SECRETKEY_AWS_ACCESS_KEY" | "SECRETKEY_AWS_SECRET_KEY" | "SECRETKEY_AWS_REGION"
            | "SECRETKEY_AWS_NAME" | "S3_STORAGE_BUCKET_NAME" | "S3_STORAGE_ACCESS_KEY_ID"
            | "S3_STORAGE_SECRET_ACCESS_KEY" | "S3_STORAGE_REGION" | "S3_ENDPOINT_URL"
            | "S3_FORCE_PATH_STYLE" | "GOOGLE_CLOUD_STORAGE_CREDENTIAL"
            | "GOOGLE_CLOUD_STORAGE_PROJ_ID" | "GOOGLE_CLOUD_STORAGE_BUCKET_NAME"
            | "GOOGLE_CLOUD_UNIFORM_BUCKET_ACCESS" | "AZURE_BLOB_STORAGE_CONNECTION_STRING"
            | "AZURE_BLOB_STORAGE_ACCOUNT_NAME" | "AZURE_BLOB_STORAGE_ACCOUNT_KEY"
            | "AZURE_BLOB_STORAGE_CONTAINER_NAME" => return Some("".to_string()),
            "BLOB_STORAGE_PATH" => {
                return Some(
                    project_root
                        .join(".portpilot")
                        .join("flowise-blob")
                        .to_string_lossy()
                        .to_string(),
                )
            }
            "NUMBER_OF_PROXIES" => return Some("0".to_string()),
            "CORS_ORIGINS" | "IFRAME_ORIGINS" => return Some(app_url.clone()),
            "FLOWISE_FILE_SIZE_LIMIT" => return Some("50mb".to_string()),
            "SHOW_COMMUNITY_NODES" => return Some("true".to_string()),
            "DISABLE_FLOWISE_TELEMETRY" | "OFFLINE" => return Some("true".to_string()),
            "DISABLED_NODES" | "MODEL_LIST_CONFIG_JSON" => return Some("".to_string()),
            "QUEUE_NAME" => return Some(format!("{project_slug}-queue")),
            "QUEUE_REDIS_EVENT_STREAM_MAX_LEN" => return Some("1000".to_string()),
            "WORKER_CONCURRENCY" => return Some("2".to_string()),
            "REMOVE_ON_AGE" => return Some("3600".to_string()),
            "REMOVE_ON_COUNT" => return Some("1000".to_string()),
            "REDIS_HOST" => return Some(localhost.to_string()),
            "REDIS_PORT" => return Some("6379".to_string()),
            "REDIS_USERNAME" | "REDIS_PASSWORD" | "REDIS_CERT" | "REDIS_KEY"
            | "REDIS_CA" => return Some("".to_string()),
            "REDIS_TLS" => return Some("false".to_string()),
            "REDIS_KEEP_ALIVE" => return Some("30000".to_string()),
            "ENABLE_BULLMQ_DASHBOARD" => return Some("false".to_string()),
            "CUSTOM_MCP_SECURITY_CHECK" | "CUSTOM_MCP_PROTOCOL" | "HTTP_DENY_LIST" => {
                return Some("".to_string())
            }
            "HTTP_SECURITY_CHECK" | "PATH_TRAVERSAL_SAFETY" => return Some("true".to_string()),
            "TRUST_PROXY" => return Some("false".to_string()),
            "JWT_AUTH_TOKEN_SECRET" => {
                return Some(format!("{project_slug}-jwt-auth-dev-secret"))
            }
            "JWT_REFRESH_TOKEN_SECRET" => {
                return Some(format!("{project_slug}-jwt-refresh-dev-secret"))
            }
            "JWT_ISSUER" => return Some("portpilot-local".to_string()),
            "JWT_AUDIENCE" => return Some("flowise-local".to_string()),
            "JWT_TOKEN_EXPIRY_IN_MINUTES" => return Some("60".to_string()),
            "JWT_REFRESH_TOKEN_EXPIRY_IN_MINUTES" => return Some("43200".to_string()),
            "EXPIRE_AUTH_TOKENS_ON_RESTART" => return Some("false".to_string()),
            "EXPRESS_SESSION_SECRET" => {
                return Some(format!("{project_slug}-express-session-dev"))
            }
            "PASSWORD_RESET_TOKEN_EXPIRY_IN_MINS" => return Some("15".to_string()),
            "PASSWORD_SALT_HASH_ROUNDS" => return Some("10".to_string()),
            "TOKEN_HASH_SECRET" => {
                return Some(format!("{project_slug}-token-hash-dev-secret"))
            }
            "SECURE_COOKIES" => return Some("false".to_string()),
            "SMTP_HOST" | "SMTP_USER" | "SMTP_PASSWORD" | "SENDER_EMAIL" | "LICENSE_URL"
            | "FLOWISE_EE_LICENSE_KEY" | "WORKSPACE_INVITE_TEMPLATE_PATH"
            | "POSTHOG_PUBLIC_API_KEY" | "GLOBAL_AGENT_HTTP_PROXY"
            | "GLOBAL_AGENT_HTTPS_PROXY" | "GLOBAL_AGENT_NO_PROXY" => {
                return Some("".to_string())
            }
            "SMTP_PORT" => return Some("587".to_string()),
            "SMTP_SECURE" => return Some("false".to_string()),
            "ALLOW_UNAUTHORIZED_CERTS" => return Some("false".to_string()),
            "INVITE_TOKEN_EXPIRY_IN_HOURS" => return Some("72".to_string()),
            "ENABLE_METRICS" => return Some("false".to_string()),
            "METRICS_PROVIDER" => return Some("console".to_string()),
            "METRICS_INCLUDE_NODE_METRICS" => return Some("false".to_string()),
            "METRICS_SERVICE_NAME" => return Some("flowise-local".to_string()),
            "METRICS_OPEN_TELEMETRY_METRIC_ENDPOINT"
            | "METRICS_OPEN_TELEMETRY_PROTOCOL" => return Some("".to_string()),
            "METRICS_OPEN_TELEMETRY_DEBUG" => return Some("false".to_string()),
            "MODE" => return Some("development".to_string()),
            _ => {}
        }
    }

    if project_name.contains("librechat") {
        match upper.as_str() {
            "PORT" => return Some(default_port.to_string()),
            "HOST" => return Some(localhost.to_string()),
            "UID" => return Some(uid.clone()),
            "GID" => return Some(gid.clone()),
            "RAG_PORT" => return Some("8001".to_string()),
            "MEILI_HOST" | "MEILISEARCH_HOST" | "MEILISEARCH_URL" => {
                return Some(format!("http://{localhost}:7700"))
            }
            "MEILI_MASTER_KEY" => return Some("masterkey".to_string()),
            _ => {}
        }
    }

    if project_name.contains("open-webui") {
        match upper.as_str() {
            "OLLAMA_BASE_URL" | "OLLAMA_API_BASE_URL" => {
                return Some(format!("http://{localhost}:11434"))
            }
            "WEBUI_AUTH" => return Some("False".to_string()),
            _ => {}
        }
    }

    if project_name.contains("anythingllm") || project_name.contains("anything-llm") {
        match upper.as_str() {
            "OLLAMA_BASE_URL" | "OLLAMA_API_BASE_URL" => {
                return Some(format!("http://{localhost}:11434"))
            }
            "SERVER_PORT" => return Some(default_port.to_string()),
            _ => {}
        }
    }

    match group {
        "app" | "frontend" | "server" => match upper.as_str() {
            "PORT" => Some(default_port.to_string()),
            "HOST" => Some(localhost.to_string()),
            "APP_URL" | "WEB_URL" | "SERVER_URL" | "API_URL" => Some(app_url),
            _ => None,
        },
        "database" => {
            if upper == "MONGO_URI" || upper == "MONGODB_URI" {
                return Some(format!("mongodb://{localhost}:27017/{project_slug}"));
            }
            if upper == "DATABASE_URL" || upper == "POSTGRES_URL" {
                return Some(format!(
                    "postgresql://postgres:postgres@{localhost}:5432/{project_slug}"
                ));
            }
            match upper.as_str() {
                "DATABASE_HOST" | "MONGO_HOST" | "MONGODB_HOST" | "POSTGRES_HOST" => {
                    Some(localhost.to_string())
                }
                "DATABASE_PORT" => Some(if uses_mongo(project) { "27017" } else { "5432" }.to_string()),
                "DATABASE_NAME" | "POSTGRES_DB" | "MONGO_DB" | "MONGODB_DB" => {
                    Some(project_slug.to_string())
                }
                "DATABASE_USER" | "POSTGRES_USER" => Some("postgres".to_string()),
                "DATABASE_PASSWORD" | "POSTGRES_PASSWORD" => Some("postgres".to_string()),
                "DATABASE_TYPE" => Some(if uses_mongo(project) {
                    "mongodb".to_string()
                } else {
                    "postgres".to_string()
                }),
                _ => None,
            }
        }
        "search" => match upper.as_str() {
            "MEILI_MASTER_KEY" => Some("masterkey".to_string()),
            "MEILI_HOST" | "MEILISEARCH_HOST" | "MEILI_URL" | "MEILISEARCH_URL" => {
                Some(format!("http://{localhost}:7700"))
            }
            "QDRANT_URL" => Some(format!("http://{localhost}:6333")),
            "WEAVIATE_URL" => Some(format!("http://{localhost}:8080")),
            "CHROMA_URL" | "VECTOR_DB_URL" => Some(format!("http://{localhost}:8000")),
            _ => None,
        },
        "rag" => match upper.as_str() {
            "RAG_PORT" => Some("8001".to_string()),
            "RAG_API_URL" | "RAG_URL" => Some(format!("http://{localhost}:8001")),
            _ => None,
        },
        "queue" => match upper.as_str() {
            "REDIS_URL" => Some(format!("redis://{localhost}:6379")),
            "REDIS_HOST" => Some(localhost.to_string()),
            "REDIS_PORT" => Some("6379".to_string()),
            "QUEUE_NAME" => Some(format!("{project_slug}-jobs")),
            _ => None,
        },
        "workspace" => match upper.as_str() {
            "OPENCLAW_CONFIG_DIR" => Some(
                project_root
                    .join(".portpilot")
                    .join("openclaw-config")
                    .to_string_lossy()
                    .to_string(),
            ),
            "OPENCLAW_WORKSPACE_DIR" => Some(
                project_root
                    .join(".portpilot")
                    .join("openclaw-workspace")
                    .to_string_lossy()
                    .to_string(),
            ),
            _ => None,
        },
        "gateway" => match upper.as_str() {
            "OPENCLAW_GATEWAY_PORT" => Some("18789".to_string()),
            "OPENCLAW_WEBCHAT_PORT" => Some("18790".to_string()),
            "OPENCLAW_GATEWAY_URL" => Some("http://127.0.0.1:18789".to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn current_unix_id(flag: &str) -> Option<String> {
    let output = Command::new("id").arg(flag).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn uses_mongo(project: &ManagedProject) -> bool {
    project
        .project_profile
        .required_services
        .iter()
        .any(|service| service.eq_ignore_ascii_case("mongodb"))
        || project.env_template.iter().any(|field| {
            let upper = field.key.to_ascii_uppercase();
            upper.contains("MONGO")
        })
}

fn env_group_label(group: &str) -> &'static str {
    match group {
        "app" => "App",
        "database" => "Database",
        "search" => "Search",
        "rag" => "RAG",
        "queue" => "Queue",
        "workspace" => "Workspace",
        "gateway" => "Gateway",
        "credentials" => "Credentials",
        "model-providers" => "Model Providers",
        "llm-provider" => "LLM Provider",
        "models" => "Models",
        "frontend" => "Frontend",
        "server" => "Server",
        _ => "Environment",
    }
}

fn env_group_description(group: &str) -> &'static str {
    match group {
        "app" => "Good local defaults for the primary app URL and port.",
        "database" => "Fill the most common localhost database values for this stack.",
        "search" => "Preset local search or vector service endpoints.",
        "rag" => "Preset the local RAG sidecar URL and port.",
        "queue" => "Preset the local queue/cache service values.",
        "workspace" => "Set repo-local working directories required by this stack.",
        "gateway" => "Preset localhost gateway and webchat entrypoints.",
        "credentials" => "Keys in this group usually still need real secrets.",
        "model-providers" => "Provider keys usually need manual input even in local mode.",
        "llm-provider" => "Provider-specific credentials still need manual input.",
        "models" => "Model paths and provider tokens often need manual input.",
        "frontend" => "Preset the local frontend URL and port.",
        "server" => "Preset the local server URL and port.",
        _ => "Local development defaults for this environment group.",
    }
}

fn local_service_start_command(service_name: &str) -> Option<&'static str> {
    match service_name.to_ascii_lowercase().as_str() {
        "ollama" => Some("ollama serve"),
        "mongodb" => Some("docker run -d --name mongodb -p 27017:27017 mongo:7"),
        "meilisearch" => {
            Some("docker run -d --name meilisearch -p 7700:7700 getmeili/meilisearch:v1.12")
        }
        "redis" => Some("docker run -d --name redis -p 6379:6379 redis:7"),
        "postgres" | "postgresql" | "db" => Some(
            "docker run -d --name postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:16",
        ),
        "qdrant" => Some("docker run -d --name qdrant -p 6333:6333 qdrant/qdrant"),
        "weaviate" => {
            Some("docker run -d --name weaviate -p 8080:8080 semitechnologies/weaviate:latest")
        }
        "chroma" | "vectordb" => {
            Some("docker run -d --name chroma -p 8000:8000 chromadb/chroma:latest")
        }
        _ => None,
    }
}

fn service_dependency_ready(service_name: &str, services: &[ComposeServiceStatus]) -> bool {
    if let Some(service) = services.iter().find(|service| service.name == service_name) {
        return service
            .state
            .as_deref()
            .map(|state| matches!(state, "running" | "healthy"))
            .unwrap_or(false);
    }

    known_local_service_port(service_name)
        .map(port_is_open)
        .unwrap_or(true)
}

fn runtime_services_for_project(project: &ManagedProject) -> Vec<ComposeServiceStatus> {
    let mut services = if project.has_docker_compose {
        collect_compose_services(project)
    } else {
        Vec::new()
    };

    for required in &project.project_profile.required_services {
        if services.iter().any(|service| service.name == *required) {
            continue;
        }

        if let Some(port) = known_local_service_port(required) {
            let running = port_is_open(port);
            services.push(ComposeServiceStatus {
                name: required.clone(),
                state: Some(if running { "running" } else { "missing" }.to_string()),
                health: Some("local dependency".to_string()),
                container_name: None,
                published_ports: vec![format!("127.0.0.1:{port}")],
            });
        }
    }

    services
}

fn missing_service_action_hint(project: &ManagedProject) -> Option<String> {
    let services = runtime_services_for_project(project);

    let missing = project
        .project_profile
        .required_services
        .iter()
        .find(|service| !service_dependency_ready(service, &services))?;

    if let Some(command) = local_service_start_command(missing) {
        if let Some(port) = known_local_service_port(missing) {
            return Some(format!(
                "Start {missing} first (`{command}` on localhost:{port}), then run the recommended entrypoint."
            ));
        }
        return Some(format!(
            "Start {missing} first (`{command}`), then run the recommended entrypoint."
        ));
    }

    if let Some(port) = known_local_service_port(missing) {
        return Some(format!(
            "Start {} on localhost:{port}, then run the recommended entrypoint.",
            missing
        ));
    }

    Some(format!(
        "Start the required service {} before launching this project.",
        missing
    ))
}

fn fixed_port_from_command(command: &str) -> Option<u16> {
    let patterns = [
        regex::Regex::new(r"--port\s+(\d{2,5})").expect("regex"),
        regex::Regex::new(r"-p\s+(\d{2,5})").expect("regex"),
        regex::Regex::new(r"PORT=(\d{2,5})").expect("regex"),
    ];

    for pattern in patterns {
        if let Some(capture) = pattern.captures(command) {
            if let Some(port) = capture
                .get(1)
                .and_then(|value| value.as_str().parse::<u16>().ok())
            {
                return Some(port);
            }
        }
    }

    None
}

fn fixed_port_from_project_config(project: &ManagedProject) -> Option<String> {
    let preferred_port = project.preferred_port?;
    let root = Path::new(&project.root_path);
    let candidates = ["config.yaml", "config.yml", "settings.json", "config/config.yaml"];
    let yaml_pattern = regex::Regex::new(r"(?m)^\s*port:\s*(-?\d{1,5})\s*$").expect("regex");
    let json_pattern =
        regex::Regex::new(r#""port"\s*:\s*(-?\d{1,5})"#).expect("regex");

    for relative in candidates {
        let path = root.join(relative);
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };

        let matched = yaml_pattern
            .captures(&contents)
            .and_then(|capture| capture.get(1))
            .and_then(|value| value.as_str().parse::<i32>().ok())
            .or_else(|| {
                json_pattern
                    .captures(&contents)
                    .and_then(|capture| capture.get(1))
                    .and_then(|value| value.as_str().parse::<i32>().ok())
            });

        let matches_preferred = matched == Some(i32::from(preferred_port));
        let is_known_default = matched == Some(-1)
            && preferred_port == 8000
            && project.name.eq_ignore_ascii_case("SillyTavern");

        if matches_preferred || is_known_default {
            return Some(relative.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        build_compose_requirements, build_env_group_presets, collect_local_service_presets,
        fixed_port_from_command, fixed_port_from_project_config, infer_run_phase,
        known_local_service_hint, known_local_service_port, local_service_start_command, now_iso,
        parse_compose_ps_json, parse_compose_service_names_from_file, project_port_conflicts,
    };
    use crate::core::models::{
        ActionExecution, ActionKind, ActionSource, EnvFieldType, EnvProfile, EnvTemplateField,
        ManagedProject, ProjectAction, ProjectKind, ProjectProfile, ProjectProfileKind, RunPhase,
        RuntimeKind, RuntimeStatus,
    };
    use std::{collections::HashMap, fs};

    #[test]
    fn parses_compose_ps_json_rows() {
        let contents = r#"{"Service":"vote","Name":"example-vote-1","State":"running","Health":"healthy","Publishers":[{"URL":"0.0.0.0","PublishedPort":8080,"TargetPort":80}]}
{"Service":"result","Name":"example-result-1","State":"running","Health":"","Publishers":[]}"#;

        let services = parse_compose_ps_json(contents).expect("expected compose json");
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].name, "vote");
        assert_eq!(services[0].state.as_deref(), Some("running"));
        assert_eq!(services[0].published_ports[0], "0.0.0.0:8080->80");
        assert_eq!(services[1].name, "result");
    }

    #[test]
    fn detects_fixed_port_from_command() {
        assert_eq!(
            fixed_port_from_command("npm start -- --port 8123"),
            Some(8123)
        );
        assert_eq!(
            fixed_port_from_command("PORT=3000 node server.js"),
            Some(3000)
        );
        assert_eq!(fixed_port_from_command("pnpm run dev"), None);
    }

    #[test]
    fn marks_waiting_for_service_before_waiting_for_port() {
        let execution = ActionExecution {
            id: "exec".to_string(),
            project_id: "project".to_string(),
            action_id: "run".to_string(),
            label: "Run".to_string(),
            command: "pnpm run dev".to_string(),
            status: crate::core::models::ExecutionStatus::Running,
            pid: None,
            port_hint: Some(3000),
            resolved_port: Some(3000),
            started_at: now_iso(),
            finished_at: None,
            last_log: None,
        };

        assert_eq!(
            infer_run_phase(&execution, &[], false),
            RunPhase::WaitingForService
        );
    }

    #[test]
    fn reports_non_reassignable_fixed_port_conflict() {
        let project = ManagedProject {
            id: "project".to_string(),
            name: "SillyTavern".to_string(),
            slug: "sillytavern".to_string(),
            root_path: "/tmp/silly".to_string(),
            git_url: None,
            project_kind: ProjectKind::Repo,
            runtime_kind: RuntimeKind::Node,
            status: RuntimeStatus::Stopped,
            last_error: None,
            preferred_port: Some(8000),
            resolved_port: None,
            route_subdomain_url: "http://silly.localhost:42300".to_string(),
            route_path_url: "http://gateway.localhost:42300/p/silly/".to_string(),
            has_docker_compose: false,
            has_dockerfile: false,
            detected_files: vec!["package.json".to_string()],
            primary_target_id: None,
            workspace_targets: Vec::new(),
            readme_hints: Vec::new(),
            project_profile: ProjectProfile {
                kind: ProjectProfileKind::AiUi,
                preferred_entrypoint: Some("run-start".to_string()),
                required_services: Vec::new(),
                required_env_groups: Vec::new(),
                known_ports: vec![8000],
                route_strategy: None,
                summary: None,
            },
            env_template: Vec::new(),
            env_profile: EnvProfile::default(),
            actions: vec![ProjectAction {
                id: "run-start".to_string(),
                label: "Start".to_string(),
                kind: ActionKind::Run,
                command: "npm start -- --port 8000".to_string(),
                workdir: "/tmp/silly".to_string(),
                env_profile: Some("default".to_string()),
                port_hint: Some(8000),
                healthcheck_url: None,
                source: ActionSource::Inferred,
            }],
            created_at: now_iso(),
            updated_at: now_iso(),
        };

        let conflicts = project_port_conflicts(&project);
        assert_eq!(conflicts.len(), 1);
        assert!(!conflicts[0].can_auto_reassign);
    }

    #[test]
    fn detects_fixed_port_from_project_config_file() {
        let root = std::env::temp_dir().join(format!("portpilot-config-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("config.yaml"), "port: 8000\n").unwrap();

        let project = ManagedProject {
            id: "project".to_string(),
            name: "SillyTavern".to_string(),
            slug: "sillytavern".to_string(),
            root_path: root.to_string_lossy().to_string(),
            git_url: None,
            project_kind: ProjectKind::Repo,
            runtime_kind: RuntimeKind::Node,
            status: RuntimeStatus::Stopped,
            last_error: None,
            preferred_port: Some(8000),
            resolved_port: None,
            route_subdomain_url: "http://silly.localhost:42300".to_string(),
            route_path_url: "http://gateway.localhost:42300/p/silly/".to_string(),
            has_docker_compose: false,
            has_dockerfile: false,
            detected_files: vec!["config.yaml".to_string()],
            primary_target_id: None,
            workspace_targets: Vec::new(),
            readme_hints: Vec::new(),
            project_profile: ProjectProfile::default(),
            env_template: Vec::new(),
            env_profile: EnvProfile::default(),
            actions: vec![ProjectAction {
                id: "run-start".to_string(),
                label: "Start".to_string(),
                kind: ActionKind::Run,
                command: "npm start".to_string(),
                workdir: root.to_string_lossy().to_string(),
                env_profile: Some("default".to_string()),
                port_hint: Some(8000),
                healthcheck_url: None,
                source: ActionSource::Inferred,
            }],
            created_at: now_iso(),
            updated_at: now_iso(),
        };

        assert_eq!(
            fixed_port_from_project_config(&project).as_deref(),
            Some("config.yaml")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn treats_sillytavern_default_config_port_as_non_reassignable() {
        let root = std::env::temp_dir().join(format!("portpilot-silly-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("config.yaml"), "dataRoot: ./data\nport: -1\n").unwrap();

        let project = ManagedProject {
            id: "project".to_string(),
            name: "SillyTavern".to_string(),
            slug: "sillytavern".to_string(),
            root_path: root.to_string_lossy().to_string(),
            git_url: None,
            project_kind: ProjectKind::Repo,
            runtime_kind: RuntimeKind::Node,
            status: RuntimeStatus::Stopped,
            last_error: None,
            preferred_port: Some(8000),
            resolved_port: None,
            route_subdomain_url: "http://silly.localhost:42300".to_string(),
            route_path_url: "http://gateway.localhost:42300/p/silly/".to_string(),
            has_docker_compose: false,
            has_dockerfile: false,
            detected_files: vec!["config.yaml".to_string()],
            primary_target_id: None,
            workspace_targets: Vec::new(),
            readme_hints: Vec::new(),
            project_profile: ProjectProfile::default(),
            env_template: Vec::new(),
            env_profile: EnvProfile::default(),
            actions: vec![ProjectAction {
                id: "run-start".to_string(),
                label: "Start".to_string(),
                kind: ActionKind::Run,
                command: "npm start".to_string(),
                workdir: root.to_string_lossy().to_string(),
                env_profile: Some("default".to_string()),
                port_hint: Some(8000),
                healthcheck_url: None,
                source: ActionSource::Inferred,
            }],
            created_at: now_iso(),
            updated_at: now_iso(),
        };

        assert_eq!(
            fixed_port_from_project_config(&project).as_deref(),
            Some("config.yaml")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn builds_workspace_env_group_preset_for_openclaw() {
        let root = std::env::temp_dir().join(format!("portpilot-openclaw-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();

        let project = ManagedProject {
            id: "project".to_string(),
            name: "OpenClaw".to_string(),
            slug: "openclaw".to_string(),
            root_path: root.to_string_lossy().to_string(),
            git_url: None,
            project_kind: ProjectKind::Repo,
            runtime_kind: RuntimeKind::Node,
            status: RuntimeStatus::Stopped,
            last_error: None,
            preferred_port: Some(18789),
            resolved_port: None,
            route_subdomain_url: String::new(),
            route_path_url: String::new(),
            has_docker_compose: true,
            has_dockerfile: false,
            detected_files: vec!["docker-compose.yml".to_string()],
            primary_target_id: None,
            workspace_targets: Vec::new(),
            readme_hints: Vec::new(),
            project_profile: ProjectProfile {
                kind: ProjectProfileKind::GatewayStack,
                preferred_entrypoint: None,
                required_services: vec!["openclaw-gateway".to_string()],
                required_env_groups: vec!["workspace".to_string(), "gateway".to_string(), "credentials".to_string()],
                known_ports: vec![18789, 18790],
                route_strategy: None,
                summary: None,
            },
            env_template: vec![
                EnvTemplateField {
                    key: "OPENCLAW_CONFIG_DIR".to_string(),
                    default_value: None,
                    description: None,
                    field_type: EnvFieldType::Text,
                },
                EnvTemplateField {
                    key: "OPENCLAW_WORKSPACE_DIR".to_string(),
                    default_value: None,
                    description: None,
                    field_type: EnvFieldType::Text,
                },
                EnvTemplateField {
                    key: "OPENAI_API_KEY".to_string(),
                    default_value: None,
                    description: None,
                    field_type: EnvFieldType::Secret,
                },
            ],
            env_profile: EnvProfile::default(),
            actions: Vec::new(),
            created_at: now_iso(),
            updated_at: now_iso(),
        };

        let presets = build_env_group_presets(&project);
        let workspace = presets.iter().find(|preset| preset.id == "workspace").unwrap();
        assert!(workspace.values.contains_key("OPENCLAW_CONFIG_DIR"));
        assert!(workspace.values.contains_key("OPENCLAW_WORKSPACE_DIR"));

        let credentials = presets.iter().find(|preset| preset.id == "credentials").unwrap();
        assert_eq!(credentials.manual_keys, vec!["OPENAI_API_KEY".to_string()]);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn builds_database_and_search_presets_for_librechat() {
        let project = ManagedProject {
            id: "project".to_string(),
            name: "LibreChat".to_string(),
            slug: "librechat".to_string(),
            root_path: "/tmp/librechat".to_string(),
            git_url: None,
            project_kind: ProjectKind::Compose,
            runtime_kind: RuntimeKind::Node,
            status: RuntimeStatus::Stopped,
            last_error: None,
            preferred_port: Some(3080),
            resolved_port: None,
            route_subdomain_url: String::new(),
            route_path_url: String::new(),
            has_docker_compose: true,
            has_dockerfile: false,
            detected_files: vec!["docker-compose.yml".to_string()],
            primary_target_id: None,
            workspace_targets: Vec::new(),
            readme_hints: Vec::new(),
            project_profile: ProjectProfile {
                kind: ProjectProfileKind::GatewayStack,
                preferred_entrypoint: None,
                required_services: vec!["mongodb".to_string(), "meilisearch".to_string(), "rag_api".to_string()],
                required_env_groups: vec!["database".to_string(), "search".to_string(), "rag".to_string()],
                known_ports: vec![3080, 8000],
                route_strategy: None,
                summary: None,
            },
            env_template: vec![
                EnvTemplateField {
                    key: "MONGO_URI".to_string(),
                    default_value: None,
                    description: None,
                    field_type: EnvFieldType::Text,
                },
                EnvTemplateField {
                    key: "MEILI_MASTER_KEY".to_string(),
                    default_value: None,
                    description: None,
                    field_type: EnvFieldType::Secret,
                },
                EnvTemplateField {
                    key: "RAG_PORT".to_string(),
                    default_value: None,
                    description: None,
                    field_type: EnvFieldType::Text,
                },
            ],
            env_profile: EnvProfile::default(),
            actions: Vec::new(),
            created_at: now_iso(),
            updated_at: now_iso(),
        };

        let presets = build_env_group_presets(&project);
        let database = presets.iter().find(|preset| preset.id == "database").unwrap();
        assert_eq!(
            database.values.get("MONGO_URI").map(String::as_str),
            Some("mongodb://127.0.0.1:27017/librechat")
        );
        let search = presets.iter().find(|preset| preset.id == "search").unwrap();
        assert_eq!(
            search.values.get("MEILI_MASTER_KEY").map(String::as_str),
            Some("masterkey")
        );
        let rag = presets.iter().find(|preset| preset.id == "rag").unwrap();
        assert_eq!(rag.values.get("RAG_PORT").map(String::as_str), Some("8001"));
    }

    #[test]
    fn collects_local_service_presets_from_projects() {
        let project = ManagedProject {
            id: "project".to_string(),
            name: "Open WebUI".to_string(),
            slug: "open-webui".to_string(),
            root_path: "/tmp/open-webui".to_string(),
            git_url: None,
            project_kind: ProjectKind::Repo,
            runtime_kind: RuntimeKind::Python,
            status: RuntimeStatus::Stopped,
            last_error: None,
            preferred_port: Some(8080),
            resolved_port: None,
            route_subdomain_url: "http://open-webui.localhost:42300".to_string(),
            route_path_url: "http://gateway.localhost:42300/p/open-webui/".to_string(),
            has_docker_compose: true,
            has_dockerfile: false,
            detected_files: vec!["docker-compose.yaml".to_string()],
            primary_target_id: None,
            workspace_targets: Vec::new(),
            readme_hints: Vec::new(),
            project_profile: ProjectProfile {
                kind: ProjectProfileKind::AiUi,
                preferred_entrypoint: None,
                required_services: vec!["ollama".to_string(), "open-webui".to_string()],
                required_env_groups: vec!["model-providers".to_string()],
                known_ports: vec![8080],
                route_strategy: None,
                summary: None,
            },
            env_template: Vec::new(),
            env_profile: EnvProfile::default(),
            actions: Vec::new(),
            created_at: now_iso(),
            updated_at: now_iso(),
        };

        let presets = collect_local_service_presets(&[project]);
        assert!(presets.len() >= 5);
        let ollama = presets.iter().find(|preset| preset.name == "ollama").unwrap();
        assert!(ollama
            .used_by_projects
            .iter()
            .any(|name| name == "Open WebUI"));
        assert_eq!(ollama.start_command.as_deref(), Some("ollama serve"));
    }

    #[test]
    fn parses_compose_services_from_file_without_running_docker() {
        let root = std::env::temp_dir().join(format!("portpilot-compose-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let compose_file = root.join("docker-compose.yml");
        fs::write(
            &compose_file,
            r#"
services:
  gateway:
    image: example/gateway
  webchat:
    image: example/webchat
  redis:
    image: redis:latest
"#,
        )
        .unwrap();

        let services = parse_compose_service_names_from_file(&compose_file);
        assert_eq!(services, vec!["gateway", "webchat", "redis"]);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn compose_requirements_include_missing_env_values() {
        let root = std::env::temp_dir().join(format!("portpilot-compose-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("docker-compose.yml"),
            r#"
services:
  gateway:
    image: example/gateway
"#,
        )
        .unwrap();

        let project = ManagedProject {
            id: "project".to_string(),
            name: "OpenClaw".to_string(),
            slug: "openclaw".to_string(),
            root_path: root.to_string_lossy().to_string(),
            git_url: None,
            project_kind: ProjectKind::Repo,
            runtime_kind: RuntimeKind::Node,
            status: RuntimeStatus::Stopped,
            last_error: None,
            preferred_port: Some(18789),
            resolved_port: None,
            route_subdomain_url: "http://openclaw.localhost:42300".to_string(),
            route_path_url: "http://gateway.localhost:42300/p/openclaw/".to_string(),
            has_docker_compose: true,
            has_dockerfile: false,
            detected_files: vec!["docker-compose.yml".to_string()],
            primary_target_id: None,
            workspace_targets: Vec::new(),
            readme_hints: Vec::new(),
            project_profile: ProjectProfile {
                kind: ProjectProfileKind::GatewayStack,
                preferred_entrypoint: Some("gateway".to_string()),
                required_services: vec!["gateway".to_string()],
                required_env_groups: vec!["workspace".to_string()],
                known_ports: vec![18789],
                route_strategy: None,
                summary: None,
            },
            env_template: vec![EnvTemplateField {
                key: "OPENCLAW_WORKSPACE_DIR".to_string(),
                default_value: None,
                description: Some("Detected from docker-compose configuration".to_string()),
                field_type: EnvFieldType::Text,
            }],
            env_profile: EnvProfile {
                values: HashMap::new(),
                raw_editor_text: None,
            },
            actions: Vec::new(),
            created_at: now_iso(),
            updated_at: now_iso(),
        };

        let requirements = build_compose_requirements(&project, &HashMap::new());
        assert!(requirements
            .iter()
            .any(|item| item.kind == "service" && item.name == "gateway"));
        assert!(requirements.iter().any(|item| item.kind == "env"
            && item.name == "OPENCLAW_WORKSPACE_DIR"
            && !item.ready));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn maps_common_local_services_to_ports() {
        assert_eq!(known_local_service_port("ollama"), Some(11434));
        assert_eq!(known_local_service_port("mongodb"), Some(27017));
        assert_eq!(known_local_service_port("meilisearch"), Some(7700));
        assert_eq!(known_local_service_port("redis"), Some(6379));
        assert_eq!(known_local_service_port("postgres"), Some(5432));
        assert_eq!(known_local_service_port("unknown-service"), None);
    }

    #[test]
    fn returns_hints_for_common_local_services() {
        assert!(known_local_service_hint("ollama").is_some());
        assert!(known_local_service_hint("meilisearch").is_some());
        assert!(known_local_service_hint("redis").is_some());
        assert!(known_local_service_hint("unknown-service").is_none());
    }

    #[test]
    fn returns_start_commands_for_common_local_services() {
        assert_eq!(local_service_start_command("ollama"), Some("ollama serve"));
        assert!(local_service_start_command("mongodb").is_some());
        assert!(local_service_start_command("redis").is_some());
        assert_eq!(local_service_start_command("rag_api"), None);
    }

    #[test]
    fn classifies_known_local_dependencies_as_local_services() {
        let project = ManagedProject {
            id: "project".to_string(),
            name: "Open WebUI".to_string(),
            slug: "open-webui".to_string(),
            root_path: "/tmp/open-webui".to_string(),
            git_url: None,
            project_kind: ProjectKind::Repo,
            runtime_kind: RuntimeKind::Python,
            status: RuntimeStatus::Stopped,
            last_error: None,
            preferred_port: Some(8080),
            resolved_port: None,
            route_subdomain_url: "http://open-webui.localhost:42300".to_string(),
            route_path_url: "http://gateway.localhost:42300/p/open-webui/".to_string(),
            has_docker_compose: false,
            has_dockerfile: false,
            detected_files: vec!["pyproject.toml".to_string()],
            primary_target_id: None,
            workspace_targets: Vec::new(),
            readme_hints: Vec::new(),
            project_profile: ProjectProfile {
                kind: ProjectProfileKind::AiUi,
                preferred_entrypoint: Some("run-python".to_string()),
                required_services: vec!["ollama".to_string()],
                required_env_groups: vec!["model-providers".to_string()],
                known_ports: vec![8080],
                route_strategy: None,
                summary: None,
            },
            env_template: Vec::new(),
            env_profile: EnvProfile::default(),
            actions: Vec::new(),
            created_at: now_iso(),
            updated_at: now_iso(),
        };

        let requirements = build_compose_requirements(&project, &HashMap::new());
        assert!(requirements
            .iter()
            .any(|item| item.kind == "local-service" && item.name == "ollama"));
    }
}

fn merged_env_values(project: &ManagedProject) -> HashMap<String, String> {
    let mut values = project.env_profile.values.clone();
    if let Some(raw) = &project.env_profile.raw_editor_text {
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || !trimmed.contains('=') {
                continue;
            }
            if let Some((key, value)) = trimmed.split_once('=') {
                values.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }
    values
}

fn tooling_check(project: &ManagedProject) -> DoctorCheck {
    let (missing, fix_command) = match project.runtime_kind {
        RuntimeKind::Node => node_tooling_check_requirements(project),
        RuntimeKind::Python => (
            if binary_exists("uv") || binary_exists("python3") || binary_exists("python") {
                Vec::new()
            } else {
                vec!["uv or python3".to_string()]
            },
            Some("brew install uv || brew install python".to_string()),
        ),
        RuntimeKind::Rust => (
            missing_binaries(&["cargo"]),
            Some("brew install rustup-init".to_string()),
        ),
        RuntimeKind::Go => (
            missing_binaries(&["go"]),
            Some("brew install go".to_string()),
        ),
        RuntimeKind::Compose => {
            let docker_ready = binary_exists("docker") || binary_exists("docker-compose");
            (
                if docker_ready {
                    Vec::new()
                } else {
                    vec!["docker".to_string()]
                },
                Some(
                    "Install Docker Desktop or Colima before running compose actions.".to_string(),
                ),
            )
        }
        RuntimeKind::Unknown => (Vec::new(), None),
    };

    if missing.is_empty() {
        return DoctorCheck {
            id: "tooling".to_string(),
            label: "Tooling".to_string(),
            status: DoctorStatus::Ok,
            summary: "Required local tooling is available.".to_string(),
            detail: None,
            fix_label: None,
            fix_command: None,
        };
    }

    DoctorCheck {
        id: "tooling".to_string(),
        label: "Tooling".to_string(),
        status: DoctorStatus::Error,
        summary: format!("Missing required tools: {}.", missing.join(", ")),
        detail: Some("Install the missing runtime before running this repository.".to_string()),
        fix_label: Some("Suggested fix".to_string()),
        fix_command,
    }
}

fn node_tooling_check_requirements(project: &ManagedProject) -> (Vec<String>, Option<String>) {
    let commands = project
        .actions
        .iter()
        .map(|action| action.command.as_str())
        .collect::<Vec<_>>();

    if commands.iter().any(|command| command.starts_with("bun ")) {
        return (
            missing_binaries(&["bun"]),
            Some("brew install oven-sh/bun/bun".to_string()),
        );
    }

    if commands.iter().any(|command| command.starts_with("pnpm ")) {
        return (
            missing_binaries(&["node", "pnpm"]),
            Some("brew install node && corepack enable pnpm".to_string()),
        );
    }

    if commands.iter().any(|command| command.starts_with("yarn ")) {
        return (
            missing_binaries(&["node", "yarn"]),
            Some("brew install node && corepack enable yarn".to_string()),
        );
    }

    (
        missing_binaries(&["node", "npm"]),
        Some("brew install node".to_string()),
    )
}

fn env_check(project: &ManagedProject, missing_env_keys: &[String]) -> DoctorCheck {
    if project.env_template.is_empty() {
        return DoctorCheck {
            id: "env".to_string(),
            label: "Environment".to_string(),
            status: DoctorStatus::Info,
            summary: "No .env template was detected for this repository.".to_string(),
            detail: Some(
                "Use the raw editor if this project expects undocumented environment variables."
                    .to_string(),
            ),
            fix_label: None,
            fix_command: None,
        };
    }

    if missing_env_keys.is_empty() {
        return DoctorCheck {
            id: "env".to_string(),
            label: "Environment".to_string(),
            status: DoctorStatus::Ok,
            summary: "Environment values are filled in for the detected template.".to_string(),
            detail: None,
            fix_label: None,
            fix_command: None,
        };
    }

    DoctorCheck {
        id: "env".to_string(),
        label: "Environment".to_string(),
        status: DoctorStatus::Warn,
        summary: format!("Missing {} environment value(s).", missing_env_keys.len()),
        detail: Some(missing_env_keys.join(", ")),
        fix_label: Some("Fill env values".to_string()),
        fix_command: None,
    }
}

fn install_check(project: &ManagedProject) -> DoctorCheck {
    let root = Path::new(&project.root_path);
    let install_hint = project
        .actions
        .iter()
        .find(|action| matches!(action.kind, ActionKind::Install))
        .map(|action| action.command.clone());

    let install_ready = match project.runtime_kind {
        RuntimeKind::Node => root.join("node_modules").exists() || install_hint.is_none(),
        RuntimeKind::Python => root.join(".venv").exists() || install_hint.is_none(),
        RuntimeKind::Rust | RuntimeKind::Go | RuntimeKind::Compose | RuntimeKind::Unknown => true,
    };

    if install_ready {
        return DoctorCheck {
            id: "install-state".to_string(),
            label: "Dependencies".to_string(),
            status: DoctorStatus::Ok,
            summary: "PortPilot did not detect a blocking dependency install gap.".to_string(),
            detail: install_hint.map(|command| format!("Primary install action: {command}")),
            fix_label: None,
            fix_command: None,
        };
    }

    DoctorCheck {
        id: "install-state".to_string(),
        label: "Dependencies".to_string(),
        status: DoctorStatus::Warn,
        summary: "This repo still looks like it needs an install step.".to_string(),
        detail: install_hint.clone(),
        fix_label: Some("Run install".to_string()),
        fix_command: install_hint,
    }
}

fn port_check(project: &ManagedProject, conflicts: &[DoctorPortConflict]) -> DoctorCheck {
    let port = project.resolved_port.or(project.preferred_port);
    let Some(port) = port else {
        return DoctorCheck {
            id: "port".to_string(),
            label: "Port".to_string(),
            status: DoctorStatus::Info,
            summary: "No preferred port was inferred yet.".to_string(),
            detail: Some(
                "PortPilot can still learn the actual route when the app boots.".to_string(),
            ),
            fix_label: None,
            fix_command: None,
        };
    };
    let conflict = conflicts.iter().find(|item| item.port == port);

    if matches!(project.status, RuntimeStatus::Running) {
        let reachable = std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().expect("socket addr"),
            Duration::from_millis(350),
        )
        .is_ok();
        return DoctorCheck {
            id: "port".to_string(),
            label: "Port".to_string(),
            status: if reachable {
                DoctorStatus::Ok
            } else {
                DoctorStatus::Warn
            },
            summary: if reachable {
                format!("Route is currently reachable on port {port}.")
            } else {
                format!("Port {port} is assigned, but the service did not answer immediately.")
            },
            detail: Some(project.route_path_url.clone()),
            fix_label: None,
            fix_command: None,
        };
    }

    let available = std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().expect("socket addr"),
        Duration::from_millis(250),
    )
    .is_err();

    DoctorCheck {
        id: "port".to_string(),
        label: "Port".to_string(),
        status: if available {
            DoctorStatus::Ok
        } else {
            DoctorStatus::Warn
        },
        summary: if available {
            format!("Preferred port {port} is currently free.")
        } else {
            format!("Preferred port {port} is already busy.")
        },
        detail: Some(if available {
            "PortPilot can start the primary run action without needing a reassignment.".to_string()
        } else if conflict.map(|item| item.can_auto_reassign).unwrap_or(false) {
            "PortPilot can auto-reassign the port when the project starts.".to_string()
        } else {
            "This command hardcodes its port. Free the existing process or change the command arguments first.".to_string()
        }),
        fix_label: None,
        fix_command: None,
    }
}

fn missing_binaries(binaries: &[&str]) -> Vec<String> {
    binaries
        .iter()
        .filter(|binary| !binary_exists(binary))
        .map(|binary| (*binary).to_string())
        .collect()
}

fn binary_exists(binary: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    #[cfg(target_os = "windows")]
    let extensions = std::env::var_os("PATHEXT")
        .map(|value| {
            value
                .to_string_lossy()
                .split(';')
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![".EXE".to_string(), ".CMD".to_string(), ".BAT".to_string()]);
    #[cfg(not(target_os = "windows"))]
    let extensions = vec![String::new()];

    std::env::split_paths(&paths).any(|path| {
        extensions.iter().any(|extension| {
            let candidate = path.join(format!("{binary}{extension}"));
            candidate.is_file()
        })
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error: tauri::Error| error.to_string())?;
            fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;

            let store = Arc::new(ProjectStore::load(data_dir.join("portpilot.db"))?);
            if store.list_workspace_roots()?.is_empty() {
                store.replace_workspace_roots(&[DEFAULT_WORKSPACE_ROOT.to_string()])?;
            }
            store.normalize_stale_runtime_state()?;
            let persisted_executions = store.list_executions()?;
            let runtime = Arc::new(RuntimeManager::new(
                data_dir.join("logs"),
                persisted_executions,
            )?);
            let (gateway_port, local_https_status) =
                tauri::async_runtime::block_on(gateway::start_gateway(
                    Arc::clone(&store),
                    data_dir.clone(),
                ))?;
            refresh_routes(&store, gateway_port)?;

            app.manage(AppState {
                store,
                runtime,
                gateway_port: Arc::new(Mutex::new(gateway_port)),
                local_https_status: Arc::new(Mutex::new(local_https_status)),
                data_dir,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_workspace_roots,
            set_workspace_roots,
            list_projects,
            scan_local_projects,
            register_local_project,
            list_project_actions,
            get_env_template,
            get_doctor_report,
            list_env_group_presets,
            get_project_recipe,
            write_project_recipe,
            save_env_profile,
            list_workspace_sessions,
            save_workspace_session,
            delete_workspace_session,
            list_action_executions,
            list_runtime_nodes,
            get_local_https_status,
            refresh_local_https_status,
            install_local_https,
            list_local_service_presets,
            inspect_local_service,
            start_local_service,
            restart_local_service,
            stop_local_service,
            get_project_logs,
            list_ports,
            list_routes,
            stop_action_execution,
            run_batch_action,
            stop_projects,
            restart_projects,
            restore_workspace_session,
            restart_project,
            run_project_action,
            import_repo_from_git,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PortPilot");
}
