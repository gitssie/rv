use super::{WorkAssistant, persistence};
use gpui::{AppContext, Focusable, TestAppContext};
use rv_core::assistant::{AssistantService, Draft};
#[gpui::test]
fn typing_stays_local_and_invalid_price_does_not_clear_store_conflict(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let dir = tempfile::tempdir().unwrap();
    let service = AssistantService::shared(dir.path().to_owned());
    let draft = service.call(|s| s.save_draft(&Draft::default())).unwrap();
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| WorkAssistant::new(service, window, cx));
        view.update(cx, |v, cx| v.select_draft(draft, window, cx));
        entity = Some(view.clone());
        gpui_component::Root::new(view, window, cx)
    });
    let view = entity.unwrap();
    cx.update(|window, cx| {
        view.read(cx).title.focus_handle(cx).focus(window, cx);
        window.draw(cx).clear(cx);
    });
    cx.simulate_input("商品标题");
    view.read_with(cx, |v, cx| {
        assert_eq!(v.title.read(cx).value(), "商品标题");
        assert_eq!(v.draft.as_ref().unwrap().title, "商品标题");
        assert!(v.dirty);
    });
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.save_error = Some("数据库保存冲突".into());
            v.price.update(cx, |p, cx| p.set_value("-1", window, cx));
            v.capture(cx);
            assert!(v.price_invalid);
            v.price.update(cx, |p, cx| p.set_value("18,50", window, cx));
            v.capture(cx);
            assert!(!v.price_invalid);
            assert_eq!(v.save_error.as_deref(), Some("数据库保存冲突"));
            assert_eq!(v.draft.as_ref().unwrap().price_minor, Some(1850));
        })
    });
}
#[test]
fn pending_close_uses_actual_save_revision_and_recovers_failed_save() {
    let dir = tempfile::tempdir().unwrap();
    let service = AssistantService::shared(dir.path().to_owned());
    let first = service.call(|s| s.save_draft(&Draft::default())).unwrap();
    let pending = persistence::PendingSave::default();
    let mut latest = first.clone();
    latest.description = "tail edit".into();
    pending.close(latest, &service);
    let saved = service.call(move |s| s.save_draft(&first)).unwrap();
    pending.finish(&Ok(saved.clone()), &service);
    let id = saved.id.clone();
    assert_eq!(
        service
            .call(move |s| s.load_draft(&id))
            .unwrap()
            .description,
        "tail edit"
    );
    let pending = persistence::PendingSave::default();
    let mut conflicting = saved;
    conflicting.description = "conflicting tail".into();
    pending.close(conflicting, &service);
    pending.finish(&Err("conflict".into()), &service);
    let list = service.call(|s| s.list_drafts()).unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().any(|d| d.description == "tail edit"));
    assert!(list.iter().any(|d| d.description == "conflicting tail"));
}

#[gpui::test]
fn image_error_and_invalid_price_do_not_block_save_or_close(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let dir = tempfile::tempdir().unwrap();
    let service = AssistantService::shared(dir.path().to_owned());
    let d = service.call(|s| s.save_draft(&Draft::default())).unwrap();
    let id = d.id.clone();
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|w, cx| {
        let v = cx.new(|cx| WorkAssistant::new(service.clone(), w, cx));
        v.update(cx, |v, cx| v.select_draft(d, w, cx));
        entity = Some(v.clone());
        gpui_component::Root::new(v, w, cx)
    });
    let v = entity.unwrap();
    cx.update(|w, cx| {
        v.update(cx, |v, cx| {
            v.title
                .update(cx, |i, cx| i.set_value("preserved title", w, cx));
            v.description
                .update(cx, |i, cx| i.set_value("preserved description", w, cx));
            v.price.update(cx, |i, cx| i.set_value("-", w, cx));
            v.capture(cx);
            // A real failed asset operation records an operation error, not a save error.
            v.error = Some("unsupported image".into());
            v.flush(cx);
            assert!(v.save_busy);
        })
    });
    for _ in 0..100 {
        cx.run_until_parked();
        if !v.read_with(cx, |v, _| v.save_busy) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!v.read_with(cx, |v, _| v.save_busy));
    let loaded = service.call(move |s| s.load_draft(&id)).unwrap();
    assert_eq!(loaded.title, "preserved title");
    assert_eq!(loaded.description, "preserved description");
    assert_eq!(loaded.price_input.as_deref(), Some("-"));
    // Verify the actual Drop path, without a preceding explicit save.
    let mut closing = None;
    cx.update(|w, cx| {
        let entity = cx.new(|cx| WorkAssistant::new(service.clone(), w, cx));
        entity.update(cx, |v, _| {
            v.draft = Some(loaded);
            v.dirty = true;
            v.price_invalid = true;
            v.draft.as_mut().unwrap().description = "close with unfinished price".into();
        });
        closing = Some(entity);
    });
    drop(closing);
    cx.run_until_parked();
    cx.update(|_, _| {});
    let list = service.call(|s| s.list_drafts()).unwrap();
    assert_eq!(list[0].description, "close with unfinished price");
    assert_eq!(list[0].price_input.as_deref(), Some("-"));
}
