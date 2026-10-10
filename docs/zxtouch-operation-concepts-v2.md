# RV ZXTouch 操作工具设计 v2

使用内置 imagegen 生成；尚未实现 UI。根据用户反馈，改为操作入口与参数表单，取消状态看板。

- App 操作：目标 App、打开/停止/重启、保存当前环境、选择备份切换、创建/切换空白基线、备份编辑删除入口。
- Apple 操作：登录/退出、修改姓名、转发邮箱、停止 App 授权、移除其它设备；选择动作后展示参数表单。

实施细节：已有备份的 save 会更新关联快照，表单须遵循原生语义。restart 使用既有 App 控制操作；App 状态保存/切换后保持停止。退出须明确选择本机数据保留策略。验证码仅在真实 challenge 后输入。执行时互斥冲突操作，返回未知结果时不自动重试。

## 最终提示词：App 操作

```text
Use case: ui-mockup. Generate a new high-fidelity RV macOS desktop UI concept, landscape 16:10, flat screenshot, razor-sharp readable Simplified Chinese. IMPORTANT DESIGN CORRECTION: user wants DIRECT FUNCTION OPERATIONS, NOT dashboards or account/environment status displays. Do not show status cards, snapshot inventory tables, recent operations history, app version, dates, battery, account summaries or read-only metrics. This is a compact execution tool beside a real remote phone, not a management console.
Use reference only for RV dark charcoal colors, small blue accents, compact titlebar/toolbar. Completely replace management layout. Titlebar "RV · 我的 iPhone". Compact standard remote toolbar with zoom, keyboard, file transfer, Home, screenshot, lock, small app shortcut icons; top-right selected "操作工具". LEFT 60% canvas: centered large live portrait iPhone showing Vinted app profile page with simple generic content, surrounding dark canvas. RIGHT 40% compact tools drawer, header "操作工具" with close icon. Two main tabs "App 操作" selected and "Apple 操作". No persistent "设备管理" dashboard.
App operations drawer content:
A single compact labeled dropdown "目标 App" showing Vinted with small turquoise icon, and three small inline action buttons "打开", "停止", "重启".
Below separator section heading "保存 App 环境". Text field label "备份名称" prefilled "工作账号 A"; optional one-line field "备注" placeholder "可选"; full-width primary blue action "保存当前环境". Small explanatory line "停止 App 后保存数据，完成后保持停止。"
Below separator section heading "切换 App 环境". Label "目标备份" with dropdown value "工作账号 B". A single checkbox checked "切换前保存当前环境". Compact label beneath "当前环境名称" with field "工作账号 A". Primary action button "保存并切换". This is an operation form, NOT a table of snapshots. No listing all backups on screen.
Below separator section heading "空白环境". One outlined action button with refresh/clean icon "创建空白基线", second outlined action "切换到空白基线". Small neutral line "基线对应当前安装的 App 版本。"
Bottom collapsed disclosure row "编辑或删除备份" with chevron, no expanded CRUD table. Keep all visible controls consistent 30-36px, restrained blue only on active and main action, charcoal fields, 1px separators, crisp SF-style Chinese typography. Layout is simple, practical, direct, immediately clear what to click. No marketing copy, no static service-state cards, no big empty dashboard tiles, no operation history, no phone frame hardware bezel if possible (show live framebuffer as straight portrait screen), no separate ZXTouch toggles or protocol names. Show only executable functionality.
```

## 最终提示词：Apple 操作

```text
Use case: ui-mockup. Create a companion RV Apple OPERATIONS screen, matching reference's native charcoal macOS top toolbar, large left real remote phone framebuffer and compact right action drawer. Landscape 16:10, flat screenshot, sharp readable simplified Chinese UI. Primary requirement: executable function controls, NEVER a status dashboard. No account status tiles, connected service summaries, recent operation tables, inventory tables or readonly account metrics.
Keep title "RV · 我的 iPhone" and compact zoom, keyboard, files, Home, screenshot, lock, app shortcut icons, selected top-right "操作工具". Left 60% dark canvas with large portrait iPhone live framebuffer showing a simple App Store browsing page (app listings and icons), no drawn phone hardware bezel. Right 40% drawer header "操作工具" and close icon; tabs "App 操作" and selected "Apple 操作".
Right panel action layout:
Heading "登录与退出", a compact two-column action button grid: selected blue-tinted "登录 App Store", outlined "登录 iCloud"; second row outlined "退出 Apple 账户", "退出 iCloud". These are functional action launchers, not state cards. Small icons optional.
Next heading "账户与授权", compact 2-column grid of outlined buttons: "修改姓名", "设置转发邮箱", "停止 App 授权", "移除其它设备". Each opens its own focused parameter form. No displayed user identity data outside action form. Do not include unsupported password change or mass account pool.
A thin divider, then clearly selected action parameter form heading "登录 App Store", small dismiss/back icon on right. Field "Apple ID" placeholder "输入邮箱"; field "密码" masked characters. A single primary blue button "登录 App Store". A muted helper "需要验证码时会提示输入。". Do not show six-digit boxes before an actual challenge. No auto-code URL, no saved credential pool, no one-click automatic switching account.
A collapsed bottom action row "更多操作" with chevron, no status cards. Optional tiny footer text "操作结果在执行后显示" is acceptable but no historical logs. Everything is compact, 30-36px controls, precise alignment, 1px dividers, restrained blue highlight and main action, subtle gray field fills. Focused native tool UI, practical not decorative. Preserve phone visibility, no oversized management title, no protocol task IDs or ZXTouch mode switch. Forms are for direct execution of existing native Apple service methods.
```

