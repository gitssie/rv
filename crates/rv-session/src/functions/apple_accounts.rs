//! Persists Apple account credentials and coordinates first-login password rotation.
//! The existing native worker still owns all HTTP/2 requests/challenges and never receives a code URL.
use super::*;
use rv_core::{
    StorePaths,
    workspace::{
        AppleAccount, AppleAccountStatus, AppleInitStatus, AppleLoginFailure, WorkspaceStore,
    },
};
#[derive(Clone)]
pub(super) struct Context {
    paths: Option<StorePaths>,
    device_id: Option<String>,
    unlock_request: Option<rv_core::ConnectRequest>,
}
impl Context {
    #[cfg(test)]
    pub(super) fn at(paths: StorePaths, device_id: Option<String>) -> Self {
        Self {
            paths: Some(paths),
            device_id,
            unlock_request: None,
        }
    }
    pub(super) fn system(request: &rv_core::ConnectRequest) -> Self {
        Self {
            paths: StorePaths::default_dir().ok(),
            device_id: request.connection_id.map(|v| v.to_string()),
            unlock_request: Some(request.clone()),
        }
    }
    async fn db<T: Send + 'static>(
        &self,
        action: impl FnOnce(&mut WorkspaceStore) -> Result<T, rv_core::StoreError> + Send + 'static,
    ) -> Result<T, String> {
        let paths = self.paths.clone().ok_or("无法读取 Apple ID 数据库目录")?;
        tokio::task::spawn_blocking(move || {
            let mut store = WorkspaceStore::open(&paths)?;
            action(&mut store)
        })
        .await
        .map_err(|_| "Apple ID 数据库操作中断".to_owned())?
        .map_err(|_| "Apple ID 记录保存失败，请检查数据库或重新读取记录".into())
    }
    async fn unlock_code(&self) -> Result<Option<String>, String> {
        let Some(request) = self.unlock_request.clone() else {
            return Ok(None);
        };
        let paths = self.paths.clone().ok_or("无法读取本地锁屏配置")?;
        tokio::task::spawn_blocking(move || {
            paths
                .load_unlock_code(&request)
                .map(|code| code.map(|v| String::from_utf8(v.digits().to_vec()).unwrap()))
        })
        .await
        .map_err(|_| "本地锁屏配置读取中断".to_owned())?
        .map_err(|_| "读取本地锁屏配置失败".into())
    }
    pub(super) async fn load(&self, id: &str) -> Result<Option<AppleAccount>, String> {
        let id = id.to_owned();
        self.db(move |s| s.apple_account(&id)).await
    }
    async fn prepare(&self, text: &str) -> Result<AppleAccount, String> {
        let spec = OPERATIONS.iter().find(|s| s.id == "store-login").unwrap();
        let parsed = parse_operation(spec, text)?;
        let id = parsed.params["apple_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let password = parsed.params["password"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let url = parsed.code_url;
        let device = self.device_id.clone();
        let a = self
            .db(move |s| {
                s.prepare_apple_login(
                    &id,
                    &password,
                    url.as_deref(),
                    device.as_deref(),
                    &json!({}),
                )
            })
            .await?
            .account;
        if matches!(
            a.init_status,
            AppleInitStatus::ChangingPassword | AppleInitStatus::Unknown
        ) {
            return Err(
                "上次改密结果尚未确认，已保留原密码和待用新密码；请先查询原操作结果".into(),
            );
        }
        Ok(a)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Login,
    ChangePassword,
    LoginNewPassword,
}
struct Managed {
    account: AppleAccount,
    phase: Phase,
    current_operation: Option<String>,
    manual: bool,
}
fn input_text(account: &AppleAccount, new_password: bool) -> String {
    let mut lines = vec![
        format!("apple_id={}", account.apple_id),
        format!("password={}", account.password),
    ];
    if new_password {
        lines.push(format!(
            "new_password={}",
            account.pending_password.as_deref().unwrap_or_default()
        ));
    }
    if let Some(url) = &account.verification_code_url {
        lines.push(format!("code_url={url}"));
    }
    lines.join("\n")
}
fn login_command(account: &AppleAccount) -> FunctionCommand {
    FunctionCommand::Invoke {
        id: "store-login".into(),
        text: input_text(account, false),
    }
}
fn failure(record: &Value) -> AppleLoginFailure {
    let reason = record["error"]["reason"].as_str();
    AppleLoginFailure {
        domain: record["error"]["domain"].as_str().map(str::to_owned),
        code: record["error"].get("code").map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
        }),
        message: match reason {
            Some("account_locked") => "Apple 明确返回账号已锁定",
            Some("account_disabled") => "Apple 明确返回账号已停用",
            _ => "Apple 登录或改密未成功，请查看原操作结果",
        }
        .into(),
        account_status: match reason {
            Some("account_locked") => Some(AppleAccountStatus::Locked),
            Some("account_disabled") => Some(AppleAccountStatus::Disabled),
            _ => None,
        },
    }
}
impl Managed {
    async fn failed(
        &mut self,
        ctx: &Context,
        record: &Value,
        uncertain: bool,
    ) -> Result<(), String> {
        let expected = self.account.clone();
        let fail = failure(record);
        self.account = ctx
            .db(move |s| s.apple_login_failed(&expected, &fail, uncertain))
            .await?;
        Ok(())
    }
    async fn receipt(&mut self, ctx: &Context, record: &Value) -> Result<(), String> {
        if let Some(id) = record["operation_id"].as_str() {
            self.current_operation = Some(id.to_owned());
        }
        if self.phase == Phase::ChangePassword
            && self.account.password_change_operation_id.is_none()
        {
            if let Some(id) = record["operation_id"].as_str() {
                let expected = self.account.clone();
                let id = id.to_owned();
                self.account = ctx
                    .db(move |s| s.record_apple_password_operation(&expected, &id))
                    .await?;
            }
        }
        Ok(())
    }
    async fn completed(
        &mut self,
        ctx: &Context,
        record: &Value,
    ) -> Result<Option<FunctionCommand>, String> {
        let succeeded = record.get("error").is_none() && record["result"]["outcome"] == "succeeded";
        if self.phase == Phase::ChangePassword && record["result"]["password_changed"] == true {
            // A confirmed remote commit must update the current password even when a
            // later native step reports an error. Keep the original password unchanged.
            let expected = self.account.clone();
            self.account = ctx
                .db(move |s| s.apple_password_change_succeeded(&expected))
                .await?;
            if !succeeded {
                self.failed(ctx, record, record["result"]["outcome"] != "failed")
                    .await?;
                return Ok(None);
            }
            if self.manual {
                return Ok(None);
            }
            self.phase = Phase::LoginNewPassword;
            return Ok(Some(login_command(&self.account)));
        }
        if !succeeded {
            let uncertain = record["result"]["outcome"] != "failed";
            self.failed(ctx, record, uncertain).await?;
            return Ok(None);
        }
        if self.phase == Phase::ChangePassword {
            self.failed(ctx, &json!({}), true).await?;
            return Err("iOS 未确认密码修改成功；新旧密码已保留，请查询原操作结果".into());
        }
        if record["result"]["authenticated"] != true {
            self.failed(ctx, &json!({}), true).await?;
            return Err("iOS 未返回已认证的登录结果".into());
        }
        let expected = self.account.clone();
        self.account = ctx.db(move |s| s.apple_login_succeeded(&expected)).await?;
        if self.phase == Phase::Login
            && (self.account.init_status == AppleInitStatus::Pending || self.manual)
        {
            let passcode = ctx.unlock_code().await?;
            let expected = self.account.clone();
            self.account = ctx
                .db(move |s| s.begin_apple_password_change(&expected))
                .await?;
            self.phase = Phase::ChangePassword;
            let mut input = input_text(&self.account, true);
            if let Some(code) = passcode {
                input.push_str(&format!("\ndevice_passcode={code}"));
            }
            return Ok(Some(FunctionCommand::Invoke {
                id: "apple-password-change-internal".into(),
                text: input,
            }));
        }
        Ok(None)
    }
}
struct Driver(tokio::task::JoinHandle<()>);
impl Drop for Driver {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub(super) async fn run(
    client: crate::phone_http::PhoneHttpClient,
    mut commands: mpsc::Receiver<FunctionCommand>,
    events: mpsc::Sender<FunctionEvent>,
    ctx: Context,
) {
    let (tx, rx) = mpsc::channel(8);
    let (out, mut input) = mpsc::channel(32);
    let driver_ctx = ctx.clone();
    let driver_client = client.clone();
    let _driver = Driver(tokio::spawn(async move {
        super::worker_with_context(driver_client, rx, out, &mailbox::Router, Some(driver_ctx))
            .await;
    }));
    let mut managed: Option<Managed> = None;
    loop {
        tokio::select! {
         command=commands.recv()=>{
          let Some(command)=command else{if let Some(m)=managed.as_mut(){let _=m.failed(&ctx,&json!({}),true).await;}return;};
          let (command,expected_account)=match command {
           FunctionCommand::InvokeForAccount{id,text,apple_id} if matches!(id.as_str(),"store-login"|"apple-password-change")=>(FunctionCommand::Invoke{id,text},Some(apple_id)),
           other=>(other,None),
          };
          let command=match command {
           FunctionCommand::Invoke{id,text} if id=="apple-password-change"=>{
            if managed.is_some(){let _=events.send(FunctionEvent::Error("Apple 登录或改密仍在执行".into())).await;continue;}
            let prepare=async {
             let spec=OPERATIONS.iter().find(|s|s.id=="apple-password-change").unwrap();
             let parsed=parse_operation(spec,&text)?;
             let target=account::query_current_account(&client,"changePassword").await?;
             if expected_account.as_deref().is_some_and(|id|!id.eq_ignore_ascii_case(&target)){return Err("手机当前 Apple ID 已变化，请刷新后重试".into());}
             let password=parsed.params["password"].as_str().unwrap_or_default();
             let mut input=format!("apple_id={target}\npassword={password}");
             if let Some(url)=parsed.code_url {input.push_str(&format!("\ncode_url={url}"));}
             ctx.prepare(&input).await
            }.await;
            let staged=match prepare {
             Ok(account)=>{
              let passcode=ctx.unlock_code().await;
              match passcode {
               Ok(passcode)=>ctx.db(move|s|s.begin_manual_apple_password_change(&account)).await.map(|account|(account,passcode)),
               Err(e)=>Err(e),
              }
             },Err(e)=>Err(e),
            };
            match staged {Ok((account,passcode))=>{
             let mut input=input_text(&account,true);if let Some(code)=passcode{input.push_str(&format!("\ndevice_passcode={code}"));}
             managed=Some(Managed{account,phase:Phase::ChangePassword,current_operation:None,manual:true});
             FunctionCommand::Invoke{id:"apple-password-change-internal".into(),text:input}
            },Err(e)=>{let _=events.send(FunctionEvent::Error(e)).await;continue;}}
           }
           FunctionCommand::Invoke{id,text} if id=="store-login"=>{
            if managed.is_some(){let _=events.send(FunctionEvent::Error("首次登录处理仍在执行，请等待完成或查询原操作".into())).await;continue;}
            if let Some(expected)=expected_account {
             let spec=OPERATIONS.iter().find(|s|s.id=="store-login").unwrap();
             match parse_operation(spec,&text) {
              Ok(parsed) if parsed.params["apple_id"].as_str().is_some_and(|id|id.eq_ignore_ascii_case(expected.trim()))=>(),
              _=>{let _=events.send(FunctionEvent::Error("登录资料与指定 Apple ID 不一致".into())).await;continue;}
             }
            }
            match ctx.prepare(&text).await {Ok(account)=>{let command=login_command(&account);managed=Some(Managed{account,phase:Phase::Login,current_operation:None,manual:false});command},Err(e)=>{let _=events.send(FunctionEvent::Error(e)).await;continue;}}
           }
           FunctionCommand::Resume{task:31,operation_id} if managed.is_none()=>{
            let id=operation_id.clone();
            match ctx.db(move|s|s.apple_account_for_password_operation(&id)).await {
             Ok(Some(account))=>{let manual=account.params["_password_change_origin"]=="manual";managed=Some(Managed{account,phase:Phase::ChangePassword,current_operation:Some(operation_id.clone()),manual});},
             Ok(None)=>(),Err(e)=>{let _=events.send(FunctionEvent::Error(e)).await;continue;}
            }
            FunctionCommand::Resume{task:31,operation_id}
           }
           c=>c,
          };
          if tx.send(command).await.is_err(){return;}
         },
         event=input.recv()=>{
          let Some(mut event)=event else{return;};
          if let Some(m)=managed.as_mut(){
           match &mut event {
            FunctionEvent::State{record,..}=>{
             if let Err(e)=m.receipt(&ctx,record).await{let _=m.failed(&ctx,&json!({}),true).await;let _=events.send(FunctionEvent::Uncertain(e)).await;managed=None;continue;}
             if record["status"]=="completed" {
              match m.completed(&ctx,record).await {
               Ok(Some(next))=>{
                let message=if m.phase==Phase::ChangePassword{"登录已确认，正在修改为随机八位密码…"}else{"密码修改已确认，正在使用新密码登录…"};
                let _=events.send(FunctionEvent::Progress(message.into())).await;
                m.current_operation=None;
                if tx.send(next).await.is_err(){return;}continue;
               }
               Ok(None)=>{
                record["method"]=json!(if m.manual {"changePassword"} else {"signInAppStore"});
                if m.manual&&m.phase==Phase::LoginNewPassword&&m.account.init_status==AppleInitStatus::Ready&&record["result"]["authenticated"]==true {record["result"]["password_changed"]=json!(true);}
                managed=None;
               },
               Err(e)=>{let _=events.send(FunctionEvent::Uncertain(e)).await;managed=None;continue;}
              }
             } else {record["method"]=json!(if m.manual {"changePassword"} else {"signInAppStore"});}
            },
            FunctionEvent::Error(_)|FunctionEvent::Uncertain(_)=>{
             let uncertain=matches!(event,FunctionEvent::Uncertain(_));
             // Code-entry errors can leave the native challenge active. Keep its owner
             // and pending password so a corrected code can finish the same operation.
             if uncertain||m.current_operation.is_none(){let _=m.failed(&ctx,&json!({}),uncertain).await;managed=None;}
            },
            _=>(),
           }
          }
          if events.send(event).await.is_err(){return;}
         }
        }
    }
}
