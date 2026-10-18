# 域名管理 v2 试用指南（Cloudflare）

目标：用一个测试子域名（下文用 `komodo-test.example.com`）走完 创建 → plan → apply → 验证 → 删除。

## 1. 创建权限收窄的 Cloudflare API token

Cloudflare 控制台 -> My Profile -> API Tokens -> Create Custom Token，最小权限：

| 范围 | 权限 |
|---|---|
| 区域 -> 区域 | 读取 |
| 区域 -> DNS | 编辑 |
| 帐户 -> Cloudflare Tunnel | 编辑 |

- 区域资源：包括 -> 特定区域 -> 只选目标 zone（如 `example.com`）。
- 帐户资源：只选隧道所在的账号。
- 只用 DNS、不用隧道时可不加「Cloudflare Tunnel」；发现时会提示缺少隧道权限，不影响保存。

## 2. 在 UI 创建 Domain 账号（需管理员）

Settings -> Domains -> New Account，不需要手写 JSON：

1. 填 Name（如 `cf-main`）、Kind 选 `cloudflare`、Token 粘贴上一步的 token（保存后不再显示）。
2. 点「Read from Cloudflare」（用表单里的 token 预览，此时不会保存任何东西）。
3. Zones：勾选允许使用的 zone；一个都不勾表示不限制，token 能访问的 zone 都可用。
4. Tunnels：勾选要启用的隧道，本地名称默认取 Cloudflare 上的隧道名，可修改；可选绑定它所在的 Komodo Server（用于按 Deployment/Stack 所在 Server 自动选隧道，只有一条隧道时可不选）。账号 ID、隧道 ID 自动填写。
5. 点 Create。读取失败或有警告（如缺少隧道权限）时，仍可保存并只使用 DNS。

之后编辑账号会自动用已保存的 token 重新读取；Token 留空表示不修改。非 Cloudflare 的 Kind（无结构化表单）才会显示折叠的「Advanced: params (JSON)」。

## 3. 在服务页面添加域名（最短流程）

1. 打开 Stack 或 Deployment 详情页，找到「Domains」区块，点 **Add domain**。
2. 填主机名前缀（如 `app`），在下拉里选 zone（只有一个时自动选中）。只允许字母、数字、连字符，不能有下划线（`home_komodo` 要写成 `home-komodo`）。
3. Stack 选 service；端口自动从已发布端口读取（只有一个时自动选中，读不到可手输）。
4. 看弹窗里的提示：有隧道会说明走隧道；没有隧道则解析到服务器地址，若是内网 IP 会提示「只能在局域网访问，想从外网访问需先给服务器开通隧道」。
5. 点 **Add**。一步完成：创建 Domain 并 apply。只有远端已有不归 Komodo 管的同名记录时才会弹出确认，列出将被覆盖的对象。

区块里会出现可点击的链接、接入方式（Tunnel / DNS）、状态（ready / drifted / error，附人话原因）和删除按钮。接入方式自动决定：服务所在 Server 在账号里绑定了隧道则走 Tunnel，否则走 DNS；需要强制时在弹窗「高级选项」里覆盖。

Tunnel 的规则是 `hostname -> http://localhost:<发布端口>`，默认假定 cloudflared 以 host 网络运行在该服务所在的服务器上。DNS 模式没有端口转发，链接是 `http://<hostname>:<端口>`。

手工方式（高级）：侧边栏 Domains -> New，在 Service 里选目标和端口，必要时展开 Advanced 填 DNS 记录 / 隧道规则，保存后在 Plan 区块 Preview plan -> Apply。

## 4. 验证

- Cloudflare 控制台：DNS 里出现 `komodo-test` 的 CNAME（comment 为 `komodo:<id>`），隧道的 Public Hostname 里出现对应规则。
- 命令行：`dig +short komodo-test.example.com`，以及 `curl -I https://komodo-test.example.com`（需本机 8080 有服务）。
- 再点一次 Preview plan，应全是 Noop。

## 5. 删除并确认远端已清理

1. 在服务页面 Domains 区块点删除（或在 Domain 页面删除该资源）（On Delete 默认 Cleanup，会先清远端；失败则资源不会被删除，需修复后重试）。
2. 在 Cloudflare 确认 CNAME 与隧道规则都已消失，且隧道里其它规则、catch-all 保持不变。

## 等价 CLI

先配置好 `km` 的 host / key / secret。账号管理只能在 UI 做，CLI 不接收 token。

```sh
km domain add stack my-stack app.example.com --service web --port 8080   # 一步完成，有冲突会询问；-y 直接覆盖
km domain add deployment my-app app.example.com                           # 端口从 Deployment 的 ports 读取
km list domains
km domain plan komodo-test            # 加 --json 输出原始 JSON
km domain status komodo-test          # 加 --json 输出原始 JSON
km domain apply komodo-test           # 先显示 plan，回车确认
km domain apply komodo-test --plan-id <plan_id> -y   # 只应用审阅过的 plan，免确认
km domain apply komodo-test --adopt-conflicts        # 有 Conflict 时接管已有记录
km execute apply-domain komodo-test   # 等价的通用执行命令
```

## 常见问题

- `AuthFailed` / `ZoneNotFound`：token 权限或授权范围不对，或 hostname 不在 `params.zones` 内。
- `TunnelAmbiguous`：显式填写 Tunnel 名。apply 超时后出现 Conflict：确认是自己的规则后用 `--adopt-conflicts`。
- 删除 Stack / Deployment 时，确认框会列出绑定的域名，可勾选「同时删除这些域名」；不勾选则 Domain 保留，状态显示「目标服务已不存在」。
- 「DNS 解析到内网 IP」：只能局域网访问。外网访问需要隧道（一键开通隧道将在后续版本提供）。
