# PortPilot 竞品研究

PortPilot 最核心的目标应该是：

> 把一个刚 clone 下来的仓库，尽快变成一个可运行、可观察、可恢复、可访问的本地应用。

这份研究关注的不是“别人做了哪些功能”，而是“别人帮用户省掉了哪些步骤”，以及这些能力怎么转化成 PortPilot 的产品优先级。

## 参考方向

### Portainer

- Portainer 把 stacks、环境变量、secrets、策略和服务级控制都放进同一套 UI。
- 对 PortPilot 的启发：只要仓库里有 Compose 或容器，导入后就不能让它继续像黑盒。
- 参考：
  - [Portainer docs: environment variables, ConfigMaps, secrets](https://docs.portainer.io/2.27/user/kubernetes/applications/add)
  - [Portainer docs: Docker security policies](https://docs.portainer.io/admin/environments/policies/docker-policies/security-policy)

### Homepage

- Homepage 强项是把多服务的状态和关键入口汇总到一个总览页。
- 对 PortPilot 的启发：项目状态、路由状态和关键信息应该在总览里就看得到，而不是层层点进去。
- 参考：
  - [Homepage 首页](https://gethomepage.dev/index.html)
  - [Homepage Docker labels 配置](https://gethomepage.dev/configs/docker/)

### Homarr

- Homarr 强调 integration 和模块化 widgets。
- 对 PortPilot 的启发：项目不应该只是列表项，也应该能像 dashboard 模块一样被组织和观察。
- 参考：
  - [Homarr docs: after the installation](https://homarr.dev/docs/getting-started/after-the-installation)

### DevPod

- DevPod 是 client-only、provider-independent，还能根据项目做最佳努力的环境分析。
- 它也支持 prebuild，让环境更快进入可用状态。
- 对 PortPilot 的启发：推断不能只看 manifest，还应该重用项目已有配置并支持可复用 setup。
- 参考：
  - [DevPod engine overview](https://devpod.sh/engine/)
  - [DevPod docs: what are machines](https://devpod.sh/docs/managing-machines/what-are-machines)
  - [DevPod docs: prebuild a workspace](https://devpod.sh/docs/developing-in-workspaces/prebuild-a-workspace)

### GitHub Codespaces

- Codespaces prebuilds 证明了：如果 setup 可以缓存和描述，启动体验会完全不同。
- 对 PortPilot 的启发：应该尽可能复用 repo 元数据，并加入 setup 缓存 / session restore。
- 参考：
  - [GitHub docs: Codespaces prebuilds](https://docs.github.com/en/codespaces/prebuilding-your-codespaces/configuring-prebuilds)

### DDEV

- DDEV 的价值之一是本地路由、hooks、项目启动体验都非常顺。
- 对 PortPilot 的启发：route 和 boot flow 必须尽量自动、可靠、可重复。
- 参考：
  - [DDEV docs: hooks](https://ddev.readthedocs.io/en/stable/users/configuration/hooks/)

### ServBay

- ServBay 强调多技术栈和数据库服务的本地开发体验。
- 对 PortPilot 的启发：PortPilot 可以成为多运行时、多服务项目的上层控制台。
- 参考：
  - [ServBay installation guide](https://support.servbay.com/getting-started/installation)

### Local / LocalWP

- Local 把 blueprints 和 live links 这种“复制 setup / 分享预览”的能力做得很有代表性。
- 对 PortPilot 的启发：recipe 导出导入、预览分享都是真正省事的能力。
- 参考：
  - [Local blueprints community thread](https://community.localwp.com/t/local-environment-blueprints/12189)

## PortPilot 应该优先抄什么

## 必须做

- `Doctor` 检查
- 导入后向导式首次运行
- 从 README、版本文件、devcontainer 等信号做更强推断
- monorepo 子目标选择
- Dashboard 里的 route / health 可见性
- Compose 和本地运行时统一可视化

## 应该做

- workspace session 保存与恢复
- 多项目批量动作
- 更强的日志切片和错误高亮
- 可复用的 project recipe
- repo 级 override，用来固定端口、命令和健康检查

## 可以做

- 预览分享 tunnel
- 常见技术栈模板
- 可插拔推断器
- 可选的远程 / devcontainer aware 流程

## 为什么现在这个优先级合理

- PortPilot 当前成败首先取决于导入后能不能跑起来，而不是高级协作功能。
- 第一个 stable release 目前只被 Apple notarization 卡住，不是 Windows/Linux 侧结构性问题。
- 首次运行成功率和运行时可见性，会同时改善三平台体验。
- recipe、tunnel、扩展能力都很有价值，但应该放在 first-run reliability 之后。

## 推荐落地顺序

1. 发出 stable `v0.1.0`
2. 上 `DoctorCheck`、首次运行向导、更强推断
3. 上 `WorkspaceSession`、批量动作、健康阶段
4. 打通 Docker Compose 与本地运行时统一视图
5. 加入 `ProjectRecipe` 和分享/tunnel 的基础设施
