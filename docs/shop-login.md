# 店铺登录凭据

Vinted 登录用户名来自已验证的 `GET users/current` 响应 `user.login`，例如店铺卡片标题；数字 `user.id` 是平台用户 ID，IDFV 是 RV 店铺记录的关联键，二者都不是登录用户名。不把邮箱推断为该字段。

SQLite schema 15 新增 `app_login_credentials`：`app_id` 为主键并关联 `app_instances(id)`，`login_username` 保存用户名，`login_password` 保存本地明文密码，NULL 表示尚未知晓。密码不进入卡片元数据、全文搜索或 OAuth token/state。删除店铺时级联删除其登录凭据。

升级仅从已有 `metadata.login`（缺失时 `metadata.shop.login`）回填用户名，不猜测密码。成功的业务资料同步在原事务中更新用户名，保留已保存的密码；普通备份同步、token 刷新和接管均不覆盖登录密码。仓库 API `login_credentials` / `save_login_credentials` 用于独立读取和保存已知凭据，店铺编辑表单显示只读登录用户名及明文登录密码输入框，不遮罩、不设置显示切换。保存只更新本地记录，与标签、备注和代理配置共用事务；不会发起平台设置／修改密码请求。
