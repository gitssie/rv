use crate::app::ui_tests::{click, setup};
use gpui::{TestAppContext, px, size};
use rv_core::Connection;
#[gpui::test]
fn store_chat_uses_three_columns_one_composer_and_no_search(cx: &mut TestAppContext) {
    let device = Connection::new("fixture", "example.invalid", 5900);
    let (app, cx, _dir) = setup(cx, vec![device.clone()]);
    let ids = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.home.task_worker.take();
            super::test_support::seed(app.book.paths(), device.id)
        })
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_chat_for(ids[0].clone(), window, cx)));
    cx.run_until_parked();
    let handle=app.read_with(cx,|app,_|app.chat.window.unwrap());
    let mut chat_cx=gpui::VisualTestContext::from_window(handle.into(),cx);
    let cx=&mut chat_cx;
    cx.simulate_resize(size(px(1180.), px(820.)));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    for id in ["chat-panel", "chat-stores", "chat-conversations"] {
        assert!(cx.debug_bounds(id).is_some());
    }
    for id in [
        "chat-search",
        "chat-store-search",
        "chat-conversation-search",
        "chat-translation-preview",
    ] {
        assert!(cx.debug_bounds(id).is_none());
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_chat("conversation-0".into(), window, cx)
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("chat-send").is_some());
    assert!(cx.debug_bounds("chat-transcript").is_some());
    app.read_with(cx, |app, _| {
        assert_eq!(app.chat.account.as_ref(), Some(&ids[0]));
        assert_eq!(app.chat.thread.messages.len(), 3);
        assert_eq!(
            app.chat
                .thread
                .customer
                .as_ref()
                .unwrap()
                .country_code
                .as_deref(),
            Some("ES")
        );
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.chat
                .composer
                .update(cx, |input, cx| input.set_value("测试中文原文", window, cx))
        })
    });
    let before = app.read_with(cx, |app, _| {
        rv_core::workspace::WorkspaceStore::open(app.book.paths())
            .unwrap()
            .tasks(&ids[0])
            .unwrap()
            .len()
    });
    click(cx, "chat-send");
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        let t = rv_business::chat::thread(app.book.paths(), "conversation-0").unwrap();
        assert_eq!(t.pending.len(), 1);
        assert_eq!(t.pending[0].body, "测试中文原文");
        assert_eq!(t.pending[0].status, "pending");
        assert!(t.pending[0].translation.is_none());
        assert_eq!(
            rv_core::workspace::WorkspaceStore::open(app.book.paths())
                .unwrap()
                .tasks(&ids[0])
                .unwrap()
                .len(),
            before
        );
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.filter_chat_account(Some(ids[1].clone()), window, cx)
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.chat.selected.is_none());
        assert_eq!(app.chat.visible().count(), 1);
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.filter_chat_account(Some(ids[0].clone()), window, cx)
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.chat.selected.as_deref(), Some("conversation-0"))
    });
}

#[gpui::test]
fn notifications_are_shop_scoped_and_viewing_never_sends_or_marks_read(cx: &mut TestAppContext) {
    use rv_core::workspace::WorkspaceStore;
    let device = Connection::new("fixture", "example.invalid", 5900);
    let (app, cx, _dir) = setup(cx, vec![device.clone()]);
    let ids = cx.update(|_, cx| app.update(cx, |app, _| {
        app.home.task_worker.take();
        let ids = super::test_support::seed(app.book.paths(), device.id);
        rv_business::notifications::list(app.book.paths()).unwrap();
        let mut store = WorkspaceStore::open(app.book.paths()).unwrap();
        for id in &ids[..2] {
            let mut shop=store.load(id).unwrap();
            let mut metadata:serde_json::Value=serde_json::from_str(&shop.metadata).unwrap();
            metadata["notification_unread_count"]=serde_json::json!(7);
            shop.metadata=metadata.to_string();store.save(&shop).unwrap();
        }
        store.business_transaction(|db| {
            for id in &ids[..2] {
                db.execute("INSERT INTO vinted_notifications(app_id,remote_id,updated_at,payload) VALUES(?1,'notice-1',100,?2)",
                    rusqlite::params![id.as_str(),serde_json::json!({"id":"notice-1","body":"通知正文","read":false,"updated_at":100}).to_string()])?;
            }
            Ok(())
        }).unwrap();
        ids
    }));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_chat_for(ids[0].clone(), window, cx)));
    cx.run_until_parked();
    let handle=app.read_with(cx,|app,_|app.chat.window.unwrap());
    let mut chat_cx=gpui::VisualTestContext::from_window(handle.into(),cx);
    let cx=&mut chat_cx;
    cx.simulate_resize(size(px(1180.), px(820.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_chat("conversation-0".into(), window, cx);
            app.chat
                .composer
                .update(cx, |input, cx| input.set_value("保留草稿", window, cx));
        })
    });
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| {
        ids.iter()
            .map(|id| {
                format!(
                    "{:?}",
                    WorkspaceStore::open(app.book.paths())
                        .unwrap()
                        .tasks(id)
                        .unwrap()
                )
            })
            .collect::<Vec<_>>()
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.switch_message_section(1, window, cx);
            // Enter in a hidden composer must not enqueue a reply.
            app.send_chat(window, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("chat-notifications").is_some());
    assert!(cx.debug_bounds("notification-detail").is_some());
    assert!(cx.debug_bounds("chat-send").is_none());
    assert!(cx.debug_bounds("chat-transcript").is_none());
    app.read_with(cx, |app, _| {
        assert_eq!(app.chat.visible_notifications().count(), 1);
        assert_eq!(app.chat.selected_notification().unwrap().app_id, ids[0]);
        assert_eq!(app.chat.notification_unread(&ids[0]), Some(7));
        assert!(
            rv_business::chat::thread(app.book.paths(), "conversation-0")
                .unwrap()
                .pending
                .is_empty()
        );
    });
    // A shop change while looking at notifications must not request a chat sync,
    // including when that shop has a remembered conversation.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.chat
                .remembered
                .insert(ids[1].clone(), "conversation-2".into());
            app.filter_chat_account(Some(ids[1].clone()), window, cx);
            assert_eq!(app.chat.selected_notification().unwrap().app_id, ids[1]);
            app.filter_chat_account(Some(ids[0].clone()), window, cx);
            app.switch_message_section(0, window, cx);
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.chat.selected.as_deref(), Some("conversation-0"));
        assert_eq!(app.chat.composer.read(cx).value().as_ref(), "保留草稿");
        assert_eq!(
            rv_business::notifications::list(app.book.paths()).unwrap()[0].unread,
            Some(true)
        );
        let after = ids
            .iter()
            .map(|id| {
                format!(
                    "{:?}",
                    WorkspaceStore::open(app.book.paths())
                        .unwrap()
                        .tasks(id)
                        .unwrap()
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(before, after);
    });
}

#[gpui::test]
fn committed_events_update_hidden_unread_and_selected_thread_without_touching_draft(
    cx: &mut TestAppContext,
) {
    use rv_core::workspace::WorkspaceStore;
    let device = Connection::new("fixture", "example.invalid", 5900);
    let (app, cx, _dir) = setup(cx, vec![device.clone()]);
    let ids = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.home.task_worker.take();
            super::test_support::seed(app.book.paths(), device.id)
        })
    });
    cx.run_until_parked();
    // No open chat and no timer advance: unread is the server projection in SQLite.
    app.read_with(cx, |app, _| {
        assert!(app.chat.window.is_none());
        let mut store = WorkspaceStore::open(app.book.paths()).unwrap();
        let mut row = store.load(&ids[0]).unwrap();
        let mut metadata: serde_json::Value = serde_json::from_str(&row.metadata).unwrap();
        metadata["unread_count"] = serde_json::json!(17);
        row.metadata = metadata.to_string();
        store.save(&row).unwrap();
    });
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.chat
                .accounts
                .iter()
                .find(|a| a.id == ids[0])
                .unwrap()
                .unread,
            Some(17)
        );
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_chat_for(ids[0].clone(), window, cx);
        })
    });
    cx.run_until_parked();
    let handle=app.read_with(cx,|app,_|app.chat.window.unwrap());
    let mut chat_cx=gpui::VisualTestContext::from_window(handle.into(),cx);
    let cx=&mut chat_cx;
    cx.update(|window,cx| {
        app.update(cx,|app,cx| {
            app.select_chat("conversation-0".into(), window, cx);
            app.chat
                .composer
                .update(cx, |input, cx| input.set_value("保留草稿", window, cx));
        })
    });
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| app.chat.thread_sequence);
    app.read_with(cx, |app, _| {
        let mut store = WorkspaceStore::open(app.book.paths()).unwrap();
        store.business_transaction(|db| {
            db.execute("UPDATE chat_messages SET body='other account' WHERE conversation_id='conversation-2'", [])?;
            Ok(())
        }).unwrap();
    });
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert_eq!(app.chat.thread_sequence, before));
    app.read_with(cx, |app, _| {
        let mut store = WorkspaceStore::open(app.book.paths()).unwrap();
        store.business_transaction(|db| {
            db.execute("UPDATE chat_messages SET body='new message',data=json_set(data,'$.body','new message') WHERE id='message-0-0'", [])?;
            Ok(())
        }).unwrap();
    });
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.chat.thread.messages[0].body, "new message");
        assert_eq!(app.chat.composer.read(cx).value().as_ref(), "保留草稿");
    });
}

#[gpui::test]
fn native_chat_window_reuses_handle_and_reopens_with_saved_draft(cx:&mut TestAppContext) {
    let device=Connection::new("fixture","example.invalid",5900);
    let (app,cx,_dir)=setup(cx,vec![device.clone()]);
    let ids=cx.update(|_,cx|app.update(cx,|app,_|{app.home.task_worker.take();super::test_support::seed(app.book.paths(),device.id)}));
    cx.run_until_parked();
    let main=cx.update(|window,_|window.window_handle().window_id());
    cx.update(|window,cx|app.update(cx,|app,cx|app.open_chat_for(ids[0].clone(),window,cx)));
    cx.run_until_parked();
    let first=app.read_with(cx,|app,_|{assert!(matches!(app.modal,crate::app::Modal::None));app.chat.window.unwrap()});
    assert_ne!(first.window_id(),main);
    assert!(cx.debug_bounds("chat-panel").is_none());
    cx.update(|window,cx|app.update(cx,|app,cx|app.open_chat(window,cx)));cx.run_until_parked();
    assert_eq!(app.read_with(cx,|app,_|app.chat.window.unwrap().window_id()),first.window_id());
    first.update(cx,|_,window,cx|app.update(cx,|app,cx| {
        app.select_chat("conversation-0".into(),window,cx);
        app.chat.composer.update(cx,|input,cx|input.set_value("保存的草稿",window,cx));
    })).unwrap();cx.run_until_parked();
    first.update(cx,|_,window,_|window.remove_window()).unwrap();cx.run_until_parked();
    app.read_with(cx,|app,_|{assert!(app.chat.window.is_none());assert!(app.chat.input_subscription.is_none());});
    cx.update(|window,cx|app.update(cx,|app,cx|app.open_chat(window,cx)));cx.run_until_parked();
    app.read_with(cx,|app,cx| {
        assert_ne!(app.chat.window.unwrap().window_id(),first.window_id());
        assert_eq!(app.chat.composer.read(cx).value().as_ref(),"保存的草稿");
    });
}
