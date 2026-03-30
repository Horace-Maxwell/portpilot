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
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::core::inference::{
    infer_project_from_path, now_iso, parse_env_template, repo_name_from_git_url,
    scan_workspace_roots, slugify, DEFAULT_WORKSPACE_ROOT,
};
use crate::core::models::{
    ActionExecution, ActionKind, BatchActionItemResult, BatchActionResult, BatchItemStatus,
    DoctorCheck, DoctorReport, DoctorStatus, EnvProfile, EnvTemplateField, ImportedRepo,
    LogEntry, ManagedProject, PortLease, ProjectAction, RouteBinding, RuntimeKind,
    RuntimeStatus, WorkspaceSession, WorkspaceSessionProject,
};
use crate::runtime::manager::RuntimeManager;
use crate::storage::store::ProjectStore;

#[derive(Clone)]
struct AppState {
    store: Arc<ProjectStore>,
    runtime: Arc<RuntimeManager>,
    gateway_port: Arc<Mutex<u16>>,
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
fn set_workspace_roots(state: State<'_, AppState>, roots: Vec<String>) -> Result<Vec<String>, String> {
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
fn list_project_actions(state: State<'_, AppState>, project_id: String) -> Result<Vec<ProjectAction>, String> {
    let project = fresh_project(&state.store, &project_id, *state.gateway_port.lock())?;
    Ok(project.actions)
}

#[tauri::command]
fn get_env_template(state: State<'_, AppState>, project_id: String) -> Result<Vec<EnvTemplateField>, String> {
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
    Ok(build_doctor_report(&project))
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
        .ok_or_else(|| "PortPilot cloned the repo, but could not infer a supported project.".to_string())?;
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

fn fresh_project(store: &Arc<ProjectStore>, project_id: &str, gateway_port: u16) -> Result<ManagedProject, String> {
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

fn build_doctor_report(project: &ManagedProject) -> DoctorReport {
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

    let env_values = merged_env_values(project);
    let missing_env_keys = project
        .env_template
        .iter()
        .filter_map(|field| {
            let value = env_values.get(&field.key).map(|value| value.trim()).unwrap_or("");
            if value.is_empty() {
                Some(field.key.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut checks = Vec::new();
    checks.push(tooling_check(project));
    checks.push(env_check(project, &missing_env_keys));
    checks.push(install_check(project));
    checks.push(port_check(project));
    if !project.workspace_targets.is_empty() {
        checks.push(DoctorCheck {
            id: "workspace-targets".to_string(),
            label: "Monorepo Targets".to_string(),
            status: DoctorStatus::Info,
            summary: format!(
                "Detected {} runnable app target{} inside this repo.",
                project.workspace_targets.len(),
                if project.workspace_targets.len() == 1 { "" } else { "s" }
            ),
            detail: Some(
                project
                    .workspace_targets
                    .iter()
                    .map(|target| format!("{} ({})", target.name, target.relative_path))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
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

    DoctorReport {
        project_id: project.id.clone(),
        generated_at: now_iso(),
        missing_env_keys,
        install_action_id,
        run_action_id,
        open_action_id,
        checks,
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
        RuntimeKind::Node => (
            missing_binaries(&["node", "npm"]),
            Some("brew install node".to_string()),
        ),
        RuntimeKind::Python => (
            if binary_exists("uv") || binary_exists("python3") || binary_exists("python") {
                Vec::new()
            } else {
                vec!["uv or python3".to_string()]
            },
            Some("brew install uv || brew install python".to_string()),
        ),
        RuntimeKind::Rust => (missing_binaries(&["cargo"]), Some("brew install rustup-init".to_string())),
        RuntimeKind::Go => (missing_binaries(&["go"]), Some("brew install go".to_string())),
        RuntimeKind::Compose => {
            let docker_ready = binary_exists("docker") || binary_exists("docker-compose");
            (
                if docker_ready { Vec::new() } else { vec!["docker".to_string()] },
                Some("Install Docker Desktop or Colima before running compose actions.".to_string()),
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

fn env_check(project: &ManagedProject, missing_env_keys: &[String]) -> DoctorCheck {
    if project.env_template.is_empty() {
        return DoctorCheck {
            id: "env".to_string(),
            label: "Environment".to_string(),
            status: DoctorStatus::Info,
            summary: "No .env template was detected for this repository.".to_string(),
            detail: Some("Use the raw editor if this project expects undocumented environment variables.".to_string()),
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

fn port_check(project: &ManagedProject) -> DoctorCheck {
    let port = project.resolved_port.or(project.preferred_port);
    let Some(port) = port else {
        return DoctorCheck {
            id: "port".to_string(),
            label: "Port".to_string(),
            status: DoctorStatus::Info,
            summary: "No preferred port was inferred yet.".to_string(),
            detail: Some("PortPilot can still learn the actual route when the app boots.".to_string()),
            fix_label: None,
            fix_command: None,
        };
    };

    if matches!(project.status, RuntimeStatus::Running) {
        let reachable = std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().expect("socket addr"),
            Duration::from_millis(350),
        )
        .is_ok();
        return DoctorCheck {
            id: "port".to_string(),
            label: "Port".to_string(),
            status: if reachable { DoctorStatus::Ok } else { DoctorStatus::Warn },
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
        } else {
            "PortPilot can auto-reassign the port, or you can free the existing process first.".to_string()
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
            let runtime = Arc::new(RuntimeManager::new(data_dir.join("logs"), persisted_executions)?);
            let gateway_port = tauri::async_runtime::block_on(gateway::start_gateway(Arc::clone(&store)))?;
            refresh_routes(&store, gateway_port)?;

            app.manage(AppState {
                store,
                runtime,
                gateway_port: Arc::new(Mutex::new(gateway_port)),
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
            save_env_profile,
            list_workspace_sessions,
            save_workspace_session,
            delete_workspace_session,
            list_action_executions,
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
