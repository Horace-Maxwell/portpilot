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
    ImportedRepo, ManagedProject, ProjectAction, ProjectKind, ProjectProfile, ProjectProfileKind,
    ProjectRecipe, RouteStrategy, RuntimeKind, RuntimeStatus,
};

pub fn default_workspace_root() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/".to_string())
}

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

fn prettify_script_label(script: &str) -> String {
    script
        .split(':')
        .map(|part| {
            let mut chars = part.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            let mut label = String::new();
            label.extend(first.to_uppercase());
            label.push_str(chars.as_str());
            label
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

fn is_preferred_node_run_script(script: &str, command: &str) -> bool {
    matches!(script, "dev" | "start" | "preview" | "desktop:dev")
        || script == "dev:all"
        || script.starts_with("dev:")
        || script.ends_with(":dev")
        || script.ends_with(":start")
        || script.ends_with(":preview")
        || script.starts_with("ui:")
        || script.starts_with("web:")
        || script.contains("gateway")
        || command.contains("vite")
        || command.contains("server.js")
        || command.contains("serve")
        || command.contains("run-node.mjs")
}

fn collect_preferred_node_run_scripts(scripts: &BTreeMap<String, String>) -> Vec<String> {
    let mut selected = Vec::new();
    let mut seen = HashSet::new();

    for script in ["dev", "start", "preview", "desktop:dev"] {
        if scripts.contains_key(script) {
            selected.push(script.to_string());
            seen.insert(script.to_string());
        }
    }

    for (script, command) in scripts {
        if seen.contains(script) || !is_preferred_node_run_script(script, command) {
            continue;
        }
        selected.push(script.clone());
        seen.insert(script.clone());
    }

    selected
}

fn label_for_node_run_script(script: &str) -> String {
    match script {
        "dev" => "Web Dev".to_string(),
        "dev:all" => "Run All".to_string(),
        "start" => "Start".to_string(),
        "preview" => "Preview".to_string(),
        "desktop:dev" => "Desktop Dev".to_string(),
        _ => format!("Run {}", prettify_script_label(script)),
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

            let default_value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
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
            } else if default_value.contains("\\n")
                || default_value.contains('{')
                || default_value.contains('[')
            {
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
                    project_profile: project.project_profile,
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
    let requirements_txt = root.join("requirements.txt");
    let cargo_toml = root.join("Cargo.toml");
    let go_mod = root.join("go.mod");
    let compose_file = find_compose_file(root);
    let dockerfile = root.join("Dockerfile");

    let runtime_kind = if package_json.exists() {
        RuntimeKind::Node
    } else if pyproject.exists() || requirements_txt.exists() {
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

    let project_kind = if compose_file.is_some()
        && !package_json.exists()
        && !pyproject.exists()
        && !cargo_toml.exists()
        && !go_mod.exists()
    {
        ProjectKind::Compose
    } else {
        ProjectKind::Repo
    };

    let mut detected_files = Vec::new();
    for path in [
        &package_json,
        &pyproject,
        &requirements_txt,
        &cargo_toml,
        &go_mod,
        &dockerfile,
    ] {
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
    let recipe = read_project_recipe(root);
    if recipe.is_some() {
        detected_files.push(".portpilot.json".to_string());
    }

    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("project")
        .to_string();
    let slug = slugify(&name);
    let inferred_port = infer_port_hint(root, compose_file.as_deref());
    let mut env_template = parse_env_template(root);
    if let Some(compose) = compose_file.as_deref() {
        merge_discovered_env_fields(&mut env_template, parse_compose_env_template(compose));
    }
    let mut workspace_targets = detect_workspace_targets(root);
    let inferred_readme_hints = infer_readme_hints(root);
    // Priority: explicit recipe > well-known project name > generic file-scan inference.
    // Builtin ports are intentional product defaults (e.g. lobe-chat=3210, n8n=5678)
    // and should not be overridden by framework heuristics (e.g. Next.js → 3000).
    let preferred_port = recipe
        .as_ref()
        .and_then(|item| item.preferred_port)
        .or_else(|| builtin_default_port(&name))
        .or(inferred_port);
    if let Some(recipe) = &recipe {
        merge_recipe_env_keys(&mut env_template, recipe);
        apply_recipe_targets(&mut workspace_targets, recipe);
    }
    let primary_target_id = select_primary_target_id(&workspace_targets, recipe.as_ref());
    let readme_hints = merge_readme_hints(inferred_readme_hints, recipe.as_ref());
    let route_subdomain_url = format!("http://{}.localhost:{}", slug, gateway_port);
    let route_path_url = format!("http://gateway.localhost:{}/p/{}/", gateway_port, slug);
    let mut actions = infer_actions(
        root,
        &runtime_kind,
        preferred_port,
        compose_file.as_deref(),
        &route_path_url,
        &workspace_targets,
    );
    if let Some(recipe) = &recipe {
        apply_recipe_action_preferences(&mut actions, recipe);
    }
    let project_profile = infer_project_profile(
        root,
        &name,
        &runtime_kind,
        &project_kind,
        preferred_port,
        compose_file.as_deref(),
        &workspace_targets,
        &actions,
        &readme_hints,
        recipe.as_ref(),
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
        primary_target_id,
        workspace_targets,
        readme_hints,
        project_profile,
        env_template,
        env_profile: EnvProfile::default(),
        actions,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    })
}

fn builtin_default_port(name: &str) -> Option<u16> {
    match name.to_ascii_lowercase().as_str() {
        // AI chat UIs
        "sillytavern" => Some(8000),
        "open-webui" => Some(8080),
        "librechat" => Some(3080),
        "anything-llm" => Some(3001),
        "localai" | "local-ai" => Some(8080),
        "lobe-chat" | "lobechat" => Some(3210),
        "chatbot-ui" => Some(3000),
        // AI workflow builders
        "flowise" => Some(3000),
        "langflow" => Some(7860),
        "dify" => Some(3000),
        "n8n" => Some(5678),
        "ragflow" => Some(9380),
        // Image / diffusion
        "comfyui" => Some(8188),
        "stable-diffusion-webui" | "stable-diffusion-webui-forge" | "invokeai" => Some(7860),
        // Gateway / infra
        "openclaw" => Some(18789),
        _ => None,
    }
}

fn read_project_recipe(root: &Path) -> Option<ProjectRecipe> {
    let path = root.join(".portpilot.json");
    let contents = fs::read_to_string(path).ok()?;
    serde_json::from_str::<ProjectRecipe>(&contents).ok()
}

fn infer_project_profile(
    root: &Path,
    name: &str,
    runtime_kind: &RuntimeKind,
    project_kind: &ProjectKind,
    preferred_port: Option<u16>,
    compose_file: Option<&Path>,
    workspace_targets: &[DetectedAppTarget],
    actions: &[ProjectAction],
    readme_hints: &[String],
    recipe: Option<&ProjectRecipe>,
) -> ProjectProfile {
    let mut profile = builtin_project_profile(
        root,
        name,
        runtime_kind,
        project_kind,
        preferred_port,
        compose_file,
        workspace_targets,
        actions,
    );

    if matches!(profile.kind, ProjectProfileKind::Unknown) {
        profile.kind =
            infer_generic_profile_kind(runtime_kind, project_kind, compose_file, workspace_targets);
    }

    if profile.summary.is_none() {
        profile.summary = Some(default_profile_summary(
            &profile.kind,
            compose_file.is_some(),
            readme_hints,
        ));
    }

    if profile.route_strategy.is_none() {
        profile.route_strategy = Some(default_route_strategy(
            &profile.kind,
            compose_file.is_some(),
        ));
    }

    if profile.preferred_entrypoint.is_none() {
        profile.preferred_entrypoint = actions
            .iter()
            .find(|action| matches!(action.kind, ActionKind::Run))
            .map(|action| action.id.clone());
    }

    if profile.known_ports.is_empty() {
        let mut ports = Vec::new();
        if let Some(port) = preferred_port {
            ports.push(port);
        }
        ports.extend(
            workspace_targets
                .iter()
                .filter_map(|target| target.suggested_port),
        );
        ports.sort_unstable();
        ports.dedup();
        profile.known_ports = ports;
    }

    if let Some(recipe) = recipe {
        apply_recipe_profile_overrides(&mut profile, recipe);
    }

    profile
}

fn builtin_project_profile(
    _root: &Path,
    name: &str,
    runtime_kind: &RuntimeKind,
    project_kind: &ProjectKind,
    preferred_port: Option<u16>,
    compose_file: Option<&Path>,
    workspace_targets: &[DetectedAppTarget],
    actions: &[ProjectAction],
) -> ProjectProfile {
    let lower = name.to_ascii_lowercase();
    let compose_services = compose_file
        .map(parse_compose_service_names_from_file)
        .unwrap_or_default();

    if lower == "sillytavern" {
        return ProjectProfile {
            kind: ProjectProfileKind::AiUi,
            preferred_entrypoint: actions
                .iter()
                .find(|action| {
                    action.command.contains("server.js") || action.command.contains("npm start")
                })
                .map(|action| action.id.clone()),
            required_services: Vec::new(),
            required_env_groups: vec!["model-providers".to_string()],
            known_ports: vec![preferred_port.unwrap_or(8000)],
            route_strategy: Some(RouteStrategy::LocalhostDirect),
            summary: Some(
                "Local AI chat UI with a fixed localhost port and optional provider credentials."
                    .to_string(),
            ),
        };
    }

    if lower == "open-webui" {
        return ProjectProfile {
            kind: ProjectProfileKind::AiUi,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command == "open-webui serve")
                .or_else(|| actions.iter().find(|action| action.command.contains("make start")))
                .or_else(|| actions.iter().find(|action| matches!(action.kind, ActionKind::Run)))
                .map(|action| action.id.clone()),
            required_services: if compose_services.is_empty() {
                vec!["open-webui".to_string(), "ollama".to_string()]
            } else {
                compose_services
            },
            required_env_groups: vec!["model-providers".to_string()],
            known_ports: vec![preferred_port.unwrap_or(8080), 3000],
            route_strategy: Some(RouteStrategy::Hybrid),
            summary: Some("Hybrid local AI workspace with a Python backend, web UI, and optional compose services.".to_string()),
        };
    }

    if lower == "openclaw" {
        return ProjectProfile {
            kind: ProjectProfileKind::GatewayStack,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command == "pnpm run gateway:dev")
                .or_else(|| actions.iter().find(|action| action.command.contains("openclaw gateway")))
                .or_else(|| actions.iter().find(|action| matches!(action.kind, ActionKind::Run)))
                .map(|action| action.id.clone()),
            required_services: if compose_services.is_empty() {
                vec!["gateway".to_string(), "webchat".to_string()]
            } else {
                compose_services
            },
            required_env_groups: vec!["workspace".to_string(), "gateway".to_string(), "credentials".to_string()],
            known_ports: vec![18789, 18790],
            route_strategy: Some(RouteStrategy::GatewayPath),
            summary: Some("Gateway-style localhost stack with multiple entrypoints, bridge services, and required workspace env.".to_string()),
        };
    }

    if lower == "example-voting-app" {
        return ProjectProfile {
            kind: ProjectProfileKind::ComposeStack,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command.contains("compose up"))
                .map(|action| action.id.clone()),
            required_services: if compose_services.is_empty() {
                vec![
                    "vote".to_string(),
                    "result".to_string(),
                    "worker".to_string(),
                    "redis".to_string(),
                    "db".to_string(),
                ]
            } else {
                compose_services
            },
            required_env_groups: Vec::new(),
            known_ports: preferred_port.into_iter().collect(),
            route_strategy: Some(RouteStrategy::ComposeService),
            summary: Some("Compose-first multi-service stack that needs dependent services healthy before the app is really online.".to_string()),
        };
    }

    if lower == "librechat" {
        return ProjectProfile {
            kind: ProjectProfileKind::GatewayStack,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command.contains("start:deployed"))
                .or_else(|| actions.iter().find(|action| action.command.contains("frontend:dev")))
                .or_else(|| actions.iter().find(|action| action.command.contains("backend:dev")))
                .or_else(|| actions.iter().find(|action| action.command.contains("compose up")))
                .or_else(|| actions.iter().find(|action| matches!(action.kind, ActionKind::Run)))
                .map(|action| action.id.clone()),
            required_services: if compose_services.is_empty() {
                vec![
                    "api".to_string(),
                    "mongodb".to_string(),
                    "meilisearch".to_string(),
                    "rag_api".to_string(),
                ]
            } else {
                compose_services
            },
            required_env_groups: vec!["database".to_string(), "search".to_string(), "rag".to_string()],
            known_ports: vec![preferred_port.unwrap_or(3080), 8000],
            route_strategy: Some(RouteStrategy::Hybrid),
            summary: Some("Local AI chat platform with a primary web route, backend services, vector search, and optional RAG sidecar.".to_string()),
        };
    }

    if lower == "anything-llm" {
        return ProjectProfile {
            kind: ProjectProfileKind::AiUi,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command.contains("dev:all"))
                .or_else(|| actions.iter().find(|action| action.command.contains("compose up")))
                .or_else(|| actions.iter().find(|action| action.command.contains("dev:server")))
                .or_else(|| actions.iter().find(|action| matches!(action.kind, ActionKind::Run)))
                .map(|action| action.id.clone()),
            required_services: if compose_services.is_empty() {
                vec!["anything-llm".to_string()]
            } else {
                compose_services
            },
            required_env_groups: vec!["llm-provider".to_string(), "frontend".to_string(), "server".to_string()],
            known_ports: vec![preferred_port.unwrap_or(3001), 3000],
            route_strategy: Some(RouteStrategy::Hybrid),
            summary: Some("Local LLM workspace with coordinated frontend, API, collector, and optional provider backends.".to_string()),
        };
    }

    if lower == "localai" || lower == "local-ai" {
        return ProjectProfile {
            kind: ProjectProfileKind::AiUi,
            preferred_entrypoint: actions
                .iter()
                .find(|action| {
                    action.command.contains("docker run") || action.command.contains("local-ai")
                })
                .or_else(|| {
                    actions
                        .iter()
                        .find(|action| matches!(action.kind, ActionKind::Run))
                })
                .map(|action| action.id.clone()),
            required_services: if compose_services.is_empty() {
                vec!["localai".to_string()]
            } else {
                compose_services
            },
            required_env_groups: vec!["models".to_string()],
            known_ports: vec![preferred_port.unwrap_or(8080)],
            route_strategy: Some(RouteStrategy::LocalhostDirect),
            summary: Some(
                "Local OpenAI-compatible model server with a single localhost API surface."
                    .to_string(),
            ),
        };
    }

    if lower == "lobe-chat" || lower == "lobechat" {
        return ProjectProfile {
            kind: ProjectProfileKind::AiUi,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command.contains("pnpm run dev"))
                .or_else(|| actions.iter().find(|action| action.command.contains("next dev")))
                .or_else(|| actions.iter().find(|action| matches!(action.kind, ActionKind::Run)))
                .map(|action| action.id.clone()),
            required_services: compose_services,
            required_env_groups: vec!["model-providers".to_string(), "frontend".to_string()],
            known_ports: vec![preferred_port.unwrap_or(3210)],
            route_strategy: Some(if compose_file.is_some() {
                RouteStrategy::Hybrid
            } else {
                RouteStrategy::LocalhostDirect
            }),
            summary: Some("Local-first AI chat UI with a primary Next.js route and optional provider credentials.".to_string()),
        };
    }

    if lower == "flowise" {
        return ProjectProfile {
            kind: ProjectProfileKind::AiUi,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command == "pnpm run start")
                .or_else(|| actions.iter().find(|action| action.command.contains("compose up")))
                .or_else(|| actions.iter().find(|action| action.command.contains("npx flowise start")))
                .or_else(|| actions.iter().find(|action| matches!(action.kind, ActionKind::Run)))
                .map(|action| action.id.clone()),
            required_services: if compose_services.is_empty() {
                vec!["flowise".to_string()]
            } else {
                compose_services
            },
            required_env_groups: vec!["app".to_string(), "database".to_string(), "queue".to_string()],
            known_ports: vec![preferred_port.unwrap_or(3000), 8080],
            route_strategy: Some(RouteStrategy::Hybrid),
            summary: Some("Node-based local AI builder with a primary UI, optional worker queue, and many compose-driven environment requirements.".to_string()),
        };
    }

    if lower == "langflow" {
        return ProjectProfile {
            kind: ProjectProfileKind::AiUi,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command.contains("langflow"))
                .or_else(|| actions.iter().find(|action| matches!(action.kind, ActionKind::Run)))
                .map(|action| action.id.clone()),
            required_services: compose_services,
            required_env_groups: vec!["models".to_string(), "database".to_string()],
            known_ports: vec![preferred_port.unwrap_or(7860)],
            route_strategy: Some(if compose_file.is_some() {
                RouteStrategy::Hybrid
            } else {
                RouteStrategy::LocalhostDirect
            }),
            summary: Some("Local flow builder with a single web route and optional model or database backing services.".to_string()),
        };
    }

    if lower == "n8n" {
        return ProjectProfile {
            kind: ProjectProfileKind::WebApp,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command.contains("compose up"))
                .or_else(|| actions.iter().find(|action| action.command.contains("n8n")))
                .or_else(|| actions.iter().find(|action| matches!(action.kind, ActionKind::Run)))
                .map(|action| action.id.clone()),
            required_services: compose_services,
            required_env_groups: vec!["database".to_string(), "queue".to_string(), "auth".to_string()],
            known_ports: vec![preferred_port.unwrap_or(5678)],
            route_strategy: Some(if compose_file.is_some() {
                RouteStrategy::Hybrid
            } else {
                RouteStrategy::LocalhostDirect
            }),
            summary: Some("Local automation workbench with a primary web UI and optional database or queue backends.".to_string()),
        };
    }

    if lower == "comfyui" {
        return ProjectProfile {
            kind: ProjectProfileKind::AiUi,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command == "python main.py")
                .or_else(|| actions.iter().find(|action| matches!(action.kind, ActionKind::Run)))
                .map(|action| action.id.clone()),
            required_services: Vec::new(),
            required_env_groups: vec!["models".to_string()],
            known_ports: vec![preferred_port.unwrap_or(8188)],
            route_strategy: Some(RouteStrategy::LocalhostDirect),
            summary: Some("Local image generation UI with a fixed Python entrypoint and a well-known localhost port.".to_string()),
        };
    }

    if compose_file.is_some() && !workspace_targets.is_empty() {
        return ProjectProfile {
            kind: ProjectProfileKind::FullstackMixed,
            preferred_entrypoint: actions
                .iter()
                .find(|action| matches!(action.kind, ActionKind::Run))
                .map(|action| action.id.clone()),
            required_services: compose_services,
            required_env_groups: if matches!(runtime_kind, RuntimeKind::Python)
                || workspace_targets
                    .iter()
                    .any(|target| matches!(target.runtime_kind, RuntimeKind::Python))
            {
                vec!["backend".to_string()]
            } else {
                Vec::new()
            },
            known_ports: preferred_port.into_iter().collect(),
            route_strategy: Some(RouteStrategy::Hybrid),
            summary: Some(
                "Mixed-stack repo with multiple app targets and supporting local services."
                    .to_string(),
            ),
        };
    }

    if matches!(project_kind, ProjectKind::Compose) || compose_file.is_some() {
        return ProjectProfile {
            kind: ProjectProfileKind::ComposeStack,
            preferred_entrypoint: actions
                .iter()
                .find(|action| action.command.contains("compose up"))
                .map(|action| action.id.clone()),
            required_services: compose_services,
            required_env_groups: Vec::new(),
            known_ports: preferred_port.into_iter().collect(),
            route_strategy: Some(RouteStrategy::ComposeService),
            summary: Some(
                "Compose-backed local stack with multiple services and published ports."
                    .to_string(),
            ),
        };
    }

    ProjectProfile::default()
}

fn infer_generic_profile_kind(
    runtime_kind: &RuntimeKind,
    project_kind: &ProjectKind,
    compose_file: Option<&Path>,
    workspace_targets: &[DetectedAppTarget],
) -> ProjectProfileKind {
    if compose_file.is_some() && !workspace_targets.is_empty() {
        return ProjectProfileKind::FullstackMixed;
    }

    if matches!(project_kind, ProjectKind::Compose) || compose_file.is_some() {
        return ProjectProfileKind::ComposeStack;
    }

    match runtime_kind {
        RuntimeKind::Node | RuntimeKind::Python => ProjectProfileKind::WebApp,
        _ => ProjectProfileKind::Unknown,
    }
}

fn default_profile_summary(
    kind: &ProjectProfileKind,
    has_compose: bool,
    readme_hints: &[String],
) -> String {
    match kind {
        ProjectProfileKind::AiUi => "Local AI interface with a primary web entrypoint and optional provider or model dependencies.".to_string(),
        ProjectProfileKind::GatewayStack => "Gateway-style localhost platform with multiple runnable surfaces and route-aware entrypoints.".to_string(),
        ProjectProfileKind::ComposeStack => "Multi-service stack that should be observed as one local platform instead of separate repos.".to_string(),
        ProjectProfileKind::FullstackMixed => "Hybrid repo with multiple local entrypoints that PortPilot should guide in the right order.".to_string(),
        ProjectProfileKind::WebApp => {
            if has_compose {
                "Web app with supporting local services and a primary routed entrypoint.".to_string()
            } else if let Some(hint) = readme_hints.first() {
                format!("Web app with a likely primary startup path: {hint}")
            } else {
                "Web app that can be brought online from one local control surface.".to_string()
            }
        }
        ProjectProfileKind::Unknown => "Local project with inferred actions, routes, and runtime controls.".to_string(),
    }
}

fn default_route_strategy(kind: &ProjectProfileKind, has_compose: bool) -> RouteStrategy {
    match kind {
        ProjectProfileKind::GatewayStack => RouteStrategy::GatewayPath,
        ProjectProfileKind::ComposeStack => RouteStrategy::ComposeService,
        ProjectProfileKind::FullstackMixed => RouteStrategy::Hybrid,
        ProjectProfileKind::AiUi | ProjectProfileKind::WebApp => {
            if has_compose {
                RouteStrategy::Hybrid
            } else {
                RouteStrategy::LocalhostDirect
            }
        }
        ProjectProfileKind::Unknown => RouteStrategy::LocalhostDirect,
    }
}

fn apply_recipe_profile_overrides(profile: &mut ProjectProfile, recipe: &ProjectRecipe) {
    if let Some(kind) = recipe.kind.clone() {
        profile.kind = kind;
    }
    if recipe.preferred_entrypoint.is_some() {
        profile.preferred_entrypoint = recipe.preferred_entrypoint.clone();
    }
    if !recipe.required_services.is_empty() {
        profile.required_services = recipe.required_services.clone();
    }
    if !recipe.required_env_groups.is_empty() {
        profile.required_env_groups = recipe.required_env_groups.clone();
    }
    if !recipe.known_ports.is_empty() {
        profile.known_ports = recipe.known_ports.clone();
    }
    if recipe.route_strategy.is_some() {
        profile.route_strategy = recipe.route_strategy.clone();
    }
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

fn merge_recipe_env_keys(fields: &mut Vec<EnvTemplateField>, recipe: &ProjectRecipe) {
    let recipe_fields = recipe
        .env_keys
        .iter()
        .filter_map(|key| {
            let trimmed = key.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(EnvTemplateField {
                key: trimmed.to_string(),
                default_value: None,
                description: Some("Suggested by .portpilot.json".to_string()),
                field_type: EnvFieldType::Text,
            })
        })
        .collect::<Vec<_>>();
    merge_discovered_env_fields(fields, recipe_fields);
}

fn merge_discovered_env_fields(fields: &mut Vec<EnvTemplateField>, extra: Vec<EnvTemplateField>) {
    let mut seen = fields
        .iter()
        .map(|field| field.key.clone())
        .collect::<HashSet<_>>();
    for field in extra {
        if !seen.insert(field.key.clone()) {
            continue;
        }
        fields.push(field);
    }
}

fn parse_compose_env_template(compose_file: &Path) -> Vec<EnvTemplateField> {
    let Ok(contents) = fs::read_to_string(compose_file) else {
        return Vec::new();
    };

    let pattern = Regex::new(r"\$\{([A-Z0-9_]+)(?::-([^}]*))?\}").expect("regex");
    let mut fields = Vec::new();
    let mut seen = HashSet::new();

    for capture in pattern.captures_iter(&contents) {
        let Some(key) = capture.get(1).map(|value| value.as_str().trim()) else {
            continue;
        };
        if key.is_empty() || !seen.insert(key.to_string()) {
            continue;
        }

        let default_value = capture
            .get(2)
            .map(|value| value.as_str().trim().trim_matches('"').to_string())
            .filter(|value| !value.is_empty());
        let upper = key.to_ascii_uppercase();
        let field_type = if upper.contains("TOKEN")
            || upper.contains("SECRET")
            || upper.contains("PASSWORD")
            || upper.contains("KEY")
            || upper.contains("COOKIE")
        {
            EnvFieldType::Secret
        } else if matches!(default_value.as_deref(), Some("true" | "false" | "1" | "0")) {
            EnvFieldType::Boolean
        } else {
            EnvFieldType::Text
        };

        fields.push(EnvTemplateField {
            key: key.to_string(),
            default_value,
            description: Some("Detected from docker-compose configuration".to_string()),
            field_type,
        });
    }

    fields
}

fn apply_recipe_targets(targets: &mut Vec<DetectedAppTarget>, recipe: &ProjectRecipe) {
    for recipe_target in &recipe.targets {
        let Some(target) = targets.iter_mut().find(|candidate| {
            candidate.id == recipe_target.id
                || candidate.relative_path == recipe_target.relative_path
        }) else {
            continue;
        };

        if let Some(priority) = recipe_target.priority {
            target.priority = priority;
        }
        if let Some(port) = recipe_target.suggested_port {
            target.suggested_port = Some(port);
        }
        if let Some(runtime_kind) = recipe_target.runtime_kind.clone() {
            target.runtime_kind = runtime_kind;
        }
    }

    targets.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
}

fn select_primary_target_id(
    targets: &[DetectedAppTarget],
    recipe: Option<&ProjectRecipe>,
) -> Option<String> {
    let Some(recipe) = recipe else {
        return targets.first().map(|target| target.id.clone());
    };

    if let Some(primary_target_id) = &recipe.primary_target_id {
        if let Some(target) = targets.iter().find(|candidate| {
            candidate.id == *primary_target_id || candidate.relative_path == *primary_target_id
        }) {
            return Some(target.id.clone());
        }
    }

    targets.first().map(|target| target.id.clone())
}

fn merge_readme_hints(inferred: Vec<String>, recipe: Option<&ProjectRecipe>) -> Vec<String> {
    let Some(recipe) = recipe else {
        return inferred;
    };

    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    for hint in recipe
        .readme_hints
        .iter()
        .chain(inferred.iter())
        .map(|value| value.trim())
    {
        if hint.is_empty() || !seen.insert(hint.to_string()) {
            continue;
        }
        merged.push(hint.to_string());
    }
    merged.truncate(6);
    merged
}

fn apply_recipe_action_preferences(actions: &mut Vec<ProjectAction>, recipe: &ProjectRecipe) {
    let preferred = [
        recipe.install_action_id.as_ref(),
        recipe.run_action_id.as_ref(),
        recipe.open_action_id.as_ref(),
    ];
    let mut ordered = Vec::new();
    let mut seen = HashSet::new();

    for action_id in preferred.into_iter().flatten() {
        if let Some(action) = actions.iter().find(|item| item.id == *action_id) {
            ordered.push(action.clone());
            seen.insert(action.id.clone());
        }
    }

    ordered.extend(
        actions
            .iter()
            .filter(|action| !seen.contains(&action.id))
            .cloned(),
    );
    *actions = ordered;
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
            for script in collect_preferred_node_run_scripts(&scripts) {
                actions.push(action(
                    &format!("run-{script}"),
                    &label_for_node_run_script(&script),
                    ActionKind::Run,
                    &package_manager.run_script_command(&script),
                    &workdir,
                    port_hint,
                    ActionSource::Inferred,
                ));
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

        actions.extend(infer_auxiliary_root_actions(root, runtime_kind, port_hint));
        for target in workspace_targets {
            actions.extend(infer_target_actions(target));
        }
    } else {
        match runtime_kind {
            RuntimeKind::Python => {
                let requirements = root.join("requirements.txt");
                if requirements.exists() {
                    actions.push(action(
                        "install-pip",
                        "Install",
                        ActionKind::Install,
                        "pip install -r requirements.txt",
                        &workdir,
                        None,
                        ActionSource::Inferred,
                    ));
                } else {
                    actions.push(action(
                        "install-uv",
                        "Install",
                        ActionKind::Install,
                        "uv sync || pip install -e .",
                        &workdir,
                        None,
                        ActionSource::Inferred,
                    ));
                }
                let python_run_command = if root.join("main.py").exists() {
                    "python main.py"
                } else {
                    "uv run . || python -m ."
                };
                actions.push(action(
                    "run-python",
                    "Run",
                    ActionKind::Run,
                    python_run_command,
                    &workdir,
                    port_hint,
                    ActionSource::Inferred,
                ));
            }
            RuntimeKind::Rust => {
                actions.push(action(
                    "run-rust",
                    "Run",
                    ActionKind::Run,
                    "cargo run",
                    &workdir,
                    port_hint,
                    ActionSource::Inferred,
                ));
                actions.push(action(
                    "build-rust",
                    "Build",
                    ActionKind::Build,
                    "cargo build --release",
                    &workdir,
                    None,
                    ActionSource::Inferred,
                ));
            }
            RuntimeKind::Go => {
                let make_targets = read_make_targets(root);
                if make_targets.contains("install") {
                    actions.push(action(
                        "install-go-make",
                        "Install",
                        ActionKind::Install,
                        "make install",
                        &workdir,
                        None,
                        ActionSource::Inferred,
                    ));
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
                actions.push(action(
                    "run-go",
                    "Run",
                    ActionKind::Run,
                    run_command,
                    &workdir,
                    port_hint,
                    ActionSource::Inferred,
                ));
                actions.push(action(
                    "build-go",
                    "Build",
                    ActionKind::Build,
                    build_command,
                    &workdir,
                    None,
                    ActionSource::Inferred,
                ));
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

    actions.extend(infer_make_actions(root, runtime_kind, port_hint));

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

fn infer_auxiliary_root_actions(
    root: &Path,
    primary_runtime_kind: &RuntimeKind,
    port_hint: Option<u16>,
) -> Vec<ProjectAction> {
    let mut actions = Vec::new();
    let workdir = root.to_string_lossy().to_string();

    if !matches!(primary_runtime_kind, RuntimeKind::Python) && has_runtime_pyproject(root) {
        actions.push(action(
            "install-python-backend",
            "Install Backend",
            ActionKind::Install,
            "uv sync || pip install -e .",
            &workdir,
            None,
            ActionSource::Inferred,
        ));

        let mut seen_commands = HashSet::new();
        let scripts = read_pyproject_scripts(root);
        for command in infer_python_run_commands(root, &scripts) {
            if !seen_commands.insert(command.clone()) {
                continue;
            }
            let label = if command.contains("serve") {
                "Serve Backend".to_string()
            } else {
                "Run Backend".to_string()
            };
            actions.push(action(
                &format!("run-python-backend-{}", slugify(&command)),
                &label,
                ActionKind::Run,
                &command,
                &workdir,
                port_hint,
                ActionSource::Inferred,
            ));
        }
    }

    actions
}

fn infer_make_actions(
    root: &Path,
    runtime_kind: &RuntimeKind,
    port_hint: Option<u16>,
) -> Vec<ProjectAction> {
    if matches!(runtime_kind, RuntimeKind::Go) {
        return Vec::new();
    }

    let make_targets = read_make_targets(root);
    if make_targets.is_empty() {
        return Vec::new();
    }

    let workdir = root.to_string_lossy().to_string();
    let mut actions = Vec::new();

    if make_targets.contains("install") {
        actions.push(action(
            "make-install",
            "Make Install",
            ActionKind::Install,
            "make install",
            &workdir,
            None,
            ActionSource::Inferred,
        ));
    }
    for (target, label, kind) in [
        ("startAndBuild", "Compose Start + Build", ActionKind::Run),
        ("start", "Make Start", ActionKind::Run),
        ("run", "Make Run", ActionKind::Run),
        ("stop", "Make Stop", ActionKind::Stop),
        ("remove", "Make Remove", ActionKind::Stop),
    ] {
        if make_targets.contains(target) {
            let is_run = matches!(kind, ActionKind::Run);
            actions.push(action(
                &format!("make-{}", slugify(target)),
                label,
                kind,
                &format!("make {target}"),
                &workdir,
                if is_run { port_hint } else { None },
                ActionSource::Inferred,
            ));
        }
    }

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

    for target in &mut output {
        target.priority = score_target(root, target);
    }

    output.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
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
        Regex::new(r"`([a-z0-9][\w-]*\s+(?:serve|start|dev|gateway)(?:\s+[^`]+)?)`")
            .expect("regex"),
    ];

    let mut hints = Vec::new();
    let mut seen = HashSet::new();
    for pattern in patterns {
        for capture in pattern.captures_iter(&contents) {
            let Some(command) = capture
                .get(1)
                .map(|value| value.as_str().trim().to_string())
            else {
                continue;
            };
            if seen.insert(command.clone()) {
                hints.push((score_readme_hint(&command, &contents), command));
            }
        }
    }

    for line in contents.lines() {
        let normalized = line
            .trim()
            .trim_matches('`')
            .trim_start_matches("- ")
            .trim_start_matches("* ")
            .trim_start_matches("> ")
            .trim();
        if !looks_like_readme_command_hint(normalized) {
            continue;
        }
        let command = normalized.to_string();
        if seen.insert(command.clone()) {
            hints.push((score_readme_hint(&command, &contents), command));
        }
    }

    hints.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    hints
        .into_iter()
        .map(|(_, command)| command)
        .take(4)
        .collect()
}

fn looks_like_readme_command_hint(line: &str) -> bool {
    if line.is_empty() || line.len() > 140 {
        return false;
    }

    let starts_with_tool = [
        "npm ",
        "pnpm ",
        "yarn ",
        "bun ",
        "make ",
        "docker compose ",
        "docker-compose ",
        "python -m ",
        "uv ",
        "node ",
        "open-webui ",
        "openclaw ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix));

    let contains_runtime = [
        " install", " dev", " start", " preview", " serve", " gateway", " run",
    ]
    .iter()
    .any(|needle| line.contains(needle));

    starts_with_tool && contains_runtime
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
            | "requirements.txt"
            | "Cargo.toml"
            | "go.mod"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "compose.yml"
            | "compose.yaml"
            | "deploy-compose.yml"
            | "deploy-compose.yaml"
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
    names
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.exists())
}

pub fn find_compose_file(root: &Path) -> Option<PathBuf> {
    first_existing(
        root,
        &[
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
            "deploy-compose.yml",
            "deploy-compose.yaml",
            "docker/docker-compose.yml",
            "docker/docker-compose.yaml",
            "docker/compose.yml",
            "docker/compose.yaml",
        ],
    )
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
    value
        .get("packageManager")?
        .as_str()
        .map(ToString::to_string)
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

    let prefix = normalized
        .split('*')
        .next()
        .unwrap_or_default()
        .trim_end_matches('/');
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
        suggested_port: infer_port_hint(target_path, find_compose_file(target_path).as_deref()),
        priority: 0,
        available_actions,
    })
}

fn score_readme_hint(command: &str, readme_contents: &str) -> i32 {
    let mut score = 0;
    let command_lower = command.to_ascii_lowercase();
    if command_lower.contains(" install") || command_lower.ends_with(" install") {
        score += 1;
    }
    if command_lower.contains(" dev") || command_lower.contains(" run dev") {
        score += 6;
    }
    if command_lower.contains(" start") {
        score += 5;
    }
    if command_lower.contains("preview") {
        score += 3;
    }
    if command_lower.contains("docker compose up") {
        score += 4;
    }
    if command_lower.contains("serve") {
        score += 4;
    }
    if command_lower.contains("gateway") {
        score += 5;
    }
    if readme_contents.to_ascii_lowercase().contains("quick start") {
        score += 1;
    }
    score
}

fn score_target(root: &Path, target: &DetectedAppTarget) -> i32 {
    let mut score = 0;
    let relative = target.relative_path.to_ascii_lowercase();
    let name = target.name.to_ascii_lowercase();
    let root_name = root
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    if relative.starts_with("apps/") {
        score += 5;
    }
    if relative.starts_with("frontend") || relative.contains("/frontend") {
        score += 4;
    }
    if relative.starts_with("web") || relative.contains("/web") {
        score += 4;
    }
    if name.contains("web") || name.contains("frontend") || name.contains("app") {
        score += 3;
    }
    if relative.starts_with("backend") || relative.contains("/backend") || name.contains("api") {
        score -= 1;
    }
    if root_name.contains(&name) || name.contains(&root_name) {
        score += 1;
    }
    if target
        .available_actions
        .iter()
        .any(|action| action == "dev" || action == "start" || action == "run")
    {
        score += 2;
    }
    if target
        .available_actions
        .iter()
        .any(|action| action == "build")
    {
        score += 1;
    }
    if matches!(target.runtime_kind, RuntimeKind::Node) {
        score += 1;
    }

    score
}

fn target_available_actions(target_path: &Path, runtime_kind: &RuntimeKind) -> Vec<String> {
    match runtime_kind {
        RuntimeKind::Node => {
            let scripts = read_package_scripts(target_path).unwrap_or_default();
            let mut actions = collect_preferred_node_run_scripts(&scripts);
            actions.extend(
                scripts
                    .keys()
                    .filter(|script| is_build_script(script))
                    .cloned(),
            );
            actions
        }
        RuntimeKind::Python => vec!["run".to_string(), "install".to_string()],
        RuntimeKind::Rust => vec!["run".to_string(), "build".to_string()],
        RuntimeKind::Go => {
            let make_targets = read_make_targets(target_path);
            if make_targets.contains("run")
                || make_targets.contains("build")
                || make_targets.contains("install")
            {
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

            for script in collect_preferred_node_run_scripts(&target_scripts) {
                actions.push(action(
                    &format!("workspace-{}-{script}", target.id),
                    &format!("{} {}", label_for_node_run_script(&script), target.name),
                    ActionKind::Run,
                    &package_manager.run_script_command(&script),
                    &target.root_path,
                    target.suggested_port,
                    ActionSource::Inferred,
                ));
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
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed.starts_with('.')
                || trimmed.starts_with('\t')
            {
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
    use std::sync::OnceLock;
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r"--port\s+(\d{2,5})").expect("regex"),
            Regex::new(r"localhost:(\d{2,5})").expect("regex"),
            Regex::new(r"127\.0\.0\.1:(\d{2,5})").expect("regex"),
            Regex::new(r"PORT(?:=|\s+)(\d{2,5})").expect("regex"),
            Regex::new(r"port\s*:\s*(\d{2,5})").expect("regex"),
            Regex::new(r#""(\d{2,5}):\d{2,5}""#).expect("regex"),
            Regex::new(r#"(\d{2,5}):\d{2,5}"#).expect("regex"),
        ]
    });

    let mut files_to_scan = vec![
        root.join("package.json"),
        root.join("server.mjs"),
        root.join("server.js"),
        root.join("src/command-line.js"),
        root.join("backend/start.sh"),
        root.join("Makefile"),
        root.join("README.md"),
        root.join(".env.example"),
        root.join(".env.local.example"),
    ];
    if let Some(compose) = compose_file {
        files_to_scan.push(compose.to_path_buf());
    }

    for file in &files_to_scan {
        if !file.exists() {
            continue;
        }
        let Ok(contents) = fs::read_to_string(file) else {
            continue;
        };
        for pattern in patterns {
            if let Some(capture) = pattern.captures(&contents) {
                if let Some(port) = capture
                    .get(1)
                    .and_then(|value| value.as_str().parse::<u16>().ok())
                {
                    return Some(port);
                }
            }
        }
        if file.ends_with("package.json") {
            // Vite-based apps default to 5173
            if contents.contains("\"vite\"") && contents.contains("\"dev\"") {
                return Some(5173);
            }
            // Next.js defaults to 3000
            if (contents.contains("\"next\"") || contents.contains("next dev"))
                && contents.contains("\"dev\"")
            {
                return Some(3000);
            }
            // Remix defaults to 3000
            if contents.contains("\"remix\"") || contents.contains("remix vite:dev") {
                return Some(3000);
            }
            // Astro defaults to 4321
            if contents.contains("\"astro\"") || contents.contains("astro dev") {
                return Some(4321);
            }
            // Nuxt defaults to 3000
            if contents.contains("\"nuxt\"") || contents.contains("nuxt dev") {
                return Some(3000);
            }
        }
    }

    None
}

fn has_runtime_pyproject(root: &Path) -> bool {
    let path = root.join("pyproject.toml");
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    contents.contains("[project]")
}

fn read_pyproject_scripts(root: &Path) -> Vec<String> {
    let path = root.join("pyproject.toml");
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut in_scripts = false;
    let mut scripts = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_scripts = trimmed == "[project.scripts]";
            continue;
        }
        if !in_scripts || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, _)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_matches('"').trim_matches('\'');
        if !key.is_empty() {
            scripts.push(key.to_string());
        }
    }

    scripts
}

fn infer_python_run_commands(root: &Path, scripts: &[String]) -> Vec<String> {
    let mut commands = Vec::new();
    let mut seen = HashSet::new();

    for hint in infer_readme_hints(root) {
        let normalized = hint.trim().to_string();
        let relevant = normalized.starts_with("uv run ")
            || normalized.starts_with("python -m ")
            || normalized.contains(" serve");
        if relevant && seen.insert(normalized.clone()) {
            commands.push(normalized);
        }
    }

    for script in scripts {
        let command = format!("uv run {script}");
        if seen.insert(command.clone()) {
            commands.push(command);
        }
    }

    if commands.is_empty() {
        commands.push("uv run . || python -m .".to_string());
    }

    commands
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
        detect_workspace_targets, infer_actions, infer_project_from_path,
        parse_env_template_contents, repo_name_from_git_url,
    };
    use crate::core::models::{ActionKind, ProjectProfileKind, RuntimeKind};
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

        assert!(actions
            .iter()
            .any(|action| action.command == "pnpm install"));
        assert!(actions
            .iter()
            .any(|action| action.command == "pnpm run dev"));
        assert!(actions
            .iter()
            .any(|action| action.command == "pnpm run build"));

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

        assert!(actions
            .iter()
            .any(|action| action.command == "make install"));
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
        fs::write(
            root.join("backend/pyproject.toml"),
            "[project]\nname='backend'\n",
        )
        .unwrap();
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
        assert!(targets
            .iter()
            .any(|target| target.relative_path == "backend"));
        assert!(targets
            .iter()
            .any(|target| target.relative_path == "frontend"));
        assert_eq!(targets[0].relative_path, "frontend");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn infers_primary_target_for_mixed_stack_repository() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("frontend")).unwrap();
        fs::create_dir_all(root.join("backend")).unwrap();
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
        fs::write(
            root.join("backend/pyproject.toml"),
            "[project]\nname='backend'\n",
        )
        .unwrap();
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

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.primary_target_id.as_deref(), Some("frontend"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn applies_project_recipe_overrides() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("frontend")).unwrap();
        fs::create_dir_all(root.join("backend")).unwrap();
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
        fs::write(
            root.join("backend/pyproject.toml"),
            "[project]\nname='backend'\n",
        )
        .unwrap();
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
        fs::write(
            root.join(".portpilot.json"),
            r#"{
              "version": 1,
              "primaryTargetId": "backend",
              "kind": "gateway_stack",
              "preferredPort": 5123,
              "preferredEntrypoint": "run-backend-dev",
              "envKeys": ["API_TOKEN"],
              "requiredServices": ["gateway"],
              "requiredEnvGroups": ["credentials"],
              "knownPorts": [5123, 8123],
              "routeStrategy": "gateway_path",
              "readmeHints": ["uv run backend.app:app"],
              "runActionId": "run-backend-dev",
              "targets": [
                {
                  "id": "backend",
                  "relativePath": "backend",
                  "priority": 999,
                  "suggestedPort": 8123
                }
              ]
            }"#,
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.primary_target_id.as_deref(), Some("backend"));
        assert_eq!(project.preferred_port, Some(5123));
        assert!(project
            .detected_files
            .iter()
            .any(|item| item == ".portpilot.json"));
        assert!(project
            .env_template
            .iter()
            .any(|field| field.key == "API_TOKEN"));
        assert_eq!(
            project.readme_hints.first().map(String::as_str),
            Some("uv run backend.app:app")
        );
        assert_eq!(
            project
                .workspace_targets
                .first()
                .map(|target| target.id.as_str()),
            Some("backend")
        );
        assert_eq!(
            project.project_profile.kind,
            ProjectProfileKind::GatewayStack
        );
        assert_eq!(
            project.project_profile.required_services,
            vec!["gateway".to_string()]
        );
        assert_eq!(project.project_profile.known_ports, vec![5123, 8123]);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn adds_python_backend_actions_for_hybrid_root_repository() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("backend")).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "open-webui-like",
              "scripts": {
                "dev": "vite dev --host",
                "preview": "vite preview"
              }
            }"#,
        )
        .unwrap();
        fs::write(
            root.join("pyproject.toml"),
            r#"[project]
name = "open-webui"

[project.scripts]
open-webui = "open_webui:app"
"#,
        )
        .unwrap();
        fs::write(
            root.join("README.md"),
            r#"
```bash
open-webui serve
```
"#,
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert!(project
            .actions
            .iter()
            .any(|action| action.command == "open-webui serve"));
        assert!(project
            .actions
            .iter()
            .any(|action| action.command == "uv run open-webui"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn smokes_real_public_repositories_when_local_clones_are_present() {
        let selftest_root = std::env::var("PORTPILOT_SELFTEST_ROOT").unwrap_or_else(|_| {
            format!(
                "{}/portpilot-selftest/repos",
                std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string())
            )
        });
        let repo_root = Path::new(&selftest_root);
        if !repo_root.exists() {
            return;
        }

        let repos = [
            "Crucix",
            "worldmonitor",
            "vitesse",
            "turborepo-shadcn-ui",
            "example-voting-app",
            "full-stack-fastapi-template",
            "pagoda",
            "SillyTavern",
            "open-webui",
            "openclaw",
            "LibreChat",
            "anything-llm",
            "Flowise",
            "ComfyUI",
        ];

        for repo in repos {
            let path = repo_root.join(repo);
            if !path.exists() {
                continue;
            }

            let project = infer_project_from_path(&path, None, 42300)
                .unwrap_or_else(|| panic!("failed to infer project for {repo}"));
            assert!(
                !project.actions.is_empty(),
                "expected inferred actions for {repo}"
            );

            match repo {
                "vitesse" => assert!(
                    project
                        .actions
                        .iter()
                        .any(|action| action.command.starts_with("pnpm ")),
                    "expected pnpm commands for vitesse"
                ),
                "turborepo-shadcn-ui" => assert!(
                    project.primary_target_id.is_some() && !project.workspace_targets.is_empty(),
                    "expected a recommended primary target for turborepo-shadcn-ui"
                ),
                "example-voting-app" => assert!(
                    project.has_docker_compose
                        && project.project_profile.kind == ProjectProfileKind::ComposeStack,
                    "expected compose profile detection for example-voting-app"
                ),
                "SillyTavern" => {
                    assert_eq!(project.preferred_port, Some(8000));
                    assert_eq!(project.project_profile.kind, ProjectProfileKind::AiUi);
                    assert!(
                        project
                            .actions
                            .iter()
                            .any(|action| action.command.contains("start")),
                        "expected runnable start command for SillyTavern"
                    );
                }
                "open-webui" => {
                    assert!(
                        project.has_docker_compose,
                        "expected compose detection for open-webui"
                    );
                    assert_eq!(project.project_profile.kind, ProjectProfileKind::AiUi);
                    assert!(
                        project
                            .actions
                            .iter()
                            .any(|action| action.command == "open-webui serve"
                                || action.command == "make start"
                                || action.command == "make startAndBuild"),
                        "expected backend or compose entrypoint for open-webui"
                    );
                    assert!(
                        project
                            .readme_hints
                            .iter()
                            .any(|hint| hint.contains("open-webui serve")),
                        "expected serve hint for open-webui"
                    );
                    assert!(
                        project
                            .project_profile
                            .required_services
                            .iter()
                            .any(|service| service == "open-webui"),
                        "expected open-webui service requirement"
                    );
                }
                "openclaw" => {
                    assert!(
                        project.has_docker_compose,
                        "expected compose detection for openclaw"
                    );
                    assert_eq!(project.preferred_port, Some(18789));
                    assert_eq!(
                        project.project_profile.kind,
                        ProjectProfileKind::GatewayStack
                    );
                    assert!(
                        project
                            .actions
                            .iter()
                            .any(|action| action.command == "pnpm run gateway:dev"),
                        "expected gateway dev action for openclaw"
                    );
                    assert!(
                        project
                            .env_template
                            .iter()
                            .any(|field| field.key == "OPENCLAW_CONFIG_DIR")
                            && project
                                .env_template
                                .iter()
                                .any(|field| field.key == "OPENCLAW_WORKSPACE_DIR"),
                        "expected compose env requirements for openclaw"
                    );
                    assert_eq!(project.project_profile.known_ports, vec![18789, 18790]);
                }
                "LibreChat" => {
                    assert!(
                        project.has_docker_compose,
                        "expected compose detection for LibreChat"
                    );
                    assert_eq!(project.preferred_port, Some(3080));
                    assert_eq!(
                        project.project_profile.kind,
                        ProjectProfileKind::GatewayStack
                    );
                    assert!(
                        project
                            .project_profile
                            .required_services
                            .iter()
                            .any(|service| service == "api" || service == "mongodb"),
                        "expected LibreChat services to be inferred"
                    );
                    assert!(
                        project.actions.iter().any(|action| action
                            .command
                            .contains("frontend:dev")
                            || action.command.contains("backend:dev")
                            || action.command.contains("compose up")),
                        "expected LibreChat runnable entrypoint"
                    );
                }
                "anything-llm" => {
                    assert!(
                        project.has_docker_compose,
                        "expected compose detection for anything-llm"
                    );
                    assert_eq!(project.preferred_port, Some(3001));
                    assert_eq!(project.project_profile.kind, ProjectProfileKind::AiUi);
                    assert!(
                        project
                            .actions
                            .iter()
                            .any(|action| action.command.contains("dev:all")),
                        "expected anything-llm dev:all entrypoint"
                    );
                    assert!(
                        project
                            .project_profile
                            .required_services
                            .iter()
                            .any(|service| service == "anything-llm"),
                        "expected anything-llm service requirement"
                    );
                }
                "Flowise" => {
                    assert!(
                        project.has_docker_compose,
                        "expected compose detection for Flowise"
                    );
                    assert_eq!(project.preferred_port, Some(3000));
                    assert_eq!(project.project_profile.kind, ProjectProfileKind::AiUi);
                    assert!(
                        project
                            .actions
                            .iter()
                            .any(|action| action.command == "pnpm run start"
                                || action.command.contains("compose up")),
                        "expected Flowise start entrypoint"
                    );
                    assert!(
                        project.env_template.iter().any(|field| field.key == "PORT"),
                        "expected Flowise compose env requirements"
                    );
                }
                "ComfyUI" => {
                    assert_eq!(project.preferred_port, Some(8188));
                    assert_eq!(project.project_profile.kind, ProjectProfileKind::AiUi);
                    assert!(
                        project
                            .actions
                            .iter()
                            .any(|action| action.command == "python main.py"),
                        "expected ComfyUI python main entrypoint"
                    );
                }
                "pagoda" => assert!(
                    project
                        .actions
                        .iter()
                        .any(|action| action.command == "make run" || action.command == "go run ."),
                    "expected runnable Go command for pagoda"
                ),
                _ => {}
            }
        }
    }

    #[test]
    fn applies_recipe_overrides_for_real_public_repo_when_present() {
        let selftest_root = std::env::var("PORTPILOT_SELFTEST_ROOT").unwrap_or_else(|_| {
            format!(
                "{}/portpilot-selftest/repos",
                std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string())
            )
        });
        let path = Path::new(&selftest_root).join("vitesse");
        if !path.join(".portpilot.json").exists() {
            return;
        }

        let project =
            infer_project_from_path(&path, None, 42300).expect("expected vitesse inference");
        assert_eq!(project.preferred_port, Some(4517));
        assert!(project
            .env_template
            .iter()
            .any(|field| field.key == "VITE_SELFTEST"));
        assert_eq!(
            project.readme_hints.first().map(String::as_str),
            Some("pnpm run dev -- --host 127.0.0.1 --port 4517")
        );
        assert!(project
            .detected_files
            .iter()
            .any(|item| item == ".portpilot.json"));
    }

    // ── New project / local-port tests ────────────────────────────────────────

    /// Next.js projects should be detected as Node, default port 3000.
    #[test]
    fn infers_nextjs_project_with_port_3000() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "my-next-app",
              "scripts": {
                "dev": "next dev",
                "build": "next build",
                "start": "next start"
              },
              "dependencies": {
                "next": "^14.0.0",
                "react": "^18.0.0"
              }
            }"#,
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.runtime_kind, RuntimeKind::Node);
        assert_eq!(project.preferred_port, Some(3000));
        assert!(
            project.actions.iter().any(|a| a.command == "npm run dev"),
            "expected npm run dev action"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// Astro projects should be detected as Node, default port 4321.
    #[test]
    fn infers_astro_project_with_port_4321() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "my-astro-site",
              "scripts": {
                "dev": "astro dev",
                "build": "astro build",
                "preview": "astro preview"
              },
              "dependencies": {
                "astro": "^4.0.0"
              }
            }"#,
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.runtime_kind, RuntimeKind::Node);
        assert_eq!(project.preferred_port, Some(4321));
        assert!(
            project.actions.iter().any(|a| a.command == "npm run dev"),
            "expected npm run dev action"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// Remix projects (Vite-based) should be detected as Node, port 3000.
    #[test]
    fn infers_remix_project_with_port_3000() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "my-remix-app",
              "scripts": {
                "dev": "remix vite:dev",
                "build": "remix vite:build",
                "start": "remix-serve ./build/server/index.js"
              },
              "dependencies": {
                "@remix-run/node": "^2.0.0",
                "@remix-run/react": "^2.0.0"
              }
            }"#,
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.runtime_kind, RuntimeKind::Node);
        assert_eq!(project.preferred_port, Some(3000));
        let _ = fs::remove_dir_all(root);
    }

    /// Lobe Chat (pnpm, Next.js) should resolve to port 3210 via builtin_default_port.
    #[test]
    fn infers_lobe_chat_builtin_port_3210() {
        let root = std::env::temp_dir()
            .join(format!("portpilot-test-{}", uuid::Uuid::new_v4()))
            .join("lobe-chat");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "lobe-chat",
              "packageManager": "pnpm@9.0.0",
              "scripts": {
                "dev": "next dev",
                "build": "next build",
                "start": "next start"
              },
              "dependencies": {
                "next": "^14.0.0"
              }
            }"#,
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        // builtin_default_port("lobe-chat") = 3210 takes precedence over Next.js 3000
        assert_eq!(project.preferred_port, Some(3210));
        assert_eq!(project.project_profile.kind, ProjectProfileKind::AiUi);
        assert!(
            project.actions.iter().any(|a| a.command.contains("pnpm")),
            "expected pnpm commands for lobe-chat"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// n8n automation should resolve to its builtin port 5678.
    #[test]
    fn infers_n8n_builtin_port_5678() {
        let root = std::env::temp_dir()
            .join(format!("portpilot-test-{}", uuid::Uuid::new_v4()))
            .join("n8n");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "n8n",
              "scripts": {
                "dev": "turbo run dev",
                "start": "node packages/cli/bin/n8n"
              }
            }"#,
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.preferred_port, Some(5678));
        assert_eq!(project.project_profile.kind, ProjectProfileKind::WebApp);
        let _ = fs::remove_dir_all(root);
    }

    /// A Python project with only requirements.txt should be inferred as Python.
    #[test]
    fn detects_python_project_from_requirements_txt_only() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("requirements.txt"),
            "fastapi>=0.100\nuvicorn[standard]>=0.23\n",
        )
        .unwrap();
        fs::write(root.join("main.py"), "import uvicorn\n").unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.runtime_kind, RuntimeKind::Python);
        assert!(
            project
                .detected_files
                .iter()
                .any(|f| f == "requirements.txt"),
            "requirements.txt should appear in detected_files"
        );
        assert!(
            project
                .actions
                .iter()
                .any(|a| a.command == "python main.py"),
            "expected python main.py run action"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// A FastAPI project with requirements.txt should produce a pip install action.
    #[test]
    fn infers_fastapi_install_from_requirements_txt() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("requirements.txt"),
            "fastapi>=0.100\nuvicorn[standard]>=0.23\nhttpx\n",
        )
        .unwrap();
        fs::write(root.join("main.py"), "from fastapi import FastAPI\n").unwrap();

        let actions = infer_actions(
            &root,
            &RuntimeKind::Python,
            Some(8000),
            None,
            "http://gateway.localhost:42300/p/api/",
            &[],
        );

        assert!(
            actions
                .iter()
                .any(|a| a.command == "pip install -r requirements.txt"),
            "expected pip install action for requirements.txt project"
        );
        assert!(
            actions.iter().any(|a| a.command == "python main.py"),
            "expected python main.py run action"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// Langflow port should resolve via builtin_default_port to 7860.
    #[test]
    fn infers_langflow_builtin_port_7860() {
        let root = std::env::temp_dir()
            .join(format!("portpilot-test-{}", uuid::Uuid::new_v4()))
            .join("langflow");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("pyproject.toml"),
            "[project]\nname = \"langflow\"\nversion = \"1.0.0\"\n\n[project.scripts]\nlangflow = \"langflow.__main__:main\"\n",
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.runtime_kind, RuntimeKind::Python);
        assert_eq!(project.preferred_port, Some(7860));
        assert_eq!(project.project_profile.kind, ProjectProfileKind::AiUi);
        let _ = fs::remove_dir_all(root);
    }

    /// LocalAI should resolve to port 8080 and use an AI UI profile.
    #[test]
    fn infers_localai_builtin_port_8080() {
        let root = std::env::temp_dir()
            .join(format!("portpilot-test-{}", uuid::Uuid::new_v4()))
            .join("LocalAI");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("docker-compose.yml"),
            "services:\n  localai:\n    image: localai/localai:latest\n    ports:\n      - '8080:8080'\n",
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.preferred_port, Some(8080));
        assert_eq!(project.project_profile.kind, ProjectProfileKind::AiUi);
        let _ = fs::remove_dir_all(root);
    }

    /// stable-diffusion-webui should resolve to port 7860 via builtin_default_port.
    #[test]
    fn infers_stable_diffusion_webui_builtin_port_7860() {
        let root = std::env::temp_dir()
            .join(format!("portpilot-test-{}", uuid::Uuid::new_v4()))
            .join("stable-diffusion-webui");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("requirements.txt"),
            "torch\ntorchvision\ngradio\npillow\n",
        )
        .unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.runtime_kind, RuntimeKind::Python);
        assert_eq!(project.preferred_port, Some(7860));
        let _ = fs::remove_dir_all(root);
    }

    /// A full-stack Next.js + FastAPI monorepo should infer two targets.
    #[test]
    fn infers_nextjs_fastapi_monorepo_targets() {
        let root = std::env::temp_dir().join(format!("portpilot-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("frontend")).unwrap();
        fs::create_dir_all(root.join("backend")).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{
              "name": "fullstack",
              "scripts": {
                "dev": "concurrently \"npm run dev --prefix frontend\" \"uvicorn backend.main:app\""
              }
            }"#,
        )
        .unwrap();
        fs::write(
            root.join("frontend/package.json"),
            r#"{
              "name": "frontend",
              "scripts": {
                "dev": "next dev",
                "build": "next build"
              },
              "dependencies": { "next": "^14.0.0" }
            }"#,
        )
        .unwrap();
        fs::write(root.join("backend/requirements.txt"), "fastapi\nuvicorn\n").unwrap();

        let targets = detect_workspace_targets(&root);
        assert!(
            targets.iter().any(|t| t.relative_path == "frontend"),
            "expected frontend target"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// ComfyUI port should resolve to 8188 via builtin_default_port.
    #[test]
    fn infers_comfyui_builtin_port_8188() {
        let root = std::env::temp_dir()
            .join(format!("portpilot-test-{}", uuid::Uuid::new_v4()))
            .join("ComfyUI");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("requirements.txt"),
            "torch\ntorchvision\nPillow\naiohttp\n",
        )
        .unwrap();
        fs::write(root.join("main.py"), "# ComfyUI entry point\n").unwrap();

        let project = infer_project_from_path(&root, None, 42300).unwrap();
        assert_eq!(project.preferred_port, Some(8188));
        assert!(
            project
                .actions
                .iter()
                .any(|a| a.command == "python main.py"),
            "expected python main.py for ComfyUI"
        );
        let _ = fs::remove_dir_all(root);
    }
}
