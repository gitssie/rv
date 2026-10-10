# App 优先的环境与备份设计 v5

使用内置 imagegen 生成的设计图：[设计图](zxtouch-app-first-concept-v5.png)。本次只调整设计方案，不修改 UI 实现。

交互顺序：常用 App 图标网格 → 选中 App 的操作与备份记录 → 按需打开简洁文本输入。

- 常用 App 由用户从手机已安装 App 图标网格添加、移除。
- 选中 App 后再展示打开、停止、保存当前环境、创建空白基线和备份记录。
- 每条备份直接提供切换；更多菜单提供编辑名称与备注、复制备份标识、删除。
- 参数输入只粘贴名称、备注等文本；目标 App 来自已选上下文，不输入 Bundle ID。
- 更多功能归入该 App 的更多菜单，备份列表可搜索、滚动。

## 最终生成提示词

```text
Use case: ui-mockup.
Create a high fidelity desktop RV client UI DESIGN BOARD, landscape, three clearly labeled related UI states side by side, showing a completely redesigned App environment / backups workflow. This is an App-first operational tool for Chinese users, NOT a dashboard. Render precise readable simplified Chinese, crisp desktop typography, compact macOS desktop proportions, usable native-looking components. Dark charcoal RV toolbar and popovers, subtle thin borders, blue accent, real colorful iOS App icons. The reference style is an existing RV toolbar with 24px app shortcut icons and a compact dark popover containing a four-column grid of 40px app icons and small names underneath. Keep the design aligned with this style. Background of presentation board neutral dark gray, modest section captions; no marketing decoration.

Layout: 3 panels of similar compact width, approximately 360 logical pixels each, generous enough to read text but tightly spaced functional UI, 28px action buttons, 12–14px body text. Panel 1 and panel 2 represent mutually exclusive main states of the SAME App environment popover; panel 3 is a small text-input popover over the selected App panel. This is a polished design image, no HTML, no source code.

Panel 1 caption: "01  选择常用 App". Show a cropped RV toolbar at top, home/camera/lock small line icons followed by colorful Safari, Settings, Vinted, App Store app icons and +. Below, a dark compact popover titled "App 环境与备份" with small close X. Search field "搜索常用 App". Section label "常用 App" and tiny text action "管理". Four-column icon grid in two rows: Vinted teal V icon, Safari, 微信, App Store, 设置, 照片, ProjectX, and outlined dashed plus tile labeled "添加 App". Icon and app name only, no status cards, no statistics. Vinted is highlighted with restrained blue focus border. Below grid a thin separator and quiet link "从手机已安装 App 中添加". Show an associated small inset/popover for adding apps: title "添加常用 App", search "搜索手机上的 App", a 4-column grid of installed app icons (Books, Calculator, Calendar, Camera), selected checkmarks on favorites, a tiny "完成" button. Make clear users choose actual installed apps and manage their own favorites, never type App bundle IDs.

Panel 2 caption: "02  选中 App 后操作". Same RV toolbar crop and main popover shell. Header small back chevron, 32px teal Vinted icon, title "Vinted", and small close X. Directly beneath header a compact row of actual functional actions "打开" / "停止" / "更多 ⋯", 26–28px high. Then thin divider and section "环境与备份". Small primary blue button "+ 保存当前环境", secondary text action "创建空白基线" (not giant tiles). Below, search input "搜索备份名称或备注". A dense list with 3 backup records, light dividers, not huge cards. Each record has a small folder icon, bold name, optional one-line note, small muted date metadata; each row has right-aligned small outlined "切换" button and ellipsis menu. Records: "工作账号 A", note "日常使用", timestamp "10月10日 10:20"; "备用账号 B", note "备用环境", timestamp "10月09日 18:45"; "空白基线", note "首次启动前", timestamp "10月08日 09:12". One record ellipsis menu visibly open with compact menu rows "编辑名称与备注", "复制备份标识", "删除备份"; deletion has understated red text and separate divider. Back at top always returns to common apps. No metrics, no account identities, no unnecessary current-status dashboard, no red giant buttons. Selecting record operations stays scoped to Vinted.

Panel 3 caption: "03  一次粘贴保存资料". Show selected Vinted panel faintly behind a small anchored input popover, NOT a full-screen form. Title "保存当前环境", subheader small Vinted icon and "Vinted" to show the already selected target App. A single light-on-dark monospace multiline text box with exact two lines:
"名称=工作账号 C"
"备注=备用环境"
No Bundle ID input, no separate name and note form fields, no app dropdown. Small helper line "首次保存需填写名称；已有环境可直接保存。" Footer compact blue text "复制示例" on left, two small buttons on right "取消" and blue "保存". Below this state use a subtle caption "选 App → 选择操作或备份 → 按需粘贴资料". Preserve compactness and extension potential: more operations can live in 更多 menu; backups grow as a scrollable list, no giant action button grid.

Critical: users first choose an App from customizable favorite icon grid, THEN see that App's operations and backup records. Do not repeat the rejected design of choosing a global operation first then asking for App in a form. All panels must have high contrast text, consistent dark theme, restrained desktop iconography, realistic clickable small controls. No large operation tiles, no input bundle identifiers, no statistics/status display panels.
```

