# ZXTouch 返回字段对齐清单

本清单区分当前返回与后续开发事项。2026-10-30 已只读核对真机备份目录；其它仅源码检查的条目不代表已做真机验收。旧 App-state 协议代码不能作为现行 HTTP 能力依据。

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
| `/api/app-state/list` 的完成结果 | 4.0-15 返回 `bundle_id`、`format=ShadowAppState2`、`backups`；4.0-16 增加 `current_idfv` | 不要求不存在的 active、pending 字段；交接直接校验所选备份的 manifest 和 state |
| `backups[]` | `backup_id`、`kind`、`display_name`、`note`、`version`、`build`、`idfv`、`date` | snapshot 提供真实 IDFV。`idfv` 为 null/无效时显式标记缺失，不由 RV 猜测或生成 |
| 备份创建／恢复 | 当前由 Shadow 执行；旧 `save`／`switch` 接口已移除 | RV 不重新接入旧备份、切换或基线创建操作 |
| 当前环境 | 4.0-16 目录独立提供 `current_idfv`，不读 Keychain、不依赖 token；没有身份时显式 null，无效身份报错 | 不从“最近备份”推断；检测到同一 IDFV 时提示自行备份并切换；查询失败不当作非 active |
| 备份对应的店铺/账号 | 列表目前只有名称、备注等备份元数据 | 按 Bundle 策略补充从备份读取业务身份的控制入口；Vinted 的用户标识、登录名、市场等需约定返回结构。名称不能充当账号唯一身份 |
| 脱机业务检测、消息、会话和任务 | RV `rv-business` 按 Bundle 选择策略并直接调用业务 HTTP；SQLite 存储任务和加密凭据 | 不存在手机端业务任务入口；会话检测、消息与未读统计已有源码和本机测试，真实服务验收待完成 |

设备 GUID 和账号资料读取的新源码已接入，安装包与真机权限仍待验收。上表的历史接口状态需要与安装版本逐项核对。

## 脱机任务的执行边界

脱机任务由 RV 本机的 `rv-business` 模块执行，各 App 按 Bundle 注册业务策略，直接使用账号 access token 调用业务 HTTP 接口。该模块不依赖 rv-session，不要求手机连接、打开 App 或恢复备份。任务和结果保存在本地 SQLite，凭据加密保存；不会向 zxtouch 发送业务任务。

RV 经 `/api/app-state/download` 读取所选 snapshot 的 manifest 与 state；创建和恢复手机备份仍由 Shadow 执行。普通同步不覆盖 RV 已更新凭据，明确交接／修复才采用手机备份凭据。手机备份通道不能承担业务账号检测、消息、会话或通知请求。

## RV 的缺失值处理

- GUID 缺失时设备 GUID 留空；保留本地主键，不使用 IP、IDFV 或随机 UUID 冒充 GUID。
- snapshot 缺少 IDFV 时保存完整原始返回，暂不创建虚拟 App；再次同步提供有效 IDFV 后才能建立业务对象。
- 从同一 IDFV 再次读取备份，保留本地标签、备注及策略业务元数据，不以手机备份备注覆盖用户编辑。
- 备份列表的完整原始返回及额外字段存入 SQLite，避免只留下 UI 使用的少数字段。
- 店铺会话、未读数和任务未返回时展示未知，不提供模拟结果，也不自动打开 App 或恢复备份。
