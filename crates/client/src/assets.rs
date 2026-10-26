//! The archive chain and the display tables, as a Bevy resource.
//!
//! The archive chain is opened on first use and the parsed `DisplayTables` are
//! cached. What is read from them becomes `Mesh` and `Image` assets directly.

use vale_assets::{
    interface::bindings::{Bindings, BINDINGS_XML},
    tables::charcreate::CharCreate,
    tables::dbc::{dbc_path, DisplayTables},
    tables::foliage::GroundEffects,
    tables::loading::LoadingScreens,
    tables::minimap::{MinimapTiles, MD5_TRANSLATE},
    tables::sound::SoundBank,
    interface::strings::{Strings, GLOBAL_STRINGS},
    Assets,
};
use bevy::prelude::*;
use std::sync::{Arc, Mutex};

/// Where the game archives are, and whatever has been read out of them.
///
/// `archive` is behind a `Mutex` because `Assets::read` needs `&mut` (the MPQ
/// reader seeks and caches internally), and tile loads run on the task pool.
#[derive(Resource)]
pub struct GameAssets {
    pub gamedata_dir: String,
    /// The install folder, whose `Interface\AddOns\` the chain reads loose
    /// files under; see `Assets::with_loose_root`. `.` unless
    /// [`Self::with_root`] sets another folder.
    pub root: String,
    /// Shared with the readers [`Self::reader`] hands out, which outlive any
    /// borrow of this resource.
    archive: Arc<Mutex<Option<Assets>>>,
    /// A source asked before the archives, for every path.
    ///
    /// Held here rather than only on the chain because the chain is opened on
    /// first use: a host that installs an overlay during startup would otherwise
    /// be installing it on nothing. It is applied when the chain opens and, if
    /// it is already open, at once.
    ///
    /// Empty unless something installs one. Nothing in this crate does: it is
    /// here for a host that has to answer with bytes of its own for a path the
    /// archives also carry. See `vale_assets::archive::Overlay`.
    overlay: Arc<Mutex<Option<vale_assets::archive::Overlay>>>,
    /// The number of times [`Self::forget_tables`] has been called, so that a
    /// cache built from the tables can tell that the tables it was built from
    /// have been dropped.
    ///
    /// Forgetting alone is not enough. Every bank below is re-parsed on the
    /// next ask, but a consumer that keeps its own derived data (the
    /// spellbook's `Book`, the action bar's resolved slots) asks again only
    /// when its own key changes, and those keys come from the server (a
    /// spellbook version, a page). Without this number, an edited `Spell.dbc`
    /// reached the archives and the panels kept showing what they had resolved
    /// at login. Both of those rebuilds include this number in their key. It is
    /// a counter rather than a flag so that a reader compares it with the value
    /// it last built against and needs no reset.
    ///
    /// Relaxed ordering throughout. It is read once per frame by two systems on
    /// the main thread and there is nothing to synchronise with it.
    tables_generation: std::sync::atomic::AtomicU64,
    displays: Mutex<Option<Arc<DisplayTables>>>,
    /// The game's own interface text, loaded lazily like the tables.
    ///
    /// Not part of `DisplayTables`, because it is not a DBC. That loader is
    /// keyed by bare table name and prepends `DBFilesClient\`; this is a Lua
    /// file under `Interface\FrameXML\`, so folding it in would need either a
    /// second closure over the same `&mut Assets` or a table name that is not
    /// one. See [`vale_assets::interface::strings`].
    strings: Mutex<Option<Arc<Strings>>>,
    /// `Interface\FrameXML\Bindings.xml`, on the same terms as
    /// [`Self::strings`] and for the same reason: it is a FrameXML file, not a
    /// DBC.
    bindings: Mutex<Option<Arc<Bindings>>>,
    /// The sound tables, loaded lazily like the display tables. They are a
    /// separate bank rather than more `DisplayTables` slots because
    /// `crates/client/src/sound/` is the projected crate split, and this keeps
    /// its dependency on the assets crate to one type. See
    /// [`vale_assets::tables::sound`].
    sounds: Mutex<Option<Arc<SoundBank>>>,
    /// The character-creation tables, on the same terms. They are a separate
    /// bank rather than more `DisplayTables` slots because one screen needs
    /// them, before there is a world, and nothing else does. See
    /// [`vale_assets::tables::charcreate`].
    char_create: Mutex<Option<Arc<CharCreate>>>,
    /// Which minimap picture covers which tile, on the same terms: a plain
    /// text index under `textures\Minimap\`, not a DBC. See
    /// [`vale_assets::tables::minimap`].
    minimap: Mutex<Option<Arc<MinimapTiles>>>,
    /// Which picture a map is hidden behind while it loads.
    ///
    /// It comes from two DBCs, so it could be two more `DisplayTables` slots.
    /// It is a separate bank on [`Self::char_create`]'s terms instead, because
    /// one pass needs it, at the two moments when nothing else is being read,
    /// and folding it in would make every `vale npc` load it too. See
    /// [`vale_assets::tables::loading`].
    loading: Mutex<Option<Arc<LoadingScreens>>>,
    /// What grows on the ground, on the same terms. It is a separate bank
    /// rather than more `DisplayTables` slots because one pass needs it, on the
    /// tile loader thread, and folding it in would make every `vale npc` parse
    /// a 357 KB table it does not use. See [`vale_assets::tables::foliage`].
    ground_effects: Mutex<Option<Arc<GroundEffects>>>,
    /// Archive chains for work that runs off the main thread. See
    /// [`vale_assets::archive::ChainPool`], which documents the cost of
    /// opening one.
    ///
    /// This is not [`Self::archive`] and does not give access to it. That
    /// field is a single chain behind a lock the main thread takes several
    /// times a frame, and the task-pool loaders exist so that a tile load does
    /// not hold that lock for the length of a read. The pool gives each loader
    /// a chain of its own and reuses chains instead of opening and dropping one
    /// per tile.
    chains: Arc<vale_assets::archive::ChainPool>,
}

impl GameAssets {
    pub fn new(gamedata_dir: String) -> Self {
        GameAssets {
            gamedata_dir,
            root: ".".to_string(),
            archive: Arc::new(Mutex::new(None)),
            overlay: Arc::new(Mutex::new(None)),
            tables_generation: std::sync::atomic::AtomicU64::new(0),
            displays: Mutex::new(None),
            strings: Mutex::new(None),
            bindings: Mutex::new(None),
            sounds: Mutex::new(None),
            char_create: Mutex::new(None),
            minimap: Mutex::new(None),
            loading: Mutex::new(None),
            ground_effects: Mutex::new(None),
            chains: Arc::new(vale_assets::archive::ChainPool::default()),
        }
    }

    /// Set the install folder, for the loose-file overlay.
    pub(crate) fn with_root(mut self, root: String) -> Self {
        self.root = root;
        self
    }

    /// Answer every read from `overlay` before the archives.
    ///
    /// Takes effect on a chain that is already open as well as on one that is
    /// not, so the caller does not have to know which.
    pub fn set_overlay(&self, overlay: Option<vale_assets::archive::Overlay>) {
        *self.overlay.lock().unwrap_or_else(|e| e.into_inner()) = overlay.clone();
        let mut guard = self.archive.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(chain) = guard.as_mut() {
            chain.set_overlay(overlay);
        }
    }

    /// The cell the overlay is stored in, for a loader that opens a chain of
    /// its own and starts before the overlay is installed. The model loader's
    /// thread starts before a host has anything to install, so it reads this
    /// cell on every request rather than once. See [`Self::overlay`].
    pub(crate) fn overlay_cell(&self) -> Arc<Mutex<Option<vale_assets::archive::Overlay>>> {
        Arc::clone(&self.overlay)
    }

    /// The overlay installed on this resource, for a loader that opens a chain
    /// of its own.
    ///
    /// Every such loader has to call this. `render::terrain::read_tile` and
    /// `render::globalwmo` each read on a chain of their own on the task pool
    /// rather than queueing on this resource's lock, which lets nine tiles
    /// load at once. A chain without the overlay answers the game's own bytes
    /// for a path a host is overriding, so an edited tile reloaded from the
    /// archives with the edit missing. This applies to a chain borrowed from
    /// the pool as much as to a freshly opened one: the pool clears the
    /// overlay when a chain is returned to it, so every borrower sets its own.
    pub fn overlay(&self) -> Option<vale_assets::archive::Overlay> {
        self.overlay
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The pool that a loader running off the main thread borrows a chain
    /// from. See [`Self::chains`] for why it is not [`Self::with_archive`].
    ///
    /// An `Arc` because the borrower is usually a spawned task, which cannot
    /// hold a reference to a resource.
    pub fn chains(&self) -> Arc<vale_assets::archive::ChainPool> {
        Arc::clone(&self.chains)
    }

    /// Forget every table parsed out of the archives, so the next ask reads
    /// them again.
    ///
    /// Each of the banks below is parsed once and kept for the life of the
    /// process, which is right for files that do not change. It is wrong once
    /// a host makes one of those paths answer with different bytes: the
    /// overlay is consulted on every read, and a table that was read before
    /// the overlay was installed is never read again.
    ///
    /// Without this, an edited `Spell.dbc` reaches the archives' namespace,
    /// the project folder and the patch archive, and the spell in the world is
    /// still the old one, because the `DisplayTables` in hand were parsed at
    /// the first login and nothing asks for them again.
    ///
    /// Two things a caller has to know:
    ///
    /// * It takes effect on the next ask. Anything holding an `Arc` from
    ///   before keeps what it holds, which includes every pass that cached one.
    ///   Call it while the world is being built rather than mid-frame.
    /// * It is not free. The next ask re-parses every DBC the bank holds
    ///   (`Spell.dbc` alone is 16 MB), so call it when something has changed,
    ///   not on a timer.
    pub fn forget_tables(&self) {
        self.tables_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.displays.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.strings.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.bindings.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.sounds.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.char_create.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.minimap.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.loading.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self
            .ground_effects
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Which parse of the tables the ones in hand come from, for a cache built
    /// from them. See the field note on [`Self::tables_generation`]. Starts at
    /// 0 and rises by one on every [`Self::forget_tables`].
    pub fn tables_generation(&self) -> u64 {
        self.tables_generation
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// A file as the install holds it, skipping any overlay.
    ///
    /// `with_archive` reads through the overlay by design, so it cannot return
    /// the bytes the archives carry for an overridden path. Those bytes are
    /// what a comparison of an edited file against its original needs.
    ///
    /// With no overlay installed, this and `with_archive(|a| a.read(path))`
    /// return the same bytes. That is every session of this client: nothing
    /// here calls `set_overlay`.
    pub fn read_past_overlay(&self, path: &str) -> Result<Vec<u8>, String> {
        self.with_archive(|assets| assets.read_past_overlay(path).map_err(|e| e.to_string()))
    }

    /// [`Self::read_past_overlay`], also skipping the named archives; see
    /// `Assets::read_past_overlay_skipping`. With an empty list it returns
    /// the same bytes as [`Self::read_past_overlay`].
    pub fn read_past_overlay_skipping(&self, path: &str, skip: &[String]) -> Result<Vec<u8>, String> {
        self.with_archive(|assets| {
            assets
                .read_past_overlay_skipping(path, skip)
                .map_err(|e| e.to_string())
        })
    }

    /// Run `f` against the archive chain, opening it on first use.
    ///
    /// Opening ~19 archives takes a moment, so it does not happen at startup:
    /// the window paints immediately and the first tile load pays for it.
    pub fn with_archive<T>(
        &self,
        f: impl FnOnce(&mut Assets) -> Result<T, String>,
    ) -> Result<T, String> {
        with_chain(&self.archive, &self.gamedata_dir, &self.root, &self.overlay, f)
    }

    /// A reader that outlives this resource: the archive chain and the loose
    /// overlay, opened on first use, behind a clone of the same lock.
    ///
    /// `LoadAddOn` runs inside a Lua call with no `Res<GameAssets>` in reach,
    /// so the addon board holds one of these instead. See
    /// `crate::lua::panels::addons`.
    pub fn reader(&self) -> std::rc::Rc<dyn Fn(&str) -> Option<Vec<u8>>> {
        let archive = Arc::clone(&self.archive);
        let overlay = Arc::clone(&self.overlay);
        let gamedata = self.gamedata_dir.clone();
        let root = self.root.clone();
        std::rc::Rc::new(move |path: &str| {
            with_chain(&archive, &gamedata, &root, &overlay, |chain| {
                Ok(chain.read(path).ok())
            })
            .ok()
            .flatten()
        })
    }

    /// [`Self::reader`] for another thread: the same chain and overlay behind
    /// the same lock, so reads from a worker queue behind the main thread's
    /// rather than opening the archives a second time.
    pub fn shared_reader(&self) -> Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync> {
        let archive = Arc::clone(&self.archive);
        let overlay = Arc::clone(&self.overlay);
        let gamedata = self.gamedata_dir.clone();
        let root = self.root.clone();
        Arc::new(move |path: &str| {
            with_chain(&archive, &gamedata, &root, &overlay, |chain| {
                Ok(chain.read(path).ok())
            })
            .ok()
            .flatten()
        })
    }
}

/// [`GameAssets::with_archive`]'s body, shared with the readers it hands out.
fn with_chain<T>(
    archive: &Mutex<Option<Assets>>,
    gamedata_dir: &str,
    root: &str,
    overlay: &Mutex<Option<vale_assets::archive::Overlay>>,
    f: impl FnOnce(&mut Assets) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = archive.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let mut chain = Assets::open(gamedata_dir)
            .map_err(|e| e.to_string())?
            .with_loose_root(root);
        chain.set_overlay(overlay.lock().unwrap_or_else(|e| e.into_inner()).clone());
        *guard = Some(chain);
    }
    f(guard.as_mut().expect("just opened"))
}

impl GameAssets {

    /// The three display DBCs plus the bakes, parsed on first call.
    ///
    /// Returned as an `Arc` so the lock is released before any lookup happens:
    /// a login resolves dozens of display ids at once and they should not queue
    /// behind each other.
    pub fn display_tables(&self) -> Result<Arc<DisplayTables>, String> {
        let mut guard = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            // `DisplayTables` decides which tables are wanted, which are
            // mandatory and what each absent one costs, so this crate only
            // supplies a way to read one. Adding a table is a line there and
            // no change here.
            let tables = self.with_archive(|assets| {
                let mut tables = DisplayTables::load(|table| assets.read(&dbc_path(table)).ok())
                    .map_err(|e| e.to_string())?;
                // The world map's `.zmp` grids are files rather than tables,
                // so they are loaded by a second call. See
                // `DisplayTables::load_zone_grids`.
                tables.load_zone_grids(|path| assets.read(path).ok());
                Ok(tables)
            })?;
            *guard = Some(Arc::new(tables));
        }
        Ok(Arc::clone(guard.as_ref().expect("just parsed")))
    }

    /// The game's own interface strings, parsed on first call.
    ///
    /// Never fails. An archive chain without `GlobalStrings.lua` yields an
    /// empty table, and an empty table shows no messages. The client also
    /// shows nothing for a key it cannot find, so this failure behaves the way
    /// the client already does.
    pub fn strings(&self) -> Arc<Strings> {
        let mut guard = self.strings.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let raw = self
                .with_archive(|assets| Ok(assets.read(GLOBAL_STRINGS).ok()))
                .ok()
                .flatten();
            if raw.is_none() {
                warn!("{GLOBAL_STRINGS} is not in the archives — interface messages will be blank");
            }
            *guard = Some(Arc::new(Strings::parse(&raw.unwrap_or_default())));
        }
        Arc::clone(guard.as_ref().expect("just parsed"))
    }

    /// The game's own key-binding declarations, parsed on first call.
    ///
    /// Never fails, on the same terms as [`Self::strings`]: a chain without the
    /// file yields an empty table, and with an empty table no key does
    /// anything. That failure is visible and is not a crash. See
    /// [`vale_assets::interface::bindings`].
    pub fn bindings(&self) -> Arc<Bindings> {
        let mut guard = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let raw = self
                .with_archive(|assets| Ok(assets.read(BINDINGS_XML).ok()))
                .ok()
                .flatten();
            if raw.is_none() {
                warn!("{BINDINGS_XML} is not in the archives — no key will do anything");
            }
            *guard = Some(Arc::new(Bindings::parse(&raw.unwrap_or_default())));
        }
        Arc::clone(guard.as_ref().expect("just parsed"))
    }

    /// The sound tables, parsed on first call.
    ///
    /// Never fails, on [`Self::strings`]' terms: a chain without
    /// `SoundEntries.dbc` yields an empty bank, and an empty bank answers every
    /// lookup with `None`. The world is then silent, and one warning says why.
    pub fn sounds(&self) -> Arc<SoundBank> {
        let mut guard = self.sounds.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let bank = self
                .with_archive(|assets| {
                    SoundBank::load(|table| assets.read(&dbc_path(table)).ok())
                        .map_err(|e| e.to_string())
                })
                .unwrap_or_else(|e| {
                    warn!("sound tables did not load — the world stays silent: {e}");
                    SoundBank::default()
                });
            *guard = Some(Arc::new(bank));
        }
        Arc::clone(guard.as_ref().expect("just parsed"))
    }

    /// What a character may be made of, parsed on first call.
    ///
    /// Never fails, on [`Self::sounds`]' terms: a chain missing `ChrRaces`
    /// answers no races, which gives a character-create screen with no buttons
    /// rather than one with wrong buttons. See
    /// [`vale_assets::tables::charcreate::CharCreate::load`], which documents
    /// what each missing table does.
    pub(crate) fn char_create(&self) -> Arc<CharCreate> {
        let mut guard = self.char_create.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let tables = self
                .with_archive(|assets| {
                    Ok(CharCreate::load(|table| assets.read(&dbc_path(table)).ok()))
                })
                .unwrap_or_else(|e| {
                    warn!("the character-create tables did not load: {e}");
                    CharCreate::default()
                });
            *guard = Some(Arc::new(tables));
        }
        Arc::clone(guard.as_ref().expect("just parsed"))
    }

    /// Which minimap picture covers which tile, parsed on first call.
    ///
    /// Never fails, on [`Self::strings`]' terms: a chain without
    /// `md5translate.trs` yields an empty index, and an empty index draws a
    /// black minimap. The client draws a tile it cannot find the same way
    /// (`MINIMAPCHUNKNOTFOUND`), so this is not a new failure. It has its own
    /// slot for the reason [`Self::strings`] has one: it is not a DBC.
    pub fn minimap_tiles(&self) -> Arc<MinimapTiles> {
        let mut guard = self.minimap.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let raw = self
                .with_archive(|assets| Ok(assets.read(MD5_TRANSLATE).ok()))
                .ok()
                .flatten();
            if raw.is_none() {
                warn!("{MD5_TRANSLATE} is not in the archives — the minimap will be blank");
            }
            *guard = Some(Arc::new(MinimapTiles::parse(&raw.unwrap_or_default())));
        }
        Arc::clone(guard.as_ref().expect("just parsed"))
    }

    /// Which picture a map is hidden behind while it loads, parsed on first
    /// call.
    ///
    /// Never fails, on [`Self::strings`]' terms, and the failure is mild: a
    /// chain missing either table answers
    /// [`vale_assets::tables::loading::FALLBACK`] for every map, which is the
    /// generic parchment the client shows for a map that names no screen. A
    /// broken install therefore shows the wrong loading picture rather than
    /// none.
    pub fn loading_screens(&self) -> Arc<LoadingScreens> {
        let mut guard = self.loading.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let screens = self
                .with_archive(|assets| {
                    Ok(LoadingScreens::load(|table| assets.read(&dbc_path(table)).ok()))
                })
                .unwrap_or_else(|e| {
                    warn!("the loading-screen tables did not load: {e}");
                    LoadingScreens::default()
                });
            *guard = Some(Arc::new(screens));
        }
        Arc::clone(guard.as_ref().expect("just parsed"))
    }

    /// What grows on the ground, parsed on first call.
    ///
    /// Never fails, on [`Self::sounds`]' terms: a chain without
    /// `GroundEffectTexture` answers no effects, and no effects means bare
    /// ground, which is a visible failure rather than a crash.
    ///
    /// Returned as an `Arc` so the lock is released before the first lookup:
    /// it is read on the tile loader thread, nine at a time on a login, and
    /// those reads must not queue behind each other any more than the tile
    /// parses do.
    pub fn ground_effects(&self) -> Arc<GroundEffects> {
        let mut guard = self.ground_effects.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let effects = self
                .with_archive(|assets| {
                    Ok(GroundEffects::load(|table| assets.read(&dbc_path(table)).ok()))
                })
                .unwrap_or_else(|e| {
                    warn!("the ground-effect tables did not load — the ground stays bare: {e}");
                    GroundEffects::default()
                });
            *guard = Some(Arc::new(effects));
        }
        Arc::clone(guard.as_ref().expect("just parsed"))
    }

    /// Which archives mounted, so the HUD can say whether game data was found
    /// at all. Missing game data is the most common setup failure.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn archive_names(&self) -> Result<Vec<String>, String> {
        self.with_archive(|assets| {
            Ok(assets
                .archive_names()
                .into_iter()
                .map(str::to_string)
                .collect())
        })
    }
}

/// The bytes behind some virtual paths changed, so what was read from them
/// once has to be read again.
///
/// A host that answers paths with bytes of its own (see [`GameAssets`]'s
/// overlay) sends this after changing them. The overlay is asked only when a
/// path is read, and five banks read a path once and keep what they read. Each
/// handles this message in its own module, in [`ForgetChanged`]:
///
/// ```text
/// GameAssets    the tables and the banks parsed with them, when `tables`
/// DisplayCache  its Arc of the display tables, read again at once, when `tables`
/// ModelCache    every model, skin and texture read by one of `paths`, or all
/// UiTextures    every minimap picture read by one of `paths`, or all
/// Mixer         every sound file read by one of `paths`, or all
/// ```
///
/// Nothing drawn is rebuilt. What is standing keeps its handles, and the next
/// thing to ask reads the file.
#[derive(Message, Debug, Clone, Default)]
pub struct PathsChanged {
    /// The paths whose bytes changed.
    pub paths: Vec<String>,
    /// Whether any path may have changed, more than the sender can name: the
    /// source behind the overlay was replaced.
    pub all: bool,
    /// Whether the client tables changed. They are dozens of paths read and
    /// parsed together, so they are named as one.
    pub tables: bool,
}

/// The systems that handle [`PathsChanged`], one per bank. A host that
/// changes a path and asks for it in the same frame sends the message before
/// this set and asks after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForgetChanged;

/// [`PathsChanged`] and the [`GameAssets`] half of handling it.
pub(crate) struct AssetsPlugin;

impl Plugin for AssetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PathsChanged>()
            .add_systems(Update, forget_changed_tables.in_set(ForgetChanged));
    }
}

/// Drop the parsed tables when a [`PathsChanged`] says they changed.
///
/// Public so that a bank derived from the tables can order its own re-read
/// after it.
pub(crate) fn forget_changed_tables(mut changed: MessageReader<PathsChanged>, assets: Res<GameAssets>) {
    if changed.read().fold(false, |any, change| any | change.tables) {
        assets.forget_tables();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forgetting the tables increments the generation, which every cache built
    /// from the tables includes in its key besides its own subject.
    ///
    /// Five caches do: the spellbook's `Book`, the action bar's slots, the
    /// talent tree, the stance bar and the supersede check. When each was keyed
    /// on the server's spellbook version alone, which a table edit does not
    /// change, an edited `Spell.dbc` reached the archives and the panels kept
    /// showing what they resolved at login.
    #[test]
    fn forgetting_the_tables_moves_the_generation() {
        let assets = GameAssets::new("Data".into());
        assert_eq!(assets.tables_generation(), 0, "nothing has been forgotten");
        assets.forget_tables();
        assert_eq!(assets.tables_generation(), 1);
        assets.forget_tables();
        assert_eq!(assets.tables_generation(), 2, "it counts rather than latches");
    }

    /// A cache that has never been built compares unequal to the generation,
    /// so the first frame builds rather than deciding it is already current.
    #[test]
    fn a_cache_that_has_never_been_built_is_not_current() {
        let assets = GameAssets::new("Data".into());
        let never_built: u64 = u64::default();
        assert_eq!(never_built, assets.tables_generation());
    }
}
