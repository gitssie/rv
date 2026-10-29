# 店铺会话与发送队列

消息使用独立原生 GPUI Window，布局参考 `designs/rv-chat-store-conversations-v3.png`。初始内容尺寸沿用原聊天弹层，三栏比例保持不变。左侧店铺，中间当前店铺的会话，右侧消息与单一输入框；店铺、会话不提供搜索。列表、头像、输入与消息气泡使用 GPUI Component，原文与已有译文在同一气泡内显示。未接入翻译服务时按原文发送，不生成模拟译文。

## 模块边界

- `rv-business::chat::repository`：SQLite 查询与业务结果落库，不进行 HTTP。
- `queue`：只保存发送／已读／会话选择意图，不创建或唤醒 Tasks。`sync`：立即异步刷新、发送和已读，复用共享执行器；周期任务作后台同步及消费兜底。
- `workflow`：发送、详情、已读消费者，由定时任务或手动异步操作共用；代理等待完成后重新校验任务、接管模式、会话权限和发送正文，再领取队列。
- `vinted`：HTTP 协议、响应解析和客户字段提取；其他 Bundle 可注册独立策略。
- `translation`：预留异步 `TranslationProvider` 接口、输入与结果类型。暂未调用提供商；语言设置仅保存客户沟通语言。
- `rv-app::app::chat`：SQLite DTO 与本地图片展示、草稿、选择和用户意图；不发业务或图片 HTTP。

## 存储

| 表 | 用途 |
| --- | --- |
| chat_conversations | 店铺＋平台会话 ID 唯一，保存会话、未读和商品上下文 |
| chat_messages | 会话＋平台消息 ID 唯一，保存消息和可选原文／译文 |
| chat_customers | 店铺＋平台客户 ID 唯一，昵称、头像、国家、语言及来源、标签、备注 |
| chat_conversation_customers | 会话与客户关联 |
| chat_outbox | 动态发送队列：原文、可选待发译文／语言、可发送时间、当前结果、关联固定任务和平台回执 |
| chat_drafts | 每个会话一个本地草稿 |
| chat_view_state | 每个店铺当前关注会话，固定详情任务优先同步 |
| chat_read_requests | 用户标记已读的待处理意图 |
| chat_detail_sync | 已覆盖的会话更新时间及本地拉取时间，用于详情增量选择和公平轮询；不保存任务执行日志 |

客户语言优先采用显式设置，其次平台语言，再按国家提供默认值；手动设置不会被资料更新覆盖。未知国家不猜语言。消息的原文与已有译文会在后续远端同步中保留。

## 执行路径

导入凭据后创建固定店铺任务。收件箱默认每分钟，详情每 15 秒、发送每 2 秒、已读每 5 秒；这些任务沿用店铺 cron 配置和启停。无队列／无待读取会话时，消费者不占代理、不发 HTTP。会话保持只控制自动刷新 token，不阻止现有 token 的请求；手机接管和 401 拒绝状态由共享 HTTP 管线校验。

发送先在事务中保存 outbox 并删除草稿，再清空输入框。异步消费者按会话顺序消费，拿到有效平台消息 ID 才记为 submitted。会话详情按同一平台 ID 确认并合并；不靠文本相同猜测送达。网络中断、无回执或结果落库未确认标为 unknown，不自动重放。用户核对后可取消该记录；取消未知记录不停止整个店铺发送消费者。确定失败保留记录，不自动重新发送该消息。

图片统一复用 `business-images`，按完整 URL 哈希存储。头像、商品、订单、会话和消息共享缓存；命中不请求，未命中由所属任务通过其捕获的代理下载、验证并原子发布，完成后才提交业务数据。任务中途更换代理会拒绝继续，不能自动换出口。渲染只读本地路径。

本地与测试 HTTP 服务验证覆盖发送队列、任务复用、权限、未知结果、对账、客户隔离和组件布局；未向真实客户发送测试消息。原生组件来源见 `crates/rv-gpui-chat/SOURCE.md`。

实测新版接口的店主使用 `opposite_users.type=member`、`login_name`，商品上下文为 `data.items`／`inventory_item_id`。发送成功返回 201，消息对象位于响应根节点。`legacy_reply` 的正文读取 `data.entity.body`；其外层 UUID 每次读取可能变化，内部 entity ID 才是稳定的平台消息标识。发送的 `reply_plain.data.id` 与该 entity ID 对齐，RV 使用同一 `reply:<平台ID>` 保存与对账，避免重复拉取产生重复消息；不按相同正文猜测已送达。

## 只读通知

消息窗口沿用店铺／列表／详情三栏，中栏以「会话 / 通知」Tab 切换。
通知按店铺隔离，列表显示正文摘要、平台时间和平台未读标记；右栏显示完整正文、备注及接口提供的商品／订单标识、本地缓存图片。每家店铺分别记住选中的通知，会话和草稿继续保留。

通知由固定 `notification_list` 任务（kind=`notifications`，默认每五分钟）通过店铺绑定代理读取 `GET https://api.vinted.com/inbox-notifications/v1/notifications?page=N&per_page=20`，收齐分页、缓存图片后原子 upsert 到 SQLite `vinted_notifications`。既有 `notification_stats` 仍为聊天未读统计，不能当作通知列表或通知未读数。新任务沿用会话保持、基础资料前置和 RV／手机归属约束；打开页面、切换 Tab／店铺、选择通知只读本地，不提交拉取、已读回执或写操作。

通知标题缺失时用正文首行；HTML 转为惰性纯文本，未知类型保留原始字段并可读展示，不按类型过滤通知。平台未提供时间／已读状态时保留未知，不推算。通知与会话数字角标分别使用专用接口的未读数，不数本地记录；打开通知不修改平台或本地已读状态，不提供回复、订单操作或链接跳转。空列表沿用 Empty 图标；通知页提供显式刷新图标，调用共享异步执行器，不创建 Tasks；loading 与错误反馈沿用原生按钮和统一 Notify。打开/选择通知仍只读本地，不发送已读回执。

## 后台未读消息提示

店铺侧栏及会话页签的未读数字来自 `GET https://api.vinted.com/messaging/main/users/unread_count` 的 `unread_count`，以 `unread_count` 保存到店铺 metadata。通知数量读取 `GET https://api.vinted.com/inbox-notifications/v1/notifications/unread_count` 的 `count`，随原通知任务保存为 `notification_unread_count`。接口未提供数量时保留未知，不默认 0；不按本地会话／通知行数推算，也不根据已读操作在本地加减。

未读消息查询复用既有 `notification_stats` 后台任务（task_key=`notifications`）；新建任务默认每十分钟查询（`0 */10 * * * *`），已有店铺保留用户原 cron 和启停。定时收件箱同步不提前触发查询；用户手动刷新会话或显式标记已读后，直接异步查询该接口更新数量。调度和结果落库均不依赖聊天窗口打开，主窗口收到任务变更即读取最新店铺计数。

顶部消息入口仅在某店铺接口未读数大于 0 时显示红点，悬停分别显示店铺与接口数量，不合计多个店铺的数字。通知未读数不触发这个新消息红点。仍遵守 RV／手机接管和任务启停，未读查询不刷新令牌、不自行启用停止的任务。2026-10-28 两个计数接口均已只读实测 HTTP 200。

接口确认：2026-10-28 从当前手机 Vinted 二进制确认路径，复用账号与绑定代理只读实测返回 HTTP 200，`notifications=[]`，分页为 `current_page/per_page/total_entries/total_pages`。当前账号无通知；非空通知正文、跨店铺隔离和重复同步由离线样例验证，不声称已有真实非空通知的手机对照。

## 发送图片

会话输入区的「发送图片」按钮选择一个本地文件，预览完整图片并明确显示接收人，确认后才加入队列。图片单独发送，保留文字草稿，不附带文字；取消或切换店铺／会话／通知页不会误发到其它目标。手机接管、只读或不允许回复时禁用。

`chat::images::prepare` 处理 EXIF 方向，保留比例、不裁切、不放大小图，长边最多 1600 像素，编码为 JPEG。源文件最大 8 MB，透明区域转白。确认时将图片内容保存到 SQLite `chat_outbox_images`，与空正文的 `chat_outbox` 行在同一个事务入库，删除或移动原文件不影响发送。待发预览读取 SQLite 图片快照；已接收／确认的消息图片由任务下载到共用 `business-images`，以完整图片适配气泡显示。

图片与文字复用同一待发箱及队列顺序，确认入库后立即触发异步消费者，不等待固定 `chat_send` 任务；定时任务仅兜底。领取后通过同一店铺绑定代理与租约：

1. `POST https://www.vinted.com/api/v2/photos`，multipart `photo[type]=user_msg`，图片字段 `photo[file]`，取成功回执的 `temp_uuid`。
2. `POST https://api.vinted.com/messaging/main/conversations/{uuid}/replies`，JSON 为 `{"content":"","photo_temp_uuids":["上传引用"],"is_personal_data_sharing_check_skipped":false}`。
3. 成功根回执 `reply_plain.data.id` 与回读 `legacy_reply.data.entity.id` 使用相同 `reply:<id>` 对账。图片位于 `data.entity.photos`；会话摘要显示 `[图片]`，消息正文保持为空。确认对账后删除 outbox，图片快照随外键级联清理。

任一步网络中断、上传引用或发送回执不完整时，不自动重发；使用已有 unknown／取消核对流程。claim 同时核对图片快照，避免代理等待期间队列变化后发送旧内容。

2026-10-28 已获授权向既有测试店主发送一张 [雏菊图](assets/chat-daisy.png)：上传 HTTP 200、消息 HTTP 201，回读确认为同一消息 ID、空正文、一张照片。未回复店主提问，未重复发送。协议实际验证通过；新增 GPUI 入口仍按约定只做类型和样式检查，未构建或部署。


## 手动操作与 Session

会话列表和消息详情各有刷新按钮，选择会话也立即异步获取详情。文字/图片先进入持久化待发箱，再按会话串行消费；重复触发不重复领取，不自动重放 unknown/submitted。没有 task_id 的手动 sending 在 90 秒窗口内不会被恢复流程误判为中断。标记已读只处理选定会话，401 时保留待读意图。

手动刷新、发送和已读不创建或修改 Tasks，结果落库后通过 EventBus 刷新 UI，错误使用统一 Notify。Session ID 在明文 SQLite 表按店铺及 access token 摘要保存，同一 token 的所有操作和重启共用一个随机 UUID v4；凭据 token 仍保持加密。

## 独立消息窗口

入口复用单个原生窗口，已打开时激活并按需切换店铺，不改变主窗口当前弹层。标题栏、关闭及拖动使用 GPUI Component TitleBar，支持 Cmd+W。窗口自己的 Root 渲染图片确认弹窗和 Notify，主窗口不再绘制 Chat Modal。

聊天控制器与 EventBus 订阅继续共享，不新建任务调度器；输入框在聊天窗口中创建并只绑定一个输入订阅，关闭后清理句柄及图片选择忙状态，重开从持久草稿恢复。迟到的旧窗口回调按窗口标识隔离。已添加窗口复用、关闭重开、草稿恢复的 GPUI 回归用例；按项目约定仅执行类型和样式检查，不构建/运行 GUI 或部署。
