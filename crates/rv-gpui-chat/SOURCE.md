Message and Bubble originate from https://github.com/longbridge/gpui-component at commit 80230b65 (Apache-2.0). Source files are preserved separately from RV business/UI code, retaining native component layout and styling. This adapter re-exports RV's pinned GPUI Component dependencies; it can be removed when RV adopts an upstream release containing these components. Translation text is supplied through native content slots, not a new bubble implementation.

Compatibility change: inline the upstream radius_2xl accessor (Theme::radius * 2.5), which is not exposed by 0.5.2. No new colors or control dimensions are introduced.

MessageScroller and MessageScrollerState are copied from the official upstream
commit 944803984cf0ba3111861d6e1aa9d6265c109ff9,
crates/component/src/message_scroller.rs (Apache-2.0). The component uses the same
pinned gpui-base dependency already used by GPUI Component; no framework upgrade
or parallel scrolling implementation is introduced.

Scroller compatibility: the pinned base has no ScrollBounce wrapper. Its viewport
contains the original GPUI list directly; native scrolling, scrollbar, tail
following and jump-to-latest behavior remain supplied by the upstream scroller.

Upstream test compatibility: ScrollEvent is named ScrollWheelEvent in the pinned
GPUI. The event payload and test assertions are unchanged.
