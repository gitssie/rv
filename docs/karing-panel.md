# Karing 控制面板

快捷图标左键打开 App，右键打开控制浮层。所有控制走既有认证 HTTP/2；
不借用 VNC 或旧 TCP 作为回退。界面使用 GPUI Component 的按钮和主题。

## 出口与链式代理

支持新能力 `karing_node_control` 的 daemon 会读取原生 profiles 和 VPN 状态。
订阅标识、显示名称与节点均来自手机，不能写死测试用的 M4、FR 或节点标签。

- 单节点模式：全部已启用配置中的真实节点都可以作为出口，应用时清除旧链。
- 链式模式：用户选择前置订阅与最终出口；从该订阅排除出口自身，逐个测速，
  选取成功且延迟最低的节点作为中介。延迟 0 或失败不是成功候选。
- 测速前先清除旧全局 detour、保留原出口，避免旧中介使候选测速失真。
  测速全部失败或传输中断时尝试恢复原选择；无法确认恢复则要求重新读取。
  既有多跳/后置链无法无损恢复，须先明确应用单节点出口。
- 切换同时保存原生 setting、subscribe_use 与 service_core，保留未知字段、
  节点凭据和不属于所选默认节点的规则；只更新已识别的默认路由规则和 DNS。
  旧 final 必须为已确认的真实选中节点；内置或自定义默认出口不猜测改写。
- 原生重载必须观察到新的 core startTime；面板继续读取实际 GLOBAL.now，
  核对最终出口才显示切换完成。超时和未确认修改不会自动重放。

原生写入前仅停止 Karing 主程序，防止 App 内存中的旧设置覆盖文件；
独立 packet tunnel 保留。下次打开 Karing 会重新读取保存的设置。

## 接口与兼容

| 操作 | 认证 HTTP/2 请求 |
| --- | --- |
| 原生订阅/节点 | `POST /api/karing-native/profiles` |
| VPN 状态/连接/断开 | `POST /api/karing-native/connection|connect|disconnect` |
| 应用选择 | `POST /api/karing-native/select`，exit_tag、relay_tag、upstream_group |
| 运行节点/当前出口 | 原始 `GET /api/karing/proxies` |
| 延迟测速 | 原始 `GET /api/karing/proxies/{name}/delay?url=…&timeout=5000` |

raw 请求限时 15 秒，原生 helper 请求限时 60 秒；回复最多 32 MiB。
取消不能撤销已经提交的操作。配置响应含私密节点信息，不写日志。
不支持新能力的 daemon 保留原只读节点面板和真实 HTTP provider 刷新。
当前手机没有 Clash providers，**App 原生订阅下载刷新尚未接入**，不能将
重新读取缓存叫作订阅刷新，也不能将 disabled 刷新按钮说成可用功能。

## 2026-10-27 验证

通过 Wi-Fi6000 的已认证 HTTP/2 shell 调用同一签名 production helper：
单节点切换确认后，对当前所选订阅六个节点独立测速，171/173/179/68/184/166 ms；
以 68 ms 候选为中介、所选出口为最终出口，重载和 raw GLOBAL 都确认。
两次选择分别约 0.689/0.795 秒。经 Karing mixed 入口的 HTTPS 公网请求成功。
手机端已部署 4.0-7，并通过正式 `/api/karing-native/select` 验证另一 M4 节点
作为单节点出口、再恢复最快中介 → FR；耗时约 0.684/0.293 秒。
正式 connection/connect 接口确认 VPN 仍连接。这不代表运行中旧 RV GUI 已更新。

原生 Documents/Client 回归、rv-session Karing 模块测试、独立快照的
`cargo check -p rv-app --tests` 和 UI 样式检查通过；rootless 手机包构建通过。
共享工作区的完整检查目前受其它未完成的 automation API 变更影响，独立快照
只覆盖已提交基线加本次 Karing 代码。按 AGENTS.md 未构建或打包 RV。
