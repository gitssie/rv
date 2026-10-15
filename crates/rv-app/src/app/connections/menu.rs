//! Connection actions shared by the ellipsis and context menus.
use super::super::AddressBookApp;
use crate::actions::*;
use gpui::*;
use gpui_component::menu::PopupMenuItem;
use rv_core::ConnectionId;
pub(in crate::app) fn connection_menu(
    book: Entity<AddressBookApp>,
    id: ConnectionId,
) -> impl Fn(
    gpui_component::menu::PopupMenu,
    &mut Window,
    &mut Context<gpui_component::menu::PopupMenu>,
) -> gpui_component::menu::PopupMenu {
    move |menu, _, cx| {
        let busy = book.read(cx).credential_busy;
        let connect = book.clone();
        let props = book.clone();
        let dup = book.clone();
        let del = book.clone();
        menu.item(
            PopupMenuItem::new("连接")
                .disabled(busy)
                .on_click(move |_, _, cx| {
                    connect.update(cx, |this, cx| {
                        this.selected = Some(id);
                        this.connect_id(id, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new("编辑…")
                .disabled(busy)
                .on_click(move |_, window, cx| {
                    props.update(cx, |this, cx| {
                        this.selected = Some(id);
                        this.on_properties(&OpenProperties, window, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new("复制连接")
                .disabled(busy)
                .on_click(move |_, window, cx| {
                    dup.update(cx, |this, cx| {
                        this.selected = Some(id);
                        this.on_duplicate(&DuplicateSelected, window, cx);
                    });
                }),
        )
        .separator()
        .item(
            PopupMenuItem::new("删除…")
                .disabled(busy)
                .on_click(move |_, _, cx| {
                    del.update(cx, |this, cx| {
                        this.ask_delete(id, cx);
                    });
                }),
        )
    }
}
