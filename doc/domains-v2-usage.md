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

## 3. 创建 Domain 并 plan / apply

1. 侧边栏 Domains -> New，名称 `komodo-test`。
2. Hostname 填 `komodo-test.example.com`。
3. DNS：Account 选 `cf-main`（类型默认 CNAME，内容留空，自动指向隧道）。
4. Ingress：Account 选 `cf-main`，Tunnel 选 `home`（或选 Target 让它自动选），Service 填 `http://localhost:8080`。保存。
5. Plan 区块点 Preview plan：应看到 2 条 Create（dns_record、tunnel_rule），errors 为空。
6. 点 Apply 并确认。若 plan 里有 Conflict（同名记录不是 Komodo 创建的），需勾选确认后才会接管。
7. 状态变为 `ready`。

## 4. 验证

- Cloudflare 控制台：DNS 里出现 `komodo-test` 的 CNAME（comment 为 `komodo:<id>`），隧道的 Public Hostname 里出现对应规则。
- 命令行：`dig +short komodo-test.example.com`，以及 `curl -I https://komodo-test.example.com`（需本机 8080 有服务）。
- 再点一次 Preview plan，应全是 Noop。

## 5. 删除并确认远端已清理

1. 在 Domain 页面删除该资源（On Delete 默认 Cleanup，会先清远端；失败则资源不会被删除，需修复后重试）。
2. 在 Cloudflare 确认 CNAME 与隧道规则都已消失，且隧道里其它规则、catch-all 保持不变。

## 等价 CLI

先配置好 `km` 的 host / key / secret。账号管理只能在 UI 做，CLI 不接收 token。

```sh
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
