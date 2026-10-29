//! Development entry point for explicit phases; no RV application build needed.
//! JSON arrives on stdin, never command-line arguments or a credential file.
use rv_automation::{InputValues, RunControl, RunState, Runner};
use rv_session::{
    automation::{
        ZxTouchDevice,
        vinted::{self, Field, Preferences},
    },
    phone_http::PhoneHttpClient,
};
use serde::Deserialize;
use std::io::Read;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    host: String,
    port: u16,
    token: String,
    phase: String,
    username: String,
    email: String,
    password: String,
    marketing: bool,
    accept_terms: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = Zeroizing::new(String::new());
    std::io::stdin().take(64 * 1024).read_to_string(&mut raw)?;
    let request: Request = serde_json::from_str(&raw)?;
    drop(raw);
    let client = PhoneHttpClient::new(&request.host, request.port, request.token)?;
    let mut inputs = InputValues::default();
    inputs.insert("username", request.username);
    inputs.insert("email", request.email);
    inputs.insert("password", request.password);
    let control = RunControl::default();
    let preferences = Preferences {
        marketing: request.marketing,
        accept_terms: request.accept_terms,
    };
    let mut runner = Runner::new(ZxTouchDevice::new(client.clone()));
    let workflow = match request.phase.as_str() {
        "open" => Some(vinted::open_registration()),
        "email-choice" => Some(vinted::choose_email()),
        "username" | "email" | "password" => {
            let field = match request.phase.as_str() {
                "username" => Field::Username,
                "email" => Field::Email,
                _ => Field::Password,
            };
            let result = vinted::fill_field(&client, field, &inputs, &control).await?;
            println!(
                "phase={} state={:?}; this is not account-creation evidence",
                request.phase, result
            );
            return Ok(());
        }
        "consent" => {
            let (_, checked) = vinted::inspect_phone(&client).await?;
            vinted::consent_changes(checked, preferences)
        }
        "submit" => {
            use rv_automation::Device;
            let mut device = ZxTouchDevice::new(client.clone());
            let observed = device.observe().await?;
            let password = vinted::confirm_password(&observed, &inputs)?;
            if let Some(decline) = vinted::decline_password_save(&observed)? {
                let result = runner.run(&decline, &inputs, &control).await;
                if result.state != RunState::Succeeded {
                    return Err("password-save prompt remains; no submit sent".into());
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            let (observed, checked) = vinted::inspect_phone(&client).await?;
            Some(vinted::submit(
                &observed,
                checked,
                preferences,
                &inputs,
                &password,
            )?)
        }
        _ => return Err("unknown explicit phase".into()),
    };
    if let Some(workflow) = workflow {
        let result = runner.run(&workflow, &inputs, &control).await;
        println!("phase={} state={:?}", request.phase, result.state);
        if result.state != RunState::Succeeded {
            return Err("phase stopped; inspect device before repeating".into());
        }
    }
    if request.phase == "consent" {
        let (_, checked) = vinted::inspect_phone(&client).await?;
        println!("marketing={} terms={}", checked.marketing, checked.terms);
        if checked.marketing != preferences.marketing || checked.terms != preferences.accept_terms {
            return Err("consent state not confirmed; do not blindly repeat".into());
        }
    }
    Ok(())
}
