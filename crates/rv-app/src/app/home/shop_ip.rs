//! Explicit shop proxy observation, displayed in an anchored native popover.
use super::*;
use gpui_component::{
    IconName,
    button::{Button, ButtonVariants},
    description_list::{DescriptionItem, DescriptionList},
    popover::Popover,
};

#[derive(Default)]
pub(super) struct ShopIpView {
    pub generation: u64,
    pub loading: bool,
    pub result: Option<Result<rv_business::shop_ip::ShopIp, String>>,
}

impl AddressBookApp {
    pub(super) fn render_shop_ip_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(app) = &self.home.editing else {
            return div().into_any_element();
        };
        let id = app.id.clone();
        let owner = cx.entity();
        Popover::new(SharedString::from(format!(
            "shop-ip-popover-{}",
            id.as_str()
        )))
        .trigger(
            Button::new("shop-ip")
                .ghost()
                .icon(IconName::Globe)
                .debug_selector(|| "shop-ip".into())
                .tooltip("检测 IP")
                .accessibility_label("检测 IP"),
        )
        .on_open_change(cx.listener(move |this, open: &bool, _, cx| {
            this.home.business.ip.generation += 1;
            this.home.business.ip.loading = false;
            this.home.business.ip.result = None;
            if *open {
                this.inspect_shop_ip(id.clone(), cx);
            }
            cx.notify();
        }))
        .content(move |_, _, cx| {
            owner.update(cx, |this, _| {
                let view = &this.home.business.ip;
                if view.loading {
                    return div()
                        .child(gpui_component::spinner::Spinner::new())
                        .into_any_element();
                }
                match &view.result {
                    Some(Ok(ip)) => DescriptionList::horizontal()
                        .label_width(rems(5.5))
                        .columns(1)
                        .children([
                            DescriptionItem::new("IP").value(ip.ip.clone()),
                            DescriptionItem::new("Country").value(ip.country.clone()),
                            DescriptionItem::new("Region").value(ip.region.clone()),
                            DescriptionItem::new("City").value(ip.city.clone()),
                        ])
                        .into_any_element(),
                    Some(Err(error)) => div().text_sm().child(error.clone()).into_any_element(),
                    None => div().into_any_element(),
                }
            })
        })
        .into_any_element()
    }

    fn inspect_shop_ip(&mut self, id: AppId, cx: &mut Context<Self>) {
        let paths = self.book.paths().clone();
        self.home.business.ip.loading = true;
        let generation = self.home.business.ip.generation;
        cx.spawn(async move |this, cx| {
            let result = rv_business::shop_ip::inspect(paths, id.clone()).await;
            let _ = this.update(cx, |this, cx| {
                if this.home.business.ip.generation != generation
                    || !this.home.editing.as_ref().is_some_and(|app| app.id == id)
                {
                    return;
                }
                this.home.business.ip.loading = false;
                this.home.business.ip.result = Some(result);
                cx.notify();
            });
        })
        .detach();
    }
}
