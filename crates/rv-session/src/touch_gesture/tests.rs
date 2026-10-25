use super::*;
use std::sync::{Arc, Mutex};

#[test]
fn recorded_plans_fit_native_screen_and_preserve_nonuniform_timing() {
    assert_eq!(recordings().unwrap().len(), 2356);
    for size in [(1080, 1920), (1920, 1080), (750, 1334)] {
        for value in 0..32 {
            let samples = plan(size, [value; 16]).unwrap();
            assert!(samples.len() >= 3);
            assert!(samples.len() <= 91);
            assert_eq!(samples[0].at, Duration::ZERO);
            assert_ne!(samples[0].point, samples.last().unwrap().point);
            let duration = samples.last().unwrap().at;
            assert!(duration >= Duration::from_millis(299));
            assert!(duration <= Duration::from_millis(1501));
            assert!(samples.windows(2).all(|pair| pair[0].at <= pair[1].at));
            for sample in &samples {
                assert!(sample.point[0] >= 0. && sample.point[0] < f64::from(size.0));
                assert!(sample.point[1] >= 0. && sample.point[1] < f64::from(size.1));
            }
        }
    }
    assert!(plan((0, 800), [0; 16]).is_err());
    assert!(plan((400, 20000), [0; 16]).is_err());
    let recording = Recording {
        points: vec![[0., 0.], [2., 1.], [4., 0.]],
        timing: vec![0., 100., 400.],
    };
    let samples = transform(&recording, [10., 10.], [50., 10.], (100, 100)).unwrap();
    assert_eq!(samples[0].point, [10., 10.]);
    assert_eq!(samples[1].point, [30., 20.]);
    assert_eq!(samples[2].point, [50., 10.]);
    assert_eq!(samples[1].at, Duration::from_millis(100));
    assert_eq!(samples[2].at, Duration::from_millis(400));
    let sampled = resample(&samples, 0);
    assert_eq!(sampled.len(), 25);
    assert_eq!(sampled[0].point, samples[0].point);
    assert_eq!(sampled.last().unwrap().point, samples[2].point);
    assert_eq!(sampled.last().unwrap().at, samples[2].at);
    assert!(sampled.windows(2).all(|pair| pair[0].at < pair[1].at));
    let intervals: Vec<_> = sampled
        .windows(2)
        .map(|pair| pair[1].at - pair[0].at)
        .collect();
    assert!(intervals.iter().any(|gap| *gap != intervals[0]));
    assert!(transform(&recording, [10., 95.], [50., 95.], (100, 100)).is_none());
}

type Events = Arc<Mutex<Vec<(tokio::time::Instant, serde_json::Value)>>>;

async fn peer(reject_move: bool) -> (PhoneHttpClient, Events, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut connection = h2::server::handshake(socket).await.unwrap();
        while let Some(request) = connection.accept().await {
            let (request, mut reply) = request.unwrap();
            assert_eq!(request.uri().path(), "/api/device/touch");
            assert_eq!(request.headers()["authorization"], "Bearer fixture");
            let captured = captured.clone();
            tokio::spawn(async move {
                let mut body = request.into_body();
                let mut bytes = Vec::new();
                while let Some(chunk) = body.data().await {
                    let chunk = chunk.unwrap();
                    body.flow_control().release_capacity(chunk.len()).unwrap();
                    bytes.extend_from_slice(&chunk);
                }
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let phase = value["touches"][0]["type"].as_u64().unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push((tokio::time::Instant::now(), value));
                let response = if reject_move && phase == 2 {
                    b"{\"error\":\"rejected\"}".as_slice()
                } else {
                    b"{\"ok\":true}".as_slice()
                };
                let mut stream = reply
                    .send_response(hyper::Response::new(()), false)
                    .unwrap();
                stream
                    .send_data(hyper::body::Bytes::copy_from_slice(response), true)
                    .unwrap();
            });
        }
    });
    (
        PhoneHttpClient::new("127.0.0.1", port, "fixture".into()).unwrap(),
        events,
        task,
    )
}

fn samples() -> Vec<Sample> {
    vec![
        Sample {
            point: [10., 20.],
            at: Duration::ZERO,
        },
        Sample {
            point: [20., 25.],
            at: Duration::from_millis(40),
        },
        Sample {
            point: [30., 40.],
            at: Duration::from_millis(90),
        },
    ]
}

#[tokio::test]
async fn http2_replay_uses_one_connection_and_one_finger_with_ordered_phases() {
    let (client, events, task) = peer(false).await;
    let (_cancel, mut receiver) = watch::channel(false);
    replay(&client, &samples(), &mut receiver).await.unwrap();
    let captured = events.lock().unwrap();
    let phases: Vec<_> = captured
        .iter()
        .map(|(_, v)| v["touches"][0]["type"].as_u64().unwrap())
        .collect();
    assert_eq!(phases, [1, 2, 2, 0]);
    assert!(captured.iter().all(|(_, v)| v["touches"][0]["finger"] == 0));
    assert!(captured[1].0.duration_since(captured[0].0) >= Duration::from_millis(35));
    assert!(captured[2].0.duration_since(captured[0].0) >= Duration::from_millis(85));
    assert_eq!(captured[3].1["touches"][0]["x"], 30.);
    task.abort();
}

#[tokio::test]
async fn rejected_move_still_releases_and_never_replays_begin() {
    let (client, events, task) = peer(true).await;
    let (_cancel, mut receiver) = watch::channel(false);
    assert!(replay(&client, &samples(), &mut receiver).await.is_err());
    let phases: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .map(|(_, v)| v["touches"][0]["type"].as_u64().unwrap())
        .collect();
    assert_eq!(phases, [1, 2, 0]);
    task.abort();
}

#[tokio::test]
async fn cancellation_releases_without_waiting_for_next_move() {
    let (client, events, task) = peer(false).await;
    let (cancel, mut receiver) = watch::channel(false);
    let captured = events.clone();
    let canceller = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(2), async {
            while captured.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        cancel.send_replace(true);
    });
    let samples = vec![
        samples()[0].clone(),
        Sample {
            point: [20., 20.],
            at: Duration::from_secs(5),
        },
    ];
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        replay(&client, &samples, &mut receiver),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    canceller.await.unwrap();
    let phases: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .map(|(_, v)| v["touches"][0]["type"].as_u64().unwrap())
        .collect();
    assert_eq!(phases, [1, 0]);
    task.abort();
}

#[tokio::test]
#[ignore = "Sends one real gesture; requires explicit RV_TOUCH_* environment and the local SDK Probe foreground"]
async fn live_sdk_probe_single_gesture() {
    let host = std::env::var("RV_TOUCH_HOST").expect("RV_TOUCH_HOST required");
    let port = std::env::var("RV_TOUCH_PORT")
        .unwrap_or_else(|_| "6000".into())
        .parse()
        .unwrap();
    let password = std::env::var("RV_TOUCH_PASSWORD").expect("RV_TOUCH_PASSWORD required");
    let client = PhoneHttpClient::new(&host, port, String::new()).unwrap();
    client.connect(&password).await.unwrap();
    let result: Result<(), String> = async {
        let (status, app) = client.json("/api/apps/foreground", &json!({})).await?;
        if status != 200 || app["bundle_id"] != "local.sdkprobe.app" {
            return Err("SDK Probe is not foreground; no touch sent".into());
        }
        let require_callbacks = std::env::var("RV_TOUCH_REQUIRE_CALLBACKS").as_deref() == Ok("1");
        if require_callbacks && visible_probe_callbacks(&client).await? != 0 {
            return Err("Clear the SDK Probe event display before the live test".into());
        }
        let size = screen_size(&client).await?;
        println!("Native screenshot dimensions: {} x {}", size.0, size.1);
        let mut operation = GestureOperation::start(client.clone(), 1)?;
        tokio::time::timeout(Duration::from_secs(20), &mut operation.result)
            .await
            .map_err(|_| "live gesture deadline".to_string())?
            .map_err(|_| "live gesture task closed".to_string())??;
        println!("One begin/move/end sequence acknowledged; inspect SDK Probe for UIKit delivery.");
        if require_callbacks {
            tokio::time::sleep(Duration::from_millis(350)).await;
            let count = visible_probe_callbacks(&client).await?;
            if count < 3 {
                return Err(format!(
                    "HTTP acknowledged but SDK Probe shows only {count} touch callbacks"
                ));
            }
            println!("SDK Probe visible callback count after isolated stroke: {count}");
        }
        Ok(())
    }
    .await;
    let _ = client.json("/api/control/disconnect", &json!({})).await;
    result.unwrap();
}

// Live diagnostic only, never part of normal gesture dispatch. Read the owned
// Probe's actual counter so a successful HTTP ACK cannot pass UIKit acceptance.
async fn visible_probe_callbacks(client: &PhoneHttpClient) -> Result<u32, String> {
    let (status, response) = client
        .json(
            "/api/device/ocr",
            &json!({"fields":[
                "1", "0,,0,,0,,0", "", "0.008", "0", "en-US,,zh-Hans", "1", ""
            ]}),
        )
        .await?;
    let text = response["value"]
        .as_str()
        .ok_or("Probe OCR reply missing")?;
    if status != 200 {
        return Err("Probe OCR rejected".into());
    }
    parse_probe_counter(text)
}

fn parse_probe_counter(text: &str) -> Result<u32, String> {
    // Vision can read the small blue zero as a parenthesis. The Probe's empty
    // event-log placeholder is a separate, unambiguous indication of zero.
    if text.contains("等待触摸回调") {
        return Ok(0);
    }
    let pattern = regex::Regex::new(r"当前显示\s*(\d+)\s*条回调").unwrap();
    pattern
        .captures(text)
        .and_then(|value| value[1].parse().ok())
        .ok_or_else(|| "SDK Probe event counter not visible; no UIKit acceptance claim".into())
}

#[test]
fn live_counter_requires_real_probe_text_and_accepts_empty_log_placeholder() {
    assert_eq!(
        parse_probe_counter("当前显示 59 条回调/0 份请求 events,,46,,1407,,605,,41").unwrap(),
        59
    );
    assert_eq!(
        parse_probe_counter(
            "当前显示 （条回调/0份请求 events;;等待触摸回调。可点击表单或在页面滑动。"
        )
        .unwrap(),
        0
    );
    assert!(parse_probe_counter("SDK 已启动").is_err());
}
