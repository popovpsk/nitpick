use gpui::{AssetSource, SharedString};
use rust_embed::RustEmbed;
use std::borrow::Cow;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*"]
struct OwnIcons;

gpui_kit_assets::icon_assets!(
    ExtraIcons,
    [
        GitBranch,
        GitCompare,
        GitPullRequest,
        GitMerge,
        FolderGit2,
        ChevronsDownUp,
        ChevronsUpDown,
        MessageSquare,
        MessageSquarePlus,
        MessageSquareText,
        ClipboardCheck,
        GitPullRequestDraft,
        Columns2,
        Rows2,
        CircleDot,
        FilePlus,
        FileMinus,
        FileDiff,
        ArrowDownToLine,
        TextWrap,
        SquareCheck,
        Send,
        Reply,
        ListTree,
        CircleCheckBig,
        Undo2,
        Space,
        CheckCheck,
        FilePen,
        EyeOff,
        Eye,
        CircleX,
        GitCommitHorizontal,
        RotateCcw,
        FileClock,
        ArrowUpRight,
        ArrowDownUp,
        Pencil,
        Trash,
        ThumbsUp,
        Square,
        SquareArrowOutUpRight,
    ]
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        if let Some(file) = OwnIcons::get(path) {
            return Ok(Some(file.data));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        match gpui_kit_assets::Assets.load(path) {
            Ok(Some(bytes)) => Ok(Some(bytes)),
            _ => Err(anyhow::anyhow!("asset not found: {path}")),
        }
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut paths: Vec<SharedString> = OwnIcons::iter()
            .filter(|p| p.starts_with(path))
            .map(|p| p.into())
            .collect();
        paths.extend(ExtraIcons.list(path)?);
        paths.extend(gpui_kit_assets::Assets.list(path).unwrap_or_default());
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;
