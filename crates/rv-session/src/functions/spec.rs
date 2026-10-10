use serde_json::{Map, Value};

#[derive(Debug)]
pub struct OperationSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub task: u8,
    pub method: &'static str,
    /// User-supplied keys, in shortcut order. Current Apple IDs are resolved from the phone. Optional keys use a trailing `?`.
    pub fields: &'static [&'static str],
    pub example: &'static str,
    pub hint: &'static str,
}

macro_rules! op {
    ($id:literal,$title:literal,$category:literal,$task:literal,$method:literal,[$($field:literal),*],$example:literal,$hint:literal) => {
        OperationSpec {id:$id,title:$title,category:$category,task:$task,method:$method,fields:&[$($field),*],example:$example,hint:$hint}
    };
}

pub static OPERATIONS: &[OperationSpec] = &[
    op!(
        "store-login",
        "登录 App Store",
        "Apple",
        31,
        "signInAppStore",
        ["apple_id", "password", "code_url?"],
        "账号=demo@example.com\n密码=填写密码\n验证码链接=https://code.example.test/get?id=123",
        "也支持：账号----密码----验证码链接；链接可省略。"
    ),
    op!(
        "cloud-login",
        "登录 iCloud",
        "Apple",
        31,
        "enableICloud",
        ["apple_id", "password", "device_passcode", "code_url?"],
        "账号=demo@example.com\n密码=填写密码\n设备密码=填写设备密码",
        "也支持：账号----密码----设备密码----验证码链接。"
    ),
    op!(
        "store-logout",
        "退出 App Store",
        "Apple",
        31,
        "signOutAppStore",
        [],
        "",
        "退出手机当前 App Store 账户。"
    ),
    op!(
        "apple-logout",
        "退出 Apple 账户",
        "Apple",
        31,
        "signOutAppleID",
        ["local_data_policy"],
        "本机数据=保留",
        "退出当前 Apple 账户的 iCloud、身份与媒体登录。"
    ),
    op!(
        "cloud-logout",
        "退出 iCloud",
        "Apple",
        31,
        "signOutICloud",
        ["local_data_policy"],
        "本机数据=保留",
        "退出手机当前的 iCloud；保留或移除其本机数据。"
    ),
    op!(
        "findmy-off",
        "关闭查找我的 iPhone",
        "Apple",
        31,
        "disableFindMy",
        ["password"],
        "密码=填写当前 iCloud 账户密码",
        "针对手机当前 iCloud 账户，只需输入其密码。"
    ),
    op!(
        "name",
        "修改姓名",
        "Apple",
        31,
        "setName",
        ["first_name", "last_name"],
        "名=Adaline\n姓=Hill",
        "名----姓；修改手机当前账户的姓名。"
    ),
    op!(
        "email-add",
        "添加联系邮箱",
        "Apple",
        31,
        "beginAddingContactEmail",
        ["email", "password?", "client_id?", "refresh_token?"],
        "contact@outlook.com----邮箱密码----OAuth客户端ID----刷新令牌",
        "粘贴四段资料后自动取码并验证；只填邮箱可手动验证。"
    ),
    op!(
        "email-verify",
        "添加联系邮箱",
        "Apple",
        31,
        "verifyContactEmail",
        ["code"],
        "验证码=012345",
        "验证码保留前导零，须为六位数字。"
    ),
    op!(
        "forward",
        "设置转发邮箱",
        "Apple",
        31,
        "setForwardingEmail",
        ["email"],
        "转发邮箱=forward@example.com",
        "影响该 Apple 账户的全部隐藏邮箱。"
    ),
    op!(
        "authorization-list",
        "查看 App 授权",
        "Apple",
        31,
        "querySignInWithAppleApps",
        [],
        "",
        "查询结果可复制，用于选择授权 App。"
    ),
    op!(
        "authorization-stop",
        "停止 App 授权",
        "Apple",
        31,
        "stopUsingAppleID",
        ["bundle_id"],
        "App=fr.vinted",
        "填写需要停止授权的 Bundle ID。"
    ),
    op!(
        "device-list",
        "查看关联设备",
        "Apple",
        31,
        "queryICloudDevices",
        [],
        "",
        "查看原生账户关联设备。"
    ),
    op!(
        "device-remove",
        "移除其它设备",
        "Apple",
        31,
        "removeOtherICloudDevices",
        [],
        "",
        "移除该账户的其它关联设备。"
    ),
    op!(
        "app-save",
        "保存 App 环境",
        "App 环境",
        32,
        "save",
        ["bundle_id", "display_name?", "note?"],
        "App=fr.vinted\n名称=工作账号 A\n备注=备用环境",
        "首次保存必须提供名称；保存后 App 保持停止。"
    ),
    op!(
        "app-switch",
        "切换 App 环境",
        "App 环境",
        32,
        "switch",
        [
            "bundle_id",
            "backup_id",
            "display_name?",
            "note?",
            "discard_current?"
        ],
        "App=fr.vinted\n备份=填写备份标识\n名称=当前环境名称",
        "默认先保存当前环境。放弃当前环境=是 时不填写名称和备注。"
    ),
    op!(
        "app-baseline",
        "创建空白基线",
        "App 环境",
        32,
        "createBaseline",
        ["bundle_id"],
        "App=fr.vinted",
        "创建当前 App 版本的空白模板，不改变正在使用的数据。"
    ),
    op!(
        "app-list",
        "选择备份 / 查看标识",
        "App 环境",
        32,
        "list",
        ["bundle_id"],
        "App=fr.vinted",
        "复制返回的 backup_id，粘贴到切换、编辑或删除操作。"
    ),
    op!(
        "app-edit",
        "编辑备份",
        "App 环境",
        32,
        "edit",
        ["bundle_id", "backup_id", "display_name", "note"],
        "App=fr.vinted\n备份=填写备份标识\n名称=工作账号 B\n备注=备用环境",
        "仅编辑快照名称与备注；备注允许为空。"
    ),
    op!(
        "app-delete",
        "删除备份",
        "App 环境",
        32,
        "delete",
        ["bundle_id", "backup_id"],
        "App=fr.vinted\n备份=填写备份标识",
        "当前关联备份与空白基线不能删除。"
    ),
    op!(
        "app-open",
        "打开 App",
        "App 环境",
        32,
        "launch",
        ["bundle_id"],
        "App=fr.vinted",
        "打开所选 App。"
    ),
    op!(
        "app-stop",
        "停止 App",
        "App 环境",
        32,
        "stop",
        ["bundle_id"],
        "App=fr.vinted",
        "停止 App 及其扩展。"
    ),
    op!(
        "apple-query",
        "读取账户资料",
        "Apple",
        31,
        "queryState",
        [],
        "",
        "按需查询，用于确认操作结果。"
    ),
    op!(
        "name-query",
        "读取姓名",
        "Apple",
        31,
        "queryName",
        [],
        "",
        "按需读取姓名资料。"
    ),
    op!(
        "email-query",
        "读取联系邮箱",
        "Apple",
        31,
        "queryEmail",
        [],
        "",
        "按需读取邮箱与验证信息。"
    ),
    op!(
        "app-describe",
        "读取 App 环境详情",
        "App 环境",
        32,
        "describe",
        ["bundle_id"],
        "App=fr.vinted",
        "按需读取环境详情。"
    ),
    op!(
        "app-status",
        "确认 App 是否停止",
        "App 环境",
        32,
        "status",
        ["bundle_id"],
        "App=fr.vinted",
        "按需确认 App 和扩展进程。"
    ),
];

pub struct PreparedOperation {
    pub spec: &'static OperationSpec,
    pub params: Value,
    pub code_url: Option<String>,
    pub(crate) mailbox: Option<super::mailbox::Credentials>,
}
impl std::fmt::Debug for PreparedOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedOperation")
            .field("id", &self.spec.id)
            .finish_non_exhaustive()
    }
}

fn key(key: &str) -> &str {
    match key.trim() {
        "账号" | "账户" | "Apple ID" => "apple_id",
        "密码" => "password",
        "客户端ID" => "client_id",
        "刷新令牌" => "refresh_token",
        "设备密码" => "device_passcode",
        "验证码链接" => "code_url",
        "本机数据" => "local_data_policy",
        "名" => "first_name",
        "姓" => "last_name",
        "邮箱" | "转发邮箱" => "email",
        "验证标识" => "challenge_id",
        "验证码" => "code",
        "App" | "应用" => "bundle_id",
        "名称" => "display_name",
        "备注" => "note",
        "备份" => "backup_id",
        "放弃当前环境" => "discard_current",
        other => other,
    }
}

pub fn parse_operation(
    spec: &'static OperationSpec,
    text: &str,
) -> Result<PreparedOperation, String> {
    if text.len() > 16384 {
        return Err("输入超过 16 KiB".into());
    }
    let mut params = Map::new();
    if spec.fields.is_empty() {
        if !text.trim().is_empty() {
            return Err("此功能不需要参数".into());
        }
        return Ok(PreparedOperation {
            spec,
            params: Value::Object(params),
            code_url: None,
            mailbox: None,
        });
    }

    // Recognize named syntax by the first key, never by `=` inside a URL/password.
    let named = text
        .lines()
        .find(|l| !l.is_empty())
        .and_then(|l| l.split_once('='))
        .is_some_and(|(k, _)| {
            spec.fields
                .iter()
                .any(|f| f.trim_end_matches('?') == key(k))
        });
    if named {
        for (index, line) in text.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let (k, value) = line
                .split_once('=')
                .ok_or_else(|| format!("第 {} 行须为 字段=内容", index + 1))?;
            let k = key(k);
            if !spec.fields.iter().any(|f| f.trim_end_matches('?') == k) {
                return Err(format!("第 {} 行字段不适用于此功能", index + 1));
            }
            if params
                .insert(k.into(), Value::String(value.into()))
                .is_some()
            {
                return Err(format!("第 {} 行字段重复", index + 1));
            }
        }
    } else {
        let fields: Vec<_> = text.split("----").collect();
        let required = spec.fields.iter().take_while(|f| !f.ends_with('?')).count();
        if text.contains(['\n', '\r'])
            || fields.len() < required
            || fields.len() > spec.fields.len()
        {
            return Err("参数数量不匹配，请使用示例格式；密码包含 ---- 时使用多行格式".into());
        }
        for (field, value) in spec.fields.iter().zip(fields) {
            params.insert(
                field.trim_end_matches('?').into(),
                Value::String(value.into()),
            );
        }
    }
    for field in spec.fields {
        let k = field.trim_end_matches('?');
        let v = params.get(k).and_then(Value::as_str);
        if !field.ends_with('?') && (v.is_none() || (v == Some("") && k != "note")) {
            return Err(format!("缺少必填字段：{k}"));
        }
        if v.is_some_and(|s| s.len() > 4096) {
            return Err(format!("字段过长：{k}"));
        }
    }
    if let Some(policy) = params.get_mut("local_data_policy") {
        *policy = match policy.as_str() {
            Some("保留" | "keep") => Value::String("keep".into()),
            Some("移除" | "remove") => Value::String("remove".into()),
            _ => return Err("本机数据必须填写保留或移除".into()),
        };
    }
    if let Some(discard) = params.get_mut("discard_current") {
        *discard = match discard.as_str() {
            Some("是" | "true") => Value::Bool(true),
            Some("否" | "false") => Value::Bool(false),
            _ => return Err("放弃当前环境必须填写是或否".into()),
        };
    }
    if params.get("discard_current") == Some(&Value::Bool(true))
        && (params.contains_key("display_name") || params.contains_key("note"))
    {
        return Err("放弃当前环境时不能同时提供名称或备注".into());
    }
    if let Some(code) = params.get("code").and_then(Value::as_str)
        && !valid_code(code)
    {
        return Err("验证码须为六位数字".into());
    }
    let code_url = params
        .remove("code_url")
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.is_empty());
    if let Some(url) = &code_url {
        super::validate_code_url(url)?;
    }
    let mailbox = if spec.id == "email-add" {
        super::mailbox::Credentials::take_from(&mut params)?
    } else {
        None
    };
    Ok(PreparedOperation {
        spec,
        params: Value::Object(params),
        code_url,
        mailbox,
    })
}

pub fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oauth_email_material_is_removed_from_phone_parameters() {
        let spec = OPERATIONS.iter().find(|s| s.id == "email-add").unwrap();
        let parsed = parse_operation(spec, " demo@HOTMAIL.COM ----demo-password----00000000-0000-4000-8000-000000000001----demo-refresh").unwrap();
        assert_eq!(
            parsed.params,
            serde_json::json!({"email":"demo@HOTMAIL.COM"})
        );
        let credentials = parsed.mailbox.as_ref().unwrap();
        assert_eq!(
            credentials.client_id,
            "00000000-0000-4000-8000-000000000001"
        );
        assert!(!format!("{parsed:?} {credentials:?}").contains("demo-refresh"));
        assert!(parse_operation(spec, "a@gmail.com----demo-password----00000000-0000-4000-8000-000000000001----demo-refresh").is_err());
        assert!(parse_operation(spec, "a@outlook.com----demo-password").is_err());
        assert!(
            parse_operation(spec, "a@outlook.com")
                .unwrap()
                .mailbox
                .is_none()
        );
    }
    #[test]
    fn shortcut_preserves_secrets_and_url_query() {
        let r = parse_operation(
            &OPERATIONS[0],
            "a@example.test---- p=a ss ----https://example.test/get?id=12&x=3",
        )
        .unwrap();
        assert_eq!(r.params["password"], " p=a ss ");
        assert!(!r.params.as_object().unwrap().contains_key("code_url"));
        assert!(r.code_url.unwrap().ends_with("id=12&x=3"));
    }
    #[test]
    fn named_handles_delimiters_and_reports_no_secret_values() {
        let r = parse_operation(&OPERATIONS[0], "账号=a@example.test\n密码= p----x=y ").unwrap();
        assert_eq!(r.params["password"], " p----x=y ");
        assert!(!format!("{r:?}").contains("p----"));
        assert!(parse_operation(&OPERATIONS[0], "账号=a\n密码=b\n密码=secret").is_err());
        assert!(parse_operation(&OPERATIONS[0], "账号=a\n密码=b\n未知=secret").is_err());
    }
    #[test]
    fn typed_switch_and_explicit_logout_policy() {
        let spec = OPERATIONS.iter().find(|s| s.id == "app-switch").unwrap();
        assert_eq!(
            parse_operation(spec, "App=fr.vinted\n备份=baseline:1\n放弃当前环境=是")
                .unwrap()
                .params["discard_current"],
            true
        );
        assert!(parse_operation(spec, "App=fr.vinted\n备份=x\n放弃当前环境=是\n名称=A").is_err());
        assert!(
            parse_operation(
                OPERATIONS.iter().find(|s| s.id == "apple-logout").unwrap(),
                "账号=a"
            )
            .is_err()
        );
        assert_eq!(
            parse_operation(
                OPERATIONS.iter().find(|s| s.id == "apple-logout").unwrap(),
                "本机数据=保留"
            )
            .unwrap()
            .params["local_data_policy"],
            "keep"
        );
    }
    #[test]
    fn current_account_operations_accept_only_the_actual_user_input() {
        for (id, text) in [
            ("name", "Adaline----Hill"),
            ("findmy-off", "密码=example"),
            ("email-add", "邮箱=contact@example.test"),
            ("email-verify", "验证码=012345"),
            ("forward", "邮箱=forward@example.test"),
            ("authorization-list", ""),
            ("device-list", ""),
        ] {
            let spec = OPERATIONS.iter().find(|s| s.id == id).unwrap();
            let parsed = parse_operation(spec, text).unwrap();
            assert!(parsed.params.get("apple_id").is_none());
            assert!(parse_operation(spec, &format!("账号=override@example.test\n{text}")).is_err());
        }
        let verify = OPERATIONS.iter().find(|s| s.id == "email-verify").unwrap();
        assert!(parse_operation(verify, "验证码=12345").is_err());
        assert!(parse_operation(verify, "验证标识=override\n验证码=012345").is_err());
    }
}
