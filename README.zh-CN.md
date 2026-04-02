[English](./README.md) | **简体中文**

<div align="center">
  <h1>PortPilot</h1>
  <p><strong>为 localhost 应用整栈准备的桌面工作台。</strong></p>
  <p>导入 GitHub 仓库，填入安全的本地默认值，启动正确整栈，再把路由、日志、端口、健康状态和共享服务收进一个界面里。</p>

  <p>
    <a href="https://github.com/Horace-Maxwell/portpilot/releases/tag/v0.1.2">
      <img src="https://img.shields.io/badge/Download-v0.1.2-0f172a?style=for-the-badge&logo=github" alt="Download v0.1.2" />
    </a>
    <a href="https://github.com/Horace-Maxwell/portpilot/releases">
      <img src="https://img.shields.io/badge/Browse-Releases-1d4ed8?style=for-the-badge&logo=github" alt="Browse Releases" />
    </a>
    <a href="./README.md">
      <img src="https://img.shields.io/badge/Read%20in-English-0f766e?style=for-the-badge" alt="Read in English" />
    </a>
  </p>

  <p>
    <img src="https://img.shields.io/github/v/release/Horace-Maxwell/portpilot?display_name=tag&style=for-the-badge" alt="Release" />
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-0f766e?style=for-the-badge" alt="Platforms" />
    <img src="https://img.shields.io/badge/localhost-HTTPS%20%2B%20routes-111827?style=for-the-badge" alt="localhost platform" />
    <img src="https://img.shields.io/badge/Tauri-2.x-24C8DB?style=for-the-badge&logo=tauri" alt="Tauri" />
    <img src="https://img.shields.io/github/license/Horace-Maxwell/portpilot?style=for-the-badge" alt="License" />
  </p>
</div>

![PortPilot hero](./docs/media/hero-banner.svg)

## PortPilot 是什么

PortPilot 的目标，是把“很难在本机顺手跑起来”的仓库，变成一个可控的桌面工作流：

- 导入仓库，或注册已有本地目录
- 为 Node、Python、Rust、Go、Compose-heavy 项目推断正确入口
- 在真正卡住之前先填入安全的本地默认 env
- 启动的是整栈，而不是孤零零一条命令
- 通过本地网关把应用收进干净的 `.localhost` 地址
- 把日志、端口、健康状态、依赖服务和 Doctor 提示放进同一个工作台

如果你现在的日常是“五个终端、一个 README、两份 `.env`、一个 Docker 服务，再加一次端口冲突”，PortPilot 就是为这个时刻准备的。

## 适合谁

- 同时维护多个 localhost 仓库的独立开发者
- 正在折腾 Open WebUI、Flowise、LibreChat、OpenClaw、ComfyUI、LocalAI 这类 AI 本地栈的人
- 需要接管 Node + Python + Compose 混合仓库的全栈团队
- 不想把一切都搬到云开发环境里，但也不想继续靠 README 和终端硬撑的人

## 为什么值得用

| 一次导入 | 一键起整栈 | 出问题时知道坏在哪 |
| --- | --- | --- |
| 把 GitHub URL 和本地目录整理成托管项目档案。 | 自动启动推荐入口和可受管依赖服务。 | Doctor、日志、路由、健康、端口和运行态都留在一个界面里。 |

## 它和别的工具怎么分工

PortPilot 不是要替代所有开发工具，而是补上“本机仓库工作台”这一层。

| 工具 | 更强的地方 | PortPilot 的位置 |
| --- | --- | --- |
| Portainer | 容器和环境管理 | PortPilot 更强在 repo-first 的 localhost 工作流和混合应用栈 |
| DDEV | 高度成体系的本地 web 环境 | PortPilot 更适合更广的仓库类型，不绑死在单一栈模型 |
| ServBay | 本地服务和 GUI 管理 | PortPilot 额外提供 repo 导入、动作推断、运行态和项目级路由 |
| 终端 + README | 灵活度最高 | PortPilot 负责减少重复初始化、路由和排障成本 |

## 已经在真实仓库上验证过

PortPilot 是按人们真实会 clone 下来跑的仓库来打磨的：

| 仓库 | 栈类型 | PortPilot 会给什么 |
| --- | --- | --- |
| [`calesthio/Crucix`](https://github.com/calesthio/Crucix) | Node + Compose | install、dev 入口、env 模板、Compose 动作、统一访问地址 |
| [`koala73/worldmonitor`](https://github.com/koala73/worldmonitor) | desktop + web | web 目标、desktop 目标、构建变体、本地运行态 |
| [`open-webui/open-webui`](https://github.com/open-webui/open-webui) | Python + web + Compose | backend 入口、frontend 提示、Ollama 依赖、本地路由 |
| [`openclaw/openclaw`](https://github.com/openclaw/openclaw) | gateway stack | gateway 入口、workspace/config 阻塞项、Compose 需求 |
| [`FlowiseAI/Flowise`](https://github.com/FlowiseAI/Flowise) | AI UI + env-heavy Compose | 本地 env 预设、整栈启动路径、服务依赖 |
| [`danny-avila/LibreChat`](https://github.com/danny-avila/LibreChat) | gateway stack + RAG services | Mongo + Meili + RAG 默认值、依赖服务、推荐启动顺序 |
| [`SillyTavern/SillyTavern`](https://github.com/SillyTavern/SillyTavern) | localhost web app | 固定端口识别、路由管理、运行态可见性 |
| [`comfyanonymous/ComfyUI`](https://github.com/comfyanonymous/ComfyUI) | Python localhost app | 已知入口、已知端口、运行指导 |

## 产品预览

### 一个工作区接管多个项目

![Dashboard preview](./docs/media/dashboard-preview.png)

### 项目页围绕动作、环境变量和运行时展开

![Actions preview](./docs/media/actions-preview.png)

### 路由、日志、端口和服务收进同一块界面

![Observability preview](./docs/media/observability-preview.png)

## 核心使用路径

1. 添加工作区根目录。
2. 导入 GitHub 仓库，或注册现有本地项目。
3. 查看 Doctor 提示，并填入本地默认值。
4. 点击 `Launch Stack`。
5. 通过 `.localhost` 和本地 HTTPS 打开应用。

## 核心能力

| 模块 | 能力 |
| --- | --- |
| 导入 | 从 GitHub clone，或注册已有本地仓库 |
| 识别 | 为 Node、Python、Rust、Go、Docker Compose 自动推断入口 |
| 环境变量 | 解析 env 模板，应用本地预设，并保存可编辑 `.env` |
| 整栈启动 | 启动、重启、停止推荐的本地整栈 |
| 本地平台 | 管理 Ollama、Redis、MongoDB、Postgres、Meilisearch 等共享服务 |
| 路由 | 通过本地网关分配干净的 `.localhost` 地址 |
| 观测 | 在一个 UI 里查看日志、端口、健康状态、路由和运行态 |
| 更新 | 通过 GitHub Releases 发布，并支持稳定更新 feed |

## 下载

PortPilot `v0.1.2` 现在已经可通过 [GitHub Releases](https://github.com/Horace-Maxwell/portpilot/releases) 下载。

- macOS: `.dmg`, `.zip`, updater `.tar.gz`
- Windows x64: `.msi`, 可选 `.zip`
- Linux x64: `.AppImage`, `.deb`, `.rpm`

## 快速开始

```bash
npm install
npm run tauri:dev
```

然后：

1. 添加工作区根目录。
2. 导入一个仓库，例如 `https://github.com/open-webui/open-webui.git`。
3. 查看 Doctor 阻塞项。
4. 应用本地默认值。
5. 点击 `Launch Stack`。

## 开发校验

```bash
npm run typecheck
npm test
npm run build
npm run tauri build -- --debug
```

## 贡献

欢迎提交 Issue 和 PR，尤其是仓库推断、localhost 服务管理、运行时编排、路由和打包相关方向。

## License

MIT，见 [LICENSE](./LICENSE)。
