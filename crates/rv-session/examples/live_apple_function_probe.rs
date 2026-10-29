//! Interactive real-device Apple function probe through the production RV actor.
//! JSON commands arrive on private stdin; passwords, URLs, codes and JWTs are never printed.
//! Uses the saved device credential and local lock-screen configuration unchanged.
use rv_core::{AddressBook, ConnectRequest, StorePaths};
use rv_session::{FunctionCommand, FunctionEvent, SessionEvent, SessionHandle};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::BufRead,
    time::{Duration, Instant},
};

fn domain_summary(state: &Value, target: &str) -> Value {
    let mut result = serde_json::Map::new();
    for domain in ["app_store", "icloud", "identity"] {
        let section = &state[domain];
        let account = &section["account"];
        result.insert(domain.into(), json!({
            "status":section["status"],
            "has_account":account.is_object(),
            "matches_target":account["apple_id"].as_str().map(|id| id.eq_ignore_ascii_case(target)),
            "authenticated":account["authenticated"],
            "enabled_dataclasses":account["enabled_dataclasses"],
            "find_my_enabled":section["find_my_enabled"]
        }));
    }
    Value::Object(result)
}

fn emit(
    event: FunctionEvent,
    scope: Option<String>,
    target: &str,
    mailbox: Option<&str>,
    name: Option<&(String, String)>,
    receipts: &mut HashMap<String, (u8, String)>,
) {
    match event {
        FunctionEvent::Registration(_) => {}
        FunctionEvent::ForOperation { id, event } => {
            emit(*event, Some(id), target, mailbox, name, receipts)
        }
        FunctionEvent::AccountStateLoading => println!("{}", json!({"event":"account_loading"})),
        FunctionEvent::AccountState(Ok(state)) => println!(
            "{}",
            json!({"event":"account_state","domains":domain_summary(&state,target)})
        ),
        FunctionEvent::AccountState(Err(error)) => {
            println!("{}", json!({"event":"account_error","message":error}))
        }
        FunctionEvent::State { task, record } => {
            if let (Some(scope), Some(operation)) =
                (scope.as_ref(), record["operation_id"].as_str())
            {
                receipts.insert(scope.clone(), (task, operation.to_owned()));
            }
            let result = &record["result"];
            let mut summary = json!({
                "event":"operation", "scope":scope, "method":record["method"],
                "status":record["status"], "outcome":result["outcome"],
                "error_domain":record["error"]["domain"], "error_code":record["error"]["code"],
                "error_message":record["error"]["message"],
                "challenge_present":record["challenge"]["challenge_id"].is_string(),
                "result_keys":result.as_object().map(|object| object.keys().cloned().collect::<Vec<_>>()),
                "matches_target":result["apple_id"].as_str().map(|id| id.eq_ignore_ascii_case(target)),
                "completed_domains":result["completed_domains"],
                "name_fields_present": (["first_name","last_name"].map(|key| result[key].as_str().is_some())),
                "contact_email_count":result["reachable_emails"].as_array().map(Vec::len),
                "forwarding_email_present":result["forwarding_email"].as_str().is_some(),
                "app_count":result["apps"].as_array().map(Vec::len),
                "device_count":result["devices"].as_array().map(Vec::len),
                "selected_mailbox_verified":mailbox.and_then(|email| result["reachable_emails"].as_array().map(|emails| emails.iter().any(|item| item.as_str().is_some_and(|value| value.eq_ignore_ascii_case(email))))),
                "selected_mailbox_forwarding":mailbox.and_then(|email| result["forwarding_email"].as_str().map(|value| value.eq_ignore_ascii_case(email))),
                "name_matches_test":name.map(|(first,last)| result["first_name"] == first.as_str() && result["last_name"] == last.as_str()),
            });
            if record["method"] == "queryAccountState" {
                summary["domains"] = domain_summary(result, target);
            }
            if let Some(apps) = result["apps"].as_array() {
                summary["apps"] = json!(
                    apps.iter()
                        .map(|app| json!({"name":app["name"],"bundle_id":app["bundle_id"]}))
                        .collect::<Vec<_>>()
                );
            }
            if let Some(devices) = result["devices"].as_array() {
                summary["devices"] = json!(devices.iter().map(|device| json!({"name":device["name"],"model":device["model"],"current_device":device["current_device"],"find_my_enabled":device["find_my_enabled"]})).collect::<Vec<_>>());
            }
            if record["method"] == "queryState" {
                summary["section_statuses"] = json!(result.as_object().map(|object| {
                    object
                        .iter()
                        .map(|(key, value)| (key.clone(), value["status"].clone()))
                        .collect::<serde_json::Map<_, _>>()
                }));
            }
            println!("{summary}");
        }
        FunctionEvent::Available(value) => println!(
            "{}",
            json!({"event":"available","scope":scope,"value":value})
        ),
        FunctionEvent::Progress(message) => println!(
            "{}",
            json!({"event":"progress","scope":scope,"message":message})
        ),
        FunctionEvent::Error(message) => println!(
            "{}",
            json!({"event":"error","scope":scope,"message":message})
        ),
        FunctionEvent::Uncertain(message) => println!(
            "{}",
            json!({"event":"uncertain","scope":scope,"message":message})
        ),
        FunctionEvent::CodeFetchWaiting { remaining_seconds } => println!(
            "{}",
            json!({"event":"code_waiting","scope":scope,"remaining_seconds":remaining_seconds})
        ),
        FunctionEvent::VerificationCodeReceived(_) => println!(
            "{}",
            json!({"event":"code_received_and_submitted","scope":scope})
        ),
        FunctionEvent::CodeFetched(_) | FunctionEvent::EmailCodeFetched(_) => {
            println!("{}", json!({"event":"code_fetched","scope":scope}))
        }
        FunctionEvent::EmailCodePending(_) => {
            println!("{}", json!({"event":"email_code_pending","scope":scope}))
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::args()
        .nth(1)
        .ok_or("usage: live_apple_function_probe HOST")?;
    let paths = StorePaths::default_dir()?;
    let book = AddressBook::load(paths.clone())?;
    let connection = book
        .connections()
        .iter()
        .find(|connection| connection.host == host)
        .ok_or("phone is not saved in RV")?;
    let mut request =
        ConnectRequest::from_connection(connection, paths.load_password(connection.id)?);
    request.zxtouch_port = Some(6000);
    request.view_only = false;
    let session = SessionHandle::spawn_control(request.clone());
    let (sender, commands) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                return;
            }
        }
        let _ = sender.send("{\"action\":\"close\"}".into());
    });
    let mut target = String::new();
    let mut receipts = HashMap::new();
    let mut mailbox_row: Option<String> = None;
    let mut mailbox_email: Option<String> = None;
    let mut expected_name: Option<(String, String)> = None;
    let deadline = Instant::now() + Duration::from_secs(3600);
    println!(
        "{}",
        json!({"event":"ready","host":host,"port":6000,"private_stdin":true})
    );
    while Instant::now() < deadline {
        while let Ok(line) = commands.try_recv() {
            let Ok(command) = serde_json::from_str::<Value>(&line) else {
                println!("{}", json!({"event":"invalid_command"}));
                continue;
            };
            match command["action"].as_str().unwrap_or_default() {
                "close" => {
                    session.close();
                    return Ok(());
                }
                "target" => {
                    target = command["apple_id"].as_str().unwrap_or_default().to_owned();
                }
                "refresh" => session.function(FunctionCommand::RefreshAccountState),
                "mailbox-load" => {
                    let content = std::fs::read_to_string(
                        command["path"].as_str().ok_or("missing mailbox path")?,
                    )?;
                    let selected = content
                        .lines()
                        .filter_map(|line| {
                            let fields: Vec<_> = line.trim().split("----").collect();
                            (fields.len() == 4
                                && fields[0].contains('@')
                                && fields[2].len() == 36
                                && fields[2].matches('-').count() == 4)
                                .then(|| (line.trim().to_owned(), fields[0].trim().to_owned()))
                        })
                        .nth(command["index"].as_u64().unwrap_or(0) as usize);
                    if let Some((row, email)) = selected {
                        mailbox_row = Some(row);
                        mailbox_email = Some(email);
                        println!(
                            "{}",
                            json!({"event":"mailbox_loaded","index":command["index"].as_u64().unwrap_or(0)})
                        );
                    } else {
                        println!("{}", json!({"event":"mailbox_not_found"}));
                    }
                }
                "mailbox-add" | "mailbox-forward" => {
                    let forwarding = command["action"] == "mailbox-forward";
                    let text = if forwarding {
                        mailbox_email.as_ref().map(|email| format!("邮箱={email}"))
                    } else {
                        mailbox_row.clone()
                    };
                    if let Some(text) = text {
                        let id = if forwarding { "forward" } else { "email-add" };
                        session.function(FunctionCommand::Scoped {
                            id: id.into(),
                            command: Box::new(FunctionCommand::InvokeForAccount {
                                id: id.into(),
                                text,
                                apple_id: target.clone(),
                            }),
                        });
                        println!("{}", json!({"event":"sent","scope":id}));
                    }
                }
                "test-name" => {
                    let seed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .subsec_nanos() as usize;
                    let first = ["Mira", "Nora", "Elena", "Lena"][seed % 4].to_owned();
                    let last = ["Vale", "Hart", "Reed", "Lane"][(seed / 4) % 4].to_owned();
                    session.function(FunctionCommand::Scoped {
                        id: "name".into(),
                        command: Box::new(FunctionCommand::InvokeForAccount {
                            id: "name".into(),
                            text: format!("名={first}\n姓={last}"),
                            apple_id: target.clone(),
                        }),
                    });
                    println!(
                        "{}",
                        json!({"event":"test_name","first_name":first,"last_name":last})
                    );
                    expected_name = Some((first, last));
                }
                "resume" => {
                    let id = command["id"].as_str().unwrap_or_default();
                    if let Some((task, operation_id)) = receipts.get(id).cloned() {
                        session.function(FunctionCommand::Scoped {
                            id: id.into(),
                            command: Box::new(FunctionCommand::Resume { task, operation_id }),
                        });
                    }
                }
                "invoke" => {
                    let id = command["id"].as_str().unwrap_or_default();
                    let mut text = command["text"].as_str().unwrap_or_default().to_owned();
                    if id == "cloud-login" {
                        let Some(code) = rv_core::load_unlock_code(&request)? else {
                            println!("{}", json!({"event":"missing_saved_unlock_code"}));
                            continue;
                        };
                        text.push_str("\n设备密码=");
                        text.push_str(std::str::from_utf8(code.digits())?);
                    }
                    let cmd = if let Some(apple_id) = command["expected_account"].as_str() {
                        FunctionCommand::InvokeForAccount {
                            id: id.into(),
                            text,
                            apple_id: apple_id.into(),
                        }
                    } else {
                        FunctionCommand::Invoke {
                            id: id.into(),
                            text,
                        }
                    };
                    session.function(FunctionCommand::Scoped {
                        id: id.into(),
                        command: Box::new(cmd),
                    });
                    println!("{}", json!({"event":"sent","scope":id}));
                }
                _ => println!("{}", json!({"event":"invalid_command"})),
            }
        }
        for event in session.drain() {
            match event {
                SessionEvent::ControlState(value) => {
                    println!("{}", json!({"event":"control_authenticated","value":value}))
                }
                SessionEvent::Functions(event) => emit(
                    event,
                    None,
                    &target,
                    mailbox_email.as_deref(),
                    expected_name.as_ref(),
                    &mut receipts,
                ),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    session.close();
    Ok(())
}
