# 手机备份接管

店铺顶部 Smartphone 按钮选择来源手机，经 AlertDialog 确认后执行：

1. SQLite `app_execution` 记录 `to_phone`、目标设备、操作 UUID；同事务禁用该 App 全部 tasks 并取消后续时间，保留任务记录和 cron。
2. 等待已运行任务结束；不重放结果未知的令牌刷新。导出最终凭据对应 state，冻结 SHA。刷新返回的 `scope` 写入主 App 数据容器偏好的 `any`／`current` 两个作用域的 `token_scope`，到期时间同步更新；响应未提供 scope 时保留已有值。
3. 检查目标相同 Bundle／IDFV 的 manifest 和 archiveInfo 与本地来源一致。
4. 经已认证 HTTP/2 依次上传 state.tar.zst、manifest.plist；每块携带期望 data SHA，手机在 Shadow 目录锁内核对。两个文件分别发布，保留 mobile 属主与0600。
5. 从手机读回清单（逐字一致）、state（SHA256／大小一致），再把模式改为 `phone`。

中断／失败保留 `to_phone` 和冻结版本，可在原手机显式重试。重启不会自动恢复 RV 调度。业务请求按当前凭据判断，刷新接口仅允许 RV Token 来源；接管模式不作为业务请求总闸。

接管只表示备份回写完成及 RV 停止。Shadow 仍自行负责恢复；本功能不改 Shadow、不写 live Keychain 或偏好、不恢复 data.tar.zst、不启动 App。

## 交给 RV

手机来源账号的备份行 Monitor 按钮与店铺 Token 来源菜单的 RV 选项共用交接流程。确认已保存该环境的最新备份、且手机不再使用该环境后，经已认证 HTTP/2 重新读取所选 Bundle／IDFV 对应的完整 state。非 active 备份同样读取自己的 state，不读取当前 live Keychain，不恢复或启动手机 App。

明确交接以手机此次交付的凭据为准：覆盖 RV 原有 access token、refresh token、scope、到期元数据和请求上下文。state、凭据、TokenSource::Rv 与旧接管归属在同一 SQLite 保存事务完成；失败保留原凭据与来源。成功后允许 RV 刷新，但不立即强制刷新。启用中的 Token 等待任务随新凭据唤醒，用户已停止的任务与 cron 保持原样。

刷新备份列表仅更新备份目录和设备关联；已有账号的 state、凭据、来源和任务不变。新发现账号缺少 state 时，首次导入仍默认手机来源。普通账号 state 同步继续保留 RV 当前凭据，不能代替明确交接。

验证：core 归属／停止／最终令牌导出／重启持久化测试，HTTP2 分块顺序和读回损坏／来源不符测试，原生分块／错误哈希／身份／权限／data 保留测试，GPUI 确认取消保留 RV 与店铺测试。手机4.0-12同内容回写全链路通过，手机接管标记验证使用隔离数据库，未切换真实 RV 账号模式。

## 2026-10-30 真机往返验证

本轮使用现有 RV 数据库（预先 SQLite backup，升级到 schema14），目标为已认证的 iPhone7Plus / iPhone9,2 / iOS15.8.8。手机安装正式4.0-15包并核对上传 SHA；未发布仓库、未构建或安装 RV GUI。

通过生产 SessionHandle / HTTP2 / WorkspaceStore 验证当前环境备份→RV采用完整 token 对→刷新 HTTP200→店铺资料 HTTP200→state/manifest交回手机并读回校验→Shadow UI 恢复→Vinted 登录资料仍为 michelley299→保存手机最新备份→交回 RV→再次刷新 HTTP200。两次成功刷新都返回不同 access/refresh 对并保存在现有 SQLite；最后新 access token 查询商品 HTTP200，商品结果落库且手动同步未修改 Tasks。未重放旧 refresh token 来试探失效，也不能据此声称 token 永久有效。

非 active 验证：Shadow 当前为另一 IDFV 时，读取目标 C148FA97…F420 自己的 state 并采用其 token 对，通过指纹比较确认没有使用 active 环境凭据。Shadow 不允许恢复当前 IDFV；切换环境时明确关闭先保存当前数据，避免 live 旧凭据覆盖刚由 RV 回写的备份。未修改 Shadow 代码。

本轮修复两个生产问题：交给手机成功后原子切换 TokenSource::Phone，避免仅改变旧 execution；RV采用手机备份最外层用 BEGIN IMMEDIATE，避免手机自动 token 写入与先读后写事务竞争产生 database is locked。回归246项、rv-app类型检查通过。最终目标来源与 execution 均为 RV，手机 Vinted 已停止，任务保持停止；最新轮换凭据只由 RV 使用。历史备份仍存在，不能宣称物理上没有旧 token 副本。
