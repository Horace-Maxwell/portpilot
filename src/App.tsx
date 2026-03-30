import { useEffect, useMemo, useState } from "preact/hooks";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./App.css";
import { useUpdate } from "./contexts/UpdateContext";
import { api } from "./lib/tauri";
import { getCurrentVersion } from "./lib/updater";
import type {
  ActionExecution,
  ActionKind,
  DoctorReport,
  ImportedRepo,
  LogEntry,
  ManagedProject,
  PortLease,
  ProjectAction,
  RouteBinding,
} from "./shared/types";

type NavKey =
  | "dashboard"
  | "import"
  | "projects"
  | "routes"
  | "ports"
  | "logs"
  | "settings";

const NAV_ITEMS: Array<{ key: NavKey; label: string }> = [
  { key: "dashboard", label: "Dashboard" },
  { key: "import", label: "Import" },
  { key: "projects", label: "Projects" },
  { key: "routes", label: "Routes" },
  { key: "ports", label: "Ports" },
  { key: "logs", label: "Logs" },
  { key: "settings", label: "Settings" },
];

export default function App() {
  const [view, setView] = useState<NavKey>("dashboard");
  const [workspaceRoots, setWorkspaceRoots] = useState<string[]>([]);
  const [projects, setProjects] = useState<ManagedProject[]>([]);
  const [candidates, setCandidates] = useState<ImportedRepo[]>([]);
  const [executions, setExecutions] = useState<ActionExecution[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [ports, setPorts] = useState<PortLease[]>([]);
  const [routes, setRoutes] = useState<RouteBinding[]>([]);
  const [doctorReports, setDoctorReports] = useState<Record<string, DoctorReport>>({});
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [embedUrl, setEmbedUrl] = useState<string | null>(null);
  const [importUrl, setImportUrl] = useState("https://github.com/calesthio/Crucix.git");
  const [importRootDraft, setImportRootDraft] = useState("/Users/horacedong/Desktop/Github");
  const [workspaceDraft, setWorkspaceDraft] = useState("");
  const [statusMessage, setStatusMessage] = useState("Booting PortPilot...");
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [envValues, setEnvValues] = useState<Record<string, string>>({});
  const [envRawText, setEnvRawText] = useState("");
  const [currentVersion, setCurrentVersion] = useState("0.1.0");
  const update = useUpdate();

  const selectedProject = useMemo(
    () => projects.find((project) => project.id === selectedProjectId) ?? null,
    [projects, selectedProjectId],
  );
  const selectedLogs = useMemo(
    () =>
      selectedProject
        ? logs.filter((entry) =>
            executions.some(
              (execution) =>
                execution.id === entry.execution_id &&
                execution.project_id === selectedProject.id,
            ),
          )
        : logs,
    [executions, logs, selectedProject],
  );
  const selectedExecutions = useMemo(
    () =>
      selectedProject
        ? executions.filter((execution) => execution.project_id === selectedProject.id)
        : executions,
    [executions, selectedProject],
  );

  useEffect(() => {
    void refreshAll();

    const unlisteners: Array<() => void> = [];
    const subscribe = async () => {
      unlisteners.push(
        await listen("repo-import-progress", (event) => {
          const payload = event.payload as { stage?: string; destination?: string };
          setStatusMessage(
            payload.stage
              ? `Repository import: ${payload.stage}${payload.destination ? ` (${payload.destination})` : ""}`
              : "Repository import updated.",
          );
          void refreshProjectsOnly();
        }),
      );
      unlisteners.push(
        await listen<ActionExecution>("action-started", (event) => {
          setExecutions((current) => upsertById(current, event.payload));
          setStatusMessage(`Started ${event.payload.label}`);
          void refreshProjectsOnly();
        }),
      );
      unlisteners.push(
        await listen<ActionExecution>("action-finished", (event) => {
          setExecutions((current) => upsertById(current, event.payload));
          setStatusMessage(`Finished ${event.payload.label} with ${event.payload.status}`);
          void refreshProjectsOnly();
        }),
      );
      unlisteners.push(
        await listen<LogEntry>("action-log", (event) => {
          setLogs((current) => [...current.slice(-499), event.payload]);
        }),
      );
    };

    void subscribe();
    return () => {
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, []);

  useEffect(() => {
    if (!selectedProjectId && projects[0]) {
      setSelectedProjectId(projects[0].id);
    }
  }, [projects, selectedProjectId]);

  useEffect(() => {
    if (!selectedProject) {
      setEnvValues({});
      setEnvRawText("");
      return;
    }

    const nextValues: Record<string, string> = {};
    for (const field of selectedProject.env_template) {
      nextValues[field.key] =
        selectedProject.env_profile.values[field.key] ?? field.default_value ?? "";
    }
    for (const [key, value] of Object.entries(selectedProject.env_profile.values)) {
      nextValues[key] = value;
    }
    setEnvValues(nextValues);
    setEnvRawText(selectedProject.env_profile.raw_editor_text ?? "");
  }, [selectedProject]);

  useEffect(() => {
    if (!selectedProject) {
      return;
    }

    void api
      .getDoctorReport(selectedProject.id)
      .then((report) => {
        setDoctorReports((current) => ({
          ...current,
          [selectedProject.id]: report,
        }));
      })
      .catch((error) => {
        setStatusMessage(error instanceof Error ? error.message : String(error));
      });
  }, [selectedProject?.id, selectedProject?.updated_at]);

  useEffect(() => {
    void getCurrentVersion().then((value) => {
      if (value) {
        setCurrentVersion(value);
      }
    });
  }, []);

  const runningProjects = projects.filter((project) => project.status === "running").length;
  const selectedDoctorReport = selectedProject ? doctorReports[selectedProject.id] ?? null : null;

  async function refreshAll() {
    const [roots, nextProjects, nextExecutions, nextLogs, nextPorts, nextRoutes] =
      await Promise.all([
        api.listWorkspaceRoots(),
        api.listProjects(),
        api.listActionExecutions(),
        api.getProjectLogs(),
        api.listPorts(),
        api.listRoutes(),
      ]);
    setWorkspaceRoots(roots);
    setWorkspaceDraft(roots.join("\n"));
    setImportRootDraft(roots[0] ?? "/Users/horacedong/Desktop/Github");
    setProjects(nextProjects);
    setExecutions(nextExecutions);
    setLogs(nextLogs);
    setPorts(nextPorts);
    setRoutes(nextRoutes);
    setStatusMessage(`Loaded ${nextProjects.length} project${nextProjects.length === 1 ? "" : "s"}.`);
  }

  async function refreshProjectsOnly() {
    const [nextProjects, nextExecutions, nextPorts, nextRoutes] = await Promise.all([
      api.listProjects(),
      api.listActionExecutions(),
      api.listPorts(),
      api.listRoutes(),
    ]);
    setProjects(nextProjects);
    setExecutions(nextExecutions);
    setPorts(nextPorts);
    setRoutes(nextRoutes);
  }

  async function handleScan() {
    await runBusy("scan", async () => {
      const nextCandidates = await api.scanLocalProjects(workspaceRoots);
      setCandidates(nextCandidates);
      setView("import");
      setStatusMessage(`Found ${nextCandidates.length} candidate repositories.`);
    });
  }

  async function handleImportGit() {
    await runBusy("import", async () => {
      const project = await api.importRepoFromGit(importUrl, importRootDraft);
      setStatusMessage(`Imported ${project.name}.`);
      await refreshAll();
      setSelectedProjectId(project.id);
      setView("projects");
    });
  }

  async function handleRegisterCandidate(candidate: ImportedRepo) {
    await runBusy(`register-${candidate.root_path}`, async () => {
      const project = await api.registerLocalProject(candidate.root_path, candidate.git_url);
      setStatusMessage(`Registered ${project.name}.`);
      await refreshAll();
      setSelectedProjectId(project.id);
      setView("projects");
    });
  }

  async function handleRunAction(project: ManagedProject, action: ProjectAction) {
    await runBusy(`${project.id}-${action.id}`, async () => {
      if (action.kind === "open") {
        setEmbedUrl(project.route_path_url);
        await openUrl(project.route_path_url);
        return;
      }
      await api.runProjectAction(project.id, action.id);
      await refreshProjectsOnly();
    });
  }

  async function handleRestart(project: ManagedProject, action: ProjectAction) {
    await runBusy(`restart-${project.id}`, async () => {
      await api.restartProject(project.id, action.id);
      await refreshProjectsOnly();
    });
  }

  async function handleStop(project: ManagedProject) {
    const running = executions.find(
      (execution) => execution.project_id === project.id && execution.status === "running",
    );
    if (!running) {
      setStatusMessage(`No active execution found for ${project.name}.`);
      return;
    }
    await runBusy(`stop-${running.id}`, async () => {
      await api.stopActionExecution(running.id);
      await refreshProjectsOnly();
    });
  }

  async function handleSaveEnv() {
    if (!selectedProject) return;
    await runBusy(`env-${selectedProject.id}`, async () => {
      const updated = await api.saveEnvProfile(
        selectedProject.id,
        envValues,
        envRawText.trim() ? envRawText : null,
      );
      setStatusMessage(`Saved .env for ${updated.name}.`);
      await refreshAll();
    });
  }

  async function handleSaveRoots() {
    const roots = workspaceDraft
      .split("\n")
      .map((value) => value.trim())
      .filter(Boolean);
    await runBusy("roots", async () => {
      const saved = await api.setWorkspaceRoots(roots);
      setWorkspaceRoots(saved);
      setStatusMessage(`Saved ${saved.length} workspace root${saved.length === 1 ? "" : "s"}.`);
    });
  }

  async function handleCheckUpdate() {
    await runBusy("check-update", async () => {
      const available = await update.checkUpdate();
      setStatusMessage(
        available
          ? `Update ${update.updateInfo?.availableVersion ?? ""} is ready to install.`
          : "PortPilot is already up to date.",
      );
    });
  }

  async function handleInstallUpdate() {
    await runBusy("install-update", async () => {
      try {
        update.resetDismiss();
        await update.installUpdate();
      } catch (reason) {
        setStatusMessage(reason instanceof Error ? reason.message : String(reason));
        await openUrl("https://github.com/Horace-Maxwell/portpilot/releases");
      }
    });
  }

  async function runBusy(key: string, task: () => Promise<void>) {
    try {
      setBusyKey(key);
      await task();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatusMessage(message);
    } finally {
      setBusyKey(null);
    }
  }

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand__eyebrow">GitHub Repo Console</span>
          <h1>PortPilot</h1>
          <p>Import, configure, run, stop, route, and package local-first repositories from one desktop cockpit.</p>
        </div>

        <div className="stats">
          <StatCard label="Managed" value={String(projects.length)} />
          <StatCard label="Running" value={String(runningProjects)} />
          <StatCard label="Routes" value={String(routes.length)} />
          <StatCard label="Ports" value={String(ports.length)} />
        </div>

        <nav className="nav">
          {NAV_ITEMS.map((item) => (
            <button
              key={item.key}
              className={`nav__item ${view === item.key ? "is-active" : ""}`}
              onClick={() => setView(item.key)}
              type="button"
            >
              {item.label}
            </button>
          ))}
        </nav>

        <div className="sidebar__footer">
          <button className="secondary-button" onClick={() => void refreshAll()} type="button">
            Refresh
          </button>
          <button className="primary-button" onClick={() => void handleScan()} type="button">
            Scan Roots
          </button>
          <p>{statusMessage}</p>
        </div>
      </aside>

      <main className="main">
        <header className="hero">
          <div>
            <span className="hero__eyebrow">PortPilot v1</span>
            <h2>One-click control for repos like Crucix and WorldMonitor</h2>
          </div>
          <div className="hero__actions">
            <button className="secondary-button" onClick={() => setView("import")} type="button">
              Import Repo
            </button>
            <button className="primary-button" onClick={() => setView("projects")} type="button">
              Open Projects
            </button>
          </div>
        </header>

        {view === "dashboard" && (
          <section className="panel-grid">
            <section className="panel panel--wide">
              <div className="panel__header">
                <h3>Command Deck</h3>
                <p>The fastest path from clone to live route.</p>
              </div>
              <div className="project-grid">
                {projects.map((project) => {
                  const runAction = firstAction(project, "run");
                  const buildAction = firstAction(project, "build");
                  const deployAction = firstAction(project, "deploy");
                  return (
                    <article key={project.id} className="project-card">
                      <div className="project-card__top">
                        <div>
                          <span className={`status-pill status-pill--${project.status}`}>
                            {project.status.replace("_", " ")}
                          </span>
                          <h4>{project.name}</h4>
                          <p>{project.runtime_kind} / {project.project_kind}</p>
                        </div>
                        <button
                          className="ghost-button"
                          onClick={() => {
                            setSelectedProjectId(project.id);
                            setView("projects");
                          }}
                          type="button"
                        >
                          Details
                        </button>
                      </div>
                      <div className="project-card__meta">
                        <span>{project.route_path_url}</span>
                        <span>{project.resolved_port ?? project.preferred_port ?? "No port hint"}</span>
                      </div>
                      <div className="action-row">
                        {runAction && (
                          <button className="primary-button" onClick={() => void handleRunAction(project, runAction)} type="button">
                            Run
                          </button>
                        )}
                        <button className="secondary-button" onClick={() => void handleStop(project)} type="button">
                          Stop
                        </button>
                        <button
                          className="secondary-button"
                          onClick={() => {
                            setEmbedUrl(project.route_path_url);
                            void openUrl(project.route_path_url);
                          }}
                          type="button"
                        >
                          Open
                        </button>
                      </div>
                      <div className="action-row">
                        {buildAction && (
                          <button className="ghost-button" onClick={() => void handleRunAction(project, buildAction)} type="button">
                            Build
                          </button>
                        )}
                        {deployAction && (
                          <button className="ghost-button" onClick={() => void handleRunAction(project, deployAction)} type="button">
                            Deploy
                          </button>
                        )}
                      </div>
                    </article>
                  );
                })}
              </div>
            </section>

            <section className="panel">
              <div className="panel__header">
                <h3>Live Preview</h3>
                <p>Open the routed app inside PortPilot when you want quick verification.</p>
              </div>
              {embedUrl ? (
                <iframe className="embed-frame" src={embedUrl} title="Embedded project preview" />
              ) : (
                <EmptyState
                  title="No embedded target yet"
                  description="Open a managed route and it will appear here."
                />
              )}
            </section>
          </section>
        )}

        {view === "import" && (
          <section className="panel-grid panel-grid--double">
            <section className="panel">
              <div className="panel__header">
                <h3>Import from GitHub</h3>
                <p>Paste a repository URL, clone it into your workspace, then let PortPilot infer actions and env.</p>
              </div>
              <label className="field">
                <span>GitHub URL</span>
                <input value={importUrl} onInput={(event) => setImportUrl(event.currentTarget.value)} />
              </label>
              <label className="field">
                <span>Workspace Root</span>
                <input
                  value={importRootDraft}
                  onInput={(event) => setImportRootDraft(event.currentTarget.value)}
                />
              </label>
              <button
                className="primary-button"
                disabled={busyKey === "import"}
                onClick={() => void handleImportGit()}
                type="button"
              >
                Clone + Register
              </button>
            </section>

            <section className="panel">
              <div className="panel__header">
                <h3>Scan Existing Roots</h3>
                <p>PortPilot detects Node, Python, Rust, Go, and Compose repos already on disk.</p>
              </div>
              <button
                className="secondary-button"
                disabled={busyKey === "scan"}
                onClick={() => void handleScan()}
                type="button"
              >
                Scan Now
              </button>
              <div className="candidate-list">
                {candidates.map((candidate) => (
                  <article key={candidate.root_path} className="candidate-card">
                    <div>
                      <h4>{candidate.name}</h4>
                      <p>{candidate.root_path}</p>
                      {candidate.readme_hints[0] && (
                        <p className="candidate-card__hint">README hint: {candidate.readme_hints[0]}</p>
                      )}
                    </div>
                    <div className="candidate-card__meta">
                      <span>{candidate.runtime_kind}</span>
                      <span>port {candidate.suggested_port ?? "?"}</span>
                      {candidate.workspace_target_count > 0 && (
                        <span>{candidate.workspace_target_count} app targets</span>
                      )}
                      {candidate.has_env_template && <span>.env template</span>}
                      {candidate.has_docker_compose && <span>compose</span>}
                    </div>
                    <button className="primary-button" onClick={() => void handleRegisterCandidate(candidate)} type="button">
                      Register
                    </button>
                  </article>
                ))}
                {candidates.length === 0 && (
                  <EmptyState
                    title="No scan results yet"
                    description="Use Scan Now to discover repos across your configured workspace roots."
                  />
                )}
              </div>
            </section>
          </section>
        )}

        {view === "projects" && (
          <section className="detail-layout">
            <aside className="project-list panel">
              <div className="panel__header">
                <h3>Projects</h3>
                <p>Switch between managed repositories and action groups.</p>
              </div>
              {projects.map((project) => (
                <button
                  key={project.id}
                  className={`project-list__item ${selectedProjectId === project.id ? "is-active" : ""}`}
                  onClick={() => setSelectedProjectId(project.id)}
                  type="button"
                >
                  <div>
                    <strong>{project.name}</strong>
                    <span>{project.runtime_kind}</span>
                  </div>
                  <span className={`status-dot status-dot--${project.status}`} />
                </button>
              ))}
            </aside>

            <section className="panel panel--wide">
              {selectedProject ? (
                <>
                  <div className="panel__header">
                    <div>
                      <h3>{selectedProject.name}</h3>
                      <p>{selectedProject.root_path}</p>
                    </div>
                    <div className="action-row">
                      <button className="secondary-button" onClick={() => void openUrl(selectedProject.route_path_url)} type="button">
                        Open Route
                      </button>
                      {firstAction(selectedProject, "run") && (
                        <button
                          className="primary-button"
                          onClick={() => {
                            const runAction = firstAction(selectedProject, "run");
                            if (runAction) {
                              void handleRestart(selectedProject, runAction);
                            }
                          }}
                          type="button"
                        >
                          Restart Primary Run
                        </button>
                      )}
                    </div>
                  </div>

                  <div className="section-grid">
                    <section className="subpanel">
                      <h4>Overview</h4>
                      <Definition label="Status" value={selectedProject.status} />
                      <Definition label="Port" value={String(selectedProject.resolved_port ?? selectedProject.preferred_port ?? "Unknown")} />
                      <Definition label="Subdomain" value={selectedProject.route_subdomain_url} />
                      <Definition label="Path Route" value={selectedProject.route_path_url} />
                      <Definition label="Detected" value={selectedProject.detected_files.join(", ")} />
                      <Definition
                        label="App Targets"
                        value={
                          selectedProject.workspace_targets.length > 0
                            ? String(selectedProject.workspace_targets.length)
                            : "Single app"
                        }
                      />
                      {selectedProject.readme_hints.length > 0 && (
                        <Definition
                          label="README Hints"
                          value={selectedProject.readme_hints.slice(0, 2).join(" • ")}
                        />
                      )}
                    </section>

                    <section className="subpanel">
                      <h4>Setup / Doctor</h4>
                      {selectedDoctorReport ? (
                        <>
                          <SetupWizard
                            project={selectedProject}
                            report={selectedDoctorReport}
                            onInstall={() => {
                              const action = selectedProject.actions.find(
                                (item) => item.id === selectedDoctorReport.install_action_id,
                              );
                              if (action) {
                                void handleRunAction(selectedProject, action);
                              }
                            }}
                            onRun={() => {
                              const action = selectedProject.actions.find(
                                (item) => item.id === selectedDoctorReport.run_action_id,
                              );
                              if (action) {
                                void handleRunAction(selectedProject, action);
                              }
                            }}
                            onOpen={() => {
                              setEmbedUrl(selectedProject.route_path_url);
                              void openUrl(selectedProject.route_path_url);
                            }}
                            onFocusEnv={() => {
                              const envSection = document.getElementById("env-editor");
                              envSection?.scrollIntoView({ behavior: "smooth", block: "start" });
                            }}
                          />
                          <DoctorChecks checks={selectedDoctorReport.checks} />
                        </>
                      ) : (
                        <EmptyState
                          title="Building doctor report"
                          description="PortPilot is checking tooling, env readiness, ports, and monorepo targets."
                        />
                      )}
                    </section>

                    <section className="subpanel">
                      <h4>Actions</h4>
                      <div className="action-stack">
                        {selectedProject.actions.map((action) => (
                          <div key={action.id} className="action-line">
                            <div>
                              <strong>{action.label}</strong>
                              <p>{action.command}</p>
                            </div>
                            <button
                              className={action.kind === "run" ? "primary-button" : "ghost-button"}
                              onClick={() => void handleRunAction(selectedProject, action)}
                              type="button"
                            >
                              {verbForAction(action.kind)}
                            </button>
                          </div>
                        ))}
                        <button className="secondary-button" onClick={() => void handleStop(selectedProject)} type="button">
                          Stop Active Execution
                        </button>
                      </div>
                    </section>

                    <section className="subpanel">
                      <h4>Environment</h4>
                      <div id="env-editor" />
                      {selectedProject.env_template.length > 0 ? (
                        <div className="env-grid">
                          {selectedProject.env_template.map((field) => (
                            <label key={field.key} className="field">
                              <span>{field.key}</span>
                              <small>{field.description ?? "No description provided."}</small>
                              {field.field_type === "multiline" ? (
                                <textarea
                                  rows={4}
                                  value={envValues[field.key] ?? ""}
                                  onInput={(event) =>
                                    setEnvValues((current) => ({
                                      ...current,
                                      [field.key]: event.currentTarget.value,
                                    }))
                                  }
                                />
                              ) : (
                                <input
                                  type={field.field_type === "secret" ? "password" : "text"}
                                  value={envValues[field.key] ?? ""}
                                  onInput={(event) =>
                                    setEnvValues((current) => ({
                                      ...current,
                                      [field.key]: event.currentTarget.value,
                                    }))
                                  }
                                />
                              )}
                            </label>
                          ))}
                        </div>
                      ) : (
                        <EmptyState
                          title="No env template detected"
                          description="Use the raw editor below to write a manual .env file if this repo has implicit environment variables."
                        />
                      )}

                      <label className="field">
                        <span>Advanced Raw Editor</span>
                        <textarea
                          rows={8}
                          value={envRawText}
                          onInput={(event) => setEnvRawText(event.currentTarget.value)}
                        />
                      </label>
                      <button className="primary-button" onClick={() => void handleSaveEnv()} type="button">
                        Save .env
                      </button>
                    </section>

                    <section className="subpanel">
                      <h4>Runtime</h4>
                      <div className="runtime-list">
                        {selectedExecutions.length > 0 ? (
                          selectedExecutions.map((execution) => (
                            <article key={execution.id} className="runtime-card">
                              <strong>{execution.label}</strong>
                              <p>{execution.command}</p>
                              <div className="runtime-card__meta">
                                <span>{execution.status}</span>
                                <span>{execution.resolved_port ?? execution.port_hint ?? "no port"}</span>
                                <span>{execution.pid ?? "no pid"}</span>
                              </div>
                            </article>
                          ))
                        ) : (
                          <EmptyState
                            title="No executions yet"
                            description="Run any action above and PortPilot will stream the result here."
                          />
                        )}
                      </div>
                    </section>

                    {selectedProject.workspace_targets.length > 0 && (
                      <section className="subpanel subpanel--full">
                        <h4>Detected App Targets</h4>
                        <div className="target-grid">
                          {selectedProject.workspace_targets.map((target) => (
                            <article key={target.id} className="target-card">
                              <strong>{target.name}</strong>
                              <p>{target.relative_path}</p>
                              <div className="target-card__meta">
                                <span>{target.runtime_kind}</span>
                                <span>port {target.suggested_port ?? "?"}</span>
                              </div>
                              <div className="target-card__meta">
                                {target.available_actions.map((action) => (
                                  <span key={`${target.id}-${action}`}>{action}</span>
                                ))}
                              </div>
                            </article>
                          ))}
                        </div>
                      </section>
                    )}
                  </div>
                </>
              ) : (
                <EmptyState
                  title="No project selected"
                  description="Import or register a repository to unlock runtime and environment controls."
                />
              )}
            </section>
          </section>
        )}

        {view === "routes" && (
          <section className="panel">
            <div className="panel__header">
              <h3>Unified Routes</h3>
              <p>Every managed repo gets both subdomain and path-style addresses.</p>
            </div>
            <DataTable
              headers={["Project", "Slug", "Target Port", "Subdomain", "Path Route"]}
              rows={routes.map((route) => [
                route.project_name,
                route.slug,
                String(route.target_port ?? "n/a"),
                route.subdomain_url,
                route.path_url,
              ])}
            />
          </section>
        )}

        {view === "ports" && (
          <section className="panel">
            <div className="panel__header">
              <h3>Port Center</h3>
              <p>Track which managed app owns which port and which execution is behind it.</p>
            </div>
            <DataTable
              headers={["Project", "Action", "Port", "PID", "Status"]}
              rows={ports.map((lease) => [
                lease.project_name,
                lease.action_label,
                String(lease.port),
                String(lease.pid ?? "n/a"),
                lease.status,
              ])}
            />
          </section>
        )}

        {view === "logs" && (
          <section className="panel panel--wide">
            <div className="panel__header">
              <h3>Live Logs</h3>
              <p>Stream action output from installs, runs, builds, deploys, and compose flows.</p>
            </div>
            <div className="log-console">
              {selectedLogs.map((entry) => (
                <div key={`${entry.execution_id}-${entry.timestamp}-${entry.message}`} className={`log-line log-line--${entry.stream}`}>
                  <span>{entry.timestamp}</span>
                  <strong>{entry.stream}</strong>
                  <p>{entry.message}</p>
                </div>
              ))}
              {selectedLogs.length === 0 && (
                <EmptyState title="No logs yet" description="Run an action and PortPilot will begin streaming output." />
              )}
            </div>
          </section>
        )}

        {view === "settings" && (
          <section className="panel-grid panel-grid--double">
            <section className="panel">
              <div className="panel__header">
                <h3>Workspace Roots</h3>
                <p>One path per line. PortPilot scans these locations for existing repos and uses the first root for Git imports.</p>
              </div>
              <textarea
                className="settings-textarea"
                rows={10}
                value={workspaceDraft}
                onInput={(event) => setWorkspaceDraft(event.currentTarget.value)}
              />
              <button className="primary-button" onClick={() => void handleSaveRoots()} type="button">
                Save Roots
              </button>
            </section>
            <section className="panel">
              <div className="panel__header">
                <h3>About & Updates</h3>
                <p>Cross-platform release status, auto-update controls, and release links.</p>
              </div>
              <Definition label="Current Version" value={`v${currentVersion}`} />
              <Definition
                label="Update State"
                value={
                  update.hasUpdate
                    ? `Update available: v${update.updateInfo?.availableVersion ?? "unknown"}`
                    : update.phase === "checking"
                      ? "Checking for updates..."
                      : update.phase === "upToDate"
                        ? "Up to date"
                        : update.phase
                }
              />
              {update.hasUpdate && update.updateInfo && !update.isDismissed && (
                <div className="update-banner">
                  <strong>PortPilot {update.updateInfo.availableVersion} is available.</strong>
                  <p>{update.updateInfo.notes ?? "A new cross-platform release is ready to install."}</p>
                  {update.progressTotal > 0 && (
                    <p>
                      Downloaded {Math.min(update.progressDownloaded, update.progressTotal)} /{" "}
                      {update.progressTotal} bytes
                    </p>
                  )}
                  <div className="action-row">
                    <button className="primary-button" onClick={() => void handleInstallUpdate()} type="button">
                      Install Update
                    </button>
                    <button className="secondary-button" onClick={() => update.dismissUpdate()} type="button">
                      Dismiss This Version
                    </button>
                    <button
                      className="ghost-button"
                      onClick={() => void openUrl("https://github.com/Horace-Maxwell/portpilot/releases")}
                      type="button"
                    >
                      Release Notes
                    </button>
                  </div>
                </div>
              )}
              {update.hasUpdate && update.updateInfo && update.isDismissed && (
                <div className="action-row">
                  <button className="secondary-button" onClick={() => update.resetDismiss()} type="button">
                    Show Dismissed Update
                  </button>
                </div>
              )}
              {!update.hasUpdate && (
                <div className="action-row">
                  <button className="primary-button" onClick={() => void handleCheckUpdate()} type="button">
                    Check for Updates
                  </button>
                  <button
                    className="secondary-button"
                    onClick={() => void openUrl("https://github.com/Horace-Maxwell/portpilot/releases")}
                    type="button"
                  >
                    Open Releases
                  </button>
                </div>
              )}
              {update.error && <p className="error-copy">{update.error}</p>}
              <Definition label="Workspace Roots" value={String(workspaceRoots.length)} />
              <Definition label="Managed Projects" value={String(projects.length)} />
              <Definition
                label="Running Executions"
                value={String(executions.filter((execution) => execution.status === "running").length)}
              />
              <Definition label="Last Status" value={statusMessage} />
            </section>
          </section>
        )}
      </main>
    </div>
  );
}

function firstAction(project: ManagedProject, kind: ActionKind): ProjectAction | null {
  return project.actions.find((action) => action.kind === kind) ?? null;
}

function upsertById<T extends { id: string }>(items: T[], next: T): T[] {
  const found = items.some((item) => item.id === next.id);
  if (!found) return [next, ...items];
  return items.map((item) => (item.id === next.id ? next : item));
}

function verbForAction(kind: ActionKind) {
  switch (kind) {
    case "install":
      return "Install";
    case "run":
      return "Run";
    case "stop":
      return "Stop";
    case "restart":
      return "Restart";
    case "build":
      return "Build";
    case "deploy":
      return "Deploy";
    case "open":
      return "Open";
    case "logs":
      return "Logs";
  }
}

function StatCard(props: { label: string; value: string }) {
  return (
    <article className="stat-card">
      <span>{props.label}</span>
      <strong>{props.value}</strong>
    </article>
  );
}

function Definition(props: { label: string; value: string }) {
  return (
    <div className="definition">
      <span>{props.label}</span>
      <strong>{props.value}</strong>
    </div>
  );
}

function EmptyState(props: { title: string; description: string }) {
  return (
    <div className="empty-state">
      <strong>{props.title}</strong>
      <p>{props.description}</p>
    </div>
  );
}

function DataTable(props: { headers: string[]; rows: string[][] }) {
  return (
    <div className="table-shell">
      <table>
        <thead>
          <tr>
            {props.headers.map((header) => (
              <th key={header}>{header}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {props.rows.map((row, rowIndex) => (
            <tr key={`${rowIndex}-${row.join("-")}`}>
              {row.map((cell, cellIndex) => (
                <td key={`${rowIndex}-${cellIndex}`}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {props.rows.length === 0 && (
        <EmptyState title="No data yet" description="Run some projects and PortPilot will populate this view." />
      )}
    </div>
  );
}

function SetupWizard(props: {
  project: ManagedProject;
  report: DoctorReport;
  onInstall: () => void;
  onRun: () => void;
  onOpen: () => void;
  onFocusEnv: () => void;
}) {
  const installState = props.report.checks.find((check) => check.id === "install-state");
  const envReady = props.report.missing_env_keys.length === 0;
  const runReady = props.project.status === "running";
  const wizardSteps = [
    {
      id: "clone",
      label: "Project imported",
      status: "done",
      description: "This repo is already managed by PortPilot.",
      action: null,
    },
    {
      id: "install",
      label: "Install dependencies",
      status:
        props.report.install_action_id == null || installState?.status === "ok" ? "done" : "todo",
      description:
        installState?.summary ?? "PortPilot inferred an install action for this repository.",
      action:
        props.report.install_action_id != null && installState?.status !== "ok"
          ? { label: "Run install", onClick: props.onInstall }
          : null,
    },
    {
      id: "env",
      label: "Fill environment",
      status: envReady ? "done" : "todo",
      description: envReady
        ? "Environment values look ready for the detected template."
        : `Missing ${props.report.missing_env_keys.length} value(s): ${props.report.missing_env_keys.join(", ")}`,
      action: envReady ? null : { label: "Open env editor", onClick: props.onFocusEnv },
    },
    {
      id: "run",
      label: "Start the app",
      status: runReady ? "done" : "todo",
      description: runReady
        ? "The primary run action is live."
        : "Use the inferred primary run action once dependencies and env values are ready.",
      action:
        !runReady && props.report.run_action_id != null
          ? { label: "Run primary action", onClick: props.onRun }
          : null,
    },
    {
      id: "open",
      label: "Open routed preview",
      status: runReady ? "ready" : "todo",
      description: runReady
        ? "Open the unified PortPilot route in the browser or embedded preview."
        : "The preview becomes useful after the run action starts responding.",
      action: runReady ? { label: "Open route", onClick: props.onOpen } : null,
    },
  ] as const;

  return (
    <div className="wizard">
      {wizardSteps.map((step, index) => (
        <article key={step.id} className={`wizard-step wizard-step--${step.status}`}>
          <div className="wizard-step__index">{index + 1}</div>
          <div className="wizard-step__body">
            <strong>{step.label}</strong>
            <p>{step.description}</p>
          </div>
          {step.action && (
            <button className="ghost-button" onClick={step.action.onClick} type="button">
              {step.action.label}
            </button>
          )}
        </article>
      ))}
    </div>
  );
}

function DoctorChecks(props: { checks: DoctorReport["checks"] }) {
  return (
    <div className="doctor-list">
      {props.checks.map((check) => (
        <article key={check.id} className={`doctor-card doctor-card--${check.status}`}>
          <div className="doctor-card__top">
            <strong>{check.label}</strong>
            <span className={`status-pill status-pill--doctor-${check.status}`}>{check.status}</span>
          </div>
          <p>{check.summary}</p>
          {check.detail && <small>{check.detail}</small>}
          {check.fix_label && check.fix_command && (
            <code>
              {check.fix_label}: {check.fix_command}
            </code>
          )}
        </article>
      ))}
    </div>
  );
}
