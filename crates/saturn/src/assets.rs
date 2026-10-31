//! The app's assets: the few Lucide icons Saturn uses beyond gpui-kit's default set (ISC, see
//! `assets/icons/LICENSE-LUCIDE`), falling back to gpui-kit's bundled assets for the rest, and the
//! sharp version of every icon, made from it as it loads.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

const ICONS: &[(&str, &[u8])] = &[
    ("icons/panel-top.svg", include_bytes!("../assets/icons/panel-top.svg")),
    ("icons/columns-2.svg", include_bytes!("../assets/icons/columns-2.svg")),
    ("icons/rows-2.svg", include_bytes!("../assets/icons/rows-2.svg")),
    ("icons/x.svg", include_bytes!("../assets/icons/x.svg")),
    ("icons/trash.svg", include_bytes!("../assets/icons/trash.svg")),
    ("icons/calendar-clock.svg", include_bytes!("../assets/icons/calendar-clock.svg")),
    ("icons/text-search.svg", include_bytes!("../assets/icons/text-search.svg")),
    ("icons/git-fork.svg", include_bytes!("../assets/icons/git-fork.svg")),
    ("icons/send.svg", include_bytes!("../assets/icons/send.svg")),
    ("icons/users.svg", include_bytes!("../assets/icons/users.svg")),
    ("icons/ellipsis.svg", include_bytes!("../assets/icons/ellipsis.svg")),
    ("icons/chevron-down.svg", include_bytes!("../assets/icons/chevron-down.svg")),
    ("icons/list-checks.svg", include_bytes!("../assets/icons/list-checks.svg")),
    ("icons/folder-git.svg", include_bytes!("../assets/icons/folder-git.svg")),
    ("icons/git-compare.svg", include_bytes!("../assets/icons/git-compare.svg")),
    ("icons/locate.svg", include_bytes!("../assets/icons/locate.svg")),
    ("icons/git-commit-horizontal.svg", include_bytes!("../assets/icons/git-commit-horizontal.svg")),
    ("icons/tag.svg", include_bytes!("../assets/icons/tag.svg")),
    ("icons/undo-2.svg", include_bytes!("../assets/icons/undo-2.svg")),
    ("icons/redo-2.svg", include_bytes!("../assets/icons/redo-2.svg")),
    ("icons/moon.svg", include_bytes!("../assets/icons/moon.svg")),
    ("icons/circle-check.svg", include_bytes!("../assets/icons/circle-check.svg")),
    ("icons/circle-dot.svg", include_bytes!("../assets/icons/circle-dot.svg")),
    ("icons/wrench.svg", include_bytes!("../assets/icons/wrench.svg")),
    ("icons/message-square.svg", include_bytes!("../assets/icons/message-square.svg")),
    ("icons/locate-fixed.svg", include_bytes!("../assets/icons/locate-fixed.svg")),
    ("icons/square-dot.svg", include_bytes!("../assets/icons/square-dot.svg")),
    ("icons/square-plus.svg", include_bytes!("../assets/icons/square-plus.svg")),
    ("icons/square-minus.svg", include_bytes!("../assets/icons/square-minus.svg")),
    ("icons/square-arrow-right.svg", include_bytes!("../assets/icons/square-arrow-right.svg")),
    ("icons/square-x.svg", include_bytes!("../assets/icons/square-x.svg")),
    ("icons/git-branch.svg", include_bytes!("../assets/icons/git-branch.svg")),
    ("icons/git-pull-request.svg", include_bytes!("../assets/icons/git-pull-request.svg")),
    ("icons/git-pull-request-draft.svg", include_bytes!("../assets/icons/git-pull-request-draft.svg")),
    ("icons/git-pull-request-closed.svg", include_bytes!("../assets/icons/git-pull-request-closed.svg")),
    ("icons/git-merge.svg", include_bytes!("../assets/icons/git-merge.svg")),
    ("icons/bot.svg", include_bytes!("../assets/icons/bot.svg")),
    ("icons/chart-no-axes-column.svg", include_bytes!("../assets/icons/chart-no-axes-column.svg")),
    ("icons/database.svg", include_bytes!("../assets/icons/database.svg")),
    ("icons/key-round.svg", include_bytes!("../assets/icons/key-round.svg")),
    ("icons/link-2.svg", include_bytes!("../assets/icons/link-2.svg")),
    ("icons/zoom-in.svg", include_bytes!("../assets/icons/zoom-in.svg")),
    ("icons/zoom-out.svg", include_bytes!("../assets/icons/zoom-out.svg")),
    ("icons/scan.svg", include_bytes!("../assets/icons/scan.svg")),
    ("icons/maximize-2.svg", include_bytes!("../assets/icons/maximize-2.svg")),
    ("icons/minimize-2.svg", include_bytes!("../assets/icons/minimize-2.svg")),
    ("icons/waypoints.svg", include_bytes!("../assets/icons/waypoints.svg")),
    ("icons/file-code.svg", include_bytes!("../assets/icons/file-code.svg")),
    ("icons/arrow-left.svg", include_bytes!("../assets/icons/arrow-left.svg")),
    ("icons/search.svg", include_bytes!("../assets/icons/search.svg")),
    ("icons/refresh-cw.svg", include_bytes!("../assets/icons/refresh-cw.svg")),
    ("icons/sun-moon.svg", include_bytes!("../assets/icons/sun-moon.svg")),
    ("icons/key-square.svg", include_bytes!("../assets/icons/key-square.svg")),
    ("icons/server.svg", include_bytes!("../assets/icons/server.svg")),
    ("icons/square-check.svg", include_bytes!("../assets/icons/square-check.svg")),
    ("icons/scan-search.svg", include_bytes!("../assets/icons/scan-search.svg")),
    ("icons/keyboard.svg", include_bytes!("../assets/icons/keyboard.svg")),
    ("icons/rotate-ccw.svg", include_bytes!("../assets/icons/rotate-ccw.svg")),
    ("icons/sprout.svg", include_bytes!("../assets/icons/sprout.svg")),
    ("icons/camera.svg", include_bytes!("../assets/icons/camera.svg")),
    ("icons/box.svg", include_bytes!("../assets/icons/box.svg")),
    ("icons/workflow.svg", include_bytes!("../assets/icons/workflow.svg")),
    ("icons/list-filter.svg", include_bytes!("../assets/icons/list-filter.svg")),
    ("icons/text-align-start.svg", include_bytes!("../assets/icons/text-align-start.svg")),
    ("icons/square-terminal.svg", include_bytes!("../assets/icons/square-terminal.svg")),
    ("icons/braces.svg", include_bytes!("../assets/icons/braces.svg")),
    ("icons/book-open.svg", include_bytes!("../assets/icons/book-open.svg")),
    ("icons/lock.svg", include_bytes!("../assets/icons/lock.svg")),
    ("icons/settings-2.svg", include_bytes!("../assets/icons/settings-2.svg")),
    ("icons/image.svg", include_bytes!("../assets/icons/image.svg")),
    ("icons/file-terminal.svg", include_bytes!("../assets/icons/file-terminal.svg")),
    ("icons/scale.svg", include_bytes!("../assets/icons/scale.svg")),
    ("icons/container.svg", include_bytes!("../assets/icons/container.svg")),
    ("icons/package.svg", include_bytes!("../assets/icons/package.svg")),
    ("icons/palette.svg", include_bytes!("../assets/icons/palette.svg")),
    ("icons/file-text.svg", include_bytes!("../assets/icons/file-text.svg")),
    ("icons/file-archive.svg", include_bytes!("../assets/icons/file-archive.svg")),
    ("icons/shield.svg", include_bytes!("../assets/icons/shield.svg")),
    ("icons/globe.svg", include_bytes!("../assets/icons/globe.svg")),
    ("icons/table-2.svg", include_bytes!("../assets/icons/table-2.svg")),
    ("icons/external-link.svg", include_bytes!("../assets/icons/external-link.svg")),
    ("icons/panel-right.svg", include_bytes!("../assets/icons/panel-right.svg")),
    ("icons/circle-x.svg", include_bytes!("../assets/icons/circle-x.svg")),
    ("icons/triangle-alert.svg", include_bytes!("../assets/icons/triangle-alert.svg")),
    ("icons/file-plus.svg", include_bytes!("../assets/icons/file-plus.svg")),
    ("icons/folder-plus.svg", include_bytes!("../assets/icons/folder-plus.svg")),
    ("icons/pencil.svg", include_bytes!("../assets/icons/pencil.svg")),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // `icons-sharp/x.svg` is `icons/x.svg` with its corners squared, for a theme with sharp
        // icons: one set of drawings, two looks.
        if let Some(name) = path.strip_prefix("icons-sharp/") {
            let Some(round) = self.load(&format!("icons/{name}"))? else { return Ok(None) };
            let sharp = saturn_core::sharp_icon::sharpen(&String::from_utf8_lossy(&round));
            return Ok(Some(Cow::Owned(sharp.into_bytes())));
        }
        if let Some((_, bytes)) = ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names = gpui_kit::assets::Assets.list(path)?;
        names
            .extend(ICONS.iter().filter(|(name, _)| name.starts_with(path)).map(|(name, _)| SharedString::from(*name)));
        Ok(names)
    }
}
