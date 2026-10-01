# App shortcut icons

Safari, WeChat, Photos, Settings and ProjectX PNG assets were recreated with ImageGen from the approved
[`app-shortcuts-toolbar-v2.png`](../design/app-shortcuts-toolbar-v2.png) design.
They are local fallback artwork. `app-store.png` is a 512×512 PNG converted
from the local macOS App Store `AppIcon.icns`, used until the device provides
its actual icon. Its 50px canvas padding is cropped at decode time so the
412×412 artwork fills the same visual size as the other icons, retaining the
source PNG without lossy compression. Unconfigured connections default to Safari, Photos, Settings
and App Store; WeChat and ProjectX remain available as optional shortcuts.
The production assets are 512×512 transparent PNGs, optimized for the 28px
toolbar and 38px picker icons, including Retina displays. The files are exported directly from the original generated artwork rather
than enlarged from the previous 128px assets. RV embeds these
files in the executable and shares the decoded textures across sessions.
Before rendering, RV uses an alpha-aware Lanczos filter to prepare a cached
texture at the icon's logical size multiplied by the current display scale.
This avoids aliasing from directly shrinking the large source in GPUI's
bilinear-only texture atlas. Toolbar, picker and drag preview sizes are
prepared separately; moving between display scales selects the matching size.

The remote device's actual App icon takes priority when available. Unknown Apps
use a graphical placeholder instead of a name initial. The original generated
files are retained in the generator output directory.
