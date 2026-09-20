use rv_core::{ClipboardMode, QualityPreset};
use vnc::VncEncoding;

pub fn encodings_for(quality: QualityPreset, clipboard: ClipboardMode) -> Vec<VncEncoding> {
    let mut list = match quality {
        QualityPreset::Best => vec![VncEncoding::Tight, VncEncoding::CopyRect],
        QualityPreset::Fast => vec![VncEncoding::Zrle, VncEncoding::CopyRect],
        QualityPreset::Auto => vec![
            VncEncoding::Tight,
            VncEncoding::Zrle,
            VncEncoding::Trle,
            VncEncoding::CopyRect,
        ],
    };
    list.push(VncEncoding::DesktopSizePseudo);
    if clipboard == ClipboardMode::Utf8 {
        list.push(VncEncoding::ExtendedClipboardPseudo);
    }
    // No `CursorPseudo`: the compositor has no local cursor layer, so the
    // server must keep drawing the pointer into the framebuffer itself.
    list.push(VncEncoding::Raw);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_common_encodings_and_only_enables_extended_clipboard_for_utf8() {
        for q in [
            QualityPreset::Auto,
            QualityPreset::Best,
            QualityPreset::Fast,
        ] {
            let e = encodings_for(q, ClipboardMode::Utf8);
            assert!(e.contains(&VncEncoding::Raw));
            assert!(e.contains(&VncEncoding::DesktopSizePseudo));
            assert!(e.contains(&VncEncoding::ExtendedClipboardPseudo));
            assert!(!e.contains(&VncEncoding::CursorPseudo));

            let latin1 = encodings_for(q, ClipboardMode::Latin1);
            assert!(!latin1.contains(&VncEncoding::ExtendedClipboardPseudo));
        }
    }
}
