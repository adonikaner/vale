//! The toolbar icons. Two files draw them: [`super::viewbar`] for the bottom
//! toolbar's view toggles, and [`super::rail`] for the side and top toolbars'
//! tool tiles, the workspace control and the two pointer buttons.
//!
//! ## No icons are shipped yet
//!
//! [`EMBEDDED`] is empty, so every button draws its text label: a view toggle
//! its short label (`FOG`, `WMO`), a tool tile its name. A button with no icon
//! is a complete button, so nothing here decides whether a toggle or a tool
//! exists.
//!
//! ## Adding an icon
//!
//! 1. Add a PNG to the editor crate. 32x32 is the size the toolbars were
//!    laid out for; it is drawn at 24 points.
//! 2. Add a line to [`EMBEDDED`] naming the button it belongs to, with the
//!    file's bytes from `include_bytes!` (the path is relative to this file):
//!
//!    ```text
//!    ("terrain", include_bytes!("<path to terrain.png>")),
//!    ```
//!
//! The name is the button's own, compared without case: a rail subject's name
//! (`Terrain`, `Doodads`, `Spells`) or a view toggle's switch label (`fog`,
//! `doodads`, `collision`, `navmesh`). `every_embedded_name_is_a_button`
//! fails the build for a name that matches no button. A tool tile grows to
//! fit an icon over its name as soon as any icon is present.
//!
//! The files are compiled into the binary rather than read from a folder, so
//! the editor has the same toolbars whether it is run from the repository or
//! from an install folder.

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiTextureHandle};

/// The icons compiled into the editor: `(button name, PNG bytes)`. See the
/// module comment for how to add one.
const EMBEDDED: &[(&str, &[u8])] = &[];

/// The textures the toolbars draw their buttons with.
#[derive(Resource, Default)]
pub struct Icons {
    /// Keyed by the lowercase button name.
    ready: HashMap<String, egui::TextureId>,
    /// Set after the one attempt to register [`EMBEDDED`], so the table is
    /// decoded once rather than every frame.
    looked: bool,
}

impl Icons {
    /// The texture id egui draws this button's icon by, if it has one.
    pub fn get(&self, name: &str) -> Option<egui::TextureId> {
        self.ready.get(&name.to_lowercase()).copied()
    }

    /// Whether any icon is registered. The rail uses it to choose the height
    /// of a tool tile.
    pub fn any(&self) -> bool {
        !self.ready.is_empty()
    }
}

pub struct IconPlugin;

impl Plugin for IconPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Icons>().add_systems(Update, load);
    }
}

/// Decode every icon in [`EMBEDDED`] and register it with egui, once.
///
/// In `Update` rather than `Startup` because it needs an egui context to
/// register a texture with, and bevy_egui creates that on the first pass.
fn load(mut icons: ResMut<Icons>, mut images: ResMut<Assets<Image>>, mut contexts: EguiContexts) {
    if icons.looked {
        return;
    }
    // `looked` is set after the context check, not before. `ctx_mut` fails
    // until bevy_egui has made a context, and marking the attempt done on a
    // frame where it could not have worked would leave the toolbars without
    // icons for the whole session.
    if contexts.ctx_mut().is_err() {
        return;
    }
    icons.looked = true;

    for (name, bytes) in EMBEDDED {
        let decoded = Image::from_buffer(
            bytes,
            ImageType::Extension("png"),
            CompressedImageFormats::NONE,
            // The files are art rather than data, so they are sRGB, the same
            // answer `thumbnails` gives for a tileset.
            true,
            ImageSampler::Default,
            RenderAssetUsages::all(),
        );
        let Ok(decoded) = decoded else {
            warn!("toolbar icon {name}: not a readable PNG, drawing its label instead");
            continue;
        };
        // A strong handle, so egui owns the asset for as long as the id names
        // it. See `thumbnails::decode`, where a weak one draws whatever the
        // atlas holds at that slot now.
        let handle = images.add(decoded);
        let id = contexts.add_image(EguiTextureHandle::Strong(handle));
        icons.ready.insert(name.to_lowercase(), id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An editor with no icons answers `None` for every button, which each
    /// toolbar draws as a label.
    #[test]
    fn an_empty_set_answers_nothing_rather_than_failing() {
        let icons = Icons::default();
        assert!(!icons.any());
        for name in super::super::viewbar::names() {
            assert_eq!(icons.get(name), None);
        }
    }

    /// Every name in [`EMBEDDED`] is a button on one of the toolbars. The
    /// lookup joins by name, so a misspelt entry would be decoded and never
    /// drawn, with no other error.
    #[test]
    fn every_embedded_name_is_a_button() {
        let buttons: Vec<String> = super::super::viewbar::names()
            .into_iter()
            .chain(super::super::rail::names())
            .map(str::to_lowercase)
            .collect();
        for (name, _) in EMBEDDED {
            assert!(
                buttons.contains(&name.to_lowercase()),
                "{name} is not the name of any toolbar button"
            );
        }
    }
}
