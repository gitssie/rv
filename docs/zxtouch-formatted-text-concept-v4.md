# RV 功能参数文本输入 v4

使用内置 imagegen 生成，尚未实现 UI。截图使用虚构资料，用户提供的实际账户、密码与验证码链接未写入设计文件。

## 输入约定

每个功能只展示一个可粘贴的文本框，并提供“复制示例”。默认使用便于粘贴的单行格式；复杂参数使用字段名多行格式。两者均为同一个操作的输入方式，不要求用户逐项填写控件。

### App Store 登录

快捷格式：

```text
demo@example.com----示例密码----https://code.example.test/get?id=123
```

两段账户/密码也可使用，验证码链接缺省时由用户输入验证码：

```text
demo@example.com----示例密码
```

带字段名格式：

```text
账号=demo@example.com
密码=示例密码
验证码链接=https://code.example.test/get?id=123
```

### iCloud 登录

```text
账号=demo@example.com
密码=示例密码
设备密码=示例设备密码
验证码链接=https://code.example.test/get?id=123
```

设备密码是该原生操作的必要参数，不应与账户密码混淆。

### 修改姓名

```text
账号=demo@example.com
名=Adaline
姓=Hill
```

### 设置转发邮箱

```text
账号=demo@example.com
转发邮箱=forward@example.com
```

### 保存 App 环境

```text
名称=工作账号 A
备注=备用环境
```

目标 App 使用当前所选 App；当前已关联快照的 save 按原生语义更新关联备份，不应暗示此格式总会新增一个快照。

## 解析与交互

- 依据所选操作的参数 schema 解析，不把一段文本猜测为多种操作；新增功能增加 schema 和示例，不增加固定表单。
- 快捷格式仅接受该功能规定的字段数量。密码含有分隔符 `----` 时使用字段名格式。
- 多行格式每行仅在首个 `=` 处分隔键和值，URL 的查询参数可保留 `=`。键可以修剪空白，密码值不得自动修剪或截断。当前版本不支持值内换行。
- 必填字段缺失、重复字段或未知字段明确定位错误，不执行部分解析结果。
- 密码在解析成功后用遮挡展示，遮挡文本不覆盖内存中的原值；提供临时查看/重新粘贴入口。截图中的圆点是展示方式，不能被当作实际密码。
- 功能菜单项与执行按钮保持紧凑。选择功能后才显示文本框；执行后保留必要结果，避免变为状态看板。
- 登录密码和链接不进入提示词、历史日志、收藏或示例。验证码链接可包含访问凭据，不能作为普通日志打印。

## 验证码链接的实现边界

这是 RV 客户端新增流程的设计。现有 Apple 原生服务只接受外部提供的验证码，不接受或读取 URL。后续需由 RV 在收到真实验证 challenge 后读取用户输入的链接并提取验证码，再提交到原生服务；不能假定登录开始时就需要验证码。接口返回格式尚未确认，当前设计未读取用户提供的真实链接。连接中断、超时或结果未知时不能重放登录操作；后续查询原操作结果。

## 最终 imagegen 提示词

```text
Use case: ui-mockup. Edit the referenced RV compact function menu design. Preserve the entire dark macOS RV window, live iPhone view on left, compact toolbar, small categorized function browser and search at upper right. Keep action rows small. Main change: replace the previous traditional two-field parameter form popover with a SINGLE formatted-text paste input for "登录 App Store". Do not create a dashboard or big permanent side panel.
In command browser highlight the row "登录 App Store" instead of "设置转发邮箱". A compact contextual popover immediately left of the menu, width about 390px and height about 245px, anchored to the highlighted login row, sits entirely on empty dark canvas if possible. Header "登录 App Store", small close icon.
Inside popover a small label "粘贴登录资料". Under it ONE multiline monospace text box with three lines, accurate text:
"账号=demo@example.com"
"密码=••••••••"
"验证码链接=https://code.example.test/get?id=123"
This is one paste box, NOT three separate fields. Password is masked after parsing. Use fictitious data exactly as above; never include real user credentials or real verification service links.
A small helper line beneath: "也支持：账号----密码----验证码链接". Beneath it a quiet small example-format affordance "复制示例" and footer right buttons "取消" and small blue "解析并登录", normal native 26-28px height, not wide blue CTA. If there is room, muted footer line "需要验证时，再读取链接。" This is a proposed client workflow, not current backend functionality.
No read-only account cards or parsed data summary cards. No automatic giant progress section, no status dashboard, no conventional credential form fields. User wants ONE convenient formatted text input, paste once and execute. Keep remaining compact action browser categories 常用 Apple App环境 脚本 识别 设备 and action menu unchanged. Native charcoal matte surfaces, thin borders, small type 12-13px, quiet blue accent, high fidelity app screenshot, no decorative graphic or marketing captions.
```

