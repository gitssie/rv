# CRUD 界面风格统一

设计图由内置 `imagegen` 工具生成，沿用 `management-ui-concept-v2.png` 的主界面风格。

- [设计参考图](management-ui-crud-concept-v1.png)
- [新增实际渲染](management-ui-crud-new.png)
- [编辑实际渲染](management-ui-crud-edit.png)
- [删除实际渲染](management-ui-crud-delete.png)
- [浅色设置实际渲染](management-ui-crud-settings.png)
- [浅色新增实际渲染](management-ui-crud-new-light.png)
- [浅色编辑实际渲染](management-ui-crud-edit-light.png)
- [浅色删除实际渲染](management-ui-crud-delete-light.png)
- [深色设置实际渲染](management-ui-crud-settings-dark.png)

新增、编辑采用同一表单，左侧标签、右侧输入框；操作区使用 28px 高按钮：取消、保存、保存并连接。设置复用同一行布局和蓝色主按钮；删除确认使用较窄弹窗和危险操作颜色。对话框沿用主题表面、边框、文字颜色，宽度限制在窗口内，较矮窗口支持滚动。配置选项保持现有点击循环行为，设计图中的箭头不代表新增下拉组件。

实现仍在 `app/dialogs/` 按内容模块组织。界面文案本地化不改变枚举值、保存格式或密码存储方式。

实际渲染通过独立临时连接簿和原生 Metal 渲染器导出，背景缩略图为合成测试数据，不读取用户设备或配置。

生成提示词：

```text
Use case: ui-mockup. Asset: RV native macOS connection manager CRUD dialog design reference sheet. Input image is STYLE REFERENCE only, retain its compact graphite dark surfaces, fine borders, subtle corners, blue accent, Chinese interface, restrained native typography. Generate one wide high-fidelity sheet with three separate dialogs at equal natural scale on subdued charcoal background, clearly labeled 新增连接 / 编辑连接 / 删除连接. New and edit dialogs width480 logical px, padding20, label12px, input28px, 12px row gap, header16px semibold. Exact form labels VNC 地址, 名称, 用户名, 密码, 标签 in single-column full-width input rows. Edit sample values 192.168.1.26:5900, 测试 iPhone, blank username, blank password and caption 留空则保留已保存的密码。, 标签 测试. Below separator compact right aligned option rows 加密方式 / 画面质量 / 剪贴板编码 / 本地光标 with small controls, then small switches 记住密码 / 仅查看 / 共享会话. Bottom aligned right buttons 取消 (neutral), 保存 (outline), 保存并连接 (solid blue #3389f5); all28px height, compact. Delete dialog narrower420px, title 删除连接, text 确定删除“测试 iPhone”吗？, muted text 已保存的密码和画面预览也会一并删除，此操作无法撤销。, actions 取消 and small red 删除. Panels #20272f borders #303944, inputs subtle dark #171c22. No oversized buttons, decorative gradients, extra features, footer bars, browser chrome. Legible polished Chinese text. All panel contents and bottom buttons fully visible.
```
