# RV 手机工作助手实现

入口是会话工具栏右侧「助手」。展开本地侧栏后仍可用顶部 App 快捷栏打开 Vinted、照片或其他 iOS App；切换手机前台 App、收起侧栏都不会重置草稿。展开时手机画面按比例适应剩余空间，收起后恢复原缩放设置。

## 模块边界

| 模块 | 职责 |
| --- | --- |
| `rv-core/src/assistant/models.rs` | 草稿、素材、模板、导入回执；整数分金额解析 |
| `rv-core/src/assistant/repository.rs` | SQLite 迁移、版本冲突、素材归档、原子领取导入任务 |
| `rv-core/src/assistant/service.rs` | 每个数据目录共享数据库工作线程，UI 不直接操作数据库 |
| `rv-app/src/work_assistant/` | 独立 UI Entity；drafts、images、templates、notifications、widgets、persistence 分工 |
| `rv-app/src/work_assistant/draft_form.rs` | 商品草稿表单与紧凑操作布局，和 CRUD/保存逻辑分离 |
| `rv-app/src/session_window/assistant_bridge.rs` | 把明确的助手事件连接到当前手机会话 |
| `rv-app/src/photo_transfer.rs` | 可复用图片批次、传输预约、进度、停止后续任务、回执恢复 |
| `rv-session/src/apps.rs` | 能力协商、UTF-8 剪贴板请求和确认 |
| `rv-session/src/file_transfer.rs` | 跨窗口共享的传输预约；排队请求不能被批次抢占 |
| `TrollVNC/src/AppManagement.mm` | 手机剪贴板写入与关联确认 |
| `TrollVNC/src/PhotoLibrary.m` | UUID 照片任务持久回执与查询，防止未知结果自动重复导入 |

## 本地数据与编辑

在现有 RV 数据目录创建 `assistant.db` 和 `assistant-assets/`。数据库开启 WAL、外键和 FULL 同步，使用 `user_version` 迁移。连接、偏好、VNC 密码和解锁密码继续沿用原配置文件；不接入系统钥匙串。

商品草稿支持新增、切换、复制、删除、标题、描述、EUR 价格和库存备注。修改停止 500 ms 后自动保存；金额用整数分，不使用浮点计算。多窗口冲突会显示错误；关闭仍有冲突的编辑器会保存为「恢复副本」，保留原草稿。关闭时等待实际保存结果处理尾部修改，不猜版本号。未完成或非法价格单独保留，关闭、重开、复制草稿均不丢失；发送价格前仍需修正。素材操作错误不阻止草稿保存，数据库保存错误单独显示。

图片通过选择文件或拖入添加，复制为本地拥有的不可变素材，SHA-256 命名；支持拖动排序和移除。支持 JPEG/PNG/HEIC/HEIF，单文件最多 50 MiB。HEIC/HEIF 在侧栏显示文件图标，手机导入仍由 PhotoKit 解码。复制草稿共享不可变素材；目前移除后保留素材文件，不自动清理孤立文件。

常用文案有名称、分类、内容和显式保存/删除。输入变化会即时更新保存、切换和新增按钮状态；有未保存内容时禁用切换和新增，并提供放弃修改。文案可替换商品描述或直接发送手机剪贴板；从商品页选择文案、或从文案页点击「用作当前商品描述」，都会更新实际草稿并进入自动保存流程。

## UI 对齐与实际渲染

助手侧栏采用下划线页签；标题、描述与小型发送按钮并排；价格与本地库存备注同一行。选择器保留长标题省略，右侧新增和操作菜单始终可见；复制、删除在草稿操作菜单内。图片使用紧凑缩略图，导入与文案操作按钮按内容宽度显示。长描述在输入框内滚动，侧栏内容可独立滚动，不覆盖价格或底部状态。

实际 GPUI 渲染截图（手机棋盘画面、衣服缩略图和数据均为测试样例）：

- 商品草稿：[深色](assistant-implemented-dark.png)、[浅色](assistant-implemented-light.png)、[小窗口](assistant-implemented-compact.png)
- 常用文案：[深色](assistant-templates-dark.png)、[浅色](assistant-templates-light.png)
- 手机通知：[深色](assistant-notifications-dark.png)、[浅色](assistant-notifications-light.png)

## 手机操作

发送文本使用 capability `clipboard_write` + operation 16，带 UUID token，UTF-8 JSON 请求最多 4 KiB。只有收到相同 token 的成功确认才显示写入完成。旧服务器走现有 CopyText 通道并明确提示结果未确认，不重试。发送后在手机输入框粘贴，发布仍由用户在 Vinted 内完成。

图片导入复用文件上传、校验和 PhotoKit 链路。数据库在一个事务中按图片和手机地址领取任务；其他窗口不能重复领取正在执行的任务。批次预约同一会话的传输通道，避免与文件窗口竞争。成功保存相册资产 ID；再次导入会跳过已完成图片。未知结果保留 UUID，支持新服务端时仅查询已有任务；旧服务端提示用户核实，不自动重发。停止会取消后续发送；已开始的手机端操作不能保证撤销。

`photo_tasks` 表示服务端支持持久 UUID 任务。上传名为 `rv-upload-{UUID}-{16位十六进制随机数}--{filename}`，每次上传使用独立临时文件，任务 UUID 保持不变；手机端兼容旧 `rv-upload-{UUID}--{filename}` 格式。复用 File Management 导入/任务查询操作。服务端在 PhotoKit 操作前持久记录 pending，完成后持久记录结果；启动后读到未完成回执返回未知，不重新执行。服务端回执位于 mobile 本地缓存目录。

上传前的本地校验失败立即返回原任务 ID，只有确定尚未提交的失败可重新领取。PhotoKit 已提交后的超时保留未知状态；提交成功且取得资产 ID 后即记录完成，不因读取照片暂时不可见而重发。查询提示任务不存在时，用户核实相册后通过「确认未导入，恢复」以原 UUID 恢复；查询请求未发送也保留未知状态。旧数据库无法判定提交阶段的失败回执在 v2 迁移时转为未知。

通知页首次打开即读取会话已经收到的通知；复用 SpringBoard 通知捕获、RFB 推送和现有缓存，支持刷新、自动接收开关以及全部/Vinted 来源筛选。筛选按 source 中的 Vinted 字样匹配。通知不是完整通知中心历史，也没有自动回复或跳转到指定聊天。

## 验证

当前验证：182 项测试通过，2 项手动预览导出测试默认跳过；助手三页深浅色及小窗口导出已单独运行通过。

运行 `cargo test --workspace --locked -- --test-threads=1`、`cargo clippy --workspace --all-targets --locked -- -D warnings`。回归覆盖冲突恢复、非法价格关闭恢复、素材错误与保存隔离、照片提交阶段分类、缺失任务确认恢复、查询未发送仍保留未知状态、首次通知缓存、剪贴板能力/只读/大小/确认，以及助手输入焦点和缩放坐标。

`cargo test -p rv-app export_assistant_previews --locked -- --ignored --test-threads=1` 使用原生 Metal 渲染实际会话和助手三页的深浅色、小窗口截图。数据库为临时目录；这些预览不代表真实设备验证。可见控件点击测试覆盖文案编辑、保存、应用到草稿并落库，断线/只读发送限制，以及长标题在窄侧栏中的布局。

TrollVNC 已通过 Theos 构建、App 管理策略测试和新旧照片临时文件名解析/清理的 Foundation 测试。新剪贴板、PhotoKit 持久任务及 SpringBoard 通知仍需更新真实设备服务端后联调确认。
