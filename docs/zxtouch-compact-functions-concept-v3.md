# RV 紧凑功能菜单 v3

使用内置 imagegen 生成，尚未实现 UI。

设计方向：工具栏单一“功能”入口；弹出分类菜单，包含搜索、常用收藏和紧凑操作项；仅在选择操作时展开小型参数表单。新增功能增加分类或操作项，不增加常驻大按钮与展示卡片。

建议实现尺寸：工具栏与菜单项高 26–28px；功能面板宽 360–400px，长列表滚动；参数表单按所需字段定宽，避免覆盖手机画面。生成图用于风格与结构参考，不作为精确尺寸规范。

## 最终提示词

```text
Use case: ui-mockup. Create a radically corrected RV desktop functional operations UI proposal as a realistic flat macOS app screenshot, landscape 16:10, sharp Simplified Chinese typography. USER'S EXACT DESIGN GOAL: SMALL compact functional command items, extensible to MANY future functions. NOT a panel full of large buttons or permanent forms. Think native application menu / IDE action browser: tight rows, categories, search, optional submenus. Never use oversized blue buttons, big cards, large section gaps or always-visible account form.
Keep RV's dark charcoal native UI from reference, thin titlebar "RV · 我的 iPhone", compact top toolbar with fit, keyboard, files, Home, screenshot, lock, Safari/Photos/Settings small icons. Main area almost all live phone workspace: a centered big portrait remote iPhone framebuffer showing a neutral iOS home screen with app icons, straight screen edges without phone hardware bezel, dark surrounding canvas.
Top-right toolbar has a tiny ordinary 28px high icon button "功能" (wrench icon), currently open. Anchored below this toolbar button is a COMPACT floating command browser, only about 390 pixels wide and 470 pixels tall in a 1440x900 app window, positioned at far right over empty canvas, never a full-height sidebar. Native popover with restrained shadow and subtle border. This popover is the whole new feature UI.
Popover:
top a 28px search field with magnifying glass placeholder "搜索功能…".
Below, two-column compact browser. Left narrow 90px category list: "常用", selected "Apple", "App 环境", "脚本", "识别", "设备". Tiny monochrome line icons. Rows 28px, no card borders; selected row subtle blue-gray fill.
Right main command list approx 265px width, grouped using TINY muted section headings and separators, not cards:
"登录与退出" tiny heading, rows with small icons, 28px height:
"登录 App Store" (trailing chevron)
"登录 iCloud" (trailing chevron)
"退出 Apple 账户" (trailing chevron)
"退出 iCloud" (trailing chevron)
thin divider.
"资料与邮箱" tiny heading, 28px rows:
"修改姓名" chevron
"添加联系邮箱" chevron
"验证联系邮箱" chevron
"设置转发邮箱" chevron
thin divider.
"授权与设备" tiny heading, rows:
"停止 App 授权" chevron
"移除其它设备" chevron.
Each command row has a tiny outline star at far right or on hover to pin to 常用, only one hover row "设置转发邮箱" subtle highlight to illustrate interaction. No repeated primary buttons. No account status, email summary, operation history.
Popover bottom tiny help line "选择功能后填写参数", muted. A very small gear icon bottom-right for customizing pinned functions, no protocol toggle. List supports scrolling, minimal unobtrusive scroll indicator if needed. No fake unsupported new functions, no keyboard shortcuts invented.
Also illustrate an anchored compact submenu for highlighted "设置转发邮箱", immediately to the LEFT of command browser, ONLY about 250px wide x 160px high. It contains title "设置转发邮箱", two dense labeled single-line inputs "Apple ID" and "转发邮箱", small ordinary 26px buttons "执行" (small blue) and "取消" at footer, not full width. This submenu is the focused argument editor, subordinate to compact command list. Fields empty with placeholder "邮箱". No large authentication form, no modal blanket over phone, no dominant blue CTA, no UX dashboard.
Everything fits compactly on the right-side empty canvas leaving live phone visible. 12-13px UI labels, precise compact spacing, native menu-like interaction, minimal blue. Critical: make the actions look like menu ROWS not rounded large action buttons. The tool's architecture is extensible categorization + search + favorites + per-action small forms. Match reference colors only; completely abandon reference's large side-panel layout. Output a polished practical app screenshot, no annotated presentation board or marketing text.
```

