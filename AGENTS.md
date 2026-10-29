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
- 通用 App 卡片图标来自 ZXTouch 的 Bundle ID 图标接口，按 Bundle ID 存入本地 `app-icons` 缓存；同 Bundle 的卡片和控制窗口共用，不能使用业务策略的字母假冒 App 图标。Vinted 以店铺作为最终 virtual App：卡片与详情共用店铺名称及缓存头像，没有头像时使用同一 Vinted App 图标，不再分开显示两套身份／头像。

## 文件传输

- FileTransfer 的 Files / Photos / Safari 使用同级 Tab。Safari 上方像 Photos 相册一样下拉选择实际文件目录，包含存在的 Downloads 和已确认保存 PDF 的 Safari `tmp/QuickLookContent-*` 目录；不显示路径输入、后退、上一级或新建文件夹导航。不能因标准 Downloads 不存在而遗漏用户确认的实际保存文件。
- 通过当前已认证 ZXTouch HTTP/2 只读查找 LocalStorage AppGroup 及 Safari App 容器元数据，由 RV 解析 plist 并校验身份；文件列表和传输继续复用原有 VNC 文件通道。不写死设备容器 UUID，不把 `/Media/Downloads` 的下载记录数据库当成下载文件夹。
- 下载目录未创建或列表为空时只显示 Empty 图标，不显示长提示或红色错误状态。切换目录时等待目标列表确认，期间禁止对旧目录上传或管理。

## 手机功能面板与自动化

- 手机工具面板的 Apple / App 环境 / 功能为同级 Tab；注册位于功能 Tab 内，沿用 360 宽的公共工具容器和四行 Textarea 统一粘贴资料，不拆成多个输入框。已确认设计为 `docs/designs/rv-function-registration-v2.png`。
- 注册策略位于 `rv-session::automation::vinted`，本机任务由 `functions::registration` 持有，使用当前已认证 HTTP/2 客户端，不每步登录。运行、取消、断线及结果待确认必须明确展示，不能从命令 ACK 宣称注册成功。
- 当前注册覆盖实测法语竖屏表单提交、邮箱取码与验证码提交，到添加手机号页面停止；不能宣称完整注册成功。不得在未验证页面上猜坐标或自动重试填写/提交。运行期间不要操作真机或其它外部控制客户端。

## 工作流程

- 脱机后台任务在 RV 本机业务策略模块执行，通过 access token 和业务 HTTP 接口完成；不发送到手机，不依赖真机连接、打开 App 或恢复备份。zxtouch 只负责备份、身份与账号资料读取及显式恢复。刷新后的凭据保存在 RV 加密 SQLite，不能改写原始备份。

- 以 `docs/designs/rv-home-v4.png` 和 `docs/designs/rv-home.md` 为已确认界面依据。
- 对影响数据模型、业务行为或范围的疑问及时询问用户。
- 编码阶段不构建 rv-app，也不打包应用；可做 Rust 类型检查和必要模块测试。
- 保留有意义的测试，不额外生成测试报告。

## 脱机凭据与代理

- 手机原始 plist/备份保留在手机；RV 从已认证、Bundle+IDFV 校验的 profile 通道读取业务凭据。access_token、refresh_token、expires_at 和 credentials 额外字段保存在加密 SQLite，不放入卡片元数据、全文索引、任务结果或日志。历史备份不能覆盖 RV 已刷新的凭据。
- SOCKS5 代理配置及 IDFV 绑定保存在 SQLite，用户名、密码和连接配置加密。每个 IDFV 固定选择一个代理，不因等待或故障改出口。所有 Vinted 查询、令牌刷新、聊天和图片网络请求必须使用该虚拟 App 绑定的代理；未绑定时返回错误，禁止直连或自动选取其它代理。图片缓存命中不触发网络请求。
- 请求期间占用代理：共享模式允许不同 IDFV 并发；排他模式下其他 IDFV 等待，同 IDFV 可重入；请求序列完成、失败或取消都释放占用，空闲不占用。HTTP 连接池存活不代表代理仍被占用。等待不计入业务请求超时，也不提前创建 running 任务。
- 需要稳定出口的任务使用 `rv-business::proxy::ProxyManager` 的托管租约，调用方传独立任务 owner；同任务重入共用准备结果，不用 IDFV 冒充任务 owner。托管租约与普通业务共用同一 Gate，换 IP 和整个任务期间排他占用；内层更严格但不满足的策略直接失败，不重换外层 IP。详见 `docs/proxy-exit-leases.md`。
- 换 IP URL/IP 查询 URL 只在代理设置中维护并加密保存；保存不请求。历史池是同一 SQLite 内全出口共享的 `proxy_ip_history`，IPv4 唯一约束原子去重，失败或释放租约不删除已见历史。注册页不承担 IP 管理。
- 商品、订单、聊天和首次图片下载按对应 IDFV 的同一代理执行；图片缓存命中不触发网络请求。代理管理使用 GPUI Component 默认表单/按钮，App 编辑选择绑定。

## 设置页面

- 已确认设置布局参照 `docs/designs/rv-settings-v1.png`：主窗口全页设置，左侧分类导航与返回主界面，右侧内容；不再用设置 Dialog 或顶栏 Tabs。
- 设置主布局使用 GPUI Component `setting::{Settings, SettingPage, SettingGroup, SettingItem}`；左侧保留通用、代理和 Apple ID 菜单，不显示搜索设置；内容页不重复显示通用/代理标题栏。返回主界面按钮放在窗口 TitleBar 中，不另占设置内容高度。表单控件继续使用库内 GroupBox、Form/Field、Select、Input、Switch、Button，保留默认控件尺寸和样式；设置页面的背景、边框和控件颜色遵循 RV 主题调色板。
- 切换设置分类后转移焦点，返回主界面保留设备选择、筛选与折叠状态；设置页的快捷键不能操作隐藏的手机列表或创建 VNC 连接。


## Apple ID 账户资料与改密（2026-10-27）

- 设置 Apple ID 分类使用 gpui_component 的 DataTable，读取 apple_accounts SQLite 表，支持元数据搜索、刷新和密码显隐；不把界面模拟数据作为实际账户。
- 当前密码、首次原密码与待用新密码分别保存；用户已明确要求 Apple ID 密码明文，VNC/锁屏密码仍按原有本地加密配置读取。
- 手动「修改密码」放在 Apple 工具面板，与首次登录自动改密共用候选先落盘和原生 changePassword 接口。手动操作不强制登录 App Store，不切换不同媒体账号；设备关联为可空非唯一弱关联。
- 隔离 HTTP2 真机改密及随后正式 Wi-Fi6000 退出/新密码登录已通过；不能将这些结果说成运行中的旧 RV GUI 已包含新界面或手机 daemon 已更新。详见 docs/apple-accounts.md。

## 任务与业务 UI 边界（2026-10-27）

- 所有 Vinted 接口拉取、落库与图片下载归业务 tasks 执行器；页面只提交任务指令、读取 SQLite 和本地图片缓存，不按打开页面触发拉取。
- state 导入时确认凭据；缺失则报告导入错误，不创建接口任务。凭据齐全后创建阶段任务，基础资料默认一次性 cron，成功后不再自动获取；其余任务等待成功，并允许按店铺修改 cron／启动／停止；未关联代理时 tasks 等待，用户编辑虚拟 App 关联代理后继续。
- 店铺数据 UI 和业务表不承载任务执行状态、进度、上次同步信息；店铺独立「任务配置」Tab 仅配置 cron 与启停，执行日志仍在设置任务列表；空业务数据展示空状态。任务的时间、结果、错误只保存在 tasks，通过设置中的任务列表管理。
- 删除旧业务定时开关和独立内存调度，不做旧功能／旧数据迁移兼容。不增加执行明细表。详见 docs/tasks.md。

- 图片下载是对应拉取任务的必要步骤（基础资料头像、商品／订单／消息图片），磁盘已有就复用；不建立图片任务或图片 cron。成功提交业务数据前完成需要的缓存。
- 店铺任务配置采用 GPUI Component 默认横向 Form/Field、Input、Button、TabBar；只组合紧凑布局，不照设计图覆盖控件高度、圆角和内边距，不重复展示表头与说明段落。

- 店铺信息集中在同一紧凑 header，国家／评分／评价／商品数用图标加数值，说明放 tooltip。右侧以 Shield／Refresh 图标和本地日期展示 Access Token／Refresh Token 过期时间，完整说明放 tooltip；UI 只持有时间投影，不持有或显示 Token。过期时间来自服务器字段或 JWT exp（仅展示，不验证有效性），无依据显示未知，不给 opaque Token 推算默认期限。

- 商品／订单 Tab 使用库内 icon + label；任务名称与 cron 统一正文常规字重，编辑／启停等操作为带 tooltip 的图标按钮。执行列表入口放任务工具栏右上角，不在底部保留文字按钮。数据内容区填满剩余高度，Spinner、Empty 在该区域水平／垂直居中，Tab 固定在顶部，列表自身滚动。

- 空数据／空任务仅显示 `ui::empty_icon`（Inbox），在数据视口水平／垂直居中，不输出“暂无数据／暂无任务”文字。GPUI Component Tab 的 `icon()` 为纯图标模式；需要图标和文字时用 `prefix(Icon)` + `label`，商品／订单／任务配置三项保持一致，任务配置使用 Settings2。

- 已确认店铺布局参考 docs/designs/rv-shop-task-config-v3.png，沿用默认组件。Header 第一行昵称＋横排查询资料／消息／未读／刷新令牌／读取账号／关闭图标按钮；第二行左侧统计，右侧一个凭据图标＋caption字号的 `Access 时间 | Refresh 时间`。日期为被动文本，不使用 Button 的字号和内边距；完整类型／日期放 Tooltip。多备份时仅读取按钮使用来源选择菜单。

- 店铺操作使用「刷新资料」（通过 Vinted 更新店铺业务数据）与「同步账号备份」（通过手机读取 state／凭据／设备信息）区分来源；移除所有手动刷新令牌的 UI 操作入口，执行列表中的 refresh 记录仅供查看。凭据图标按 Access Token 期限使用现有 success／danger／muted 主题状态色，不把到期时间当作服务端接受认证的证明。

- 店铺手动「同步账号备份」（含多来源选择后）先弹出 GPUI Component AlertDialog，明确会更新本地 state／设备请求参数，令牌不同则保留 RV 当前凭据，且不会写回手机；只有确认后才读取，取消不触发同步。首次自动导入缺失 state 的流程维持原有行为。

- 虚拟卡片底部展示「手机名称 · 备份标题」，多来源悬停列出全部，右侧 Info 图标打开 GPUI Component 只读备份详情 Dialog；详情从 SQLite 读取来源手机、备份标题／时间、App 版本／构建、备注和备份标识，不触发手机请求。Vinted 无昵称时展示 Vinted，备份标题不作为店铺昵称；账号配置仅编辑标签、备注和关联代理，不编辑名称或改写 app.title。

- 卡片主体打开详情，收藏／更多操作独立响应；菜单按当前状态显示置顶／取消置顶、归档／取消归档。卡片没有商品时省略预览，不显示“暂无数据”。评分、数量、凭据等只读信息用图标＋文本＋Tooltip，不用无动作 Button。卡片与详情共用 Token 到期规则及主题色，卡片可见关联代理名称，配置仍在账号编辑。名称排序与任务列表显示店铺昵称，任务列表可按昵称查询。
- 手动从手机同步账号备份的店铺及备份列表入口共用 AlertDialog；只有确认后读取，状态保存后才提示完成。显式同步按钮 loading，失败／断线／窗口关闭清理对应读取窗口的忙状态，迟到回调不清其它窗口发起的读取。删除账号默认只删本地，删除所有来源手机备份必须主动勾选。

- 店铺与账号编辑等受管模态关闭／切换时清理该窗口整个 Dialog 栈（含子确认／详情），不只 pop 最上一层，避免 Modal 已清空但旧 builder 留下空白小窗；保留正常子确认取消不关闭店铺的行为。
- 店铺「会话保持」（kind=refresh、task_key=keepalive）仅控制自动刷新 token，默认 cron `0 0 * * * *`；停止它不停止、不阻止其它任务，也不擅自刷新。所有任务统一读取 SQLite 当前凭据；任意受认证接口返回 401 即标记当前 access token 被拒绝，后续依赖该 token 的请求等待新凭据，保留各任务 enabled/cron，不继续用同一 token 请求。换入不同 access token 后清除拒绝标记，经 Credentials EventBus 唤醒等待任务；旧请求迟到的 401 不影响新 token，同 token 元数据保存不能清除拒绝状态。新 token 仅表示可尝试，不以 JWT 到期时间猜测服务端有效性。

- 「交给手机」仅回写所选手机相同 Bundle／IDFV 的备份 `state.tar.zst` 与匹配 manifest，用户明确不改 Shadow、不恢复 live Keychain／偏好、不启动 App、不触碰 data.tar.zst。先持久化 app_execution.mode=to_phone 并取消全部调度／禁用任务，等待已运行任务提交最终凭据后导出；顺序 state→manifest，再读回逐字／SHA／大小校验，通过才持久化 phone。失败或断线保持 to_phone，可显式重试同一冻结版本，不自动恢复 RV 任务。新任务／领取／立即执行／启用均校验执行归属，初始化不重开手机接管任务。
- 卡片与店铺共用 Monitor／Clock／Smartphone 接管图标，说明放 Tooltip；Phone 模式只表示备份已校验且 RV 停止，不声称手机 live 环境已使用新令牌。按钮使用默认 AlertDialog 确认和来源选择，取消不改变模式。
- 手机备份菜单的「交给 RV…」仅允许当前 Phone 归属设备；确认框要求用户勾选已保存最新备份并停止手机 App，不把用户确认当作程序自动检测。重新读取完整 state，明确采用手机凭据，导入与模式切回 RV 原子保存；失败仍为 Phone。全部任务保持停止、cron 保留，用户手动启动。普通备份同步继续保留 RV 已更新凭据。详见 docs/phone-handoff.md。

- Dialog 归属同步必须在 `WindowRoot::render` 构建 `Root::render_dialog_layer` 之前完成，不放在其子 `AddressBookApp::render` 中。WindowRoot 观察 AddressBook 更新以刷新弹层，避免关闭后旧 builder 根据 Modal::None 生成默认空 Dialog 并缓存。回归要检查关闭首帧实际渲染层消失，不能只检查 has_active_dialog=false 或靠第二次 draw 掩盖。

- 手机备份详情使用 GPUI Component DescriptionList / DescriptionItem，只读展示 SQLite 中的来源手机、标题、时间、App 版本、构建、备注和标识，沿用默认控件效果。备份行移除恢复/切换箭头，在原位置显示 Monitor「交给 RV」；仅 Phone 模式且匹配当前接管设备时可执行，RV 模式表示已接管，ToPhone 表示交接未完成。更多菜单不重复放接管入口。app-switch 已注销，原生同名方法明确拒绝；恢复由 Shadow 完成。

- 手机备份面板的工具栏仅保留「刷新备份」，无启动、停止、运行状态、创建基线、环境详情或保存环境按钮。App 环境操作注册仅保留列表与元数据编辑；创建、保存、切换、删除、describe 及旧 profile/sessionSupplement 不再作为备份接口提供。首页删除卡片只删本地，不提供连带删除手机备份选项；手机备份创建、恢复、删除由 Shadow 管理。


## 聊天模块（2026-10-27）

- 已确认设计 docs/designs/rv-chat-store-conversations-v3.png：店铺／会话／消息三栏，店铺与会话均无搜索；记住每家店铺当前会话。单一 Textarea 输入，图标按钮发送。原文与已有译文在同一个消息气泡展示，不增加翻译预览框。
- 优先官方 GPUI Component Message／Bubble／ListItem／Avatar；当前锁定 SDK 没有 Message／Bubble，rv-gpui-chat 隔离回移官方组件，来源及唯一半径兼容调整记录在 SOURCE.md，不升级整套 SDK、不手绘替代气泡。
- UI 只读 SQLite 和统一 business-images 缓存。用户发送先写 chat_outbox，再立即触发异步消费者；手动会话/消息刷新及已读走 chat::sync，不创建或改写 Tasks。固定店铺 tasks 仅负责周期后台同步和兜底消费。图片在业务任务内缓存后才提交消息，不创建图片任务或聊天独立图片缓存。
- chat_customers 按店铺＋平台客户标识隔离，国家与语言分开保存；默认可按国家推导语言，手动设置不被后续资料同步覆盖。翻译目前仅保留 TranslationProvider 接口、双语存储及渲染位置，没有实际翻译服务，当前按原文发送。
- HTTP 使用店铺绑定代理，图片沿用所属任务捕获的代理配置；代理变化则拒绝继续，不切换出口。发送缺少有效回执或网络结果不明时标记 unknown，禁止自动重发，用户可核对后取消待发记录。会话详情按平台消息 ID 对账，不按相同文本猜测送达。详见 docs/chat.md。


## 代理链与代理设置（2026-10-29）

- 代理设置采用已确认 `docs/designs/rv-proxy-list-detail-chain-v1.png` 的左右布局：左 ListItem，右单条 Form/Field；不自绘控件，不用顶部代理选择下拉替代列表。
- 每个代理配置一次，可选 front_proxy_id 引用已有单代理；最多一个前置，禁止自身、循环及二级前置，被引用记录删除需先解除引用。店铺绑定最终出口。
- 链式代理使用用户自行安装的外部 sing-box，RV 不打包或自动安装；只管理自己启动的实例，不改用户其它实例。单代理保持原网络方式。全部业务及 IP 探测使用完整链路，失败不直连。
- 排他开关针对当前记录作为最终出口的任务并发。细节与进程回收见 docs/proxy-chains.md。

- 代理详情「协议」用原生 Select 支持 HTTP / SOCKS5；前置与出口各自使用记录协议，无字段旧记录按 SOCKS5。HTTP 支持通过 CONNECT 访问 HTTPS 目标，不根据端口自动推断、不失败后自动切协议。密码按用户要求明文输入，加密保存。

- X-Session-Id 按 access token 绑定并明文保存在 SQLite app_request_sessions，主键为店铺和 token SHA-256；相同 token 跨任务、重启复用同一小写 UUID v4，token 变化时使用对应的新 Session，不把 Session 放入加密凭据 blob。业务、聊天、头像共用请求头入口。Locale 来自 ISOLocaleDefaultKey，Accept-Language 独立来自 CurrentLanguageCodeDefaultsKey，不从 Locale 推导。

- 店铺 UI 的刷新资料、商品、订单及店铺通知读取走 `shop_sync::synchronize` 共享 Tokio 异步执行器，不创建、认领、重启或修改 Tasks。Tasks 负责定时/延迟/周期调度；手动与定时共用业务及 HTTP 管线。商品/订单页使用原生刷新图标按钮、loading 和统一 Notify；结果提交后经 EventBus 刷新页面。每轮读取在后续 API/图片请求前核对手机交接归属，避免手动操作绕过 Phone 模式。

- 聊天手动操作复用共享异步执行器；文字/图片按会话串行、先保存待发箱、按平台消息 ID 对账，不重放 unknown/submitted。手动 sending 使用 started_at 的 90 秒恢复窗口，不以没有 task_id 判为中断；其 HTTP 执行预算 75 秒。会话与消息刷新按钮各自 loading，错误走统一 Notify；标记已读后查询服务端未读数，不按本地会话计数。

- 店铺头像确认后通过 `avatar::set` 立即异步上传/绑定/回读，不创建 Tasks；原生头像按钮 loading 持续到操作结束，错误统一 Notify。旧头像调度记录仍可执行，复用同一 HTTP 管线。通知页提供显式异步刷新按钮，仅拉取数据，不标记通知已读。

- 消息聊天使用单个独立原生窗口，保持原三栏布局与初始内容尺寸；重复入口激活已存在窗口，不切换主窗口 Modal。聊天窗口自己承载 Root 的 Dialog/Notify 层，输入框与输入订阅归属该窗口，关闭/重开清理按窗口 ID 隔离并恢复持久草稿。复用现有聊天控制器/EventBus，不另起 Tasks 调度器。

- 代理换 IP 统一走 ProxyManager 的 Rotate/NeverSeen 入口，内部按 ProxyRotation 分派 HTTP URL 或密码 Session。密码规则默认 `_session-`，仅替换紧随标记的字母数字串并保留长度，其它参数不变；原子保存后重建含前置的完整连接，再观察出口。旧记录默认 HTTP；与账号 X-Session-Id 无关。保存配置和普通检测不触发换 IP。

- 排他代理的普通业务入口按真实 virtual App IDFV 记住最后使用者（进程内、跨任务租约释放保留）：A→A 复用，A→B 换 IP 成功后才发送 B 请求；共享代理不自动换。店铺/聊天/头像/独立图片共用异步入口；任务内部图片复用已捕获链路。HTTP 换 IP 同样清理旧池连接，密码换 IP 沿用核验后的新密码完整链路；失败不发送下一 app 的业务请求、不改变任务开关。

- 会话工具栏 Copy 旁的「粘贴后触摸」为原生图标切换按钮，仅 SessionView 内存状态，默认关闭、不持久化、不放设置。关闭时 Command+V 正常粘贴但不安排后续触摸；开启才使用既有触摸计划。关闭按钮立即取消既有计划并释放已按下的指针。

- 主窗口设备列表在在线状态点旁复用 SessionView 的锁屏/解锁原生按钮，状态来自设备 HTTP 控制推送，不依赖打开 VNC 窗口。未知/离线禁用；密码设置弹窗在主窗口 Root Dialog 层生成前同步，沿用现有密码存储与锁屏流程。
- VNC 密码与锁屏密码统一在 credentials 表的同一设备 ID 记录加密保存；已保存设备的锁屏密码只按稳定 connection_id 获取，不再因 IP/端口编辑失效。无设备 ID 的临时连接仍按端点隔离；修改设备 IP 不清密码或其他设备设置。

## Token 来源（2026-10-29，覆盖早先按接管模式禁止业务请求的约定）

- 业务接口与任务能否请求只看当前凭据，不因 Phone/RV 接管模式限制。备份交接流程仍独立，显式停止的任务保持停止；不能把“获取新 Token”当成启用所有任务。
- app_token_sources 的缺省来源为 RV，存在记录则来源为指定设备（删除来源设备后保留手机来源且 device_id 为 NULL，不回退 RV）。来源为手机时 RV 不调用刷新接口、不消费其 refresh token；手机实时 access token 不改写本地/手机备份或 refresh token。
- 手机 token 通过现有已认证 HTTP/2 `/api/apps/access-token` 读取，返回 Bundle+Shadow 当前 IDFV+access token+过期元数据/必要请求头，不导出 refresh token。保存事务复核来源设备/Bundle/IDFV；同 token 已被401拒绝时不能用重新读取同值清除拒绝。
- 获取触发：店铺面板手动按钮、设置手机来源、401/依赖任务等待、手机控制连接重连；每30秒检查缺失/已拒绝/过期元数据的手机来源令牌，不对已有有效缓存逐请求查询。相同店铺的并发取令牌合并，自动请求至少间隔10秒；读取不会自动打开/刷新手机 App。
- token 失效后业务任务以 pending+error_code=access_token 等待，不改 enabled/cron；新凭据提交后清除等待标记并经 Credentials EventBus 唤醒。RV 在途刷新提交时必须在同一事务核对来源和预期 token 对，不能覆盖用户已切换/手机新提供的凭据。

- 首次从手机 state 导入凭据时，默认 Token 来源为对应手机，直接使用导入的 access token；已有凭据的重复导入不重置用户来源。有效缓存不会因来源通知/控制连接重连而被自动重新读取；手动获取仍可强制读取。会话保持初始化从正常 cron 下次时间开始，不再于初始化后一秒立即刷新。

- 刷新定时任务的“RV 接管”唯一判据为 TokenSource::Rv；调度、任务认领、HTTP 发送前及刷新结果事务提交均核对来源。HTTP 发送前同时核对当前存储的 access/refresh token 对，拒绝排队期间已被替换的旧 refresh token；手机来源只可执行业务请求。

- 明确手机→RV 交接（备份行 Monitor 与 Token 来源菜单 RV 共用）必须重新读取所选备份自己的最新完整 state，非 active 不取当前 live token、不恢复手机环境。此次交付的 access/refresh token、scope、到期及请求上下文覆盖 RV 旧凭据，与 TokenSource::Rv 同事务提交；失败保持原来源和凭据。刷新备份列表只更新目录关联，普通 state 同步继续保留 RV 当前凭据。交接不重启用户停止的任务，不强制立即刷新；启用中的 Token 等待任务按新凭据唤醒。此条覆盖上文旧交给 RV 仅按 Phone 模式开放及全部任务强制停止的规则。

- 手机交接成功时 app_execution.phone 与 TokenSource::Phone 同一事务提交，不能保留 RV 刷新责任；RV 采用手机备份最外层使用 BEGIN IMMEDIATE，嵌套事务使用 SAVEPOINT，避免自动读取手机 token 时先读后写升级失败。2026-10-30 已用现有 schema14 数据库和7Plus完成 token 轮换、双向交接、非 active 采用、Shadow恢复与手机登录、再交回RV刷新 HTTP200；桌面 GUI 未重构建。最终手机 Vinted 已停、目标 TokenSourceRv，历史备份不可当最新有效凭据重放。
