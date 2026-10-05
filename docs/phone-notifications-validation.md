# 手机通知与工具栏开发验证

日期：2026-10-05

## 已实现

- 28px 紧凑工具栏，保留 App 快捷图标，新增 Home、截图菜单和通知入口。
- PNG 截图复制/保存远程 framebuffer，不包含本地工具栏。
- 已建立连接发生临时传输错误时自动重连，可立即重试或停止；不自动重放解锁及 App 操作。
- SpringBoard BBServer 接收的手机通知通过 TrollVNC 的现有 RFB 扩展推送到 RV，默认订阅、可暂停、可手动刷新。
- 最近 100 条内存缓存；长度及报文上限、订阅串行化、读权限与旧客户端兼容。
- 通知面板打开时键盘输入保持本地。原有密码文件继续复用。

## 本地检查

- `cargo test --workspace --locked -- --test-threads=1`：156 项通过（rv-app 67、rv-core 32、rv-session 57），含 GPUI 面板输入、通知真实 RFB 传输、截图颜色及重连策略测试。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo fmt --all -- --check`、`cargo build --workspace --locked`：通过。
- vendored vnc-rs 的 9 项库测试通过；其原有文档示例测试缺少 anyhow/minifb/tracing_subscriber 依赖，未修改该示例。
- TrollVNC App management policy 测试通过；arm64 服务端、通知适配器及 RootHide DEB 构建通过。使用 Theos RootHide / iPhoneOS 16.5 / 最低 iOS 15；构建参数 `ADDITIONAL_CFLAGS=-Wno-error=deprecated-declarations` 保留原有弃用 API 告警。
- DEB 内已核对包含 TVNCNotificationAdapter.dylib 和仅 SpringBoard 的注入过滤文件。
- 代码复审后无明确阻断问题。

## 尚需真机验证

未安装或重启手机。本地测试不能证明每个 iOS 版本的 BBServer hook、RootHide socket 可达性和 SpringBoard peer 验证实际生效。需要手机验证 App 通知、BBServer 系统通知、内容隐藏时的表现、暂停/重新开启、断线重连和返回桌面。

缓存是适配器启动后收到的通知，不导入通知中心旧历史、不跟踪通知删除；不经 BBServer 发布的系统弹窗不属于该通道。模拟器及 app-only THEBOOTSTRAP 构建不含通知适配器。
