use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;

use chrono::Utc;
use parking_lot::Mutex;
use portpicker::pick_unused_port;
use regex::Regex;
use tauri::{AppHandle, Emitter};

use crate::core::models::{
    ActionExecution, ActionKind, ExecutionStatus, LogEntry, ManagedProject, ProjectAction,
    RuntimeStatus,
};
use crate::storage::store::ProjectStore;

#[derive(Debug)]
pub struct RuntimeManager {
    executions: Arc<Mutex<HashMap<String, ActionExecution>>>,
    children: Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
    logs: Arc<Mutex<HashMap<String, Vec<LogEntry>>>>,
    stopped_ids: Arc<Mutex<HashSet<String>>>,
    log_dir: PathBuf,
}

impl RuntimeManager {
    pub fn new(log_dir: PathBuf, persisted_executions: Vec<ActionExecution>) -> Result<Self, String> {
        fs::create_dir_all(&log_dir).map_err(|error| error.to_string())?;
        let execution_map = persisted_executions
            .into_iter()
            .map(|execution| (execution.id.clone(), execution))
            .collect::<HashMap<_, _>>();
        Ok(Self {
            executions: Arc::new(Mutex::new(execution_map)),
            children: Arc::new(Mutex::new(HashMap::new())),
            logs: Arc::new(Mutex::new(HashMap::new())),
            stopped_ids: Arc::new(Mutex::new(HashSet::new())),
            log_dir,
        })
    }

    pub fn list_executions(&self) -> Vec<ActionExecution> {
        let mut items = self.executions.lock().values().cloned().collect::<Vec<_>>();
        items.sort_by(|left, right| right.started_at.cmp(&left.started_at));
        items
    }

    pub fn list_logs(&self, project_id: Option<&str>) -> Vec<LogEntry> {
        let executions = self.executions.lock().clone();
        let logs = self.logs.lock();
        let mut output = Vec::new();
        for (execution_id, entries) in logs.iter() {
            if let Some(project_id) = project_id {
                if executions
                    .get(execution_id)
                    .map(|execution| execution.project_id.as_str() != project_id)
                    .unwrap_or(true)
                {
                    continue;
                }
            }
            output.extend(entries.clone());
        }
        output.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
        output
    }

    pub fn run_action(
        &self,
        app: AppHandle,
        store: Arc<ProjectStore>,
        project: ManagedProject,
        action: ProjectAction,
    ) -> Result<ActionExecution, String> {
        let execution_id = format!("{}-{}", project.id, action.id);
        let assigned_port = if matches!(action.kind, ActionKind::Run) {
            fixed_port_from_command(&action.command).or_else(|| action.port_hint.map(select_port))
        } else {
            None
        };
        let command_text = prepare_command(&project, &action, assigned_port);

        let mut command = shell_command(&command_text);
        command
            .current_dir(&action.workdir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (key, value) in &project.env_profile.values {
            command.env(key, value);
        }
        if let Some(port) = assigned_port {
            command.env("PORT", port.to_string());
        }

        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let pid = Some(child.id());
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(child));

        let execution = ActionExecution {
            id: execution_id.clone(),
            project_id: project.id.clone(),
            action_id: action.id.clone(),
            label: action.label.clone(),
            command: command_text.clone(),
            status: ExecutionStatus::Running,
            pid,
            port_hint: action.port_hint,
            resolved_port: assigned_port,
            started_at: now_iso(),
            finished_at: None,
            last_log: None,
        };

        self.executions
            .lock()
            .insert(execution_id.clone(), execution.clone());
        store.upsert_execution(&execution)?;
        self.children
            .lock()
            .insert(execution_id.clone(), Arc::clone(&child));
        self.logs.lock().entry(execution_id.clone()).or_default();

        if matches!(action.kind, ActionKind::Run) {
            let last_error = if assigned_port != action.port_hint {
                Some(format!(
                    "Preferred port {:?} was busy. PortPilot reassigned the route to {:?}.",
                    action.port_hint, assigned_port
                ))
            } else {
                None
            };
            let _ = store.update(&project.id, |item| {
                item.status = RuntimeStatus::Running;
                item.resolved_port = assigned_port.or(item.preferred_port);
                item.last_error = last_error.clone();
                item.updated_at = now_iso();
            });
        }

        app.emit("action-started", &execution)
            .map_err(|error| error.to_string())?;

        if matches!(action.kind, ActionKind::Run) {
            let route_message = if let Some(port) = assigned_port {
                format!(
                    "Monitoring {} on port {} via {}.",
                    project.name, port, project.route_path_url
                )
            } else {
                format!("Started {} without a resolved port hint yet.", project.name)
            };
            self.emit_system_log(app.clone(), &execution_id, route_message);
            if let Some(fixed_port) = fixed_port_from_command(&action.command) {
                self.emit_system_log(
                    app.clone(),
                    &execution_id,
                    format!(
                        "Detected a fixed port ({fixed_port}) in the command. Port reassignment is disabled for this action."
                    ),
                );
            }
        }

        if let Some(stdout) = stdout {
            self.spawn_stream_reader(
                app.clone(),
                execution_id.clone(),
                "stdout",
                stdout,
            );
        }
        if let Some(stderr) = stderr {
            self.spawn_stream_reader(
                app.clone(),
                execution_id.clone(),
                "stderr",
                stderr,
            );
        }

        self.spawn_waiter(app, store, project.id, action.kind, execution_id, child);

        Ok(execution)
    }

    pub fn stop_execution(
        &self,
        app: AppHandle,
        store: Arc<ProjectStore>,
        execution_id: &str,
    ) -> Result<Option<ActionExecution>, String> {
        let Some(child) = self.children.lock().remove(execution_id) else {
            return Ok(None);
        };

        self.stopped_ids.lock().insert(execution_id.to_string());
        child.lock().kill().map_err(|error| error.to_string())?;

        let project_id = self
            .executions
            .lock()
            .get(execution_id)
            .map(|execution| execution.project_id.clone());

        if let Some(project_id) = project_id {
            let _ = store.update(&project_id, |item| {
                item.status = RuntimeStatus::Stopped;
                item.updated_at = now_iso();
            });
        }

        let execution = self
            .executions
            .lock()
            .get(execution_id)
            .cloned()
            .map(|mut execution| {
                execution.status = ExecutionStatus::Stopped;
                execution.finished_at = Some(now_iso());
                execution
            });

        if let Some(execution) = &execution {
            self.executions
                .lock()
                .insert(execution.id.clone(), execution.clone());
            store.upsert_execution(execution)?;
            self.emit_system_log(
                app.clone(),
                execution_id,
                format!("Stopped {}.", execution.label),
            );
            app.emit("action-finished", execution)
                .map_err(|error| error.to_string())?;
        }

        Ok(execution)
    }

    fn spawn_stream_reader<R>(
        &self,
        app: AppHandle,
        execution_id: String,
        stream_name: &str,
        reader: R,
    ) where
        R: std::io::Read + Send + 'static,
    {
        let stream_name = stream_name.to_string();
        let logs = self.logs.clone();
        let executions = self.executions.clone();
        let log_dir = self.log_dir.clone();
        thread::spawn(move || {
            for line in BufReader::new(reader).lines().map_while(Result::ok) {
                Self::push_log_entry(
                    &app,
                    &executions,
                    &logs,
                    &log_dir,
                    LogEntry {
                        execution_id: execution_id.clone(),
                        stream: stream_name.clone(),
                        message: line,
                        timestamp: now_iso(),
                    },
                );
            }
        });
    }

    fn spawn_waiter(
        &self,
        app: AppHandle,
        store: Arc<ProjectStore>,
        project_id: String,
        action_kind: ActionKind,
        execution_id: String,
        child: Arc<Mutex<Child>>,
    ) {
        let executions = self.executions.clone();
        let children = self.children.clone();
        let stopped_ids = self.stopped_ids.clone();
        let logs = self.logs.clone();
        let log_dir = self.log_dir.clone();

        thread::spawn(move || {
            let exit_status = child.lock().wait();
            children.lock().remove(&execution_id);
            let was_stopped = stopped_ids.lock().remove(&execution_id);
            let status = match exit_status {
                Ok(status) if was_stopped => ExecutionStatus::Stopped,
                Ok(status) if status.success() => ExecutionStatus::Success,
                Ok(_) | Err(_) => ExecutionStatus::Failed,
            };

            let updated = {
                let mut guard = executions.lock();
                let Some(current) = guard.get_mut(&execution_id) else {
                    return;
                };
                current.status = status.clone();
                current.finished_at = Some(now_iso());
                current.clone()
            };
            let _ = store.upsert_execution(&updated);
            let result_message = match status {
                ExecutionStatus::Success => format!("{} finished successfully.", updated.label),
                ExecutionStatus::Stopped => format!("{} was stopped.", updated.label),
                ExecutionStatus::Failed => format!("{} exited unexpectedly.", updated.label),
                ExecutionStatus::Running => return,
            };
            Self::push_log_entry(
                &app,
                &executions,
                &logs,
                &log_dir,
                LogEntry {
                    execution_id: execution_id.clone(),
                    stream: "system".to_string(),
                    message: result_message,
                    timestamp: now_iso(),
                },
            );

            if matches!(action_kind, ActionKind::Run) {
                let _ = store.update(&project_id, |item| {
                    item.status = if matches!(status, ExecutionStatus::Failed) {
                        RuntimeStatus::Error
                    } else {
                        RuntimeStatus::Stopped
                    };
                    if matches!(status, ExecutionStatus::Failed) {
                        item.last_error = Some("The run action exited unexpectedly.".to_string());
                    }
                    item.updated_at = now_iso();
                });
            }

            let _ = app.emit("action-finished", &updated);
        });
    }

    fn emit_system_log(&self, app: AppHandle, execution_id: &str, message: String) {
        Self::push_log_entry(
            &app,
            &self.executions,
            &self.logs,
            &self.log_dir,
            LogEntry {
                execution_id: execution_id.to_string(),
                stream: "system".to_string(),
                message,
                timestamp: now_iso(),
            },
        );
    }

    fn push_log_entry(
        app: &AppHandle,
        executions: &Arc<Mutex<HashMap<String, ActionExecution>>>,
        logs: &Arc<Mutex<HashMap<String, Vec<LogEntry>>>>,
        log_dir: &PathBuf,
        entry: LogEntry,
    ) {
        if let Some(execution) = executions.lock().get_mut(&entry.execution_id) {
            execution.last_log = Some(entry.message.clone());
        }
        logs.lock()
            .entry(entry.execution_id.clone())
            .or_default()
            .push(entry.clone());

        let log_file = log_dir.join(format!("{}.log", entry.execution_id));
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_file) {
            let _ = writeln!(file, "[{}] [{}] {}", entry.timestamp, entry.stream, entry.message);
        }

        let _ = app.emit("action-log", &entry);
    }
}

fn shell_command(command: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc").arg(command);
        cmd
    }
}

fn prepare_command(project: &ManagedProject, action: &ProjectAction, assigned_port: Option<u16>) -> String {
    if fixed_port_from_command(&action.command).is_some() {
        return action.command.clone();
    }

    let mut command = action.command.clone();
    if let Some(port) = assigned_port {
        if matches!(project.runtime_kind, crate::core::models::RuntimeKind::Node)
            && action.command.starts_with("npm run dev")
        {
            command = format!("{} -- --host 127.0.0.1 --port {}", action.command, port);
        } else if action.command.starts_with("npm run preview") {
            command = format!("{} -- --host 127.0.0.1 --port {}", action.command, port);
        }
    }
    command
}

fn select_port(preferred_port: u16) -> u16 {
    if port_is_free(preferred_port) {
        return preferred_port;
    }
    pick_unused_port().unwrap_or(preferred_port)
}

fn port_is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn fixed_port_from_command(command: &str) -> Option<u16> {
    let patterns = [
        Regex::new(r"--port\s+(\d{2,5})").expect("regex"),
        Regex::new(r"-p\s+(\d{2,5})").expect("regex"),
        Regex::new(r"PORT=(\d{2,5})").expect("regex"),
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
