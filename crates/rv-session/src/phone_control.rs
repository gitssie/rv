//! ZXTouch controller lifetime belongs to the saved device, never to its VNC window.
use crate::{AppCommand, SessionCommand, SessionEvent};
use rv_core::ConnectRequest;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedReceiver;

pub(crate) async fn run(
    request: ConnectRequest,
    mut commands: UnboundedReceiver<SessionCommand>,
    events: mpsc::Sender<SessionEvent>,
) {
    let send = |event| {
        let _ = events.send(event);
    };
    let app_send = |event| send(SessionEvent::Apps(event));
    let function_send = |event| send(SessionEvent::Functions(event));
    let port = request.zxtouch_port.unwrap_or(6000);
    let Ok(client) = crate::phone_http::PhoneHttpClient::new(&request.host, port, String::new())
    else {
        send(SessionEvent::Status("无法初始化 ZXTouch 控制客户端".into()));
        return;
    };
    let mut apps = crate::apps::AppRuntime::new(!request.view_only);
    let mut functions = crate::functions::FunctionRuntime::new(
        request.host.clone(),
        Some(port),
        !request.view_only,
    );
    let mut configured = false;
    let mut healthy = false;
    let mut online = false;
    let mut next_probe = Instant::now();
    let mut renew_at = Instant::now();
    let mut probe = None;
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                None | Some(SessionCommand::Close) => break,
                Some(SessionCommand::App(command)) => {
                    if online { apps.command(command, &app_send); }
                    else { app_send(crate::AppEvent::Failed { command, message: "ZXTouch 尚未连接，请稍后重试".into() }); }
                },
                Some(SessionCommand::Function(command)) => {
                    if online { functions.command(command, &function_send); }
                    else { function_send(crate::FunctionEvent::Error("ZXTouch 尚未连接，请稍后重试".into())); }
                }
                // Desktop input and file transfers never belong to this controller.
                _ => {}
            },
            _ = tick.tick() => {}
        }
        if probe
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
        {
            let task: tokio::task::JoinHandle<Result<Option<crate::AppCapabilities>, String>> =
                probe.take().unwrap();
            let result = task
                .await
                .unwrap_or_else(|_| Err("ZXTouch 连接检查已停止".into()));
            match result {
                Ok(caps) => {
                    healthy = true;
                    if configured {
                        apps.command(AppCommand::Foreground, &app_send);
                    }
                    if let Some(caps) = caps {
                        renew_at = Instant::now() + Duration::from_secs(3300);
                        if !configured {
                            functions.set_http_client(Some(client.clone()));
                            functions.configure(&caps);
                            apps.configure_http(caps, client.clone(), &app_send);
                            configured = true;
                        }
                    }
                }
                Err(message) => {
                    healthy = false;
                    send(SessionEvent::Status(message));
                }
            }
            next_probe = Instant::now() + Duration::from_secs(5);
        }
        if probe.is_none() && Instant::now() >= next_probe {
            let client = client.clone();
            let password = request.password.clone().unwrap_or_default();
            let renew = !configured || Instant::now() >= renew_at;
            probe = Some(tokio::spawn(async move {
                if !renew {
                    if let Ok((200, _)) = client
                        .json("/api/control/authorize", &serde_json::json!({}))
                        .await
                    {
                        return Ok(None);
                    }
                }
                client.connect(&password).await.map(Some)
            }));
        }
        apps.advance_http(&app_send).await;
        functions.advance(&function_send);
        let ready = healthy && apps.http_ready();
        if ready != online {
            online = ready;
            send(SessionEvent::ControlState(online));
            if online {
                for command in [AppCommand::DeviceInfo, AppCommand::Foreground] {
                    apps.command(command, &app_send);
                }
            }
        }
    }
    if let Some(task) = probe {
        task.abort();
    }
    // Explicit controller shutdown revokes only its own token; VNC is unaffected.
    if configured {
        let _ = client
            .json("/api/control/disconnect", &serde_json::json!({}))
            .await;
    }
}

#[cfg(test)]
#[path = "phone_control_tests.rs"]
mod tests;
