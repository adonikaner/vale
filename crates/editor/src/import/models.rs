//! Importing a model: a glTF 2.0 file written as an M2, with its pictures
//! written as BLP textures beside it.
//!
//! ## What is written
//!
//! `vale_edit::m2::gltf` reads the file and `vale_edit::m2::write` writes the
//! model, its skeleton and one sequence per animation taken. Each material's
//! base colour picture is written as a BLP by the texture import's own
//! encoder ([`super::assets::Import`]), resized to powers of two, in the
//! model's folder. Every file is saved through `EditSession::save_bytes`, as
//! a texture import is, and the import is recorded in
//! [`super::sources`] so Re-import can make the same choices again.
//!
//! ## The dialog asks what a model is for
//!
//! A [`ModelPurpose`] decides the folder, how the textures are named to the
//! game and whether the model is solid:
//!
//! * a creature goes in `Creature\<Name>\`, and its first three materials are
//!   the three creature skins: the M2 names no file, and a
//!   `CreatureDisplayInfo` row's texture columns name the BLP in the model's
//!   folder, as every shipped creature does. The pictures are written as
//!   `<Name>Skin.blp`, `<Name>Skin2.blp` and `<Name>Skin3.blp`;
//! * a doodad goes under `World\`, where the placement tools list models, names
//!   its texture files and is solid where its triangles are;
//! * any other model goes in a typed folder and names its texture files.
//!
//! ## Animations
//!
//! Each animation in the file is a row: whether it is taken, the
//! `AnimationData.dbc` animation it is written as, whether it loops and the
//! speed it was made for. The animation is guessed from the clip's name: its
//! `AnimationData.dbc` name, or a common word for one ([`WORDS`]). Two clips
//! written as one animation are its variations, which the client chooses
//! between. The model's playable-animation table, which says what it plays for
//! every animation it lacks, is built from `AnimationData.dbc` by
//! `vale_assets::tables::dbc::AnimationData::playable`.

use std::path::{Path, PathBuf};

use super::assets::{folder_of, Import, Purpose};
use super::sources::Imported;
use vale_assets::tables::dbc::AnimationData;
use vale_assets::world::m2::anim;
use vale_edit::m2::gltf::{self, ClipChoice, Source};
use vale_edit::m2::write::{self, Collision, Texture};
use vale_edit::project::Project;

/// Where a creature's folder is made.
pub(crate) const CREATURE_ROOT: &str = "Creature\\";
/// Where a doodad's folder is made: under `World\`, which the placement tools
/// list.
pub(crate) const DOODAD_ROOT: &str = "World\\";
/// The folder under [`DOODAD_ROOT`] a doodad starts in.
pub(crate) const DOODAD_FOLDER: &str = "World\\Custom\\";
/// Where any other model starts.
pub(crate) const OTHER_FOLDER: &str = "Custom\\";

/// The animations a clip name commonly means, after its `AnimationData.dbc`
/// name: a word in the name and the animation.
pub(crate) const WORDS: &[(&str, u16)] = &[
    ("idle", anim::STAND),
    ("stand", anim::STAND),
    ("walk", anim::WALK),
    ("run", anim::RUN),
    ("death", anim::DEATH),
    ("die", anim::DEATH),
    ("attack", anim::ATTACK_UNARMED),
    ("hit", anim::COMBAT_WOUND),
    ("hurt", anim::COMBAT_WOUND),
    ("cast", anim::SPELL),
    ("spell", anim::SPELL),
];

/// The animations a sequence loops by default: the ones the shipped wolf
/// loops (stand, the gaits, the shuffles, the jump's air, the swims, sitting,
/// sleep, stun).
const LOOPING: &[u16] = &[0, 4, 5, 11, 12, 13, 14, 38, 40, 41, 42, 43, 44, 45, 69, 97, 100];

/// The speed a gait is made for by default, yards a second: the shipped
/// wolf's, which are the server's base walk, run and swim speeds.
const SPEEDS: &[(u16, f32)] = &[(4, 2.5), (5, 7.0), (13, 2.5), (41, 2.5), (42, 4.72), (45, 2.5)];

/// What an imported model is for. See the module comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelPurpose {
    Creature,
    Doodad,
    Other,
}

impl ModelPurpose {
    pub(crate) const OFFERED: [ModelPurpose; 3] = [ModelPurpose::Creature, ModelPurpose::Doodad, ModelPurpose::Other];

    pub(crate) fn name(self) -> &'static str {
        match self {
            ModelPurpose::Creature => "Creature",
            ModelPurpose::Doodad => "Doodad",
            ModelPurpose::Other => "Other",
        }
    }

    pub(crate) fn hint(self) -> &'static str {
        match self {
            ModelPurpose::Creature => {
                "A creature or NPC model in Creature\\<Name>\\. Its first three materials are the creature \
                 skins a CreatureDisplayInfo row names, written beside it as <Name>Skin.blp."
            }
            ModelPurpose::Doodad => {
                "A model placed on the map with the doodad tool, under World\\. Its textures are named \
                 in the file, and it is solid where its triangles are."
            }
            ModelPurpose::Other => "Any other model, in the folder below, naming its texture files: a spell effect, an item.",
        }
    }

    fn word(self) -> &'static str {
        match self {
            ModelPurpose::Creature => "creature",
            ModelPurpose::Doodad => "doodad",
            ModelPurpose::Other => "other",
        }
    }

    fn of_word(word: &str) -> Option<ModelPurpose> {
        ModelPurpose::OFFERED.into_iter().find(|p| p.word() == word)
    }

    /// What a model at `vpath` is for, from its folder.
    pub(crate) fn of_path(vpath: &str) -> ModelPurpose {
        let lower = vpath.replace('/', "\\").to_ascii_lowercase();
        if lower.starts_with("creature\\") {
            ModelPurpose::Creature
        } else if lower.starts_with("world\\") {
            ModelPurpose::Doodad
        } else {
            ModelPurpose::Other
        }
    }
}

/// The texture one material slot draws.
#[derive(Debug, Clone)]
pub(crate) struct TextureSlot {
    /// 0 for a file, 11, 12 or 13 for a creature skin.
    pub(crate) kind: u32,
    /// The file the slot's picture is written as, without `.blp`, in the
    /// model's folder.
    pub(crate) name: String,
    /// For a file slot with no picture: the archive path of a texture that is
    /// there already.
    pub(crate) path: String,
    /// The slot's picture, read from the file: width, height, RGBA.
    pub(crate) picture: Option<(u32, u32, Vec<u8>)>,
    /// Why the file's picture for this slot could not be read.
    pub(crate) unread: Option<String>,
    /// The slot's texture flags from its sampler.
    pub(crate) wrap: u32,
}

/// The open Import Model dialog.
pub(crate) struct ModelImport {
    pub(crate) source_path: PathBuf,
    pub(crate) source: Box<Source>,
    pub(crate) purpose: ModelPurpose,
    /// The archive folder, ending in a backslash.
    pub(crate) folder: String,
    /// The model's file name, without `.m2`.
    pub(crate) name: String,
    pub(crate) scale: f32,
    /// One per clip in the file.
    pub(crate) clips: Vec<ClipChoice>,
    /// One per material slot.
    pub(crate) textures: Vec<TextureSlot>,
    pub(crate) collision: Collision,
    /// How tall the bind pose stands at a scale of 1, in the model's units.
    pub(crate) height: f32,
}

impl ModelImport {
    /// Read the file at `path` and choose the defaults: what it is for from
    /// `folder` (or, for an empty one, from whether the model has
    /// animations), the name from the file, the animations from the clip
    /// names.
    /// `record` is the line of an earlier import of the same file, whose
    /// choices are taken instead. See [`super::sources`].
    pub(crate) fn open(
        path: &Path,
        folder: &str,
        name: Option<String>,
        animations: Option<&AnimationData>,
        record: Option<&Imported>,
    ) -> Result<ModelImport, String> {
        let said = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let bytes = std::fs::read(path).map_err(|e| said(&e))?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let source = gltf::open(&bytes, &|file| std::fs::read(dir.join(file)).ok()).map_err(|e| said(&e))?;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Model");
        let name = name.unwrap_or_else(|| clean_name(stem));
        let clips = source
            .clips
            .iter()
            .map(|clip| {
                let id = guess(&clip.name, animations);
                // A clip whose name says nothing is taken as Stand when it is
                // the only one, and left out otherwise.
                let take = id.is_some() || source.clips.len() == 1;
                defaults(take, id.unwrap_or(anim::STAND))
            })
            .collect();
        let textures = source
            .slots
            .iter()
            .map(|slot| {
                let (picture, unread) = match slot.image.map(|i| &source.images[i]) {
                    None => (None, None),
                    Some(image) => match &image.bytes {
                        Err(why) => (None, Some(why.clone())),
                        Ok(bytes) => match super::assets::read_picture_bytes(bytes, &image.mime) {
                            Ok(read) => (Some(read), None),
                            Err(why) => (None, Some(why)),
                        },
                    },
                };
                TextureSlot { kind: 0, name: String::new(), path: String::new(), picture, unread, wrap: slot.wrap }
            })
            .collect();
        let mut import = ModelImport {
            source_path: path.to_path_buf(),
            purpose: ModelPurpose::of_path(folder),
            folder: folder_of(folder),
            name,
            scale: 1.0,
            clips,
            textures,
            collision: Collision::None,
            height: 0.0,
            source: Box::new(source),
        };
        import.height = import.measure();
        // With no folder asked for, a model that moves is taken for a
        // creature and one that does not for a doodad.
        let purpose = match (folder.trim().is_empty(), import.source.clips.is_empty()) {
            (true, false) => ModelPurpose::Creature,
            (true, true) => ModelPurpose::Doodad,
            (false, _) => import.purpose,
        };
        import.set_purpose(purpose);
        if let Some(record) = record {
            import.take_choices(record);
        }
        Ok(import)
    }

    /// The bind pose's height at a scale of 1.
    fn measure(&self) -> f32 {
        let choices = self.choices(Vec::new(), 1.0);
        match self.source.model(&choices) {
            Ok(model) => {
                let (lo, hi) = model
                    .vertices
                    .iter()
                    .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.position[2]), hi.max(v.position[2])));
                (hi - lo).max(0.0)
            }
            Err(_) => 0.0,
        }
    }

    /// Change what the model is for, and with it the folder, the texture
    /// slots' kinds and names, and the collision.
    pub(crate) fn set_purpose(&mut self, purpose: ModelPurpose) {
        let now = ModelPurpose::of_path(&self.folder);
        self.folder = match purpose {
            ModelPurpose::Creature if now == ModelPurpose::Creature && self.folder.len() > CREATURE_ROOT.len() => self.folder.clone(),
            ModelPurpose::Creature => format!("{CREATURE_ROOT}{}\\", self.name.trim()),
            ModelPurpose::Doodad if now == ModelPurpose::Doodad && self.folder.len() > DOODAD_ROOT.len() => self.folder.clone(),
            ModelPurpose::Doodad => format!("{DOODAD_FOLDER}{}\\", self.name.trim()),
            ModelPurpose::Other if now != ModelPurpose::Other => OTHER_FOLDER.to_string(),
            ModelPurpose::Other => self.folder.clone(),
        };
        self.purpose = purpose;
        self.collision = match purpose {
            ModelPurpose::Doodad => Collision::Mesh,
            ModelPurpose::Creature | ModelPurpose::Other => Collision::None,
        };
        self.name_textures();
    }

    /// Name the texture slots for the purpose: the three creature skins, or a
    /// file each named after the model and the material.
    pub(crate) fn name_textures(&mut self) {
        let model = self.name.trim().to_string();
        let slots: Vec<String> = self.source.slots.iter().map(|s| clean_name(&s.name)).collect();
        let one = slots.len() == 1;
        for (index, slot) in self.textures.iter_mut().enumerate() {
            let skin = self.purpose == ModelPurpose::Creature && index < 3;
            slot.kind = match skin {
                true => 11 + index as u32,
                false => 0,
            };
            slot.name = match (skin, index) {
                (true, 0) => format!("{model}Skin"),
                (true, n) => format!("{model}Skin{}", n + 1),
                (false, _) if one => model.clone(),
                (false, _) => format!("{model}_{}", slots[index]),
            };
        }
    }

    /// The archive path of the model.
    pub(crate) fn vpath(&self) -> String {
        format!("{}{}.m2", folder_of(&self.folder), self.name.trim())
    }

    /// The archive path a slot's picture is written at, or `None` for a slot
    /// with no picture.
    pub(crate) fn texture_vpath(&self, slot: usize) -> Option<String> {
        let texture = self.textures.get(slot)?;
        texture.picture.as_ref()?;
        Some(format!("{}{}.blp", folder_of(&self.folder), texture.name.trim()))
    }

    /// The path the model names for a slot: the written picture, a typed path,
    /// or nothing for a creature skin.
    fn named_path(&self, slot: usize) -> String {
        let texture = &self.textures[slot];
        match (texture.kind, self.texture_vpath(slot)) {
            (0, Some(written)) => written,
            (0, None) => texture.path.trim().replace('/', "\\"),
            _ => String::new(),
        }
    }

    fn choices(&self, playable: Vec<(u16, u16)>, scale: f32) -> gltf::Choices {
        gltf::Choices {
            name: self.name.trim().to_string(),
            scale,
            clips: self.clips.clone(),
            textures: (0..self.textures.len())
                .map(|slot| Texture {
                    kind: self.textures[slot].kind,
                    flags: self.textures[slot].wrap,
                    path: self.named_path(slot),
                })
                .collect(),
            collision: self.collision,
            playable,
        }
    }

    /// Why it cannot be written as it stands, or nothing.
    pub(crate) fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let name = self.name.trim();
        if name.is_empty() {
            out.push("The name is empty.".to_string());
        }
        if name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            out.push("The name may not contain \\ / : * ? \" < > |.".to_string());
        }
        if folder_of(&self.folder).is_empty() {
            out.push("The folder is empty: a model at the archives' root is found by nothing.".to_string());
        }
        if !Project::is_game_data(&self.vpath()) {
            out.push("The folder is one of the project's own (sql, server, publish, editor), which is not published.".to_string());
        }
        if self.source.vertices > u16::MAX as usize {
            out.push(format!(
                "The model has {} vertices, and a 1.12 model holds 65,535 at most. Reduce it in the tool it was made in.",
                self.source.vertices
            ));
        }
        if self.source.bones.len() > 256 {
            out.push(format!("The model has {} bones, and a 1.12 model holds 256 at most.", self.source.bones.len()));
        }
        if !(self.scale.is_finite() && self.scale > 0.0) {
            out.push("The scale must be more than 0.".to_string());
        }
        for (slot, texture) in self.textures.iter().enumerate() {
            let material = &self.source.slots[slot].name;
            if texture.kind == 0 && texture.picture.is_none() && texture.path.trim().is_empty() {
                out.push(format!("Material \"{material}\" has no picture in the file: type the path of a texture."));
            }
            if texture.picture.is_some() && texture.name.trim().is_empty() {
                out.push(format!("Material \"{material}\"'s texture has no name."));
            }
        }
        let written: Vec<String> = (0..self.textures.len()).filter_map(|s| self.texture_vpath(s)).map(|p| p.to_ascii_lowercase()).collect();
        if (1..written.len()).any(|i| written[..i].contains(&written[i])) {
            out.push("Two materials' textures have the same name.".to_string());
        }
        out
    }

    /// The files written, each archive path with its bytes: each slot's
    /// picture as a BLP, then the model.
    pub(crate) fn encode(&self, animations: Option<&AnimationData>) -> Result<Vec<(String, Vec<u8>)>, String> {
        let mut files = Vec::new();
        for (slot, texture) in self.textures.iter().enumerate() {
            let Some(picture) = texture.picture.clone() else { continue };
            let mut import = Import::new(self.source_path.clone(), picture, self.folder.clone(), texture.name.trim().to_string(), None);
            import.set_purpose(Purpose::Other);
            import.folder = folder_of(&self.folder);
            import.resize = true;
            if let Some(problem) = import.problems().into_iter().next() {
                return Err(format!("material \"{}\": {problem}", self.source.slots[slot].name));
            }
            files.extend(import.encode()?);
        }
        let taken: Vec<u16> = self.clips.iter().filter(|c| c.take).map(|c| c.id).collect();
        let playable = match (animations, taken.first()) {
            (Some(animations), Some(&first)) => animations.playable(&taken, first),
            _ => Vec::new(),
        };
        let model = self.source.model(&self.choices(playable, self.scale)).map_err(|e| e.to_string())?;
        let bytes = write::write(&model).map_err(|e| e.to_string())?;
        files.push((self.vpath(), bytes));
        Ok(files)
    }

    /// The choices, for the record. See [`super::sources`].
    pub(crate) fn settings(&self) -> Vec<(String, String)> {
        let clips: Vec<String> = self
            .clips
            .iter()
            .map(|c| format!("{}:{}:{}:{}", u8::from(c.take), c.id, u8::from(c.loops), c.move_speed))
            .collect();
        let textures: Vec<String> = self.textures.iter().map(|t| format!("{}:{}:{}", t.kind, t.name, t.path)).collect();
        vec![
            ("purpose".to_string(), self.purpose.word().to_string()),
            ("scale".to_string(), self.scale.to_string()),
            ("collision".to_string(), collision_word(self.collision).to_string()),
            ("clips".to_string(), clips.join(",")),
            ("textures".to_string(), textures.join(",")),
        ]
    }

    /// Take an earlier import's choices: its path, purpose, scale, collision,
    /// clips and texture names, where they still fit the file.
    fn take_choices(&mut self, record: &Imported) {
        let vpath = record.vpath.replace('/', "\\");
        if let Some(at) = vpath.rfind('\\') {
            self.folder = vpath[..=at].to_string();
            self.name = vpath[at + 1..].trim_end_matches(".m2").trim_end_matches(".M2").to_string();
        }
        if let Some(purpose) = record.setting("purpose").and_then(ModelPurpose::of_word) {
            self.purpose = purpose;
            self.name_textures();
        }
        if let Some(scale) = record.setting("scale").and_then(|s| s.parse().ok()) {
            self.scale = scale;
        }
        if let Some(collision) = record.setting("collision").and_then(collision_of) {
            self.collision = collision;
        }
        for (clip, text) in self.clips.iter_mut().zip(record.setting("clips").unwrap_or("").split(',')) {
            let parts: Vec<&str> = text.split(':').collect();
            if let [take, id, loops, speed] = parts[..] {
                if let (Ok(id), Ok(speed)) = (id.parse(), speed.parse()) {
                    *clip = ClipChoice { take: take == "1", id, loops: loops == "1", move_speed: speed };
                }
            }
        }
        for (slot, (kind, name, path)) in self.textures.iter_mut().zip(recorded_textures(record)) {
            slot.kind = kind;
            slot.name = name;
            slot.path = path;
        }
    }
}

/// The texture slots an import record names, in slot order: each slot's
/// kind, the file name its picture was written as, and its typed path.
pub(crate) fn recorded_textures(entry: &Imported) -> Vec<(u32, String, String)> {
    entry
        .setting("textures")
        .unwrap_or("")
        .split(',')
        .filter_map(|text| {
            let mut parts = text.splitn(3, ':');
            let kind = parts.next()?.parse().ok()?;
            Some((kind, parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .collect()
}

/// The creature skins an imported model wears, as a portrait's
/// `Worn::skins` holds them: the archive path of each of the three skins its
/// import wrote into `folder`, empty where it wrote none. A creature model
/// names no file for a skin, so without these it is drawn with none.
pub(crate) fn imported_skins(entry: &Imported, folder: &str, project: &Project) -> Vec<String> {
    let mut skins = vec![String::new(); 3];
    for (kind, name, _) in recorded_textures(entry) {
        let path = format!("{}{}.blp", folder_of(folder), name);
        if (11..=13).contains(&kind) && !name.is_empty() && project.has(&path) {
            skins[(kind - 11) as usize] = path;
        }
    }
    skins
}

/// A clip's default row for animation `id`.
pub(crate) fn defaults(take: bool, id: u16) -> ClipChoice {
    ClipChoice {
        take,
        id,
        loops: LOOPING.contains(&id),
        move_speed: SPEEDS.iter().find(|(i, _)| *i == id).map_or(0.0, |(_, s)| *s),
    }
}

/// The animation a clip's name means: its `AnimationData.dbc` name, ignoring
/// case, spaces, underscores and a trailing number, or a word of [`WORDS`].
pub(crate) fn guess(name: &str, animations: Option<&AnimationData>) -> Option<u16> {
    let bare = |text: &str| {
        let letters: String = text.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        letters.trim_end_matches(|c: char| c.is_ascii_digit()).to_string()
    };
    // Exporters prefix a clip with its armature: `Armature|Walk`.
    let clip = bare(name.rsplit('|').next().unwrap_or(name));
    if let Some(animations) = animations {
        if let Some(id) = animations.ids().into_iter().find(|&id| animations.name(id).is_some_and(|n| bare(n) == clip)) {
            return Some(id);
        }
    }
    WORDS.iter().find(|(word, _)| clip.contains(word)).map(|(_, id)| *id)
}

/// A name usable as a file name: the characters a path may not hold removed.
fn clean_name(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect::<String>()
        .trim()
        .replace(' ', "_");
    match cleaned.is_empty() {
        true => "Model".to_string(),
        false => cleaned,
    }
}

fn collision_word(collision: Collision) -> &'static str {
    match collision {
        Collision::None => "none",
        Collision::Box => "box",
        Collision::Mesh => "mesh",
    }
}

fn collision_of(word: &str) -> Option<Collision> {
    match word {
        "none" => Some(Collision::None),
        "box" => Some(Collision::Box),
        "mesh" => Some(Collision::Mesh),
        _ => None,
    }
}

/// Whether a file on disk is one the model import reads, by its extension.
pub(crate) fn is_model_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("glb") || e.eq_ignore_ascii_case("gltf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clip is guessed from a common word when the table is not to hand,
    /// with an exporter's armature prefix and a take number ignored.
    #[test]
    fn a_clip_is_guessed_from_its_name() {
        assert_eq!(guess("Armature|Walk_01", None), Some(anim::WALK));
        assert_eq!(guess("Idle", None), Some(anim::STAND));
        assert_eq!(guess("Running", None), Some(anim::RUN));
        assert_eq!(guess("Wave", None), None);
        let run = defaults(true, anim::RUN);
        assert!(run.loops && run.move_speed == 7.0);
        assert!(!defaults(true, anim::DEATH).loops);
    }

    /// What a model is for follows from its folder, and a name loses the
    /// characters a path may not hold.
    #[test]
    fn the_purpose_and_the_name() {
        assert_eq!(ModelPurpose::of_path("Creature\\Mage\\"), ModelPurpose::Creature);
        assert_eq!(ModelPurpose::of_path("World\\Custom\\"), ModelPurpose::Doodad);
        assert_eq!(ModelPurpose::of_path("Spells\\"), ModelPurpose::Other);
        assert_eq!(clean_name("hooded mage: v2"), "hooded_mage_v2");
        assert!(is_model_file(Path::new("a.GLB")) && !is_model_file(Path::new("a.png")));
    }
}
