# RV 管理界面实现与模块组织

按 [第二版设计](management-ui-concept-v2.png) 实现。移除常驻底栏和重复设备信息，连接按钮位于选中/悬停的卡片或列表行内；编辑、复制、删除统一进入“…”菜单。预览完整等比缩放，保存的画面标注“上次画面”。没有截图时显示占位图标，隐藏预览设置仍生效。

工具栏只包含分组开关、连接搜索、网格/列表、设置和新增连接，按钮高度 28px。展开侧栏宽度 176px；网格按窗口宽度分配一至三列，窄窗口可滚动。保留双击/回车连接以及原有快捷键、密码文件、标签和最近使用逻辑。连接簿搜索不涉及文件搜索。

移除底栏后的反馈显示在内容区或对应弹窗内，避免地址错误和凭据加载失败变得不可见。

## 文件职责

| 文件 | 职责 |
| --- | --- |
| `crates/rv-app/src/app.rs` | 应用状态、连接簿、事件处理、凭据与会话流程 |
| `app/layout.rs` | 标题栏与各区域组合、全局动作绑定 |
| `app/toolbar.rs` | 搜索、视图切换、新增、设置、按需反馈 |
| `app/sidebar.rs` | 全部连接、最近使用和标签筛选 |
| `app/connections/mod.rs` | 网格与列表集合布局 |
| `app/connections/card.rs` | 卡片、缩略图、选中及悬停操作 |
| `app/connections/row.rs` | 列表行 |
| `app/connections/menu.rs` | 共用连接操作菜单 |
| `app/connections/widgets.rs` | 连接按钮与标签展示 |
| `app/connections/empty.rs` | 空连接簿与搜索无结果状态 |
| `app/dialogs/mod.rs` | 弹窗容器和路由 |
| `app/dialogs/connection.rs` | 连接表单和连接选项 |
| `app/dialogs/preferences.rs` | 全局偏好设置 |
| `app/dialogs/widgets.rs` | 表单字段与选项行 |
| `app/ui_tests.rs` | 用户交互与布局回归测试 |
| `app/visual_tests.rs` | 可选的原生 Metal 渲染导出 |

修改某一区域时在对应模块内完成；持久化和连接行为仍由 `AddressBookApp` 的处理方法负责。卡片与列表共用菜单和连接按钮，避免两套业务规则。

## 验证

- `cargo test --workspace --locked -- --test-threads=1`：159 项通过，视觉导出测试默认忽略。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`、格式检查和应用构建通过。
- 新增交互测试覆盖网格/列表菜单编辑正确设备、800px 与 1100px 网格布局、28px 卡片内按钮和移除底栏后的表单反馈。
- 独立执行 `cargo test -p rv-app --locked export_management_previews -- --ignored --test-threads=1`，通过原生文本排版与 Metal 渲染器导出下列实际 UI 图。棋盘格是测试缩略图，用于检查横竖比例，不是真实设备画面，也不使用用户连接簿。

[深色网格](management-ui-implemented-dark.png) · [浅色网格](management-ui-implemented-light.png) · [800px 列表](management-ui-implemented-compact.png)
