mod app {
 #[path="/opt/workspace/github/cazer/rv/crates/rv-app/src/app/chat/test_support.rs"] mod fixture;
 pub fn seed(paths:&rv_core::StorePaths,device:rv_core::ConnectionId)->Vec<rv_core::workspace::AppId>{fixture::seed(paths,device)}
}
fn main()->Result<(),Box<dyn std::error::Error>> {
 let paths=rv_core::StorePaths::in_dir(std::path::PathBuf::from(std::env::args().nth(1).ok_or("missing test directory")?));
 let mut book=rv_core::AddressBook::load(paths.clone())?;
 let phone=rv_core::Connection::new("GUI Test Device","example.invalid",5900);book.upsert(phone.clone());book.save()?;
 let ids=app::seed(&paths,phone.id);let mut store=rv_core::workspace::WorkspaceStore::open(&paths)?;
 for id in ids {for task in store.configured_tasks(&id)?{store.set_task_enabled(&task.id,false)?;}}
 println!("isolated chat GUI fixture ready");Ok(())
}
