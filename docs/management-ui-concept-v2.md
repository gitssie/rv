# RV 连接管理界面设计 v2

使用内置 imagegen 编辑 v1，设计图为 `management-ui-concept-v2.png`。

取消底部重复设备信息和操作栏；连接按钮移入选中卡片，编辑、复制、删除集中到卡片的“…”菜单；保留双击连接。设计图已检查：底部栏完全移除，选中卡片内只有一个连接入口，名称和地址没有重复。实现时按钮高度为 28px，卡片预留固定操作空间，避免悬停导致布局变化。

本轮仅设计，未修改界面代码。

## 完整提示词

```text
Use case: ui-mockup, precise-object-edit.
Edit target: supplied RV connection manager mockup.
Create a refined second version of this same native macOS management screen. Preserve the graphite dark palette, titlebar, search toolbar, sidebar filters, Chinese text, all six device card identities, 3-column 2-row grid, selected first iPhone card, restrained blue selection, thumbnails and native feeling.
Required changes:
1. REMOVE the entire bottom selected-device/action bar: remove its dividing line, phone icon, repeated device name and address, Edit button, ellipsis button, Connect button, and keyboard shortcut caption. The main content surface must continue uninterrupted to the bottom, with comfortable clean empty margin below the grid. No footer, status strip, replacement bottom bar, repeated details or duplicated actions.
2. Move the Connect action INTO the FIRST SELECTED CARD only. Show one compact restrained blue rectangular button with small play icon and exact Chinese text "连接", positioned right-aligned beside the card's device name "测试 iPhone", in the metadata area immediately under the thumbnail. Button ~28px high at desktop scale. Keep address on a separate muted line beneath name. Keep label "测试" and "刚刚使用" on final metadata row. Do not overlay connection button on phone screenshot. Do not put connect buttons on the other five unselected cards. A single selected-card button communicates selection/hover behavior.
3. First selected card retains one small ellipsis at top right of preview, secondary editing/copy/deletion options live in this menu; show menu closed. No separate Edit/Delete controls.
4. Slightly reduce sidebar width and oversized main heading so hierarchy feels compact and calm. Keep cards consistent in size, enough air around controls. Keep all thumbnail proportions, especially portrait iPhones centered in landscape preview wells with dark sides. Preview caption remains "上次画面" (stored screenshot).
5. Keep top toolbar compact, 28px controls: search, grid/list, settings, "新增连接". Preserve Chinese labels clearly. Same window with large readable UI, full-frame front-facing screenshot. No annotations, presentation frame, arrows, mockup laptop, gradients, extra stats, new features, online status or tabs.
Deliver one polished final image.
```
