//! The Assets workspace's state: the files the project adds to the archives,
//! and a picture turned into a BLP texture.
//!
//! ## What an asset is here
//!
//! Every file of the project that the publish packs and that is not a tile or
//! a table: an imported texture, a model baked in the attachment lab, a
//! minimap picture, a flight map. They are files in the project folder at
//! their archive path. A read through the overlay finds them at once, and
//! [`EditSession::listed`] adds them to every picker built from the archives'
//! listing, which is all that registering one takes. The publish packs them
//! with the rest of the project.
//!
//! ## Importing a texture
//!
//! A PNG, TGA or BLP picture is read to RGBA pixels and written as a BLP with
//! [`vale_assets::world::blp::encode`], in one of the three DXT shapes the
//! shipped archives use and with its mip chain. Both sides must be powers of
//! two, as every shipped texture's are; a picture whose sides are not is
//! resized up to the next powers of two when the dialog asks for it. The file
//! is saved through [`EditSession::save_bytes`], so it is undone by removing
//! it, not by `Ctrl+Z`.
//!
//! ## The dialog asks what a texture is for, not where it goes
//!
//! A [`Purpose`] decides the folder, the size, the compression and the files
//! written, so a person chooses Ground texture rather than typing `Tileset\`.
//! The folder is shown and, for Other, typed; for a ground texture only the
//! folder under `Tileset\` is typed.
//!
//! A ground texture is two files, as the game ships its ground: `<name>.blp`
//! in DXT1 with no alpha, and `<name>_s.blp` in DXT3, the same picture with
//! the gloss mask in its alpha. The 1.12 client opens the `_s` file in place
//! of the plain one whenever its `specular` setting is on, and has no fallback
//! to the plain file, so a ground texture without one is drawn as a missing
//! texture. Every shipped tileset has both, and none is larger than 256x256;
//! every shipped icon is 64x64. The dialog scales a larger picture down to its
//! purpose's size unless asked not to.
//!
//! An import can be asked for by a form field (a creature display's skin):
//! the dialog then starts in that field's folder, and the field is set to the
//! imported texture's name once it is written. See [`FieldTarget`]. The
//! texture tool's panel asks for a ground texture, and the brush takes it once
//! it is written ([`Assets::imported`]).
//!
//! The file dialog is the system's own and blocks while it is open, so it is
//! run by [`answer`], outside the panel, as the tile tool runs its picture
//! dialogs.

use crate::session::EditSession;
use vale_assets::world::blp::{self, Encoding};
use vale_edit::manifest::Kind;
use vale_edit::project::Project;
use bevy::prelude::*;
use std::path::{Path, PathBuf};

/// Where a ground texture goes, and the folder under it the dialog starts in.
pub const GROUND_ROOT: &str = "Tileset\\";
pub const GROUND_FOLDER: &str = "Tileset\\Custom\\";
/// Where an icon goes.
pub const ICON_FOLDER: &str = "Interface\\Icons\\";
/// Where any other texture starts.
pub const OTHER_FOLDER: &str = "Textures\\Custom\\";

/// What an imported texture is for. It decides where the texture is written,
/// how large it is and which files are written. See the module comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Ground, painted with the texture tool: under `Tileset\`, as a pair with
    /// its `_s` gloss file, at most 256x256.
    Ground,
    /// A bag, spell or button icon: in `Interface\Icons\`, 64x64.
    Icon,
    /// The texture a form field names, in the folder the field's file is found
    /// in. Only a field asks for this; see [`FieldTarget`].
    Field,
    /// Any other texture, in a folder typed in the dialog.
    Other,
}

impl Purpose {
    /// The purposes the dialog offers a choice between. A field's import is
    /// its field's.
    pub const OFFERED: [Purpose; 3] = [Purpose::Ground, Purpose::Icon, Purpose::Other];

    pub fn name(self) -> &'static str {
        match self {
            Purpose::Ground => "Ground texture",
            Purpose::Icon => "Icon",
            Purpose::Field => "For the field",
            Purpose::Other => "Other",
        }
    }

    /// One line on what it is for and what is written.
    pub fn hint(self) -> &'static str {
        match self {
            Purpose::Ground => "Painted on the terrain with the texture tool. Written as the \
                                texture and its _s gloss file, as the game's ground is.",
            Purpose::Icon => "An item, spell or button icon in Interface\\Icons\\, 64x64 as \
                              every shipped icon is.",
            Purpose::Field => "Named by the field that asked, in the folder it is read from.",
            Purpose::Other => "Any other texture, in the folder below: a model's skin, a \
                               loading screen, a particle.",
        }
    }

    /// The side the purpose's shipped textures are at most, which an import
    /// is scaled down to unless the dialog says otherwise.
    pub fn largest(self) -> Option<u32> {
        match self {
            Purpose::Ground => Some(GROUND_SIDE),
            Purpose::Icon => Some(ICON_SIDE),
            Purpose::Field | Purpose::Other => None,
        }
    }

    /// What a texture at `vpath` is for, from its folder.
    pub fn of_path(vpath: &str) -> Purpose {
        let lower = vpath.replace('/', "\\").to_ascii_lowercase();
        if lower.starts_with("tileset\\") {
            Purpose::Ground
        } else if lower.starts_with("interface\\icons\\") {
            Purpose::Icon
        } else {
            Purpose::Other
        }
    }
}

/// A form field an import was asked for by, which is set to the imported
/// texture's name once it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldTarget {
    pub table: String,
    pub record: usize,
    pub field: usize,
    /// The end of the file name the field leaves off, or nothing. A body
    /// texture's field names `Cloth_A_01Black_Sleeve_AU` and the file is
    /// `Cloth_A_01Black_Sleeve_AU_U.blp`, the `_U` for either body.
    pub suffix: &'static str,
}

impl FieldTarget {
    /// The file name an import for this field is written under: `name` with
    /// [`Self::suffix`], unless it already ends in one of the three a body
    /// texture can.
    pub fn file_name(&self, name: &str) -> String {
        let lower = name.to_ascii_lowercase();
        let suffixed = ["_u", "_m", "_f"].iter().any(|end| lower.ends_with(end));
        match self.suffix.is_empty() || suffixed {
            true => name.to_string(),
            false => format!("{name}{}", self.suffix),
        }
    }

    /// What the field is set to for a texture written as `file`: the name
    /// without its suffix, for a field that leaves one off.
    pub fn field_text(&self, file: &str) -> String {
        if self.suffix.is_empty() {
            return file.to_string();
        }
        let cut = file.len().saturating_sub(2);
        let lower = file.to_ascii_lowercase();
        match ["_u", "_m", "_f"].iter().any(|end| lower.ends_with(end)) {
            true => file[..cut].to_string(),
            false => file.to_string(),
        }
    }
}

/// The largest side of any ground texture the archives ship.
pub const GROUND_SIDE: u32 = 256;
/// The side of every icon the archives ship.
pub const ICON_SIDE: u32 = 64;

/// What a ground texture's `_s` file carries in its alpha, which the client
/// reads as the gloss mask: where the sun's highlight lies on the ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gloss {
    /// No highlight: the ground is drawn matte.
    Matte,
    /// The picture's own alpha.
    FromAlpha,
}

/// How the dialog chooses the compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// [`Encoding::for_pixels`]: the least that keeps the picture's alpha.
    Auto,
    Fixed(Encoding),
}

/// The open Import Texture dialog.
#[derive(Debug, Clone)]
pub struct Import {
    /// The picture on disk.
    pub source: PathBuf,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// What it is for. See [`Purpose`].
    pub purpose: Purpose,
    /// The archive folder it is written into, ending in a backslash. Under
    /// `Tileset\` for a ground texture and `Interface\Icons\` for an icon;
    /// [`Self::set_purpose`] keeps it so.
    pub folder: String,
    /// The file name, without `.blp`.
    pub name: String,
    pub choice: Choice,
    pub mips: bool,
    /// Resize to the next powers of two when a side is not one.
    pub resize: bool,
    /// For a ground texture, what its `_s` file's alpha is.
    pub gloss: Gloss,
    /// Scale it down to its purpose's largest side ([`Purpose::largest`]).
    pub shrink: bool,
    pub for_field: Option<FieldTarget>,
}

impl Import {
    /// The dialog's starting choices for a picture: the compression chosen
    /// from its alpha, mipmaps, a resize where a side is not a power of two,
    /// and for a ground texture, the gloss from its alpha where it has one and
    /// the scale-down.
    pub fn new(
        source: PathBuf,
        (width, height, rgba): (u32, u32, Vec<u8>),
        folder: String,
        name: String,
        for_field: Option<FieldTarget>,
    ) -> Import {
        let alpha = rgba.chunks_exact(4).any(|pixel| pixel[3] != 255);
        let purpose = match for_field {
            Some(_) => Purpose::Field,
            None => Purpose::of_path(&folder),
        };
        Import {
            purpose,
            source,
            width,
            height,
            resize: !width.is_power_of_two() || !height.is_power_of_two(),
            rgba,
            folder,
            name,
            choice: Choice::Auto,
            mips: true,
            gloss: match alpha {
                true => Gloss::FromAlpha,
                false => Gloss::Matte,
            },
            shrink: true,
            for_field,
        }
    }

    /// Whether it is a ground texture. See the module comment.
    pub fn ground(&self) -> bool {
        self.purpose == Purpose::Ground
    }

    /// Change what it is for, and the folder with it: a ground texture keeps
    /// a folder under `Tileset\` it already has, an icon goes to
    /// `Interface\Icons\`, and Other leaves either of those for its own
    /// folder.
    pub fn set_purpose(&mut self, purpose: Purpose) {
        let now = Purpose::of_path(&folder_of(&self.folder));
        self.folder = match purpose {
            Purpose::Ground if now == Purpose::Ground => folder_of(&self.folder),
            Purpose::Ground => GROUND_FOLDER.to_string(),
            Purpose::Icon => ICON_FOLDER.to_string(),
            Purpose::Other if now != Purpose::Other => OTHER_FOLDER.to_string(),
            Purpose::Other | Purpose::Field => folder_of(&self.folder),
        };
        self.purpose = purpose;
    }

    /// The size the texture is written at.
    pub fn written_size(&self) -> (u32, u32) {
        let (width, height) = match self.resize {
            true => (self.width.next_power_of_two(), self.height.next_power_of_two()),
            false => (self.width, self.height),
        };
        match (self.shrink, self.purpose.largest()) {
            (true, Some(side)) => (width.min(side), height.min(side)),
            _ => (width, height),
        }
    }

    /// The compression the texture is written with. A ground texture's plain
    /// file is DXT1, as every shipped one is; its `_s` file is DXT3.
    pub fn encoding(&self) -> Encoding {
        if self.ground() {
            return Encoding::Dxt1;
        }
        match self.choice {
            Choice::Auto => Encoding::for_pixels(&self.rgba),
            Choice::Fixed(encoding) => encoding,
        }
    }

    /// The archive path of a ground texture's `_s` file, or `None` for any
    /// other texture.
    pub fn gloss_vpath(&self) -> Option<String> {
        self.ground()
            .then(|| format!("{}{}_s.blp", folder_of(&self.folder), self.name.trim()))
    }

    /// The archive path it is written at.
    pub fn vpath(&self) -> String {
        format!("{}{}.blp", folder_of(&self.folder), self.name.trim())
    }

    /// Why it cannot be written as it stands, or nothing.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let name = self.name.trim();
        if name.is_empty() {
            out.push("The name is empty.".to_string());
        }
        if name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            out.push("The name may not contain \\ / : * ? \" < > |.".to_string());
        }
        if folder_of(&self.folder).is_empty() {
            out.push("The folder is empty: a texture at the archives' root is found by nothing.".to_string());
        }
        if !Project::is_game_data(&self.vpath()) {
            out.push("The folder is one of the project's own (sql, server, publish, editor), which is not published.".to_string());
        }
        let (width, height) = self.written_size();
        if !width.is_power_of_two() || !height.is_power_of_two() {
            out.push(format!(
                "{}x{} is not a power of two on each side. Resize, or import a picture that is.",
                self.width, self.height
            ));
        }
        if width > blp::MAX_SIDE || height > blp::MAX_SIDE {
            out.push(format!("{width}x{height} is larger than {0}x{0}.", blp::MAX_SIDE));
        }
        if self.ground() && !folder_of(&self.folder).to_ascii_lowercase().starts_with("tileset\\") {
            out.push("A ground texture's folder must be under Tileset\\.".to_string());
        }
        if self.ground() && name.to_ascii_lowercase().ends_with("_s") {
            out.push(
                "A ground texture's name may not end in _s: the client opens <name>_s.blp in \
                 its place, and would look for this one's as <name>_s_s.blp."
                    .to_string(),
            );
        }
        out
    }

    /// The files written, each archive path with its bytes: the texture, and
    /// for a ground texture its `_s` file beside it.
    pub fn encode(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        let (width, height) = self.written_size();
        let pixels = match (width, height) == (self.width, self.height) {
            true => self.rgba.clone(),
            false => blp::resize(&self.rgba, self.width, self.height, width, height),
        };
        let encode = |pixels: &[u8], encoding: Encoding| {
            blp::encode(pixels, width, height, encoding, self.mips).map_err(|e| e.to_string())
        };
        let Some(gloss_vpath) = self.gloss_vpath() else {
            return Ok(vec![(self.vpath(), encode(&pixels, self.encoding())?)]);
        };
        let opaque: Vec<u8> = pixels
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect();
        let glossed: Vec<u8> = pixels
            .chunks_exact(4)
            .flat_map(|pixel| {
                let mask = match self.gloss {
                    Gloss::Matte => 0,
                    Gloss::FromAlpha => pixel[3],
                };
                [pixel[0], pixel[1], pixel[2], mask]
            })
            .collect();
        Ok(vec![
            (self.vpath(), encode(&opaque, Encoding::Dxt1)?),
            (gloss_vpath, encode(&glossed, Encoding::Dxt3)?),
        ])
    }
}

/// The `_s` gloss file beside a ground texture at `vpath`, or `None` for a
/// path that is not a ground texture's plain file.
pub fn gloss_of(vpath: &str) -> Option<String> {
    let lower = vpath.to_ascii_lowercase();
    let plain = Purpose::of_path(vpath) == Purpose::Ground
        && lower.ends_with(".blp")
        && !lower.ends_with("_s.blp");
    plain.then(|| format!("{}_s.blp", &vpath[..vpath.len() - 4]))
}

/// The folder part of a typed folder: backslashes, no leading separator, one
/// trailing backslash; empty for nothing typed.
pub fn folder_of(typed: &str) -> String {
    let cleaned = typed.trim().replace('/', "\\");
    let cleaned = cleaned.trim_matches('\\');
    match cleaned.is_empty() {
        true => String::new(),
        false => format!("{cleaned}\\"),
    }
}

/// A picture on disk as RGBA pixels, top row first: a PNG of any colour type
/// at 8 or 16 bits, a TGA, or a BLP's top level.
pub fn read_picture(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let said = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = std::fs::read(path).map_err(|e| said(&e))?;
    match extension.as_str() {
        "png" => read_png(&bytes).map_err(|e| said(&e)),
        "tga" => {
            let tga = vale_assets::world::tga::decode(&bytes).map_err(|e| said(&e))?;
            Ok((tga.width, tga.height, tga.rgba))
        }
        "blp" => {
            let picture = blp::decode(&bytes).map_err(|e| said(&e))?;
            Ok((picture.width, picture.height, picture.rgba))
        }
        other => Err(said(&format!("a .{other} file is not a picture this reads; use PNG, TGA or BLP"))),
    }
}

/// A PNG as RGBA8, whatever its colour type and depth.
fn read_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader.output_buffer_size().ok_or("too large to read")?;
    let mut buffer = vec![0u8; size];
    let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    buffer.truncate(frame.buffer_size());
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("an indexed-colour PNG that could not be expanded".to_string()),
    };
    let rgba: Vec<u8> = buffer
        .chunks_exact(channels)
        .flat_map(|p| match channels {
            1 => [p[0], p[0], p[0], 255],
            2 => [p[0], p[0], p[0], p[1]],
            3 => [p[0], p[1], p[2], 255],
            _ => [p[0], p[1], p[2], p[3]],
        })
        .collect();
    Ok((frame.width, frame.height, rgba))
}

/// One of the project's files the workspace lists.
#[derive(Debug, Clone)]
pub struct Own {
    /// Its archive path, as the project folder spells it.
    pub vpath: String,
    pub kind: Kind,
    pub bytes: u64,
}

/// What the workspace asked for, carried out by [`answer`] after the panel
/// is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Choose a picture with the system's file dialog, then open the import
    /// dialog on it, in `folder`, named `name` (or after the picture), for
    /// `for_field` when a form asked.
    Choose {
        folder: String,
        name: Option<String>,
        for_field: Option<FieldTarget>,
    },
    /// Write the open import.
    Import,
    /// Open the import dialog on one of the project's own BLP files, to write
    /// it again: at its own path for what its folder says it is for, or for
    /// `purpose` (a texture imported elsewhere made a ground texture). A
    /// ground texture without its `_s` file is written as the pair this way.
    Rewrite { vpath: String, purpose: Option<Purpose> },
    /// Remove one of the project's files.
    Remove(String),
}

/// The workspace's state.
#[derive(Resource, Debug, Default)]
pub struct Assets {
    /// The selected file's archive path.
    pub selected: Option<String>,
    /// What the search box holds.
    pub query: String,
    /// The open import dialog.
    pub import: Option<Import>,
    pub ask: Option<Ask>,
    /// A texture imported for a form field, and its name, for the form to
    /// write into the field. Taken by the form.
    pub imported_for: Option<(FieldTarget, String)>,
    /// The texture an import last wrote and what it is for, for the shell:
    /// a ground texture goes on the texture tool's brush. Taken by the shell.
    pub imported: Option<(String, Purpose)>,
    /// The folder the last picture was chosen from, where the file dialog
    /// opens next.
    pub picture_dir: Option<PathBuf>,
    /// The project's asset files and the `EditSession::files_revision` they
    /// were listed at.
    listed: Option<(u64, Vec<Own>)>,
}

impl Assets {
    /// The project's asset files: every game file but its tiles and tables,
    /// in path order, listed again when the project's files change.
    pub fn files(&mut self, session: &EditSession) -> &[Own] {
        let current = self
            .listed
            .as_ref()
            .is_some_and(|(at, _)| *at == session.files_revision);
        if !current {
            let found = session
                .project
                .files()
                .into_iter()
                .filter(|vpath| Project::is_game_data(vpath))
                .map(|vpath| (Kind::of(&vpath), vpath))
                .filter(|(kind, _)| !matches!(kind, Kind::Tile | Kind::Table))
                .map(|(kind, vpath)| Own {
                    bytes: session
                        .project
                        .path_for(&vpath)
                        .and_then(|path| std::fs::metadata(path).ok())
                        .map_or(0, |meta| meta.len()),
                    vpath,
                    kind,
                })
                .collect();
            self.listed = Some((session.files_revision, found));
        }
        self.listed.as_ref().map_or(&[], |(_, found)| found)
    }
}

pub struct AssetsPlugin;

impl Plugin for AssetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Assets>()
            .add_systems(Update, (on_the_command_line, answer).chain());
    }
}

/// `--import-texture <picture> <archive path>`: open the import on the
/// picture with the path's folder and name, and write it, once there is a
/// session. See `crate::args::import::Import::import_texture`.
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut assets: ResMut<Assets>,
    session: Option<Res<EditSession>>,
    mut done: Local<bool>,
) {
    if *done || session.is_none() {
        return;
    }
    *done = true;
    if let Some((picture, purpose)) = args.import.import_dialog.clone() {
        let purpose = match purpose.to_ascii_lowercase().as_str() {
            "ground" => Purpose::Ground,
            "icon" => Purpose::Icon,
            _ => Purpose::Other,
        };
        match read_picture(Path::new(&picture)) {
            Ok(read) => {
                let name = Path::new(&picture)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("Texture")
                    .to_string();
                let mut import = Import::new(PathBuf::from(&picture), read, OTHER_FOLDER.to_string(), name, None);
                import.set_purpose(purpose);
                assets.import = Some(import);
            }
            Err(why) => warn!("--import-dialog: {why}"),
        }
    }
    let Some((picture, vpath)) = args.import.import_texture.clone() else { return };
    let vpath = vpath.replace('/', "\\");
    let (folder, file) = match vpath.rfind('\\') {
        Some(at) => (vpath[..=at].to_string(), vpath[at + 1..].to_string()),
        None => (String::new(), vpath.clone()),
    };
    let name = file
        .strip_suffix(".blp")
        .or_else(|| file.strip_suffix(".BLP"))
        .unwrap_or(&file)
        .to_string();
    match read_picture(Path::new(&picture)) {
        Ok((width, height, rgba)) => {
            info!("--import-texture: {picture} is {width}x{height}");
            assets.import = Some(Import::new(
                PathBuf::from(&picture),
                (width, height, rgba),
                folder,
                name,
                None,
            ));
            assets.ask = Some(Ask::Import);
        }
        Err(why) => warn!("--import-texture: {why}"),
    }
}

/// Carry out what the panel asked for: the file dialog, the write, the
/// removal.
fn answer(
    mut assets: ResMut<Assets>,
    mut session: Option<ResMut<EditSession>>,
    mut thumbnails: ResMut<crate::ui::widgets::thumbnails::Thumbnails>,
) {
    let Some(ask) = assets.ask.take() else { return };
    let Some(session) = session.as_mut() else { return };
    match ask {
        Ask::Choose { folder, name: chosen, for_field } => {
            let mut dialog = rfd::FileDialog::new().add_filter("Picture", &["png", "tga", "blp"]);
            if let Some(dir) = &assets.picture_dir {
                dialog = dialog.set_directory(dir);
            }
            let Some(path) = dialog.pick_file() else { return };
            assets.picture_dir = path.parent().map(Path::to_path_buf);
            match read_picture(&path) {
                Ok((width, height, rgba)) => {
                    let name = chosen.unwrap_or_else(|| {
                        path.file_stem()
                            .and_then(|stem| stem.to_str())
                            .unwrap_or("Texture")
                            .to_string()
                    });
                    let name = match &for_field {
                        Some(target) => target.file_name(&name),
                        None => name,
                    };
                    // The shell draws the dialog under any tool, so the tool
                    // that asked (the workspace, a form, the texture tool) stays.
                    assets.import = Some(Import::new(path, (width, height, rgba), folder, name, for_field));
                }
                Err(why) => session.status = why,
            }
        }
        Ask::Import => {
            let Some(import) = assets.import.take() else { return };
            if let Some(problem) = import.problems().into_iter().next() {
                session.status = problem;
                assets.import = Some(import);
                return;
            }
            let files = match import.encode() {
                Ok(files) => files,
                Err(why) => {
                    session.status = why;
                    assets.import = Some(import);
                    return;
                }
            };
            let vpath = import.vpath();
            let mut saved = true;
            for (path, bytes) in files {
                // A picture already drawn at this path is the file it replaces.
                thumbnails.forget(&path);
                saved &= session.save_bytes(&path, bytes);
            }
            if saved {
                let (width, height) = import.written_size();
                let mips = match import.mips {
                    true => ", with mips",
                    false => "",
                };
                session.status = match import.gloss_vpath() {
                    Some(gloss) => format!(
                        "imported {vpath} (DXT1) and {gloss} (DXT3, its gloss): {width}x{height}{mips}"
                    ),
                    None => format!("imported {vpath}: {width}x{height}, {}{mips}", import.encoding().name()),
                };
                assets.selected = Some(vpath.clone());
                assets.imported = Some((vpath, import.purpose));
                if let Some(target) = import.for_field {
                    let text = target.field_text(import.name.trim());
                    assets.imported_for = Some((target, text));
                }
            }
        }
        Ask::Rewrite { vpath, purpose } => {
            let decoded = session
                .project
                .read(&vpath)
                .ok_or_else(|| format!("{vpath} is not one of the project's files"))
                .and_then(|bytes| blp::decode(&bytes).map_err(|e| format!("{vpath}: {e}")));
            match decoded {
                Ok(picture) => {
                    let (folder, file) = match vpath.rfind('\\') {
                        Some(at) => (vpath[..=at].to_string(), &vpath[at + 1..]),
                        None => (String::new(), vpath.as_str()),
                    };
                    let name = file.get(..file.len().saturating_sub(4)).unwrap_or(file).to_string();
                    let mut import = Import::new(
                        PathBuf::from(&vpath),
                        (picture.width, picture.height, picture.rgba),
                        folder,
                        name,
                        None,
                    );
                    if let Some(purpose) = purpose {
                        import.set_purpose(purpose);
                    }
                    assets.import = Some(import);
                }
                Err(why) => session.status = why,
            }
        }
        Ask::Remove(vpath) => {
            // A ground texture goes with its `_s` file: either alone is a
            // texture the client cannot draw as the other expects.
            let mut gone = vec![vpath.clone()];
            if let Some(gloss) = gloss_of(&vpath) {
                if session.project.has(&gloss) {
                    gone.push(gloss);
                }
            }
            for path in gone {
                thumbnails.forget(&path);
                session.remove_file(&path);
            }
            if assets.selected.as_deref() == Some(vpath.as_str()) {
                assets.selected = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body texture is written with `_U` and its field names it without.
    /// A name that already ends in a body's suffix is kept as it is.
    #[test]
    fn a_body_texture_is_written_with_its_suffix_and_named_without() {
        let target = FieldTarget {
            table: "ItemDisplayInfo".to_string(),
            record: 0,
            field: 17,
            suffix: "_U",
        };
        assert_eq!(target.file_name("Robe_Chest_TU"), "Robe_Chest_TU_U");
        assert_eq!(target.file_name("Robe_Chest_TU_F"), "Robe_Chest_TU_F");
        assert_eq!(target.field_text("Robe_Chest_TU_U"), "Robe_Chest_TU");
        let skin = FieldTarget { suffix: "", ..target };
        assert_eq!(skin.file_name("WolfSkinBlue_u"), "WolfSkinBlue_u");
        assert_eq!(skin.field_text("WolfSkinBlue_u"), "WolfSkinBlue_u");
    }

    fn import(width: u32, height: u32) -> Import {
        Import {
            resize: false,
            ..Import::new(
                PathBuf::from("grass.png"),
                (width, height, vec![255; (width * height * 4) as usize]),
                "Textures/Custom".to_string(),
                "Grass".to_string(),
                None,
            )
        }
    }

    /// A ground texture is the plain file in DXT1 and its `_s` file in DXT3
    /// with the gloss in its alpha, scaled down to the largest shipped size.
    #[test]
    fn a_ground_texture_is_written_with_its_gloss_file() {
        let mut rgba = vec![200u8; 512 * 512 * 4];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 96;
        }
        let ground = Import::new(
            PathBuf::from("rock.png"),
            (512, 512, rgba),
            "Tileset\\Custom".to_string(),
            "Rock".to_string(),
            None,
        );
        assert!(ground.ground());
        assert_eq!(ground.purpose, Purpose::Ground);
        assert_eq!(ground.gloss, Gloss::FromAlpha);
        assert_eq!(ground.written_size(), (GROUND_SIDE, GROUND_SIDE));
        let files = ground.encode().expect("encoded");
        assert_eq!(files[0].0, "Tileset\\Custom\\Rock.blp");
        assert_eq!(files[1].0, "Tileset\\Custom\\Rock_s.blp");
        let plain = blp::decode(&files[0].1).expect("plain");
        let gloss = blp::decode(&files[1].1).expect("gloss");
        assert_eq!((plain.width, plain.height), (256, 256));
        assert_eq!(files[0].1[8..10], [2, 0], "DXT1, no alpha");
        assert_eq!(files[1].1[8..11], [2, 8, 1], "DXT3");
        assert!(plain.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert!(gloss.rgba.chunks_exact(4).all(|pixel| pixel[3].abs_diff(96) <= 9));
        let mut matte = ground.clone();
        matte.gloss = Gloss::Matte;
        let gloss = blp::decode(&matte.encode().expect("encoded")[1].1).expect("gloss");
        assert!(gloss.rgba.chunks_exact(4).all(|pixel| pixel[3] == 0));
        let mut named = ground;
        named.name = "Rock_s".to_string();
        assert!(!named.problems().is_empty());
    }

    /// A typed folder becomes one archive folder, and the file is named in it.
    #[test]
    fn the_path_is_the_folder_and_the_name() {
        assert_eq!(import(64, 64).vpath(), "Textures\\Custom\\Grass.blp");
        assert_eq!(folder_of("\\Creature\\Wolf\\"), "Creature\\Wolf\\");
        assert_eq!(folder_of("  "), "");
    }

    /// Choosing what a texture is for moves it: a ground texture under
    /// `Tileset\`, an icon to `Interface\Icons\` at 64x64, and Other out of
    /// both. The pair is removed together.
    #[test]
    fn the_purpose_decides_the_folder_and_the_size() {
        let mut texture = import(512, 512);
        assert_eq!(texture.purpose, Purpose::Other);
        texture.set_purpose(Purpose::Ground);
        assert_eq!(texture.vpath(), "Tileset\\Custom\\Grass.blp");
        assert_eq!(texture.written_size(), (256, 256));
        texture.folder = "Tileset\\Moor\\".to_string();
        texture.set_purpose(Purpose::Ground);
        assert_eq!(texture.vpath(), "Tileset\\Moor\\Grass.blp", "a ground folder is kept");
        texture.set_purpose(Purpose::Icon);
        assert_eq!(texture.vpath(), "Interface\\Icons\\Grass.blp");
        assert_eq!(texture.written_size(), (64, 64));
        assert_eq!(texture.gloss_vpath(), None);
        texture.set_purpose(Purpose::Other);
        assert_eq!(texture.vpath(), "Textures\\Custom\\Grass.blp");
        assert_eq!(texture.written_size(), (512, 512));
        assert_eq!(
            gloss_of("Tileset\\Custom\\Grass.blp").as_deref(),
            Some("Tileset\\Custom\\Grass_s.blp")
        );
        assert_eq!(gloss_of("Tileset\\Custom\\Grass_s.blp"), None);
        assert_eq!(gloss_of("Textures\\Custom\\Grass.blp"), None);
    }

    /// A side that is not a power of two is refused until the dialog resizes
    /// it, and the resize is up to the next power of two.
    #[test]
    fn a_picture_that_is_not_powers_of_two_needs_the_resize() {
        let mut odd = import(100, 64);
        assert!(!odd.problems().is_empty());
        odd.resize = true;
        assert!(odd.problems().is_empty(), "{:?}", odd.problems());
        assert_eq!(odd.written_size(), (128, 64));
        let files = odd.encode().expect("encoded");
        assert_eq!(files.len(), 1, "not a ground texture: one file");
        let back = blp::decode(&files[0].1).expect("decoded");
        assert_eq!((back.width, back.height), (128, 64));
    }

    /// An opaque picture is DXT1, and a name or folder that would put the file
    /// where nothing finds it is refused.
    #[test]
    fn the_encoding_and_the_refusals() {
        let mut ok = import(32, 32);
        assert_eq!(ok.encoding(), Encoding::Dxt1);
        ok.name = "a/b".to_string();
        assert!(!ok.problems().is_empty());
        let mut root = import(32, 32);
        root.folder.clear();
        assert!(!root.problems().is_empty());
        let mut private = import(32, 32);
        private.folder = "sql\\".to_string();
        assert!(!private.problems().is_empty());
    }
}
