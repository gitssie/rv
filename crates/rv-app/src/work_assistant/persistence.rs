//! Closing an editor waits for its actual save result; revisions are never guessed.
use rv_core::assistant::{AssistantService, Draft};
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct State {
    result: Option<Result<Draft, String>>,
    closing: Option<Draft>,
}
#[derive(Clone, Default)]
pub(super) struct PendingSave(Arc<Mutex<State>>);
impl PendingSave {
    pub fn finish(&self, result: &Result<Draft, String>, service: &AssistantService) {
        let mut state = self.0.lock().unwrap();
        state.result = Some(result.clone());
        if let Some(draft) = state.closing.take() {
            persist(service, draft, Some(result));
        }
    }
    pub fn close(&self, draft: Draft, service: &AssistantService) {
        let mut state = self.0.lock().unwrap();
        if let Some(result) = &state.result {
            persist(service, draft, Some(result));
        } else {
            state.closing = Some(draft);
        }
    }
}
pub(super) fn persist(
    service: &AssistantService,
    mut draft: Draft,
    result: Option<&Result<Draft, String>>,
) {
    if let Some(Ok(saved)) = result {
        draft.revision = saved.revision;
        draft.updated_at = saved.updated_at;
        if draft == *saved {
            return;
        }
    }
    service.submit(move |s| s.save_closing_draft(&draft).map(|_| ()));
}
