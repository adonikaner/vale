//! The flags that move pictures in and out of the project: a tile's height map
//! as a PNG, a PNG imported onto a tile, and a texture imported as a BLP.

use super::Arity::*;
use super::{text, Flag};
use crate::tools::Tool;

/// The scripted imports and exports.
#[derive(Debug, Clone, Default)]
pub struct Import {
    /// `--export-heights <file>`: write the height map of the tile under the
    /// camera to that PNG once the tile is open. `--import-image <file>`:
    /// open the map window's import dialog for that PNG on the camera's
    /// tile, which a scripted run cannot reach through the file dialog. See
    /// `crate::import::images`.
    pub export_heights: Option<std::path::PathBuf>,
    pub import_image: Option<std::path::PathBuf>,
    /// `--import-confirm`: with `--import-image`, press the dialog's Import.
    pub import_confirm: bool,
    /// `--import-texture <picture> <archive path>`: import a PNG, TGA or BLP
    /// picture as a BLP at that path, as the Import Texture dialog would with
    /// its defaults for what the path's folder is for (a ground texture under
    /// `Tileset\` is written with its `_s` file), and open the Assets workspace
    /// on it. It implies the Assets workspace as the tool. See
    /// `crate::import::assets`.
    pub import_texture: Option<(String, String)>,
    /// `--import-dialog <picture> [purpose]`: open the Import Texture dialog on
    /// a picture without writing anything, for `ground`, `icon` or `other` (the
    /// default). The dialog is otherwise behind the system's file dialog, which
    /// a scripted `--shot` cannot answer.
    pub import_dialog: Option<(String, String)>,
}

pub(super) const FLAGS: &[Flag] = &[
    Flag::new("--export-heights", One, |a, v| a.import.export_heights = text(v).map(Into::into)),
    Flag::new("--import-image", One, |a, v| a.import.import_image = text(v).map(Into::into)),
    Flag::new("--import-confirm", Switch, |a, _| a.import.import_confirm = true),
    Flag::new("--import-texture", Two, |a, v| {
        if let [picture, vpath] = v {
            a.import.import_texture = Some((picture.clone(), vpath.clone()));
        }
        a.view.tool = Some(Tool::Assets);
    }),
    // The purpose is optional, so a following flag is not eaten.
    Flag::new("--import-dialog", OneThenOptional, |a, v| {
        if let Some(picture) = v.first() {
            let purpose = v.get(1).cloned().unwrap_or_default();
            a.import.import_dialog = Some((picture.clone(), purpose));
        }
        a.view.tool = Some(Tool::Assets);
    }),
];
