# 让服务从公网访问：3 步

你只需要关心服务。Komodo 会自己决定怎么连通（隧道、DNS 记录、cloudflared 都是自动的）。

## 1. 建 Cloudflare token，在 Settings 里添加账号

Cloudflare 控制台 -> My Profile -> API Tokens -> Create Custom Token，最小权限：

| 范围 | 权限 |
|---|---|
| 区域 -> 区域 | 读取 |
| 区域 -> DNS | 编辑 |
| 帐户 -> Cloudflare Tunnel | 编辑 |

区域资源只选要用的域名，帐户资源只选你的账号。

然后在 Komodo：Settings -> Domains -> New Account（需管理员）：填 Name、Kind 选 `cloudflare`、粘贴 token，点「Read from Cloudflare」确认能读到域名，点 Create。隧道不用勾选，之后会自动创建。

## 2. 打开 Services

侧边栏 **Services**：一张表列出所有服务器上的 Stack 服务和 Deployment，包括状态、已发布端口、局域网地址（LAN URL，可点）和公网地址（Public URL，可点）。可按 Server 过滤、搜索。

Stack / Deployment 详情页的 **Access** 区块显示同样的信息。

## 3. 点 Make public

1. 点该行的 **Make public**。
2. 弹窗只有一个输入：子域名，默认是服务名（如 `grafana`），后缀（如 `.example.com`）自动带出。有多个域名时才出现下拉；有多个端口时才出现端口下拉，默认选最像网页的端口（80、8080、3000、9120 等）。
3. 点确定。Komodo 依次：开通这台服务器的连接（只在该服务器第一次时做，约 1 分钟）-> 配置地址 -> 完成，并显示可点击的 `https://...` 链接。

取消公网访问：同一位置点 **Stop public access**。删除服务时，确认框里勾选「Also stop public access」会一并清理。

只有远端已存在同名、且不归 Komodo 管理的记录时才会弹出确认。

### 服务器连不上 Docker Hub？

不用管。拉取 cloudflared 失败时，Komodo 会自动依次换 `docker.1ms.run`、`docker.m.daocloud.io` 镜像源重试，成功后记住可用的镜像，下次直接使用。

## 命令行（km）

```sh
km service list                      # 加 --server <名称>、--json
km service expose grafana            # Deployment；Stack 服务写成 my-stack/web
km service expose my-stack/web app   # 指定子域名；--port 指定端口，--zone 指定域名，-y 免确认覆盖
km service unexpose grafana
```

高级：`km domain ...`（plan / apply / status / tunnel）仍可用，侧边栏不再有 Domains，入口在 Settings -> Domains 底部的「Advanced: Domains」链接，路由 `/domains` 保留。

## 常见问题

- **Make public 提示没有可用域名**：Settings -> Domains 还没有账号，或 token 看不到任何域名。
- **提示没有已发布端口**：服务需要用 `ports` 发布一个宿主机端口（`8080:80`）。公网转发的目标是 `http://localhost:<宿主机端口>`。
- **Server 页面「Public access: ...」一直不是 connected**：看 `cloudflared-<服务器名>` 这个 Deployment 的日志，多半是服务器出站访问不了 Cloudflare。再点一次 Make public 会继续未完成的步骤，已创建的隧道不会重复创建。
- **关闭某台服务器的公网连接**：Server 页面「Public access」一行点 Disable（需要先对该服务器上的服务全部 Stop public access）。
- 状态 `drifted` / `error`：Access 区块的 Details 里有原因；多数情况再点一次 Make public 即可修复。
