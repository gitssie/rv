//! Typed, process-local EventBus. Posting queues events without executing subscriber code.
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
};
type Payload = Arc<dyn Any + Send + Sync>;
type Subscribers = HashMap<TypeId, HashMap<u64, async_channel::Sender<Payload>>>;
#[derive(Default)]
struct Inner {
    subscribers: Mutex<Subscribers>,
    next: AtomicU64,
}
#[derive(Clone, Default)]
pub struct EventBus {
    inner: Arc<Inner>,
}
impl EventBus {
    pub fn subscribe<E: Send + Sync + 'static>(&self) -> Subscription<E> {
        let (sender, receiver) = async_channel::unbounded();
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        self.inner
            .subscribers
            .lock()
            .unwrap()
            .entry(TypeId::of::<E>())
            .or_default()
            .insert(id, sender);
        Subscription {
            bus: self.clone(),
            id,
            receiver,
            event: PhantomData,
        }
    }
    pub fn post<E: Send + Sync + 'static>(&self, event: E) {
        let mut subscribers = self.inner.subscribers.lock().unwrap();
        if let Some(list) = subscribers.get_mut(&TypeId::of::<E>()) {
            let event: Payload = Arc::new(event);
            list.retain(|_, sender| sender.try_send(event.clone()).is_ok());
        }
    }
}
pub struct Subscription<E: 'static> {
    bus: EventBus,
    id: u64,
    receiver: async_channel::Receiver<Payload>,
    event: PhantomData<E>,
}
impl<E: Send + Sync + 'static> Subscription<E> {
    pub async fn recv(&self) -> Result<Arc<E>, async_channel::RecvError> {
        self.receiver
            .recv()
            .await
            .map(|event| event.downcast::<E>().expect("EventBus type routing"))
    }
    pub fn try_recv(&self) -> Result<Arc<E>, async_channel::TryRecvError> {
        self.receiver
            .try_recv()
            .map(|event| event.downcast::<E>().expect("EventBus type routing"))
    }
}
impl<E: 'static> Drop for Subscription<E> {
    fn drop(&mut self) {
        let mut subscribers = self.bus.inner.subscribers.lock().unwrap();
        if let Some(list) = subscribers.get_mut(&TypeId::of::<E>()) {
            list.remove(&self.id);
            if list.is_empty() {
                subscribers.remove(&TypeId::of::<E>());
            }
        }
    }
}
/// Connections to the same workspace share a bus; isolated databases never mix events.
pub fn workspace_bus(paths: &crate::StorePaths) -> Result<EventBus, crate::StoreError> {
    static BUSES: OnceLock<Mutex<HashMap<PathBuf, Weak<Inner>>>> = OnceLock::new();
    let path = std::fs::canonicalize(&paths.database)?;
    let mut buses = BUSES.get_or_init(Default::default).lock().unwrap();
    buses.retain(|_, bus| bus.strong_count() > 0);
    if let Some(inner) = buses.get(&path).and_then(Weak::upgrade) {
        return Ok(EventBus { inner });
    }
    let bus = EventBus::default();
    buses.insert(path, Arc::downgrade(&bus.inner));
    Ok(bus)
}

/// Only invalidation keys cross the bus. Business data and task results remain in SQLite.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ChangeDomain {
    Tasks,
    Apps,
    Credentials,
    Execution,
    Conversations,
    Messages,
    Customers,
    Products,
    Orders,
    Notifications,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct EntityChange {
    pub domain: ChangeDomain,
    pub app_id: Option<String>,
    pub conversation_id: Option<String>,
    pub task_id: Option<String>,
}
#[derive(Clone, Debug)]
pub struct WorkspaceChanged {
    pub changes: Vec<EntityChange>,
}

#[cfg(test)]
mod tests;

/// An invalidation/request only; token values never travel on the EventBus.
#[derive(Clone, Debug)]
pub struct PhoneTokenRequested { pub app_id: String, pub manual: bool }
#[derive(Clone, Debug)]
pub struct PhoneTokenResult { pub app_id: String, pub manual: bool, pub result: Result<(), String> }
