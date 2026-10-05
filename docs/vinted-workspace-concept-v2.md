# 工作助手交互设计 v2

设计图：[vinted-workspace-concept-v2.png](vinted-workspace-concept-v2.png)。由内置 imagegen 生成，属于拟议 UI，并非已实现功能。

## 图中三个状态

1. 默认收起：手机画面使用整个操作区域。工具栏右侧保留一个小型「助手」入口。
2. 手动展开：点击「助手」后，右侧显示「工作助手」，手机画面保持比例并适应剩余空间。面板内的「×」或工具栏同一个按钮可收起面板。
3. 切换手机 App：通过原有快捷栏切换照片、Safari 或其他 App，右侧草稿继续保留，不随手机前台 App 改变。快捷栏的「＋」继续用于选择更多已安装 App。

助手属于 RV 本地界面，商品草稿、常用文案、通知是互斥页签。商品草稿中的标题和描述通过「发送到剪贴板」传到当前手机，用户在手机目标输入框粘贴。图片通过「导入手机相册」传输，再在手机中选择。发布仍在手机 App 内完成。价格和库存备注属于本地草稿，界面不暗示平台数据已经同步。

图中窗口表示同一台手机在不同操作阶段的状态，不是多设备群控。手机内部页面和素材为示意，外部箭头与序号是设计说明，不进入产品界面。

## 生成提示词

```text
Use case: ui-mockup. Create a polished interaction-design presentation board for RV, a native Mac app remotely controlling one iPhone. Reference image is style reference only. Show HOW a local helper opens and survives switching iOS apps. Landscape wide high-resolution composition. Chinese UI, charcoal graphite native surfaces, blue #3389f5 actions, fine borders, compact controls. No marketing dashboard.
Layout: two rows. Top row contains TWO smaller complete native RV windows side by side, numbered captions OUTSIDE windows: "① 默认：助手收起" and "② 点击「助手」展开". Bottom row contains ONE larger wide complete RV window, caption "③ 切换手机 App，草稿继续保留". Add a thin single blue arrow between top windows, with tiny annotation "点击工具栏右侧「助手」". Plenty of readable space. All windows represent the SAME ONE PHONE at different times, not multiple devices.
Each window has title RV · 工作 iPhone, native traffic lights and one compact session toolbar. Toolbar LEFT: small Home icon, lock icon, screenshot icon; then a row of App shortcut icons with labels "Vinted", "照片", "Safari", plus small "+" to open the installed-app picker. Toolbar RIGHT: green connected dot and ONE small split-panel icon + text "助手". First window: helper button neutral outline and helper CLOSED, center ONE portrait phone screen showing illustrative Vinted Sell an item form with blue cardigan photo and English form fields, full canvas available; no sidebar of any kind.
Second window: helper button active subtle blue background; phone canvas narrows proportionally keeping complete portrait screen; right helper panel OPEN, header "工作助手" with small X close; a thin 3-tab strip "商品草稿" active underline, "常用文案", "通知". Inside show compact local draft selector "蓝色针织开衫", four small garment photos, small outlined action "导入手机相册"; title input "Blue knit cardigan · Size M" with small icon action and tooltip-like plain caption "发送到剪贴板"; description field short "Size M. Excellent condition." with small "发送到剪贴板" action; price €18 and local stock note A-023, tiny helper caption "在手机输入框内粘贴". No publish button outside phone.
Third large window: SAME TOOLBAR and SAME OPEN HELPER on right with identical draft values and photos preserved. Highlight "照片" shortcut active in toolbar, iPhone now displays iOS Photos library with a grid of imported blue cardigan photographs, other simple garment photos and standard Photos bottom tabs INSIDE PHONE ONLY. Right panel header "工作助手", same tabs and selected product, small subtle status "已保存到本地". Header also says tiny text "草稿不会随 App 切换关闭" as OUTSIDE-window explanatory callout below third window, not product control. Use thin arrow from Photos toolbar shortcut to phone screen to clearly explain change; explanatory arrows only outside product or delicately overlaid, never looking like permanent UI. Phone screen must be clearly separate from helper by vertical divider. Small explanatory line at board bottom "助手属于 RV；手机仍可自由打开任何 App。"
Constraints: show entry button exactly in consistent upper-right toolbar location across all three states. Close X appears only inside open helper header, helper-button toggles panel. App shortcuts visible in all states, including when helper open. Compact28px controls at logical scale, no giant tabs or buttons. No file search, bulk actions, automation, auto-fill, multi-phone controls, stats, duplicate bottom toolbar. This is concept not real Vinted screenshot, illustrative screens sufficient. Prioritize clearly readable layout and opening flow over decorative detail.
```
