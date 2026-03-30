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
  BatchActionResult,
  ComposeRequirement,
  DoctorReport,
  EnvGroupPreset,
  ImportedRepo,
  LocalServicePreset,
  LogEntry,
  ManagedProject,
  PortLease,
  ProjectProfile,
  ProjectAction,
  RouteBinding,
  RunPhase,
  RuntimeKind,
  RuntimeNode,
  RuntimeStatus,
  WorkspaceSession,
} from "./shared/types";

type Locale = "en" | "zh-CN";

type NavKey =
  | "dashboard"
  | "import"
  | "projects"
  | "runtime"
  | "routes"
  | "ports"
  | "logs"
  | "settings";

const NAV_ITEMS: NavKey[] = [
  "dashboard",
  "import",
  "projects",
  "runtime",
  "routes",
  "ports",
  "logs",
  "settings",
];

export default function App() {
  const [view, setView] = useState<NavKey>("dashboard");
  const [locale, setLocale] = useState<Locale>(() => {
    const saved = globalThis.localStorage?.getItem("portpilot.locale");
    return saved === "zh-CN" ? "zh-CN" : "en";
  });
  const [workspaceRoots, setWorkspaceRoots] = useState<string[]>([]);
  const [projects, setProjects] = useState<ManagedProject[]>([]);
  const [candidates, setCandidates] = useState<ImportedRepo[]>([]);
  const [executions, setExecutions] = useState<ActionExecution[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [runtimeNodes, setRuntimeNodes] = useState<RuntimeNode[]>([]);
  const [localServicePresets, setLocalServicePresets] = useState<LocalServicePreset[]>([]);
  const [envGroupPresets, setEnvGroupPresets] = useState<EnvGroupPreset[]>([]);
  const [ports, setPorts] = useState<PortLease[]>([]);
  const [routes, setRoutes] = useState<RouteBinding[]>([]);
  const [doctorReports, setDoctorReports] = useState<Record<string, DoctorReport>>({});
  const [sessions, setSessions] = useState<WorkspaceSession[]>([]);
  const [selectedProjectIds, setSelectedProjectIds] = useState<string[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [embedUrl, setEmbedUrl] = useState<string | null>(null);
  const [importUrl, setImportUrl] = useState("https://github.com/calesthio/Crucix.git");
  const [importRootDraft, setImportRootDraft] = useState("/Users/horacedong/Desktop/Github");
  const [workspaceDraft, setWorkspaceDraft] = useState("");
  const [statusMessage, setStatusMessage] = useState(
    locale === "zh-CN" ? "PortPilot 正在启动…" : "Booting PortPilot...",
  );
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [envValues, setEnvValues] = useState<Record<string, string>>({});
  const [envRawText, setEnvRawText] = useState("");
  const [logQuery, setLogQuery] = useState("");
  const [logStreamFilter, setLogStreamFilter] = useState<"all" | "stdout" | "stderr" | "system">("all");
  const [currentVersion, setCurrentVersion] = useState("0.1.0");
  const update = useUpdate();
  const t = (en: string, zh: string) => (locale === "zh-CN" ? zh : en);

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
  const selectedRuntimeNodes = useMemo(
    () =>
      selectedProject
        ? runtimeNodes.filter((node) => node.project_id === selectedProject.id)
        : runtimeNodes,
    [runtimeNodes, selectedProject],
  );
  const filteredLogs = useMemo(
    () =>
      selectedLogs.filter((entry) => {
        const streamMatches = logStreamFilter === "all" || entry.stream === logStreamFilter;
        const query = logQuery.trim().toLowerCase();
        const queryMatches = !query || entry.message.toLowerCase().includes(query);
        return streamMatches && queryMatches;
      }),
    [logQuery, logStreamFilter, selectedLogs],
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
              ? locale === "zh-CN"
                ? `仓库导入：${payload.stage}${payload.destination ? `（${payload.destination}）` : ""}`
                : `Repository import: ${payload.stage}${payload.destination ? ` (${payload.destination})` : ""}`
              : locale === "zh-CN"
                ? "仓库导入进度已更新。"
                : "Repository import updated.",
          );
          void refreshProjectsOnly();
        }),
      );
      unlisteners.push(
        await listen<ActionExecution>("action-started", (event) => {
          setExecutions((current) => upsertById(current, event.payload));
          setStatusMessage(
            locale === "zh-CN"
              ? `已启动 ${event.payload.label}`
              : `Started ${event.payload.label}`,
          );
          void refreshProjectsOnly();
        }),
      );
      unlisteners.push(
        await listen<ActionExecution>("action-finished", (event) => {
          setExecutions((current) => upsertById(current, event.payload));
          setStatusMessage(
            locale === "zh-CN"
              ? `${event.payload.label} 已结束，状态为 ${localizeExecutionStatus(
                  event.payload.status,
                  locale,
                )}`
              : `Finished ${event.payload.label} with ${event.payload.status}`,
          );
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
  }, [locale]);

  useEffect(() => {
    if (!selectedProjectId && projects[0]) {
      setSelectedProjectId(projects[0].id);
    }
  }, [projects, selectedProjectId]);

  useEffect(() => {
    setSelectedProjectIds((current) =>
      current.filter((projectId) => projects.some((project) => project.id === projectId)),
    );
  }, [projects]);

  useEffect(() => {
    if (!selectedProject) {
      setEnvValues({});
      setEnvRawText("");
      setEnvGroupPresets([]);
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
      setEnvGroupPresets([]);
      return;
    }

    void api
      .listEnvGroupPresets(selectedProject.id)
      .then(setEnvGroupPresets)
      .catch(() => setEnvGroupPresets([]));
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

  useEffect(() => {
    const timer = globalThis.setInterval(() => {
      void refreshRuntimeOnly();
    }, 2000);
    return () => globalThis.clearInterval(timer);
  }, []);

  useEffect(() => {
    globalThis.localStorage?.setItem("portpilot.locale", locale);
  }, [locale]);

  const runningProjects = projects.filter((project) => project.status === "running").length;
  const selectedDoctorReport = selectedProject ? doctorReports[selectedProject.id] ?? null : null;

  async function refreshAll() {
    const [
      roots,
      nextProjects,
      nextExecutions,
      nextLogs,
      nextRuntimeNodes,
      nextLocalServices,
      nextPorts,
      nextRoutes,
      nextSessions,
    ] =
      await Promise.all([
        api.listWorkspaceRoots(),
        api.listProjects(),
        api.listActionExecutions(),
        api.getProjectLogs(),
        api.listRuntimeNodes(),
        api.listLocalServicePresets(),
        api.listPorts(),
        api.listRoutes(),
        api.listWorkspaceSessions(),
      ]);
    setWorkspaceRoots(roots);
    setWorkspaceDraft(roots.join("\n"));
    setImportRootDraft(roots[0] ?? "/Users/horacedong/Desktop/Github");
    setProjects(nextProjects);
    setExecutions(nextExecutions);
    setLogs(nextLogs);
    setRuntimeNodes(nextRuntimeNodes);
    setLocalServicePresets(nextLocalServices);
    setPorts(nextPorts);
    setRoutes(nextRoutes);
    setSessions(nextSessions);
      setStatusMessage(
        locale === "zh-CN"
          ? `已加载 ${nextProjects.length} 个项目。`
          : `Loaded ${nextProjects.length} project${nextProjects.length === 1 ? "" : "s"}.`,
      );
  }

  async function refreshProjectsOnly() {
    const [nextProjects, nextExecutions, nextRuntimeNodes, nextPorts, nextRoutes] = await Promise.all([
      api.listProjects(),
      api.listActionExecutions(),
      api.listRuntimeNodes(),
      api.listPorts(),
      api.listRoutes(),
    ]);
    setProjects(nextProjects);
    setExecutions(nextExecutions);
    setRuntimeNodes(nextRuntimeNodes);
    setPorts(nextPorts);
    setRoutes(nextRoutes);
  }

  async function refreshRuntimeOnly() {
    const [nextExecutions, nextRuntimeNodes, nextLocalServices, nextPorts, nextRoutes] = await Promise.all([
      api.listActionExecutions(),
      api.listRuntimeNodes(),
      api.listLocalServicePresets(),
      api.listPorts(),
      api.listRoutes(),
    ]);
    setExecutions(nextExecutions);
    setRuntimeNodes(nextRuntimeNodes);
    setLocalServicePresets(nextLocalServices);
    setPorts(nextPorts);
    setRoutes(nextRoutes);
  }

  async function handleScan() {
    await runBusy("scan", async () => {
      const nextCandidates = await api.scanLocalProjects(workspaceRoots);
      setCandidates(nextCandidates);
      setView("import");
      setStatusMessage(
        locale === "zh-CN"
          ? `发现了 ${nextCandidates.length} 个候选仓库。`
          : `Found ${nextCandidates.length} candidate repositories.`,
      );
    });
  }

  async function handleImportGit() {
    await runBusy("import", async () => {
      const project = await api.importRepoFromGit(importUrl, importRootDraft);
      setStatusMessage(locale === "zh-CN" ? `已导入 ${project.name}。` : `Imported ${project.name}.`);
      await refreshAll();
      setSelectedProjectId(project.id);
      setView("projects");
    });
  }

  async function handleRegisterCandidate(candidate: ImportedRepo) {
    await runBusy(`register-${candidate.root_path}`, async () => {
      const project = await api.registerLocalProject(candidate.root_path, candidate.git_url);
      setStatusMessage(
        locale === "zh-CN" ? `已注册 ${project.name}。` : `Registered ${project.name}.`,
      );
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
      setStatusMessage(
        locale === "zh-CN"
          ? `${project.name} 当前没有活跃执行。`
          : `No active execution found for ${project.name}.`,
      );
      return;
    }
    await runBusy(`stop-${running.id}`, async () => {
      await api.stopActionExecution(running.id);
      await refreshProjectsOnly();
    });
  }

  async function handleBatchAction(
    kind: "run" | "stop" | "restart" | "restore",
    sessionId?: string,
  ) {
    if (kind !== "restore" && selectedProjectIds.length === 0) {
      setStatusMessage(
        locale === "zh-CN" ? "请先至少选择一个项目。" : "Select at least one project first.",
      );
      return;
    }

    await runBusy(`batch-${kind}`, async () => {
      let result: BatchActionResult;
      if (kind === "run") {
        result = await api.runBatchAction(selectedProjectIds);
      } else if (kind === "stop") {
        result = await api.stopProjects(selectedProjectIds);
      } else if (kind === "restart") {
        result = await api.restartProjects(selectedProjectIds);
      } else {
        result = await api.restoreWorkspaceSession(sessionId ?? "");
      }

      await refreshAll();
      setStatusMessage(
        locale === "zh-CN"
          ? `${localizeBatchKind(result.kind, locale)}：成功 ${result.success_count}，失败 ${result.failure_count}，跳过 ${result.skipped_count}。`
          : `${result.kind}: ${result.success_count} succeeded, ${result.failure_count} failed, ${result.skipped_count} skipped.`,
      );
      if (kind === "restore") {
        const firstStarted = result.items.find((item) => item.status === "success");
        if (firstStarted) {
          setSelectedProjectId(firstStarted.project_id);
        }
      }
    });
  }

  async function handleSaveSession() {
    if (selectedProjectIds.length === 0) {
      setStatusMessage(
        locale === "zh-CN"
          ? "保存工作区前请至少选择一个项目。"
          : "Select at least one project before saving a session.",
      );
      return;
    }

    const proposedName =
      globalThis.prompt?.(
        locale === "zh-CN" ? "为这个工作区命名" : "Name this workspace session",
        `Workspace ${new Date().toLocaleString()}`,
      ) ?? "";
    if (!proposedName.trim()) {
      return;
    }

    await runBusy("save-session", async () => {
      const session = await api.saveWorkspaceSession(proposedName, selectedProjectIds, null);
      setSessions((current) => [session, ...current.filter((item) => item.id !== session.id)]);
      setStatusMessage(
        locale === "zh-CN"
          ? `已保存工作区“${session.name}”，包含 ${session.projects.length} 个项目。`
          : `Saved session "${session.name}" with ${session.projects.length} projects.`,
      );
    });
  }

  async function handleCopyRecipe() {
    if (!selectedProject) {
      return;
    }
    await runBusy(`recipe-copy-${selectedProject.id}`, async () => {
      const recipe = await api.getProjectRecipe(selectedProject.id);
      const contents = JSON.stringify(recipe, null, 2);
      await navigator.clipboard.writeText(contents);
      setStatusMessage(
        locale === "zh-CN"
          ? `已复制 ${selectedProject.name} 的 .portpilot.json recipe。`
          : `Copied .portpilot.json recipe for ${selectedProject.name}.`,
      );
    });
  }

  async function handleCopyServiceCommand(service: LocalServicePreset) {
    if (!service.start_command) {
      return;
    }
    await navigator.clipboard.writeText(service.start_command);
    setStatusMessage(
      locale === "zh-CN"
        ? `已复制 ${service.label} 的启动命令。`
        : `Copied start command for ${service.label}.`,
    );
  }

  function handleApplyEnvGroupPreset(preset: EnvGroupPreset) {
    let appliedCount = 0;
    setEnvValues((current) => {
      const next = { ...current };
      for (const [key, value] of Object.entries(preset.values)) {
        if (!next[key]?.trim()) {
          next[key] = value;
          appliedCount += 1;
        }
      }
      return next;
    });
    setStatusMessage(
      locale === "zh-CN"
        ? `已为 ${localizeEnvGroupLabel(preset.label, locale)} 预填 ${appliedCount} 个本地默认值。`
        : `Filled ${appliedCount} local default value(s) for ${preset.label}.`,
    );
  }

  async function handleWriteRecipe() {
    if (!selectedProject) {
      return;
    }
    await runBusy(`recipe-write-${selectedProject.id}`, async () => {
      const updated = await api.writeProjectRecipe(selectedProject.id);
      setStatusMessage(
        locale === "zh-CN"
          ? `已为 ${updated.name} 写入 .portpilot.json。`
          : `Wrote .portpilot.json for ${updated.name}.`,
      );
      await refreshAll();
      setSelectedProjectId(updated.id);
    });
  }

  async function handleDeleteSession(sessionId: string) {
    await runBusy(`delete-session-${sessionId}`, async () => {
      const nextSessions = await api.deleteWorkspaceSession(sessionId);
      setSessions(nextSessions);
      setStatusMessage(locale === "zh-CN" ? "已删除工作区会话。" : "Deleted workspace session.");
    });
  }

  function toggleProjectSelection(projectId: string) {
    setSelectedProjectIds((current) =>
      current.includes(projectId)
        ? current.filter((id) => id !== projectId)
        : [...current, projectId],
    );
  }

  function toggleAllProjects(projectList: ManagedProject[]) {
    const ids = projectList.map((project) => project.id);
    setSelectedProjectIds((current) =>
      ids.every((id) => current.includes(id))
        ? current.filter((id) => !ids.includes(id))
        : Array.from(new Set([...current, ...ids])),
    );
  }

  async function handleSaveEnv() {
    if (!selectedProject) return;
    await runBusy(`env-${selectedProject.id}`, async () => {
      const updated = await api.saveEnvProfile(
        selectedProject.id,
        envValues,
        envRawText.trim() ? envRawText : null,
      );
      setStatusMessage(locale === "zh-CN" ? `已为 ${updated.name} 保存 .env。` : `Saved .env for ${updated.name}.`);
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
      setStatusMessage(
        locale === "zh-CN"
          ? `已保存 ${saved.length} 个工作区根目录。`
          : `Saved ${saved.length} workspace root${saved.length === 1 ? "" : "s"}.`,
      );
    });
  }

  async function handleCheckUpdate() {
    await runBusy("check-update", async () => {
      const available = await update.checkUpdate();
      setStatusMessage(
        available
          ? locale === "zh-CN"
            ? `更新 ${update.updateInfo?.availableVersion ?? ""} 已可安装。`
            : `Update ${update.updateInfo?.availableVersion ?? ""} is ready to install.`
          : locale === "zh-CN"
            ? "PortPilot 当前已经是最新版本。"
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
          <span className="brand__eyebrow">{t("GitHub Repo Console", "GitHub 仓库控制台")}</span>
          <h1>PortPilot</h1>
          <p>{t(
            "Import, configure, run, stop, route, and package local-first repositories from one desktop cockpit.",
            "在一个桌面控制台里完成本地优先仓库的导入、配置、运行、停止、路由和打包。",
          )}</p>
        </div>

        <div className="stats">
          <StatCard label={t("Managed", "已托管")} value={String(projects.length)} />
          <StatCard label={t("Running", "运行中")} value={String(runningProjects)} />
          <StatCard label={t("Routes", "路由")} value={String(routes.length)} />
          <StatCard label={t("Ports", "端口")} value={String(ports.length)} />
        </div>

        <nav className="nav">
          {NAV_ITEMS.map((item) => (
            <button
              key={item}
              className={`nav__item ${view === item ? "is-active" : ""}`}
              onClick={() => setView(item)}
              type="button"
            >
              {navLabel(item, locale)}
            </button>
          ))}
        </nav>

        <div className="sidebar__footer">
          <div className="action-row">
            <button
              className={locale === "en" ? "secondary-button" : "ghost-button"}
              onClick={() => setLocale("en")}
              type="button"
            >
              EN
            </button>
            <button
              className={locale === "zh-CN" ? "secondary-button" : "ghost-button"}
              onClick={() => setLocale("zh-CN")}
              type="button"
            >
              中文
            </button>
          </div>
          <button className="secondary-button" onClick={() => void refreshAll()} type="button">
            {t("Refresh", "刷新")}
          </button>
          <button className="primary-button" onClick={() => void handleScan()} type="button">
            {t("Scan Roots", "扫描工作区")}
          </button>
          <p>{statusMessage}</p>
        </div>
      </aside>

      <main className="main">
        <header className="hero">
          <div>
            <span className="hero__eyebrow">PortPilot v1</span>
            <h2>{t(
              "One-click control for repos like Crucix and WorldMonitor",
              "像 Crucix 和 WorldMonitor 这样的仓库，也能一键控制",
            )}</h2>
          </div>
          <div className="hero__actions">
            <button className="secondary-button" onClick={() => setView("import")} type="button">
              {t("Import Repo", "导入仓库")}
            </button>
            <button className="primary-button" onClick={() => setView("projects")} type="button">
              {t("Open Projects", "打开项目")}
            </button>
          </div>
        </header>

        {view === "dashboard" && (
          <section className="panel-grid">
            <section className="panel panel--wide">
              <div className="panel__header">
                <div>
                  <h3>{t("Command Deck", "指挥面板")}</h3>
                  <p>{t("The fastest path from clone to live route.", "从 clone 到在线路由的最快路径。")}</p>
                </div>
                <SelectionToolbar
                  locale={locale}
                  selectedCount={selectedProjectIds.length}
                  allSelected={projects.length > 0 && projects.every((project) => selectedProjectIds.includes(project.id))}
                  onToggleAll={() => toggleAllProjects(projects)}
                  onRun={() => void handleBatchAction("run")}
                  onStop={() => void handleBatchAction("stop")}
                  onRestart={() => void handleBatchAction("restart")}
                  onSaveSession={() => void handleSaveSession()}
                />
              </div>
              <div className="project-grid">
                {projects.map((project) => {
                  const runAction = firstAction(project, "run");
                  const buildAction = firstAction(project, "build");
                  const deployAction = firstAction(project, "deploy");
                  const primaryTarget = projectPrimaryTarget(project);
                  const blockerCount = doctorReports[project.id]?.blockers.length ?? 0;
                  return (
                    <article key={project.id} className="project-card">
                      <div className="project-card__top">
                        <label className="selection-check">
                          <input
                            checked={selectedProjectIds.includes(project.id)}
                            onChange={() => toggleProjectSelection(project.id)}
                            type="checkbox"
                          />
                          <span>{t("Select", "选择")}</span>
                        </label>
                        <div>
                          <span className={`status-pill status-pill--${project.status}`}>
                            {formatRuntimeStatus(project.status, locale)}
                          </span>
                          <span className="status-pill status-pill--doctor-info">
                            {formatProfileKind(project.project_profile.kind, locale)}
                          </span>
                          <h4>{project.name}</h4>
                          <p>{formatRuntimeKind(project.runtime_kind, locale)} / {project.project_kind}</p>
                        </div>
                        <button
                          className="ghost-button"
                          onClick={() => {
                            setSelectedProjectId(project.id);
                            setView("projects");
                          }}
                          type="button"
                        >
                          {t("Details", "详情")}
                        </button>
                      </div>
                      <div className="project-card__meta">
                        <span>{project.route_path_url}</span>
                        <span>{project.resolved_port ?? project.preferred_port ?? t("No port hint", "无端口提示")}</span>
                        {primaryTarget && <span>{t("Target", "目标")}: {primaryTarget.name}</span>}
                        {project.project_profile.required_services.length > 0 && (
                          <span>{project.project_profile.required_services.length} {t("service deps", "服务依赖")}</span>
                        )}
                        {blockerCount > 0 && <span>{blockerCount} {t(blockerCount === 1 ? "blocker" : "blockers", blockerCount === 1 ? "阻塞项" : "阻塞项")}</span>}
                      </div>
                      {project.project_profile.summary && (
                        <p className="project-card__summary">{project.project_profile.summary}</p>
                      )}
                      <div className="action-row">
                        {runAction && (
                          <button className="primary-button" onClick={() => void handleRunAction(project, runAction)} type="button">
                            {t("Run", "运行")}
                          </button>
                        )}
                        <button className="secondary-button" onClick={() => void handleStop(project)} type="button">
                          {t("Stop", "停止")}
                        </button>
                        <button
                          className="secondary-button"
                          onClick={() => {
                            setEmbedUrl(project.route_path_url);
                            void openUrl(project.route_path_url);
                          }}
                          type="button"
                        >
                          {t("Open", "打开")}
                        </button>
                      </div>
                      <div className="action-row">
                        {buildAction && (
                          <button className="ghost-button" onClick={() => void handleRunAction(project, buildAction)} type="button">
                            {t("Build", "构建")}
                          </button>
                        )}
                        {deployAction && (
                          <button className="ghost-button" onClick={() => void handleRunAction(project, deployAction)} type="button">
                            {t("Deploy", "部署")}
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
                <div>
                  <h3>{t("Saved Sessions", "已保存工作区")}</h3>
                  <p>{t("Restore a whole workspace in one click, then verify the result in the live preview below.", "一键恢复整个工作区，然后在下面的实时预览里验证结果。")}</p>
                </div>
              </div>
              <div className="session-list">
                {sessions.map((session) => (
                  <article key={session.id} className="session-card">
                    <div>
                      <strong>{session.name}</strong>
                      <p>{session.projects.length} {t("projects", "个项目")}</p>
                    </div>
                    <div className="action-row">
                      <button
                        className="primary-button"
                        onClick={() => void handleBatchAction("restore", session.id)}
                        type="button"
                      >
                        {t("Restore", "恢复")}
                      </button>
                      <button
                        className="ghost-button"
                        onClick={() => void handleDeleteSession(session.id)}
                        type="button"
                      >
                        {t("Delete", "删除")}
                      </button>
                    </div>
                  </article>
                ))}
                {sessions.length === 0 && (
                  <EmptyState
                    title={t("No sessions saved yet", "还没有保存的工作区")}
                    description={t("Select a group of projects and save them as a workspace session.", "选择一组项目并保存成一个工作区会话。")}
                  />
                )}
              </div>
              <div className="panel__header">
                <div>
                  <h3>{t("Live Preview", "实时预览")}</h3>
                  <p>{t("Open the routed app inside PortPilot when you want quick verification.", "需要快速验证时，直接在 PortPilot 内打开路由后的应用。")}</p>
                </div>
              </div>
              {embedUrl ? (
                <iframe className="embed-frame" src={embedUrl} title={t("Embedded project preview", "嵌入式项目预览")} />
              ) : (
                <EmptyState
                  title={t("No embedded target yet", "还没有嵌入目标")}
                  description={t("Open a managed route and it will appear here.", "打开一个受管路由后，它会显示在这里。")}
                />
              )}
            </section>
          </section>
        )}

        {view === "import" && (
          <section className="panel-grid panel-grid--double">
            <section className="panel">
              <div className="panel__header">
                <h3>{t("Import from GitHub", "从 GitHub 导入")}</h3>
                <p>{t("Paste a repository URL, clone it into your workspace, then let PortPilot infer actions and env.", "粘贴仓库 URL，把它 clone 到工作区，然后让 PortPilot 自动推断动作和环境变量。")}</p>
              </div>
              <label className="field">
                <span>{t("GitHub URL", "GitHub 地址")}</span>
                <input value={importUrl} onInput={(event) => setImportUrl(event.currentTarget.value)} />
              </label>
              <label className="field">
                <span>{t("Workspace Root", "工作区根目录")}</span>
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
                {t("Clone + Register", "克隆并注册")}
              </button>
            </section>

            <section className="panel">
              <div className="panel__header">
                <h3>{t("Scan Existing Roots", "扫描现有工作区")}</h3>
                <p>{t("PortPilot detects Node, Python, Rust, Go, and Compose repos already on disk.", "PortPilot 会检测磁盘上已有的 Node、Python、Rust、Go 和 Compose 仓库。")}</p>
              </div>
              <button
                className="secondary-button"
                disabled={busyKey === "scan"}
                onClick={() => void handleScan()}
                type="button"
              >
                {t("Scan Now", "立即扫描")}
              </button>
              <div className="candidate-list">
                {candidates.map((candidate) => (
                  <article key={candidate.root_path} className="candidate-card">
                    <div>
                      <h4>{candidate.name}</h4>
                      <p>{candidate.root_path}</p>
                      {candidate.project_profile.summary && (
                        <p className="candidate-card__hint">{candidate.project_profile.summary}</p>
                      )}
                      {candidate.readme_hints[0] && (
                        <p className="candidate-card__hint">{t("README hint", "README 提示")}: {candidate.readme_hints[0]}</p>
                      )}
                    </div>
                    <div className="candidate-card__meta">
                      <span>{formatProfileKind(candidate.project_profile.kind, locale)}</span>
                      <span>{formatRuntimeKind(candidate.runtime_kind, locale)}</span>
                      <span>{t("Port", "端口")} {candidate.suggested_port ?? "?"}</span>
                      {candidate.workspace_target_count > 0 && (
                        <span>{candidate.workspace_target_count} {t("app targets", "个应用目标")}</span>
                      )}
                      {candidate.has_env_template && <span>{t(".env template", ".env 模板")}</span>}
                      {candidate.has_docker_compose && <span>Compose</span>}
                    </div>
                    <button className="primary-button" onClick={() => void handleRegisterCandidate(candidate)} type="button">
                      {t("Register", "注册")}
                    </button>
                  </article>
                ))}
                {candidates.length === 0 && (
                  <EmptyState
                    title={t("No scan results yet", "还没有扫描结果")}
                    description={t("Use Scan Now to discover repos across your configured workspace roots.", "点击立即扫描，查找你已配置工作区里的仓库。")}
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
                <div>
                  <h3>{t("Projects", "项目")}</h3>
                  <p>{t("Switch between managed repositories and action groups.", "在托管仓库和动作组之间切换。")}</p>
                </div>
              </div>
              <SelectionToolbar
                locale={locale}
                selectedCount={selectedProjectIds.length}
                allSelected={
                  projects.length > 0 && projects.every((project) => selectedProjectIds.includes(project.id))
                }
                onToggleAll={() => toggleAllProjects(projects)}
                onRun={() => void handleBatchAction("run")}
                onStop={() => void handleBatchAction("stop")}
                onRestart={() => void handleBatchAction("restart")}
                onSaveSession={() => void handleSaveSession()}
              />
              {projects.map((project) => (
                <button
                  key={project.id}
                  className={`project-list__item ${selectedProjectId === project.id ? "is-active" : ""}`}
                  onClick={() => setSelectedProjectId(project.id)}
                  type="button"
                >
                  <label
                    className="selection-check"
                    onClick={(event) => event.stopPropagation()}
                  >
                    <input
                      checked={selectedProjectIds.includes(project.id)}
                      onChange={() => toggleProjectSelection(project.id)}
                      type="checkbox"
                    />
                    <span />
                  </label>
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
                        {t("Open Route", "打开路由")}
                      </button>
                      <button className="ghost-button" onClick={() => void handleCopyRecipe()} type="button">
                        {t("Copy Recipe JSON", "复制 Recipe JSON")}
                      </button>
                      <button className="secondary-button" onClick={() => void handleWriteRecipe()} type="button">
                        {t("Write .portpilot.json", "写入 .portpilot.json")}
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
                          {t("Restart Primary Run", "重启主运行入口")}
                        </button>
                      )}
                    </div>
                  </div>

                  <div className="section-grid">
                    <section className="subpanel">
                      <h4>{t("Overview", "概览")}</h4>
                      <Definition label={t("Status", "状态")} value={formatRuntimeStatus(selectedProject.status, locale)} />
                      <Definition
                        label={t("Platform Profile", "平台画像")}
                        value={formatProfileKind(selectedProject.project_profile.kind, locale)}
                      />
                      {selectedProject.project_profile.summary && (
                        <Definition label={t("Profile Summary", "画像摘要")} value={selectedProject.project_profile.summary} />
                      )}
                      <Definition
                        label={t("Recommended Target", "推荐目标")}
                        value={
                          projectPrimaryTarget(selectedProject)
                            ? `${projectPrimaryTarget(selectedProject)?.name} (${projectPrimaryTarget(selectedProject)?.relative_path})`
                            : t("Root project", "根项目")
                        }
                      />
                      <Definition label={t("Port", "端口")} value={String(selectedProject.resolved_port ?? selectedProject.preferred_port ?? t("Unknown", "未知"))} />
                      <Definition label={t("Subdomain", "子域名")} value={selectedProject.route_subdomain_url} />
                      <Definition label={t("Path Route", "路径路由")} value={selectedProject.route_path_url} />
                      {selectedProject.project_profile.route_strategy && (
                        <Definition
                          label={t("Route Strategy", "路由策略")}
                          value={formatRouteStrategy(selectedProject.project_profile.route_strategy, locale)}
                        />
                      )}
                      <Definition label={t("Detected", "已检测到")} value={selectedProject.detected_files.join(", ")} />
                      <Definition
                        label={t("Recipe", "配方")}
                        value={
                          selectedProject.detected_files.includes(".portpilot.json")
                            ? t(".portpilot.json detected", "已检测到 .portpilot.json")
                            : t("Write a recipe to lock in this repo's best defaults", "写入 recipe 来锁定这个仓库的最佳默认配置")
                        }
                      />
                      <Definition
                        label={t("App Targets", "应用目标")}
                        value={
                          selectedProject.workspace_targets.length > 0
                            ? String(selectedProject.workspace_targets.length)
                            : t("Single app", "单应用")
                        }
                      />
                      {selectedProject.project_profile.required_services.length > 0 && (
                        <Definition
                          label={t("Required Services", "依赖服务")}
                          value={selectedProject.project_profile.required_services.join(" • ")}
                        />
                      )}
                      {selectedProject.project_profile.known_ports.length > 0 && (
                        <Definition
                          label={t("Known Ports", "已知端口")}
                          value={selectedProject.project_profile.known_ports.join(" • ")}
                        />
                      )}
                      {selectedProject.readme_hints.length > 0 && (
                        <Definition
                          label={t("README Hints", "README 提示")}
                          value={selectedProject.readme_hints.slice(0, 2).join(" • ")}
                        />
                      )}
                    </section>

                    <section className="subpanel">
                      <h4>{t("Setup / Doctor", "初始化 / Doctor")}</h4>
                      {selectedDoctorReport ? (
                        <>
                          {selectedDoctorReport.recommended_next_step && (
                            <div className="info-banner">
                              <strong>{t("Recommended next step", "推荐下一步")}</strong>
                              <p>{localizeBackendMessage(selectedDoctorReport.recommended_next_step, locale)}</p>
                            </div>
                          )}
                          {selectedDoctorReport.blockers.length > 0 && (
                            <div className="doctor-blocker-list">
                              {selectedDoctorReport.blockers.map((blocker) => (
                                <article key={blocker.id} className="doctor-blocker-card">
                                  <strong>{localizeDoctorLabel(blocker.label, locale)}</strong>
                                  <p>{localizeBackendMessage(blocker.summary, locale)}</p>
                                  {(blocker.fix_label || blocker.fix_command) && (
                                    <small>
                                      {blocker.fix_label
                                        ? locale === "zh-CN"
                                          ? localizeFixLabel(blocker.fix_label)
                                          : blocker.fix_label
                                        : t("Suggested fix", "建议修复")}
                                      {blocker.fix_command ? `: ${blocker.fix_command}` : ""}
                                    </small>
                                  )}
                                </article>
                              ))}
                            </div>
                          )}
                          {selectedDoctorReport.compose_requirements.length > 0 && (
                            <ComposeRequirements locale={locale} requirements={selectedDoctorReport.compose_requirements} />
                          )}
                          {selectedDoctorReport.port_conflicts.length > 0 && (
                            <div className="doctor-port-grid">
                              {selectedDoctorReport.port_conflicts.map((conflict) => (
                                <article key={`${selectedProject.id}-${conflict.port}`} className="doctor-port-card">
                                  <strong>Port {conflict.port}</strong>
                                  <p>{conflict.detail}</p>
                                  <small>
                                    {conflict.occupied
                                      ? conflict.can_auto_reassign
                                        ? t("Busy, but PortPilot can reassign it.", "端口被占用，但 PortPilot 可以自动改派。")
                                        : t("Busy, and this command cannot be auto-reassigned.", "端口被占用，而且这个命令无法自动改派。")
                                      : t("Currently free.", "当前空闲。")}
                                  </small>
                                </article>
                              ))}
                            </div>
                          )}
                          <SetupWizard
                            locale={locale}
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
                          <DoctorChecks checks={selectedDoctorReport.checks} locale={locale} />
                        </>
                      ) : (
                        <EmptyState
                          title={t("Building doctor report", "正在生成 Doctor 报告")}
                          description={t("PortPilot is checking tooling, env readiness, ports, and monorepo targets.", "PortPilot 正在检查工具链、环境变量、端口和 monorepo 目标。")}
                        />
                      )}
                    </section>

                    <section className="subpanel">
                      <h4>{t("Actions", "动作")}</h4>
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
                              {verbForAction(action.kind, locale)}
                            </button>
                          </div>
                        ))}
                        <button className="secondary-button" onClick={() => void handleStop(selectedProject)} type="button">
                          {t("Stop Active Execution", "停止当前执行")}
                        </button>
                      </div>
                    </section>

                    <section className="subpanel">
                      <h4>{t("Environment", "环境变量")}</h4>
                      <div id="env-editor" />
                      {envGroupPresets.length > 0 && (
                        <div className="env-preset-grid">
                          {envGroupPresets.map((preset) => (
                            <article key={preset.id} className="env-preset-card">
                              <div className="doctor-card__top">
                                <strong>{localizeEnvGroupLabel(preset.label, locale)}</strong>
                                <span className="status-pill status-pill--doctor-warn">
                                  {Object.keys(preset.values).length} {t("defaults", "默认值")}
                                </span>
                              </div>
                              <p>{localizeEnvGroupDescription(preset.description, locale)}</p>
                              {Object.keys(preset.values).length > 0 && (
                                <small>{Object.keys(preset.values).join(" • ")}</small>
                              )}
                              {preset.manual_keys.length > 0 && (
                                <small>
                                  {t("Still manual", "仍需手动填写")}: {preset.manual_keys.join(" • ")}
                                </small>
                              )}
                              <button
                                className="ghost-button"
                                onClick={() => handleApplyEnvGroupPreset(preset)}
                                type="button"
                              >
                                {t("Apply Local Defaults", "填入本地默认值")}
                              </button>
                            </article>
                          ))}
                        </div>
                      )}
                      {selectedProject.env_template.length > 0 ? (
                        <div className="env-grid">
                          {selectedProject.env_template.map((field) => (
                            <label key={field.key} className="field">
                              <span>{field.key}</span>
                              <small>{field.description ?? t("No description provided.", "没有描述信息。")}</small>
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
                          title={t("No env template detected", "未检测到 env 模板")}
                          description={t("Use the raw editor below to write a manual .env file if this repo has implicit environment variables.", "如果这个仓库有隐式环境变量，请使用下面的原始编辑器手动写入 .env 文件。")}
                        />
                      )}

                      <label className="field">
                        <span>{t("Advanced Raw Editor", "高级原始编辑器")}</span>
                        <textarea
                          rows={8}
                          value={envRawText}
                          onInput={(event) => setEnvRawText(event.currentTarget.value)}
                        />
                      </label>
                      <button className="primary-button" onClick={() => void handleSaveEnv()} type="button">
                        {t("Save .env", "保存 .env")}
                      </button>
                    </section>

                    <section className="subpanel">
                      <h4>{t("Runtime", "运行时")}</h4>
                      {selectedRuntimeNodes[0] && (
                        <article className="runtime-summary">
                          <div className="runtime-summary__row">
                            <strong>{selectedRuntimeNodes[0].execution_label ?? t("No active execution", "没有活跃执行")}</strong>
                            <span className={`status-pill status-pill--${selectedRuntimeNodes[0].status}`}>
                              {formatRuntimeStatus(selectedRuntimeNodes[0].status, locale)}
                            </span>
                          </div>
                          <div className="runtime-summary__meta">
                            <span>{t("Profile", "画像")}: {formatProfileKind(selectedRuntimeNodes[0].kind, locale)}</span>
                            <span>{t("Phase", "阶段")}: {formatRunPhase(selectedRuntimeNodes[0].run_phase, locale)}</span>
                            <span>{t("Port", "端口")}: {selectedRuntimeNodes[0].port ?? "n/a"}</span>
                            <span>{t("Route", "路由")}: {selectedRuntimeNodes[0].route_url}</span>
                          </div>
                          {selectedRuntimeNodes[0].health?.summary && (
                            <p className="runtime-summary__copy">{localizeBackendMessage(selectedRuntimeNodes[0].health?.summary, locale)}</p>
                          )}
                          {selectedRuntimeNodes[0].health?.readiness_reason && (
                            <small className="runtime-summary__reason">{localizeBackendMessage(selectedRuntimeNodes[0].health.readiness_reason, locale)}</small>
                          )}
                          {selectedRuntimeNodes[0].recommended_action && (
                            <div className="info-banner">
                              <strong>{t("Recommended action", "推荐动作")}</strong>
                              <p>{localizeBackendMessage(selectedRuntimeNodes[0].recommended_action, locale)}</p>
                            </div>
                          )}
                          {!selectedRuntimeNodes[0].dependencies_ready && (
                            <span className="status-pill status-pill--doctor-warn">{t("Waiting for services", "等待服务")}</span>
                          )}
                          {selectedRuntimeNodes[0].services.length > 0 && (
                            <div className="compose-service-list">
                              {selectedRuntimeNodes[0].services.map((service) => (
                                <article key={`${selectedRuntimeNodes[0].project_id}-${service.name}`} className="compose-service-chip">
                                  <strong>{service.name}</strong>
                                  <span>{localizeBackendMessage(service.state ?? "unknown", locale)}</span>
                                  {service.health && <span>{localizeBackendMessage(service.health, locale)}</span>}
                                  {service.published_ports[0] && <code>{service.published_ports[0]}</code>}
                                </article>
                              ))}
                            </div>
                          )}
                        </article>
                      )}
                      <div className="runtime-list">
                        {selectedExecutions.length > 0 ? (
                          selectedExecutions.map((execution) => (
                            <article key={execution.id} className="runtime-card">
                              <strong>{execution.label}</strong>
                              <p>{execution.command}</p>
                              <div className="runtime-card__meta">
                            <span>{localizeExecutionStatus(execution.status, locale)}</span>
                            <span>{execution.resolved_port ?? execution.port_hint ?? t("No port", "无端口")}</span>
                            <span>{execution.pid ?? t("No PID", "无 PID")}</span>
                          </div>
                        </article>
                      ))
                    ) : (
                      <EmptyState
                            title={t("No executions yet", "还没有执行记录")}
                            description={t("Run any action above and PortPilot will stream the result here.", "运行上面的任意动作后，PortPilot 会在这里显示结果。")}
                      />
                    )}
                  </div>
                    </section>

                    {selectedProject.workspace_targets.length > 0 && (
                      <section className="subpanel subpanel--full">
                        <h4>{t("Detected App Targets", "检测到的应用目标")}</h4>
                        <div className="target-grid">
                          {selectedProject.workspace_targets.map((target) => (
                            <article key={target.id} className="target-card">
                              <div className="target-card__header">
                                <strong>{target.name}</strong>
                                <span className={`status-pill ${selectedProject.primary_target_id === target.id ? "status-pill--doctor-ok" : "status-pill--doctor-info"}`}>
                                  {selectedProject.primary_target_id === target.id ? t("Recommended", "推荐") : `${t("Priority", "优先级")} ${target.priority}`}
                                </span>
                              </div>
                              <p>{target.relative_path}</p>
                              <div className="target-card__meta">
                                <span>{formatRuntimeKind(target.runtime_kind, locale)}</span>
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
                  title={t("No project selected", "还没有选中项目")}
                  description={t("Import or register a repository to unlock runtime and environment controls.", "导入或注册一个仓库后，就可以使用运行时和环境变量控制。")}
                />
              )}
            </section>
          </section>
        )}

        {view === "runtime" && (
          <section className="panel panel--wide">
            <div className="panel__header">
              <div>
                <h3>{t("Unified Runtime", "统一运行时")}</h3>
                <p>{t("See local processes, compose-backed apps, health signals, and routes in one place.", "在一个地方查看本地进程、Compose 应用、健康状态和路由。")}</p>
              </div>
            </div>
            <div className="panel__header">
              <div>
                <h3>{t("Local Service Presets", "本地服务预设")}</h3>
                <p>{t("Track shared localhost dependencies like Ollama, Redis, MongoDB, Postgres, and Meilisearch before you launch app stacks.", "在启动应用栈之前，先跟踪像 Ollama、Redis、MongoDB、Postgres 和 Meilisearch 这样的共享 localhost 依赖。")}</p>
              </div>
            </div>
            <div className="runtime-node-grid">
              {localServicePresets.map((service) => (
                <article key={service.name} className="runtime-node-card runtime-node-card--service">
                  <div className="runtime-summary__row">
                    <div>
                      <strong>{service.label}</strong>
                      <p>{service.hint ? localizeBackendMessage(service.hint, locale) : t("Shared localhost dependency", "共享 localhost 依赖")}</p>
                    </div>
                    <span className={`status-pill ${service.ready ? "status-pill--doctor-ok" : "status-pill--doctor-warn"}`}>
                      {service.ready ? t("Ready", "就绪") : t("Missing", "缺失")}
                    </span>
                  </div>
                  <div className="runtime-summary__meta">
                    <span>{t("Port", "端口")}: {service.port ?? "n/a"}</span>
                    <span>{t("Used by", "被以下项目使用")}: {service.used_by_projects.length}</span>
                  </div>
                  {service.used_by_projects.length > 0 && (
                    <p className="runtime-summary__copy">{service.used_by_projects.join(" • ")}</p>
                  )}
                  {service.start_command && (
                    <code className="runtime-node-card__log">{service.start_command}</code>
                  )}
                  <div className="action-row">
                    {service.start_command && (
                      <button
                        className="secondary-button"
                        onClick={() => void handleCopyServiceCommand(service)}
                        type="button"
                      >
                        {t("Copy Start Command", "复制启动命令")}
                      </button>
                    )}
                  </div>
                </article>
              ))}
              {localServicePresets.length === 0 && (
                <EmptyState
                  title={t("No shared local services detected", "还没有检测到共享本地服务")}
                  description={t("Import AI, gateway, or compose-heavy repos and PortPilot will surface common local dependencies here.", "导入 AI、网关或 Compose 较重的仓库后，PortPilot 会在这里显示常见本地依赖。")}
                />
              )}
            </div>
            <div className="runtime-node-grid">
              {runtimeNodes.map((node) => (
                <article key={node.project_id} className="runtime-node-card">
                  <div className="runtime-summary__row">
                    <div>
                      <strong>{node.project_name}</strong>
                      <p>{node.execution_label ?? t("No active execution", "没有活跃执行")}</p>
                    </div>
                    <span className={`status-pill status-pill--${node.status}`}>
                      {formatRuntimeStatus(node.status, locale)}
                    </span>
                  </div>
                  <div className="runtime-summary__meta">
                    <span>{t("Profile", "画像")}: {formatProfileKind(node.kind, locale)}</span>
                    <span>{t("Phase", "阶段")}: {formatRunPhase(node.run_phase, locale)}</span>
                    <span>{t("Port", "端口")}: {node.port ?? "n/a"}</span>
                    <span>{formatRuntimeKind(node.runtime_kind, locale)}</span>
                  </div>
                  {node.health?.summary && <p className="runtime-summary__copy">{localizeBackendMessage(node.health.summary, locale)}</p>}
                  {node.health?.readiness_reason && (
                    <small className="runtime-summary__reason">{localizeBackendMessage(node.health.readiness_reason, locale)}</small>
                  )}
                  {node.recommended_action && (
                    <div className="info-banner">
                      <strong>{t("Recommended action", "推荐动作")}</strong>
                      <p>{localizeBackendMessage(node.recommended_action, locale)}</p>
                    </div>
                  )}
                  {!node.dependencies_ready && (
                    <span className="status-pill status-pill--doctor-warn">{t("Waiting for services", "等待服务")}</span>
                  )}
                  {node.services.length > 0 && (
                    <div className="compose-service-list">
                      {node.services.map((service) => (
                        <article key={`${node.project_id}-${service.name}`} className="compose-service-chip">
                          <strong>{service.name}</strong>
                          <span>{localizeBackendMessage(service.state ?? "unknown", locale)}</span>
                          {service.health && <span>{localizeBackendMessage(service.health, locale)}</span>}
                          {service.published_ports[0] && <code>{service.published_ports[0]}</code>}
                        </article>
                      ))}
                    </div>
                  )}
                  {node.last_log && <code className="runtime-node-card__log">{node.last_log}</code>}
                  <div className="action-row">
                    <button
                      className="secondary-button"
                      onClick={() => void openUrl(node.route_url)}
                      type="button"
                    >
                      {t("Open Route", "打开路由")}
                    </button>
                    <button
                      className="ghost-button"
                      onClick={() => {
                        setSelectedProjectId(node.project_id);
                        setView("projects");
                      }}
                      type="button"
                    >
                      {t("Inspect Project", "查看项目")}
                    </button>
                  </div>
                </article>
              ))}
              {runtimeNodes.length === 0 && (
                <EmptyState title={t("No runtime nodes yet", "还没有运行节点")} description={t("Import a repo and start a run action to populate the runtime surface.", "导入仓库并启动 run 动作后，这里就会出现运行时节点。")} />
              )}
            </div>
          </section>
        )}

        {view === "routes" && (
          <section className="panel">
            <div className="panel__header">
              <h3>{t("Unified Routes", "统一路由")}</h3>
              <p>{t("Every managed repo gets both subdomain and path-style addresses.", "每个托管仓库都会获得子域名和路径两种地址。")}</p>
            </div>
            <DataTable
              locale={locale}
              headers={[
                t("Project", "项目"),
                "Slug",
                t("Target Port", "目标端口"),
                t("Subdomain", "子域名"),
                t("Path Route", "路径路由"),
              ]}
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
              <h3>{t("Port Center", "端口中心")}</h3>
              <p>{t("Track which managed app owns which port and which execution is behind it.", "跟踪每个托管应用占用了哪个端口，以及背后的执行实例。")}</p>
            </div>
            <DataTable
              locale={locale}
              headers={[t("Project", "项目"), t("Action", "动作"), t("Port", "端口"), "PID", t("Status", "状态")]}
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
              <div>
                <h3>{t("Live Logs", "实时日志")}</h3>
                <p>{t("Stream action output from installs, runs, builds, deploys, and compose flows.", "串流查看安装、运行、构建、部署和 Compose 流程的输出。")}</p>
              </div>
              <div className="log-toolbar">
                <select value={logStreamFilter} onChange={(event) => setLogStreamFilter(event.currentTarget.value as "all" | "stdout" | "stderr" | "system")}>
                  <option value="all">{t("All streams", "全部流")}</option>
                  <option value="stdout">stdout</option>
                  <option value="stderr">stderr</option>
                  <option value="system">system</option>
                </select>
                <input
                  placeholder={t("Search logs", "搜索日志")}
                  value={logQuery}
                  onInput={(event) => setLogQuery(event.currentTarget.value)}
                />
              </div>
            </div>
            <div className="log-console">
              {groupLogsByExecution(filteredLogs, executions).map((group) => (
                <div key={group.executionId} className="log-group">
                  <div className="log-group__header">
                    <strong>{group.label}</strong>
                    <span>{group.entries.length} {t("lines", "行")}</span>
                  </div>
                  {group.entries.map((entry) => (
                    <div key={`${entry.execution_id}-${entry.timestamp}-${entry.message}`} className={`log-line log-line--${entry.stream}`}>
                      <span>{entry.timestamp}</span>
                      <strong>{entry.stream}</strong>
                      <p>{entry.message}</p>
                    </div>
                  ))}
                </div>
              ))}
              {filteredLogs.length === 0 && (
                <EmptyState title={t("No logs yet", "还没有日志")} description={t("Run an action and PortPilot will begin streaming output.", "执行一个动作后，PortPilot 就会开始串流输出。")} />
              )}
            </div>
          </section>
        )}

        {view === "settings" && (
          <section className="panel-grid panel-grid--double">
            <section className="panel">
              <div className="panel__header">
                <h3>{t("Workspace Roots", "工作区根目录")}</h3>
                <p>{t("One path per line. PortPilot scans these locations for existing repos and uses the first root for Git imports.", "每行一个路径。PortPilot 会扫描这些位置的现有仓库，并用第一个根目录作为 Git 导入目标。")}</p>
              </div>
              <textarea
                className="settings-textarea"
                rows={10}
                value={workspaceDraft}
                onInput={(event) => setWorkspaceDraft(event.currentTarget.value)}
              />
              <button className="primary-button" onClick={() => void handleSaveRoots()} type="button">
                {t("Save Roots", "保存工作区")}
              </button>
            </section>
            <section className="panel">
              <div className="panel__header">
                <h3>{t("About & Updates", "关于与更新")}</h3>
                <p>{t("Cross-platform release status, auto-update controls, and release links.", "跨平台发布状态、自动更新控制和 release 链接。")}</p>
              </div>
              <Definition label={t("Current Version", "当前版本")} value={`v${currentVersion}`} />
              <Definition
                label={t("Update State", "更新状态")}
                value={
                  update.hasUpdate
                    ? (locale === "zh-CN" ? `有可用更新：v${update.updateInfo?.availableVersion ?? "未知"}` : `Update available: v${update.updateInfo?.availableVersion ?? "unknown"}`)
                    : update.phase === "checking"
                      ? t("Checking for updates...", "正在检查更新...")
                      : update.phase === "upToDate"
                        ? t("Up to date", "已是最新")
                        : update.phase
                }
              />
              {update.hasUpdate && update.updateInfo && !update.isDismissed && (
                <div className="update-banner">
                  <strong>{locale === "zh-CN" ? `PortPilot ${update.updateInfo.availableVersion} 已可用。` : `PortPilot ${update.updateInfo.availableVersion} is available.`}</strong>
                  <p>{update.updateInfo.notes ?? t("A new cross-platform release is ready to install.", "新的跨平台版本已经可以安装。")}</p>
                  {update.progressTotal > 0 && (
                    <p>
                      {locale === "zh-CN" ? "已下载" : "Downloaded"} {Math.min(update.progressDownloaded, update.progressTotal)} / {update.progressTotal} bytes
                    </p>
                  )}
                  <div className="action-row">
                    <button className="primary-button" onClick={() => void handleInstallUpdate()} type="button">
                      {t("Install Update", "安装更新")}
                    </button>
                    <button className="secondary-button" onClick={() => update.dismissUpdate()} type="button">
                      {t("Dismiss This Version", "忽略这个版本")}
                    </button>
                    <button
                      className="ghost-button"
                      onClick={() => void openUrl("https://github.com/Horace-Maxwell/portpilot/releases")}
                      type="button"
                    >
                      {t("Release Notes", "更新说明")}
                    </button>
                  </div>
                </div>
              )}
              {update.hasUpdate && update.updateInfo && update.isDismissed && (
                <div className="action-row">
                  <button className="secondary-button" onClick={() => update.resetDismiss()} type="button">
                    {t("Show Dismissed Update", "显示已忽略更新")}
                  </button>
                </div>
              )}
              {!update.hasUpdate && (
                <div className="action-row">
                  <button className="primary-button" onClick={() => void handleCheckUpdate()} type="button">
                    {t("Check for Updates", "检查更新")}
                  </button>
                  <button
                    className="secondary-button"
                    onClick={() => void openUrl("https://github.com/Horace-Maxwell/portpilot/releases")}
                    type="button"
                  >
                    {t("Open Releases", "打开 Releases")}
                  </button>
                </div>
              )}
              {update.error && <p className="error-copy">{update.error}</p>}
              <Definition label={t("Workspace Roots", "工作区根目录")} value={String(workspaceRoots.length)} />
              <Definition label={t("Managed Projects", "托管项目")} value={String(projects.length)} />
              <Definition
                label={t("Running Executions", "运行中执行")}
                value={String(executions.filter((execution) => execution.status === "running").length)}
              />
              <Definition label={t("Language", "语言")} value={locale === "zh-CN" ? "中文" : "English"} />
              <Definition label={t("Last Status", "最新状态")} value={statusMessage} />
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

function projectPrimaryTarget(project: ManagedProject) {
  return (
    project.workspace_targets.find((target) => target.id === project.primary_target_id) ??
    project.workspace_targets[0] ??
    null
  );
}

function navLabel(key: NavKey, locale: Locale) {
  const zh: Record<NavKey, string> = {
    dashboard: "总览",
    import: "导入",
    projects: "项目",
    runtime: "运行时",
    routes: "路由",
    ports: "端口",
    logs: "日志",
    settings: "设置",
  };
  const en: Record<NavKey, string> = {
    dashboard: "Dashboard",
    import: "Import",
    projects: "Projects",
    runtime: "Runtime",
    routes: "Routes",
    ports: "Ports",
    logs: "Logs",
    settings: "Settings",
  };
  return locale === "zh-CN" ? zh[key] : en[key];
}

function formatProfileKind(kind: ManagedProject["project_profile"]["kind"], locale: Locale) {
  const labels: Record<ManagedProject["project_profile"]["kind"], { en: string; zh: string }> = {
    unknown: { en: "unknown", zh: "未知" },
    web_app: { en: "web app", zh: "Web 应用" },
    ai_ui: { en: "AI UI", zh: "AI 界面" },
    gateway_stack: { en: "gateway stack", zh: "网关栈" },
    compose_stack: { en: "compose stack", zh: "Compose 栈" },
    fullstack_mixed: { en: "fullstack mixed", zh: "混合全栈" },
  };
  return locale === "zh-CN" ? labels[kind].zh : labels[kind].en;
}

function formatRuntimeKind(kind: RuntimeKind, locale: Locale) {
  const labels: Record<RuntimeKind, { en: string; zh: string }> = {
    node: { en: "node", zh: "Node" },
    python: { en: "python", zh: "Python" },
    rust: { en: "rust", zh: "Rust" },
    go: { en: "go", zh: "Go" },
    compose: { en: "compose", zh: "Compose" },
    unknown: { en: "unknown", zh: "未知" },
  };
  return locale === "zh-CN" ? labels[kind].zh : labels[kind].en;
}

function formatRuntimeStatus(status: RuntimeStatus, locale: Locale) {
  const labels: Record<RuntimeStatus, { en: string; zh: string }> = {
    stopped: { en: "stopped", zh: "已停止" },
    starting: { en: "starting", zh: "启动中" },
    running: { en: "running", zh: "运行中" },
    unhealthy: { en: "unhealthy", zh: "异常" },
    port_conflict: { en: "port conflict", zh: "端口冲突" },
    error: { en: "error", zh: "错误" },
  };
  return locale === "zh-CN" ? labels[status].zh : labels[status].en;
}

function formatRunPhase(phase: RunPhase | null, locale: Locale) {
  if (!phase) {
    return locale === "zh-CN" ? "未知" : "unknown";
  }
  const labels: Record<RunPhase, { en: string; zh: string }> = {
    installing: { en: "installing", zh: "安装中" },
    starting: { en: "starting", zh: "启动中" },
    waiting_for_port: { en: "waiting for port", zh: "等待端口" },
    waiting_for_service: { en: "waiting for service", zh: "等待服务" },
    healthy: { en: "healthy", zh: "健康" },
    failed: { en: "failed", zh: "失败" },
    stopped: { en: "stopped", zh: "已停止" },
  };
  return locale === "zh-CN" ? labels[phase].zh : labels[phase].en;
}

function formatRouteStrategy(strategy: NonNullable<ProjectProfile["route_strategy"]>, locale: Locale) {
  const labels: Record<NonNullable<ProjectProfile["route_strategy"]>, { en: string; zh: string }> = {
    gateway_path: { en: "gateway path", zh: "网关路径" },
    localhost_direct: { en: "localhost direct", zh: "localhost 直连" },
    compose_service: { en: "compose service", zh: "Compose 服务" },
    hybrid: { en: "hybrid", zh: "混合" },
  };
  return locale === "zh-CN" ? labels[strategy].zh : labels[strategy].en;
}

function localizeBackendMessage(message: string, locale: Locale) {
  if (locale !== "zh-CN") {
    return message;
  }

  const replacements: Array<[RegExp, string]> = [
    [/Start the required compose services first, then run the recommended entrypoint\./g, "先启动所需的 Compose 服务，再运行推荐入口。"],
    [/Start the required local services first\./g, "先启动所需的本地服务。"],
    [/Open the live route or inspect the runtime panel\./g, "打开在线路由或查看运行时面板。"],
    [/Open the live route or inspect recent logs\./g, "打开在线路由或查看最近日志。"],
    [/Fill in the required compose env values before starting this stack\./g, "先补齐 Compose 所需环境变量，再启动这个栈。"],
    [/Fill in the required compose env values for ([^.]+) before starting this stack\./g, "先补齐 $1 所需的 Compose 环境变量，再启动这个栈。"],
    [/Run install first, then start the primary action\./g, "先执行安装，再启动主动作。"],
    [/Start the primary run action to bring this repo online\./g, "启动主运行动作，让这个仓库上线。"],
    [/Port opened or the process emitted a ready signal\./g, "端口已打开，或进程已经输出 ready 信号。"],
    [/Required local services are not ready yet\./g, "依赖的本地服务还没准备好。"],
    [/Waiting for supporting services before this project can be considered healthy\./g, "正在等待依赖服务，这个项目还不能算健康。"],
    [/Waiting for the project to bind a port or emit a ready signal\./g, "正在等待项目绑定端口或输出 ready 信号。"],
    [/Route is reachable and the process looks ready\./g, "路由已可访问，进程看起来已经就绪。"],
    [/Run (.+) to bring this project online\./g, "运行 $1，让这个项目上线。"],
    [/Start (.+) first \(`(.+)` on localhost:(\d+)\), then run the recommended entrypoint\./g, "先启动 $1（在 localhost:$3 上执行 `$2`），再运行推荐入口。"],
    [/Free fixed port (\d+) or change the command arguments before starting this project\./g, "请先释放固定端口 $1，或修改命令参数后再启动这个项目。"],
    [/Compose is missing (\d+) required env values?: (.+)\./g, "Compose 缺少 $1 个必需环境变量：$2。"],
    [/Start the required local services? first: (.+)\./g, "请先启动所需的本地服务：$1。"],
    [/This project hardcodes its port in ([^,]+), so PortPilot cannot move it automatically\./g, "这个项目在 $1 里写死了端口，所以 PortPilot 无法自动改派。"],
    [/This project hardcodes its port, so PortPilot cannot move it automatically\./g, "这个项目写死了端口，所以 PortPilot 无法自动改派。"],
    [/\brunning\b/g, "运行中"],
    [/\bstopped\b/g, "已停止"],
    [/\bmissing\b/g, "缺失"],
    [/\bready\b/g, "就绪"],
    [/local dependency/g, "本地依赖"],
    [/No active execution/g, "没有活跃执行"],
    [/\bunknown\b/g, "未知"],
  ];

  let localized = message;
  for (const [pattern, replacement] of replacements) {
    localized = localized.replace(pattern, replacement);
  }
  return localized;
}

function groupLogsByExecution(logs: LogEntry[], executions: ActionExecution[]) {
  const labelById = new Map(
    executions.map((execution) => [execution.id, `${execution.label} · ${execution.command}`]),
  );
  const groups = new Map<string, { executionId: string; label: string; entries: LogEntry[] }>();

  for (const entry of logs) {
    const existing = groups.get(entry.execution_id);
    if (existing) {
      existing.entries.push(entry);
      continue;
    }
    groups.set(entry.execution_id, {
      executionId: entry.execution_id,
      label: labelById.get(entry.execution_id) ?? entry.execution_id,
      entries: [entry],
    });
  }

  return Array.from(groups.values()).sort((left, right) => {
    const leftTime = left.entries[left.entries.length - 1]?.timestamp ?? "";
    const rightTime = right.entries[right.entries.length - 1]?.timestamp ?? "";
    return rightTime.localeCompare(leftTime);
  });
}

function upsertById<T extends { id: string }>(items: T[], next: T): T[] {
  const found = items.some((item) => item.id === next.id);
  if (!found) return [next, ...items];
  return items.map((item) => (item.id === next.id ? next : item));
}

function verbForAction(kind: ActionKind, locale: Locale) {
  switch (kind) {
    case "install":
      return locale === "zh-CN" ? "安装" : "Install";
    case "run":
      return locale === "zh-CN" ? "运行" : "Run";
    case "stop":
      return locale === "zh-CN" ? "停止" : "Stop";
    case "restart":
      return locale === "zh-CN" ? "重启" : "Restart";
    case "build":
      return locale === "zh-CN" ? "构建" : "Build";
    case "deploy":
      return locale === "zh-CN" ? "部署" : "Deploy";
    case "open":
      return locale === "zh-CN" ? "打开" : "Open";
    case "logs":
      return locale === "zh-CN" ? "日志" : "Logs";
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

function SelectionToolbar(props: {
  locale: Locale;
  selectedCount: number;
  allSelected: boolean;
  onToggleAll: () => void;
  onRun: () => void;
  onStop: () => void;
  onRestart: () => void;
  onSaveSession: () => void;
}) {
  return (
    <div className="selection-toolbar">
      <button className="ghost-button" onClick={props.onToggleAll} type="button">
        {props.allSelected ? (props.locale === "zh-CN" ? "清空当前视图" : "Clear Visible") : props.locale === "zh-CN" ? "选择当前视图" : "Select Visible"}
      </button>
      <span>{props.selectedCount} {props.locale === "zh-CN" ? "已选择" : "selected"}</span>
      <div className="action-row">
        <button className="primary-button" disabled={props.selectedCount === 0} onClick={props.onRun} type="button">
          {props.locale === "zh-CN" ? "启动所选" : "Start Selected"}
        </button>
        <button className="secondary-button" disabled={props.selectedCount === 0} onClick={props.onStop} type="button">
          {props.locale === "zh-CN" ? "停止所选" : "Stop Selected"}
        </button>
        <button className="secondary-button" disabled={props.selectedCount === 0} onClick={props.onRestart} type="button">
          {props.locale === "zh-CN" ? "重启所选" : "Restart Selected"}
        </button>
        <button className="ghost-button" disabled={props.selectedCount === 0} onClick={props.onSaveSession} type="button">
          {props.locale === "zh-CN" ? "保存为工作区" : "Save as Session"}
        </button>
      </div>
    </div>
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

function DataTable(props: { headers: string[]; rows: string[][]; locale: Locale }) {
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
        <EmptyState title={props.locale === "zh-CN" ? "还没有数据" : "No data yet"} description={props.locale === "zh-CN" ? "运行一些项目后，PortPilot 会把这个视图填满。" : "Run some projects and PortPilot will populate this view."} />
      )}
    </div>
  );
}

function SetupWizard(props: {
  locale: Locale;
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
      label: props.locale === "zh-CN" ? "项目已导入" : "Project imported",
      status: "done",
      description: props.locale === "zh-CN" ? "这个仓库已经被 PortPilot 托管。" : "This repo is already managed by PortPilot.",
      action: null,
    },
    {
      id: "install",
      label: props.locale === "zh-CN" ? "安装依赖" : "Install dependencies",
      status:
        props.report.install_action_id == null || installState?.status === "ok" ? "done" : "todo",
      description:
        installState?.summary ??
        (props.locale === "zh-CN"
          ? "PortPilot 已为这个仓库推断出安装动作。"
          : "PortPilot inferred an install action for this repository."),
      action:
        props.report.install_action_id != null && installState?.status !== "ok"
          ? { label: props.locale === "zh-CN" ? "执行安装" : "Run install", onClick: props.onInstall }
          : null,
    },
    {
      id: "env",
      label: props.locale === "zh-CN" ? "填写环境变量" : "Fill environment",
      status: envReady ? "done" : "todo",
      description: envReady
        ? props.locale === "zh-CN" ? "检测到的环境模板所需值已经基本就绪。" : "Environment values look ready for the detected template."
        : props.locale === "zh-CN" ? `缺少 ${props.report.missing_env_keys.length} 个值：${props.report.missing_env_keys.join(", ")}` : `Missing ${props.report.missing_env_keys.length} value(s): ${props.report.missing_env_keys.join(", ")}`,
      action: envReady ? null : { label: props.locale === "zh-CN" ? "打开环境编辑器" : "Open env editor", onClick: props.onFocusEnv },
    },
    {
      id: "run",
      label: props.locale === "zh-CN" ? "启动应用" : "Start the app",
      status: runReady ? "done" : "todo",
      description: runReady
        ? props.locale === "zh-CN"
          ? "主运行动作已经在线。"
          : "The primary run action is live."
        : props.locale === "zh-CN"
          ? "依赖和环境变量就绪后，使用推断出的主运行动作。"
          : "Use the inferred primary run action once dependencies and env values are ready.",
      action:
        !runReady && props.report.run_action_id != null
          ? { label: props.locale === "zh-CN" ? "运行主动作" : "Run primary action", onClick: props.onRun }
          : null,
    },
    {
      id: "open",
      label: props.locale === "zh-CN" ? "打开路由预览" : "Open routed preview",
      status: runReady ? "ready" : "todo",
      description: runReady
        ? props.locale === "zh-CN"
          ? "在浏览器或嵌入预览中打开 PortPilot 的统一路由。"
          : "Open the unified PortPilot route in the browser or embedded preview."
        : props.locale === "zh-CN"
          ? "运行动作开始响应后，预览才会真正可用。"
          : "The preview becomes useful after the run action starts responding.",
      action: runReady ? { label: props.locale === "zh-CN" ? "打开路由" : "Open route", onClick: props.onOpen } : null,
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

function DoctorChecks(props: { checks: DoctorReport["checks"]; locale: Locale }) {
  return (
    <div className="doctor-list">
      {props.checks.map((check) => (
        <article key={check.id} className={`doctor-card doctor-card--${check.status}`}>
          <div className="doctor-card__top">
            <strong>{localizeDoctorLabel(check.label, props.locale)}</strong>
            <span className={`status-pill status-pill--doctor-${check.status}`}>{props.locale === "zh-CN" ? doctorStatusLabelZh(check.status) : check.status}</span>
          </div>
          <p>{localizeBackendMessage(check.summary, props.locale)}</p>
          {check.detail && <small>{localizeBackendMessage(check.detail, props.locale)}</small>}
          {check.fix_label && check.fix_command && (
            <code>
              {(props.locale === "zh-CN" ? localizeFixLabel(check.fix_label) : check.fix_label)}: {check.fix_command}
            </code>
          )}
        </article>
      ))}
    </div>
  );
}

function ComposeRequirements(props: { requirements: ComposeRequirement[]; locale: Locale }) {
  return (
    <section className="doctor-requirements">
      <div className="doctor-requirements__grid">
        {props.requirements.map((requirement) => (
          <article key={`${requirement.kind}-${requirement.name}`} className="doctor-requirement-card">
            <div className="doctor-card__top">
              <strong>{requirement.name}</strong>
              <span className={`status-pill ${requirement.ready ? "status-pill--doctor-ok" : "status-pill--doctor-warn"}`}>
                {formatRequirementKind(requirement.kind, props.locale)} · {requirement.ready ? (props.locale === "zh-CN" ? "就绪" : "ready") : props.locale === "zh-CN" ? "缺失" : "missing"}
              </span>
            </div>
            {requirement.detail && <p>{localizeBackendMessage(requirement.detail, props.locale)}</p>}
          </article>
        ))}
      </div>
    </section>
  );
}

function formatRequirementKind(kind: ComposeRequirement["kind"], locale: Locale) {
  if (locale !== "zh-CN") {
    return kind.replace(/-/g, " ");
  }
  const labels: Record<string, string> = {
    service: "服务",
    "local-service": "本地服务",
    env: "环境变量",
  };
  return labels[kind] ?? kind;
}

function doctorStatusLabelZh(status: DoctorReport["checks"][number]["status"]) {
  const labels: Record<DoctorReport["checks"][number]["status"], string> = {
    ok: "正常",
    warn: "警告",
    error: "错误",
    info: "信息",
  };
  return labels[status];
}

function localizeFixLabel(label: string) {
  const labels: Record<string, string> = {
    "Suggested fix": "建议修复",
    "Suggested start": "建议启动",
    "Fill env values": "填写环境变量",
    "Free the port": "释放端口",
  };
  return labels[label] ?? label;
}

function localizeDoctorLabel(label: string, locale: Locale) {
  if (locale !== "zh-CN") {
    return label;
  }
  const labels: Record<string, string> = {
    Tooling: "工具链",
    Environment: "环境变量",
    "Install State": "安装状态",
    "Port Conflict": "端口冲突",
    "Primary Target": "推荐目标",
    "Monorepo Targets": "Monorepo 目标",
    "README Hints": "README 提示",
    "Fixed Port Conflict": "固定端口冲突",
    "Compose Env": "Compose 环境变量",
    "Local Services": "本地服务",
  };
  return labels[label] ?? label;
}

function localizeEnvGroupLabel(label: string, locale: Locale) {
  if (locale !== "zh-CN") {
    return label;
  }
  const labels: Record<string, string> = {
    App: "应用",
    Database: "数据库",
    Search: "搜索",
    RAG: "RAG",
    Queue: "队列",
    Workspace: "工作区",
    Gateway: "网关",
    Credentials: "凭据",
    "Model Providers": "模型提供商",
    "LLM Provider": "LLM 提供商",
    Models: "模型",
    Frontend: "前端",
    Server: "服务端",
    Environment: "环境",
  };
  return labels[label] ?? label;
}

function localizeEnvGroupDescription(description: string, locale: Locale) {
  if (locale !== "zh-CN") {
    return description;
  }
  const labels: Record<string, string> = {
    "Good local defaults for the primary app URL and port.": "为主应用的 URL 和端口填入常见本地默认值。",
    "Fill the most common localhost database values for this stack.": "为这个栈填入最常见的本地数据库默认值。",
    "Preset local search or vector service endpoints.": "预填本地搜索或向量服务地址。",
    "Preset the local RAG sidecar URL and port.": "预填本地 RAG 辅助服务的 URL 和端口。",
    "Preset the local queue/cache service values.": "预填本地队列或缓存服务值。",
    "Set repo-local working directories required by this stack.": "设置这个栈要求的仓库内工作目录。",
    "Preset localhost gateway and webchat entrypoints.": "预填 localhost 网关和 WebChat 入口。",
    "Keys in this group usually still need real secrets.": "这组键通常仍然需要真实密钥。",
    "Provider keys usually need manual input even in local mode.": "即使是本地模式，这组提供商密钥通常也需要手动填写。",
    "Provider-specific credentials still need manual input.": "提供商相关凭据仍然需要手动填写。",
    "Model paths and provider tokens often need manual input.": "模型路径和提供商 token 通常仍需手动填写。",
    "Preset the local frontend URL and port.": "预填本地前端 URL 和端口。",
    "Preset the local server URL and port.": "预填本地服务端 URL 和端口。",
    "Local development defaults for this environment group.": "这组环境变量的本地开发默认值。",
  };
  return labels[description] ?? description;
}

function localizeExecutionStatus(
  status: ActionExecution["status"],
  locale: Locale,
) {
  if (locale !== "zh-CN") {
    return status;
  }
  const labels: Record<ActionExecution["status"], string> = {
    running: "运行中",
    success: "成功",
    failed: "失败",
    stopped: "已停止",
  };
  return labels[status];
}

function localizeBatchKind(kind: string, locale: Locale) {
  if (locale !== "zh-CN") {
    return kind;
  }
  const labels: Record<string, string> = {
    run: "批量启动",
    stop: "批量停止",
    restart: "批量重启",
    restore: "恢复工作区",
  };
  return labels[kind] ?? kind;
}
