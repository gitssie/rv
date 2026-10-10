# ZXTouch 功能操作与文本输入

RV 会话工具栏的扳手「功能」入口打开紧凑操作菜单。顶部仅保留 Apple / App 环境两个横向分类，下方以登录与退出、资料与邮箱、授权与设备等分组显示 28px 操作行，配有图标和展开箭头。功能列表沿用会话工具栏的深色面板与文字颜色；操作图标 15px，分类按钮 26px，无搜索框、无底部说明。键盘及截图弹出菜单同样使用深色面板、26px 操作行和 15px 图标。

选择操作后，单独的文本输入弹窗出现在菜单左侧，包含等宽字体输入框、复制示例、取消和小型蓝色执行按钮。宽度不足 900px 或可用高度不足 480px 时只显示当前输入弹窗；取消后返回菜单。宽窗口的 Fit 模式为操作区留出空间，让手机画面保持可见。功能菜单与助手互斥，避免同时占用手机画面的空间。

## 一次粘贴的输入格式

需要用户提供资料的操作使用单个格式化文本框；接口用于定位当前账户的参数由 RV 从手机读取，不要求用户粘贴。点击「复制示例」得到当前功能的模板；修改后一次粘贴，再点击「解析并执行」。复制示例不会发起手机操作。

登录 App Store 可以使用一行：

```text
demo@example.com----填写密码----https://code.example.test/get?id=123
```

验证码链接可省略。密码包含 `----` 时使用多行格式：

```text
账号=demo@example.com
密码=填写密码
验证码链接=https://code.example.test/get?id=123
```

App 环境采用先选 App、再操作的流程。工具栏只负责快捷启动，其加号只编辑快捷栏。功能菜单的「App 环境」使用独立保存的 App 列表，加号在当前面板打开环境 App 目录；两处互不修改。环境目录排除 Apple 系统 App 和 TrollVNC 自身，签名与备份资格仍由手机端最终校验。功能列表和 App 网格共用分类导航，网格页直接使用 Tab 切换，不显示返回按钮；单个 App 的备份页保留返回 App 列表的按钮，工具栏「功能」也可直接返回；关闭 App 面板后再打开「功能」，默认显示普通功能菜单。进行中或结果未知的操作仍保留原查询入口。

所选 App 页面提供打开、停止、保存当前环境，以及备份记录。记录行提供切换、编辑名称与备注、复制标识和删除；空白基线、环境详情和运行状态位于「更多」。App 网格、备份页面与功能列表统一采用深色面板；网格与备份页面均不显示搜索框。App 和备份标识由页面自动带入，用户只需填写名称、备注等可选参数：

```text
名称=工作账号 A
备注=备用环境
```

删除需再次点击「确认删除」，当前快照和空白基线不提供删除入口。恢复未完成时显示提示及显式恢复入口，禁用保存当前环境。结果未知时保留原操作查询入口，不自动重放。备份操作进行中也禁止工具栏启动、停止或重启 App。

图标复用原有 App 图片资源：工具栏 24px、常用网格 38px；操作按钮保持 26px 高。小窗口进入操作时显示单独文本编辑面板，取消返回备份列表。

默认先保存当前环境。如果需要放弃当前环境，填写 `放弃当前环境=是`，同时省略名称与备注。退出账户采用 320px 紧凑面板，本机数据使用无描边的 24px 分段选择，必须明确点选「保留」或「移除」后再执行；没有默认选择。写操作成功后收起输入与执行按钮，阻止同一编辑页重复提交；退出操作同时收起本机数据选择，显示简短结果及实际完成的范围。操作完成后直接展开原始 JSON 详情并提供复制入口；常见 Apple 错误显示可读提示。

多行字段可以调整顺序；中文字段名和对应的原生参数名均可使用。每行按第一个 `=` 拆分，所以密码中的 `=`、链接查询参数可保留。密码不自动去除首尾空格。解析会检查必填项、重复字段、未知字段和参数长度，错误消息不回显输入内容。

## 已接入的操作

| 分类 | 操作 |
| --- | --- |
| Apple | 登录 App Store、登录 iCloud、退出 Apple 账户、退出 iCloud、关闭查找我的 iPhone、修改姓名、添加/验证联系邮箱、设置转发邮箱、查看/停止 App 授权、查看关联设备、移除其它设备、读取账户资料/姓名/联系邮箱 |
| App 环境 | 保存、切换、创建空白基线、列出、编辑、删除备份，打开/停止 App，读取环境详情、确认是否停止 |

共 26 项操作。查询功能由用户按需执行；返回结果可展开查看或复制。操作定义集中在 `crates/rv-session/src/functions/spec.rs`，新增定义可复用菜单、分类、文本解析和执行流程。

## Apple 接口映射与当前账户

以 `TrollVNC/src/TVNCAppleService.mm` 的 task 31 参数定义及 `vendor/apple/docs/interface.md` 为依据：

| 操作 | 原生方法 | 用户输入 / 目标来源 |
| --- | --- | --- |
| 退出 iCloud | signOutICloud | 手机当前主 iCloud；明确选择本机数据策略 |
| 退出 Apple 账户 | signOutAppleID | 当前 phone identity / 主 iCloud；仅媒体登录时使用已认证的当前媒体账户；明确选择数据策略 |
| 关闭查找 | disableFindMy | 当前主 iCloud；只输入该账户密码 |
| 修改姓名 | setName | 当前账户；只输入名、姓 |
| 添加邮箱 | beginAddingContactEmail | 当前账户；粘贴邮箱 OAuth 资料可自动验证，仅填邮箱则手动验证 |
| 验证邮箱 | verifyContactEmail | 原添加操作的 challenge 自动带入；只输入六位验证码 |
| 设置转发邮箱 | setForwardingEmail | 当前账户；只输入邮箱（Sign in with Apple 的统一转发目的地） |
| 授权查询 / 停止 | querySignInWithAppleApps / stopUsingAppleID | 当前账户；停止操作额外提供目标 App 的 Bundle ID |
| 设备查询 / 移除其它设备 | queryICloudDevices / removeOtherICloudDevices | 当前账户；原生服务保留当前设备 |
| 姓名 / 邮箱查询 | queryName / queryEmail | 当前账户，无资料输入 |
| 登录 App Store / iCloud | signInAppStore / enableICloud | 用户提供新登录资料，按原生参数要求解析 |

提交当前账户操作前，通过独立的 `queryState` 只读操作确认目标。仅采用原生域摘要，忽略历史媒体记录；iCloud 退出不得回退为 App Store 退出。账户读取失败、未登录、需要统一身份的操作出现冲突时不提交目标操作。原生接口仍接收明确 Apple ID，并保留自己的新鲜身份检查、Find My 限制和未知结果写入围栏。

添加邮箱完成后，有 OAuth 资料时自动读取验证码并验证，否则 UI 转入同一「添加联系邮箱」标题的手动验证码步骤；challenge 与账户目标留在运行时内存。重新连接后可查询原添加操作的结果恢复 challenge，验证仍由原生服务校验其账户绑定与过期时间。不会把界面里随意输入的验证标识送入服务。

## 执行与验证

- 功能连接使用 RFB 认证后下发的 ZXTouch 控制能力与令牌，通过 task 33 认证，再调用 Apple task 31 / AppState task 32。端口使用连接配置覆盖值或服务端公布值。只读连接不允许执行。
- 功能调用使用独立的异步 TCP 短连接：每次提交、查询结果、提交验证码都重新连接并认证，收到回复立即关闭。同一时间只跟踪一个原生操作，操作标识保留在内存中，不依赖 TCP 会话。App 工具栏命令同样逐次建立短连接，不复用空闲 socket；已发出的失败请求不自动重放。
- 登录进入真实验证码挑战后，才会读取用户提供的链接；每个挑战自动读取一次，失败后可手动重试或输入六位验证码。支持纯六位文本，或 JSON 中 `code` / `verification_code` / `verificationCode` / `otp` 字段，允许位于 `data` / `result` 内。数字字符串保留前导零；不从任意文本或数字 ID 中猜验证码。
- HTTP 请求限制超时、响应体大小，HTTPS 校验证书，禁止自动跳转。链接只供 RV 取码，作为参数移除后不发给手机原生服务。
- 提交后清空输入框；账户、密码和取码链接不写入地址簿、输入历史或调试输出。现有 VNC / 锁屏密码继续使用本地配置与密码文件。
- 断线、超时和原生部分完成/结果未知会保留原操作标识，并阻止重复执行。重新连接后用户可查询原操作；不会自动重放登录、切换等操作。

## 实际界面预览

- [功能菜单](zxtouch-functions-implemented.png)
- [浅色系统主题下的统一深色功能面板](zxtouch-function-menu-light.png)
- [退出当前 iCloud（无文本输入）](zxtouch-icloud-logout-light.png)
- [退出完成（紧凑结果）](zxtouch-logout-result-compact.png)
- [登录文本输入](zxtouch-login-text-implemented.png)
- [常用 App 网格](zxtouch-app-first-common-dark.png)
- [App 备份记录（深色）](zxtouch-app-first-records-dark.png)
- [App 备份记录（浅色）](zxtouch-app-first-records-light.png)
- [保存环境文本输入（小窗口）](zxtouch-app-first-save-light.png)
- [小窗口文本输入](zxtouch-functions-compact-implemented.png)

截图由实际 GPUI 界面渲染，手机区域使用模拟画面。测试覆盖文本解析、只读限制、本地输入隔离、小窗口按钮可见性、真实 TCP 的认证与操作查询、验证码 HTTP 获取、提交拒绝处理和下一操作重新建连。手机账户服务及 App 环境数据的实际执行仍需连接配套 ZXTouch 在真机验证。

快捷启动与 App 环境分别保存列表：环境加号仅管理可备份的 App，不修改快捷栏；快捷栏加号不修改环境列表。快捷选择器、功能与环境网格均无搜索框。通知列表使用 28px App 图标、App 名称、时间与标题/正文，自动接收改为标题栏铃铛开关。未知图标沿用统一占位，安装列表中的通知来源会按需请求手机图标。

查询接口（query*、list、describe、status）选中后直接读取，不显示取消/执行确认行；读取期间显示旋转 loading，阻止重复提交。完成后可主动刷新，失败后可重试；写操作保持明确提交。

查询结果面板：完整原始 JSON 默认展开，不做业务字段解析、不隐藏空字段、不截断字符；仅对字符串 token 施加轻量颜色。标题栏沿用现有图标，14px 图标、24px 点击区域，提供刷新、复制和关闭。面板宽 360px，结果区按窗口可用高度限制并支持水平/垂直滚动；查询页不显示成功摘要和详情切换，写操作的详情入口保持原流程。

结果展示与复制排除顶层请求信封（method/action/params/request）和 operation_id，手机返回的 result/error/progress 等内容完整保留，嵌套同名字段不会删除。结果栏关闭按钮只收起当前操作栏，功能菜单保留；原操作未完成或结果未知时保留其恢复入口。Esc 在可关闭结果栏时先关闭结果栏。

读取过程中只显示 loading，不渲染 running 状态 JSON，也不显示“查询原操作结果”。原始返回区仅在操作完成且未继续读取时展示；结果未知或断线后的恢复入口仍保留。


## 联系邮箱自动添加与验证（2026-10-10）

功能菜单只显示「添加联系邮箱」，包含添加与验证两个原生步骤。一次粘贴：

```text
contact@outlook.com----邮箱密码----00000000-0000-4000-8000-000000000001----刷新令牌
```

也可使用多行 `邮箱`、`密码`、`客户端ID`、`刷新令牌`。密码为四段导入格式的一部分；Microsoft 读取邮件采用 OAuth 刷新令牌授权，不发送密码、不尝试密码登录。只填写邮箱则在同一标题的操作面板内手动输入六位验证码。

客户端先解析邮箱域名、选择服务适配器、检查 OAuth 邮箱身份/读取权限，随后再次核对手机当前 Apple 账户，刷新邮件基线，再发送 `beginAddingContactEmail`。收到原生 challenge 后等待新邮件，自动调用 `verifyContactEmail`；验证前再次查询手机账户，账户改变时停止。两次写操作各只执行一次，ZXTouch 始终使用独立的短认证连接。断线或未知结果不会重新添加或重发验证；OAuth 资料不写入连接配置，不发送给手机。

自动取码时显示小型 loading 与进度；失败后保留原联系邮箱验证步骤，用户可手动填写验证码。整个流程沿用原 360px 深色操作面板、24px 图标按钮与 26px 操作按钮，无新增表单。

模块边界：

- `functions/mailbox/mod.rs`：邮箱类别路由、凭据解析、`MailboxProvider` / `CodeMailbox` 合约、发信前基线刷新与 150 秒取码超时。
- `functions/mailbox/microsoft.rs`：Microsoft OAuth refresh grant、Graph（Mail.Read + User.Read）及 IMAP（IMAP.AccessAsUser.All）适配。已授权 Graph 则优先使用 Graph，否则尝试 IMAP；不会替用户增加授权权限。
- `functions/mailbox/message.rs`：传输无关的 Apple 邮件、收件人校验与验证码提取；按邮件内容顺序取第一个六位验证码。
- `functions/code_url.rs`：已有的 HTTP 验证码链接源，独立校验 URL、限制回复大小并读取明确 code 字段。
- `functions/mod.rs`：原生操作和上述取码模块的流程编排，不处理 OAuth 协议或邮件 MIME。

目前自动分流覆盖 Outlook / Hotmail / Live / MSN 等已列出的 Microsoft 消费邮箱域名。未注册域名会在添加前提示未接入，不会猜测协议；新服务应新增 provider 文件，并注册精确域名/自己的凭据验证规则。企业自定义域名和未列出的地区域名需显式接入，不能仅凭名称包含 outlook/hotmail 误判。

Graph 基线使用邮件 ID 与接收时间，允许 5 秒时间精度/时钟偏差；IMAP 使用 UIDNEXT / UIDVALIDITY、EXAMINE 和 BODY.PEEK，避免标记已读。不提交旧邮件、不猜测七位以上数字片段；同一封邮件存在多个数字候选时只取第一个六位验证码，不合并正文的纯文本和 HTML 版本。客户端固定禁用 IMAP codec 的原始帧 trace，避免外部日志配置输出认证信息或邮件正文。测试均使用模拟 OAuth / 邮箱 / ZXTouch 服务，不使用真实账户变更。

协议参考：[Microsoft OAuth refresh grant](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow)、[Graph messages](https://learn.microsoft.com/en-us/graph/api/user-list-messages?view=graph-rest-1.0)、[Microsoft IMAP OAuth](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth)。

处理态与完成态（2026-10-10）：提交后保留原资料，处理期间输入框只读，仍可选中复制；执行、复制示例和输入引导收起，显示小型 spinner 和当前步骤。验证码使用独立输入框，不覆盖原资料。资料仅保留当前操作内存，不写入配置或日志。最终 completed 回复自动展开原始详情，无需二次点击；等待联系邮箱验证码的中间步骤不作为最终完成。

真机调试（2026-10-10）：用户提供的 Outlook OAuth 授权通过 Graph 邮箱身份与读取检查。手机正式 `cazer-apple-email-actions-v5` 返回联系邮箱验证码挑战，新邮件已送达，但其标题为繁体「驗證你的電子郵件地址」，原客户端的简体/英文关键词规则将它排除。新增繁体邮箱和验证关键词，保留 Apple 发件域、收件人、新邮件基线及唯一码检查；回归测试在修复前返回 None，修复后通过，登录验证码和多个候选码仍拒绝自动提交。

修正规则后的真机对照取得新的邮件验证码，在同一原生进程提交验证，返回 `outcome=succeeded`；随后全新服务端查询确认目标邮箱在 reachable_emails 中（列表从 0 项变为 1 项）。此测试验证原生添加/验证接口与 Graph 取码；通过已连接 RV 的完整 UI/ZXTouch 流程尚需单独验收。此前 `org.trollvnc.apple/code=6` 本轮没有复现，不能将繁体漏识别等同于该原生错误的原因。凭据、验证码和邮件正文未写入调试文件或日志。

最终取码规则（2026-10-10，按用户确认）：取消语言和邮件用途关键词过滤。邮箱 provider 先核对 Apple 发件域、目标收件人和新邮件基线；处于手机原生联系邮箱挑战时，从邮件标题/正文按顺序提取第一个六位 ASCII 数字串。MIME 使用第一个能取出验证码的正文部分，多个不同码不触发歧义提示；七位以上数字不截取，前导零保留。Graph 与 MIME/IMAP 共用该提取函数。22 种语言示例、无关键词邮件、标题中的验证码及多码/长数字边界回归覆盖；这些语言样例使用合成验证码，不代表每种语言均做了真机发信测试。

邮件读取范围（2026-10-10，按用户确认）：每次只读取最新第一封。Graph 请求 `receivedDateTime desc` 且 `$top=1`；IMAP 使用 `UID FETCH *`，只取收件箱最新一封。只从该封邮件提取验证码，不扫描或合并更早邮件；第一封无验证码或未通过来源/新邮件检查时继续等待新邮件。回归覆盖第一封返回验证码、第一封无验证码/来源不匹配不回退旧邮件，以及更早邮件的不同码不影响第一封。

验证码回填（2026-10-10）：自动收到验证码后先通过独立事件回填现有验证码输入框，再提交手机原生验证。处理期间显示只读验证码，完成后仍保留显示；原邮箱资料不被覆盖或清空，手动接续验证沿用已回填的输入。该事件的 Debug 输出隐藏验证码。回归覆盖自动取码事件、输入回填、原资料保留及完成后仍可见。

错误说明与首读等待（2026-10-10）：手机 ZXTouch 6000 的实际只读 queryEmail 错误返回 org.trollvnc.apple/code=7，无 message；同机原生工具返回明确的 primary phone identity 不匹配说明。TVNCAppleService 原错误序列化只保留 domain/code。现通过 TVAppleDiagnosticMessage 导出模块生成的净化说明，作为 error.message，排除原生任意 userInfo/description；code=6 的传输未完成附带 HTTP/terminal/finished/model/transport_failed 元数据。已经丢弃的历史操作说明无法从数字错误码恢复，截图 code=6 的具体原因仍未确认。客户端收到联系邮箱挑战后等待3秒再首次读取，之后每3秒只读最新邮件，保留150秒整体等待上限。

操作面板关闭（2026-10-10）：关闭按钮不再因 busy/uncertain/operation receipt 存在而禁用。这些状态下仅隐藏功能面板，保留选中操作、输入、验证码、原始结果和查询凭据；再次打开功能恢复原操作，仍禁止重复执行。空闲且没有待查询操作时沿用关闭编辑器回到功能菜单的行为。

手动重新取码（2026-10-10）：联系邮箱验证码步骤新增26px「重新获取验证码」入口，使用当前操作内存中保留的四段邮箱授权资料重新认证邮箱，读取最新第一封邮件的第一个六位码并回填原验证码框。不发送新的手机邮件挑战，不调用 verifyContactEmail，不更改手机未知结果写栅栏；已有邮件可能返回同一个验证码。该手动读取允许读取当前最新邮件，而自动添加流程仍使用发信前新邮件基线。读取期间只读并显示loading；完成后恢复验证码框与按钮，保留原邮箱资料、验证码、原验证结果。缺少原OAuth资料时明确提示无法自动取码。HTTP200/terminal=0/finished=0/model=0 表示原生加载未完成，不能等同验证码无效；未知验证仍须新查询实际邮箱状态，不能重复提交。

独立关闭操作窗（2026-10-10，修正）：editor_hidden 将左侧操作窗可见性与整个功能菜单、选中操作、执行状态分离。左侧×只隐藏左侧编辑器，右侧菜单继续显示；菜单点击原功能恢复原编辑器，email-verify 对应菜单的email-add项，保留资料、验证码、原始结果和查询凭据，不发起新请求。整个菜单×仍关闭整个功能菜单。未知写结果期间禁用其它写操作，但原菜单项保持可点击供恢复查看。原生 previous mutation 错误来自进程级 uncertainMutation，重新读取同一操作或重连不会解除；不把客户端隐藏/关闭面板当作清除手机未知结果的授权。

2026-10-10 重新绑定初始化：用户主动再次beginAddingContactEmail时，服务串行核对当前账户，清空旧原生challenge/model及认证/签名会话，重新准备原生认证环境并fresh读取reachable_emails；目标已经验证则返回succeeded/already_verified，未绑定才发新请求与新challenge。只允许同一Apple ID的邮箱begin/verify未知结果通过新begin恢复，且新begin成功后才解除邮箱专属unknown标记；其它写操作未知结果不因此解锁。task31允许同账户替换旧邮箱挑战，旧challenge立即不可提交；busy仍拒绝并发，结果历史继续保留。RV提供独立「重新发送绑定」命令，保持原资料并刷新新邮件基线，旧验证码不自动重放。只读取码按钮仍不发新邮件。主机回归/编译与真机绑定验收区分，新恢复流程尚未进行真机账户写入验证。

2026-10-10 已发布3.2-312到本地Sileo源 http://172.16.20.203:18766/，包含邮箱重绑环境初始化、同账户邮箱unknown恢复、旧challenge失效与已验证邮箱幂等成功。native模块与task31真实TCP模拟回归通过；arm64服务/RV release编译通过，尚未安装手机或进行新绑定真机写入验收。源HTTP改为普通exec持久会话启动（用户明确禁止继续使用Orca Terminal），LAN全部索引/包字节核对通过，status健康；后续同样用普通exec前台持久会话运行HTTP，不用短exec启动脱离会话的nohup。RV仅在新绑定完成成功或取得有效challenge时解除重绑前unknown，拒绝/预检失败/已知失败均保留原未知状态。
