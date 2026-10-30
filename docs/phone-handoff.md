# 手机备份接管

店铺顶部接管按钮按当前归属显示目标：RV 管理时显示 Smartphone 交给手机，手机管理时显示 Monitor 交给 RV；未完成的手机交接仍可重试。两向均复用关联备份与 AlertDialog 确认。交给手机执行：

1. SQLite `app_execution` 记录 `to_phone`、目标设备、操作 UUID；同事务禁用该 App 全部 tasks 并取消后续时间，保留任务记录和 cron。
2. 经已认证 ZXTouch HTTP/2 自动关闭手机 App，确认主进程及扩展进程均退出；仅 ACK 不算成功。关闭失败不读取或回写备份。随后等待已运行 RV 任务结束；不重放结果未知的令牌刷新。导出最终凭据对应 state，冻结 SHA。刷新返回的 `scope` 写入主 App 数据容器偏好的 `any`／`current` 两个作用域的 `token_scope`，到期时间同步更新；响应未提供 scope 时保留已有值。
3. 核对手机备份的 Bundle／IDFV。以手机当前 manifest 为底，仅替换 RV 新 state 的 stateArchiveInfo；保留手机的 archiveInfo、roots 等 data 信息，不要求与 RV 历史 data 一致。
4. 经已认证 HTTP/2 依次上传 state.tar.zst、manifest.plist；每块携带本次读取的手机当前 data SHA，手机在 Shadow 目录锁内核对。两个文件分别发布，保留 mobile 属主与0600。
5. 从手机读回清单（逐字一致）、state（SHA256／大小一致），再把模式改为 `phone`。此处清单逐字比较的目标是本次合并后的上传清单，不是 RV 历史原清单。

中断／失败保留 `to_phone` 和冻结版本，可在原手机显式重试。重启不会自动恢复 RV 调度。业务请求按当前凭据判断，刷新接口仅允许 RV Token 来源；接管模式不作为业务请求总闸。

接管只表示备份回写完成及 RV 停止。Shadow 仍自行负责恢复；本功能不改 Shadow、不写 live Keychain 或偏好、不恢复 data.tar.zst、不启动 App。

## 交给 RV

店铺顶部 Monitor 与手机关联账号的备份行 Monitor 均执行交给 RV，复用同一后端交接流程。用户先在 Shadow 保存该环境最新备份并切换到其他环境，再在 RV 确认交接。RV 通过 TrollVNC 4.0-16 目录中的独立 `current_idfv` 比对目标，确认非 active 后下载所选 Bundle／IDFV 的 manifest 和完整 state，再原子采用凭据。此查询不读取或依赖 access/refresh token。RV 不创建或更新手机备份，不调用旧 save 接口，不依赖目录返回当前 active 备份字段。目标仍是 active 时明确中止，提示用户自行在 Shadow 备份并切换；下载后再次核对当前 IDFV。RV 不自动切换或恢复手机环境。

明确交接以手机此次交付的凭据为准：覆盖 RV 原有 access token、refresh token、scope、到期元数据和请求上下文。state、凭据、TokenSource::Rv 与旧接管归属在同一 SQLite 保存事务完成；失败保留原凭据与来源。成功后允许 RV 刷新，但不立即强制刷新。启用中的 Token 等待任务随新凭据唤醒，用户已停止的任务与 cron 保持原样。

刷新备份列表仅更新备份目录和设备关联；已有账号的 state、凭据、来源和任务不变。新发现账号缺少 state 时，首次导入仍默认手机来源。普通账号 state 同步继续保留 RV 当前凭据，不能代替明确交接。

## 修复 RV 本地状态

店铺详情的手机图标打开「状态与交接」选择框，提供「修复状态」和「交给手机」，选择后进入对应确认框；卡片更多菜单不再重复提供修复入口。即使当前已经由 RV 接管，也可在确认后采用关联手机中该 Bundle／IDFV 的最新备份 state，覆盖 RV 本地 state 和完整令牌对，并继续由 RV 负责刷新。修复前由用户在 Shadow 保存该环境最新备份并切换到其他环境，RV 读取所选备份已保存的状态，不自动备份或改变手机当前环境。修复最终读取的是完整 state，不单独从 live Keychain 拼接凭据。手机接管时原电脑图标仍执行交给 RV；正在交给手机的操作仍直接进入重试确认。

修复复用交给 RV 的读取及原子保存流程，不把 RV state 回写手机、不立即调用刷新接口，不启用用户已停止的任务。启用中等待 Token 的任务获得新凭据后继续调度；操作版本变化或身份不符仍拒绝提交。普通备份同步仍保留 RV 凭据，不隐式执行修复。

RV 当前 access token 有效时隐藏「修复状态」；只有缺失、过期或已被 HTTP401 明确拒绝时才允许修复，未知到期时间不视为失效。读取手机 state 前和最终保存事务内均重新检查；若期间已获取有效新 token，则拒绝采用手机备份，避免旧确认框覆盖新凭据。手机向 RV 的正常交接不受此修复条件限制。

验证：core 归属／停止／最终令牌导出／重启持久化测试，HTTP2 分块顺序和读回损坏／来源不符测试，原生分块／错误哈希／身份／权限／data 保留测试，GPUI 确认取消保留 RV 与店铺测试。手机4.0-12同内容回写全链路通过，手机接管标记验证使用隔离数据库，未切换真实 RV 账号模式。

## 2026-10-30 真机往返验证

本轮使用现有 RV 数据库（预先 SQLite backup，升级到 schema14），目标为已认证的 iPhone7Plus / iPhone9,2 / iOS15.8.8。手机安装正式4.0-15包并核对上传 SHA；未发布仓库、未构建或安装 RV GUI。

通过生产 SessionHandle / HTTP2 / WorkspaceStore 验证当前环境备份→RV采用完整 token 对→刷新 HTTP200→店铺资料 HTTP200→state/manifest交回手机并读回校验→Shadow UI 恢复→Vinted 登录资料仍为 michelley299→保存手机最新备份→交回 RV→再次刷新 HTTP200。两次成功刷新都返回不同 access/refresh 对并保存在现有 SQLite；最后新 access token 查询商品 HTTP200，商品结果落库且手动同步未修改 Tasks。未重放旧 refresh token 来试探失效，也不能据此声称 token 永久有效。

非 active 验证：Shadow 当前为另一 IDFV 时，读取目标 C148FA97…F420 自己的 state 并采用其 token 对，通过指纹比较确认没有使用 active 环境凭据。Shadow 不允许恢复当前 IDFV；切换环境时明确关闭先保存当前数据，避免 live 旧凭据覆盖刚由 RV 回写的备份。未修改 Shadow 代码。

本轮修复两个生产问题：交给手机成功后原子切换 TokenSource::Phone，避免仅改变旧 execution；RV采用手机备份最外层用 BEGIN IMMEDIATE，避免手机自动 token 写入与先读后写事务竞争产生 database is locked。回归246项、rv-app类型检查通过。最终目标来源与 execution 均为 RV，手机 Vinted 已停止，任务保持停止；最新轮换凭据只由 RV 使用。历史备份仍存在，不能宣称物理上没有旧 token 副本。

## 现行接口边界

2026-10-30 4.0-15 真机 `/api/app-state/list` 完成结果为 `bundle_id`、`format=ShadowAppState2`、`backups`，没有 `current_backup_id` 或 `restore_pending`。交给 RV 不应先要求这些字段。4.0-16 新增独立 `current_idfv`，直接读取 Shadow 当前配置，不依赖 token；明确没有身份时为 null，身份格式错误时报错。旧服务缺少此字段时提示更新，不再调用 token 接口。当前手机服务禁用了旧 save／switch 等操作，备份创建和恢复由 Shadow 执行；旧 TVASProtocol／PXAppStateSession 中仍存在的实现不代表当前 HTTP 服务开放了这些能力。

已移除错误的 active 判断、自动 save 调用及以假设响应编写的四项自动备份模拟测试。交给手机的自动关闭保留：只使用现存 stop/result 接口，关闭失败或进程未退出时不回写。保留的五项 HTTP2 测试覆盖关闭确认、失败、进程仍运行、回写顺序及读回校验；这些是模拟测试，不代表本轮真机交接成功。

## 4.0-16 独立身份查询修正

原先使用 `/api/apps/access-token` 判断 active 是错误耦合：手机切换到无 token 的环境后，非 active 目标也被阻止。现已完全移除该依赖，目录独立返回 `current_idfv`；不读取 Keychain。独立审查发现并修复了无效 UUID 被误当作 null 的问题。

已在7Plus安装4.0-16并只读核对：token 接口仍不可用时，目录仍返回与 Shadow 配置一致的当前 IDFV，且与 C65…AB53 不同。两份备份的六个文件哈希未变；本轮没有采用凭据或刷新 token。RV 需运行包含本次修改的新构建，不支持旧服务缺少独立身份字段时静默猜测。
