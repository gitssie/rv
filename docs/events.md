# EventBus 与本地数据变更

`rv_core::events::EventBus` 提供 Guava 风格的按类型广播：

```rust
let bus = rv_core::events::workspace_bus(&paths)?;
let events = bus.subscribe::<rv_core::events::WorkspaceChanged>();
// 后台发布自定义事件也使用同一个 API：bus.post(MyEvent { ... });
while let Ok(event) = events.recv().await {
    // 按 event.changes 的领域、店铺和会话标识读取 SQLite。
}
```

`subscribe::<E>()` 返回独立订阅；同类型的每个订阅者都收到事件。
`post(E)` 将事件送入队列，不在发布线程执行订阅者代码。`recv().await`
用于 GPUI 或其它异步执行器，`try_recv()` 用于合并队列。丢弃订阅即注销。
UI 将订阅任务保存在 HomeState 中，窗口销毁时取消。队列无界，订阅者应持续消费，
停止消费时应丢弃订阅；此总线用于本地变更通知，不承载图片、接口正文或令牌。

同一数据库规范路径的 WorkspaceStore 连接共用总线；不同数据库隔离。
SQLite update hook 只记录关注表的行标识；成功提交后由存储方法解析范围并发布
`WorkspaceChanged`，事务内部不投递。完整回滚清空待发布集合，嵌套 savepoint
回滚恢复外层集合。相同事务中的相同实体变更合并。删除行无法解析标识时发布
对应领域的全域失效，订阅者重新读取数据库。连接 Drop 另有已提交变更的兜底发布。

关注任务、App/备份/代理配置、凭据/state、执行归属、会话、消息/待发图片、
客户、商品、订单和通知表。其它业务表接入时需补充 changes.rs 中的表与范围映射，
并在成功提交的存储边界发布。业务插件统一使用 business_transaction 或已有的
complete_task 事务，不能在其闭包内自行提交/回滚或执行 HTTP。

UI 不再每秒轮询 TaskWorker。事件到达后合并队列，独立刷新任务列表、账号投影、
会话列表、当前会话及店铺数据。各投影有独立异步 generation，更新不改输入草稿。
未打开聊天时账号未读数仍从 SQLite 的服务端投影更新，不根据会话自行统计。
后台调度器的一秒检查和默认十分钟未读接口任务属于执行调度，保持原有频率。

这是进程内的提交后失效通知，不是持久化 CDC 日志；不跨进程、不重放历史。
页面初次加载及重新打开仍读取 SQLite，数据库始终是结果来源。其它进程直接改库
不会产生本进程事件。
