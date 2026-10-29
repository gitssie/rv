# 店铺头像

店铺详情操作栏的相机按钮提供「设置头像」：选择本地图片，显示居中正方形裁切后的预览，确认后通过 `avatar::set` 在共享异步执行器中立即上传和设置，不创建或等待 Tasks。取消不发请求。支持现有图片解码器可读取的图片，最大 8 MB；处理 EXIF 方向并居中裁切为 800×800 JPEG，透明区域转为白色。最新示例为 [雏菊头像](assets/avatar-daisy-vase.png)。

准备后的图片内容随异步操作保存，不依赖原文件继续存在；UI 持续显示 loading，完成或失败使用统一 Notify。HTTP 全部通过店铺绑定代理和同一个代理租约，读取统一 access token 及对应的持久 Session ID，不自行刷新 token。中途切换手机接管停止后续请求。

操作依次读取当前账号、上传图片、绑定头像，再重新读取资料核对图片身份、下载到共用 `business-images` 缓存。成功后原子更新店铺资料，通过 EventBus 更新卡片、详情和聊天店铺列表。失败不自动重复提交；结果未知时提示刷新资料核对。旧的一次性头像调度记录保留执行支持，手动 UI 不再生成该记录。

## 已验证协议

2026-10-28，使用手机当前 Keychain 记录中的令牌对现有账号实测设置头像成功，并下载服务器头像目视核对为生成的橘猫。未启动 RV 自动任务，未调用令牌刷新接口。

1. `POST https://www.vinted.com/api/v2/photos`，multipart 字段 `photo[type]=user`，图片字段 `photo[file]`。成功回执在根节点提供 `temp_uuid` 和 `high_resolution.id`。
2. `PUT https://www.vinted.com/api/v2/users/current`，JSON `{"user":{"photo_temp_uuid":"上传回执中的 temp_uuid"}}`。仅提交头像引用，不覆盖昵称、地址等其它资料。
3. `GET https://www.vinted.com/api/v2/users/current`，带 `Cache-Control: no-cache`。普通读取可能暂时返回旧头像。确认当前账号以及 `user.photo.high_resolution.id` 与上传图片一致；头像 `photo.id` 与上传回执 `id` 不同，不能用这两个 ID 比较。

以上协议使用实际 API 响应验证；GPUI 入口做类型与样式检查，未构建或部署新版 RV。

2026-10-29 使用 imagegen 新增雏菊花瓶头像，并在 `michelley299` 实测上传、绑定及回读确认成功；运行版本地头像缓存已更新。后续手动入口与调度分离的实现以本地四步 HTTP 服务验证，不重复向真实账号提交头像。
