use std::{future::Future, pin::Pin};

use crate::{Frame, Point, Rect};

pub type DeviceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, DeviceError>> + Send + 'a>>;

/// Errors deliberately carry no server payload, screenshot text, or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    #[error("device rejected the operation")]
    Rejected,
    #[error("device is unavailable")]
    Unavailable,
    #[error("invalid device response")]
    InvalidResponse,
    #[error("operation may have executed; inspect the device before continuing")]
    OutcomeUnknown,
}

/// OCR strings may include personal data. Intentionally does not implement Debug
/// or serialization; progress only carries step indices and structured errors.
pub struct TextElement {
    pub text: String,
    pub bounds: Rect,
}

pub struct Observation {
    pub frame: Frame,
    pub bundle_id: Option<String>,
    pub elements: Vec<TextElement>,
}

impl Observation {
    pub fn is_valid(&self) -> bool {
        self.frame.is_valid()
            && self.elements.len() <= 4096
            && self.elements.iter().all(|element| {
                element.text.len() <= 16 * 1024 && element.bounds.is_inside(self.frame)
            })
    }
}

/// Only native device operations, never business actions. Each mutation is sent
/// once. Implementations must not replay requests, must release any dispatched
/// touch on cancellation, and must distinguish rejection from unknown outcomes.
pub trait Device: Send {
    fn observe(&mut self) -> DeviceFuture<'_, Observation>;
    fn tap(&mut self, frame: Frame, point: Point) -> DeviceFuture<'_, ()>;
    /// Inserts into the focused field. Does not clear, submit, or verify it.
    fn input_text<'a>(&'a mut self, text: &'a str) -> DeviceFuture<'a, ()>;
}
