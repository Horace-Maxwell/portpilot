use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use serde_json::Value;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::core::models::{
    ActionKind, ActionSource, DetectedAppTarget, EnvFieldType, EnvProfile, EnvTemplateField,
    ImportedRepo, ManagedProject, ProjectAction, ProjectKind, RuntimeKind, RuntimeStatus,
};

pub const DEFAULT_WORKSPACE_ROOT: &str = "/Users/horacedong/Desktop/Github";

pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    fn install_command(self) -> &'static str {
        match self {
            Self::Npm => "npm install",
            Self::Pnpm => "pnpm install",
            Self::Yarn => "yarn install",
            Self::Bun => "bun install",
        }
    }

    fn run_script_command(self, script: &str) -> String {
        match self {
            Self::Npm => format!("npm run {script}"),
            Self::Pnpm => format!("pnpm run {script}"),
            Self::Yarn => format!("yarn run {script}"),
            Self::Bun => format!("bun run {script}"),
        }
    }
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
                    workspace_target_count: project.workspace_targets.len(),
                    readme_hints: project.readme_hints,
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
    let workspace_targets = detect_workspace_targets(root);
    let readme_hints = infer_readme_hints(root);
    let route_subdomain_url = format!("http://{}.localhost:{}", slug, gateway_port);
    let route_path_url = format!("http://gateway.localhost:{}/p/{}/", gateway_port, slug);
    let actions = infer_actions(
        root,
        &runtime_kind,
        preferred_port,
        compose_file.as_deref(),
        &route_path_url,
        &workspace_targets,
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
        workspace_targets,
        readme_hints,
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
    workspace_targets: &[DetectedAppTarget],
) -> Vec<ProjectAction> {
    let mut actions = Vec::new();
    let workdir = root.to_string_lossy().to_string();
    if root.join("package.json").exists() {
        let package_manager = detect_package_manager(root);
        actions.push(action(
            "install-node",
            "Install",
            ActionKind::Install,
            package_manager.install_command(),
            &workdir,
            None,
            ActionSource::Inferred,
        ));

        if let Some(scripts) = read_package_scripts(root) {
            let preferred_runs = [("dev", "Web Dev"), ("start", "Start"), ("preview", "Preview"), ("desktop:dev", "Desktop Dev")];
            let mut seen_run = HashSet::new();
            for (script, label) in preferred_runs {
                if scripts.contains_key(script) && seen_run.insert(script) {
                    actions.push(action(
                        &format!("run-{script}"),
                        label,
                        ActionKind::Run,
                        &package_manager.run_script_command(script),
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
                    &package_manager.run_script_command(&script),
                    &workdir,
                    None,
                    ActionSource::Inferred,
                ));
            }
        }

        for target in workspace_targets {
            actions.extend(infer_target_actions(target));
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
                let make_targets = read_make_targets(root);
                if make_targets.contains("install") {
                    actions.push(action("install-go-make", "Install", ActionKind::Install, "make install", &workdir, None, ActionSource::Inferred));
                }
                let run_command = if make_targets.contains("run") {
                    "make run"
                } else {
                    "go run ."
                };
                let build_command = if make_targets.contains("build") {
                    "make build"
                } else {
                    "go build ./..."
                };
                actions.push(action("run-go", "Run", ActionKind::Run, run_command, &workdir, port_hint, ActionSource::Inferred));
                actions.push(action("build-go", "Build", ActionKind::Build, build_command, &workdir, None, ActionSource::Inferred));
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

fn detect_workspace_targets(root: &Path) -> Vec<DetectedAppTarget> {
    let mut output = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut patterns = read_package_workspaces(root);
    patterns.extend(read_pnpm_workspace_patterns(root));

    for pattern in patterns {
        for target_root in expand_workspace_pattern(root, &pattern) {
            if !seen_paths.insert(target_root.clone()) {
                continue;
            }

            let target_path = Path::new(&target_root);
            if let Some(target) = build_detected_target(root, target_path) {
                output.push(target);
            }
        }
    }

    for target_path in find_nested_target_dirs(root) {
        if !seen_paths.insert(target_path.to_string_lossy().to_string()) {
            continue;
        }
        if let Some(target) = build_detected_target(root, &target_path) {
            output.push(target);
        }
    }

    output.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    output
}

fn infer_readme_hints(root: &Path) -> Vec<String> {
    let readme = root.join("README.md");
    if !readme.exists() {
        return Vec::new();
    }

    let Ok(contents) = fs::read_to_string(readme) else {
        return Vec::new();
    };

    let patterns = [
        Regex::new(r"`(npm (?:install|run [\w:-]+))`").expect("regex"),
        Regex::new(r"`(pnpm (?:install|dev|run [\w:-]+))`").expect("regex"),
        Regex::new(r"`(yarn(?:\s+[\w:-]+)?)`").expect("regex"),
        Regex::new(r"`(bun (?:install|run [\w:-]+))`").expect("regex"),
        Regex::new(r"`(make [\w:-]+)`").expect("regex"),
        Regex::new(r"`(docker compose [^`]+)`").expect("regex"),
        Regex::new(r"`(python -m [^`]+)`").expect("regex"),
        Regex::new(r"`(uv (?:sync|run [^`]+))`").expect("regex"),
    ];

    let mut hints = Vec::new();
    let mut seen = HashSet::new();
    for pattern in patterns {
        for capture in pattern.captures_iter(&contents) {
            let Some(command) = capture.get(1).map(|value| value.as_str().trim().to_string()) else {
                continue;
            };
            if seen.insert(command.clone()) {
                hints.push(command);
            }
            if hints.len() == 4 {
                return hints;
            }
        }
    }

    hints
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

fn read_package_name(root: &Path) -> Option<String> {
    let contents = fs::read_to_string(root.join("package.json")).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    value.get("name")?.as_str().map(ToString::to_string)
}

fn read_package_manager(root: &Path) -> Option<String> {
    let contents = fs::read_to_string(root.join("package.json")).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    value.get("packageManager")?.as_str().map(ToString::to_string)
}

fn detect_package_manager(root: &Path) -> PackageManager {
    if let Some(package_manager) = read_package_manager(root) {
        let normalized = package_manager.to_ascii_lowercase();
        if normalized.starts_with("pnpm@") {
            return PackageManager::Pnpm;
        }
        if normalized.starts_with("yarn@") {
            return PackageManager::Yarn;
        }
        if normalized.starts_with("bun@") {
            return PackageManager::Bun;
        }
    }

    for (file_name, package_manager) in [
        ("pnpm-lock.yaml", PackageManager::Pnpm),
        ("yarn.lock", PackageManager::Yarn),
        ("bun.lock", PackageManager::Bun),
        ("bun.lockb", PackageManager::Bun),
        ("package-lock.json", PackageManager::Npm),
    ] {
        if root.join(file_name).exists() {
            return package_manager;
        }
    }

    PackageManager::Npm
}

fn read_package_workspaces(root: &Path) -> Vec<String> {
    let contents = match fs::read_to_string(root.join("package.json")) {
        Ok(contents) => contents,
        Err(_) => return Vec::new(),
    };
    let value: Value = match serde_json::from_str(&contents) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    if let Some(workspaces) = value.get("workspaces") {
        if let Some(items) = workspaces.as_array() {
            return items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect();
        }

        if let Some(packages) = workspaces.get("packages").and_then(Value::as_array) {
            return packages
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect();
        }
    }

    Vec::new()
}

fn read_pnpm_workspace_patterns(root: &Path) -> Vec<String> {
    let path = root.join("pnpm-workspace.yaml");
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut output = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('-') {
            continue;
        }
        let pattern = trimmed
            .trim_start_matches('-')
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if !pattern.is_empty() {
            output.push(pattern.to_string());
        }
    }
    output
}

fn expand_workspace_pattern(root: &Path, pattern: &str) -> Vec<String> {
    let normalized = pattern.trim().trim_start_matches("./");
    if normalized.is_empty() {
        return Vec::new();
    }

    if !normalized.contains('*') {
        let path = root.join(normalized);
        if path.exists() && path.is_dir() {
            return vec![path.to_string_lossy().to_string()];
        }
        return Vec::new();
    }

    let prefix = normalized.split('*').next().unwrap_or_default().trim_end_matches('/');
    let base = root.join(prefix);
    let Ok(entries) = fs::read_dir(base) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

fn find_nested_target_dirs(root: &Path) -> Vec<PathBuf> {
    let mut targets = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return targets;
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() || should_skip_dir(&path) {
            continue;
        }
        if has_supported_manifest(&path) {
            targets.push(path.clone());
        }

        let Some(dir_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !matches!(dir_name, "apps" | "packages" | "services") {
            continue;
        }

        let Ok(child_entries) = fs::read_dir(&path) else {
            continue;
        };
        for child in child_entries.filter_map(Result::ok).map(|item| item.path()) {
            if child.is_dir() && !should_skip_dir(&child) && has_supported_manifest(&child) {
                targets.push(child);
            }
        }
    }

    targets
}

fn has_supported_manifest(path: &Path) -> bool {
    path.join("package.json").exists()
        || path.join("pyproject.toml").exists()
        || path.join("go.mod").exists()
        || path.join("Cargo.toml").exists()
}

fn infer_runtime_kind_for_path(path: &Path) -> Option<RuntimeKind> {
    if path.join("package.json").exists() {
        Some(RuntimeKind::Node)
    } else if path.join("pyproject.toml").exists() {
        Some(RuntimeKind::Python)
    } else if path.join("Cargo.toml").exists() {
        Some(RuntimeKind::Rust)
    } else if path.join("go.mod").exists() {
        Some(RuntimeKind::Go)
    } else {
        None
    }
}

fn build_detected_target(root: &Path, target_path: &Path) -> Option<DetectedAppTarget> {
    let runtime_kind = infer_runtime_kind_for_path(target_path)?;
    let relative_path = target_path
        .strip_prefix(root)
        .ok()
        .map(|value| value.to_string_lossy().to_string())?;
    if relative_path.is_empty() {
        return None;
    }

    let name = match runtime_kind {
        RuntimeKind::Node => read_package_name(target_path),
        _ => None,
    }
    .or_else(|| {
        target_path
            .file_name()
            .and_then(|value| value.to_str())
            .map(ToString::to_string)
    })
    .unwrap_or_else(|| relative_path.clone());

    let available_actions = target_available_actions(target_path, &runtime_kind);
    if available_actions.is_empty() {
        return None;
    }

    Some(DetectedAppTarget {
        id: slugify(&relative_path),
        name,
        relative_path,
        root_path: target_path.to_string_lossy().to_string(),
        runtime_kind,
        suggested_port: infer_port_hint(target_path, first_existing(target_path, &["docker-compose.yml", "docker-compose.yaml", "compose.yaml"]).as_deref()),
        available_actions,
    })
}

fn target_available_actions(target_path: &Path, runtime_kind: &RuntimeKind) -> Vec<String> {
    match runtime_kind {
        RuntimeKind::Node => read_package_scripts(target_path)
            .unwrap_or_default()
            .keys()
            .filter(|script| {
                matches!(script.as_str(), "dev" | "start" | "preview")
                    || is_build_script(script)
            })
            .cloned()
            .collect(),
        RuntimeKind::Python => vec!["run".to_string(), "install".to_string()],
        RuntimeKind::Rust => vec!["run".to_string(), "build".to_string()],
        RuntimeKind::Go => {
            let make_targets = read_make_targets(target_path);
            if make_targets.contains("run") || make_targets.contains("build") || make_targets.contains("install") {
                let mut actions = Vec::new();
                if make_targets.contains("install") {
                    actions.push("install".to_string());
                }
                if make_targets.contains("run") {
                    actions.push("run".to_string());
                }
                if make_targets.contains("build") {
                    actions.push("build".to_string());
                }
                actions
            } else {
                vec!["run".to_string(), "build".to_string()]
            }
        }
        RuntimeKind::Compose => vec!["run".to_string()],
        RuntimeKind::Unknown => Vec::new(),
    }
}

fn infer_target_actions(target: &DetectedAppTarget) -> Vec<ProjectAction> {
    let target_root = Path::new(&target.root_path);
    match target.runtime_kind {
        RuntimeKind::Node => {
            let Some(target_scripts) = read_package_scripts(target_root) else {
                return Vec::new();
            };
            let package_manager = detect_package_manager(target_root);
            let mut actions = Vec::new();

            for (script, label) in [("dev", "Run"), ("start", "Start"), ("preview", "Preview"), ("desktop:dev", "Desktop Dev")] {
                if target_scripts.contains_key(script) {
                    actions.push(action(
                        &format!("workspace-{}-{script}", target.id),
                        &format!("{label} {}", target.name),
                        ActionKind::Run,
                        &package_manager.run_script_command(script),
                        &target.root_path,
                        target.suggested_port,
                        ActionSource::Inferred,
                    ));
                }
            }

            for script in target_scripts.keys() {
                if is_build_script(script) {
                    actions.push(action(
                        &format!("workspace-{}-build-{script}", target.id),
                        &format!("Build {}", target.name),
                        ActionKind::Build,
                        &package_manager.run_script_command(script),
                        &target.root_path,
                        None,
                        ActionSource::Inferred,
                    ));
                    break;
                }
            }

            actions
        }
        RuntimeKind::Python => vec![
            action(
                &format!("workspace-{}-install", target.id),
                &format!("Install {}", target.name),
                ActionKind::Install,
                "uv sync || pip install -e .",
                &target.root_path,
                None,
                ActionSource::Inferred,
            ),
            action(
                &format!("workspace-{}-run", target.id),
                &format!("Run {}", target.name),
                ActionKind::Run,
                "uv run . || python -m .",
                &target.root_path,
                target.suggested_port,
                ActionSource::Inferred,
            ),
        ],
        RuntimeKind::Rust => vec![
            action(
                &format!("workspace-{}-run", target.id),
                &format!("Run {}", target.name),
                ActionKind::Run,
                "cargo run",
                &target.root_path,
                target.suggested_port,
                ActionSource::Inferred,
            ),
            action(
                &format!("workspace-{}-build", target.id),
                &format!("Build {}", target.name),
                ActionKind::Build,
                "cargo build --release",
                &target.root_path,
                None,
                ActionSource::Inferred,
            ),
        ],
        RuntimeKind::Go => {
            let make_targets = read_make_targets(target_root);
            let install_command = if make_targets.contains("install") {
                "make install"
            } else {
                "go mod download"
            };
            let run_command = if make_targets.contains("run") {
                "make run"
            } else {
                "go run ."
            };
            let build_command = if make_targets.contains("build") {
                "make build"
            } else {
                "go build ./..."
            };
            vec![
                action(
                    &format!("workspace-{}-install", target.id),
                    &format!("Install {}", target.name),
                    ActionKind::Install,
                    install_command,
                    &target.root_path,
                    None,
                    ActionSource::Inferred,
                ),
                action(
                    &format!("workspace-{}-run", target.id),
                    &format!("Run {}", target.name),
                    ActionKind::Run,
                    run_command,
                    &target.root_path,
                    target.suggested_port,
                    ActionSource::Inferred,
                ),
                action(
                    &format!("workspace-{}-build", target.id),
                    &format!("Build {}", target.name),
                    ActionKind::Build,
                    build_command,
                    &target.root_path,
                    None,
                    ActionSource::Inferred,
                ),
            ]
        }
        RuntimeKind::Compose | RuntimeKind::Unknown => Vec::new(),
    }
}

fn read_make_targets(root: &Path) -> HashSet<String> {
    let makefiles = ["Makefile", "makefile", "GNUmakefile"];
    for file_name in makefiles {
        let path = root.join(file_name);
        let Ok(contents) = fs::read_to_string(path) else {
            continue;
        };
        let mut targets = HashSet::new();
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('.') || trimmed.starts_with('\t') {
                continue;
            }
            let Some((target, _rest)) = trimmed.split_once(':') else {
                continue;
            };
            let target = target.trim();
            if !target.is_empty()
                && target
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
            {
                targets.insert(target.to_string());
            }
        }
        return targets;
    }

    HashSet::new()
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
    use super::{
        detect_workspace_targets, infer_actions, parse_env_template_contents, repo_name_from_git_url,
    };
    use crate::core::models::{ActionKind, RuntimeKind};
    use std::{fs, path::Path};

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
            &[],
        );
        assert!(actions.iter().any(|action| {
            action.kind == ActionKind::Run && action.command.contains("docker-compose up -d")
        }));
    }

    #[test]
    fn detects_workspace_targets_from_package_workspaces() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("apps/web")).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "root",
              "workspaces": ["apps/*"]
            }"#,
        )
        .unwrap();
        fs::write(
            root.join("apps/web/package.json"),
            r#"{
              "name": "@demo/web",
              "scripts": {
                "dev": "vite",
                "build": "vite build"
              }
            }"#,
        )
        .unwrap();

        let targets = detect_workspace_targets(&root);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].relative_path, "apps/web");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uses_pnpm_for_node_actions_when_package_manager_requires_it() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "vitesse-like",
              "packageManager": "pnpm@10.30.2",
              "scripts": {
                "dev": "vite",
                "build": "vite build"
              }
            }"#,
        )
        .unwrap();

        let actions = infer_actions(
            &root,
            &RuntimeKind::Node,
            Some(3333),
            None,
            "http://gateway.localhost:42300/p/demo/",
            &[],
        );

        assert!(actions.iter().any(|action| action.command == "pnpm install"));
        assert!(actions.iter().any(|action| action.command == "pnpm run dev"));
        assert!(actions.iter().any(|action| action.command == "pnpm run build"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uses_bun_for_node_actions_when_bun_lock_is_present() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "bun-like",
              "workspaces": ["frontend"],
              "scripts": {
                "dev": "bun run --filter frontend dev"
              }
            }"#,
        )
        .unwrap();
        fs::write(root.join("bun.lock"), "").unwrap();

        let actions = infer_actions(
            &root,
            &RuntimeKind::Node,
            None,
            None,
            "http://gateway.localhost:42300/p/demo/",
            &[],
        );

        assert!(actions.iter().any(|action| action.command == "bun install"));
        assert!(actions.iter().any(|action| action.command == "bun run dev"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prefers_make_targets_for_go_repositories() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("go.mod"), "module example.com/demo\n").unwrap();
        fs::write(
            root.join("Makefile"),
            r#"
install:
	go install ./...

run:
	go run cmd/web/main.go

build:
	go build ./...
"#,
        )
        .unwrap();

        let actions = infer_actions(
            &root,
            &RuntimeKind::Go,
            Some(8000),
            None,
            "http://gateway.localhost:42300/p/demo/",
            &[],
        );

        assert!(actions.iter().any(|action| action.command == "make install"));
        assert!(actions.iter().any(|action| action.command == "make run"));
        assert!(actions.iter().any(|action| action.command == "make build"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_nested_targets_outside_explicit_workspaces() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("backend")).unwrap();
        fs::create_dir_all(root.join("frontend")).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "mixed-stack",
              "scripts": {
                "dev": "bun run --filter frontend dev"
              }
            }"#,
        )
        .unwrap();
        fs::write(root.join("bun.lock"), "").unwrap();
        fs::write(root.join("backend/pyproject.toml"), "[project]\nname='backend'\n").unwrap();
        fs::write(
            root.join("frontend/package.json"),
            r#"{
              "name": "frontend",
              "scripts": {
                "dev": "vite"
              }
            }"#,
        )
        .unwrap();

        let targets = detect_workspace_targets(&root);
        assert_eq!(targets.len(), 2);
        assert!(targets.iter().any(|target| target.relative_path == "backend"));
        assert!(targets.iter().any(|target| target.relative_path == "frontend"));

        let _ = fs::remove_dir_all(root);
    }
}
