//! Shared runtime for interactive business operations, separate from task scheduling.
pub(crate) fn runtime()->Result<&'static tokio::runtime::Runtime,String> {
    static RUNTIME:std::sync::OnceLock<Result<tokio::runtime::Runtime,String>>=std::sync::OnceLock::new();
    RUNTIME.get_or_init(||tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("rv-business").build().map_err(|e|format!("无法启动业务执行器：{e}"))).as_ref().map_err(Clone::clone)
}
