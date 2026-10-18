# 本次实现与验证记录

分支：`feat/cloudflare-domains`。基线：`780ac68b992094a9fccd5fffb760e0c84fd3c3d1`（v2.3.3）。仅本地开发；没有推送、上游 PR、合并或生产部署，也没有修改真实 DNS/Tunnel。

实现包括 Domain/DnsProvider/Ingress、AttachDomain/Attach 权限、服务端持久计划和幂等操作、generation/CAS、单写租约与 fencing、所有权及漂移保护、实际 Cloudflare DNS/Tunnel 驱动、Periphery 受限原子文件发布、迁移/清理/接管/放弃、CLI、UI、ResourceSync、生成类型和资源 Schema。功能默认关闭。

## 最终通过

| 检查 | 结果 |
| --- | --- |
| `cargo check --workspace` | 退出码 0；存在 2 条非致命 dead_code 告警 |
| `CARGO_PROFILE_TEST_DEBUG=0 cargo test --workspace` | 退出码 0；65 passed、0 failed、1 ignored；doctest 阶段完成 |
| CLI | 上述结果含 5 个单元测试和 10 个真实 `km` 进程 HTTP 契约测试 |
| Core | 上述结果含 28 个测试；包括策略、目标、ResourceSync，以及 4 个本地 Cloudflare HTTP fixture 测试 |
| 客户端/OpenAPI | 6 个模型/权限/兼容性测试和 1 个完整 OpenAPI 文档生成检查 |
| Periphery | 10 个正式测试；所有权、fencing、文件安全、中断状态恢复、回滚和 rawdata 观察 |
| 真实本地 MongoDB | 显式执行 `cargo test -p database --test domain_persistence -- --ignored`，1 passed；正常 workspace 中唯一被忽略的测试已单独验证 |
| TypeScript 类型生成 | `node client/core/ts/generate_types.mjs` 成功 |
| TS 客户端 | `yarn --cwd client/core/ts build` 成功 |
| 资源 Schema | `cargo xtask generate resource-schema --pretty --file=./ui/public/schema/resources.json` 成功，包含三类新资源 |
| UI | 带最新客户端和 Schema 的 `yarn --cwd ui build` 成功（TypeScript + Vite） |
| cloudflared 离线检查 | 官方 cloudflared 2026.10.1 对实际 merge_tunnel 生成配置执行 `ingress validate` 成功；新增、原有、catch-all 规则匹配均正确 |
| 格式及差异 | 修改 Rust 文件格式检查、`git diff --check` 通过 |

MongoDB 使用专用临时容器和随机测试数据库，覆盖 hostname/request_id 唯一性、并发 generation CAS、竞争租约、过期后 fencing 增长，以及旧持有者无法续租/释放新租约。测试容器已停止并自动删除。

Cloudflare HTTP fixture 运行真实 reqwest 客户端路径；测试 endpoint 注入仅存在于 `cfg(test)`。覆盖分页、精确主机名过滤、429 重试、403 脱敏、服务端写入成功但响应超时后的观察恢复/无重复创建、漂移拒写、Tunnel 完整配置和未知字段保留、无变化时避免 PUT。它不验证 Cloudflare 实际账号、权限或边缘行为。

Periphery 的中断恢复测试通过构造 rename 前、rename 后及 unlink 后的磁盘状态执行；不是实际断电/杀进程压力测试。CLI 超时 fixture 已修复客户端正常断开时的 BrokenPipe 竞态，完整请求断言保留。

## 失败尝试及处理

- `cargo test --workspace --all-features` 不适用本仓库：启用客户端 `blocking` 会使异步 CLI 的调用无法编译。最终使用正常 workspace 特性组合；Core 默认特性包含 OpenAPI，因此仍覆盖新增文档测试。
- 验证中发现并修复了 OpenAPI 类型别名导致 schema 覆盖、列表泛型宏兼容、整合重复 helper 和上述 CLI fixture 问题。最终正常 workspace 检查/测试无失败。
- 执行通道曾短暂断开，文件与构建产物保留；恢复后重新启动并完整执行最终 workspace 测试，退出码 0。

## 未运行及能力边界

没有真实 Cloudflare DNS/Tunnel、Traefik 加载/ACME、公网 HTTP/TLS、认证浏览器端到端或生产部署测试。没有进行真实断电、磁盘故障、多进程争锁压力测试；不能宣称外部 exactly-once 或生产验证完成。按用户决定，真实联调由用户执行，参见 [实测与清理步骤](README.zh-CN.md)。

当前 URL 规则是管理员精确 origin 白名单，不是通用 CIDR/持续 DNS 防重绑定防火墙。每个 adopt 计划仅处理一个明确 DNS record 或 Tunnel hostname；手工 Traefik 文件接管、多记录地址集合明确拒绝。共享 Tunnel 整份配置漂移需要恢复已知基线，不能用 adopt 绕过。Blocked 状态不自动覆盖，Retryable/Interrupted 使用同一计划和 request_id 恢复。

本环境主要证据日志：`/tmp/komodo-check-delivery.log`、`/tmp/komodo-tests-verified.log`、`/tmp/domain-mongo-test.log`、`/tmp/komodo-tsgen-final.log`、`/tmp/komodo-tsbuild-final.log`、`/tmp/komodo-schema-final.log`、`/tmp/komodo-ui-build-with-schema.log`。
