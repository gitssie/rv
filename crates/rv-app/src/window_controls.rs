//! Platform window controls for the home window. AppKit hides its titlebar in
//! native fullscreen, so keep one set of AppKit-created standard buttons in the
//! reserved titlebar space in both modes. Never swap groups during transitions.
#[cfg(target_os = "macos")]
mod macos {
    use gpui::Window;
    use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained, sel};
    use objc2_app_kit::{
        NSAccessibility, NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua,
        NSAppearanceNameDarkAqua, NSAutoresizingMaskOptions, NSButton, NSView, NSWindow,
        NSWindowButton, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[derive(Default)]
    pub struct WindowControls {
        view: Option<Retained<NSView>>,
        minimize: Option<Retained<NSButton>>,
        dark: Option<bool>,
    }
    impl WindowControls {
        pub fn sync(&mut self, window: &Window, dark: bool) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let Ok(handle) = HasWindowHandle::window_handle(window) else {
                return;
            };
            let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
                return;
            };
            // SAFETY: GPUI owns this AppKit content view for the lifetime of the
            // current Window. All view operations occur on the AppKit main thread.
            let content = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
            let Some(native) = content.window() else {
                return;
            };
            // AppKit restores and animates its own titlebar buttons on mode
            // changes. Hide those originals in both modes, not only fullscreen,
            // so hover/enter/exit cannot briefly reveal a second moving group.
            for kind in [
                NSWindowButton::CloseButton,
                NSWindowButton::MiniaturizeButton,
                NSWindowButton::ZoomButton,
            ] {
                if let Some(button) = native.standardWindowButton(kind) {
                    button.setHidden(true);
                }
            }
            if let Some(button) = &self.minimize {
                button.setEnabled(!window.is_fullscreen());
            }
            let height = f64::from(f32::from(crate::theme::toolbar_height()));
            let y = if content.isFlipped() {
                0.
            } else {
                content.bounds().size.height - height
            };
            let frame = NSRect::new(NSPoint::new(0., y), NSSize::new(80., height));
            if let Some(view) = &self.view {
                view.setFrame(frame);
                if self.dark != Some(dark) {
                    set_appearance(view, dark);
                    self.dark = Some(dark);
                }
                return;
            }
            let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
            set_appearance(&view, dark);
            self.dark = Some(dark);
            view.setAutoresizingMask(if content.isFlipped() {
                NSAutoresizingMaskOptions::ViewMaxYMargin
            } else {
                NSAutoresizingMaskOptions::ViewMinYMargin
            });
            let style = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable;
            for (index, kind, action, label) in [
                (
                    0,
                    NSWindowButton::CloseButton,
                    sel!(performClose:),
                    "关闭窗口",
                ),
                (
                    1,
                    NSWindowButton::MiniaturizeButton,
                    // The standard AppKit button recognizes this action;
                    // miniaturize: makes its native cell disappear.
                    sel!(performMiniaturize:),
                    "最小化窗口",
                ),
                (
                    2,
                    NSWindowButton::ZoomButton,
                    sel!(toggleFullScreen:),
                    "切换全屏",
                ),
            ] {
                let Some(button) = NSWindow::standardWindowButton_forStyleMask(kind, style, mtm)
                else {
                    continue;
                };
                // This NSView is not flipped: convert the toolbar's top inset
                // once. The same frame is retained throughout fullscreen changes.
                let size = button.frame().size;
                button.setFrame(NSRect::new(
                    NSPoint::new(9. + index as f64 * 23., height - 12. - size.height),
                    size,
                ));
                // SAFETY: the target is this button's containing NSWindow, which
                // implements these AppKit actions and outlives its content view.
                unsafe {
                    button.setTarget(Some(&native));
                    button.setAction(Some(action));
                }
                button.setAccessibilityLabel(Some(&NSString::from_str(label)));
                // AppKit does not permit minimizing a fullscreen window.
                if kind == NSWindowButton::MiniaturizeButton {
                    button.setEnabled(!window.is_fullscreen());
                    self.minimize = Some(button.clone());
                }
                view.addSubview(&button);
            }
            content.addSubview(&view);
            self.view = Some(view);
        }
    }

    fn set_appearance(view: &NSView, dark: bool) {
        // GPUI draws the app theme itself. This native overlay must use the same
        // appearance so AppKit draws disabled/inactive buttons with the right
        // contrast; leave all button colors and hover states to AppKit.
        let name = unsafe {
            if dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            }
        };
        view.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
    }
}
#[cfg(target_os = "macos")]
pub use macos::WindowControls;

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct WindowControls;
#[cfg(not(target_os = "macos"))]
impl WindowControls {
    pub fn sync(&mut self, _window: &gpui::Window, _dark: bool) {}
}
