# 虚拟 App 延时任务

任务在 RV 本机通过业务 HTTP 执行，不依赖手机在线。只使用一张 `tasks` 表，一条任务记录复用到结束，保留最新执行结果，不创建执行明细表。该功能新建表，不读取或迁移旧任务数据。

## 时间与 cron

所有时间计算在 Rust 中完成。SQL 只保存、比较计算后的 Unix 秒时间戳，不使用 SQL 日期函数。cron 按运行 RV 的电脑本地时间解释，不提供 timezone 字段或参数。

格式为 `秒 分 时 日 月 星期 [年]`，支持六段或七段。例如：

| 表达式 | 执行时间 |
| --- | --- |
| `0 */5 * * * *` | 每五分钟 |
| `0 30 9 * * *` | 每天 09:30 |
| `0 30 9 28 10 * 2026` | 2026 年 10 月 28 日 09:30，仅一次 |
| `0,10 30 9 28 10 * 2026` | 指定日期 09:30:00 和 09:30:10 |

`next_task_time` 返回下一次匹配或 `None`，以此决定继续或结束，不设一次性/周期类型或执行次数上限。`task_cron_at(timestamp)` 可将延迟操作的目标时间转换为具体日期 cron。创建时允许当前秒执行；完成后从当前时间与开始时间的较大值继续查找，不逐条补跑执行期间错过的匹配。

RV 后台每秒读取到期任务。同一虚拟 App 执行中的任务完成后再处理后续任务。RV 关闭时不执行；重新启动后，尚未过期的 pending 任务可以继续处理。已运行但超过执行期限的任务标为 unknown，不自动重复未知写操作。

## 调用

业务调用方使用 `rv_business::tasks::enqueue`，由它验证操作与参数；不要直接向表插入业务任务。

```rust,ignore
use rv_core::workspace::{TaskSpec, task_cron_at};

// run_at 在调用方 Rust 代码中按所需延迟计算，单位为 Unix 秒。
let cron = task_cron_at(run_at)?;
let spec = TaskSpec::new("products", cron);
let task = rv_business::tasks::enqueue(&paths, &app_id, &spec)?;
```

支持的操作：

| kind | payload |
| --- | --- |
| session、notification_stats、products、orders、refresh | `{}` |
| chat_inbox、chat_threads、chat_send、chat_read | `{}`，固定店铺任务；动态参数在聊天队列与读取意图中保存 |

执行时重新读取此 IDFV 最新的加密凭据和代理配置。payload 不接受 token、密码或额外字段；结果沿用现有业务脱敏摘要。发送消息、标记已读的 `max_attempts` 必须为 1，避免结果不明确时重复提交。

`WorkspaceStore` 提供 `task`、`tasks`、`query_tasks`、`cancel_task`、`reschedule_task`、`run_task_now`、`set_task_enabled`；取消只接受 pending 任务；修改 cron 可以独立于启停状态保存。立即执行保留周期 cron，用户可以显式重试任务，聊天结果未知的 outbox 仍不自动重发。后台通过 `due_tasks` 读取，再由执行器认领原记录，不额外创建执行记录。

## 表字段与结果

- 身份与参数：id、app_id（真实 IDFV）、kind、payload、cron_expression。
- 流程：task_key（每个 App 内唯一的自动任务用途）、depends_on（同 App 的前置任务）。
- 调度与预算：next_run_at、expires_at、attempt_count、max_attempts、timeout_seconds、deadline。
- 最新结果：status、last_run_status、result、error_code、error_message。
- 时间：created_at、updated_at、started_at、finished_at。

status 为 pending、running、completed、failed、unknown 或 cancelled。周期任务完成一次后仍为 pending，last_run_status 和 result 保存上一轮结果；没有后续匹配时保留最终状态。

默认请求超时 90 秒、每轮最多尝试 1 次。允许重试的读取操作遇到网络结果未知或 HTTP 429/5xx 时，按 30、120、480、1920 秒退避，且受 max_attempts 和 expires_at 限制。新的 cron 轮次重新计算尝试预算。HTTP 401 标记当前 access token 被拒绝；依赖它的后续请求等待新凭据，不关闭任务开关；403 保留具体错误，不当作全局 token 失效。前置校验失败不消耗请求次数，记录错误后计算下一次 cron；单次任务则失败结束。执行期限在请求超时之外预留 15 秒，用于超时收尾和结果落盘。

归档虚拟 App、到达 expires_at 会取消等待任务。刷新失败保留错误并按配置调度，用户可以显式重试；停止会话保持后不自动刷新。

## 导入与阶段任务

备份目录同步完成后，使用已认证 HTTP/2 串行下载本地尚未保存的 canonical Shadow state；解析、验证后保存完整 state 及独立请求凭据。导入时即校验凭据，缺失则明确报告导入失败，不建立接口任务。目录元数据本身不代表已有账号凭据。不重复下载已保存 state，不覆盖 RV 已更新的令牌。

state 导入成功后建立以下任务，稳定 task_key 保证重启／重复导入不重复创建，不重新启用已取消的任务：

| task_key | kind | 默认 cron | 首次执行／前置条件 |
| --- | --- | --- | --- |
| profile | session | `task_cron_at(导入时间 + 10秒)` | 具体日期的一次性 cron，成功后结束 |
| products | products | `0 */15 * * * *` | 基础资料成功后 |
| orders | orders | `0 */15 * * * *` | 基础资料成功后 |
| inbox | chat_inbox | `0 * * * * *` | 基础资料成功后 |
| chat_threads | chat_threads | `*/15 * * * * *` | 基础资料成功后，同步待更新与当前选择的会话 |
| chat_send | chat_send | `*/2 * * * * *` | 消费发送队列，无待发消息时不联网 |
| chat_read | chat_read | `*/5 * * * * *` | 消费已读意图，无请求时不联网 |
| notifications | notification_stats | `0 */10 * * * *` | 基础资料成功后 |

首次时间可以早于周期 cron 的下一匹配，后续时间仍由原 cron 计算。所有任务使用当前 App 的已关联代理；未关联时保持 pending，在 tasks 中记录等待原因，不发请求、不消耗尝试次数。用户编辑虚拟 App 保存代理后，下次调度检查会继续执行；不重新解压 state。已有任务运行前发现凭据被移除时同样等待。前置任务失败／取消时，依赖任务保持等待，不绕过前置条件。

`TaskWorker` 位于业务模块，独立线程持有执行器，每秒处理到期任务；UI 不执行 HTTP。调度遍历全部到期任务后判断凭据／代理，缺少条件的账号不会挤掉其它可执行账号。缓存锁、代理等待和图片下载共用所属拉取任务期限，超时会取消等待并正常结束本次读取，不让底层下载越过 watchdog 继续执行。

店铺资料、商品、订单及店铺通知的手动读取直接调用 `shop_sync::synchronize`，不 enqueue、不创建或修改 Tasks。任务配置中的“立即执行/重试”仍操作调度任务本身：周期任务保留原 cron；一次性任务提前执行将具体执行日期改为当前时间。发送消息通过 `queue_message` 保存发送队列；选择／刷新会话、标记已读分别保存读取意图并唤醒已有固定任务，不为每条消息创建任务。停止的任务不会被这些操作重新启用。

## UI 边界

店铺、商品、订单和聊天 UI 只查询 SQLite 与本地图片缓存；打开页面不会拉接口。空列表显示空状态，只在 SQLite 读取期间显示加载提示。删除旧的店铺定时开关和内存计时调度，不再建立 business_schedules 或 vinted_sync_state，也不把 last_task 等执行摘要写入店铺 metadata。

设置 → 任务使用 GPUI Component DataTable 查询 tasks，支持搜索、立即执行、取消、修改 cron；展示虚拟 App、任务类型、状态、cron、下次时间、最近开始／结束、最近结果、前置任务和等待／错误原因。最多显示最近 500 条匹配记录，无单独执行明细表。

店铺新增「任务配置」Tab，只展示该店铺的任务名称、cron 和修改／启动／停止控制，不混入执行日志。使用 GPUI Component 默认 TabBar、横向 Form/Field、Input 和 Button，编辑时同一行保存／取消，去掉说明段落与表头，保留紧凑布局。停止只阻止后续调度；已发送的本轮读取会完成，完成后不再安排下一次。停用期间修改 cron 不会启动任务；启动时按已保存 cron 计算下一次时间。默认 cron 只用于首次创建，重复初始化不覆盖用户配置。


## 图片属于拉取任务

不存在独立 images 任务或图片 cron 配置。基础资料任务缓存头像；商品、订单、会话／消息任务缓存当前回复中需要展示的图片，完成缓存后才提交业务数据和成功结果。图片按 URL 复用现有磁盘缓存，命中不发送 HTTP；未命中使用当前 App 关联代理，沿用当前任务的剩余期限。

明确图片缓存失败按 failed 处理，读取任务在预算内重试，重试耗尽后周期任务继续计算下次 cron，不误标为 unknown 永久停调度。一次性基础资料未成功时保留失败结果，由用户修改计划或立即重试。未知的写操作仍禁止自动重放。


## 初始化错误与日志

数据库版本 10 检查本功能开发期间的空任务表。若缺少 task_key、depends_on 或 enabled，只有确认 tasks 为零条记录后，才在事务内重建本表；其它业务数据不改动，非空的不兼容任务表拒绝自动删除。不迁移旧任务数据。

数据库初始化、state 导入、任务提交、调度、执行和配置操作的失败均进入现有 tracing 日志，带操作名与适用的 App／任务标识，不输出凭据或任务 payload。配置界面对数据库等内部错误只显示简短提示，详细错误写日志。店铺各 Tab 使用同一宽度上限和固定的自适应内容高度，空状态与报错不改变面板尺寸。


## 本地删除后的重新导入

仅删除本地卡片时保留 app_dismissed 标记，普通自动同步不会让卡片重新出现。需要重新导入时，在手机工具 → App 环境刷新备份列表，从所选快照右侧更多菜单选择「同步到本地」。该操作验证手机目录中的 Bundle／IDFV／快照身份，只清除所选 IDFV 的删除标记并重新建立本地卡片，随后使用只读 ReadProfile 下载 Vinted state。其它删除标记保留；不执行手机恢复、删除或写入。


## 2026-10-27 真机链路实测

手机4.0-10只提供新备份目录列表，不提供state下载，实测下载HTTP400。安装4.0-11后，RV正式HTTP/2会话完成204800字节state下载、电脑端SHA校验／解压、上下文提取和当前SQLite落库。仅TrollVNC服务重启，无整机重启。

发现并修复首次凭据保存对偏好的无意义修改：保留到期时间的小数秒，且只修改实际读取的data容器／any-host App域，不向其它host同名域写入凭据字段。真机未修改state导出保持下载归档逐字节一致，回归覆盖小数秒与同域不同host记录。测试探针比较归档时仅报告布尔失败，不打印归档字节。

用户授权将现有127.0.0.1代理关联到当前店铺。手机Vinted结束后仅RV刷新令牌，HTTP200；基础资料与商品、订单、会话、未读统计任务成功。实际账户商品／订单／会话均为空，商品数量与基础资料一致为0；无头像URL，未发生头像下载。本轮未将RV更新后的state写回手机，手机Vinted保持关闭。无Token／响应正文写入测试日志。


## 店铺身份与凭据时间显示

Vinted 卡片和详情使用同一店铺身份及头像逻辑，资料返回后以店铺昵称作为显示名称；真实头像命中本地缓存时使用头像，否则复用原 Vinted Bundle 图标。详情合并为一块 header，移除备份名／App 标题与独立店铺资料的重复层级。国家、评分、评价数、商品数使用现有组件图标与数值，标签放 tooltip。

已确认布局参考 [紧凑店铺面板](designs/rv-shop-task-config-v3.png)。顶部第一行展示昵称和查询资料／消息／未读／刷新令牌／读取账号／关闭图标按钮；第二行右侧以一个 Shield 图标和 caption 小字单行展示 `Access 时间 | Refresh 时间`，短日期为本机时区的月日／时分，完整日期与 Token 类型放 tooltip，已过期使用主题 danger 色。时间为被动文本，不再使用按钮承载。`TokenExpirations` 仅含两个可空 Unix 秒时间戳；UI 从 SQLite 读取凭据后只保留该投影。Access 优先服务器 expires_at，再用 JWT exp；Refresh 使用当前 Token 的服务器绝对时间／JWT exp，服务器只给 duration 时必须有实际接收时间。未提供期限则显示未知，JWT 解析不作为有效性认证。刷新 Token 时清除上一对 Token 的 refresh expiry 字段，避免把旧期限显示给新 opaque Token。


最新操作命名：店铺资料查询入口统一为「刷新资料」，它通过共享异步业务执行器调用 Vinted 并更新业务表；手机 state 读取入口为「同步账号备份」，其 tooltip 明确凭据和设备信息来源。取消店铺手动刷新令牌按钮；全局执行列表只展示 refresh 记录，不提供立即执行／启停／改计划操作。凭据图标按 Access Token 的可知期限使用主题 success（未过期）、danger（过期）、muted（未知）色，到期时间不作为服务端认证有效性的证明。


### 会话保持与凭据依赖

会话保持 `keepalive`（操作 `refresh`）默认 cron 为 `0 0 * * * *`，只负责自动刷新 token。用户停止它后，商品、订单、聊天等任务仍可使用当前 access token；基础资料也不依赖先刷新成功。任务各自保留启停与 cron，执行归属规则继续生效。

所有任务从 SQLite 获取当前加密凭据。认证请求收到 401 时，在事务内只拒绝本次使用的 token；旧请求迟到的 401 不能拒绝新 token。后续请求及任务认领统一检查拒绝状态，等待新凭据，不继续消耗 HTTP 请求。等待任务保留 enabled 和 cron；刷新任务本身不被 access token 的拒绝状态阻止，但用户已停止时不自动启动。

保存不同的 access token 清除旧拒绝标记；同 token 的元数据保存不能复活它。凭据提交后发布 Credentials EventBus，后台立即重新读取等待任务；每秒扫描继续用于重启和跨进程对账。UI 通过统一 Notify 显示任务已提交或提交失败，执行结果保存在任务记录。

`X-Session-Id` 按 access token 绑定，明文保存到 `app_request_sessions`，用店铺＋token SHA-256 查询；同一 token 跨任务、操作和重启复用一个随机小写 UUID v4，token 更换时使用对应的新值。Session 不放入加密凭据，access/refresh token 仍加密。User-Agent 沿用设备/App 上下文，Locale 和 Accept-Language 分别读取偏好，不能由 Locale 推导语言。

HTTP 传输失败日志包含任务 span（task_id/app_id/kind）、method、去掉用户信息和 query 的 endpoint、失败阶段 send/body/build、timeout/connect 标记及底层错误链。任务 error_message 保留同一脱敏原因，不再只输出“聊天请求中断，结果未知”；日志与结果不记录 Authorization、令牌或代理密码。

### 店铺手动刷新与定时调度分离

`shop_sync::synchronize` 使用共享 Tokio 执行器，GPUI 直接 await 返回结果；等待代理锁、网络、分页不阻塞 UI，也不占用店铺的 Tasks running 名额。手动与定时路径共用业务处理器、HTTP/token 处理、每轮 Session ID、图片缓存及分页原子提交。手动成功只提交业务数据/店铺投影，经 EventBus 更新卡片和当前页面；失败通过统一 Notify 展示，不产生调度记录。

店铺顶部“刷新资料”和未读查询走手动执行；商品、订单 Tab 右侧新增刷新图标。按钮按店铺和操作展示 loading，同一操作进行中不重复提交，不影响其它店铺或不同操作。Tasks 中原有 cron 和启停不被手动刷新修改。聊天手动读取、发送和已读采用下述独立异步流程。

手动请求在每个 API 及图片下载开始前核对当前 RV 归属；期间交给手机则不再发送后续分页/图片请求。读取中遇到 401 沿用共享 token 拒绝状态。

聊天 `chat::sync` 直接执行 Inbox/Thread/SendPending/MarkRead。读取与写操作共用既有协议/代理/token 检查和落库逻辑；手动操作不创建/唤醒 Tasks。待发消息先持久化，立即异步消费，同会话串行，发送后回读平台 ID 对账；周期任务仅后台同步和兜底消费。已读/刷新会话后主动读取服务端未读数量。错误使用统一 Notify，保留真实 HTTP 状态/服务端错误和脱敏传输原因。

头像预览确认同样走 `avatar::set` 直接异步执行，不创建一次性 Tasks。旧的头像调度记录继续使用相同 HTTP 管线执行；两条路径共用上传/绑定/回读验证与数据提交。头像设置与普通同步的 busy 状态共同阻止操作中删除店铺，完成后各自清理，不相互覆盖。
