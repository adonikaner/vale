//! The flags that move pictures, models and sounds in and out of the
//! project: a tile's height map as a PNG, a PNG imported onto a tile, a
//! texture imported as a BLP, a glTF model imported as an M2, and a WAV or
//! MP3 imported as a sound file with the `SoundEntries` row that plays it;
//! the Game Files part opened on an archive file or folder; and a world map
//! picture imported or drawn from the minimap.

use super::Arity::*;
use super::{text, Flag};
use crate::tools::Tool;

/// The scripted imports and exports.
#[derive(Debug, Clone, Default)]
pub(crate) struct Import {
    /// `--export-heights <file>`: write the height map of the tile under the
    /// camera to that PNG once the tile is open. `--import-image <file>`:
    /// open the map window's import dialog for that PNG on the camera's
    /// tile, which a scripted run cannot reach through the file dialog. See
    /// `crate::import::images`.
    pub(crate) export_heights: Option<std::path::PathBuf>,
    pub(crate) import_image: Option<std::path::PathBuf>,
    /// `--import-confirm`: with `--import-image`, press the dialog's Import.
    pub(crate) import_confirm: bool,
    /// `--import-texture <picture> <archive path>`: import a PNG, TGA or BLP
    /// picture as a BLP at that path, as the Import Texture dialog would with
    /// its defaults for what the path's folder is for (a ground texture under
    /// `Tileset\` is written with its `_s` file), and open the Assets workspace
    /// on it. It implies the Assets workspace as the tool. See
    /// `crate::import::assets`.
    pub(crate) import_texture: Option<(String, String)>,
    /// `--import-dialog <picture> [purpose]`: open the Import Texture dialog on
    /// a picture without writing anything, for `ground`, `icon` or `other` (the
    /// default). The dialog is otherwise behind the system's file dialog, which
    /// a scripted `--shot` cannot answer. Given a `.glb` or `.gltf`, it opens
    /// the Import Model dialog instead, for `creature`, `doodad` or `other`;
    /// given a `.wav` or `.mp3`, the Import Sound dialog, for `creature`,
    /// `spell`, `object`, `interface`, `music`, `ambience` or `other`.
    pub(crate) import_dialog: Option<(String, String)>,
    /// `--import-model <file> <archive path>`: import a glTF model as an M2 at
    /// that path, with the Import Model dialog's defaults for what the path's
    /// folder is for, and open the Assets workspace on it. See
    /// `crate::import::models`.
    pub(crate) import_model: Option<(String, String)>,
    /// `--import-sound <file> <archive path>`: write a WAV or MP3 at that
    /// path with the Import Sound dialog's defaults for the purpose the path's
    /// folder implies, add a `SoundEntries` row whose `Directory` and
    /// `File 1` hold it, and open the Assets workspace on it. See
    /// `crate::import::sounds`.
    pub(crate) import_sound: Option<(String, String)>,
    /// `--game-file <archive path>`: open the Assets workspace's Game Files
    /// part on that file, or on that folder when the path ends in `\`. See
    /// `crate::import::files`.
    pub(crate) game_file: Option<String>,
    /// `--world-map <directory>`: open the Zones workspace's World Map part
    /// on the map whose pieces are in that directory: `World`, or a
    /// `WorldMapArea` row's. With `--world-map-image <picture>`, read that
    /// picture as the map's new picture, which the file dialog would
    /// otherwise ask for; with `--world-map-draw`, draw the new picture from
    /// the minimap; with `--world-map-write`, write the new picture once it
    /// is ready. See `crate::import::worldmap`.
    pub(crate) world_map: Option<String>,
    pub(crate) world_map_image: Option<std::path::PathBuf>,
    pub(crate) world_map_draw: bool,
    pub(crate) world_map_write: bool,
    /// `--world-map-sectors <n,n,...>`: with `--world-map`, choose those
    /// sectors (pieces 1 to 12) before the picture is read or drawn.
    pub(crate) world_map_sectors: Vec<u32>,
    /// `--world-map-explored`: show the explored areas rather than the
    /// sectors.
    pub(crate) world_map_explored: bool,
    /// `--world-map-places <id>`: on a continent or the whole world, show the
    /// Zones or Continents view, with that zone (`WorldMapArea` row id) or
    /// continent (map id) chosen; 0 chooses none.
    pub(crate) world_map_places: Option<u32>,
    /// `--world-map-new <zone|continent|single>:<id>:<directory>`: make a
    /// map as the New map form does, for an `AreaTable` zone (`zone`) or a
    /// `Map.dbc` map (`continent`, `single`), choose it, and fit its
    /// rectangle once its tiles are read. It implies the World Map part.
    pub(crate) world_map_new: Option<String>,
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
    Flag::new("--import-model", Two, |a, v| {
        if let [model, vpath] = v {
            a.import.import_model = Some((model.clone(), vpath.clone()));
        }
        a.view.tool = Some(Tool::Assets);
    }),
    Flag::new("--import-sound", Two, |a, v| {
        if let [sound, vpath] = v {
            a.import.import_sound = Some((sound.clone(), vpath.clone()));
        }
        a.view.tool = Some(Tool::Assets);
    }),
    Flag::new("--game-file", One, |a, v| {
        a.import.game_file = text(v).map(Into::into);
        a.view.tool = Some(Tool::GameFiles);
    }),
    Flag::new("--world-map", One, |a, v| {
        a.import.world_map = text(v);
        a.view.tool = Some(Tool::WorldMap);
    }),
    Flag::new("--world-map-image", One, |a, v| a.import.world_map_image = text(v).map(Into::into)),
    Flag::new("--world-map-draw", Switch, |a, _| a.import.world_map_draw = true),
    Flag::new("--world-map-write", Switch, |a, _| a.import.world_map_write = true),
    Flag::new("--world-map-sectors", One, |a, v| {
        a.import.world_map_sectors =
            text(v).map(|list| list.split(',').filter_map(|n| n.trim().parse().ok()).collect()).unwrap_or_default();
    }),
    Flag::new("--world-map-explored", Switch, |a, _| a.import.world_map_explored = true),
    Flag::new("--world-map-places", One, |a, v| a.import.world_map_places = text(v).and_then(|id| id.trim().parse().ok())),
    Flag::new("--world-map-new", One, |a, v| {
        a.import.world_map_new = text(v);
        a.view.tool = Some(Tool::WorldMap);
    }),
    // The purpose is optional, so a flag that follows the picture is not taken
    // as the purpose.
    Flag::new("--import-dialog", OneThenOptional, |a, v| {
        if let Some(picture) = v.first() {
            let purpose = v.get(1).cloned().unwrap_or_default();
            a.import.import_dialog = Some((picture.clone(), purpose));
        }
        a.view.tool = Some(Tool::Assets);
    }),
];
