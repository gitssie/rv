//! ZXTouch HTTP/2 confirmation for an already accepted FileTransfer Photos delete.
use crate::phone_http::PhoneHttpClient;
use serde_json::json;
use std::time::Duration;

pub(crate) struct Confirmation {
    task: tokio::task::JoinHandle<()>,
    progress: tokio::sync::watch::Receiver<&'static str>,
}
impl Confirmation {
    pub(crate) fn progress(&mut self) -> Option<&'static str> {
        self.progress
            .has_changed()
            .unwrap_or(false)
            .then(|| *self.progress.borrow_and_update())
    }

    pub(crate) fn cancel(&self) {
        self.task.abort();
    }
}
impl Drop for Confirmation {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct ButtonPoint {
    x: f64,
    y: f64,
}
struct TextBox<'a> {
    text: &'a str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn button(text: &str) -> Option<ButtonPoint> {
    let boxes: Vec<_> = text
        .split(";;")
        .filter_map(|line| {
            let fields: Vec<_> = line.split(",,").collect();
            if fields.len() != 5 {
                return None;
            }
            let values: Vec<f64> = fields[1..]
                .iter()
                .map(|v| v.parse().ok())
                .collect::<Option<_>>()?;
            if values.iter().any(|v| !v.is_finite() || *v < 0.)
                || values[2] <= 0.
                || values[3] <= 0.
            {
                return None;
            }
            Some(TextBox {
                text: fields[0].trim(),
                x: values[0],
                y: values[1],
                w: values[2],
                h: values[3],
            })
        })
        .collect();
    // Button labels are localized. Match the Photos requester above a single
    // side-by-side button row, then use the right button's relative position.
    let mut candidates = Vec::new();
    for left in &boxes {
        for right in &boxes {
            let (lx, ly) = (left.x + left.w / 2., left.y + left.h / 2.);
            let (rx, ry) = (right.x + right.w / 2., right.y + right.h / 2.);
            if left.x + left.w >= right.x || (ly - ry).abs() > left.h.max(right.h) / 2. {
                continue;
            }
            let header: Vec<_> = boxes
                .iter()
                .filter(|b| b.y + b.h + 2. * left.h.max(right.h) < left.y.min(right.y))
                .collect();
            let text = header
                .iter()
                .map(|b| b.text)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let requester: String = text
                .chars()
                .filter(|c| !c.is_whitespace() && !matches!(c, '-' | '\u{ad}'))
                .collect();
            if requester.matches("trollvncserver").count() != 1
                || !["photo", "foto", "照片", "相片", "写真", "사진", "фото"]
                    .iter()
                    .any(|word| text.contains(word))
                || ![
                    "delete", "supprim", "lösch", "elimin", "apag", "exclu", "删除", "刪除",
                    "削除", "삭제", "удал",
                ]
                .iter()
                .any(|word| text.contains(word))
            {
                continue;
            }
            let min_x = header.iter().map(|b| b.x).fold(f64::INFINITY, f64::min);
            let max_x = header
                .iter()
                .map(|b| b.x + b.w)
                .fold(f64::NEG_INFINITY, f64::max);
            let center = (min_x + max_x) / 2.;
            if lx < center && center < rx {
                candidates.push(ButtonPoint { x: rx, y: ry });
            }
        }
    }
    // Ambiguous geometry is left for manual confirmation, never guessed.
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}

async fn recognize(
    client: &PhoneHttpClient,
    languages: &str,
) -> Result<Option<(ButtonPoint, serde_json::Value)>, String> {
    let (status, response) = client
        .json(
            "/api/device/ocr",
            &json!({"fields":["1","0,,0,,0,,0","","0.008","0",languages,"1",""]}),
        )
        .await?;
    if status != 200 {
        return Err(format!("Photos confirmation OCR rejected: HTTP {status}"));
    }
    let text = response["value"]
        .as_str()
        .ok_or("Invalid Photos confirmation OCR response")?;
    let Some(point) = button(text) else {
        return Ok(None);
    };
    let frame = &response["frame"];
    for key in ["width", "height", "orientation"] {
        if frame[key].as_f64().is_none_or(|n| !n.is_finite()) {
            return Err("OCR frame metadata missing".into());
        }
    }
    Ok(Some((point, frame.clone())))
}
async fn recognition_languages(client: &PhoneHttpClient) -> Result<String, String> {
    let (status, response) = client
        .json("/api/device/ocr", &json!({"fields":["2","0"]}))
        .await?;
    if status != 200 {
        return Err(format!("Photos OCR languages rejected: HTTP {status}"));
    }
    let supported = response["value"]
        .as_str()
        .ok_or("Invalid OCR language list")?;
    let languages: Vec<_> = supported
        .split(";;")
        .filter(|language| !language.is_empty())
        .collect();
    if languages.is_empty() {
        return Err("No supported Photos OCR languages".into());
    }
    Ok(languages.join(",,"))
}

async fn observe(
    client: PhoneHttpClient,
    task: &str,
    progress: &tokio::sync::watch::Sender<&'static str>,
) -> Result<(), String> {
    let (status, caps) = client.json("/api/control/authorize", &json!({})).await?;
    if status != 200 || caps["device_tap"] != 1 {
        return Err("Phone does not support HTTP/2 Photos confirmation; confirm on the phone or update TrollVNC".into());
    }
    let languages = recognition_languages(&client).await?;
    for _ in 0..40 {
        if let Some((point, frame)) = recognize(&client, &languages).await? {
            // RV owns the delete task and cancels this observer on its terminal
            // result. The phone receives only an independent generic tap. Never replay it.
            progress.send_replace("Confirming deletion on iPhone…");
            let (status, reply) = client
                .json(
                    "/api/device/tap",
                    &json!({"x":point.x,"y":point.y,"frame":frame}),
                )
                .await?;
            if status != 200 || reply["ok"] != true {
                return Err(format!(
                    "Photos confirmation rejected: HTTP {status}, reason={}",
                    reply["error"].as_str().unwrap_or("unknown")
                ));
            }
            tracing::info!(
                task_id = task,
                "Photos confirmation tap sent through ZXTouch HTTP/2"
            );
            tokio::time::sleep(Duration::from_millis(600)).await;
            if recognize(&client, &languages).await?.is_some() {
                return Err("Photos prompt remains after HTTP/2 tap; no second tap sent".into());
            }
            progress.send_replace("Waiting for Photos deletion result…");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("No matching Photos deletion prompt detected; manual confirmation remains available".into())
}
pub(crate) fn start(client: PhoneHttpClient, task: String) -> Confirmation {
    let (progress, mut receiver) = tokio::sync::watch::channel("Recognizing iPhone confirmation…");
    receiver.mark_changed();
    let worker = tokio::spawn(async move {
        let result =
            tokio::time::timeout(Duration::from_secs(35), observe(client, &task, &progress)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                progress.send_replace("Confirm deletion on iPhone; waiting for result…");
                tracing::warn!(task_id=%task,%error,"Photos automatic confirmation stopped")
            }
            Err(_) => {
                progress.send_replace("Confirm deletion on iPhone; waiting for result…");
                tracing::warn!(task_id=%task,"Photos confirmation deadline reached; no delete request replayed")
            }
        }
        // Keep the stage channel open until the authoritative FileTransfer result
        // drops this observer. No repeated notifications after the worker finishes.
        progress.closed().await;
    });
    Confirmation {
        task: worker,
        progress: receiver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::{BodyExt, Full};
    use hyper::{
        Request, Response,
        body::{Bytes, Incoming},
        service::service_fn,
    };
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use std::sync::{Arc, Mutex};
    const PROMPT: &str = "Allow trollvncserver.unsigned to delete this photo?,,100,,100,,400,,40;;Don't Allow,,100,,500,,130,,30;;Delete,,350,,500,,100,,30";
    #[test]
    fn matches_only_the_expected_prompt_and_button_row() {
        assert_eq!(button(PROMPT), Some(ButtonPoint { x: 400., y: 515. }));
        for invalid in [
            PROMPT.replace("trollvncserver", "another-app"),
            PROMPT.replace("photo", "file"),
            PROMPT.replace("350,,500", "350,,250"),
            format!("{PROMPT};;Delete,,350,,500,,100,,30"),
            PROMPT.replace("100,,100,,400", "NaN,,100,,400"),
        ] {
            assert_eq!(button(&invalid), None, "{invalid}");
        }
        assert_eq!(
            button(
                "允许trollvncserver删除此照片？,,100,,100,,400,,40;;不允许,,100,,500,,130,,30;;删除,,350,,500,,100,,30"
            ),
            Some(ButtonPoint { x: 400., y: 515. })
        );
    }

    const FRENCH_PROMPT: &str = "Autoriser « trollvncser-,,30,,10,,190,,20;;ver.b0abb8ec.unsigned » à,,20,,30,,210,,20;;supprimer cette photo ?,,20,,50,,210,,20;;Refuser,,25,,350,,70,,20;;Supprimer,,145,,350,,90,,20";
    #[test]
    fn recognizes_localized_photos_deletion_with_split_requester() {
        assert_eq!(
            button(FRENCH_PROMPT),
            Some(ButtonPoint { x: 190., y: 360. })
        );
        for (prompt, deny, delete) in [
            (
                "Allow trollvncserver to delete this photo?",
                "Don’t Allow",
                "Delete",
            ),
            ("允許 trollvncserver 刪除此照片？", "不允許", "刪除"),
            (
                "trollvncserver dieses Foto löschen?",
                "Nicht erlauben",
                "Löschen",
            ),
            (
                "¿Permitir a trollvncserver eliminar esta foto?",
                "No permitir",
                "Eliminar",
            ),
            (
                "Consentire a trollvncserver di eliminare questa foto?",
                "Non consentire",
                "Elimina",
            ),
            (
                "Permitir que trollvncserver apague esta foto?",
                "Não Permitir",
                "Apagar",
            ),
        ] {
            let text = format!(
                "{prompt},,10,,10,,220,,50;;{deny},,10,,350,,90,,20;;{delete},,145,,350,,90,,20"
            );
            assert_eq!(
                button(&text),
                Some(ButtonPoint { x: 190., y: 360. }),
                "{text}"
            );
        }
        for text in [
            FRENCH_PROMPT.replace("photo", "fichier"),
            FRENCH_PROMPT.replace("trollvncser-", "other-"),
            FRENCH_PROMPT.replace("145,,350", "145,,5"),
            format!("{FRENCH_PROMPT};;Supprimer,,145,,350,,90,,20"),
        ] {
            assert_eq!(button(&text), None, "{text}");
        }
    }

    #[test]
    fn screenshot_geometry_is_relative_and_does_not_depend_on_button_labels() {
        // Vision revision 2 output from the user's French screenshot.
        let text = "Autoriser « trollvncser-,,33,,5,,157,,11;;ver.bOabb8ec.unsigned » à,,17,,23,,185,,13;;supprimer cette photo ?,,29,,41,,163,,15;;Refuser,,29,,260,,51,,14;;Supprimer,,131,,260,,73,,15";
        assert_eq!(button(text), Some(ButtonPoint { x: 167.5, y: 267.5 }));
        assert_eq!(
            button(&text.replace("Refuser", "Non").replace("Supprimer", "Oui")),
            button(text)
        );
        // Same dialog at twice the scale and a different screen offset.
        let shifted = text
            .split(";;")
            .map(|line| {
                let p: Vec<_> = line.split(",,").collect();
                let v: Vec<f64> = p[1..].iter().map(|n| n.parse().unwrap()).collect();
                format!(
                    "{},,{},,{},,{},,{}",
                    p[0],
                    v[0] * 2. + 80.,
                    v[1] * 2. + 120.,
                    v[2] * 2.,
                    v[3] * 2.
                )
            })
            .collect::<Vec<_>>()
            .join(";;");
        assert_eq!(button(&shifted), Some(ButtonPoint { x: 415., y: 655. }));
        assert_eq!(
            button(&text.replace("supprimer cette photo ?", "accéder à cette photo ?")),
            None
        );
    }

    #[tokio::test]
    async fn confirmation_uses_http2_once_for_the_original_task_without_replaying_delete() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let received = calls.clone();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(
                    TokioIo::new(socket),
                    service_fn(move |request: Request<Incoming>| {
                        let calls = received.clone();
                        async move {
                            assert_eq!(request.version(), hyper::Version::HTTP_2);
                            assert_eq!(request.headers()["authorization"], "Bearer fixture");
                            let path = request.uri().path().to_owned();
                            let body: serde_json::Value = serde_json::from_slice(
                                &request.into_body().collect().await.unwrap().to_bytes(),
                            )
                            .unwrap();
                            calls.lock().unwrap().push((path.clone(), body.clone()));
                            let reply = match path.as_str() {
                                "/api/control/authorize" => json!({"device_tap":1}),
                                "/api/device/ocr" if body["fields"][0] == "2" => json!({"value":"en-US;;fr-FR;;zh-Hans"}),
                                "/api/device/ocr" => {
                                    assert_eq!(body["fields"][5], "en-US,,fr-FR,,zh-Hans");
                                    json!({"value":FRENCH_PROMPT,"frame":{"width":1000,"height":1000,"orientation":0}})
                                },
                                "/api/device/tap" => {
                                    assert_eq!(body, json!({"x":190.,"y":360.,"frame":{"width":1000,"height":1000,"orientation":0}}));
                                    json!({"ok":true})
                                }
                                _ => panic!("unexpected request: {path}"),
                            };
                            Ok::<_, std::convert::Infallible>(Response::new(Full::new(
                                Bytes::from(reply.to_string()),
                            )))
                        }
                    }),
                )
                .await
                .unwrap();
        });
        let client = PhoneHttpClient::new("127.0.0.1", port, "fixture".into()).unwrap();
        let (progress, _) = tokio::sync::watch::channel("");
        let error = observe(client, "original-task", &progress)
            .await
            .unwrap_err();
        assert!(error.contains("no second tap"));
        let paths: Vec<_> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|(path, _)| path.clone())
            .collect();
        assert_eq!(
            paths,
            [
                "/api/control/authorize",
                "/api/device/ocr",
                "/api/device/ocr",
                "/api/device/tap",
                "/api/device/ocr"
            ]
        );
        server.abort();
    }
}
