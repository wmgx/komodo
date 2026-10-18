# 域名管理：自行实测与清理

这份步骤供你在自己的测试主机名上执行。开发环境没有修改真实 Cloudflare DNS、Tunnel 或部署服务；本地 HTTP fixture、MongoDB、cloudflared 离线校验不代表真实外部联调通过。

## 准备

1. 使用本分支构建的匹配版本 Core、Periphery、UI 和 `km`。按仓库原有部署方式更新测试实例；本功能不会安装 Traefik 或启动 cloudflared。
2. 选择一个未被使用的精确测试主机名，例如 `app.test.example.com`。不要使用业务主机名。Cloudflare zone 必须 active，记录所属 account/zone ID 必须正确。
3. 准备返回 HTTP 200 的可丢弃后端。Tunnel 路径需要已运行、远程管理、仅由本实例修改配置的专用 Tunnel，以及能访问后端的 cloudflared。Traefik 路径需要专用 Traefik、可访问的入口地址和本目录中的配置模板。
4. 将 Cloudflare API token 放入 Core 管理员配置的 `secrets`/secret-file，资源中只保存名称引用 `CLOUDFLARE_API_TOKEN`。cloudflared connector token 是另一种凭据，不要填到 DNS provider 的 secret 中，也不要提交 token。
5. 为测试 zone 授予 DNS 编辑及 zone 读取能力；Tunnel token 还需目标 account 的 Cloudflare Tunnel 配置读写能力（控制台可能显示 Edit，API 权限列表显示 Write）。验证橙云 Traefik 严格源站 TLS 还需要 Zone Settings Read。只选择所需 zone/account；DNS API token 的 zone 权限并不自动限定到某一条记录，因此同时将两个连接的 `allowed_hostname_suffixes` 限制到测试子域。当前权限名称以 [Cloudflare 权限列表](https://developers.cloudflare.com/fundamentals/api/reference/permissions/)和[远程 Tunnel API 指南](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/get-started/create-remote-tunnel-api/)为准。
6. Core 设置 `domain_management_enabled = true` 和持久、唯一的 `domain_instance_id`。默认仍为关闭。普通用户须拥有 Domain 的对应操作权限、目标 Deployment/Stack 的显式 `AttachDomain`、连接的显式 `Attach`；连接创建/修改需管理员。

## 先测 Tunnel 路径

复制并编辑 `provider.json`、`tunnel.json`、`domain.json`，不要直接使用占位值：

- provider：实际 account、zone ID；允许后缀改成 `test.example.com`。
- tunnel：实际 account、专用 tunnel ID；同样限制后缀；`allowed_networks` 填实际后端的完整 origin，例如 `http://app:8080`（不是 CIDR）。
- domain：hostname 改成测试名，ingress 改为 `tunnel-home`，DNS provider 保持对应连接名，`proxy_mode` 改成 `Proxied`，TTL 为 1。URL 必须能从 cloudflared 访问，不能含凭据或使用 localhost/metadata 地址。TLS profile 使用 tunnel 中的 `https`。

以下假设 `home` 是已配置好的 km profile：

```sh
km --profile home domain provider create --file provider.json --yes
km --profile home domain ingress create --file tunnel.json --yes
km --profile home domain provider validate cloudflare-home --format json
km --profile home domain ingress validate tunnel-home --format json
km --profile home domain plan --file domain.json --out create.plan.json
```

检查计划中的精确域名、入口、后端、DNS 目标及警告，确认无业务记录之后再执行：

```sh
km --profile home domain apply --plan create.plan.json --yes --format json
km --profile home domain status my-app --format json
km --profile home domain wait my-app --generation 1 --timeout 180s --format json
curl --fail --max-time 15 https://app.test.example.com/
```

`Accepted` 仅说明操作已接收。分别检查 TargetResolved、IngressReady、DnsConfigured、TlsReady、Reachable 和 observed_generation；Unknown 或超时不算通过，也不代表服务端已取消。橙云公开 DNS 返回 Cloudflare IP 是预期情况，仍应在 API/控制台检查记录实际 CNAME 目标。

复用同一个 `create.plan.json` 再 apply 应返回同一操作，而不是创建重复 DNS/route。保存该 plan 文件供超时恢复使用；不要编辑服务器计划的引用或 request ID。服务端会拒绝过期的新请求或已失效的计划。

## 再测 Traefik 或迁移

参考 `traefik.compose.yaml` 和 `periphery.config.toml`，先部署专用测试入口并启用 file-provider watch。Periphery 显式启用 `domain_enabled`；先创建 Ingress 得到 ID，再由管理员将该 ID 映射到专用绝对目录。目录必须持久、无符号链接、不允许组/其他用户写，且不混放手工配置。将配置目录整体挂载到 Traefik。

`ingress.json` 的 server、dns_target、Docker network 和 TLS resolver 必须符合实际部署。入口的 rawdata 观察地址只供受保护的管理网络读取；不配置或无法访问时，原子写文件成功也不会被报告成已加载。

单独创建新测试绑定可以使用 DnsOnly；若使用 Proxied + RequireHttps，需严格源站证书和 Cloudflare strict SSL 模式，否则 TLS 观察不能通过。Docker Deployment/StackService 目标要求入口和运行容器在同一主机的同一自定义网络。

迁移已有测试绑定时：

```sh
km --profile home domain migrate my-app --ingress traefik-home --out migrate.plan.json
km --profile home domain apply --plan migrate.plan.json --yes --wait --timeout 180s
km --profile home domain status my-app --format json
km --profile home domain finalize-migration my-app --yes --wait --timeout 180s
```

先读取迁移计划的 downtime 警告。新入口就绪且 drain 时间到达后才 finalize；旧对象在此之前应保留。不要通过直接修改连接 ID 绕过迁移。

## 检查保护与清理

仅在上述可丢弃对象上检查：无 AttachDomain 的用户应不能暴露目标；修改本地计划步骤不会改变服务端执行内容；人为修改已拥有 DNS 内容后，重新 plan/apply 应报告 Drifted 而非覆盖。先恢复已知配置再清理。共享 Tunnel 整份配置发生外部变化也会阻止写入；adopt 不会绕过该保护。

正常清理：

```sh
km --profile home domain remove my-app --out remove.plan.json
km --profile home domain apply --plan remove.plan.json --yes --wait --timeout 180s
km --profile home domain status my-app --format json
```

确认受管 DNS 已撤销、drain 后受管路由已删除、人工对象/其他规则仍在。清理成功后，在 UI 删除已清理的 Domain 记录，再删除未被引用的连接：

```sh
km --profile home domain ingress delete tunnel-home --yes
km --profile home domain provider delete cloudflare-home --yes
```

如还测试了 Traefik，另删其未引用的 Ingress。专用 Tunnel、cloudflared、Traefik 测试部署和测试后端需由你自行停止/删除；Komodo 删除连接不会删除整个 Tunnel/zone。最后撤销测试 token，按需关闭功能开关。清理失败时保留墓碑和凭据以便重试；`abandon` 是放弃管理并留下外部对象，不能当作清理成功。

仍需你真实验证：API token 权限、DNS 传播、Cloudflare 边缘证书、cloudflared 在线状态/网络、Traefik 实际加载/ACME、公开 HTTP/TLS 连通性，以及真实故障后的恢复。
