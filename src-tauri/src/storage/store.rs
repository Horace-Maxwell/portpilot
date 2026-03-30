use std::fs;
use std::path::PathBuf;

use rusqlite::{params, Connection};

use crate::core::models::{
    ActionExecution, ExecutionStatus, ManagedProject, RuntimeStatus, WorkspaceSession,
};

#[derive(Debug)]
pub struct ProjectStore {
    path: PathBuf,
}

impl ProjectStore {
    pub fn load(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let store = Self { path };
        let connection = store.connect()?;
        store.init_schema(&connection)?;
        Ok(store)
    }

    pub fn list_workspace_roots(&self) -> Result<Vec<String>, String> {
        let connection = self.connect()?;
        let mut stmt = connection
            .prepare("SELECT root_path FROM workspace_roots ORDER BY position ASC")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut roots = Vec::new();
        for row in rows {
            roots.push(row.map_err(|error| error.to_string())?);
        }
        Ok(roots)
    }

    pub fn replace_workspace_roots(&self, roots: &[String]) -> Result<(), String> {
        let mut connection = self.connect()?;
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM workspace_roots", [])
            .map_err(|error| error.to_string())?;
        for (index, root) in roots.iter().enumerate() {
            tx.execute(
                "INSERT INTO workspace_roots (position, root_path) VALUES (?1, ?2)",
                params![index as i64, root],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn list(&self) -> Result<Vec<ManagedProject>, String> {
        let connection = self.connect()?;
        let mut stmt = connection
            .prepare("SELECT payload FROM projects ORDER BY name ASC")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut projects = Vec::new();
        for row in rows {
            let payload = row.map_err(|error| error.to_string())?;
            let project = serde_json::from_str::<ManagedProject>(&payload)
                .map_err(|error| error.to_string())?;
            projects.push(project);
        }
        Ok(projects)
    }

    pub fn get(&self, id: &str) -> Result<Option<ManagedProject>, String> {
        let connection = self.connect()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload FROM projects WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;

        payload
            .map(|value| {
                serde_json::from_str::<ManagedProject>(&value).map_err(|error| error.to_string())
            })
            .transpose()
    }

    pub fn upsert(&self, project: ManagedProject) -> Result<(), String> {
        let connection = self.connect()?;
        let payload = serde_json::to_string(&project).map_err(|error| error.to_string())?;
        connection
            .execute(
                r#"
                INSERT INTO projects (
                  id, name, slug, root_path, git_url, status, preferred_port, resolved_port, updated_at, payload
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                ON CONFLICT(id) DO UPDATE SET
                  name = excluded.name,
                  slug = excluded.slug,
                  root_path = excluded.root_path,
                  git_url = excluded.git_url,
                  status = excluded.status,
                  preferred_port = excluded.preferred_port,
                  resolved_port = excluded.resolved_port,
                  updated_at = excluded.updated_at,
                  payload = excluded.payload
                "#,
                params![
                    project.id,
                    project.name,
                    project.slug,
                    project.root_path,
                    project.git_url,
                    serde_json::to_string(&project.status).map_err(|error| error.to_string())?,
                    project.preferred_port,
                    project.resolved_port,
                    project.updated_at,
                    payload,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn update<F>(&self, id: &str, updater: F) -> Result<Option<ManagedProject>, String>
    where
        F: FnOnce(&mut ManagedProject),
    {
        let Some(mut project) = self.get(id)? else {
            return Ok(None);
        };
        updater(&mut project);
        self.upsert(project.clone())?;
        Ok(Some(project))
    }

    pub fn list_executions(&self) -> Result<Vec<ActionExecution>, String> {
        let connection = self.connect()?;
        let mut stmt = connection
            .prepare("SELECT payload FROM executions ORDER BY started_at DESC")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut executions = Vec::new();
        for row in rows {
            let payload = row.map_err(|error| error.to_string())?;
            executions.push(
                serde_json::from_str::<ActionExecution>(&payload)
                    .map_err(|error| error.to_string())?,
            );
        }
        Ok(executions)
    }

    pub fn upsert_execution(&self, execution: &ActionExecution) -> Result<(), String> {
        let connection = self.connect()?;
        let payload = serde_json::to_string(execution).map_err(|error| error.to_string())?;
        connection
            .execute(
                r#"
                INSERT INTO executions (
                  id, project_id, action_id, label, status, started_at, finished_at, payload
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(id) DO UPDATE SET
                  project_id = excluded.project_id,
                  action_id = excluded.action_id,
                  label = excluded.label,
                  status = excluded.status,
                  started_at = excluded.started_at,
                  finished_at = excluded.finished_at,
                  payload = excluded.payload
                "#,
                params![
                    execution.id,
                    execution.project_id,
                    execution.action_id,
                    execution.label,
                    serde_json::to_string(&execution.status).map_err(|error| error.to_string())?,
                    execution.started_at,
                    execution.finished_at,
                    payload,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn normalize_stale_runtime_state(&self) -> Result<(), String> {
        for project in self.list()? {
            if matches!(
                project.status,
                RuntimeStatus::Running | RuntimeStatus::Starting
            ) {
                let _ = self.update(&project.id, |item| {
                    item.status = RuntimeStatus::Stopped;
                    item.updated_at = chrono::Utc::now().to_rfc3339();
                })?;
            }
        }

        for mut execution in self.list_executions()? {
            if matches!(execution.status, ExecutionStatus::Running) {
                execution.status = ExecutionStatus::Stopped;
                execution.finished_at = Some(chrono::Utc::now().to_rfc3339());
                self.upsert_execution(&execution)?;
            }
        }

        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<WorkspaceSession>, String> {
        let connection = self.connect()?;
        let mut stmt = connection
            .prepare("SELECT payload FROM workspace_sessions ORDER BY updated_at DESC")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut sessions = Vec::new();
        for row in rows {
            let payload = row.map_err(|error| error.to_string())?;
            sessions.push(
                serde_json::from_str::<WorkspaceSession>(&payload)
                    .map_err(|error| error.to_string())?,
            );
        }
        Ok(sessions)
    }

    pub fn get_session(&self, id: &str) -> Result<Option<WorkspaceSession>, String> {
        let connection = self.connect()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload FROM workspace_sessions WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;

        payload
            .map(|value| {
                serde_json::from_str::<WorkspaceSession>(&value).map_err(|error| error.to_string())
            })
            .transpose()
    }

    pub fn upsert_session(&self, session: &WorkspaceSession) -> Result<(), String> {
        let connection = self.connect()?;
        let payload = serde_json::to_string(session).map_err(|error| error.to_string())?;
        connection
            .execute(
                r#"
                INSERT INTO workspace_sessions (id, name, updated_at, payload)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(id) DO UPDATE SET
                  name = excluded.name,
                  updated_at = excluded.updated_at,
                  payload = excluded.payload
                "#,
                params![session.id, session.name, session.updated_at, payload],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_session(&self, id: &str) -> Result<(), String> {
        let connection = self.connect()?;
        connection
            .execute("DELETE FROM workspace_sessions WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn connect(&self) -> Result<Connection, String> {
        Connection::open(&self.path).map_err(|error| error.to_string())
    }

    fn init_schema(&self, connection: &Connection) -> Result<(), String> {
        connection
            .execute_batch(
                r#"
                PRAGMA journal_mode = WAL;
                PRAGMA foreign_keys = ON;

                CREATE TABLE IF NOT EXISTS workspace_roots (
                  position INTEGER PRIMARY KEY,
                  root_path TEXT NOT NULL UNIQUE
                );

                CREATE TABLE IF NOT EXISTS projects (
                  id TEXT PRIMARY KEY,
                  name TEXT NOT NULL,
                  slug TEXT NOT NULL,
                  root_path TEXT NOT NULL,
                  git_url TEXT,
                  status TEXT NOT NULL,
                  preferred_port INTEGER,
                  resolved_port INTEGER,
                  updated_at TEXT NOT NULL,
                  payload TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS executions (
                  id TEXT PRIMARY KEY,
                  project_id TEXT NOT NULL,
                  action_id TEXT NOT NULL,
                  label TEXT NOT NULL,
                  status TEXT NOT NULL,
                  started_at TEXT NOT NULL,
                  finished_at TEXT,
                  payload TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS workspace_sessions (
                  id TEXT PRIMARY KEY,
                  name TEXT NOT NULL,
                  updated_at TEXT NOT NULL,
                  payload TEXT NOT NULL
                );
                "#,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

trait OptionalRow<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use uuid::Uuid;

    use super::ProjectStore;
    use crate::core::models::{WorkspaceSession, WorkspaceSessionProject};

    #[test]
    fn stores_and_deletes_workspace_sessions() {
        let root = std::env::temp_dir().join(format!("portpilot-store-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let store = ProjectStore::load(root.join("portpilot.db")).unwrap();

        let session = WorkspaceSession {
            id: "session-a".to_string(),
            name: "Morning".to_string(),
            projects: vec![WorkspaceSessionProject {
                project_id: "project-a".to_string(),
                project_name: "Crucix".to_string(),
                auto_start: true,
                run_action_id: Some("run-dev".to_string()),
                env_profile_name: Some("default".to_string()),
            }],
            created_at: "2026-03-29T00:00:00Z".to_string(),
            updated_at: "2026-03-29T00:00:00Z".to_string(),
        };

        store.upsert_session(&session).unwrap();
        let listed = store.list_sessions().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Morning");

        store.delete_session("session-a").unwrap();
        assert!(store.list_sessions().unwrap().is_empty());

        let _ = fs::remove_dir_all(root);
    }
}
