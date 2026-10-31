# 单层前置代理

界面依据：`docs/designs/rv-proxy-list-detail-chain-v1.png`（Imagegen 生成的已确认设计）。
使用原生 GPUI Component `ListItem`、`Form/Field`、`Select`、`Input`、`Switch`、
`Button`、`GroupBox` 和 `AlertDialog`。左侧为代理记录列表，右侧仅编辑所选代理。
不再有页面顶部的代理选择下拉框，不重复填写前置的连接信息。

## 配置模型

每条 `SocksProxy` 仍是独立出口记录，支持 HTTP 和 SOCKS5 协议；旧记录未提供协议时默认 SOCKS5。
代理详情的原生 Select「协议」选择与记录一并保存。可选 `front_proxy_id` 引用已有代理。
该字段与连接信息一并加密存储在现有 `socks_proxies.encrypted` 中；已有记录缺省为无前置。
保存使用 `BEGIN IMMEDIATE` 校验引用存在、非自身、无循环、最多一层。
已被其他代理作为前置的记录不能再添加前置；被引用的代理不能删除。
店铺继续绑定最终出口记录。

例如 B 为住宅 HTTP 代理，B 的前置选择 A（127.0.0.1:12334）：

```text
RV → RV 内置 meow 本地入口 → A → B → 目标网站
```

前置下拉只列出可用的已有记录及“不使用前置代理”。排他开关约束以当前记录为
最终出口的任务；记录作为前置时只承担传输，不另占该记录“作为出口”的租约。
原有共享/排他、任务 owner 和写操作不自动重放的规则继续保留。

## 内置 meow 引擎

`rv-proxy` 通过 GitHub 依赖锁定 `gitssie/meow-rs` 的
`317fa9df484264820368ffca559ed8fcc7188765`（`feature/optional-proxy-tls`）。
关闭 meow 默认协议与代理 TLS 功能，因此不链接 BoringSSL，保留 RV 原有 OpenSSL。
普通 SOCKS5 和 HTTP CONNECT 支持目标 HTTPS；目标 TLS 由调用方 rustls 处理。

普通单代理仍由 reqwest 连接配置的协议。链式线路由专用 Tokio 线程承载，
整个 RV 进程共用一个 loopback 端口，按随机本地认证键选择不可变完整线路。
不依赖 sing-box，不生成外部配置或子进程，也不修改用户已有 sing-box 实例。
旧程序路径字段保留在数据库，但不读取、不参与线路指纹，界面不再要求配置。

客户端缓存按 workspace、出口、真实 IDFV、请求/图片类型隔离，最多 32 项，
空闲 120 秒过期，每 5 秒清理。代理事件与完整线路指纹继续使旧缓存失效。
Route 和客户端持有线路；缓存淘汰不终止在途连接。最后持有者释放时移除认证键，
已建立的隧道继续使用原线路。正常退出关闭内置入口和自有连接。

Shared + PasswordSession 配置作为密码模板。不同店铺从模板产生不同的内存 Session，
同店铺业务、聊天和图片复用；模板中的其它认证参数不变，不写回 SQLite。
有租约的 Session 不回收，无租约空闲 120 秒后回收，程序重启后重新生成。
不同 Session 不等于供应商保证不同公网 IP；实际出口仍由 IP 检测/任务策略确认。
Exclusive 和显式 managed 换 IP 的门锁、持有者、确认与历史规则继续保留。

业务、聊天、图片和 IP 查询客户端统一使用完整线路，域名经 SOCKS5 传递；链路失败报错，
不跳过前置、不跳过出口、不直连目标。现有换 IP 提供商控制 URL 的独立控制请求方式保持不变。
IP 检测只展示查询结果及地区，不增加公网地址/IPv4 有效性判断。

## 验证

- 核心存储：前置复用、自引用/缺失引用/多层拒绝、被引用删除保护、已有记录缺省行为。
- 内置 meow + 本地模拟代理：HTTP/SOCKS5 四种前置/出口组合分别认证、远程 DNS、故障不直连；不跳过用例。
- 同进程 OpenSSL HTTPS：HTTP CONNECT 和 SOCKS5 两种本地入口、HTTP/SOCKS5 两种出口。
- Session：不同 IDFV 隔离、同店铺复用、不改模板、有租约不回收、闲置回收。
- 连接缓存：容量、空闲回收、配置事件失效、前置凭据变更拒绝旧路由。
- 显式 Google 实网探针：通过已保存完整链路，TLS 校验开启；google.com HTTP/2 301、Google 跳转目标 HTTP/2 200；不换 IP、不改配置。默认测试跳过，仅显式 `--ignored` 执行。
- GPUI 回归：ListItem 选择单条详情、前置引用保存不增加重复记录；按仓库约定仅类型检查，未构建或运行 rv-app。

## Exclusive proxy handoff between virtual Apps

Ordinary shop reads, scheduled work, chat, avatar uploads and standalone image
requests use the same async occupancy entry. An exclusive proxy remembers its
last business virtual App (real IDFV) across task/request lease releases for the
lifetime of the RV process. First use adopts the current exit; A→A reuses it;
A→B invokes the configured HTTP/password-session rotation and confirms the new
exit before B's business request. B→B reuses it; B→A rotates again. Each exit
tracks ownership separately. Shared proxies do not automatically rotate the durable record; password-session
templates use separate process-local Sessions per virtual App as described above.

Rotation holds the existing proxy gate; same-App concurrent entrants wait for
the handoff to finish instead of sending through stale credentials. The verified
client/full front→exit snapshot is retained, and cached pre-rotation HTTP sockets
are removed even when an HTTP control URL leaves the password unchanged. Images
inside an existing operation reenter that operation's captured route and never
start another handoff. Failed/cancelled preparation sends no next-App business
request and releases occupancy; the prior App remains the last business owner.
No task enabled/cron switch is changed. The configured rotation method is
required when an exclusive exit changes virtual Apps; there is no direct fallback.
