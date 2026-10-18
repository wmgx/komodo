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

`bin/core/src/domain/provider/`：`DnsProvider` / `IngressProvider` trait + `ProviderFactory{kind, capabilities, params_example, validate_params, build, discover?}`（`discover` 为可选的只读发现能力，不支持的厂商返回明确错误） + 注册表。`cloudflare.rs` 同时提供两种能力，共用 `cloudflare_client.rs`（25s 超时、禁重定向、仅 GET 重试、不泄露响应体、无 Debug）。`tunnel_merge.rs` 负责「读整条隧道配置 → 合并单个 hostname 规则 → 写回」，保留未知字段与 catch-all。

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
| Read | `GetDomain` `ListServiceDomains{target}` `ListDomains`（含 status_code）`ListFullDomains` `GetDomainPlan{domain}` `ListDomainProviderKinds` `ListDomainAccounts` `GetDomainAccount` `DiscoverDomainAccount{account?\|(kind,token)?}`（仅管理员，只读，返回 `{zones, ingresses[{id,name,account_id?,status?}], warnings}`；token 不进日志/响应） |
| Write | `AddServiceDomain{target, service, hostname, port?, mode?, adopt_conflicts}` `CreateDomain` `CopyDomain` `UpdateDomain` `RenameDomain` `DeleteDomain` `CreateDomainAccount{name,kind,token,params,description}` `UpdateDomainAccount{id,token?,params?,description?}` `DeleteDomainAccount{id}` |
| Execute | `ApplyDomain{domain, plan_id?, adopt_conflicts}`（同时是 Procedure 的 Execution） |

### 6.1 以服务为中心：AddServiceDomain 与自动推导

`AddServiceDomain`（Write，仅管理员）一步完成：校验主机名 -> 解析目标 Server -> 自动推导配置 -> 在临时 Domain 上做 plan（只读）-> 创建 Domain（名称由 hostname 生成，冲突加 `-2` 后缀）-> apply。返回 `{domain?, mode?, conflicts, warnings, error?, reason?}`：

- plan 出现 `Conflict` 且 `adopt_conflicts=false`：不写入远端、不创建 Domain，`conflicts` 返回待确认对象，确认后带 `adopt_conflicts=true` 重试。
- 校验/推导失败返回 400 和人话提示（如下划线、未开通隧道）；apply 中途失败时 Domain 已创建，`reason` 带原因，状态为 Error，可在 Domain 页重试。

推导规则（`bin/core/src/domain/auto.rs`，厂商无关，只用 `IngressProvider::select` 与账号 zone）：

1. `mode=Auto|Tunnel` 且目标 Server 在某账号里绑定了隧道（`select(None, server)` 成功）：**Tunnel**。隧道规则 `hostname -> http://localhost:<port>`，**默认假定隧道 agent（cloudflared）以 host 网络运行在该 Server 上**；DNS 为 CNAME 到 `<tunnel_id>.cfargotunnel.com` 且 proxied。多条/多账号匹配报错，不猜。
2. 否则（`Auto|Dns`）：**DNS**，目标取 Server 的 `external_address`，为空则取 `address` 的 host。IP 用 A / AAAA，域名用 CNAME，不 proxied。私网（10/8、172.16/12、192.168/16、100.64/10、链路本地、IPv6 ULA）返回警告「只能在局域网访问」；回环地址报错。DNS 模式没有端口转发，访问地址是 `http://hostname:port`。
3. 都无法确定（没有隧道也没有地址、Swarm 目标、没有账号覆盖该 zone）：返回可操作提示。一键开通隧道将在下一批实现，目前提示文案只引导「先开通隧道」。
4. DNS 账号 = 其 zone（`params.zones` 白名单，否则发现结果）覆盖主机名的账号，优先与隧道同账号；只有一个 DNS 账号且 zone 未知时直接用它。多个 DNS 账号时才会联网发现 zone。
5. 端口：请求未带 `port` 时，Deployment 取 `config.ports` 唯一的宿主端口；Stack 必须由调用方给出（UI 从 `ListStackServices` 的容器端口读取）。

主机名校验在后端统一做（`validate.rs`）：字母、数字、连字符，不允许下划线，不能以连字符开头/结尾，每段 <=63，总长 <=253，错误信息指出是哪一段、怎么改。

UI 的 Add domain 弹窗会在本地复刻「有无隧道 / 是否内网」做提示，真正决策以后端为准。

## 7. UI 与 CLI

- UI：`ui/src/resources/domain/`（`index` 注册、`config` 配置表单、`plan` Plan 预览与 Apply、`table`）；设置页「Domains」标签管理账号（`ui/src/pages/settings/domain-accounts/`，仅管理员可见）。Plan 需手动点击才会联网；有 Conflict 时必须勾选确认才会带 `adopt_conflicts=true`。
- 服务页：Stack / Deployment 详情页的「Domains」区块（`ui/src/resources/domain/service-domains.tsx`，经资源的 `Page` 插槽挂载）；删除服务的确认框（`resources/delete.tsx`）会列出绑定域名并可勾选同时删除，不勾选则 Domain 状态显示「目标服务已不存在」（纯 UI 判断）。Domains 列表有 Service 列可跳转。
- CLI：`km domain add <stack|deployment> <name> <hostname> [--service] [--port] [--mode] [-y]`、`km list domains`、`km domain plan <d> [--json]`、`km domain apply <d> [--plan-id] [--adopt-conflicts] [-y]`、`km domain status <d> [--json]`、`km execute apply-domain`。不提供账号管理命令（token 不应出现在命令行参数）。

## 8. 已知限制

1. 每个 Domain 只有一条 DNS 记录和一条隧道规则；一个 hostname 要多条记录需多个 Domain 或手工管理。
2. 仅支持 Cloudflare 远程管理的命名隧道；隧道通过 `DiscoverDomainAccount` 列出后在 UI 勾选，登记到账号 `params.tunnels`。
3. apply 超时取消后，隧道规则可能已写入但未记入 `info.owned`，下次 plan 会把它识别为 Conflict，需要 `--adopt-conflicts` 接管。
4. 账号仍被 Domain 配置或 `info.owned` 引用时无法删除；需先删除 Domain（`Cleanup` 清远端，或把 `on_delete` 改为 `Retain` 保留远端），再删账号。
5. token 以明文存于 Mongo（与上游 Git/镜像仓库 token 同等安全级别），不做静态加密。
6. 无后台自动检查，状态来自最近一次 plan/apply；列表页不联网。
7. 单 Core 实例假设（进程内锁）。

## 9. 第二期方向

Traefik 文件入口（经 Periphery 安全写文件）、第二个 DNS 厂商（不支持 comment 的厂商用 TXT 标记）、`BatchApplyDomain`、凭据静态加密、Domain 变更告警。
