//! Resolve “On My iPhone / Downloads” without assuming a device-specific UUID.
use crate::phone_http::PhoneHttpClient;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

// Read only container identity and directory existence. No files are created,
// moved, downloaded or deleted, and no Safari preferences are changed.
const LOCATE: &str = r#"encoder=''
for candidate in /usr/bin/base64 /var/jb/usr/bin/base64 /bin/base64; do
    if [ -x "$candidate" ]; then encoder="$candidate"; break; fi
done
[ -n "$encoder" ] || exit 127
for group in /var/mobile/Containers/Shared/AppGroup/* /var/mobile/Containers/Data/Application/*; do
    metadata="$group/.com.apple.mobile_container_manager.metadata.plist"
    [ -f "$metadata" ] || continue
    [ -d "$group/File Provider Storage" ] || [ -d "$group/Library/Safari" ] || continue
    printf '%s\t' "$group"
    if [ -d "$group/File Provider Storage/Downloads" ]; then printf '1\t'; else printf '0\t'; fi
    "$encoder" --wrap=0 "$metadata" || exit 1
    printf '\n'
done"#;

pub(crate) async fn resolve(client: &PhoneHttpClient) -> Result<Vec<crate::SafariFolder>, String> {
    let records = shell(client, LOCATE).await?;
    let (mut folders, safari) = containers(&records)?;
    if let Some(app) = safari {
        let command = format!(
            r#"stattool=''
for candidate in /usr/bin/stat /var/jb/usr/bin/stat /bin/stat; do
    if [ -x "$candidate" ]; then stattool="$candidate"; break; fi
done
[ -n "$stattool" ] || exit 127
for directory in {app}/tmp/QuickLookContent-*; do
    [ -d "$directory" ] || continue
    printf '%s\t' "$directory"
    "$stattool" -c '%Y' "$directory" || exit 1
done"#
        );
        let output = shell(client, &command).await?;
        folders.extend(previews(&output, &app)?);
    }
    folders.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    folders.dedup_by(|a, b| a.path == b.path);
    Ok(folders)
}

async fn shell(client: &PhoneHttpClient, command: &str) -> Result<String, String> {
    let (status, response) = client
        .json("/api/scripts/shell", &json!({"fields":[command]}))
        .await
        .map_err(|_| "Safari lookup connection unavailable")?;
    if status != 200 {
        return Err("Safari lookup was rejected".into());
    }
    decode(&response)
}

fn decode(response: &Value) -> Result<String, String> {
    let encoded = response["value"]
        .as_str()
        .filter(|text| text.len() <= 1024 * 1024)
        .ok_or("Invalid Safari lookup response")?;
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "Invalid Safari lookup response")?;
    let result: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Safari lookup response")?;
    if result["exit_code"].as_i64() != Some(0) {
        return Err("Safari directory lookup failed".into());
    }
    let stdout = result["stdout_base64"]
        .as_str()
        .ok_or("Missing Safari lookup output")?;
    let bytes = STANDARD
        .decode(stdout)
        .map_err(|_| "Invalid Safari lookup output")?;
    String::from_utf8(bytes).map_err(|_| "Invalid Safari lookup output".into())
}

fn container_path(container: &str, kind: &str) -> Option<String> {
    let prefix = format!("/var/mobile/Containers/{kind}/");
    let id = container.strip_prefix(&prefix)?;
    if id.len() != 36 || uuid::Uuid::parse_str(id).is_err() {
        return None;
    }
    Some(format!("/Containers/{kind}/{id}"))
}

fn containers(output: &str) -> Result<(Vec<crate::SafariFolder>, Option<String>), String> {
    let mut folders = Vec::new();
    let mut apps = Vec::new();
    let mut local_count = 0;
    for line in output.lines().filter(|line| !line.is_empty()) {
        let mut fields = line.split('\t');
        let path = fields.next().ok_or("Invalid container metadata")?;
        let exists = match fields.next() {
            Some("1") => true,
            Some("0") => false,
            _ => return Err("Invalid container metadata".into()),
        };
        let metadata = fields.next().ok_or("Missing container metadata")?;
        if fields.next().is_some() {
            return Err("Invalid container metadata".into());
        }
        let metadata = STANDARD
            .decode(metadata)
            .map_err(|_| "Invalid container metadata")?;
        let metadata = plist::Value::from_reader(std::io::Cursor::new(metadata))
            .map_err(|_| "Invalid container metadata")?;
        let identity = metadata
            .as_dictionary()
            .and_then(|dict| dict.get("MCMMetadataIdentifier"))
            .and_then(plist::Value::as_string);
        match identity {
            Some("group.com.apple.FileProvider.LocalStorage") => {
                local_count += 1;
                let path = virtual_path(path).ok_or("Invalid local storage container")?;
                if exists {
                    folders.push(crate::SafariFolder {
                        path,
                        modified: i64::MAX,
                        downloads: true,
                    });
                }
            }
            Some("com.apple.mobilesafari") => {
                container_path(path, "Data/Application").ok_or("Invalid Safari container")?;
                apps.push(path.to_owned());
            }
            _ => {}
        }
    }
    if apps.len() > 1 || local_count > 1 {
        return Err("Safari container is ambiguous".into());
    }
    if apps.is_empty() && local_count == 0 {
        return Err("Safari file container was not found".into());
    }
    Ok((folders, apps.pop()))
}

fn previews(output: &str, app: &str) -> Result<Vec<crate::SafariFolder>, String> {
    let root = container_path(app, "Data/Application").ok_or("Invalid Safari container")?;
    let prefix = format!("{app}/tmp/");
    let mut folders = Vec::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let (path, modified) = line.split_once('\t').ok_or("Invalid Safari folder")?;
        let name = path
            .strip_prefix(&prefix)
            .filter(|name| {
                name.starts_with("QuickLookContent-")
                    && name.len() > 17
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .ok_or("Invalid Safari folder")?;
        let modified: i64 = modified.parse().map_err(|_| "Invalid Safari folder time")?;
        if modified < 0 {
            return Err("Invalid Safari folder time".into());
        }
        folders.push(crate::SafariFolder {
            path: format!("{root}/tmp/{name}"),
            modified,
            downloads: false,
        });
    }
    Ok(folders)
}

fn virtual_path(container: &str) -> Option<String> {
    let id = container.strip_prefix("/var/mobile/Containers/Shared/AppGroup/")?;
    if id.len() != 36 || uuid::Uuid::parse_str(id).is_err() {
        return None;
    }
    Some(format!(
        "/Containers/Shared/AppGroup/{id}/File Provider Storage/Downloads"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    const GROUP: &str =
        "/var/mobile/Containers/Shared/AppGroup/12345678-1234-1234-1234-123456789abc";
    const APP: &str =
        "/var/mobile/Containers/Data/Application/87654321-1234-1234-1234-123456789abc";
    fn record(path: &str, identity: &str, exists: bool) -> String {
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert("MCMMetadataIdentifier".into(), identity.into());
        let mut bytes = Vec::new();
        plist::Value::Dictionary(dictionary)
            .to_writer_binary(&mut bytes)
            .unwrap();
        format!("{path}\t{}\t{}\n", u8::from(exists), STANDARD.encode(bytes))
    }
    #[test]
    fn discovers_downloads_and_actual_safari_save_folders_by_identity() {
        let data = record(GROUP, "group.com.apple.FileProvider.LocalStorage", false)
            + &record(APP, "com.apple.mobilesafari", false);
        let (folders, app) = containers(&data).unwrap();
        assert!(folders.is_empty());
        assert_eq!(app.as_deref(), Some(APP));
        let locations = previews(
            &format!("{APP}/tmp/QuickLookContent-save1\t1793000000\n"),
            APP,
        )
        .unwrap();
        assert_eq!(
            locations[0].path,
            "/Containers/Data/Application/87654321-1234-1234-1234-123456789abc/tmp/QuickLookContent-save1"
        );
        assert!(!locations[0].downloads);
        let (folders, _) = containers(&record(
            GROUP,
            "group.com.apple.FileProvider.LocalStorage",
            true,
        ))
        .unwrap();
        assert!(folders[0].downloads);
        assert!(
            containers(&(data.clone() + &record(APP, "com.apple.mobilesafari", false))).is_err()
        );
        assert!(
            previews(
                &format!("{APP}/tmp/QuickLookContent-save/../../other\t1"),
                APP
            )
            .is_err()
        );
        assert!(previews(&format!("{APP}/tmp/QuickLookContent-save1\t-1"), APP).is_err());
        assert!(
            containers(&record(
                "/var/mobile/Containers/Data/Application/invalid",
                "com.apple.mobilesafari",
                false
            ))
            .is_err()
        );
        assert!(decode(&json!({"value":"bad"})).is_err());
    }
}
