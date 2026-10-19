# 域名管理 v2 设计（第一期，与实现一致）

- 分支：`feat/domains-v2`（基于上游 2.3.3）
- 范围：把「一个公网域名如何到达服务」声明为正式资源 `Domain`：一条 DNS 记录 + 一条隧道入口规则。第一期厂商：Cloudflare DNS + Cloudflare Tunnel（远程管理隧道）。
- 用户试用步骤见 `doc/domains-v2-usage.md`。

## 1. 目标与取舍

1. Domain 与 Stack 等资源一致：侧边栏、搜索、权限、Tag、更新记录、ResourceSync（`[[domain]]`）、Procedure（`ApplyDomain`）。
2. 对 AI 友好：plan 与结果是结构化 JSON，错误带稳定 code。
3. 换厂商便宜：新增厂商 = 新增一个实现文件 + 注册表一行，wire 类型、数据库、UI 通用页面不动。
4. 对上游公共代码改动面小；业务逻辑全部在新文件。

相对旧分支 `feat/cloudflare-domains` 明确不做：Mongo 租约/fencing、Migrate/Adopt/Abandon 状态机、tombstone/drain、自动周期 reconcile（需要时用 Procedure 的 schedule 触发 `ApplyDomain`）、整条隧道配置指纹、密钥放 core `secrets`、Traefik（第二期）。

## 2. 数据模型

### 2.1 Domain 资源

`client/core/rs/src/entities/domain.rs`，`Resource<DomainConfig, DomainInfo>`，`ResourceTarget::Domain`。

```text
DomainConfig {
  hostname,                       // 小写保存
  target?, service, port,         // 绑定的 Deployment|Stack、Stack 的 service 名、发布端口（0=未知）
  dns:     { account, record_type(A|AAAA|CNAME, 默认 CNAME), content, ttl, proxied? },
  ingress: { account, tunnel, service },
  on_delete: Cleanup | Retain     // 默认 Cleanup
}
```

- `dns.account` 为空 = 不管理 DNS；`ingress.account` 为空 = 不走隧道（只管 DNS）。
- `dns.content` 为空时取隧道目标（`<tunnel-id>.cfargotunnel.com`），`proxied` 未设置时由厂商决定（指向隧道时为 proxied）。
- **zone 不写在 Domain 上**：按 hostname 对账号可见的 zone 做最长后缀匹配（账号 `params.zones` 可作白名单）。
- **隧道不写 tunnel_id**：`ingress.tunnel` 填账号里登记的隧道名；为空时按顶层 `target`（Deployment/Stack）所在 Server 自动选账号中绑定了该 Server 的隧道；未设 target 且账号只有一条隧道时用该隧道。多条匹配报 `TunnelAmbiguous`。
- `service` 形如 `http://host:port`。

`DomainInfo`（系统维护，ResourceSync 不比较）：`status`（Ready / Drifted{reasons} / Error{reason}，None=未检查）、`observed_at`、`owned: Vec<OwnedObject>`（Komodo 创建并认领的远端对象：layer、account、key、remote_id、scope、applied_digest）。

### 2.2 厂商账号 DomainAccount（不是资源）

- 存 Mongo `domain_accounts`，也可在 core config 的 `domain_accounts` 声明（只读，`source=Config`）。**token 不进 `secrets` 也不参与 `[[NAME]]` 插值**，避免有 Stack 写权限的用户通过插值读到。`CoreConfig::sanitized()` 对其脱敏。
- 仅管理员可读写；读取只返回 `token_set`。更新时 token 为空 = 不修改。
- 账号字段：`name`（全局唯一，Domain 按名引用）、`kind`、`token`、`params`、`description`。
- Cloudflare `params`：`zones`（可选白名单）、`tunnels: [{name, account_id, tunnel_id, server?}]`，`server` 为 Komodo Server 名或 id。
- 删除账号：只要有 Domain 的配置或 `info.owned` 仍引用它就拒绝。

### 2.3 Plan

`DomainPlan { domain, generated_at, plan_id, changes, errors }`。`Change { layer, object{kind: dns_record|tunnel_rule, key, remote_id}, action, before, after, reason }`。

- action：`Create | Update | Delete | Noop | Conflict`；reason：`Missing | ContentDiffers | ManualEdit | NotOwned | HostnameChanged | OrphanOwned`。
- `Conflict` = 远端存在同名但非 Komodo 创建的对象，apply 默认拒绝，`adopt_conflicts=true` 才接管。
- `errors: Vec<DomainReason{code, layer, object_key, message, retryable}>`，message 不含 token 与响应 body。
- 常见 code：`AccountNotFound AccountKindMismatch InvalidConfig ZoneNotFound TunnelNotFound TunnelAmbiguous AuthFailed RateLimited ProviderUnavailable ProviderRejected ProviderResponse Timeout RecordMissing RecordChanged RuleMissing RuleChanged ForeignRecordConflict PlanChanged`。
- `plan_id` 是 changes 的摘要；`ApplyDomain` 带上它可确保执行的就是审阅过的内容，不一致返回 `PlanChanged`。

## 3. 厂商抽象

`bin/core/src/domain/provider/`：`DnsProvider` / `IngressProvider` trait + `ProviderFactory{kind, capabilities, params_example, validate_params, build, discover?}`（`build` 返回的 `ProviderSet` 里 `dns` / `ingress` / `provision` 都是可选能力）（`discover` 为可选的只读发现能力，不支持的厂商返回明确错误） + 注册表。`cloudflare.rs` 同时提供两种能力，共用 `cloudflare_client.rs`（25s 超时、禁重定向、仅 GET 重试、不泄露响应体、无 Debug）。`tunnel_merge.rs` 负责「读整条隧道配置 → 合并单个 hostname 规则 → 写回」，保留未知字段与 catch-all。

新增厂商：新增 `provider/<kind>.rs` 并在注册表加一行；`ListDomainProviderKinds` 返回 kind、capabilities、params 示例，UI 与 AI 由此发现。

所有权：
- DNS 记录：Cloudflare 记录 `comment = "komodo:<domain id>"`。
- 隧道规则：无标记字段，靠 `info.owned` 里的 `applied_digest` 判断是否被手改。
- drift 只检查自己认领的对象，以及同 hostname 的非自有对象（报 Conflict）；隧道中其它 hostname 的规则、其它 DNS 记录对状态零影响。

## 4. 流程

- **plan**（`GetDomainPlan`，Read 权限）：解析配置 → 本地校验 → 按账号加进程内锁 → 读取远端 → 生成 changes → 尽力把状态缓存回 `info`。
- **apply**（`ApplyDomain{domain, plan_id?, adopt_conflicts}`，Execute 权限）：持锁重新生成 plan；`plan_id` 不符报 `PlanChanged`；有 Conflict 且未确认则拒绝且无写入；顺序为隧道规则先、DNS 后（删除相反）；每个对象成功后立即写入 `info.owned`；任一步失败即停止，无回滚，下次 plan 显示剩余差异；整体超时 120s；产生 `Update`，日志为 JSON 化的 plan 与已执行 changes。
- **删除**（`pre_delete`）：`on_delete=Cleanup` 时先清理 `info.owned`（先 DNS 后隧道），任一失败则 Domain 不被删除；`Retain` 只删 Komodo 记录。
- **并发**：按账号名的进程内互斥锁（单 Core 实例）。模块内不 panic、不退出进程，错误在 API 边界转成 `DomainReason`。
- **ResourceSync**：只同步期望配置，不触碰远端；需要生效时在 Procedure 里串 `RunSync` → `ApplyDomain`。引用不存在的账号不阻止保存，plan 时报 `AccountNotFound`。账号不进 TOML，需在每个实例手动创建。

## 5. 权限

- Read：查看配置、缓存状态、`GetDomainPlan`。注意 plan 会用账号凭据联网读取远端，只读用户也能触发。
- Execute：`ApplyDomain`。Write：修改配置。
- Create：仅管理员（创建 Domain 等于允许写公网 DNS）。
- Write 权限者可以改 `service` / `hostname` 并让有 Execute 权限者 apply，这是 Write 语义本身，授权时需知晓。
- DomainAccount 的任何读写：仅管理员。

## 6. API

| 类别 | 请求 |
|---|---|
| Read | `GetDomain` `ListServiceDomains{target}` `ListDomains`（含 status_code）`ListFullDomains` `GetDomainPlan{domain}` `ListDomainProviderKinds` `ListDomainAccounts` `GetDomainAccount` `GetServerTunnel{server}`（仅管理员，开通状态 + cloudflared 部署状态 + Cloudflare 侧连接状态） `DiscoverDomainAccount{account?\|(kind,token)?}`（仅管理员，只读，返回 `{zones, ingresses[{id,name,account_id?,status?}], warnings}`；token 不进日志/响应） |
| Write | `AddServiceDomain{target, service, hostname, port?, mode?, adopt_conflicts}` `CreateDomain` `CopyDomain` `UpdateDomain` `RenameDomain` `DeleteDomain` `CreateDomainAccount{name,kind,token,params,description}` `UpdateDomainAccount{id,token?,params?,description?}` `DeleteDomainAccount{id}` `EnableServerTunnel{server, account?, image?}` `DisableServerTunnel{server, delete_remote?}`（仅管理员，见 6.2） |
| Execute | `ApplyDomain{domain, plan_id?, adopt_conflicts}`（同时是 Procedure 的 Execution） |

### 6.1 以服务为中心：AddServiceDomain 与自动推导

`AddServiceDomain`（Write，仅管理员）一步完成：校验主机名 -> 解析目标 Server -> 自动推导配置 -> 在临时 Domain 上做 plan（只读）-> 创建 Domain（名称由 hostname 生成，冲突加 `-2` 后缀）-> apply。返回 `{domain?, mode?, conflicts, warnings, error?, reason?}`：

- plan 出现 `Conflict` 且 `adopt_conflicts=false`：不写入远端、不创建 Domain，`conflicts` 返回待确认对象，确认后带 `adopt_conflicts=true` 重试。
- 校验/推导失败返回 400 和人话提示（如下划线、未开通隧道）；apply 中途失败时 Domain 已创建，`reason` 带原因，状态为 Error，可在 Domain 页重试。

推导规则（`bin/core/src/domain/auto.rs`，厂商无关，只用 `IngressProvider::select` 与账号 zone）：

1. `mode=Auto|Tunnel` 且目标 Server 在某账号里绑定了隧道（`select(None, server)` 成功）：**Tunnel**。隧道规则 `hostname -> http://localhost:<port>`，**默认假定隧道 agent（cloudflared）以 host 网络运行在该 Server 上**；DNS 为 CNAME 到 `<tunnel_id>.cfargotunnel.com` 且 proxied。多条/多账号匹配报错，不猜。
2. 否则（`Auto|Dns`）：**DNS**，目标取 Server 的 `external_address`，为空则取 `address` 的 host。IP 用 A / AAAA，域名用 CNAME，不 proxied。私网（10/8、172.16/12、192.168/16、100.64/10、链路本地、IPv6 ULA）返回警告「只能在局域网访问」；回环地址报错。DNS 模式没有端口转发，访问地址是 `http://hostname:port`。
3. 都无法确定（没有隧道也没有地址、Swarm 目标、没有账号覆盖该 zone）：返回可操作提示。提示里引导从 Services 页面 Make public（会自动开通，见 6.3）。
4. DNS 账号 = 其 zone（`params.zones` 白名单，否则发现结果）覆盖主机名的账号，优先与隧道同账号；只有一个 DNS 账号且 zone 未知时直接用它。多个 DNS 账号时才会联网发现 zone。
5. 端口：请求未带 `port` 时，Deployment 取 `config.ports` 唯一的宿主端口；Stack 必须由调用方给出（UI 从 `ListStackServices` 的容器端口读取）。

主机名校验在后端统一做（`validate.rs`）：字母、数字、连字符，不允许下划线，不能以连字符开头/结尾，每段 <=63，总长 <=253，错误信息指出是哪一段、怎么改。

UI 的 Add domain 弹窗会在本地复刻「有无隧道 / 是否内网」做提示，真正决策以后端为准。

### 6.2 一键给服务器开通隧道（EnableServerTunnel）

目标：Add domain 时服务器没有隧道，点一下就能让这台服务器上的服务从外网访问。

**厂商抽象**：`ProvisionProvider`（`provider/provision.rs`，`ProviderSet.provision` 可选）。能力：`registered(server)`、`provision(name, image)`（创建隧道 + 初始化配置 + 取 connector token，失败则自动删掉刚创建的隧道）、`connector(handle, image)`（重试用，重新取 token）、`status`、`destroy`、`register / unregister / remember_image`（返回新的账号 params，未知字段保留）。返回的 `ConnectorSpec{image, command, env}` 只描述怎么跑 connector，由 `domain/tunnel.rs` 统一编排 Deployment，core 里没有 Cloudflare 专属逻辑；不支持的厂商得到「账号不支持一键开通隧道」。Cloudflare 实现在 `cloudflare_provision.rs`。

**Enable 流程**（持账号锁，管理员）：
1. 选账号：服务器已有隧道则用其账号；否则能开通隧道的账号只有一个时自动选用，多个时报错并列出（UI 显示选择框）。账号必须是数据库账号（配置文件账号只读，无法登记隧道）。Cloudflare account_id 不需要填：沿用已登记隧道的 account_id，否则 token 只能看到一个 Cloudflare 账号时自动取，多个时报错。
2. 已开通且 cloudflared 在运行/部署中：直接返回现状（幂等）。
3. 服务器状态不是 Ok 时拒绝（还没创建任何远端对象）。
4. 隧道已登记但 cloudflared 没运行（上次中途失败）：重新取 token，继续第 6 步。
5. 未登记：`POST cfd_tunnel {name: "komodo-<server>", config_src: "cloudflare"}` -> `PUT configurations`（只有 catch-all `http_status:404`）-> `GET token`；其中任一步失败都会 `DELETE` 刚创建的隧道。然后把 `{name, account_id, tunnel_id, server, managed: true}` 写进账号 `params.tunnels`，同时记下 `params.cloudflared_image`；写库失败同样回滚远端隧道。
6. 写入 secret Variable，创建（或修正镜像后复用）Deployment `cloudflared-<server>`：镜像可配置（默认 `cloudflare/cloudflared:latest`，镜像源如 `docker.1ms.run/cloudflare/cloudflared:latest`）、`network=host`、`command="tunnel --no-autoupdate run"`、`restart=unless-stopped`、环境变量 `TUNNEL_TOKEN=[[TUNNEL_TOKEN_<SERVER>]]`；通过 `resource::create` 与 `execute::inner_handler(Deploy)` 创建和部署，因此走正常的权限检查并留下 Update 记录。同名但不在这台服务器上的 Deployment 会被拒绝，不会接管。

**Disable 流程**：只允许 `managed` 隧道；仍有 Domain 使用（`ingress.tunnel` 指向它、`info.owned` 的 scope 命中、或 `tunnel` 为空且目标在这台服务器上）时拒绝；先删 Deployment（`resource::delete`，会销毁容器），删 secret Variable，再按选项（默认是）删远端隧道（先 `DELETE connections` 清理残留连接，再 `DELETE cfd_tunnel/{id}`，404 视为已删除），最后从 params 移除登记。远端删除失败时登记保留，可直接重试。

**状态**（`GetServerTunnel`）：`NotEnabled / Starting / Healthy / Degraded / Down / Inactive`。Cloudflare 侧状态来自 `GET cfd_tunnel/{id}`；部署中或 connector 已运行但尚未连上时为 `Starting`；读取失败时 `warning` 说明，不影响其它字段。

**connector token 的存放（取舍）**：
- 方案 A（采用）：token 存为 secret Variable `TUNNEL_TOKEN_<SERVER>`，Deployment 的 environment 只写 `TUNNEL_TOKEN=[[TUNNEL_TOKEN_<SERVER>]]`，部署时由 Core 插值。有 Read 权限的用户在 Deployment 配置里只看到引用；secret Variable 的值对非管理员不返回，Deploy 日志里会被替换成 `***`。Variable 直接写库，不走 `CreateVariable`，避免值进入 Update 日志。
- 方案 B（未采用）：每次部署前由 Core 临时注入、不落库。需要改上游 Deploy 流程，侵入大，且重启策略 `unless-stopped` 下容器环境里本来就有 token，收益有限。
- 残余风险：(1) 有 Variable 写权限或能创建 Deployment 并引用 `[[...]]` 的用户可以通过插值把 token 读出来（例如写到另一个 Deployment 的环境变量或日志）；(2) token 明文在 Mongo 中（与账号 token 同级）；(3) 目标服务器上的 `docker inspect` 能看到 token（任何 connector 部署方式都一样）；(4) token 一旦泄露，持有者可冒充该隧道的 connector，处置方式是在 Cloudflare 删除/重建隧道（Disable 再 Enable）。个人单管理员场景可接受；多人协作时请限制 Variable 与 Deployment 的写权限。
- 不写进 Deployment 的 `environment` 明文，也不进 Update 日志。

**不做**：多隧道负载均衡、Access 策略、本地配置文件模式。connector 假定以 host 网络和 `localhost:<端口>` 回源（与自动推导一致）。

### 6.3 以服务为中心：ListServices / ExposeService / UnexposeService（第 3 版）

用户只关心服务，不关心 Domain / DNS / Tunnel。上面的能力保留，在其上加一层：

- `ListServices { server? }`（读）：聚合所有 Stack service 与 Deployment（按用户 Read 权限过滤），每项含 `target`、`service`、`stack`、`server`、`state`、`ports`（宿主机端口 -> 容器端口，附 `lan_url = http://<server 地址的 host>:<宿主机端口>`）、`public`（来自 Domain：`url`、`status`）、`expose`（进行中的阶段）。数据来自 stack / deployment 状态缓存与 Domain 列表，不调用 Periphery 与厂商 API；server 地址的取法与 `auto.rs` 一致（`external_address` 优先，取 host 部分），回环地址不生成 LAN URL。
- `ExposeService { target, service?, prefix?, zone?, port?, adopt_conflicts? }`（写，管理员）：服务端一次完成：
  1. 解析服务与所在 Server；`prefix` 缺省为服务名，合法化为小写字母 / 数字 / `-`（其他字符变 `-`）；`zone` 缺省时取账号 `params.zones`，为空则只读发现，只有一个才自动选，多个则报错并列出。
  2. 端口缺省时：已发布端口里选最像 HTTP 的（80、8080、3000、9120、8000、8888、443、5000，否则第一个）；Deployment 回退到配置里的单个端口。
  3. `EnableServerTunnel` 同一逻辑（`tunnel::enable`）：已连通则立即返回，否则创建隧道并部署 cloudflared，阶段 `Connecting`。
  4. `AddServiceDomain` 同一逻辑（`add_service_domain`），强制 `Tunnel` 模式，阶段 `Configuring`。随后最多等 30 秒让连接变 healthy，不影响结果。
  5. 返回 `{ url, conflicts, error }`。有远端同名且不归 Komodo 管的记录时返回 `conflicts`，确认后带 `adopt_conflicts` 重试。
  请求是同步的（首次约 1 分钟）。进度记在 Core 内存里（按服务一份，同一服务同时只允许一次），`ListServices` 返回 `expose` 字段，UI 弹窗每 1.5 秒轮询它显示步骤；Core 重启则丢失进度，但已完成的步骤是幂等的，重试即可继续。
- `UnexposeService { target, service? }`（写，管理员）：删除该服务绑定的全部 Domain（走 Domain 删除，`on_delete = Cleanup` 清理远端），返回被删除的主机名。隧道本身保留。

**cloudflared 镜像自动兜底**（`tunnel::enable`）：候选镜像 = 首选（账号已记住的镜像，否则官方）+ 提供商的 `fallback_images()`（Cloudflare：官方 `cloudflare/cloudflared:latest`、`docker.1ms.run/cloudflare/cloudflared:latest`、`docker.m.daocloud.io/cloudflare/cloudflared:latest`，常量在 `cloudflare_provision.rs`），去重后按顺序尝试。每个候选：创建 / 更新 connector Deployment 的 image，执行 `Deploy`，轮询该 Update 直到 `Complete`（最长 5 分钟）。**判定失败**：Deploy 的 Update `success == false`（`docker run` 拉不到镜像时周边服务返回失败日志）或超时；这是最简单可靠的信号，其他原因导致的失败同样会换源重试，最终都失败时给出可操作的报错。成功后若镜像与账号记住的不同，写回账号 params 的 `cloudflared_image`，下次直接使用。

## 7. UI 与 CLI

- UI：`ui/src/resources/domain/`（`index` 注册、`config` 配置表单、`plan` Plan 预览与 Apply、`table`）；设置页「Domains」标签管理账号（`ui/src/pages/settings/domain-accounts/`，仅管理员可见）。Plan 需手动点击才会联网；有 Conflict 时必须勾选确认才会带 `adopt_conflicts=true`。
- 服务总览：侧边栏 **Services**（`ui/src/pages/services/`，一张表：Service / Server / State / Ports / LAN URL / Public URL / 操作，按 Server 过滤、搜索）。「Make public」弹窗与「Stop public access」在 `ui/src/resources/domain/make-public.tsx`，总览页与详情页共用。
- 详情页：Stack / Deployment 页的「Access」区块（`service-access.tsx`，经资源的 `Page` 插槽挂载）显示每个端口的 LAN 地址、公网地址和上述两个操作；Domain 资源链接、接入方式、状态原因收在「Details」折叠项里。删除服务的确认框（`resources/delete.tsx`）提示「This service is public」并可勾选「Also stop public access」，不勾选则 Domain 状态显示「Service no longer exists」（纯 UI 判断）。
- Server 页：「Public access」一行（`server-tunnel.tsx`）只显示连接状态与 Disable；开通在第一次 Make public 时自动完成，没有单独的开通步骤和镜像输入。
- 侧边栏不再有 Domains（`SIDEBAR_RESOURCES` 排除 `Domain`），入口在 Settings -> Domains 底部的「Advanced: Domains」链接；资源路由 `/domains` 保留给 AI、ResourceSync 与排查。界面文案统一为英文。
- UI 其余部分：`ui/src/resources/domain/`（`index` 注册、`config` 配置表单、`plan` Plan 预览与 Apply、`table`）；设置页「Domains」标签管理账号（`ui/src/pages/settings/domain-accounts/`，仅管理员可见）。Plan 需手动点击才会联网；有 Conflict 时必须勾选确认才会带 `adopt_conflicts=true`。
- CLI：`km service list [--server] [--json]`、`km service expose <deployment|stack/service> [prefix] [--zone] [--port] [-y]`、`km service unexpose <service> [-y]`；高级：`km domain tunnel enable|disable|status <server> [--account] [--image] [--keep-remote] [-y]`、`km domain add <stack|deployment> <name> <hostname> [--service] [--port] [--mode] [-y]`、`km list domains`、`km domain plan <d> [--json]`、`km domain apply <d> [--plan-id] [--adopt-conflicts] [-y]`、`km domain status <d> [--json]`、`km execute apply-domain`。不提供账号管理命令（token 不应出现在命令行参数）。

## 8. 已知限制

1. 每个 Domain 只有一条 DNS 记录和一条隧道规则；一个 hostname 要多条记录需多个 Domain 或手工管理。
2. 仅支持 Cloudflare 远程管理的命名隧道，一对一（一台服务器一条隧道，不做多隧道负载）；隧道通过 `DiscoverDomainAccount` 列出后在 UI 勾选，登记到账号 `params.tunnels`。
3. apply 超时取消后，隧道规则可能已写入但未记入 `info.owned`，下次 plan 会把它识别为 Conflict，需要 `--adopt-conflicts` 接管。
4. 账号仍被 Domain 配置或 `info.owned` 引用时无法删除；需先删除 Domain（`Cleanup` 清远端，或把 `on_delete` 改为 `Retain` 保留远端），再删账号。
5. token 以明文存于 Mongo（与上游 Git/镜像仓库 token 同等安全级别），不做静态加密。
6. 无后台自动检查，状态来自最近一次 plan/apply；列表页不联网。
7. 单 Core 实例假设（进程内锁）。

## 9. 第二期方向

Traefik 文件入口（经 Periphery 安全写文件）、第二个 DNS 厂商（不支持 comment 的厂商用 TXT 标记）、`BatchApplyDomain`、凭据静态加密、Domain 变更告警。
