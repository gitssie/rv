//! RV-local registration coordinator. Uses the current authenticated phone client.
use super::FunctionEvent;
use crate::{
    automation::{
        ZxTouchDevice,
        vinted::{self, Field, Preferences},
    },
    phone_http::PhoneHttpClient,
};
use rv_automation::{Device, InputValues, RunControl, RunState, Runner, Workflow};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub running: bool,
    /// Static, sanitized UI text; never contains input, OCR or response bodies.
    pub message: &'static str,
}

/// Deliberately not Debug/Serialize. Credentials live only for this execution.
pub struct Request {
    inputs: InputValues,
    pub preferences: Preferences,
    code_url: Option<Zeroizing<String>>,
}
impl Request {
    pub fn parse(text: &str, marketing: bool, accept_terms: bool) -> Result<Self, &'static str> {
        if text.len() > 8192 {
            return Err("资料过长");
        }
        if !accept_terms {
            return Err("请先接受用户协议");
        }
        let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
        let first = lines.next().ok_or("请粘贴邮箱、用户名和密码")?;
        let fields: Vec<_> = if first.contains("----") {
            first.split("----").map(str::trim).collect()
        } else {
            first.split_whitespace().collect()
        };
        if fields.len() != 3
            || fields.iter().any(|v| {
                v.is_empty() || v.encode_utf16().count() > 128 || v.chars().any(char::is_control)
            })
        {
            return Err("首行格式：邮箱 用户名 密码（也支持 ---- 分隔）");
        }
        if !fields[0].contains('@') {
            return Err("邮箱格式无效");
        }
        let code_url = lines.next().map(str::to_owned);
        if lines.next().is_some() {
            return Err("资料只支持一组账号，取码链接放在第二行");
        }
        if let Some(url) = &code_url {
            super::validate_code_url(url).map_err(|_| "取码链接无效")?;
        }
        let mut inputs = InputValues::default();
        for (key, value) in ["email", "username", "password"].into_iter().zip(fields) {
            inputs.insert(key, value.to_owned());
        }
        Ok(Self {
            inputs,
            preferences: Preferences {
                marketing,
                accept_terms,
            },
            code_url: code_url.map(Zeroizing::new),
        })
    }
}

#[derive(Default)]
pub(super) struct Runtime {
    task: Option<tokio::task::JoinHandle<()>>,
    control: RunControl,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.control.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl Runtime {
    pub fn active(&self) -> bool {
        self.task.as_ref().is_some_and(|task| !task.is_finished())
    }
    pub fn cancel(&self) {
        self.control.cancel();
    }
    pub fn start(
        &mut self,
        client: PhoneHttpClient,
        text: String,
        marketing: bool,
        accept_terms: bool,
        events: mpsc::Sender<FunctionEvent>,
        send: &impl Fn(FunctionEvent),
    ) {
        if self.active() {
            return;
        }
        let text = Zeroizing::new(text);
        let request = match Request::parse(&text, marketing, accept_terms) {
            Ok(request) => request,
            Err(message) => {
                send(FunctionEvent::Registration(Status {
                    running: false,
                    message,
                }));
                return;
            }
        };
        self.control = RunControl::default();
        let control = self.control.clone();
        self.task = Some(tokio::spawn(async move {
            // Cancellation interrupts observation as well as the runner. No replay.
            let result = tokio::select! {
                biased;
                _ = control.cancelled() => Err("已停止；已发送的输入可能生效，请先检查手机再执行"),
                result = tokio::time::timeout(std::time::Duration::from_secs(180), run(&client, &request, &control, &events)) => result.unwrap_or(Err("注册超时；请检查手机，勿直接重复执行")),
            };
            let message = match result {
                Ok(message) => message,
                Err(message) => message,
            };
            let _ = events
                .send(FunctionEvent::Registration(Status {
                    running: false,
                    message,
                }))
                .await;
        }));
    }
}

async fn progress(events: &mpsc::Sender<FunctionEvent>, message: &'static str) {
    let _ = events
        .send(FunctionEvent::Registration(Status {
            running: true,
            message,
        }))
        .await;
}
async fn execute(
    client: &PhoneHttpClient,
    workflow: Workflow,
    request: &Request,
    control: &RunControl,
) -> Result<(), &'static str> {
    let result = Runner::new(ZxTouchDevice::new(client.clone()))
        .run(&workflow, &request.inputs, control)
        .await;
    if result.state == RunState::Succeeded {
        Ok(())
    } else {
        Err("步骤未完成；请检查手机，勿直接重复执行")
    }
}
async fn run(
    client: &PhoneHttpClient,
    request: &Request,
    control: &RunControl,
    events: &mpsc::Sender<FunctionEvent>,
) -> Result<&'static str, &'static str> {
    progress(events, "正在识别注册页面…").await;
    let mut device = ZxTouchDevice::new(client.clone());
    let observed = device.observe().await.map_err(|_| "无法读取手机页面")?;
    let has = |text: &str| {
        observed
            .elements
            .iter()
            .any(|element| element.text.trim() == text)
    };
    if observed.bundle_id.as_deref() != Some(vinted::BUNDLE_ID) {
        return Err("请先在手机打开 Vinted 注册页面");
    }
    if has("S'inscrire sur Vinted") {
        execute(client, vinted::open_registration(), request, control).await?;
        execute(client, vinted::choose_email(), request, control).await?;
    } else if has("Continuer avec une adresse e-mail") {
        execute(client, vinted::choose_email(), request, control).await?;
    } else if ![Field::Username, Field::Email, Field::Password]
        .iter()
        .all(|field| has(field.label()))
    {
        return Err("当前不是空白注册页面；请检查手机后再执行");
    }
    // Recheck all placeholders after navigation, so reruns never append to an existing form.
    let empty = device.observe().await.map_err(|_| "无法确认空白注册页面")?;
    if ![Field::Username, Field::Email, Field::Password]
        .iter()
        .all(|field| {
            empty
                .elements
                .iter()
                .any(|e| e.text.trim() == field.label())
        })
    {
        return Err("注册表单已有内容或无法确认，请先检查手机");
    }
    for (field, message) in [
        (Field::Username, "正在填写用户名…"),
        (Field::Email, "正在填写邮箱…"),
        (Field::Password, "正在填写密码…"),
    ] {
        progress(events, message).await;
        vinted::fill_field(client, field, &request.inputs, control)
            .await
            .map_err(|_| "填写结果待确认；请检查手机，勿直接重复执行")?;
    }
    let observed = device
        .observe()
        .await
        .map_err(|_| "无法确认密码输入，请检查手机")?;
    let mut password = vinted::confirm_password(&observed, &request.inputs).ok();
    if let Some(decline) =
        vinted::decline_password_save(&observed).map_err(|_| "请在手机处理密码提示")?
    {
        execute(client, decline, request, control).await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    progress(events, "正在核对订阅和协议…").await;
    let (_, checks) = vinted::inspect_phone(client)
        .await
        .map_err(|_| "无法确认协议状态，请检查手机")?;
    if let Some(changes) = vinted::consent_changes(checks, request.preferences) {
        execute(client, changes, request, control).await?;
    }
    // Blurring the password during consent may trigger iOS's save-password sheet.
    let observed = device
        .observe()
        .await
        .map_err(|_| "无法确认密码输入，请检查手机")?;
    if password.is_none() {
        password = vinted::confirm_password(&observed, &request.inputs).ok();
    }
    if let Some(decline) =
        vinted::decline_password_save(&observed).map_err(|_| "请在手机处理密码提示")?
    {
        execute(client, decline, request, control).await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let password = password.ok_or("无法确认密码已输入；请在手机检查后继续")?;
    let (observed, checks) = vinted::inspect_phone(client)
        .await
        .map_err(|_| "无法复核协议状态，请检查手机")?;
    let submit = vinted::submit(
        &observed,
        checks,
        request.preferences,
        &request.inputs,
        &password,
    )
    .map_err(|_| "提交条件未满足，请检查手机")?;
    progress(events, "正在提交注册…").await;
    execute(client, submit, request, control).await?;
    finish_verification(client, request, control, events).await
}

async fn finish_verification(
    client: &PhoneHttpClient,
    request: &Request,
    control: &RunControl,
    events: &mpsc::Sender<FunctionEvent>,
) -> Result<&'static str, &'static str> {
    use vinted::verification::{self, Page};
    let mut device = ZxTouchDevice::new(client.clone());
    let email = request.inputs.get("email").ok_or("缺少邮箱")?;
    let mut submitted_code = false;
    let mut fetched_code: Option<Zeroizing<String>> = None;
    let mut declined_save = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    progress(events, "正在确认邮箱验证页面…").await;
    while tokio::time::Instant::now() < deadline {
        let observed = device
            .observe()
            .await
            .map_err(|_| "无法读取验证页面，请检查手机")?;
        if let Some(decline) =
            vinted::decline_password_save(&observed).map_err(|_| "请在手机处理密码提示")?
        {
            if declined_save {
                return Err("密码提示仍未关闭，请检查手机");
            }
            execute(client, decline, request, control).await?;
            declined_save = true;
        } else {
            match verification::classify(&observed, email) {
                Page::PhoneRequired => {
                    return Ok(if submitted_code {
                        "邮箱验证已通过；需要添加手机号，请在手机继续"
                    } else {
                        "当前需要添加手机号；请在手机继续"
                    });
                }
                Page::Email if !submitted_code => {
                    let Some(url) = request.code_url.as_deref() else {
                        return Ok("已进入邮箱验证页；未提供取码链接，请在手机继续");
                    };
                    progress(events, "正在读取邮箱验证码…").await;
                    if fetched_code.is_none() {
                        fetched_code = super::fetch_code(url).await.ok().map(Zeroizing::new);
                    }
                    if let Some(code) = &fetched_code {
                        // Re-observe after network I/O; never use stale field geometry.
                        let current = device.observe().await.map_err(|_| "无法复核验证码页面")?;
                        let workflow = verification::enter_code(&current, email, code)
                            .map_err(|_| "验证码页面已变化，请检查手机")?;
                        let mut inputs = InputValues::default();
                        inputs.insert("verification_code", code.to_string());
                        progress(events, "正在填写并验证邮箱验证码…").await;
                        let result = Runner::new(ZxTouchDevice::new(client.clone()))
                            .run(&workflow, &inputs, control)
                            .await;
                        if result.state != RunState::Succeeded {
                            return Err("验证码操作结果待确认；请检查手机，勿重复提交");
                        }
                        submitted_code = true;
                    }
                }
                _ => {}
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Err("验证结果待确认；请检查手机，勿重复提交")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unified_input_supports_both_delimiters_and_never_debugs_secrets() {
        for text in [
            "a@example.test user secret\nhttps://example.test/code?token=private",
            "a@example.test----user----secret",
        ] {
            let request = Request::parse(text, false, true).unwrap();
            assert_eq!(request.inputs.get("username"), Some("user"));
            assert!(!request.preferences.marketing);
        }
        assert!(Request::parse("a@example.test user secret", false, false).is_err());
        assert!(Request::parse("a@example.test user secret extra", false, true).is_err());
        assert!(Request::parse("a@example.test user secret\nfile:///secret", false, true).is_err());
        assert!(
            Request::parse(
                "a@example.test user secret\nhttps://example.test\nother",
                false,
                true
            )
            .is_err()
        );
    }

    #[test]
    fn unavailable_registration_returns_its_own_terminal_status() {
        let mut runtime = super::super::FunctionRuntime::new("unused".into(), None, true);
        let received = std::cell::RefCell::new(Vec::new());
        runtime.command(
            super::super::FunctionCommand::Register {
                text: "a@example.test user secret".into(),
                marketing: false,
                accept_terms: true,
            },
            &|event| received.borrow_mut().push(event),
        );
        assert!(matches!(
            received.borrow().as_slice(),
            [FunctionEvent::Registration(Status { running: false, .. })]
        ));
    }

    #[tokio::test]
    async fn immediate_stop_prevents_io_and_does_not_queue_a_second_registration() {
        let client = PhoneHttpClient::new("127.0.0.1", 1, "fixture".into()).unwrap();
        let (events, mut rx) = mpsc::channel(8);
        let mut runtime = Runtime::default();
        let unexpected = |_| panic!("duplicate execution must not change the current status");
        runtime.start(
            client.clone(),
            "a@example.test user secret".into(),
            false,
            true,
            events.clone(),
            &unexpected,
        );
        assert!(runtime.active());
        runtime.start(client, "invalid".into(), false, true, events, &unexpected);
        runtime.cancel();
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(event, FunctionEvent::Registration(Status { running: false, message }) if message.starts_with("已停止"))
        );
        runtime.task.take().unwrap().await.unwrap();
        assert!(!runtime.active());
        assert!(rx.try_recv().is_err());
    }
}
