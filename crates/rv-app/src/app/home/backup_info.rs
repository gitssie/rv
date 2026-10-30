//! Read-only phone backup metadata; opening this popover never contacts the phone.
use super::*;
use chrono::{DateTime, Local};
use gpui_component::{
    description_list::{DescriptionItem, DescriptionList},
    group_box::GroupBox,
    Icon, h_flex, v_flex,
};

impl AddressBookApp {
    fn backup_device_name(&self, link: &BackupLink) -> String {
        self.book
            .get(link.device_id)
            .map(|device| device.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "未知手机".into())
    }

    pub(super) fn backup_source_labels(&self, links: &[BackupLink]) -> Vec<String> {
        links
            .iter()
            .map(|link| format!("{} · {}", self.backup_device_name(link), link.title))
            .collect()
    }

    pub(super) fn render_backup_info(
        &self,
        id: &AppId,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let links = match WorkspaceStore::open(self.book.paths()).and_then(|s| s.backups(&id)) {
            Ok(links) => links,
            Err(error) => {
                tracing::error!(app_id=id.as_str(),error=%error,"Backup metadata read failed");
                return div().text_sm().child("读取备份详情失败").into_any_element();
            }
        };
        let sources: Vec<_> = links
            .iter()
            .map(|link| {
                let metadata: serde_json::Value =
                    serde_json::from_str(&link.metadata).unwrap_or_default();
                let text = |key: &str| {
                    metadata[key]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("—")
                        .to_string()
                };
                let raw_date = text("date");
                let date = DateTime::parse_from_rfc3339(&raw_date)
                    .map(|date| {
                        date.with_timezone(&Local)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or(raw_date);
                // Only catalog display fields are presented, never state/auth payloads.
                (
                    self.backup_device_name(link),
                    vec![
                        ("备份标题", link.title.clone()),
                        ("备份时间", date),
                        ("App 版本", text("version")),
                        ("构建版本", text("build")),
                        ("备份备注", text("note")),
                    ],
                )
            })
            .collect();
        v_flex()
            .gap_3()
            .children(sources.iter().map(|(device, fields)| {
                GroupBox::new().title(
                    h_flex().gap_2().items_center()
                        .child(Icon::new(crate::assets::FunctionIcon::Archive))
                        .child(device.clone()),
                ).child(
                    DescriptionList::horizontal().columns(1).children(
                        fields.iter().map(|(label, value)| DescriptionItem::new(*label).value(value.clone())),
                    ),
                )
            }))
            .into_any_element()
    }
}
