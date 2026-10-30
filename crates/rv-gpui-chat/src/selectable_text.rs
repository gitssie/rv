use gpui::{ElementId, SharedString};
use gpui_component::text::{SelectionFormat, TextView};

fn literal_markdown(value: &str) -> String {
    let mut source=String::new();
    for ch in value.replace("\r\n", "\n").chars() {
        match ch {
            ' ' => source.push_str("&#32;"),
            '\n' => source.push_str("&#10;"),
            '\r' => source.push_str("&#13;"),
            '\t' => source.push_str("&#9;"),
            ch if ch.is_ascii_punctuation() => { source.push('\\'); source.push(ch); }
            ch => source.push(ch),
        }
    }
    source
}

pub fn selectable_text(id: impl Into<ElementId>, value: impl AsRef<str>) -> TextView {
    TextView::markdown(id, SharedString::from(literal_markdown(value.as_ref())))
        .selectable(true).selection_format(SelectionFormat::Plain).scrollable(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;
    use gpui_component::text::TextViewState;

    #[gpui::test]
    fn literal_selection_preserves_multiline_text(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| gpui_component::init(cx));
        for text in ["first\nsecond", "first\n\nthird", "\nfirst", " <tag> & **literal**  text", "&lt; </textarea><img src=\"https://example.invalid/x\">", "a\t  b\n\n\nend"] {
            let state = cx.new(|cx| TextViewState::markdown(&literal_markdown(text),cx).selectable(true));
            cx.run_until_parked();
            state.update(cx,|state,cx| state.select_all(cx));
            // Native TextView includes its paragraph separator in select-all.
            assert_eq!(state.read_with(cx,|state,_| state.selected_text()),format!("{text}\n"));
        }
    }
}
