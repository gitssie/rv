//! Shared photo-batch service: transport reservation, receipts, and recovery.
//! UI components only enqueue immutable assets and observe progress.
use rv_core::assistant::{AssistantService, DraftImage};
use rv_session::{FileCommand, FileTransferClient, FileTransferSnapshot, PhotoFailure};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Copy)]
struct ImportPolicy {
    durable: bool,
    confirmed_missing: bool,
}
struct BatchProgress {
    message: String,
    done: bool,
    recovery_required: bool,
}
pub struct PhotoBatch {
    state: Arc<Mutex<BatchProgress>>,
    cancel: Arc<AtomicBool>,
}
impl PhotoBatch {
    pub fn start(
        service: AssistantService,
        client: FileTransferClient,
        target: String,
        images: Vec<DraftImage>,
        durable: bool,
        confirmed_missing: bool,
    ) -> Self {
        let state = Arc::new(Mutex::new(BatchProgress {
            message: "等待传输通道…".into(),
            done: false,
            recovery_required: false,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = state.clone();
        let stop = cancel.clone();
        std::thread::Builder::new()
            .name("rv-photo-batch".into())
            .spawn(move || {
                let result = run(
                    service,
                    client,
                    target,
                    images,
                    ImportPolicy {
                        durable,
                        confirmed_missing,
                    },
                    &stop,
                    &progress,
                );
                let mut state = progress.lock().unwrap();
                state.message = result.unwrap_or_else(|e| format!("图片导入：{e}"));
                state.done = true;
            })
            .expect("photo batch thread");
        Self { state, cancel }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn recovery_required(&self) -> bool {
        self.state.lock().unwrap().recovery_required
    }
    pub fn progress(&self) -> (String, bool) {
        let state = self.state.lock().unwrap();
        (state.message.clone(), state.done)
    }
}
impl Drop for PhotoBatch {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn run(
    service: AssistantService,
    client: FileTransferClient,
    target: String,
    images: Vec<DraftImage>,
    policy: ImportPolicy,
    cancel: &AtomicBool,
    progress: &Mutex<BatchProgress>,
) -> Result<String, String> {
    let ImportPolicy {
        durable,
        confirmed_missing,
    } = policy;
    let began = Instant::now();
    let client = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消未开始的任务".into());
        }
        if !client.snapshot().can_photos {
            return Err("手机未提供照片导入能力".into());
        }
        if let Some(owned) = client.try_reserve() {
            break owned;
        }
        if began.elapsed() > Duration::from_secs(30) {
            return Err("传输通道忙，请稍后重试".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let count = images.len();
    let mut success = 0;
    let mut skipped = 0;
    let mut errors = Vec::new();
    for (index, image) in images.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(format!("已停止，成功 {success} 张；剩余图片未发送"));
        }
        let image_id = image.id.clone();
        let phone = target.clone();
        let (mut receipt, querying) = service.call(move |s| {
            s.claim_import_recovering(&image_id, &phone, durable, confirmed_missing)
        })?;
        if receipt.status == "complete" {
            skipped += 1;
            continue;
        }
        if receipt.status == "not_found" {
            progress.lock().unwrap().recovery_required = true;
            return Err("手机没有原任务记录；核实相册未导入后，点击确认恢复".into());
        }
        progress.lock().unwrap().message = format!(
            "{} {}/{} · {}",
            if querying {
                "核实导入"
            } else {
                "导入图片"
            },
            index + 1,
            count,
            image.filename
        );
        let revision = client.snapshot().photo_revision;
        let sent = if querying {
            client.send(FileCommand::PhotoQueryTask(receipt.id.clone()))
        } else if durable {
            client.send(FileCommand::UploadToPhotosTask {
                source: service.root.join(&image.relative_path),
                task_id: receipt.id.clone(),
            })
        } else {
            client.send(FileCommand::UploadToPhotosQueued(
                service.root.join(&image.relative_path),
            ))
        };
        let started = Instant::now();
        let outcome = loop {
            if !sent {
                break Err((
                    "传输请求未发送".to_owned(),
                    if querying {
                        PhotoFailure::Unknown
                    } else {
                        PhotoFailure::NotSubmitted
                    },
                ));
            }
            let snapshot = client.snapshot();
            if !snapshot.can_photos {
                break Err(("连接中断，导入结果未知".into(), PhotoFailure::Unknown));
            }
            if let Some(outcome) = import_outcome(&snapshot, revision, durable, &receipt.id) {
                break outcome;
            }
            if cancel.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(180) {
                break Err(("导入结果未知：等待已停止".into(), PhotoFailure::Unknown));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        match outcome {
            Ok(asset) => {
                receipt.status = "complete".into();
                receipt.asset_id = Some(asset);
                receipt.error = None;
                success += 1;
            }
            Err((error, failure)) => {
                receipt.status = match failure {
                    PhotoFailure::NotSubmitted => "failed",
                    PhotoFailure::Unknown => "unknown",
                    PhotoFailure::TaskNotFound => {
                        progress.lock().unwrap().recovery_required = true;
                        "not_found"
                    }
                }
                .into();
                receipt.error = Some(error.clone());
                errors.push(format!("{}：{error}", image.filename));
            }
        }
        let failed = receipt.status != "complete";
        service.call(move |s| s.record_import(&receipt))?;
        if failed {
            break;
        }
    }
    if errors.is_empty() {
        Ok(format!(
            "导入完成：成功 {success} 张，跳过已导入 {skipped} 张"
        ))
    } else {
        Err(format!(
            "成功 {success} 张；{}；其余未发送",
            errors.join("；")
        ))
    }
}

/// Correlate a completion and keep every post-submission failure non-retryable.
fn import_outcome(
    snapshot: &FileTransferSnapshot,
    revision: u64,
    durable: bool,
    task_id: &str,
) -> Option<Result<String, (String, PhotoFailure)>> {
    if snapshot.photo_revision == revision
        || snapshot.photo_busy
        || (durable && snapshot.photo_task_id.as_deref() != Some(task_id))
    {
        return None;
    }
    if let Some(error) = &snapshot.photo_error {
        return Some(Err((
            error.clone(),
            snapshot.photo_failure.unwrap_or(PhotoFailure::Unknown),
        )));
    }
    if let Some(asset) = &snapshot.photo_asset_id {
        return Some(Ok(asset.clone()));
    }
    Some(Err((
        "导入结果未知：没有相册资产 ID".into(),
        PhotoFailure::Unknown,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_and_task_identity_determine_retry_safety() {
        let mut s = FileTransferSnapshot {
            photo_revision: 1,
            photo_task_id: Some("task".into()),
            photo_error: Some("Cannot open the selected image".into()),
            photo_failure: Some(PhotoFailure::NotSubmitted),
            ..Default::default()
        };
        assert_eq!(
            import_outcome(&s, 0, true, "task").unwrap().unwrap_err().1,
            PhotoFailure::NotSubmitted
        );
        assert!(import_outcome(&s, 0, true, "other").is_none());
        s.photo_failure = Some(PhotoFailure::Unknown);
        s.photo_error = Some("Imported photo could not be verified".into());
        assert_eq!(
            import_outcome(&s, 0, true, "task").unwrap().unwrap_err().1,
            PhotoFailure::Unknown
        );
        s.photo_failure = Some(PhotoFailure::TaskNotFound);
        s.photo_error = Some("Photos job not found".into());
        assert_eq!(
            import_outcome(&s, 0, true, "task").unwrap().unwrap_err().1,
            PhotoFailure::TaskNotFound
        );
        s.photo_error = None;
        s.photo_asset_id = None;
        assert_eq!(
            import_outcome(&s, 0, true, "task").unwrap().unwrap_err().1,
            PhotoFailure::Unknown
        );
        s.photo_asset_id = Some("asset".into());
        assert_eq!(
            import_outcome(&s, 0, true, "task").unwrap().unwrap(),
            "asset"
        );
    }
    #[test]
    fn unsent_recovery_query_preserves_unknown_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let service = AssistantService::shared(dir.path().to_owned());
        let path = dir.path().join("fixture.jpg");
        std::fs::write(&path, b"fixture").unwrap();
        let image = service
            .call(move |s| {
                let d = s.save_draft(&Default::default())?;
                s.add_image(&d.id, &path)
            })
            .unwrap();
        let image_id = image.id.clone();
        let receipt = service
            .call(move |s| {
                let (mut r, _) = s.claim_import(&image_id, "phone", true)?;
                r.status = "unknown".into();
                s.record_import(&r)?;
                Ok(r)
            })
            .unwrap();
        let (handle, peer) = rv_session::SessionHandle::test_pair();
        peer.file_state.lock().unwrap().can_photos = true;
        drop(peer);
        let batch = PhotoBatch::start(
            service.clone(),
            handle.file_client(),
            "phone".into(),
            vec![image.clone()],
            true,
            false,
        );
        let until = Instant::now() + Duration::from_secs(2);
        while !batch.progress().1 {
            assert!(Instant::now() < until, "batch did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
        let saved = service
            .call(move |s| s.receipts(&image.id, "phone"))
            .unwrap();
        assert_eq!(saved[0].id, receipt.id);
        assert_eq!(saved[0].status, "unknown");
    }
    #[test]
    fn missing_task_is_not_resent_until_confirmation_and_reuses_uuid() {
        use rv_session::{SessionCommand, SessionHandle, SessionTestPeer};
        fn command(peer: &mut SessionTestPeer) -> FileCommand {
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                if let Ok(SessionCommand::File(command)) = peer.commands.try_recv() {
                    return command;
                }
                assert!(Instant::now() < until, "batch did not dispatch a command");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        fn completed(batch: &PhotoBatch) {
            let until = Instant::now() + Duration::from_secs(2);
            while !batch.progress().1 {
                assert!(Instant::now() < until, "batch did not finish");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let service = AssistantService::shared(dir.path().to_owned());
        let path = dir.path().join("fixture.jpg");
        std::fs::write(&path, b"fixture").unwrap();
        let image = service
            .call(move |s| {
                let d = s.save_draft(&Default::default())?;
                s.add_image(&d.id, &path)
            })
            .unwrap();
        let image_id = image.id.clone();
        let receipt = service
            .call(move |s| {
                let (mut r, _) = s.claim_import(&image_id, "phone", true)?;
                r.status = "unknown".into();
                s.record_import(&r)?;
                Ok(r)
            })
            .unwrap();
        let (handle, mut peer) = SessionHandle::test_pair();
        peer.file_state.lock().unwrap().can_photos = true;
        let batch = PhotoBatch::start(
            service.clone(),
            handle.file_client(),
            "phone".into(),
            vec![image.clone()],
            true,
            false,
        );
        assert!(matches!(command(&mut peer),FileCommand::PhotoQueryTask(id) if id==receipt.id));
        {
            let mut s = peer.file_state.lock().unwrap();
            s.pending_commands = 0;
            s.photo_revision += 1;
            s.photo_task_id = Some(receipt.id.clone());
            s.photo_error = Some("Photos job not found".into());
            s.photo_failure = Some(PhotoFailure::TaskNotFound);
        }
        completed(&batch);
        assert!(batch.recovery_required());
        drop(batch);
        let batch = PhotoBatch::start(
            service.clone(),
            handle.file_client(),
            "phone".into(),
            vec![image.clone()],
            true,
            false,
        );
        completed(&batch);
        assert!(batch.recovery_required());
        assert!(peer.commands.try_recv().is_err());
        drop(batch);
        let batch = PhotoBatch::start(
            service.clone(),
            handle.file_client(),
            "phone".into(),
            vec![image.clone()],
            true,
            true,
        );
        assert!(
            matches!(command(&mut peer),FileCommand::UploadToPhotosTask {task_id,..} if task_id==receipt.id)
        );
        {
            let mut s = peer.file_state.lock().unwrap();
            s.pending_commands = 0;
            s.photo_revision += 1;
            s.photo_error = None;
            s.photo_failure = None;
            s.photo_asset_id = Some("asset".into());
        }
        completed(&batch);
        assert!(!batch.recovery_required());
        drop(batch);
        let image_id = image.id.clone();
        let saved = service
            .call(move |s| s.receipts(&image_id, "phone"))
            .unwrap();
        assert_eq!(saved[0].id, receipt.id);
        assert_eq!(saved[0].status, "complete");
        let batch = PhotoBatch::start(
            service,
            handle.file_client(),
            "phone".into(),
            vec![image],
            true,
            false,
        );
        completed(&batch);
        assert!(peer.commands.try_recv().is_err());
    }
}
