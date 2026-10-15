//! HTML notices become inert readable text; original markup stays in payload.
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{NodeData, RcDom};
pub(super) fn plain_text(body: &str) -> String {
    let dom = html5ever::parse_document(RcDom::default(), Default::default()).one(body);
    let mut stack = vec![dom.document.clone()];
    let mut output = String::new();
    while let Some(node) = stack.pop() {
        match &node.data {
            NodeData::Text { contents } => output.push_str(&contents.borrow()),
            NodeData::Element { name, .. } => {
                if ["script", "style", "head", "iframe"].contains(&name.local.as_ref()) {
                    continue;
                }
                if ["p", "div", "br", "li", "h1", "h2", "h3"].contains(&name.local.as_ref())
                    && !output.ends_with('\n')
                    && !output.is_empty()
                {
                    output.push('\n');
                }
            }
            _ => {}
        }
        stack.extend(node.children.borrow().iter().rev().cloned());
    }
    output
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
#[cfg(test)]
mod tests {
    #[test]
    fn notice_entities_and_blocks_are_readable_without_loading_images() {
        assert_eq!(
            super::plain_text(
                "<p>Hello &amp; merci</p><p>世界<br>Next</p><script>hidden()</script><img src='https://example.invalid/test'>"
            ),
            "Hello & merci\n世界\nNext"
        );
    }
}
