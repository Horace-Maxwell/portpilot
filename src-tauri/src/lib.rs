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

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::core::inference::{
    infer_project_from_path, now_iso, parse_env_template, repo_name_from_git_url,
    scan_workspace_roots, slugify, DEFAULT_WORKSPACE_ROOT,
};
use crate::core::models::{
    ActionExecution, EnvProfile, EnvTemplateField, ImportedRepo, LogEntry, ManagedProject,
    PortLease, ProjectAction, RouteBinding,
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
    state.store.list()
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
    let project = state
        .store
        .get(&project_id)?
        .ok_or_else(|| "Project not found.".to_string())?;
    Ok(project.actions)
}

#[tauri::command]
fn get_env_template(state: State<'_, AppState>, project_id: String) -> Result<Vec<EnvTemplateField>, String> {
    let project = state
        .store
        .get(&project_id)?
        .ok_or_else(|| "Project not found.".to_string())?;
    if !project.env_template.is_empty() {
        return Ok(project.env_template);
    }
    Ok(parse_env_template(Path::new(&project.root_path)))
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
            save_env_profile,
            list_action_executions,
            get_project_logs,
            list_ports,
            list_routes,
            stop_action_execution,
            restart_project,
            run_project_action,
            import_repo_from_git,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PortPilot");
}
