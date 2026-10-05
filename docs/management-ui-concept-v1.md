# RV 连接管理界面设计 v1

生成方式：内置 imagegen。设计图：`management-ui-concept-v1.png`。

深色紧凑布局：左侧全部连接、最近使用和标签筛选；顶部搜索、网格/列表切换、设置和新增连接；中间显示保存的设备预览；选中后底部显示编辑与连接操作。双击卡片直接连接。预览标注“上次画面”，不表示实时在线状态。

图中尺寸为概念比例，实现时按钮高度以 28px 为准；搜索仅搜索连接簿。此轮仅生成设计，尚未修改管理界面代码。

## 完整生成提示词

```text
Use case: ui-mockup
Asset type: RV native macOS remote VNC connection manager design proposal.
Primary request: create ONE exceptionally clear high-fidelity management screen, readable Simplified Chinese, suitable for implementation with Rust GPUI. Wide landscape 1536x1024 image, one app window almost fills canvas, front-facing flat screenshot, no perspective, no laptop shell, no explanatory arrows, no montage.
Design direction: quietly refined dark native desktop utility, precise compact controls, elegant spacing and restrained blue accent. Graphite main surface #171c22, sidebar #13181e, cards #20272f, hairline separators #303944, legible off-white text and muted slate metadata. Subtle rounded 8px corners. No gradients, glowing decorations, enormous pill buttons, marketing hero, statistics widgets.
Composition: native top titlebar with three small traffic lights and title "RV". Below titlebar a 44px compact toolbar: small sidebar-toggle icon, search input "搜索连接…" left, substantial flexible space, compact grid/list toggle and settings gear, then small outline plus button "新增连接" right, 28px control height. NOT a giant button strip.
Left sidebar 176px: heading "连接"; selected "全部连接" count 6 with a soft blue-gray highlight; "最近使用" count 3; thin divider; small heading "标签"; "工作" count 2, "测试" count 2, "家用" count 2. Small subtle bottom caption "本地连接簿". No phone control actions here.
Main content: compact heading "全部连接" with small muted "6 个连接". 3-column by 2-row grid of six carefully aligned equal cards, modest 18px gaps. Each card has a LANDSCAPE preview well about 235x130 pixels, then compact metadata area, total card height about 205px. iPhone screenshots must be actual portrait shaped centered inside dark landscape well with generous sides, never stretched. Mac previews small plausible desktop windows. Each preview well bears tiny muted label "上次画面" in lower left to mean stored snapshot, NEVER live online status. Keep thumbnails unobtrusive and real-looking.
Six card names and metadata verbatim:
"测试 iPhone" / "192.168.1.26:5900" / small label "测试" / "刚刚使用";
"办公 Mac" / "192.168.1.18:5900" / "工作" / "10 分钟前";
"家用 Mac mini" / "192.168.1.36:5900" / "家用" / "昨天";
"备用 iPhone" / "192.168.1.27:5900" / "测试" / "2 天前";
"开发主机" / "192.168.1.20:5901" / "工作" / "3 天前";
"客厅电脑" / "192.168.1.42:5900" / "家用" / "上周".
First card selected with restrained thin blue border and slight tinted card metadata surface; no fat shadow. Thumbnail is a softly colored iPhone home screen, recognizable small app icons. On selected card top right only small ellipsis control, no permanent red trash button anywhere.
A slim selection/action bar anchored at bottom of main content, separated by hairline: left small phone icon followed by "测试 iPhone" and muted address; right small ghost "编辑" pencil button, ellipsis icon for secondary destructive actions, and one restrained blue 28px-high rectangular button "连接" with a tiny play icon. Tiny return-key shortcut glyph beside Connect if fits. Below or at bottom edge small status text "双击卡片连接 · ⌘N 新增连接". No duplicate Connect buttons on every card.
Typography: beautifully crisp Chinese desktop UI, names 14px, metadata 11-12px equivalent, compact balanced hierarchy and excellent contrast. Large readable app content scaled to image. Refined native feel rather than web dashboard.
Constraints: accurately reflect existing RV connection list, labels, recents, stored screenshot previews, grid/list, settings, new connection, selected connection editing and connecting. Do not invent online discovery, ping status, battery data, unread notification cards, phone control dashboard, file search, cloud account or tabs. This is the MANAGEMENT screen, not an active remote-session screen. All main control buttons small and consistent.
```
