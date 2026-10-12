# Karing 紧凑控制面板

Karing 快捷图标左键保留打开 App；右键打开控制浮层。默认布局不超过
320 × 170 逻辑像素，跟随 RV 工具栏主题。更多配置和测速结果按需在浮层内
展开，选择配置后收起；不会打开第二个浮层。

RV 直接通过已有 HTTP2 `/api/karing/*` 代理调用管理端口，不新增手机端接口。
面板请求保留 HTTP 状态、响应头与原始响应字节；仅展示层读取所需 JSON 字段。
每次有限请求最多读取 4 MiB、15 秒超时，不自动重发。关闭浮层或控制断连会
取消请求并停止后续测速；取消不能撤销已经到达手机的修改。

| 操作 | Karing 原始请求 |
| --- | --- |
| 读取配置与节点 | `GET /providers/proxies`、`GET /proxies` |
| 刷新所选 HTTP provider | `PUT /providers/proxies/{name}` |
| 节点延迟测速 | `GET /proxies/{name}/delay?url=…&timeout=5000` |

配置标签来自实际返回的 providers，选择标签决定刷新与测速范围，**不代表
切换 Karing App 的「我的配置」**。没有 providers 时展示实际节点数量，
禁用订阅刷新，仍可对运行中的真实节点逐个测速（排除 Direct/Reject/代理组）。
当前节点从 `/proxies` 的 `GLOBAL.now` 读取；选中代理组时沿其 `now`
解析到真实节点，不把 URLTest 的结果冒充 GLOBAL 的选择。
延迟结果保留节点名称，并分别统计成功和失败。更多 provider 可展开选择。

当前已核实管理端口没有 App 配置切换、链式代理修改接口，链式控件禁用；
不将 `connections[].chains` 当成可写链式配置，不编造成功返回。VPN 开关不做。
标题状态表示管理接口可用性，不表示系统 VPN 状态。

保留验证：`cargo test -p rv-session --locked` 与
`cargo test -p rv-app --locked -- --test-threads=1`。新增 3 项 HTTP2 请求测试
覆盖方法/编码路径/查询/请求体/错误返回原样传递及取消；5 项 GPUI 测试覆盖
紧凑尺寸、空配置、provider 刷新失败、只读、关闭取消、长名称更多配置点击、
结果展开及断连停止队列。这些检查不代表手机上已完成新面板的写入验收。

构建只生成 `target/debug/rv`，不打包 `.app`。
