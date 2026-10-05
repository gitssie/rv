# Vinted 工作界面概念 v1

概念图：[vinted-workspace-concept-v1.png](vinted-workspace-concept-v1.png)。使用内置 imagegen 生成，本次仅设计，不代表相关功能已实现。

左侧保留单台 iPhone 的实时画面，右侧为可收起的本地辅助面板。侧栏只显示当前选中的一个页签：商品草稿、常用文案或通知。

商品草稿页包含图片、标题、描述、价格与库存备注。图片导入手机相册后，在手机中的 Vinted 选择；文本发送到手机剪贴板后，在当前输入框粘贴。上架仍通过手机中的 Vinted 完成。价格与备注是本地草稿信息，不表示已经同步平台。草稿自动保存也是拟议功能。

沿用当前 RV 的紧凑深色风格：28px 操作按钮、细边框、蓝色主操作，没有重复底部工具栏。手机内部的 Vinted 页面及商品素材为示意，实际页面随站点、语言与版本变化。

## 生成提示词

```text
Use case: ui-mockup.
Asset type: high-fidelity RV native macOS remote-iPhone Vinted seller workspace concept, one complete desktop window, landscape 1536x1024.
Input image: STYLE REFERENCE ONLY for RV's existing compact graphite native UI. Do not copy its connection grid or checkerboard thumbnails.
Primary request: Design a clear, practical single-iPhone remote-control workspace for a Chinese-speaking Vinted seller working from a Mac. The phone stays the real interactive Vinted app; Mac side panel assists with local text drafts and importing photos. Distinct remote phone versus local helper panel, no fake integration dashboard.
Composition: full window with slender native titlebar reading "RV · 工作 iPhone", traffic lights, one 44px compact toolbar. Left toolbar small Home, lock, screenshot controls and a small Vinted shortcut. Right toolbar connected green dot "已连接", panel-toggle icon. Main area two regions separated by thin vertical border: left 55% dark remote canvas with ONE large portrait iPhone screen centered, right 45% compact native helper panel. Fit every control without clipping. Portrait screen shows illustrative Vinted mobile Sell form in English, white background with teal Vinted header, heading "Sell an item", four blue knitwear product photo thumbnails, Title field "Blue knit cardigan", Description field, Brand, Size, Condition, Price €18, small mobile "Upload" action at phone bottom. This mobile action only lives INSIDE the phone. No hardware bezel, giant outer phone frame, device grid or sync controls.
Right panel: slim header "上架助手" and collapse chevron. One modest tab strip "商品草稿" (active blue underline), "常用文案", "通知" with tiny unread dot; no second tab bar. Active draft content starts a compact draft selector "蓝色针织开衫" and subtle "已保存到本地". Section "商品图片" with four small square product photos (front, back, tag, texture) in order and understated dashed plus tile. Small outline action "导入手机相册" and adjacent helper text "上传后在手机内选择照片". Section "标题" with text field value "Blue knit cardigan · Size M" and compact right-row action "发送到手机". Section "描述" with multiline editable text "Soft blue knit cardigan. Size M. Excellent condition, no marks or damage. Measurements available on request." and compact row actions "使用文案" neutral and "发送到手机" blue. Small final two-column fields "价格" value "€18" and "库存备注" value "A-023". Bottom of panel modest explanatory text "发送到手机剪贴板后，在当前输入框粘贴。" No huge bottom call-to-action, no automatic publishing, no order statistics, sales charts, notifications permanently stacked below content, or repeated action footer. All Mac buttons approximately28px high. Panel padding20px, gaps12px, labels12px, body13px, title16px, restrained typography. Precise Chinese lettering, high readability, thin borders, subtle 6px corners.
Palette: graphite canvas #171c22, panel #20272f, borders #303944, blue action #3389f5, near-white ink, muted grey secondary. Teal only within illustrative Vinted mobile app. Use realistic garment product images, clean flat screenshot aesthetic with no dramatic shadows, gradients, explanatory arrows, marketing labels, watermarks, or exaggerated buttons. Output a polished plausible app UI that can be implemented in GPUI.
```
