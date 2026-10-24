# 随机触摸轨迹测试

VNC 工具栏的 move 图标点击执行一次完整手势，完成后自动抬起；无需再次点击。
执行期间可再次点击以提前取消。单次执行，不循环。
轨迹选择、变换和时钟调度位于 RV 的 `rv-session::touch_gesture`。手机复用已有
`/api/device/screenshot` 和 `/api/device/touch`，不新增服务端协议，不回退到 VNC。

每次先读取手机原生截图尺寸，包含当前横竖屏和显示缩放；不使用 RV 窗口或缩小后的
VNC framebuffer。起终点位于画面中间区域。按方向和距离选择适合的录制轨迹，再整体
旋转、缩放和平移。任一点越界就换候选，不把越界点裁到屏幕边缘。保留轨迹相对时间，
完整时钟缩放到 300–1500 ms，并按录制间隔权重重采样到名义 60 Hz，避免缩放时长后
事件过密或过稀；位置按原轨迹时间插值，保留速度变化与停顿。屏幕内只表示合法坐标，
不表示该位置无按钮。

录制库来自固定版本的 Cursory，保留未修改的数据、来源、哈希和许可。这是录制回放适配，
不是完整 Cursory 生成器的逐种子一致移植，也未额外添加其噪声扰动。鼠标记录不等于真实手指行为，
也不证明任何第三方接受结果。macOS 打包附带许可与来源；其他二进制分发同样须随附
`crates/rv-session/assets/cursory/` 中的许可、来源和可重建源代码。

触摸使用独立 HTTP/2 连接和当前认证，不新增密码存储。保持同一 finger，严格按
begin → move → end 顺序发送，并等待每次回复。以按下确认后的单调时钟为基准调度，
不在每次回复后重新等待完整间隔。实际投递节奏以手机 Probe 的事件时间戳为准。

悬停不取消；主动点击、滚动或按键取消手势，并阻止本次输入和触摸重叠。失焦、切换
操作区域、画面尺寸变化和断线也停止。取消、请求错误和超时均尝试 end，最后关闭本次
连接触发手机的断线清理。不自动重放 begin 或失败的 move。界面忙碌状态保留到操作
结束，并按请求 ID 忽略过期结果。HTTP ACK 不代替 UIKit 回调或界面结果验证。

## 验证

- `cargo test -p rv-session --locked touch_gesture`
- `cargo check -p rv-app --tests --locked`：只做类型检查，不构建或打包 rv-app。
- `python3 scripts/check-ui-style.py`

显式真机测试 `live_sdk_probe_single_gesture` 默认 ignored，需独立运行并提供环境变量
`RV_TOUCH_HOST`、`RV_TOUCH_PORT`、`RV_TOUCH_PASSWORD`。它校验前台为
`local.sdkprobe.app`，执行一次触摸并撤销自身认证；实际 UIKit 投递还需检查 Probe。

2026-10-24 真机观察：`10.17.12.17:6000` 转发的手机原生截图为 1170×2532。
一次完整轨迹的 begin/move/end 均获得 HTTP 成功确认，但之后 Probe 画面仍为 0 条触摸回调。
现有 `/api/device/tap` 对照也返回 `{ok:true}`，随后截图没有观察到输入框焦点或回调变化。
因此不能将这次传输通过记作触摸生效；需继续区分手机注入与 Probe 采集问题。
