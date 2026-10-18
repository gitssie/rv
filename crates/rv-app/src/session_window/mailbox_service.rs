//! Local mailbox operations, independent of phone and Apple account workflows.
use rv_core::{
    StorePaths,
    workspace::{MailboxAccount, WorkspaceStore},
};

#[derive(Clone)]
pub(super) struct MailboxDraft {
    pub email: String,
    pub password: String,
    pub client_id: String,
    pub refresh_token: String,
}

pub(super) enum Request {
    Load,
    Save(MailboxDraft),
    Delete(String),
    Fetch { mailbox: MailboxDraft, save: bool },
}

pub(super) struct Outcome {
    pub accounts: Option<Vec<MailboxAccount>>,
    pub selected: Option<String>,
    pub code: Option<(String, String)>,
    pub message: Option<String>,
}

impl MailboxDraft {
    pub fn validate(&self) -> Result<(), &'static str> {
        rv_session::mailbox::validate_credentials(&self.email, &self.client_id, &self.refresh_token)
            .map_err(|_| "邮箱资料不完整、格式有误或暂不支持此邮箱，请检查后重试")
    }
}

pub(super) fn run(paths: Option<StorePaths>, request: Request) -> Result<Outcome, &'static str> {
    run_with_reader(paths, request, |mailbox| {
        rv_session::mailbox::read_latest_blocking(
            &mailbox.email,
            &mailbox.client_id,
            &mailbox.refresh_token,
        )
    })
}

fn run_with_reader(
    paths: Option<StorePaths>,
    request: Request,
    read: impl FnOnce(&MailboxDraft) -> Result<String, rv_session::mailbox::ReadError>,
) -> Result<Outcome, &'static str> {
    let mut outcome = Outcome {
        accounts: None,
        selected: None,
        code: None,
        message: None,
    };
    let open = || {
        paths
            .as_ref()
            .ok_or("邮箱列表暂时不可用，请重试")
            .and_then(|paths| WorkspaceStore::open(paths).map_err(|_| "邮箱列表暂时不可用，请重试"))
    };
    match request {
        Request::Load => {
            outcome.accounts = Some(
                open()?
                    .mailbox_accounts()
                    .map_err(|_| "邮箱列表加载失败，请重试")?,
            );
        }
        Request::Delete(id) => {
            let mut store = open()?;
            store
                .delete_mailbox_account(&id)
                .map_err(|_| "邮箱删除失败，请重试")?;
            outcome.accounts = Some(
                store
                    .mailbox_accounts()
                    .map_err(|_| "邮箱列表加载失败，请重试")?,
            );
            outcome.message = Some("邮箱已删除".into());
        }
        Request::Save(mailbox) => {
            mailbox.validate()?;
            let mut store = open()?;
            outcome.selected = Some(
                store
                    .save_mailbox_account(
                        &mailbox.email,
                        &mailbox.password,
                        &mailbox.client_id,
                        &mailbox.refresh_token,
                    )
                    .map_err(|_| "邮箱保存失败，请重试")?
                    .id,
            );
            outcome.accounts = Some(
                store
                    .mailbox_accounts()
                    .map_err(|_| "邮箱列表加载失败，请重试")?,
            );
            outcome.message = Some("邮箱已保存".into());
        }
        Request::Fetch { mailbox, save } => {
            mailbox.validate()?;
            if save {
                let mut store = open()?;
                outcome.selected = Some(
                    store
                        .save_mailbox_account(
                            &mailbox.email,
                            &mailbox.password,
                            &mailbox.client_id,
                            &mailbox.refresh_token,
                        )
                        .map_err(|_| "邮箱保存失败，请重试")?
                        .id,
                );
                outcome.accounts = Some(
                    store
                        .mailbox_accounts()
                        .map_err(|_| "邮箱列表加载失败，请重试")?,
                );
            }
            match read(&mailbox) {
                Ok(code) => {
                    outcome.code = Some((mailbox.email, code));
                    outcome.message = Some("验证码已获取".into());
                }
                Err(error) => {
                    tracing::warn!(stage = ?error, "mailbox fetch failed");
                    outcome.message = Some(error.message().into());
                }
            }
        }
    }
    Ok(outcome)
}

pub(super) fn parse_mailbox_text(text: &str) -> Result<(String, String, String, String), ()> {
    let text = text.trim();
    let (email, password, client_id, refresh_token) = if !text.contains(['\n', '\r']) {
        let fields: Vec<_> = text.split("----").map(str::trim).collect();
        if fields.len() != 4 {
            return Err(());
        }
        (fields[0], fields[1], fields[2], fields[3])
    } else {
        let mut email = None;
        let mut password = None;
        let mut client_id = None;
        let mut refresh_token = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                return Err(());
            };
            let slot = match key.trim() {
                "邮箱" | "email" => &mut email,
                "密码" | "password" => &mut password,
                "客户端ID" | "client_id" => &mut client_id,
                "刷新令牌" | "refresh_token" => &mut refresh_token,
                _ => return Err(()),
            };
            if slot.replace(value.trim()).is_some() {
                return Err(());
            }
        }
        (
            email.ok_or(())?,
            password.unwrap_or_default(),
            client_id.ok_or(())?,
            refresh_token.ok_or(())?,
        )
    };
    if email.is_empty() || client_id.is_empty() || refresh_token.is_empty() {
        return Err(());
    }
    Ok((
        email.to_owned(),
        password.to_owned(),
        client_id.to_owned(),
        refresh_token.to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_mailbox_text;

    #[test]
    fn save_option_controls_persistence_even_when_fetch_fails() {
        use super::*;
        for save in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let paths = StorePaths::in_dir(dir.path().into());
            let result = run_with_reader(
                Some(paths.clone()),
                Request::Fetch {
                    mailbox: MailboxDraft {
                        email: "mail@outlook.com".into(),
                        password: "pw".into(),
                        client_id: "00000000-0000-0000-0000-000000000001".into(),
                        refresh_token: "token".into(),
                    },
                    save,
                },
                |_| Err(rv_session::mailbox::ReadError::Authorization),
            )
            .unwrap();
            assert!(result.code.is_none());
            assert_eq!(
                WorkspaceStore::open(&paths)
                    .unwrap()
                    .mailbox_accounts()
                    .unwrap()
                    .len(),
                usize::from(save)
            );
            assert!(result.message.unwrap().contains("登录失败"));
        }
    }

    #[test]
    fn parses_four_part_and_named_mailbox_credentials() {
        assert_eq!(
            parse_mailbox_text("mail@example.test----pw----client----refresh").unwrap(),
            (
                "mail@example.test".into(),
                "pw".into(),
                "client".into(),
                "refresh".into()
            )
        );
        assert_eq!(
            parse_mailbox_text(
                "邮箱=mail@example.test\n密码=p----w\n客户端ID=client\n刷新令牌=refresh----part"
            )
            .unwrap(),
            (
                "mail@example.test".into(),
                "p----w".into(),
                "client".into(),
                "refresh----part".into()
            )
        );
    }
}
