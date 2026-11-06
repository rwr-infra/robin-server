# MCP Server + JSON 服务器/玩家数据查询

- 日期：2026-11-04 → 2026-11-05
- 风险：中
- Commits：432c466..c70004a（含 4838b85）
- 相关 ADR：无

## 结果

rwrs-server 现在有两层只读 JSON 查询面：普通 HTTP `/api/servers.json`、`/api/players.json`，以及同进程独立端口的 MCP Streamable HTTP `/mcp`（`list_servers`、`get_server`、`search_players`、`list_maps`）。两者复用同一 XML/HTML 解析层、查询校验与 `ApiCache`；现有 HTML/XML 透传端点行为不变。

## 关键决策

| 决策 | 原因 | 否决方案及原因 |
| --- | --- | --- |
| 玩家数据 = 上游 `view_players.php` 战绩榜单 | 与 robin-web 玩家页一致；后端已有该数据源 | 本地 profile.xml 存档解析：后端无数据源，需上传入口，范围外 |
| MCP 用 `rmcp 3.8` + `axum 0.8` 独立监听端口 | Salvo 不是 tower 栈，不能直接挂 `StreamableHttpService`；独立监听最省适配 | 手写 JSON-RPC / 挂 Salvo 自定义适配：协议与 SSE 细节多，维护成本高 |
| 同时提供 JSON REST 与 MCP | 普通客户端可直接查询；MCP 工具复用同一 `RwrsService` | 只做 MCP：浏览器/脚本不可用，测试面也更难验证 |
| 响应字段 camelCase、query 参数 snake_case | 与前端 `IDisplayServerItem` / `IPlayerItem` 对齐；REST 参数保持常规 snake_case | 响应 snake_case：与前端模型漂移；query camelCase：与现有 API 不一致 |
| 服务器 `id` = `address:port`，玩家 `id` = `db:username` | 服务器下标不稳定；玩家格式与前端一致 | 服务器沿用数组下标：上游重排后 id 变化 |
| 过滤后分页；服务器最多 10 页 × 100 并按 id 去重 | 与前端 `listAll` 行为一致，限制上游放大 | 无限翻页：上游异常时无界 |
| `db` / `sort` 白名单，非法值在触达上游前拒绝 | 上游未知 sort 会返回空表，静默错误难以发现 | 透传给上游：调用方拿到空结果却不知道参数错 |
| 错误分类：InvalidQuery→400 / MCP -32602；Upstream→502 / MCP `isError` | 调用方需要区分“参数错”和“上游坏” | 统一 500：无法判断是否该重试 |
| MCP stateless + JSON 响应 | curl 与多客户端无需 session；工具本身只读 | legacy session：需要会话头管理，无收益 |
| `MCP_HOST` 默认 `127.0.0.1`，独立于 `HOST` | 避免既有 `HOST=0.0.0.0` 部署升级后静默暴露无鉴权 MCP | 继承 `HOST`：公网部署自动多开无鉴权端口 |
| loopback 默认 allowlist 追加实际绑定 host；非 loopback 无 allowlist 时关闭校验并 warn | 修复 127.0.0.2 等 loopback 绑定自拒绝；公网暴露需显式 opt-in | 始终用固定 3 项：非标准 loopback 地址全部 403 |
| `MCP_PORT` 默认 5801，0 关闭，与 `PORT` 相同则禁用并 error | 避免两个监听竞速导致主服务或 MCP 静默失败 | 允许同端口：启动结果不确定 |
| 复用 `ApiCache`，不新增第二套缓存 | 与现有透传端点共享 3 秒 TTL | 新建缓存：重复外呼与状态不一致 |

## 确立的规范

- 模块职责：`src/domain.rs` DTO/查询校验/错误类型；`src/parsers.rs` 上游 XML/HTML → DTO；`src/upstream.rs` `UpstreamClient`/`RwrsService`；`src/routes.rs` Salvo 路由与 handler；`src/mcp.rs` rmcp 工具与 axum 监听。
- JSON 契约：成功响应 camelCase；错误体只有 `{"error": "..."}` 一个键；JSON REST 仅 GET；未知 query 参数忽略（兼容 `_t` 等缓存穿透参数），已定义参数的非法值 400。
- MCP 契约：工具只读；成功 `structuredContent`，参数错 JSON-RPC `-32602`，上游/不存在 `isError: true` + `structuredContent.error`；不声明 outputSchema。
- 配置：`MCP_HOST`/`MCP_PORT`/`MCP_ALLOWED_HOSTS`；`MCP_HOST` 的 IPv6 可写 `::1` 或 `[::1]`，Config 统一去方括号，日志 URL 自动加方括号。
- 测试：数据层/路由用 wiremock + Salvo `TestClient`；MCP 用 reqwest 打临时端口 axum + 真实 rmcp 协议；不依赖公网、不占固定端口、每测试独立 `ApiCache`。
- 发布检查：`cargo fmt --all -- --check`、`cargo test --all-features`、`cargo clippy --all-features -- -D warnings`、`cargo build --release`。

## Pitfalls

- `#[tool_handler]` 不写 `name` 时 `serverInfo.name` 是 `rmcp`，不是 crate 名；必须显式 `name = "rwrs-server"`。
- rmcp 默认 Host allowlist 只有 `localhost/127.0.0.1/::1`；绑定 `127.0.0.2` 等地址时必须把绑定 host 加入 allowlist，否则全部 403。
- rmcp 对 IPv6 绑定不接受方括号；但 URL 必须带方括号。Config 去括号、URL 格式化补括号，两件事不能混。
- `MCP_HOST` 不能继承 `HOST`；否则 `HOST=0.0.0.0` 的既有部署升级即公网暴露无鉴权端点。
- `MCP_PORT` 不能等于 `PORT`；否则两个监听竞速，输家只留日志。
- Salvo `parse_queries` 的 bool 只认 `true`/`false`；`1`/`TRUE` 会 400。`ServerQuery`/`PlayerQuery` 不能加 `rename_all = "camelCase"`，否则 `online_only` 参数失效。
- 上游服务器 XML 同时存在 `<timeStamp>` 与 `<timestamp>`；优先 camelCase，回退小写。`<map_name>`/`<url>` 常为空元素，按空串处理。
- 玩家 HTML 固定 18 列；`<th>` 表头行必须跳过，数字缺失/`-` 为 null，用户名可能是 `<a>`，rankIcon 在 `<img src>`。`search` 存在时上游忽略 `start`。
- `ApiCache` 会缓存非 2xx 响应 3 秒，且没有 single-flight；并发冷请求会重复打上游（服务器列表最多 10 页/请求）。
- 玩家 `offset` 无上界，且进入上游 URL 与缓存键；枚举 offset 会绕过缓存。

## 剩余风险 / 后续

- MCP 无鉴权/无限流；默认 loopback，但显式 `MCP_HOST=0.0.0.0` 或反向代理配置错误会暴露只读数据与上游放大面。公网部署必须加认证/限流。
- 冷缓存并发无 single-flight，`/api/servers.json` 每次最多 10 个上游请求；高并发时可能放大上游压力。可后续给 `ApiCache` 加 in-flight 去重或整体响应缓存。
- `MCP_PORT=0` 与端口冲突只有 Config 单测 + 人工进程冒烟，没有进程级自动化测试。
- 未用真实 Claude Code/MCP 客户端验证；已用真实 rmcp 协议 + reqwest/curl 验证 initialize/tools/list/tools/call。
- Docker musl 镜像未实际构建；rmcp 3.8 MSRV 为 Rust 1.88，若 `messense/rust-musl-cross:x86_64-musl` 落后需升级构建镜像。
- `MCP_ALLOWED_HOSTS` 中无效 authority 会被 rmcp 过滤；若全部无效，所有请求 403，无启动校验。
- README 既有陈旧内容未修：`/api/ping` 实际为 `/ping`、`RATE_LIMIT_SECS` 无实现、`/api/maps` 文档写 `{"maps":[...]}` 实际为裸数组、Testing 小节描述旧目录。
- `CLAUDE.md` 仍写 salvo 0.76.2 / 单 main.rs；已在归档中建议更新。

## 后续更新（2026-11-06）

- `ApiCache` 增加 per-URL single-flight：并发冷请求只触发一次上游请求；新增并发测试。
- README 陈旧项已修：`/ping` 路径、删除未实现的 `RATE_LIMIT_SECS`、`/api/maps` 裸数组响应、`src/tests/` 测试结构与命令。
- Docker musl 镜像已实际构建成功；`messense/rust-musl-cross:x86_64-musl` 当前为 rustc 1.96.1，满足 rmcp 的 Rust 1.88 MSRV。
- K8s 部署改为单公网入口：Service 增加 5801，Traefik 增加 `/mcp` 路由；配置在 `k8s-remote-config` 的 `feature/robin-mcp-single-port` 分支。
- 仍未做：MCP 鉴权/限流、进程级 `MCP_PORT=0` 自动化测试、真实 MCP 客户端验证、推送与部署。

## 指针

- 数据层：`src/domain.rs`、`src/parsers.rs`、`src/upstream.rs`
- HTTP：`src/routes.rs`、`src/main.rs`
- MCP：`src/mcp.rs`
- 测试：`src/tests/json_data_tests.rs`、`src/tests/json_api_tests.rs`、`src/tests/mcp_tests.rs`、`src/tests/basic_tests.rs`
- 文档：`README.md`（JSON 端点、MCP Server、环境变量）
- 参考前端：`../robin-web/src/lib/share/{utils,player-utils}.ts`、`src/lib/models/{server,player}.model.ts`
- 上游：`http://rwr.runningwithrifles.com/rwr_server_list/get_server_list.php`、`http://rwr.runningwithrifles.com/rwr_stats/view_players.php`
