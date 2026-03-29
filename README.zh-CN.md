[English](./README.md) | **简体中文**

<div align="center">
  <h1>PortPilot</h1>
  <p><strong>把本地 GitHub 项目的运行、停止、路由、日志和端口，收进一个桌面控制台。</strong></p>
  <p>PortPilot 想解决的不是“再开一个 localhost 标签页”，而是把导入仓库、补环境变量、启动脚本、统一访问地址、查看日志这些零散动作整合成一个稳定工作流。</p>

  <p>
    <img src="https://img.shields.io/github/v/release/Horace-Maxwell/portpilot?include_prereleases&display_name=tag&style=for-the-badge" alt="Release" />
    <img src="https://img.shields.io/github/license/Horace-Maxwell/portpilot?style=for-the-badge" alt="License" />
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-0f766e?style=for-the-badge" alt="Platforms" />
    <img src="https://img.shields.io/badge/Tauri-2.x-24C8DB?style=for-the-badge&logo=tauri" alt="Tauri" />
    <img src="https://img.shields.io/badge/Rust-core-black?style=for-the-badge&logo=rust" alt="Rust" />
    <img src="https://img.shields.io/badge/TypeScript-UI-3178C6?style=for-the-badge&logo=typescript&logoColor=white" alt="TypeScript" />
  </p>
</div>

![PortPilot hero](./docs/media/hero-banner.svg)

## PortPilot 解决什么问题

很多本地网页项目直到今天还是这样运行的：

- 先 `git clone`
- 再看 README 猜怎么装依赖
- 手动复制 `.env.example`
- 自己找没被占用的端口
- 打开浏览器访问 `localhost:xxxx`
- 再回另一个终端看日志
- 最后忘了哪个命令能把它停掉

PortPilot 想把这条链路变成一个桌面产品：

- 支持 GitHub URL 导入和本地项目注册
- 自动识别 install / run / build / deploy / package 动作
- 解析 `.env.example` 并生成可编辑环境变量表单
- 给项目分配统一 `.localhost` 访问地址
- 在一个界面里看日志、端口、运行状态和路由
- 作为可发布的 macOS / Windows / Linux 桌面应用持续演进

## 真实场景

PortPilot 的目标就是让这类仓库更容易被一键接管：

| 仓库 | PortPilot 会做什么 |
| --- | --- |
| [`calesthio/Crucix`](https://github.com/calesthio/Crucix) | 识别 `npm install`、`npm run dev`、环境变量模板、compose 动作和统一访问地址 |
| [`koala73/worldmonitor`](https://github.com/koala73/worldmonitor) | 识别 web 运行入口、`desktop:dev`、`build:*`、`desktop:build:*` 和本地运行态 |

## 产品预览

### 一个总览页接管多个项目

![Dashboard preview](./docs/media/dashboard-preview.svg)

### 项目页直接围绕动作设计

![Actions preview](./docs/media/actions-preview.svg)

### 路由、日志、端口放在一起看

![Observability preview](./docs/media/observability-preview.svg)

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

首个公开版本会以 GitHub pre-release 的形式发布。

- 进入 [Releases](https://github.com/Horace-Maxwell/portpilot/releases)
- 下载你平台对应的安装包
- macOS Beta 首次打开时，如果系统拦截，需要去系统设置里手动允许，因为这一版还不是已公证安装包

### 计划中的发布资产

- macOS: `.dmg`, `.zip`, updater `.tar.gz`
- Windows x64: `.msi`, 可选 portable `.zip`
- Linux x64: `.AppImage`, `.deb`, `.rpm`

## Beta 说明

- `v0.1.0-beta.1` 是第一个公开验证版本
- macOS Beta 可能会先遇到 Gatekeeper 提示，正式版会补齐签名和公证链
- Windows / Linux 当前同样按功能验证版来定位
- 稳定的应用内自动更新会放在已签名的正式通道中完善

## Roadmap

- 更强的 monorepo 与多入口项目识别
- 更完整的 Docker / Compose 编排体验
- 完整的 macOS 签名与公证发布链
- 更顺滑的多平台自动更新体验

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
