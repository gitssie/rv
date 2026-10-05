use super::*;

#[test]
fn draft_conflicts_survive_restart_and_money_is_exact() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = AssistantStore::open(dir.path()).unwrap();
    let first = store
        .save_draft(&Draft {
            title: "中文商品".into(),
            price_minor: parse_price("18,05").unwrap(),
            ..Draft::default()
        })
        .unwrap();
    let mut changed = first.clone();
    changed.description = "新描述".into();
    let latest = store.save_draft(&changed).unwrap();
    assert!(matches!(
        store.save_draft(&first),
        Err(StoreError::Conflict)
    ));
    assert!(matches!(
        store.delete_draft(&first.id, first.revision),
        Err(StoreError::Conflict)
    ));
    drop(store);
    let store = AssistantStore::open(dir.path()).unwrap();
    assert_eq!(store.load_draft(&first.id).unwrap(), latest);
    assert_eq!(format_price(Some(1805)), "18.05");
    assert!(parse_price("1.005").is_err());
    assert!(parse_price("-1").is_err());
    assert!(parse_price("1e9").is_err());
}
#[test]
fn assets_are_owned_reordered_and_duplicate_shares_immutable_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = AssistantStore::open(dir.path()).unwrap();
    let d = store.save_draft(&Draft::default()).unwrap();
    let source = dir.path().join("source.jpg");
    std::fs::write(&source, b"fixture").unwrap();
    let a = store.add_image(&d.id, &source).unwrap();
    let b = store.add_image(&d.id, &source).unwrap();
    std::fs::remove_file(source).unwrap();
    assert!(dir.path().join(&a.relative_path).exists());
    assert_eq!(a.relative_path, b.relative_path);
    assert!(
        store
            .reorder_images(&d.id, &[a.id.clone(), a.id.clone()])
            .is_err()
    );
    store
        .reorder_images(&d.id, &[b.id.clone(), a.id.clone()])
        .unwrap();
    assert_eq!(store.images(&d.id).unwrap()[0].id, b.id);
    let copy = store.duplicate(&d.id).unwrap();
    store.delete_draft(&d.id, d.revision).unwrap();
    assert_eq!(store.images(&copy.id).unwrap().len(), 2);
    assert!(dir.path().join(a.relative_path).exists());
}
#[test]
fn template_conflicts_and_crashed_imports_are_not_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = AssistantStore::open(dir.path()).unwrap();
    let t = s
        .save_template(&Template {
            name: "回复".into(),
            content: "Hello".into(),
            ..Default::default()
        })
        .unwrap();
    s.save_template(&t).unwrap();
    assert!(s.save_template(&t).is_err());
    let d = s.save_draft(&Draft::default()).unwrap();
    let p = dir.path().join("x.png");
    std::fs::write(&p, b"data").unwrap();
    let image = s.add_image(&d.id, &p).unwrap();
    s.record_import(&ImportReceipt {
        id: "job".into(),
        image_id: image.id.clone(),
        target: "device".into(),
        status: "running".into(),
        asset_id: None,
        error: None,
    })
    .unwrap();
    drop(s);
    let s = AssistantStore::open(dir.path()).unwrap();
    assert_eq!(
        s.receipts(&image.id, "device").unwrap()[0].status,
        "unknown"
    );
}

#[test]
fn closing_conflict_preserves_both_editors() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = AssistantStore::open(dir.path()).unwrap();
    let original = s.save_draft(&Draft::default()).unwrap();
    let mut other = original.clone();
    other.title = "Other window".into();
    let latest = s.save_draft(&other).unwrap();
    let mut closing = original.clone();
    closing.description = "Unsaved changes".into();
    let recovery = s.save_closing_draft(&closing).unwrap();
    assert_ne!(recovery.id, latest.id);
    assert_eq!(s.load_draft(&latest.id).unwrap(), latest);
    assert_eq!(recovery.description, "Unsaved changes");
}
#[test]
fn atomic_import_claim_prevents_duplicate_and_recovers_same_token() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = AssistantStore::open(dir.path()).unwrap();
    let d = s.save_draft(&Draft::default()).unwrap();
    let source = dir.path().join("fixture.jpg");
    std::fs::write(&source, b"fixture").unwrap();
    let image = s.add_image(&d.id, &source).unwrap();
    let (mut first, querying) = s.claim_import(&image.id, "phone", true).unwrap();
    assert!(!querying);
    assert!(s.claim_import(&image.id, "phone", true).is_err());
    first.status = "unknown".into();
    s.record_import(&first).unwrap();
    assert!(s.claim_import(&image.id, "phone", false).is_err());
    let (mut recovery, querying) = s.claim_import(&image.id, "phone", true).unwrap();
    assert!(querying);
    assert_eq!(first.id, recovery.id);
    recovery.status = "complete".into();
    recovery.asset_id = Some("asset".into());
    s.record_import(&recovery).unwrap();
    assert_eq!(
        s.claim_import(&image.id, "phone", true).unwrap().0.status,
        "complete"
    );
}

#[test]
fn unfinished_price_and_other_edits_survive_restart_and_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = AssistantStore::open(dir.path()).unwrap();
    let d = s
        .save_draft(&Draft {
            title: "changed title".into(),
            description: "changed description".into(),
            price_minor: Some(1800),
            price_input: Some("-".into()),
            ..Default::default()
        })
        .unwrap();
    let copy = s.duplicate(&d.id).unwrap();
    assert_eq!(copy.price_input.as_deref(), Some("-"));
    drop(s);
    let mut s = AssistantStore::open(dir.path()).unwrap();
    let mut loaded = s.load_draft(&d.id).unwrap();
    assert_eq!(loaded.title, "changed title");
    assert_eq!(loaded.description, "changed description");
    assert_eq!(loaded.price_input.as_deref(), Some("-"));
    assert_eq!(loaded.price_minor, Some(1800));
    loaded.price_minor = Some(1900);
    loaded.price_input = None;
    assert!(s.save_draft(&loaded).unwrap().price_input.is_none());
}
#[test]
fn version_one_migrates_without_losing_drafts_or_replaying_old_failures() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = AssistantStore::open(dir.path()).unwrap();
    let d = s
        .save_draft(&Draft {
            title: "existing".into(),
            price_minor: Some(1000),
            ..Default::default()
        })
        .unwrap();
    let p = dir.path().join("x.jpg");
    std::fs::write(&p, b"fixture").unwrap();
    let image = s.add_image(&d.id, &p).unwrap();
    let (mut receipt, _) = s.claim_import(&image.id, "phone", true).unwrap();
    receipt.status = "failed".into();
    receipt.error = Some("Imported photo could not be verified".into());
    s.record_import(&receipt).unwrap();
    drop(s);
    let db = rusqlite::Connection::open(dir.path().join("assistant.db")).unwrap();
    db.execute_batch("ALTER TABLE drafts DROP COLUMN price_input; PRAGMA user_version=1;")
        .unwrap();
    drop(db);
    let s = AssistantStore::open(dir.path()).unwrap();
    let loaded = s.load_draft(&d.id).unwrap();
    assert_eq!(loaded.title, "existing");
    assert_eq!(loaded.price_minor, Some(1000));
    assert!(loaded.price_input.is_none());
    assert_eq!(s.receipts(&image.id, "phone").unwrap()[0].status, "unknown");
}
#[test]
fn missing_task_requires_confirmation_and_reuses_original_uuid() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = AssistantStore::open(dir.path()).unwrap();
    let d = s.save_draft(&Draft::default()).unwrap();
    let p = dir.path().join("x.jpg");
    std::fs::write(&p, b"fixture").unwrap();
    let image = s.add_image(&d.id, &p).unwrap();
    let (mut receipt, _) = s.claim_import(&image.id, "phone", true).unwrap();
    receipt.status = "not_found".into();
    s.record_import(&receipt).unwrap();
    let (blocked, _) = s.claim_import(&image.id, "phone", true).unwrap();
    assert_eq!(blocked.status, "not_found");
    let (claimed, query) = s
        .claim_import_recovering(&image.id, "phone", true, true)
        .unwrap();
    assert_eq!(claimed.id, receipt.id);
    assert_eq!(claimed.status, "running");
    assert!(!query);
    assert!(
        s.claim_import_recovering(&image.id, "phone", true, true)
            .is_err()
    );
}
