use super::*;
#[test]
fn typed_broadcast_cross_thread_and_unregister() {
    let bus = EventBus::default();
    let first = bus.subscribe::<String>();
    let second = bus.subscribe::<String>();
    let numbers = bus.subscribe::<u64>();
    let writer = bus.clone();
    std::thread::spawn(move || writer.post(String::from("done")))
        .join()
        .unwrap();
    assert_eq!(first.try_recv().unwrap().as_str(), "done");
    assert_eq!(second.try_recv().unwrap().as_str(), "done");
    assert!(numbers.try_recv().is_err());
    drop(first);
    drop(second);
    assert!(
        !bus.inner
            .subscribers
            .lock()
            .unwrap()
            .contains_key(&TypeId::of::<String>())
    );
}
#[test]
fn workspace_bus_is_shared_and_isolated() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let paths = crate::StorePaths::in_dir(first.path().into());
    let other = crate::StorePaths::in_dir(second.path().into());
    crate::AddressBook::load(paths.clone()).unwrap();
    crate::AddressBook::load(other.clone()).unwrap();
    let subscription = workspace_bus(&paths).unwrap().subscribe::<u64>();
    workspace_bus(&other).unwrap().post(1_u64);
    assert!(subscription.try_recv().is_err());
    workspace_bus(&paths).unwrap().post(2_u64);
    assert_eq!(*subscription.try_recv().unwrap(), 2);
}

#[test]
fn async_receiver_is_woken_by_a_background_post() {
    use std::{
        future::Future,
        sync::atomic::AtomicBool,
        task::{Context, Poll, Wake, Waker},
    };
    struct Signal(AtomicBool);
    impl Wake for Signal {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let bus = EventBus::default();
    let events = bus.subscribe::<u64>();
    let signal = Arc::new(Signal(AtomicBool::new(false)));
    let waker = Waker::from(signal.clone());
    let mut context = Context::from_waker(&waker);
    let mut pending = Box::pin(events.recv());
    assert!(pending.as_mut().poll(&mut context).is_pending());
    std::thread::spawn(move || bus.post(7_u64)).join().unwrap();
    assert!(signal.0.load(Ordering::SeqCst));
    match pending.as_mut().poll(&mut context) {
        Poll::Ready(Ok(value)) => assert_eq!(*value, 7),
        _ => panic!("event did not wake async receiver"),
    }
}
