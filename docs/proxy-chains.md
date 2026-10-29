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
RV → RV 自建的 sing-box 本地入口 → A → B → 目标网站
```

前置下拉只列出可用的已有记录及“不使用前置代理”。排他开关约束以当前记录为
最终出口的任务；记录作为前置时只承担传输，不另占该记录“作为出口”的租约。
原有共享/排他、任务 owner 和写操作不自动重放的规则继续保留。

## 外部 sing-box

不打包、不下载安装 sing-box，不引入 Go SDK。代理页异步检测版本，齿轮按钮
用原生 AlertDialog 配置程序路径；留空时搜索常见安装位置和 PATH。
自定义路径单独存储于 SQLite `proxy_engine_settings`，数据库版本 12 是新增表的
加法升级，不改写旧代理、凭据和备份。已经验证本机 sing-box 1.13.13。

普通单代理继续由 reqwest 直接连接所选 HTTP 或 SOCKS5 代理，不要求安装 sing-box。
链式线路在后台首次使用时调用用户安装的 sing-box：出口出站 `detour` 指向前置出站。前置、出口各自遵循记录中的协议；本地入口统一为 SOCKS5。
每条活跃配置按需启动并复用独立实例，修改一条线路不会重启其它在途线路。
入口只监听 127.0.0.1，使用随机本地认证。配置经 `-c stdin` 输入，不写明文配置文件，
不把账号密码放入命令行或日志；仅以自建 Child 句柄管理进程，不按进程名杀其它实例。

客户端缓存含完整前置和出口配置指纹；前置账号、密码、地址、端口或引擎路径改变后，
不会使用旧 HTTP 连接池。Route 持有对应进程的 Arc，活跃请求完成前不会因缓存淘汰
关闭该进程。缓存最多保留 32 项、空闲 120 秒过期，每 5 秒清理；代理/引擎配置的
EventBus 变更同时使该 workspace 缓存失效。窗口退出通过 GPUI on_app_quit 停止自建进程。
正常 Drop 同样 kill/wait 回收；宿主被强制杀死不等同于正常退出，不承诺执行退出回调。

业务、聊天、图片和 IP 查询客户端统一使用完整线路，域名经 SOCKS5 传递；链路失败报错，
不跳过前置、不跳过出口、不直连目标。现有换 IP 提供商控制 URL 的独立控制请求方式保持不变。
IP 检测只展示查询结果及地区，不增加公网地址/IPv4 有效性判断。

## 验证

- 核心存储：前置复用、自引用/缺失引用/多层拒绝、被引用删除保护、已有记录缺省行为。
- 真实外部引擎 + 本地模拟 SOCKS5：HTTP/SOCKS5 四种组合分别认证、住宅代理域名经前置处理、目标域名经出口处理、故障不直连。
  未安装 sing-box 的环境会跳过外部引擎用例；本机已实际执行通过。
- 连接缓存：容量、空闲回收、配置事件失效、前置凭据变更拒绝旧路由。
- GPUI 回归：ListItem 选择单条详情、前置引用保存不增加重复记录；按仓库约定仅类型检查，未构建或运行 rv-app。

## Exclusive proxy handoff between virtual Apps

Ordinary shop reads, scheduled work, chat, avatar uploads and standalone image
requests use the same async occupancy entry. An exclusive proxy remembers its
last business virtual App (real IDFV) across task/request lease releases for the
lifetime of the RV process. First use adopts the current exit; A→A reuses it;
A→B invokes the configured HTTP/password-session rotation and confirms the new
exit before B's business request. B→B reuses it; B→A rotates again. Each exit
tracks ownership separately. Shared proxies do not automatically rotate.

Rotation holds the existing proxy gate; same-App concurrent entrants wait for
the handoff to finish instead of sending through stale credentials. The verified
client/full front→exit snapshot is retained, and cached pre-rotation HTTP sockets
are removed even when an HTTP control URL leaves the password unchanged. Images
inside an existing operation reenter that operation's captured route and never
start another handoff. Failed/cancelled preparation sends no next-App business
request and releases occupancy; the prior App remains the last business owner.
No task enabled/cron switch is changed. The configured rotation method is
required when an exclusive exit changes virtual Apps; there is no direct fallback.
