[English](./README.md) | **简体中文**

<div align="center">
  <h1>PortPilot</h1>
  <p><strong>为本地优先 GitHub 仓库准备的桌面控制台。</strong></p>
  <p>导入仓库、填写环境变量、运行正确命令、分配干净的 <code>.localhost</code> 地址，再把日志、端口和健康状态收进一个界面里。</p>

  <p>
    <a href="https://github.com/Horace-Maxwell/portpilot/releases/tag/v0.1.0-beta.1">
      <img src="https://img.shields.io/badge/Download-Beta-0f172a?style=for-the-badge&logo=github" alt="Download Beta" />
    </a>
    <a href="https://github.com/Horace-Maxwell/portpilot/releases">
      <img src="https://img.shields.io/badge/Browse-Releases-1d4ed8?style=for-the-badge&logo=github" alt="Browse Releases" />
    </a>
    <a href="./README.md">
      <img src="https://img.shields.io/badge/Read%20in-English-0f766e?style=for-the-badge" alt="Read in English" />
    </a>
  </p>

  <p>
    <img src="https://img.shields.io/github/v/release/Horace-Maxwell/portpilot?include_prereleases&display_name=tag&style=for-the-badge" alt="Release" />
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-0f766e?style=for-the-badge" alt="Platforms" />
    <img src="https://img.shields.io/badge/Tauri-2.x-24C8DB?style=for-the-badge&logo=tauri" alt="Tauri" />
    <img src="https://img.shields.io/badge/Rust%20%2B%20TypeScript-core%20stack-111827?style=for-the-badge&logo=rust" alt="Rust and TypeScript" />
    <img src="https://img.shields.io/github/license/Horace-Maxwell/portpilot?style=for-the-badge" alt="License" />
  </p>
</div>

![PortPilot hero](./docs/media/hero-banner.svg)

## 为什么是 PortPilot

| 一次导入 | 一个地方控制 | 一眼看到运行态 |
| --- | --- | --- |
| 把 GitHub URL 和本地目录整理成可运行的项目档案。 | 不再来回切终端，直接执行 run、stop、build、deploy。 | 在同一块桌面里查看路由、日志、端口和状态。 |

## 真实项目验证

PortPilot 的目标就是让这类仓库更容易被一键接管：

| 仓库 | PortPilot 会做什么 |
| --- | --- |
| [`calesthio/Crucix`](https://github.com/calesthio/Crucix) | 识别 `npm install`、`npm run dev`、环境变量模板、compose 动作和统一访问地址 |
| [`koala73/worldmonitor`](https://github.com/koala73/worldmonitor) | 识别 web 运行入口、`desktop:dev`、`build:*`、`desktop:build:*` 和本地运行态 |

## 产品预览

### 一个总览页接管多个项目

![Dashboard preview](./docs/media/dashboard-preview.png)

### 项目页直接围绕动作设计

![Actions preview](./docs/media/actions-preview.png)

### 路由、日志、端口放在一起看

![Observability preview](./docs/media/observability-preview.png)

## 核心能力

| 模块 | 能力 |
| --- | --- |
| 导入 | 贴 GitHub URL 自动 clone，或接管已有本地项目 |
| 识别 | 自动推断 Node / Python / Rust / Go / Docker Compose 项目的动作 |
| 环境变量 | 读取 `.env.example`，保存 env profile，并生成本地 `.env` |
| 运行时 | 一键运行、停止、重启、构建、部署，并记录执行历史 |
| 路由 | 通过本地 gateway 暴露干净的 `.localhost` 地址 |
| 观测 | 在一个界面中查看日志、端口、状态和 route bindings |
| 更新 | 通过 GitHub Releases 发布，并为后续应用内更新铺路 |

## 快速开始

```bash
npm install
npm run tauri:dev
```

启动后：

1. 先确认或添加你的 workspace 根目录。
2. 粘贴 GitHub 仓库地址，例如 `https://github.com/calesthio/Crucix.git`。
3. 导入仓库，检查动作推断结果，填写环境变量，然后点 `Run`。

## 下载与安装

PortPilot 现在已经以 GitHub 公开 Beta 的形式提供下载。

- 进入 [Releases](https://github.com/Horace-Maxwell/portpilot/releases)
- 下载你平台对应的主安装包：macOS `.dmg`、Windows `.msi` 或 Linux `.AppImage`
- macOS Beta 首次打开时，如果系统拦截，需要去系统设置里手动允许，因为这一版还不是已公证安装包

### 当前 Beta 资产

- macOS: `.dmg`, `.zip`, updater `.tar.gz`
- Windows x64: `.msi`, 可选 `.zip`
- Linux x64: `.AppImage`, `.deb`, `.rpm`

## Beta 说明

- `v0.1.0-beta.1` 是第一个公开验证版本
- macOS Beta 可能会先遇到 Gatekeeper 提示，正式版会补齐签名和公证链
- Windows / Linux 当前同样按功能验证版来定位
- 稳定的应用内自动更新会放在已签名的正式通道中完善

## Roadmap

- P0：首次运行向导、Doctor 检查、更强的 monorepo 识别和一键修复动作
- P1：session 恢复、批量动作、更清晰的运行时状态、更强的日志与健康检查
- P1：更完整的 Docker / Compose 编排，并统一进同一个 Runtime 面板
- P2：项目 recipe、预览分享 tunnel、以及更可扩展的技术栈推断能力

已经落到应用里的部分：
- 首次运行向导，直接提示 Install / Env / Run / Open 的下一步
- Doctor 检查，覆盖本地工具、环境变量缺口、依赖安装状态、端口和路由可用性
- 面向 workspace 型 Node monorepo 的子应用识别

## 开发校验

```bash
npm run typecheck
npm test
npm run build
npm run tauri build -- --debug
```

## 贡献

欢迎提交 Issue 和 PR，尤其是仓库识别、动作推断、运行时管理和跨平台打包相关方向。

## License

MIT，见 [LICENSE](./LICENSE)。
