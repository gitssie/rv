//! Interactive shop reads use the same HTTP/business pipeline as scheduled work.
//! They do not create, claim, restart, or modify scheduler tasks.
use crate::*;

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
            OfflineRunner::new(None)?.execute_business(&paths,&id,operation,plugin.as_ref(),None,false).await
        }.instrument(span).await
    }).await.map_err(|e|format!("业务执行中断：{e}"))?
}
