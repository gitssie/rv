# RV 开发约定

同时遵守父目录的 AGENTS.md。

## 新版数据存储

- 设备配置、业务对象和新增元数据使用本地 SQLite 表结构。
- 不读取、迁移或兼容旧版设备配置记录；不维护 JSON 与 SQLite 双存储。
- 设备使用本地 `id` 作主键。完整真实 GUID 是设备唯一识别字段，不在 UI 显示，不使用 IP 或生成的 UUID 冒充真实 GUID。
- ZXTouch 需要补充真实设备 GUID 的返回；接口补齐前留空，不用 `identifierForVendor` 替代。IDFV 属于虚拟 App 在设备上的身份，同设备同 Bundle 的不同虚拟 App 也可以不同。
- 虚拟 App 的主键直接使用真实 IDFV，以此去重，不生成替代主键。未获取 IDFV 的备份不自动建卡片。IDFV 是 App 环境、备份业务身份和 Vinted 店铺共同使用的唯一实体标识；同一 IDFV 的多个快照不产生多个店铺。
- 虚拟 App 只来源于真机 ZXTouch App 备份的业务化，不提供文件导入或手工创建功能。空白基线不作为店铺。
- 业务对象采用统一表结构，包含多标签和文本备注。业务特色信息保存为文本，并纳入全文检索。
- 用户已明确确认所有数据使用 SQLite，包括 VNC 密码和锁屏密码。此确认覆盖父目录的旧密码文件约定；密码继续本地加密，不使用系统钥匙串，不读取或迁移旧密码文件。

## VNC 与 ZXTouch 的协议边界

- 正常 VNC 的画面、鼠标、键盘远程控制继续保留，不因 ZXTouch 改造而移除或禁用。
- 所有属于 ZXTouch 模块的调用（含 OCR、触摸、Apple/App-state 及 Photos 弹窗自动确认）统一走已认证 HTTP/2。禁止借用 VNC 输入、VNC 扩展请求或旧 ZXTouch TCP 协议作为测试替代或失败回退。
- Photos 文件传输现有流程与新增 ZXTouch 弹窗识别/确认分别核查；确认弹窗只能为已有的显式删除服务，不另发删除请求。
- 可以共用手机原生 HID/截图实现；共用原生实现不等于共用 VNC 传输路径。HTTP/2 请求成功不等于触摸生效，应验证实际界面/原操作结果。

## 模块与策略

- 按模块和插件组织实现；存储、业务逻辑、通用 UI、个性化 UI 分离。
- 根据 App Bundle ID 注册和选择策略。通用卡片容器负责共有字段与操作；各 App 策略负责业务内容与动作。
- 不在通用 UI 中堆积按 Bundle ID 判断的业务分支。
- 普通 App 的环境/备份和 Vinted 的店铺采用各自策略；店铺身份不等同于某份备份，也不绑定死在某台手机上。
- 未接入的业务请求不得以模拟状态展示为实际结果。

## UI 图标约定

- 统一使用现有 GPUI Component 的 `Icon` / `IconName`（Lucide SVG 图标库）；通用操作和状态优先使用库内图标。
- 库内缺少所需图标时，复用项目 `FunctionIcon` 注册机制和 `assets/icons/` 中的 Lucide SVG，保留 `LUCIDE-LICENSE`。不新增另一套图标库。
- 不使用文字字符、emoji、手绘图形或 Imagegen 位图替代已有标准图标；图标颜色、大小遵循现有主题和组件规范。
- 店铺详情浮层不提供进入编辑的入口；名称、标签、备注从首页卡片更多菜单编辑。编辑表单不得混入商品/订单、骨架或业务操作。
- 整个 RV UI 优先且统一使用 GPUI Component 现成组件：确认使用 AlertDialog，模态表单使用 Dialog、页面表单使用 Form/Field，切换使用 TabBar/Tab，数量使用 Badge，标签使用 Tag。没有现成组件可满足的特殊业务区域才自行实现。
- 组件尺寸、内边距、字重、圆角、悬停/选中等效果遵循组件库默认值，不按设计图逐个覆盖，不手写像素高度模拟控件。设计图用于配色和布局，颜色集中到主题配置；VNC 画布、图片和业务容器的布局尺寸不属于控件尺寸。
- 手机在线必须以 ZXTouch 使用该设备 VNC 相同密码认证成功、控制连接有效为准。HTTP 可达、端口开放或返回 401 都不算在线。使用 badge 式状态圆点：在线绿色、离线灰色；不显示状态文字，不使用叉号或插头图标。ZXTouch 密码复用现有 VNC 密码，不引入独立密码，不以 VNC 窗口是否打开作为在线判据。

## 文字层级

- 字号与密度由 `theme::apply` 全局设置，正文约 13px、辅助文字约 11px；复用 GPUI Component 的相对字号与默认控件样式，不在各窗口单独缩放或各自设置 rem。用户要求整体缩小并统一。
- 自定义内容的 label、正文统一使用 `text_sm`；辅助说明、时间与技术元数据使用 `text_xs`。label 使用 `typography::label`，辅助说明使用 `typography::caption`。
- 页面/卡片小标题使用正文大小加 semibold；Dialog 标题使用组件默认效果。不得为同一类 label 分散设置 10/11/12/13px 等任意字号，也不通过父容器的微小字号缩小所有控件。
- 全部应用控件跟随同一 GPUI Component 深/浅主题；仅远程画布保留独立深色背景。页面不得保留自己的 BG/PANEL/TEXT 配色或固定深色 toolbar 色组。`theme::apply` 更新 colors/tokens 后必须同步 Base 主题；圆角、悬停、选中、禁用和焦点沿用组件主题。
- 公共组合位于 `ui.rs`：工具面板、图标按钮、独立 Field、加载/进度、紧凑任务 Dialog 和空状态。一级/二级工具页面共用 `tool_panel`；简单 Button 直接使用默认/primary/ghost/danger，不新增 primary_button 等转发封装。不以设置正常前景色覆盖控件禁用/选中态。
- `python3 scripts/check-ui-style.py` 是 CI 样式检查，禁止页面直接写颜色、字号/字体、固定圆角及控件像素高度/内边距。主题色只在 theme.rs 定义；品牌色经 theme::brand_color 映射。图片、画布和业务布局容器的几何尺寸继续允许。
- App 卡片头像来自 ZXTouch 的 Bundle ID 图标接口，按 Bundle ID 存入本地 `app-icons` 缓存；同 Bundle 的卡片和控制窗口共用，不能使用业务策略的字母假冒 App 图标。

## 工作流程

- 脱机后台任务在 RV 本机业务策略模块执行，通过 access token 和业务 HTTP 接口完成；不发送到手机，不依赖真机连接、打开 App 或恢复备份。zxtouch 只负责备份、身份与账号资料读取及显式恢复。刷新后的凭据保存在 RV 加密 SQLite，不能改写原始备份。

- 以 `docs/designs/rv-home-v4.png` 和 `docs/designs/rv-home.md` 为已确认界面依据。
- 对影响数据模型、业务行为或范围的疑问及时询问用户。
- 编码阶段不构建 rv-app，也不打包应用；可做 Rust 类型检查和必要模块测试。
- 保留有意义的测试，不额外生成测试报告。

## 脱机凭据与代理

- 手机原始 plist/备份保留在手机；RV 从已认证、Bundle+IDFV 校验的 profile 通道读取业务凭据。access_token、refresh_token、expires_at 和 credentials 额外字段保存在加密 SQLite，不放入卡片元数据、全文索引、任务结果或日志。历史备份不能覆盖 RV 已刷新的凭据。
- SOCKS5 代理配置及 IDFV 绑定保存在 SQLite，用户名、密码和连接配置加密。每个 IDFV 固定选择一个代理，不因等待或故障改出口；只有明确未绑定时直连。
- 请求期间占用代理：共享模式允许不同 IDFV 并发；排他模式下其他 IDFV 等待，同 IDFV 可重入；请求序列完成、失败或取消都释放占用，空闲不占用。HTTP 连接池存活不代表代理仍被占用。等待不计入业务请求超时，也不提前创建 running 任务。
- 商品、订单、聊天和首次图片下载按对应 IDFV 的同一代理执行；图片缓存命中不触发网络请求。代理管理使用 GPUI Component 默认表单/按钮，App 编辑选择绑定。

## 设置页面

- 已确认设置布局参照 `docs/designs/rv-settings-v1.png`：主窗口全页设置，左侧分类导航与返回主界面，右侧内容；不再用设置 Dialog 或顶栏 Tabs。
- 设置主布局使用 GPUI Component `setting::{Settings, SettingPage, SettingGroup, SettingItem}`；左侧只保留通用、代理菜单，不显示搜索设置；内容页不重复显示通用/代理标题栏。返回主界面按钮放在窗口 TitleBar 中，不另占设置内容高度。表单控件继续使用库内 GroupBox、Form/Field、Select、Input、Switch、Button，保留默认控件尺寸和样式；设置页面的背景、边框和控件颜色遵循 RV 主题调色板。
- 切换设置分类后转移焦点，返回主界面保留设备选择、筛选与折叠状态；设置页的快捷键不能操作隐藏的手机列表或创建 VNC 连接。
