use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use serde_json::Value;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::core::models::{
    ActionKind, ActionSource, EnvFieldType, EnvProfile, EnvTemplateField, ImportedRepo,
    ManagedProject, ProjectAction, ProjectKind, RuntimeKind, RuntimeStatus,
};

pub const DEFAULT_WORKSPACE_ROOT: &str = "/Users/horacedong/Desktop/Github";

pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

pub fn slugify(value: &str) -> String {
    let mut output = String::new();
    let mut last_dash = false;
    for ch in value.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            output.push(ch);
            last_dash = false;
        } else if !last_dash {
            output.push('-');
            last_dash = true;
        }
    }
    output.trim_matches('-').to_string()
}

pub fn repo_name_from_git_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("Repository URL is empty.".to_string());
    }

    let name = trimmed
        .rsplit('/')
        .next()
        .ok_or_else(|| "Could not derive repository name from URL.".to_string())?
        .trim_end_matches(".git");

    if name.is_empty() {
        return Err("Could not derive repository name from URL.".to_string());
    }

    Ok(name.to_string())
}

pub fn parse_env_template(root: &Path) -> Vec<EnvTemplateField> {
    for candidate in [".env.example", ".env.local.example"] {
        let path = root.join(candidate);
        if path.exists() {
            if let Ok(contents) = fs::read_to_string(&path) {
                return parse_env_template_contents(&contents);
            }
        }
    }

    Vec::new()
}

pub fn parse_env_template_contents(contents: &str) -> Vec<EnvTemplateField> {
    let mut fields = Vec::new();
    let mut comment_buffer = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            comment_buffer.clear();
            continue;
        }

        if let Some(comment) = trimmed.strip_prefix('#') {
            let clean = comment.trim();
            if !clean.is_empty() {
                comment_buffer.push(clean.to_string());
            }
            continue;
        }

        if trimmed.starts_with("export ") || trimmed.contains('=') {
            let normalized = trimmed.strip_prefix("export ").unwrap_or(trimmed);
            let Some((key, value)) = normalized.split_once('=') else {
                continue;
            };

            let key = key.trim();
            if key.is_empty() {
                continue;
            }

            let default_value = value.trim().trim_matches('"').trim_matches('\'').to_string();
            let description = if comment_buffer.is_empty() {
                None
            } else {
                Some(comment_buffer.join(" "))
            };

            let upper = key.to_ascii_uppercase();
            let field_type = if upper.contains("TOKEN")
                || upper.contains("SECRET")
                || upper.contains("PASSWORD")
                || upper.contains("KEY")
            {
                EnvFieldType::Secret
            } else if matches!(default_value.as_str(), "true" | "false" | "1" | "0") {
                EnvFieldType::Boolean
            } else if default_value.contains("\\n") || default_value.contains('{') || default_value.contains('[') {
                EnvFieldType::Multiline
            } else {
                EnvFieldType::Text
            };

            fields.push(EnvTemplateField {
                key: key.to_string(),
                default_value: if default_value.is_empty() {
                    None
                } else {
                    Some(default_value)
                },
                description,
                field_type,
            });
            comment_buffer.clear();
        }
    }

    fields
}

pub fn scan_workspace_roots(roots: &[String], gateway_port: u16) -> Vec<ImportedRepo> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    for root in roots {
        let root_path = Path::new(root);
        if !root_path.exists() {
            continue;
        }

        for entry in WalkDir::new(root_path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !should_skip_dir(entry.path()))
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !entry.file_type().is_file() {
                continue;
            }

            let file_name = entry.file_name().to_string_lossy();
            if !is_manifest_name(&file_name) {
                continue;
            }

            let Some(project_root) = path.parent() else {
                continue;
            };

            let normalized = project_root.to_string_lossy().to_string();
            if !seen.insert(normalized.clone()) {
                continue;
            }

            if let Some(project) = infer_project_from_path(project_root, None, gateway_port) {
                candidates.push(ImportedRepo {
                    name: project.name,
                    root_path: normalized,
                    git_url: project.git_url,
                    project_kind: project.project_kind,
                    runtime_kind: project.runtime_kind,
                    suggested_port: project.preferred_port,
                    has_env_template: !project.env_template.is_empty(),
                    has_docker_compose: project.has_docker_compose,
                    has_dockerfile: project.has_dockerfile,
                    detected_files: project.detected_files,
                    action_count: project.actions.len(),
                });
            }
        }
    }

    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    candidates
}

pub fn infer_project_from_path(
    root: &Path,
    git_url: Option<String>,
    gateway_port: u16,
) -> Option<ManagedProject> {
    let package_json = root.join("package.json");
    let pyproject = root.join("pyproject.toml");
    let cargo_toml = root.join("Cargo.toml");
    let go_mod = root.join("go.mod");
    let compose_file = first_existing(
        root,
        &["docker-compose.yml", "docker-compose.yaml", "compose.yaml"],
    );
    let dockerfile = root.join("Dockerfile");

    let runtime_kind = if package_json.exists() {
        RuntimeKind::Node
    } else if pyproject.exists() {
        RuntimeKind::Python
    } else if cargo_toml.exists() {
        RuntimeKind::Rust
    } else if go_mod.exists() {
        RuntimeKind::Go
    } else if compose_file.is_some() {
        RuntimeKind::Compose
    } else {
        RuntimeKind::Unknown
    };

    if matches!(runtime_kind, RuntimeKind::Unknown) {
        return None;
    }

    let project_kind = if compose_file.is_some() && !package_json.exists() && !pyproject.exists() && !cargo_toml.exists() && !go_mod.exists() {
        ProjectKind::Compose
    } else {
        ProjectKind::Repo
    };

    let mut detected_files = Vec::new();
    for path in [&package_json, &pyproject, &cargo_toml, &go_mod, &dockerfile] {
        if path.exists() {
            if let Some(name) = path.file_name().and_then(|v| v.to_str()) {
                detected_files.push(name.to_string());
            }
        }
    }
    if let Some(compose) = &compose_file {
        if let Some(name) = compose.file_name().and_then(|v| v.to_str()) {
            detected_files.push(name.to_string());
        }
    }
    if root.join(".env.example").exists() {
        detected_files.push(".env.example".to_string());
    }
    if root.join(".env.local.example").exists() {
        detected_files.push(".env.local.example".to_string());
    }

    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("project")
        .to_string();
    let slug = slugify(&name);
    let preferred_port = infer_port_hint(root, compose_file.as_deref());
    let env_template = parse_env_template(root);
    let route_subdomain_url = format!("http://{}.localhost:{}", slug, gateway_port);
    let route_path_url = format!("http://gateway.localhost:{}/p/{}/", gateway_port, slug);
    let actions = infer_actions(
        root,
        &runtime_kind,
        preferred_port,
        compose_file.as_deref(),
        &route_path_url,
    );

    let timestamp = now_iso();

    Some(ManagedProject {
        id: Uuid::new_v4().to_string(),
        name,
        slug,
        root_path: root.to_string_lossy().to_string(),
        git_url,
        project_kind,
        runtime_kind,
        status: RuntimeStatus::Stopped,
        last_error: None,
        preferred_port,
        resolved_port: None,
        route_subdomain_url,
        route_path_url,
        has_docker_compose: compose_file.is_some(),
        has_dockerfile: dockerfile.exists(),
        detected_files,
        env_template,
        env_profile: EnvProfile::default(),
        actions,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    })
}

pub fn infer_actions(
    root: &Path,
    runtime_kind: &RuntimeKind,
    port_hint: Option<u16>,
    compose_file: Option<&Path>,
    route_path_url: &str,
) -> Vec<ProjectAction> {
    let mut actions = Vec::new();
    let workdir = root.to_string_lossy().to_string();
    if root.join("package.json").exists() {
        actions.push(action("install-npm", "Install", ActionKind::Install, "npm install", &workdir, None, ActionSource::Inferred));

        if let Some(scripts) = read_package_scripts(root) {
            let preferred_runs = [("dev", "Web Dev"), ("start", "Start"), ("preview", "Preview"), ("desktop:dev", "Desktop Dev")];
            let mut seen_run = HashSet::new();
            for (script, label) in preferred_runs {
                if scripts.contains_key(script) && seen_run.insert(script) {
                    actions.push(action(
                        &format!("run-{script}"),
                        label,
                        ActionKind::Run,
                        &format!("npm run {script}"),
                        &workdir,
                        port_hint,
                        ActionSource::Inferred,
                    ));
                }
            }

            if root.join("server.mjs").exists() {
                actions.push(action(
                    "run-node-fallback",
                    "Node Fallback",
                    ActionKind::Run,
                    "node server.mjs",
                    &workdir,
                    port_hint,
                    ActionSource::Inferred,
                ));
            }

            let mut grouped = BTreeMap::new();
            for script in scripts.keys() {
                if is_build_script(script) {
                    grouped.insert(script.clone(), ActionKind::Build);
                } else if is_deploy_script(script) {
                    grouped.insert(script.clone(), ActionKind::Deploy);
                }
            }

            for (script, kind) in grouped {
                actions.push(action(
                    &format!("script-{script}"),
                    &script.replace(':', " / "),
                    kind,
                    &format!("npm run {script}"),
                    &workdir,
                    None,
                    ActionSource::Inferred,
                ));
            }
        }
    } else {
        match runtime_kind {
            RuntimeKind::Python => {
                let requirements = root.join("requirements.txt");
                if requirements.exists() {
                    actions.push(action("install-pip", "Install", ActionKind::Install, "pip install -r requirements.txt", &workdir, None, ActionSource::Inferred));
                } else {
                    actions.push(action("install-uv", "Install", ActionKind::Install, "uv sync || pip install -e .", &workdir, None, ActionSource::Inferred));
                }
                actions.push(action("run-python", "Run", ActionKind::Run, "uv run . || python -m .", &workdir, port_hint, ActionSource::Inferred));
            }
            RuntimeKind::Rust => {
                actions.push(action("run-rust", "Run", ActionKind::Run, "cargo run", &workdir, port_hint, ActionSource::Inferred));
                actions.push(action("build-rust", "Build", ActionKind::Build, "cargo build --release", &workdir, None, ActionSource::Inferred));
            }
            RuntimeKind::Go => {
                actions.push(action("run-go", "Run", ActionKind::Run, "go run .", &workdir, port_hint, ActionSource::Inferred));
                actions.push(action("build-go", "Build", ActionKind::Build, "go build ./...", &workdir, None, ActionSource::Inferred));
            }
            RuntimeKind::Compose => {}
            RuntimeKind::Unknown | RuntimeKind::Node => {}
        }
    }

    if compose_file.is_some() {
        actions.push(action(
            "compose-up",
            "Compose Up",
            ActionKind::Run,
            "docker compose up -d || docker-compose up -d",
            &workdir,
            port_hint,
            ActionSource::Inferred,
        ));
        actions.push(action(
            "compose-down",
            "Compose Down",
            ActionKind::Stop,
            "docker compose down || docker-compose down",
            &workdir,
            None,
            ActionSource::Inferred,
        ));
        actions.push(action(
            "compose-build",
            "Compose Build",
            ActionKind::Build,
            "docker compose build || docker-compose build",
            &workdir,
            None,
            ActionSource::Inferred,
        ));
    }

    actions.push(action(
        "open-route",
        "Open Route",
        ActionKind::Open,
        route_path_url,
        &workdir,
        port_hint,
        ActionSource::Inferred,
    ));

    actions
}

fn action(
    id: &str,
    label: &str,
    kind: ActionKind,
    command: &str,
    workdir: &str,
    port_hint: Option<u16>,
    source: ActionSource,
) -> ProjectAction {
    ProjectAction {
        id: id.to_string(),
        label: label.to_string(),
        kind,
        command: command.to_string(),
        workdir: workdir.to_string(),
        env_profile: Some("default".to_string()),
        port_hint,
        healthcheck_url: port_hint.map(|port| format!("http://127.0.0.1:{port}/")),
        source,
    }
}

fn is_manifest_name(name: &str) -> bool {
    matches!(
        name,
        "package.json"
            | "pyproject.toml"
            | "Cargo.toml"
            | "go.mod"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "compose.yaml"
    )
}

fn should_skip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|name| {
            matches!(
                name,
                ".git"
                    | "node_modules"
                    | "dist"
                    | "build"
                    | "target"
                    | ".next"
                    | ".turbo"
                    | ".venv"
                    | "venv"
                    | "__pycache__"
                    | ".idea"
            )
        })
        .unwrap_or(false)
}

fn first_existing(root: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().map(|name| root.join(name)).find(|path| path.exists())
}

fn read_package_scripts(root: &Path) -> Option<BTreeMap<String, String>> {
    let contents = fs::read_to_string(root.join("package.json")).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    let scripts = value.get("scripts")?.as_object()?;
    let mut output = BTreeMap::new();
    for (key, value) in scripts {
        if let Some(command) = value.as_str() {
            output.insert(key.to_string(), command.to_string());
        }
    }
    Some(output)
}

fn infer_port_hint(root: &Path, compose_file: Option<&Path>) -> Option<u16> {
    let patterns = [
        Regex::new(r"--port\s+(\d{2,5})").expect("regex"),
        Regex::new(r"localhost:(\d{2,5})").expect("regex"),
        Regex::new(r"PORT(?:=|\s+)(\d{2,5})").expect("regex"),
        Regex::new(r#""(\d{2,5}):\d{2,5}""#).expect("regex"),
        Regex::new(r#"(\d{2,5}):\d{2,5}"#).expect("regex"),
    ];

    let mut files_to_scan = vec![
        root.join("package.json"),
        root.join("server.mjs"),
        root.join("README.md"),
        root.join(".env.example"),
        root.join(".env.local.example"),
    ];
    if let Some(compose) = compose_file {
        files_to_scan.push(compose.to_path_buf());
    }

    for file in files_to_scan {
        if !file.exists() {
            continue;
        }
        let Ok(contents) = fs::read_to_string(&file) else {
            continue;
        };
        for pattern in &patterns {
            if let Some(capture) = pattern.captures(&contents) {
                if let Some(port) = capture.get(1).and_then(|value| value.as_str().parse::<u16>().ok()) {
                    return Some(port);
                }
            }
        }
        if file.ends_with("package.json")
            && contents.contains("\"vite\"")
            && contents.contains("\"dev\"")
        {
            return Some(5173);
        }
    }

    None
}

fn is_build_script(script: &str) -> bool {
    script.starts_with("build")
        || script.starts_with("desktop:build")
        || script.starts_with("package")
}

fn is_deploy_script(script: &str) -> bool {
    script.starts_with("deploy") || script.starts_with("publish") || script.starts_with("release")
}

#[cfg(test)]
mod tests {
    use super::{infer_actions, parse_env_template_contents, repo_name_from_git_url};
    use crate::core::models::{ActionKind, RuntimeKind};
    use std::path::Path;

    #[test]
    fn parses_repo_name_from_git_url() {
        let name = repo_name_from_git_url("https://github.com/calesthio/Crucix.git").unwrap();
        assert_eq!(name, "Crucix");
    }

    #[test]
    fn parses_env_template_comments_and_field_types() {
        let env = r#"
# API key for alerts
ALERTS_API_KEY=
# Enable debug mode
DEBUG=false
JSON_PAYLOAD={"a":1}
"#;
        let fields = parse_env_template_contents(env);
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].key, "ALERTS_API_KEY");
        assert!(fields[0].description.as_ref().unwrap().contains("API key"));
    }

    #[test]
    fn infers_compose_fallback_actions() {
        let actions = infer_actions(
            Path::new("."),
            &RuntimeKind::Compose,
            Some(3117),
            Some(Path::new("docker-compose.yml")),
            "http://gateway.localhost:42300/p/crucix/",
        );
        assert!(actions.iter().any(|action| {
            action.kind == ActionKind::Run && action.command.contains("docker-compose up -d")
        }));
    }
}
