//! Authenticated HTTP/2 adapter for the transport-independent workflow engine.
//! Reuses the existing control session; never logs inputs or falls back to VNC.
use rv_automation::{
    Device, DeviceError, DeviceFuture, Frame, MAX_INPUT_UTF16, Observation, Point, Rect,
    TextElement,
};
use serde_json::{Value, json};

use crate::phone_http::PhoneHttpClient;

pub struct ZxTouchDevice {
    client: PhoneHttpClient,
}
impl ZxTouchDevice {
    /// Supply the phone's existing authenticated client. Does not log in, store
    /// passwords, launch apps, or acquire exclusive input ownership for the UI.
    pub fn new(client: PhoneHttpClient) -> Self {
        Self { client }
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
                    "fields":["1","0,,0,,0,,0","","0.008","0","zh-Hans,,en-US","1",""]
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
                return Ok(());
            }
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
            // Bound one keyboard command below the native 256-char ceiling and
            // HTTP deadline (ASCII keys take ~50ms each). Never split/retry a
            // partially applied string. Business layers can use separate steps.
            if text.is_empty()
                || text.encode_utf16().count() > MAX_INPUT_UTF16
                || text.chars().any(char::is_control)
            {
                return Err(DeviceError::Rejected);
            }
            let (status, body) = self
                .client
                .json(
                    "/api/device/keyboard",
                    &json!({
                        "fields":["1",text]
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
