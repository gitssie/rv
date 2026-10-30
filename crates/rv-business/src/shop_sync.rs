//! Interactive shop reads use the same HTTP/business pipeline as scheduled work.
//! They do not create, claim, restart, or modify scheduler tasks.
use crate::*;

/// Execute the operation shown in task configuration immediately, independently
/// of its scheduler row, enabled flag and next cron occurrence.
pub async fn execute_now(paths:StorePaths,id:AppId,kind:String)->Result<(),String> {
    crate::executor::runtime()?.spawn(async move {
        use tracing::Instrument;
        let span=tracing::info_span!("shop_execute_now",app_id=%id.as_str(),operation=%kind);
        async move {
            let runner=OfflineRunner::new(None)?;
            let operation=match kind.as_str() {
                "session"=>Operation::Session,
                "refresh"=>Operation::Refresh,
                "products"=>Operation::Products,
                "orders"=>Operation::Orders,
                "notification_stats"=>Operation::NotificationStats,
                "notifications"=>Operation::Notifications,
                "chat_threads"|"chat_inbox"|"inbox"|"chat_send"|"chat_read"=>{
                    let workflow=match kind.as_str() {
                        "chat_threads"|"chat_inbox"|"inbox"=>chat::workflow::Workflow::Sync,
                        "chat_send"=>chat::workflow::Workflow::Send,
                        _=>chat::workflow::Workflow::Read,
                    };
                    return runner.chat_workflow_direct(&paths,&id,workflow).await;
                },
                _=>return Err("此操作不支持立即执行".into()),
            };
            runner.run(&paths,&id,operation).await
        }.instrument(span).await
    }).await.map_err(|e|format!("立即执行中断：{e}"))?
}

/// GPUI can await this Future directly. Network waits run on a shared Tokio pool.
pub async fn synchronize(paths:StorePaths,id:AppId,operation:Operation)->Result<Value,String> {
    if !matches!(operation,Operation::Session|Operation::Products|Operation::Orders|Operation::NotificationStats|Operation::Notifications) {
        return Err("此入口仅用于店铺资料、商品、订单和通知读取".into());
    }
    let runtime=crate::executor::runtime()?;
    runtime.spawn(async move {
        use tracing::Instrument;
        let span=tracing::info_span!("shop_sync",app_id=%id.as_str(),operation=operation.key());
        async move {
            let store=WorkspaceStore::open(&paths).map_err(|e|e.to_string())?;
            let app=store.load(&id).map_err(|e|e.to_string())?;
            let plugin=strategy(&app.bundle_id).ok_or("此 App 尚未接入店铺同步")?;
            OfflineRunner::new(None)?.execute_business(&paths,&id,operation,plugin.as_ref(),None).await
        }.instrument(span).await
    }).await.map_err(|e|format!("业务执行中断：{e}"))?
}

/// Refresh one platform product through the same credentials, proxy and commit pipeline.
pub async fn refresh_product(paths: StorePaths, id: AppId, product_id: String) -> Result<Value, String> {
    crate::executor::runtime()?.spawn(async move {
        use tracing::Instrument;
        let span = tracing::info_span!("product_refresh", app_id=%id.as_str(), %product_id);
        async move {
            let store = WorkspaceStore::open(&paths).map_err(|e| e.to_string())?;
            let app = store.load(&id).map_err(|e| e.to_string())?;
            let plugin = strategy(&app.bundle_id).ok_or("此 App 尚未接入店铺同步")?
                .product_refresh(&product_id)?;
            OfflineRunner::new(None)?.execute_business(&paths, &id, Operation::Products, plugin.as_ref(), None).await
        }.instrument(span).await
    }).await.map_err(|e| format!("商品刷新中断：{e}"))?
}
