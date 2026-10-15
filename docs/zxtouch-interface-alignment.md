# ZXTouch 返回字段对齐清单

本清单记录后续手机端开发事项。状态基于本地源码检查，不代表真机安装包已具备这些字段。

## 身份与来源

| 对象 | 唯一身份 | RV 主键 | 来源 |
| --- | --- | --- | --- |
| 手机 | 完整真实设备 GUID | 本地 `id` | ZXTouch 设备信息，GUID 待补充 |
| 虚拟 App | 备份捕获的真实 IDFV | 规范格式的 IDFV 本身 | 真机 ZXTouch App snapshot 备份 |
| 备份引用 | 设备、Bundle、`backup_id` | 关联键 | App 备份列表 |

同设备、同 Bundle 可以存在多个不同 IDFV 的虚拟 App。同 IDFV 的多份备份或重复同步只产生一个业务对象。IDFV 不可用来识别手机。不存在文件导入、手工建卡或随机生成 IDFV 的入口。

## 当前接口与待补项

| 接口或业务 | 源码现状 | 后续对齐要求 |
| --- | --- | --- |
| `/api/device/info` | 新源码返回结构化信息及真实 `guid`、`guid_source`、`guid_status`；旧 task 30 保持历史语义 | 验收 MobileGestalt 读取权限与真实设备 GUID。读取失败留空，不把旧 IDFV 当作 GUID |
| `/api/app-state/list` 的完成结果 | `bundle_id`、`backups`、`current_backup_id`、`restore_pending`；冲突时可含 `duplicate_identity_backup_ids` | 确认真机安装版本与源码一致，保持 Bundle 和完整列表语义明确 |
| `backups[]` | `backup_id`、`kind`、`display_name`、`note`、`version`、`build`、`idfv`、`date` | snapshot 提供真实 IDFV。`idfv` 为 null/无效时显式标记缺失，不由 RV 猜测或生成 |
| baseline | 空白环境，不包含既有账户身份 | 保留为可用的手机环境操作，不生成店铺卡片 |
| 当前环境 | `current_backup_id`、`restore_pending` | 不从“最近备份”推断当前环境；恢复未完成或身份冲突时保留未知状态 |
| 备份对应的店铺/账号 | 列表目前只有名称、备注等备份元数据 | 按 Bundle 策略补充从备份读取业务身份的控制入口；Vinted 的用户标识、登录名、市场等需约定返回结构。名称不能充当账号唯一身份 |
| 脱机业务检测、消息、会话和任务 | RV `rv-business` 按 Bundle 选择策略并直接调用业务 HTTP；SQLite 存储任务和加密凭据 | 不存在手机端业务任务入口；会话检测、消息与未读统计已有源码和本机测试，真实服务验收待完成 |

设备 GUID 和账号资料读取的新源码已接入，安装包与真机权限仍待验收。上表的历史接口状态需要与安装版本逐项核对。

## 脱机任务的执行边界

脱机任务由 RV 本机的 `rv-business` 模块执行，各 App 按 Bundle 注册业务策略，直接使用账号 access token 调用业务 HTTP 接口。该模块不依赖 rv-session，不要求手机连接、打开 App 或恢复备份。任务和结果保存在本地 SQLite，凭据加密保存；不会向 zxtouch 发送业务任务。

手机的 `/api/app-state/profile` 仅负责读取选定 snapshot 的账号资料。RV 刷新凭据不会改写原始备份；恢复时的独立 plist 补充与合并属于手机备份恢复流程，不能承担账号检测、消息、会话或通知请求。

## RV 的缺失值处理

- GUID 缺失时设备 GUID 留空；保留本地主键，不使用 IP、IDFV 或随机 UUID 冒充 GUID。
- snapshot 缺少 IDFV 时保存完整原始返回，暂不创建虚拟 App；再次同步提供有效 IDFV 后才能建立业务对象。
- 从同一 IDFV 再次读取备份，保留本地标签、备注及策略业务元数据，不以手机备份备注覆盖用户编辑。
- 备份列表的完整原始返回及额外字段存入 SQLite，避免只留下 UI 使用的少数字段。
- 店铺会话、未读数和任务未返回时展示未知，不提供模拟结果，也不自动打开 App 或恢复备份。
