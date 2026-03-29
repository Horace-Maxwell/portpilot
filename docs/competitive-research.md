# PortPilot Competitive Research

PortPilot should optimize for one core outcome:

> turn a cloned repository into a visible, routable, recoverable local app with the fewest manual steps possible.

This research focuses on what other products remove from the user workflow and how those ideas map back to PortPilot.

## Key references

### Portainer

- Portainer surfaces stacks, environment variables, secrets, policies, and service-level control in one UI.
- Relevant PortPilot takeaway: Compose and container-heavy projects should not feel opaque once imported.
- References:
  - [Portainer docs: environment variables, ConfigMaps, secrets](https://docs.portainer.io/2.27/user/kubernetes/applications/add)
  - [Portainer docs: Docker security policies](https://docs.portainer.io/admin/environments/policies/docker-policies/security-policy)

### Homepage

- Homepage uses service discovery and rich widgets to make many services visible from one dashboard.
- Relevant PortPilot takeaway: runtime status, route status, and useful metadata should be visible without drilling in.
- References:
  - [Homepage home](https://gethomepage.dev/index.html)
  - [Homepage Docker configuration via labels](https://gethomepage.dev/configs/docker/)

### Homarr

- Homarr emphasizes integrations and modular dashboard widgets.
- Relevant PortPilot takeaway: PortPilot should treat imported projects as composable modules on a board, not only as rows in a list.
- Reference:
  - [Homarr docs: integrations after installation](https://homarr.dev/docs/getting-started/after-the-installation)

### DevPod

- DevPod is client-only, provider-independent, and can analyze projects to create best-effort environments.
- It also supports prebuilds so environments start faster.
- Relevant PortPilot takeaway: inference should go beyond manifests, and reusable setup artifacts should be first-class.
- References:
  - [DevPod engine overview](https://devpod.sh/engine/)
  - [DevPod docs: what are machines](https://devpod.sh/docs/managing-machines/what-are-machines)
  - [DevPod docs: prebuild a workspace](https://devpod.sh/docs/developing-in-workspaces/prebuild-a-workspace)

### GitHub Codespaces

- Codespaces prebuilds show how much startup friction can be removed when setup is cached and described in configuration.
- Relevant PortPilot takeaway: reuse repo metadata and add setup caching/session restore where possible.
- Reference:
  - [GitHub docs: configuring prebuilds for Codespaces](https://docs.github.com/en/codespaces/prebuilding-your-codespaces/configuring-prebuilds)

### DDEV

- DDEV demonstrates the value of opinionated local routing, repeatable hooks, and a smooth local domain experience.
- Relevant PortPilot takeaway: route setup and project boot flow should feel automatic and dependable.
- Reference:
  - [DDEV docs: hooks](https://ddev.readthedocs.io/en/stable/users/configuration/hooks/)

### ServBay

- ServBay positions itself around multi-stack local development with databases and service tooling bundled together.
- Relevant PortPilot takeaway: PortPilot can become the orchestration layer even when the underlying runtimes are heterogeneous.
- Reference:
  - [ServBay installation guide](https://support.servbay.com/getting-started/installation)

### Local / LocalWP

- Local popularized ideas like reusable blueprints and live links for sharing previews.
- Relevant PortPilot takeaway: recipe export/import and preview sharing are meaningful productivity features.
- Reference:
  - [Local blueprints community thread](https://community.localwp.com/t/local-environment-blueprints/12189)

## What PortPilot should copy

## Must do

- `Doctor` checks before first run
- guided import flow instead of one large confirmation step
- stronger detection from README, version files, and devcontainer metadata
- monorepo target selection
- route and health visibility on the dashboard
- better Compose/runtime visibility

## Should do

- saved workspace sessions
- batch actions for multiple imported repos
- richer log slicing and error highlighting
- reusable project recipes
- first-class repo overrides for ports, commands, and health checks

## Could do

- preview sharing tunnels
- stack templates for common frameworks
- plugin-style inference providers
- optional remote/devcontainer-aware flows

## Why the current priority order makes sense

- PortPilot still wins or loses on import success, not on advanced collaboration.
- The first public stable release is blocked only by Apple notarization, not by product structure on Windows/Linux.
- Better first-run automation and runtime visibility improve all three platforms.
- Recipe export, tunnels, and extensibility are high value, but they should not come before first-run reliability.

## Recommended implementation order

1. Ship stable `v0.1.0`
2. Add `DoctorCheck`, guided first run, and stronger detection
3. Add `WorkspaceSession`, batch actions, and health phases
4. Unify Docker Compose and local runtime surfaces
5. Add `ProjectRecipe` and sharing/tunnel groundwork
