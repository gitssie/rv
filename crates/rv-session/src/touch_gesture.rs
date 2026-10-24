//! Explicit, single-stroke test playback. Planning and timing stay in RV;
//! the phone receives ordinary authenticated begin/move/end HTTP/2 requests.
use crate::{AppCommand, phone_http::PhoneHttpClient};
use serde::Deserialize;
use serde_json::json;
use std::{io::Read, sync::OnceLock, time::Duration};
use tokio::sync::{oneshot, watch};

#[derive(Deserialize)]
struct Recording {
    points: Vec<[f64; 2]>,
    timing: Vec<f64>,
}

fn recordings() -> Result<&'static [Recording], String> {
    static DATA: OnceLock<Result<Vec<Recording>, String>> = OnceLock::new();
    DATA.get_or_init(|| {
        let mut data = Vec::new();
        flate2::read::GzDecoder::new(&include_bytes!("../assets/cursory/trajectories.json.gz")[..])
            .take(8 * 1024 * 1024)
            .read_to_end(&mut data)
            .map_err(|_| "触摸轨迹库读取失败".to_string())?;
        serde_json::from_slice(&data).map_err(|_| "触摸轨迹库格式错误".to_string())
    })
    .as_ref()
    .map(Vec::as_slice)
    .map_err(Clone::clone)
}

#[derive(Clone, Debug)]
struct Sample {
    point: [f64; 2],
    at: Duration,
}

/// Rotate/scale a recording onto endpoints, preserving its relative clock.
/// Overshooting outside the screen rejects the entire candidate, rather than
/// clipping individual points into an artificial path along a screen edge.
fn transform(
    recording: &Recording,
    start: [f64; 2],
    end: [f64; 2],
    size: (u32, u32),
) -> Option<Vec<Sample>> {
    let count = recording.points.len();
    if !(3..=1024).contains(&count) || recording.timing.len() != count {
        return None;
    }
    let first = recording.points[0];
    let last = recording.points[count - 1];
    let (dx, dy) = (last[0] - first[0], last[1] - first[1]);
    let squared = dx * dx + dy * dy;
    let total = recording.timing[count - 1] - recording.timing[0];
    if !squared.is_finite() || squared < 1. || !total.is_finite() || total <= 0. {
        return None;
    }
    let tx = end[0] - start[0];
    let ty = end[1] - start[1];
    let a = (tx * dx + ty * dy) / squared;
    let b = (ty * dx - tx * dy) / squared;
    // Bound one test's duration while scaling the complete relative clock.
    let clock_scale = total.clamp(300., 1500.) / total;
    let mut samples: Vec<Sample> = Vec::with_capacity(count);
    for (i, point) in recording.points.iter().enumerate() {
        let (x, y) = (point[0] - first[0], point[1] - first[1]);
        let point = if i == 0 {
            start
        } else if i == count - 1 {
            end
        } else {
            [start[0] + a * x - b * y, start[1] + b * x + a * y]
        };
        let ms = (recording.timing[i] - recording.timing[0]) * clock_scale;
        if !ms.is_finite()
            || ms < 0.
            || point.iter().any(|n| !n.is_finite())
            || point[0] < 0.
            || point[1] < 0.
            || point[0] >= f64::from(size.0)
            || point[1] >= f64::from(size.1)
        {
            return None;
        }
        let at = Duration::from_secs_f64(ms / 1000.);
        if samples.last().is_some_and(|previous| previous.at > at) {
            return None;
        }
        samples.push(Sample { point, at });
    }
    Some(samples)
}

/// Keep a bounded event density after changing duration. Interval weights come
/// from the recording, and positions are interpolated by time, not path length,
/// so resampling retains its speed changes and pauses rather than imposing a
/// constant travel speed. The original endpoints and total duration stay exact.
fn resample(recorded: &[Sample], offset: u8) -> Vec<Sample> {
    let total = recorded.last().unwrap().at;
    let count = (total.as_secs_f64() * 60.).round().max(2.) as usize;
    let gaps: Vec<_> = recorded
        .windows(2)
        .map(|pair| (pair[1].at - pair[0].at).as_secs_f64())
        .filter(|gap| *gap > 0.)
        .collect();
    let mut weights = Vec::with_capacity(count);
    for i in 0..count {
        let source = (((i as f64 + 0.5) * gaps.len() as f64 / count as f64) as usize
            + usize::from(offset))
            % gaps.len();
        weights.push(gaps[source]);
    }
    let sum: f64 = weights.iter().sum();
    let mut elapsed = 0.;
    let mut cursor = 0;
    let mut output = vec![recorded[0].clone()];
    for (i, weight) in weights.into_iter().enumerate() {
        elapsed += weight;
        let at = if i == count - 1 {
            total
        } else {
            Duration::from_secs_f64(total.as_secs_f64() * (elapsed / sum).clamp(0., 1.))
        };
        while cursor + 1 < recorded.len() - 1 && recorded[cursor + 1].at < at {
            cursor += 1;
        }
        let before = &recorded[cursor];
        let after = &recorded[cursor + 1];
        let span = (after.at - before.at).as_secs_f64();
        let alpha = if span > 0. {
            (at - before.at).as_secs_f64() / span
        } else {
            1.
        };
        let point = if i == count - 1 {
            recorded.last().unwrap().point
        } else {
            [
                before.point[0] + alpha * (after.point[0] - before.point[0]),
                before.point[1] + alpha * (after.point[1] - before.point[1]),
            ]
        };
        output.push(Sample { point, at });
    }
    output
}

fn plan(size: (u32, u32), entropy: [u8; 16]) -> Result<Vec<Sample>, String> {
    if size.0 < 32 || size.1 < 32 || size.0 > 16384 || size.1 > 16384 {
        return Err("手机画面尺寸不适合触摸测试".into());
    }
    let fraction = |i: usize| f64::from(entropy[i]) / 255.;
    // Explicit test strokes stay away from system screen edges. Either vertical
    // direction is possible; their endpoints vary within the middle of the screen.
    let start = [
        f64::from(size.0) * (0.35 + fraction(0) * 0.3),
        f64::from(size.1) * (0.3 + fraction(1) * 0.1),
    ];
    let end = [
        f64::from(size.0) * (0.35 + fraction(2) * 0.3),
        f64::from(size.1) * (0.6 + fraction(3) * 0.1),
    ];
    let (start, end) = if entropy[4] & 1 == 0 {
        (start, end)
    } else {
        (end, start)
    };
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let distance = dx.hypot(dy);
    let data = recordings()?;
    let mut candidates: Vec<_> = data
        .iter()
        .filter_map(|recording| {
            let first = recording.points.first()?;
            let last = recording.points.last()?;
            let rx = last[0] - first[0];
            let ry = last[1] - first[1];
            let length = rx.hypot(ry);
            if !length.is_finite() || length < 1. {
                return None;
            }
            let score = 0.8 * (1. - (rx * dx + ry * dy) / (length * distance))
                + 0.2 * (length - distance).abs() / distance;
            Some((score, recording))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let best: Vec<_> = candidates
        .into_iter()
        .filter_map(|(_, recording)| transform(recording, start, end, size))
        .take(5)
        .collect();
    if best.is_empty() {
        return Err("没有适合当前画面的触摸轨迹".into());
    }
    Ok(resample(
        &best[usize::from(entropy[5]) % best.len()],
        entropy[6],
    ))
}

async fn screen_size(client: &PhoneHttpClient) -> Result<(u32, u32), String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        let mut reply = client
            .request(
                reqwest::Method::POST,
                "/api/device/screenshot",
                headers,
                b"{}".to_vec(),
            )
            .await
            .map_err(|_| "手机截图请求中断")?;
        if !reply.status().is_success() {
            return Err("手机拒绝截图请求".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = reply.chunk().await.map_err(|_| "手机截图回复中断")? {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err("手机截图超过限制".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "手机截图格式错误")?
            .into_dimensions()
            .map_err(|_| "无法读取手机截图尺寸".into())
    })
    .await
    .map_err(|_| "手机截图超时".to_string())?
}

async fn touch(client: &PhoneHttpClient, phase: u8, point: [f64; 2]) -> Result<(), String> {
    let (status, body) = client
        .json(
            "/api/device/touch",
            &json!({"touches":[{
                "type":phase,"finger":0,"x":point[0],"y":point[1]
            }]}),
        )
        .await?;
    if status != 200 || body["ok"] != true {
        return Err(format!(
            "手机拒绝触摸：{}",
            body["error"].as_str().unwrap_or("回复未确认")
        ));
    }
    Ok(())
}

async fn replay(
    client: &PhoneHttpClient,
    samples: &[Sample],
    cancel: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    if samples.len() < 2 || *cancel.borrow() {
        return Err("触摸测试已取消".into());
    }
    let mut last = samples[0].point;
    // Even an interrupted begin may have reached the phone. Always attempt end,
    // then drop this operation's isolated connection as the final cleanup path.
    let result = tokio::time::timeout(Duration::from_secs(6), async {
        tokio::select! {
            biased;
            _ = cancel.changed() => return Err("触摸测试已取消".into()),
            value = touch(client, 1, last) => value?,
        }
        let clock = tokio::time::Instant::now();
        for sample in &samples[1..] {
            tokio::select! {
                biased;
                _ = cancel.changed() => return Err("触摸测试已取消".into()),
                _ = tokio::time::sleep_until(clock + sample.at) => {}
            }
            if sample.point == last {
                continue;
            }
            last = sample.point;
            tokio::select! {
                biased;
                _ = cancel.changed() => return Err("触摸测试已取消".into()),
                value = touch(client, 2, last) => value?,
            }
        }
        Ok(())
    })
    .await
    .unwrap_or_else(|_| Err("触摸测试超时，已停止后续移动".into()));
    let release = touch(client, 0, last).await;
    match (result, release) {
        (_, Err(_)) => Err("触摸释放未确认；连接已关闭，请检查手机".into()),
        (result, Ok(())) => result,
    }
}

pub(crate) struct GestureOperation {
    pub command: AppCommand,
    pub result: oneshot::Receiver<Result<(), String>>,
    cancel: watch::Sender<bool>,
}
impl GestureOperation {
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }

    pub fn start(client: PhoneHttpClient, id: u32) -> Result<Self, String> {
        let client = client.isolated().map_err(|_| "无法建立触摸测试连接")?;
        let (cancel, mut cancelled) = watch::channel(false);
        let (sender, result) = oneshot::channel();
        tokio::spawn(async move {
            let value = async {
                let size = tokio::select! {
                    biased;
                    _ = cancelled.changed() => return Err("触摸测试已取消".into()),
                    size = screen_size(&client) => size?,
                };
                let samples = plan(size, *uuid::Uuid::new_v4().as_bytes())?;
                tracing::debug!(
                    points = samples.len(),
                    width = size.0,
                    height = size.1,
                    planned_ms = samples.last().unwrap().at.as_millis() as u64,
                    "ZXTouch single-stroke test started"
                );
                replay(&client, &samples, &mut cancelled).await
            }
            .await;
            drop(client);
            let _ = sender.send(value);
        });
        Ok(Self {
            command: AppCommand::RandomTouch(id),
            result,
            cancel,
        })
    }
}
impl Drop for GestureOperation {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests;
