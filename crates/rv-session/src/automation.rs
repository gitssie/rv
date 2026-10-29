//! Authenticated HTTP/2 adapter for the transport-independent workflow engine.
//! Reuses the existing control session; never logs inputs or falls back to VNC.
pub mod vinted;
use rv_automation::{
    Device, DeviceError, DeviceFuture, Frame, MAX_INPUT_UTF16, Observation, Point, Rect,
    TextElement,
};
use serde_json::{Value, json};

use crate::phone_http::PhoneHttpClient;

pub struct ZxTouchDevice {
    client: PhoneHttpClient,
    last_tap: Option<(Frame, Point)>,
}
impl ZxTouchDevice {
    /// Supply the phone's existing authenticated client. Does not log in, store
    /// passwords, launch apps, or acquire exclusive input ownership for the UI.
    pub fn new(client: PhoneHttpClient) -> Self {
        Self {
            client,
            last_tap: None,
        }
    }

    async fn post_paste_touch(
        &self,
        frame: Frame,
        anchor: Point,
        bundle: Option<String>,
    ) -> Result<(), DeviceError> {
        use crate::paste_touch::PasteTouch;
        let mut entropy = [0; 32];
        entropy[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        entropy[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        let plan = PasteTouch::new(
            (anchor.x as u16, anchor.y as u16),
            (frame.width as u16, frame.height as u16),
            entropy,
            std::time::Instant::now(),
        )
        .ok_or(DeviceError::Rejected)?;
        tokio::time::sleep(PasteTouch::INITIAL_DELAY).await;
        // Each native hold releases its finger even if this future is cancelled.
        // No queued task survives cancellation to send the next touch.
        for (index, hold) in [plan.long_hold(), plan.short_hold()]
            .into_iter()
            .enumerate()
        {
            if index != 0 {
                tokio::time::sleep(PasteTouch::TAP_GAP).await;
            }
            if bundle.is_none() || self.foreground().await? != bundle {
                return Err(DeviceError::Rejected);
            }
            let client = self
                .client
                .isolated()
                .map_err(|_| DeviceError::Unavailable)?;
            let (status, body) = client.json("/api/device/tap", &json!({
                "x":plan.point.0,"y":plan.point.1,"frame":frame,"hold_ms":hold.as_millis() as u64,
            })).await.map_err(|_| DeviceError::OutcomeUnknown)?;
            if status != 200 || body["ok"] != true || body.get("error").is_some() {
                return Err(DeviceError::OutcomeUnknown);
            }
        }
        Ok(())
    }

    async fn foreground(&self) -> Result<Option<String>, DeviceError> {
        let body = read(&self.client, "/api/apps/foreground", &json!({})).await?;
        match body.get("bundle_id") {
            Some(Value::Null) => Ok(None),
            Some(Value::String(value)) if !value.is_empty() && value.len() <= 255 => {
                Ok(Some(value.clone()))
            }
            _ => Err(DeviceError::InvalidResponse),
        }
    }
}

impl Device for ZxTouchDevice {
    fn observe(&mut self) -> DeviceFuture<'_, Observation> {
        Box::pin(async move {
            let before = self.foreground().await?;
            let body = read(
                &self.client,
                "/api/device/ocr",
                &json!({
                "fields":["1","0,,0,,0,,0","","0.008","0","zh-Hans,,en-US,,fr-FR","1",""]
                }),
            )
            .await?;
            let after = self.foreground().await?;
            // OCR and foreground are separate requests. Reject a visible app
            // change rather than assigning the previous app's coordinates.
            if before != after {
                return Err(DeviceError::InvalidResponse);
            }
            parse_observation(&body, after)
        })
    }

    fn tap(&mut self, frame: Frame, point: Point) -> DeviceFuture<'_, ()> {
        Box::pin(async move {
            if !frame.contains(point) {
                return Err(DeviceError::Rejected);
            }
            let (status, body) = self
                .client
                .json(
                    "/api/device/tap",
                    &json!({
                        "x":point.x,"y":point.y,"frame":frame
                    }),
                )
                .await
                .map_err(|_| DeviceError::OutcomeUnknown)?;
            if status == 200 && body["ok"] == true && body.get("error").is_none() {
                self.last_tap = Some((frame, point));
                return Ok(());
            }
            self.last_tap = None;
            if matches!(status, 400 | 401 | 403 | 404)
                || matches!(
                    body["error"].as_str(),
                    Some(
                        "frame_changed"
                            | "touch_outside_screen"
                            | "touch_busy"
                            | "request_cancelled"
                    )
                )
            {
                return Err(DeviceError::Rejected);
            }
            Err(DeviceError::OutcomeUnknown)
        })
    }

    fn input_text<'a>(&'a mut self, text: &'a str) -> DeviceFuture<'a, ()> {
        Box::pin(async move {
            // Bound one literal insertion below the native 256-char ceiling.
            // Never split/retry a partially applied string.
            if text.is_empty()
                || text.encode_utf16().count() > MAX_INPUT_UTF16
                || text.chars().any(char::is_control)
            {
                return Err(DeviceError::Rejected);
            }
            // Keyboard1 is physical ASCII input and depends on the phone's
            // hardware layout. Form values require literal-text command8,
            // advertised by 4.0-7+, with no multi-request clipboard fallback.
            let capabilities = read(&self.client, "/api/control/authorize", &json!({})).await?;
            if capabilities["keyboard_literal"] != 1
                || (self.last_tap.is_some() && capabilities["device_tap_hold"] != 1)
            {
                return Err(DeviceError::Rejected);
            }
            let bundle = if self.last_tap.is_some() {
                self.foreground().await?
            } else {
                None
            };
            let (status, body) = self
                .client
                .json(
                    "/api/device/keyboard",
                    &json!({
                        "fields":["8",text]
                    }),
                )
                .await
                .map_err(|_| DeviceError::OutcomeUnknown)?;
            // Daemon keyboard returns {value:""}; the foreground app adapter
            // returns {ok:true}. Neither envelope proves field acceptance.
            let acknowledged = matches!(
                (body.get("ok"), body.get("value")),
                (Some(Value::Bool(true)), None) | (None, Some(Value::String(_)))
            );
            if status == 200 && body.get("error").is_none() && acknowledged {
                if let Some((frame, anchor)) = self.last_tap.take() {
                    self.post_paste_touch(frame, anchor, bundle)
                        .await
                        .map_err(|_| DeviceError::OutcomeUnknown)?;
                }
                return Ok(());
            }
            // The native facade also uses HTTP400 for app-adapter timeouts
            // AFTER dispatch. That status cannot establish non-execution.
            if matches!(status, 401 | 403 | 404) {
                return Err(DeviceError::Rejected);
            }
            Err(DeviceError::OutcomeUnknown)
        })
    }
}

async fn read(client: &PhoneHttpClient, path: &str, payload: &Value) -> Result<Value, DeviceError> {
    let (status, body) = client
        .json(path, payload)
        .await
        .map_err(|_| DeviceError::Unavailable)?;
    if matches!(status, 400 | 401 | 403 | 404) {
        return Err(DeviceError::Rejected);
    }
    if status != 200 {
        return Err(DeviceError::Unavailable);
    }
    if body.get("error").is_some() {
        return Err(DeviceError::Rejected);
    }
    Ok(body)
}

fn parse_observation(body: &Value, bundle_id: Option<String>) -> Result<Observation, DeviceError> {
    let frame: Frame =
        serde_json::from_value(body["frame"].clone()).map_err(|_| DeviceError::InvalidResponse)?;
    let value = body["value"].as_str().ok_or(DeviceError::InvalidResponse)?;
    let mut elements = Vec::new();
    if !value.is_empty() {
        for line in value.split(";;") {
            // Coordinates are the final four fields. Text may contain commas.
            let parts: Vec<_> = line.rsplitn(5, ",,").collect();
            if parts.len() != 5 || elements.len() >= 4096 {
                return Err(DeviceError::InvalidResponse);
            }
            let number = |index: usize| {
                parts[index]
                    .parse::<f64>()
                    .map_err(|_| DeviceError::InvalidResponse)
            };
            elements.push(TextElement {
                text: parts[4].to_owned(),
                bounds: Rect {
                    x: number(3)?,
                    y: number(2)?,
                    width: number(1)?,
                    height: number(0)?,
                },
            });
        }
    }
    let observation = Observation {
        frame,
        bundle_id,
        elements,
    };
    if !observation.is_valid() {
        return Err(DeviceError::InvalidResponse);
    }
    Ok(observation)
}

#[cfg(test)]
mod tests;
