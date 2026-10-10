# RV ZXTouch 设备管理设计图

使用内置 imagegen 生成。以下为 UI 方案，尚未实现。

- [App 状态管理](zxtouch-app-state-concept-v1.png)：备份列表、当前关联、空白基线、保存后切换、操作结果。
- [Apple 账户服务](zxtouch-apple-account-concept-v1.png)：分服务账户状态、联系与转发邮箱、授权 App、设备入口、手动验证码提交。

能力依据：TrollVNC/docs/app-state-zxtouch.md 与 TrollVNC/docs/apple-zxtouch.md。图内账户、时间与状态为示例。

实现时，Apple 原生操作忙碌期间应禁用冲突操作；截图中邮箱区域表示界面层级，不代表等待登录验证时仍能修改邮箱。App 状态保存/切换完成后保持停止；已关联当前备份不允许删除。验证码提交需继续等待原生最终结果。未知结果不得展示为成功或自动重试。

## 最终提示词：App 状态管理

```text
Use case: ui-mockup. Generate a high-fidelity, production-quality desktop RV application UI concept in Simplified Chinese, landscape 16:10, one full window screenshot, flat front-on, sharp legible typography. Reference image only informs existing RV charcoal macOS UI and small blue accent, compact top toolbar, remote iPhone view; create a new App state management workspace. Product title "RV · 我的 iPhone". This is the same integrated RV product; no separate ZXTouch app branding, protocol labels, tasks, token fields, connection toggle, or input/gesture tools. Primary feature is actual on-phone App data snapshots and baseline switching, not text input.
Window: thin native titlebar with macOS traffic lights. Compact top toolbar with zoom "适应", keyboard, files, Home, screenshot, lock, small Safari/Photos/Settings app shortcuts, then right selected action "设备管理". Below toolbar split into left 27% dark live-phone preview column with a centered smaller portrait iPhone showing iOS home screen, caption "实时画面", "390 × 844", subtle bottom connected indicator; right 73% management workspace. Phone preview should never dominate management content.
Management workspace: compact header "设备管理", horizontal module navigation "App 状态" selected, "Apple 账户", "脚本", "设备信息". Left sub-column inside management area 170px wide contains search "搜索 App" and installed app list with small authentic-looking app icons, selected "Vinted", then "Safari", "照片", "设置"; muted package ID beneath selected app "fr.vinted". No fake ability to backup system apps: only Vinted is active state-management target; other apps just navigation list.
Main content: heading with Vinted icon, "Vinted", small "已停止", subtle version "版本 26.1". Right compact actions "启动 App" and ellipsis. Current environment summary is a narrow bordered horizontal strip titled "当前环境" with name "工作账号 A", small blue pill "当前", note "已关联备份", and primary blue button "保存当前状态". Below action toolbar heading "备份与基线" with search, secondary action "创建空白基线".
Main table with high quality native rows: columns "名称", "备注", "保存时间", "版本", "操作". Row1 "工作账号 A", note "常用环境", time "今天 10:24", version "26.1", small current pill, actions "编辑" and subtle disabled "切换". Row2 "工作账号 B", note "备用环境", time "昨天 18:30", version "26.1", actions "切换" and ellipsis. Row3 "空白基线", muted type pill "基线", note "全新 App 环境", time "10 月 08 日", version "26.1", action "切换". Small footnote: "保存与切换会停止 App，完成后可手动启动。"
A restrained local contextual popover anchored to row2 "切换", width ~350px, clearly the review step before executing, without obscuring the whole app: title "切换到工作账号 B"; summary "先保存当前环境，再恢复目标备份"; current env "工作账号 A"; blue button "保存并切换"; secondary button "取消"; small less-prominent alternative text action "放弃当前状态并切换". Do not show false completed state alongside this pre-action review.
Bottom operation history is concise: header "最近操作"; one row green check "工作账号 A 已保存" with time "10:24" and small "查看详情". No long log terminal. Slim global footer connected dot and "我的 iPhone".
Style: match RV reference charcoal #202830 surface, near-black canvas, restrained bright-blue primary buttons, thin dividers, compact 28-32px toolbar buttons, 8px corner radius, readable Chinese SF-style typography. Information-rich and polished, no giant empty dashboard tiles, no gradient/marketing poster, no input editor, no keyboard tools drawer, no invented cloud backup or bulk clone or delete-current button. App state means actual snapshots/baseline, no claims that every restored login remains valid.
```

## 最终提示词：Apple 账户服务

```text
Use case: ui-mockup. Generate the companion Apple account services screen for RV desktop, matching reference image's compact native macOS charcoal UI, blue accents, split window with smaller live iPhone preview left and large device-management workspace right. Landscape 16:10 front-on, ultra readable simplified Chinese text, one full app window screenshot, no perspective. Reference is layout and style only. Primary focus is Apple native account services offered through integrated ZXTouch, not input, OCR, gestures or keyboard automation.
Titlebar "RV · 我的 iPhone", traffic lights, compact icons for zoom, keyboard, files, Home, screenshot, lock, app shortcuts; "设备管理" selected. Left 25% live iPhone portrait preview with a neutral iOS Settings "Apple 账户" screen, no personal details, caption "实时画面" and connected dot. Right 75% main workspace. Header "设备管理", module navigation "App 状态", selected "Apple 账户", "脚本", "设备信息". Do not retain installed-app sidebar from reference on this account module.
Main section heading "Apple 账户", subtle subtitle "查看各服务状态并管理当前账户", upper-right outlined button "刷新状态". Three compact horizontal status panels, accurate distinct service states instead of one global logged-in badge:
1 "App Store", green dot "已登录", masked identity "m•••@example.com", small text action "退出".
2 "iCloud", amber dot "等待验证", masked identity "m•••@example.com", small text action "继续验证".
3 "身份账户", green dot "已登录", same masked email; no invented standalone identity login action.
Below, ordinary compact settings navigation "账户资料", selected "转发邮箱", "授权 App", "设备".
Email panel: title "通过 Apple 登录", description "管理隐藏邮箱的转发目标"; one labeled field "当前转发邮箱" shows "work@example.com" with small "已验证" badge. Another row label "其他联系邮箱", "personal@example.com" with badge "已验证", and outlined "设为转发邮箱". A modest secondary action "+ 添加联系邮箱". One informational line "转发设置作用于此 Apple 账户的全部隐藏邮箱。" Keep account service operations factual; do not claim per-app forwarding.
Right-hand contextual verification sheet, ~330px wide within main workspace or anchored to iCloud "继续验证"; title "验证 iCloud 登录"; masked account; instruction "输入从受信任设备获得的验证码"; six empty digit input boxes, selected first slot. Primary blue "提交验证码", outlined "取消验证"; subtle hint "提交后需等待系统确认". No automatic code retrieval URL, no resend button unless supported, no secrets or passwords shown, no promise login success.
Bottom main area recent operation strip: clock/amber small indicator, title "iCloud 登录", state "等待验证码", small time "刚刚"; action "查看详情". A subtle separate lower account action group can offer "登录 App Store", "登录 iCloud", "退出 Apple 账户" only in understated secondary forms where applicable; do not show enabled duplicate login while waiting. Prefer no extra buttons if crowded. No giant success banner, no account pool, no batch device removal button, no blanket all-accounts success while cloud waiting. Full operation results and states are distinct; no technical protocol names or IDs in primary screen.
Style: matte charcoal panels, crisp 1px separators, modest 6-8px corners, small blue highlights, white primary text, cool gray secondary text. Dense but calm native tool layout with ample alignment, not a marketing dashboard. Preserve RV family from reference.
```

