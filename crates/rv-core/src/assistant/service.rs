use super::{AssistantStore, StoreError};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak, mpsc},
};
type Job = Box<dyn FnOnce(&mut Result<AssistantStore, String>) + Send>;
struct Worker {
    sender: mpsc::Sender<Job>,
}
#[derive(Clone)]
pub struct AssistantService {
    worker: Arc<Worker>,
    pub root: PathBuf,
}
impl AssistantService {
    /// One SQLite owner thread per data directory, shared by session windows.
    pub fn shared(root: PathBuf) -> Self {
        static WORKERS: OnceLock<Mutex<HashMap<PathBuf, Weak<Worker>>>> = OnceLock::new();
        let mut workers = WORKERS.get_or_init(Default::default).lock().unwrap();
        let worker = workers
            .get(&root)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let (sender, receiver) = mpsc::channel::<Job>();
                let path = root.clone();
                std::thread::Builder::new()
                    .name("rv-assistant-store".into())
                    .spawn(move || {
                        let mut store = AssistantStore::open(&path).map_err(|e| e.to_string());
                        for job in receiver {
                            job(&mut store);
                        }
                    })
                    .expect("assistant database worker");
                let worker = Arc::new(Worker { sender });
                workers.insert(root.clone(), Arc::downgrade(&worker));
                worker
            });
        Self { worker, root }
    }
    pub fn submit(
        &self,
        f: impl FnOnce(&mut AssistantStore) -> Result<(), StoreError> + Send + 'static,
    ) {
        let _ = self.worker.sender.send(Box::new(move |store| {
            if let Ok(store) = store
                && let Err(error) = f(store)
            {
                eprintln!("RV assistant background save failed: {error}");
            }
        }));
    }
    /// Call from a background executor, never the GPUI render thread.
    pub fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut AssistantStore) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, String> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.worker
            .sender
            .send(Box::new(move |store| {
                let result = match store {
                    Ok(store) => f(store).map_err(|e| e.to_string()),
                    Err(e) => Err(e.clone()),
                };
                let _ = sender.send(result);
            }))
            .map_err(|_| "数据库线程已关闭")?;
        receiver.recv().map_err(|_| "数据库线程已关闭".to_owned())?
    }
}
