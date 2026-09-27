//! The terrain pass: the block of tiles around the focus — the 3x3 in a
//! session, and as many as [`TerrainReach`] asks for in a host that wants
//! more.
//!
//! The parsing is `vale_assets::world::adt`'s and is untouched. What lives here is
//! the turn from a `TerrainMesh` into Bevy meshes, and the decision about how to
//! cut it up.
//!
//! One mesh per `TerrainDraw` group. A chunk's draw call used to be pinned
//! by its own alpha texture, so no two chunks could share one and a tile cost
//! 256 calls; `Adt::alpha_atlas` packs a tile's 256 blend maps into one
//! 1024x1024 texture and makes the lookup vertex data, which leaves the texture
//! set as the only per-chunk state. `Adt::to_mesh` already groups the index
//! buffer by set — measured at 23 groups for Stormwind's tile, 31 for Elwynn's,
//! 46 for Duskwood's.
//!
//! Splitting those groups into separate meshes rather than one mesh with ranges
//! costs a vertex remap and buys back something the WebGL version had to give
//! up: each group gets its own `Aabb` and Bevy frustum-culls it. A group can
//! span the tile, in which case the cull does nothing — but most do not.

use crate::assets::GameAssets;
use crate::axes;
use crate::render::focus::WorldFocus;
use vale_assets::{
    world::adt::{Adt, PlacedModel, TerrainLiquidDraw, TerrainMesh},
    adt_path,
    world::blp,
    tile_for_position,
    world::wmo::Liquid,
};
use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_resource::{AsBindGroup, ShaderType, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

/// Ground textures repeat eight times across a chunk. The mesh carries
/// chunk-local 0..1 UVs — which is also the alpha map's coordinate space — and
/// the shader scales them. One UV set, two uses.
const TEXTURE_REPEAT: f32 = 8.0;

/// How far out to load, in tiles, as a radius. 1 gives the 3x3.
///
/// The client's own number, and [`TerrainReach`]'s default. A host that wants
/// more ground resident sets that resource instead of changing this.
const RADIUS: i32 = 1;

/// How many tiles out to stream, as a radius, for a host that wants more
/// resident ground than a session does.
///
/// A session needs the 3x3 and nothing more: the character is in the middle of
/// it, the fog closes at 500 yards at the latest, and everything past the block
/// is drawn in the fog colour anyway. Somebody shaping a valley is in the
/// opposite position — they are looking at the ground from above with the fog
/// switched off, and the edge of the block is a cliff into nothing a few hundred
/// yards away.
///
/// So the number is a resource rather than a constant, defaulting to
/// [`RADIUS`]. Everything derived from it is derived through
/// [`Self::yards`] — the camera's far plane and the ground pick — so the three
/// cannot disagree.
///
/// What it costs is tiles, and the square is unforgiving: radius 1 is 9,
/// 2 is 25, 3 is 49. A tile is a 1024x1024 alpha atlas with its mips (5.3 MiB),
/// its tilesets and a couple of dozen draw groups, so the 7x7 is about five
/// times the resident cost of the 3x3 and takes about five times as long to
/// fill. [`HIGHEST`](Self::HIGHEST) is where the clamp is, and it is a bound on
/// what can be asked for rather than a recommendation.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainReach(pub i32);

impl Default for TerrainReach {
    fn default() -> TerrainReach {
        TerrainReach(RADIUS)
    }
}

impl TerrainReach {
    /// The largest radius this will stream: 11x11 tiles, 5,867 yards across.
    ///
    /// Chosen as the point where the arithmetic stops being a trade and starts
    /// being a mistake — 121 tiles is about 640 MiB of atlas alone — not as a
    /// measurement of what a machine can hold.
    pub const HIGHEST: i32 = 3;

    /// The radius actually used, clamped. Written out rather than `clamp`,
    /// which is not a `const fn`.
    pub const fn tiles(self) -> i32 {
        if self.0 < 0 {
            0
        } else if self.0 > Self::HIGHEST {
            Self::HIGHEST
        } else {
            self.0
        }
    }

    /// The furthest the loaded ground can be from the focus, in yards.
    ///
    /// From anywhere inside the middle tile the far corner of the block is
    /// `tiles + 1` tiles away along each axis. This is what the camera's far
    /// plane has to clear — a far plane short of it culls distant mountains
    /// outright and shows the sky through them — and what the ground pick walks
    /// to. See [`crate::world::camera`].
    pub const fn yards(self) -> f32 {
        (self.tiles() as f32 + 1.0) * vale_assets::world::adt::TILE_SIZE * std::f32::consts::SQRT_2
    }
}

/// How many bytes of finished geometry and texture to hand to the render
/// world in one frame.
///
/// A tile is one indivisible thing to parse and nine separable things to
/// upload — an alpha atlas, a handful of ground textures, a couple of dozen
/// group meshes and its water — and until this constant existed the hand-off
/// was "one whole tile a frame". For Elwynn's (32, 48) that is a 1024x1024
/// atlas with its mips (5.3 MiB), seven decoded tilesets (2.4 MiB) and 28 group
/// meshes over 196,608 indices (~2.8 MiB): about 10.5 MiB of `Assets::add` in
/// one frame, three frames running, every time the character crosses a tile
/// boundary. Everything added in a frame is uploaded by the render world in
/// that same frame, so the budget is really on the upload and the tile was
/// never the right unit for it.
///
/// The 1.12 client's own answer is the same shape and it is worth reading,
/// because it is the reason to believe the unit and not the size. Terrain there
/// is streamed at `MCNK` granularity into two buffers that already exist:
/// exactly two, one of vertices and one of indices, sized `2 · n²` chunks of
/// 145 vertices at 24 bytes and 768 `u16` indices — a
/// chunk's whole vertex and index count, which is where the granularity is
/// stated. `n` is the far clip in chunks plus one
/// (`1 - trunc(farclip · -0.03)`, and 1/0.03 is `CHUNK_SIZE`), so at the shipped
/// `farclip` default of 350 yards the pools are 242 chunks — 842 KiB and
/// 363 KiB, allocated once for the whole game. A chunk arriving is a
/// suballocation and a memcpy into a live buffer, never a buffer creation; the
/// buildings and the scenery are pooled the same way, free list and all.
///
/// Bevy owns buffer creation, so the pool itself is not adoptable here — its
/// `MeshAllocator` already slabs meshes into shared buffers, which is the same
/// idea one layer down. What is adoptable is the granularity: make the unit
/// of the hand-off small and spend a fixed amount of it per frame, which is
/// what this is.
///
/// Two megabytes is a choice and not a measurement, and it is stated as one:
/// it is about a fifth of a tile, so a crossing fills in over roughly fifteen
/// frames — a quarter of a second at 60 fps, at the outer ring of the 3x3 where
/// the ground is 530 yards away and behind the character. What it is sized
/// against is the frame, not the driver: a budget small enough to be invisible
/// per frame and large enough that the world is not visibly assembling itself.
const UPLOAD_BYTES: usize = 2 * 1024 * 1024;

/// …and a backstop on the number of instalments in one frame.
///
/// The bytes are the real budget — the render world's cost is the upload — but
/// each instalment is also an `Assets::add`, a material and an entity spawn,
/// and a tile whose groups happen to be small would spend the byte budget over
/// hundreds of them. Sixty-four is comfortably more than a whole tile's 28
/// groups at Elwynn or 46 at Duskwood, so it never binds in practice and is
/// there to bound the shape of the work rather than to shape it.
const UPLOAD_ITEMS: usize = 64;

/// The furthest the loaded ground can be from the character at the client's
/// own [`RADIUS`], in yards: 1,509 at the diagonal of the 3x3.
///
/// The two things that have to clear it — the camera's far plane and the ground
/// pick — read [`TerrainReach::yards`] instead, because a host may have asked
/// for more. This is what that answers with when nobody has.
pub const LOADED_REACH: f32 = TerrainReach(RADIUS).yards();

/// One tile's worth of decoded terrain, ready to become Bevy assets.
///
/// Produced off the main thread, and since this round the meshes and images in
/// it are finished — a `Mesh` or an `Image` is plain data until
/// `Assets::add` hands it to the render world, so everything that used to be
/// per-tile main-thread work (the group remaps, the atlas mips, the texture
/// chains) happens in the task, and the main thread's share of a tile is
/// handles and entities.
struct TileData {
    coord: (u32, u32),
    /// For the log line — the mesh itself is already cut into groups.
    triangles: usize,
    /// One prebuilt mesh per draw group, with the group's own texture set and,
    /// under [`LiveEdits`], where each of its vertices came from.
    groups: Vec<(vale_assets::world::adt::TerrainDraw, Mesh, Vec<u32>)>,
    /// The alpha atlas — a 16x16 grid of the chunks' 64x64 blend maps — as a
    /// finished image, mip chain included.
    atlas: Image,
    /// Decoded ground textures, parallel to the ADT's texture list.
    textures: Vec<Option<Image>>,
    /// The `MDDF` doodads this tile owns — see [`read_tile`] for what "owns"
    /// means, since a placement can appear in two tiles — each with whether
    /// its origin stands in the tile's baked `MCSH` shadow, sampled here
    /// because only the loader has the decoded chunks in hand.
    doodads: Vec<(PlacedModel, bool)>,
    /// The `MODF` buildings this tile owns, claimed the same way.
    wmos: Vec<PlacedModel>,
    /// The tile's own `MCLQ` liquid — the lakes, the rivers and the ocean —
    /// one prepared draw per kind with the surface's own centre (the light it
    /// is tinted by) and the first frame of its flipbook. Not claimed like the
    /// placements above: a chunk belongs to exactly one tile, so a lake
    /// crossing a seam is naturally split.
    liquids: Vec<crate::render::models::PreparedDraw>,
    /// The tile's ground effects: one plan per chunk that grows anything,
    /// and the CPU geometry of the models those plans name. Built here rather
    /// than on the main thread because it wants the `Adt` and an archive, and
    /// both are here — see [`crate::render::foliage`], which explains why it is
    /// a plan and not a position.
    foliage: crate::render::foliage::TileFoliage,
}

#[derive(Component)]
pub struct TerrainTile {
    pub coord: (u32, u32),
}

/// A tile's alpha atlas, on the tile root, for a host that changes what the
/// ground is painted with.
///
/// Only while [`LiveEdits`] is on, because it is only useful then: the image
/// is dropped from `Assets<Image>` after upload otherwise, so the handle would
/// name nothing. Writing to it is `vale_assets::world::adt`'s
/// `write_alpha_atlas_cell`, which is the one statement of the layout and of why
/// a chunk's cell can be minified on its own.
#[derive(Component)]
pub struct TileAlpha(pub Handle<Image>);

/// One drawn group of a tile's ground — the textured chunks, and nothing
/// else the tile carries.
///
/// A marker with no data, and it exists only so the ground can be turned off
/// on its own ([`crate::render::tuning`]). The tile root would have been the
/// obvious handle and is the wrong one: a doodad and a building are children of
/// the tile they stand on, so hiding the root hides the whole world and answers
/// nothing about which layer a shape came from.
#[derive(Component)]
pub struct TerrainGround;

/// Whether a tile is built so that its ground can be changed after it is on the
/// GPU.
///
/// Off in this crate, where a tile's contents do not change during a session.
/// A host that rewrites a chunk's heights and needs the ground to follow within
/// the frame turns it on.
///
/// It costs two things, and they are why it is a switch rather than the
/// default:
///
/// * the meshes are kept in the main world as well as the render world, because
///   a mesh with `RenderAssetUsages::RENDER_WORLD` alone is dropped from
///   `Assets<Mesh>` once it is uploaded and cannot be written to afterwards;
/// * each draw group carries a [`GroundSources`], four bytes per vertex.
///
/// Read when a tile is requested, so a change takes effect on the tiles loaded
/// after it: a host that wants it wants it set before anything is streamed.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct LiveEdits(pub bool);

/// Which tile vertex each of a draw group's vertices came from.
///
/// [`group_mesh`] renumbers a group's vertices so that the group holds only the
/// ones its own indices touch, which is what makes a draw group's buffer small
/// and is also what makes it impossible to find a chunk's vertices in it
/// afterwards. This is that renumbering, kept: `source[i]` is the index into the
/// tile-wide buffer `Adt::to_mesh` built, and `Adt::chunk_vertex_span` says
/// which chunk that index belongs to.
///
/// Present only under [`LiveEdits`].
#[derive(Component, Debug)]
pub struct GroundSources(pub Vec<u32>);

/// What colour the ground's sheen is this frame, as the `MeshTag` every
/// ground draw carries.
///
/// The sheen — `terrain1_s.bls`'s specular add, see `atmosphere.wgsl` — needs
/// one colour per frame: `Light.dbc`'s band 9 for the hour and the place. The
/// awkward part is transport, and it is worth writing down why this shape:
///
/// * not the material. `TerrainParams` used to carry a band and the whole
///   channel was deleted for it (see the note on that struct): the hour would
///   have to be written into every ground material whenever the clock moved,
///   which is a bind-group rebuild per draw group per tile.
/// * not the view bind group. The sun and the fill ride there because Bevy
///   has a `DirectionalLight` and an `AmbientLight` to put them in; there is no
///   third slot, and a second directional light standing in for one would be
///   indexed by whatever order the light query happened to iterate in.
/// * so the mesh tag, which is the per-instance `u32` Bevy already uploads
///   for every draw and which `models::RoomLight` already uses for a colour in
///   the same `0x00RRGGBB` packing. It is over-general — the value is the same
///   for every ground draw in the world — but it is the only channel that
///   costs nothing when it does not change, and 8 bits a channel is exact
///   here: the band is bytes in the file.
///
/// Written only when the packed value moves, which is what makes it cheap:
/// the atmosphere resolves several times a second while walking (every four
/// yards), and band 9 quantised to bytes changes a handful of times an hour.
/// See [`light_sheen`].
///
/// Black — which is what the switch writes — is "no sheen", and the shader
/// reads it as such.
fn sheen_tag(colour: [f32; 3]) -> u32 {
    let byte = |c: f32| u32::from((c.clamp(0.0, 1.0) * 255.0).round() as u8);
    byte(colour[0]) << 16 | byte(colour[1]) << 8 | byte(colour[2])
}

/// Keep every ground draw's tag on the current sheen colour.
///
/// The walk is every frame and the write is not, which is the half that
/// matters: `MeshTag` is change-detected, so a tag rewritten to the value it
/// already holds would re-upload every ground instance's mesh uniform sixty
/// times a second. Comparing first costs a `u32` per ground group — a few
/// hundred in the whole 3x3 — and writes only when the byte-packed band moves,
/// which is a handful of times an hour.
///
/// Walking unconditionally is also what makes a newly streamed group get its
/// colour: it spawns tagged zero and picks the current one up on the frame
/// after, without a second query for the added ones (two mutable queries over
/// the same component is a `B0001` panic, which is exactly how this was first
/// written).
fn light_sheen(
    sky: Res<crate::render::sky::Sky>,
    tuning: Res<crate::render::tuning::WorldTuning>,
    mut ground: Query<&mut bevy::mesh::MeshTag, With<TerrainGround>>,
) {
    let wanted = if tuning.specular {
        sheen_tag(sky.current.sun_halo)
    } else {
        0
    };
    for mut tag in &mut ground {
        if tag.0 != wanted {
            tag.0 = wanted;
        }
    }
}


/// …and one drawn surface of its `MCLQ` liquid, for the same reason and split
/// from the ground because it is a different surface with different open
/// questions against it. A building's `MLIQ` carries the same marker — see
/// [`crate::render::wmos`] — so one switch covers every drop of water on
/// screen.
///
/// It carries which of the four it is, because the marker is also what
/// [`crate::render::water`] finds a surface by, and that pass has to know which
/// flipbook and which pair of light bands this one takes. A tag with the answer
/// on it is one query; the alternative is a second component that could be
/// forgotten.
#[derive(Component)]
pub struct TerrainWater(pub Liquid);

/// One tile part-way through becoming entities.
///
/// A tile arrives as one parse and leaves as a dozen uploads, and this is
/// what carries the rest of them across the frames [`UPLOAD_BYTES`] spreads
/// them over. The root entity is spawned with the first instalment rather
/// than with the last, so the doodad and WMO passes — which have budgets of
/// their own — start their work while the ground under them is still arriving
/// rather than queueing behind it.
struct TileBuild {
    coord: (u32, u32),
    /// The tile root: what every group, every liquid surface, every doodad and
    /// every building on this tile is a child of.
    tile: Entity,
    /// The alpha atlas and the ground textures — as handles reserved at the
    /// start and filled in one at a time, which is what keeps the largest
    /// items of a tile from having to land in one frame together.
    ///
    /// `Assets::reserve_handle` costs an index and nothing else, so every
    /// group's material can name all five textures from the first instalment
    /// while the pixels arrive over the following frames. A material whose
    /// images are not there yet fails `AsBindGroup` with `RetryNextUpdate` and
    /// is simply not drawn until they are — which is the behaviour wanted:
    /// ground appears when its texture does, rather than the whole tile waiting
    /// on the atlas.
    atlas: Handle<Image>,
    /// …and the per-chunk tint, which is one kilobyte of nothing unless a host
    /// writes it. See [`TerrainMaterial::tint`].
    tint: Handle<Image>,
    ground: Vec<Handle<Image>>,
    /// …and the pixels still to go into them, drained from the back.
    pending_images: Vec<(Handle<Image>, Image)>,
    /// What is left to hand over, drained from the back — the order within a
    /// tile is nothing, since a group is a texture set rather than a place.
    groups: Vec<(vale_assets::world::adt::TerrainDraw, Mesh, Vec<u32>)>,
    liquids: Vec<crate::render::models::PreparedDraw>,
    /// For the finished line, which is what `vale textures <Map> <x> <y>`
    /// is compared against.
    triangles: usize,
    placements: usize,
    buildings: usize,
    liquid_draws: usize,
    spawned_groups: usize,
}

/// What one frame's hand-off cost, for the HUD.
///
/// The bytes are the measurement and the count of pieces is the corollary,
/// which is the way round it has to be: the budget is spent in bytes because
/// the render world's cost is the upload, and a frame that hands over one
/// atlas and a frame that hands over forty group meshes are the same size and
/// look nothing alike as counts.
///
/// [`Self::largest`] is the honest floor under the whole scheme and is reported
/// beside the budget for that reason: an `Assets::add` cannot be split, so the
/// worst frame is never smaller than the biggest single item a tile holds —
/// which is the alpha atlas, at 5.3 MiB with its mips, two and a half times the
/// budget on its own.
#[derive(Default, Clone, Copy)]
pub struct Handed {
    pub bytes: usize,
    pub images: usize,
    pub meshes: usize,
    pub largest: usize,
}

#[derive(Resource, Default)]
pub struct LoadedTiles {
    /// Tiles on screen or in flight, so a tile is never loaded twice.
    present: HashSet<(u32, u32)>,
    loading: Vec<Task<Option<TileData>>>,
    /// Finished loads waiting their turn — [`receive_tiles`] takes the one
    /// nearest the character first, because a crossing finishes up to three
    /// tiles in one burst and a login finishes nine, and task-completion order
    /// says nothing about which ground anybody is standing on.
    ready: Vec<TileData>,
    /// The tile currently being handed over, an instalment a frame.
    building: Option<TileBuild>,
    /// What the last frame handed over, and the largest single item this
    /// session — see [`Handed`].
    handed: Handed,
    peak_item: usize,
    /// Which tile the character was on when the block was last chosen.
    centre: Option<(u32, u32)>,
    /// …and how many tiles out it went — see [`TerrainReach`], which a host can
    /// change at any time.
    reach: Option<i32>,
    /// Which map those tiles came from.
    ///
    /// A far teleport changes the map without necessarily changing the tile —
    /// tile (37, 47) exists on both continents — so the centre alone cannot
    /// notice it, and the 3x3 would keep Azeroth's ground under a character
    /// standing in Kalimdor.
    map: Option<String>,
    /// Draw groups spawned since the process started — a running total for the
    /// log line and not a count of what is resident.
    pub draw_groups: usize,
}

impl LoadedTiles {
    /// Forget everything: the loaded set, the loads in flight, and the map the
    /// 3x3 came from.
    ///
    /// The entities are not this type's to despawn — the caller has the
    /// query — so this is only the bookkeeping half of tearing the world down.
    /// Both halves have to happen together: leaving `present` populated with a
    /// tile whose entity is gone is a tile that never loads again.
    ///
    /// See [`crate::render::residency::leave_world`], which is the one caller.
    pub fn forget(&mut self) {
        self.present.clear();
        self.loading.clear();
        self.ready.clear();
        // …and the tile part-way through arriving, whose root the caller has
        // just despawned: leaving it would parent the rest of its groups to an
        // entity that is gone.
        self.building = None;
        self.centre = None;
        self.map = None;
    }

    /// Forget one tile, so the next pass of [`request_tiles`] reads it again.
    ///
    /// The entity is not this type's to despawn, exactly as in [`Self::forget`],
    /// and the same pairing rule applies: a caller that forgets a tile without
    /// despawning it gets two copies of that tile's ground, one on top of the
    /// other. `centre` is cleared as well, because `request_tiles` returns early
    /// while the centre has not moved and would otherwise never look.
    ///
    /// Nothing in this crate calls it: a tile's contents do not change
    /// during a session. It is here for a host that has changed the bytes a
    /// tile's path reads back as and needs the ground rebuilt from them.
    pub fn reload(&mut self, coord: (u32, u32)) {
        self.present.remove(&coord);
        self.ready.retain(|data| data.coord != coord);
        if self.building.as_ref().is_some_and(|build| build.coord == coord) {
            self.building = None;
        }
        self.centre = None;
    }

    /// Which tile is part-way through being handed over, if one is.
    ///
    /// A tile arrives over several frames — its atlas, its ground textures and
    /// its draw groups are spread across them by [`UPLOAD_BYTES`] — so there is
    /// a window in which its root entity exists and most of it is not on it yet.
    /// This is that window, and it closes on the frame the last group lands.
    ///
    /// Nothing in this crate asks, like [`Self::reload`] beside it. It is for
    /// a host that is replacing a tile and wants to keep the old one on screen
    /// until the new one is whole: reloading despawns nothing by itself, so the
    /// two exist together and the question is when to swap. Without an answer
    /// the only options are a hole in the world for the third of a second the
    /// rebuild takes, or two copies of the same ground z-fighting.
    pub fn arriving(&self) -> Option<(u32, u32)> {
        self.building.as_ref().map(|build| build.coord)
    }

    /// How many of the tiles this pass is holding have not reached the
    /// screen: loads still in flight, finished loads waiting their turn, and
    /// the one part-way through being handed over.
    ///
    /// Read by [`crate::glue::loading`], which is the one thing that has to
    /// know whether the ground is there yet. It counts what is outstanding
    /// rather than what has spawned, which is what makes a tile whose ADT will
    /// not parse harmless: that tile leaves `loading` without ever arriving, so
    /// this reaches zero and the loading screen comes down, where a count of
    /// spawned tiles against wanted ones would wait for ever.
    pub fn settling(&self) -> usize {
        self.loading.len() + self.ready.len() + usize::from(self.building.is_some())
    }

    /// …and how many it wants altogether — the 3x3 around the character,
    /// clipped at the edges of the 64x64 grid, so between four and nine.
    ///
    /// Zero before the first [`request_tiles`] of a session or of a new map,
    /// which is what stops the moment before the streamer has noticed a
    /// teleport from reading as "everything has arrived".
    pub fn wanted(&self) -> usize {
        self.present.len()
    }

    /// Which map the tiles above belong to — the `Map.dbc` directory, the same
    /// string [`crate::world::session::WorldStatus::map_name`] carries.
    ///
    /// Exists for one race, and it is a real one: [`super::super::world`]'s poll
    /// adopts a far teleport's new map in the same schedule this pass streams
    /// in, so there is a frame in which the session says "Kalimdor" and the
    /// nine tiles on screen are still Azeroth's — settled, complete and about to
    /// be thrown away. Anything asking "has the world arrived" has to compare
    /// these two names or it will be told yes, once, on exactly that frame.
    pub fn map(&self) -> Option<&str> {
        self.map.as_deref()
    }
}

/// The bytes an `Assets::add` of this image will put on the GPU.
fn image_bytes(image: &Image) -> usize {
    image.data.as_ref().map_or(0, Vec::len)
}

/// …and of this mesh: its vertex buffer and its indices, which is what the
/// render world's allocator copies.
fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.get_vertex_buffer_size() + mesh.get_index_buffer_bytes().map_or(0, <[u8]>::len)
}

/// The ground material: an alpha atlas and up to four ground textures.
///
/// Bindless, exactly as [`crate::render::models::M2Material`] is, and for the
/// same measured reason. The batch-set key is
/// `(pipeline, draw function, material bind group index, mesh slab)`, and a
/// non-bindless material is its own bind group — so every draw group of every
/// tile was its own multidraw call: 45 of the 68 opaque calls in a settled
/// Elwynn frame were terrain, at ~14 µs of Vulkan encode each
/// (`VALE_DUMP_BINS=1`, 2026-08-25 — the M2 allocator packed 5,258 live
/// materials into 3 slabs in the same frame while the terrain sat at one call
/// per material). `#[bindless]` moves the five textures into shared binding
/// arrays and the params into one storage array, so the whole ground collapses
/// toward one batch set per slab. On a machine without bindless support the
/// `BINDLESS` def is absent and `terrain.wgsl`'s classic bindings are exactly
/// what they always were.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
#[data(0, TerrainParams, binding_array(13))]
#[bindless]
pub struct TerrainMaterial {
    pub params: TerrainParams,
    #[texture(1)]
    #[sampler(2)]
    pub alpha: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub layer0: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub layer1: Handle<Image>,
    #[texture(7)]
    #[sampler(8)]
    pub layer2: Handle<Image>,
    #[texture(9)]
    #[sampler(10)]
    pub layer3: Handle<Image>,
    /// A colour per map chunk, over the ground.
    ///
    /// A 16x16 image, one texel per chunk, sampled with the same `uv_b` the
    /// alpha atlas is — which already maps a vertex to its own chunk's cell of
    /// a 16x16 grid, with a half-texel inset that keeps every sample inside it.
    /// So a host can wash the ground chunk by chunk for the cost of one
    /// kilobyte and no geometry at all.
    ///
    /// Off in this crate: [`TerrainParams::tint`] is zero, and nothing
    /// samples this. It exists because a per-chunk field — which area a chunk
    /// is in, how many textures it carries, whether it has holes — has no
    /// picture of its own, and the only honest way to show one is on the ground
    /// it is about. See [`TileTint`].
    #[texture(11)]
    #[sampler(12)]
    pub tint: Handle<Image>,
}

/// The per-chunk tint image of one tile, on the tile root, so a host can
/// write it.
///
/// The sibling of [`TileAlpha`] and present on the same terms: only under
/// [`LiveEdits`], because an image with `RENDER_WORLD` alone is dropped from
/// `Assets<Image>` after upload and the handle would name nothing.
#[derive(Component)]
pub struct TileTint(pub Handle<Image>);

/// No band rides here any more, and that is the point. This used to carry
/// `Light.dbc`'s band 17 as the colour ground in a baked `MCSH` shadow was lit
/// by, which meant the hour had to be written by hand into every ground
/// material whenever the clock moved. The client's own `terrain1.bls` says the
/// shadow is a flat 30% dimming with no colour in it at all (see
/// `atmosphere.wgsl`), so the whole channel — the field, the writer, and the
/// per-hour walk over `Assets::iter_mut` — went with it. What is left is the
/// two numbers that really are per draw group.
#[derive(Clone, Copy, ShaderType)]
pub struct TerrainParams {
    pub layer_count: u32,
    pub repeat: f32,
    /// How much of [`TerrainMaterial::tint`] to show, 0 for none.
    ///
    /// Zero in every frame of the game. A host that wants the ground washed by
    /// the chunk writes the image, then raises this — on the materials of the
    /// tile it is about, which is where the fork has to be, since the image is
    /// per tile and the material is per draw group.
    pub tint: f32,
    pub _pad: f32,
    /// How fast each of the four layers crawls, in texture widths a second,
    /// packed two layers to a `Vec4` — `(u0, v0, u1, v1)` and `(u2, v2, u3, v3)`.
    ///
    /// `MCLY`'s animation bits, which is what makes the Burning Steppes lava
    /// move: see `vale_assets::world::adt::layer_flags::scroll`. It is a
    /// velocity rather than an offset so that the CPU writes it once when the
    /// tile is built and the shader multiplies it by the frame's own clock —
    /// the alternative, rewriting the material every frame, re-uploads the
    /// storage array sixty times a second for 164 layers in the whole of
    /// Azeroth.
    ///
    /// Zero on every draw group of every tile but the handful that carry the
    /// bits, and a zero velocity is the identity, so the branch is arithmetic
    /// rather than a `if`.
    pub scroll_a: Vec4,
    pub scroll_b: Vec4,
}

/// What `#[data]` extracts into the storage array — the whole of the
/// non-texture half, which is already one plain struct.
impl From<&TerrainMaterial> for TerrainParams {
    fn from(material: &TerrainMaterial) -> Self {
        material.params
    }
}

impl Material for TerrainMaterial {
    fn fragment_shader() -> ShaderRef {
        // Embedded rather than read from a runtime `assets/` directory: the
        // shader belongs to this crate, not to whichever directory the binary
        // happens to be started from.
        crate::render::shader::TERRAIN.into()
    }
}

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/terrain.wgsl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default())
            .init_resource::<LoadedTiles>()
            // Off, so the client pays nothing for it — see [`LiveEdits`].
            .init_resource::<LiveEdits>()
            // The 3x3, unless a host has already inserted its own — see
            // [`TerrainReach`]. `init_resource` keeps whatever is there.
            .init_resource::<TerrainReach>()
            .add_systems(
                Update,
                // `light_sheen` after the spawner, so a group spawned this
                // frame carries the hour's colour on the frame it appears
                // rather than one frame of matte ground.
                (request_tiles, receive_tiles, light_sheen).chain(),
            )
            // The two layers this pass draws, each switchable on its own — see
            // [`crate::render::tuning`]. Nothing else writes either one's
            // visibility, so both take the generic shape.
            .add_systems(
                Update,
                (
                    crate::render::tuning::switch::<TerrainGround>(|tuning| tuning.terrain),
                    crate::render::tuning::switch::<TerrainWater>(|tuning| tuning.water),
                ),
            );
        // …and what the streamer is doing, which is the one number that says
        // whether [`UPLOAD_BYTES`] is being obeyed and what the biggest
        // indivisible item under it costs. See `ui::report` for why the line is
        // written here rather than in `hud.rs`.
        #[cfg(feature = "diagnostics")]
        app.add_systems(
            Update,
            report
                .after(receive_tiles)
                .run_if(crate::ui::report::watched),
        );
    }
}

/// [`crate::ui::report::HudReport`] slot. See `ui::report`.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(15);

/// The HUD line: what is resident, what is queued, and what the last frame's
/// hand-off cost.
#[cfg(feature = "diagnostics")]
fn report(
    loaded: Res<LoadedTiles>,
    mut hud: ResMut<crate::ui::report::HudReport>,
    tiles: Query<&TerrainTile>,
) {
    let kib = |bytes: usize| bytes / 1024;
    let building = match &loaded.building {
        Some(build) => format!(
            "({}, {}) {} groups left",
            build.coord.0,
            build.coord.1,
            build.groups.len()
        ),
        None => "idle".to_string(),
    };
    hud.set(
            crate::ui::report::Section::Scene,
        SLOT,
        "terrain",
        format!(
            "tiles: {} resident, {} loading, {} ready, {building}  \
             (handed {} KiB / {} images / {} meshes this frame, budget {} KiB, \
             largest item {} KiB)",
            tiles.iter().count(),
            loaded.loading.len(),
            loaded.ready.len(),
            kib(loaded.handed.bytes),
            loaded.handed.images,
            loaded.handed.meshes,
            kib(UPLOAD_BYTES),
            kib(loaded.peak_item),
        ),
    );
}

/// Load the 3x3 around the character, and drop anything outside it.
fn request_tiles(
    focus: Res<WorldFocus>,
    live: Res<LiveEdits>,
    reach: Res<TerrainReach>,
    assets: Res<GameAssets>,
    mut loaded: ResMut<LoadedTiles>,
    mut commands: Commands,
    // Every tile but the one that is not one. A map with no ADTs hosts its
    // global WMO on a `TerrainTile` of its own so that the building pass can
    // find it (see [`crate::render::globalwmo`]) — and it sits at a coordinate
    // the 3x3 below can never want, so without this exclusion the streamer
    // despawns it on the first frame and again every time one is spawned.
    tiles: Query<(Entity, &TerrainTile), Without<crate::render::globalwmo::GlobalWmo>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::TerrainRequest);
    if !focus.present {
        return;
    }
    let (cx, cy) = tile_for_position(focus.position.x, focus.position.y);
    // The map is checked as well as the centre: a far teleport can land on the
    // same tile coordinates on another continent, and dropping the tiles is
    // not enough on its own either — a load already in flight would arrive
    // holding the old map's geometry and be spawned as though it belonged.
    let changed_map = loaded.map.as_deref() != Some(focus.map_name.as_str());
    // And the reach itself, which a host can change while a map is open.
    // Without it, widening the block does nothing until the focus crosses a
    // tile boundary — and narrowing it leaves every tile outside the new one on
    // screen for as long as nobody walks.
    let changed_reach = loaded.reach != Some(reach.tiles());
    if loaded.centre == Some((cx, cy)) && !changed_map && !changed_reach {
        return;
    }
    loaded.reach = Some(reach.tiles());
    if changed_map {
        loaded.map = Some(focus.map_name.clone());
        loaded.loading.clear();
        // A finished load from the old map must not spawn on the new one — and
        // neither must the rest of a tile that was half-way through arriving,
        // whose root is despawned by the loop below.
        loaded.ready.clear();
        loaded.building = None;
        loaded.present.clear();
        for (entity, _) in &tiles {
            commands.entity(entity).despawn();
        }
    }
    loaded.centre = Some((cx, cy));

    let mut wanted: HashSet<(u32, u32)> = HashSet::default();
    let radius = reach.tiles();
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let x = cx as i32 + dx;
            let y = cy as i32 + dy;
            if (0..64).contains(&x) && (0..64).contains(&y) {
                wanted.insert((x as u32, y as u32));
            }
        }
    }

    // Drop tiles that have walked out of range. Skipped after a map change,
    // which has already despawned all of them — asking twice in one frame is
    // Bevy's "despawning an entity that does not exist" warning.
    if !changed_map {
        for (entity, tile) in &tiles {
            if !wanted.contains(&tile.coord) {
                commands.entity(entity).despawn();
                loaded.present.remove(&tile.coord);
            }
        }
        // …and the finished loads that walked out of range before their turn
        // came. The loop above cannot see them: a `ready` tile has no
        // entity yet, so without this it stays in `present`, is handed over
        // when its turn comes, and is despawned on the next crossing — a whole
        // tile's upload spent on ground the character has already left. It
        // costs nothing to notice, and the budget above is what makes the
        // window wide enough for it to happen.
        let mut stale = Vec::new();
        loaded.ready.retain(|data| {
            if wanted.contains(&data.coord) {
                true
            } else {
                stale.push(data.coord);
                false
            }
        });
        for coord in stale {
            loaded.present.remove(&coord);
        }
        // The tile part-way through arriving is the one case that needs no
        // `present` bookkeeping: its root carries `TerrainTile`, so the loop
        // above has already despawned it and forgotten it.
        if loaded
            .building
            .as_ref()
            .is_some_and(|build| !wanted.contains(&build.coord))
        {
            loaded.building = None;
        }
    }

    let map = focus.map_name.clone();
    for coord in wanted {
        if !loaded.present.insert(coord) {
            continue;
        }
        let map = map.clone();
        let dir = assets.gamedata_dir.clone();
        // The overlay travels with the folder name. This task opens a chain
        // of its own (see `read_tile`), and a chain opened without the overlay
        // answers the archives' bytes for a path a host is overriding — which
        // is an edited tile that reloads as though it had never been edited.
        let overlay = assets.overlay();
        let live = live.0;
        // Borrowed rather than opened. Opening the chain is 45 ms against
        // the 27 ms this task spends on the tile itself, and what it opens does
        // not change while the process runs — see
        // `vale_assets::archive::ChainPool`.
        let chains = assets.chains();
        // Parsed once for the session and shared with every tile task — the
        // texture table is 357 KB and nine of these start at once on a login.
        let effects = assets.ground_effects();
        loaded.loading.push(
            AsyncComputeTaskPool::get()
                .spawn(async move {
                    read_tile(&chains, &dir, &map, coord, &effects, overlay, live)
                }),
        );
    }
}

/// One ground texture, preferring the `_s` variant — the same file plus the
/// gloss mask the sun's sheen rides on.
///
/// `vale_assets::world::adt::specular_texture` is the name and the client's own
/// rule; what is decided here is the two things it does not state.
///
/// It is loaded whether or not the sheen is switched on, where the client
/// loads it only under `specular` (see that function's note).
/// The reason is the switch: `WorldTuning` is a runtime subtraction and a
/// tile is loaded once, so keying the load on it would mean a tile streamed
/// with the sheen off had no mask when it came back on — and a mask that is
/// missing reads as 1 on a DXT1 texture, which is a full-strength
/// highlight over the whole tile rather than none. It costs the alpha plane:
/// `DuskwoodCobblestone` is 44,876 bytes against `_s`'s 88,580, so a tile's
/// eleven tilesets are about half a megabyte more.
///
/// And a tileset with no `_s` variant is matte, by force. Its base is DXT1
/// with `alphaDepth` 0, which samples as alpha 1 — the same full-strength
/// highlight — so the alpha is written to zero rather than left as the
/// decoder's. That is a reading of the reference ("matte where no mask") and
/// it is the conservative side: a missing mask draws no sheen
/// instead of the most sheen in the world.
fn ground_texture(
    archive: &mut vale_assets::Assets,
    name: &str,
) -> Option<crate::render::models::RawTexture> {
    let specular = vale_assets::world::adt::specular_texture(name);
    if let Some(blp) = archive.read(&specular).ok().and_then(|raw| blp::decode_mipped(&raw).ok()) {
        return Some(crate::render::models::RawTexture::from_blp(blp));
    }
    let mut texture =
        crate::render::models::RawTexture::from_blp(blp::decode_mipped(&archive.read(name).ok()?).ok()?);
    for level in &mut texture.levels {
        for texel in level.chunks_exact_mut(4) {
            texel[3] = 0;
        }
    }
    Some(texture)
}

/// Parse one tile and decode its textures. Runs off the main thread — a tile is
/// 256 chunks and its tileset is a handful of BLPs, which is milliseconds of
/// work that would otherwise be a visible hitch on every tile crossing.
///
/// Reads on a chain of its own rather than locking the shared one, for the same
/// reason the session's ground lookup does: tile loads and the session should
/// not queue behind each other. The chain is borrowed from
/// [`vale_assets::archive::ChainPool`] and not opened — opening seventeen
/// archives costs more than reading and parsing the tile they are opened for,
/// and this runs once per tile.
fn read_tile(
    chains: &vale_assets::archive::ChainPool,
    gamedata_dir: &str,
    map: &str,
    coord: (u32, u32),
    effects: &vale_assets::tables::foliage::GroundEffects,
    overlay: Option<vale_assets::archive::Overlay>,
    live: bool,
) -> Option<TileData> {
    let mut archive = chains.take(gamedata_dir).ok()?;
    archive.set_overlay(overlay);
    let raw = archive.read(&adt_path(map, coord.0, coord.1)).ok()?;
    let adt = Adt::parse(&raw).ok()?;

    let textures = adt
        .texture_names
        .iter()
        .map(|name| {
            // Mipped: a ground texture repeats eight times across a 33-yard
            // chunk, so it is the most heavily minified thing on screen and the
            // one the aliasing showed on first. Built into an `Image` here —
            // through the model image builder, so the ground samples exactly
            // as a model does — because the chain concatenation is a memcpy the
            // main thread should not make.
            Some(ground_texture(&mut archive, name)?.into_image())
        })
        .collect();

    // A doodad that touches two tiles is listed in both tiles' `MDDF` with the
    // same `unique_id`, so it has to be claimed by exactly one of them or every
    // tree along every seam is drawn twice. The tile containing the placement's
    // own origin claims it: that tile certainly lists it (it touches it), and
    // the rule needs no state shared between tiles, so a tile walking out of
    // range takes its doodads with it and leaves nobody else's behind.
    let claimed = |p: &PlacedModel| tile_for_position(p.position[0], p.position[1]) == coord;
    // Each claimed doodad's origin is on this tile by construction, so the
    // tile's own chunks answer whether it stands in the baked `MCSH` shadow —
    // the bit that picks its per-instance sun scale (see `models::sun_scale`).
    let doodads = adt
        .placed_doodads()
        .into_iter()
        .filter(&claimed)
        .map(|p| {
            let shadowed = adt.shadowed_at(p.position[0], p.position[1]);
            (p, shadowed)
        })
        .collect();
    // Buildings are claimed by the same rule, and it matters more for them: a
    // church straddling a seam is in both tiles' `MODF`, and drawing it twice
    // costs a whole second copy of the largest thing in the world and z-fights on
    // every wall of it.
    let wmos = adt.placed_wmos().into_iter().filter(&claimed).collect();

    // The mesh only. The liquid's texture is not this tile's business: every
    // surface of a kind in the world shares one flipbook, read once for the
    // session by [`crate::render::water`] — so a tile that decoded its own copy
    // of `lake_a.1` was doing 90 KB of archive read and a DXT decode to produce
    // an image identical to eight other tiles', under an `AssetId` of its own
    // that split the material every one of them should have shared.
    let liquids = adt
        .liquid_surface()
        .into_iter()
        .map(|draw| crate::render::models::PreparedDraw::from_raw(liquid_raw_draw(&draw)))
        .collect();

    // The group meshes, cut and remapped here: a tile is ~196k indices through
    // a `HashMap` per group, which was a visible slice of the frame that
    // received it.
    let mesh = adt.to_mesh();
    let triangles = mesh.triangle_count();
    let groups = mesh
        .draws
        .iter()
        .filter_map(|draw| {
            let (group, sources) = group_mesh(&mesh, draw.index_start, draw.index_count, live)?;
            Some((draw.clone(), group, sources))
        })
        .collect();

    // The ground effects, last: it reads a handful of tiny M2s of its own and
    // wants the chunks the parse above already produced.
    let foliage = crate::render::foliage::read_tile_foliage(&mut archive, &adt, effects);

    Some(TileData {
        coord,
        triangles,
        groups,
        atlas: atlas_image(&adt.alpha_atlas(), live),
        textures,
        doodads,
        wmos,
        liquids,
        foliage,
    })
}

/// Which finished load to hand over next: the one nearest the character.
///
/// The tasks run on the compute pool and finish in whatever order the archive
/// and the parse take, which says nothing about where anybody is standing. A
/// login or a teleport finishes all nine at once, and the tile that matters is
/// the one under the character's feet — with the hand-off spread over frames,
/// arrival order is now visible, where under "one whole tile a frame" the whole
/// 3x3 landed in nine frames whatever the order was.
///
/// Ties are broken by position in the list, which is arrival order — there is
/// nothing to choose between two tiles the same distance away.
///
/// Takes the coordinates rather than the resource so that the rule can be
/// checked without building a `TileData`, which is a tile's worth of meshes and
/// images.
fn nearest_index(
    coords: impl Iterator<Item = (u32, u32)>,
    centre: Option<(u32, u32)>,
) -> Option<usize> {
    let mut coords = coords.enumerate();
    let Some(centre) = centre else {
        // No 3x3 has been chosen yet, so there is nothing to be near: first
        // come, first served.
        return coords.next().map(|(index, _)| index);
    };
    coords
        .min_by_key(|&(_, coord)| {
            let dx = coord.0 as i32 - centre.0 as i32;
            let dy = coord.1 as i32 - centre.1 as i32;
            dx * dx + dy * dy
        })
        .map(|(index, _)| index)
}

/// Turn finished tile loads into entities — [`UPLOAD_BYTES`] worth a frame.
///
/// The meshes, the atlas and the texture images arrive prebuilt from the task,
/// so what happens here is `Assets::add`, the materials and the entity spawns.
/// Everything added in one frame is uploaded by the render world in that same
/// frame, so the budget is really on the upload — which is why it is counted in
/// bytes and why a tile is handed over in instalments rather than whole. See
/// [`UPLOAD_BYTES`], which carries the 1.12 client's own answer to the same
/// question.
///
/// The first item of a frame is always handed over, whatever it costs. An
/// `Assets::add` cannot be split and the alpha atlas is 5.3 MiB on its own —
/// two and a half budgets — so a hard budget would stall on it for ever. What
/// the budget can do is make sure it is the only thing that frame pays for,
/// which is why the textures are reserved handles filled in one at a time
/// rather than a block uploaded with the root: the floor under the whole scheme
/// is the largest single item and nothing else, and the HUD reports that number
/// beside the budget so the two cannot be confused.
fn receive_tiles(
    mut loaded: ResMut<LoadedTiles>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    // The liquid draws go through the model material, exactly as a WMO's
    // canal does — same blend table, same `Light.dbc` tint — so this pass
    // needs that material's pool beside its own.
    mut liquid_materials: crate::render::models::Materials,
    // …and the one colour every liquid on screen is drawn in, so a surface is
    // right on the frame it appears rather than on the one after. `Option`
    // because the headless harnesses build worlds with no render plugins.
    palette: Option<Res<crate::render::water::LiquidPalette>>,
    // …and the flipbook they are drawn with, for the same reason and on the
    // same terms.
    flipbook: Option<Res<crate::render::water::LiquidFlipbook>>,
    // …and where a tile's foliage plans put the models they name, so a zone's
    // handful of grasses is read once rather than once per tile.
    mut foliage_models: ResMut<crate::render::foliage::FoliageModels>,
    assets: Res<crate::assets::GameAssets>,
    // Whether the atlas handle is worth keeping on the tile — see [`TileAlpha`].
    live: Res<LiveEdits>,
    // The hulls, for a tile that is being read again; `Option` because the
    // headless harnesses build worlds without the session's resources.
    solids: Option<Res<crate::world::session::Solids>>,
    focus: Res<crate::render::focus::WorldFocus>,
    mut commands: Commands,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::TerrainReceive);
    let mut finished: Vec<TileData> = Vec::new();
    loaded.loading.retain_mut(|task| {
        match block_on(future::poll_once(task)) {
            Some(Some(data)) => {
                finished.push(data);
                false
            }
            // A tile that will not parse costs that tile, not the session.
            Some(None) => false,
            None => true,
        }
    });
    loaded.ready.append(&mut finished);

    let ground_effects = assets.ground_effects();
    let mut handed = Handed::default();
    // Accumulated rather than written straight into `loaded.draw_groups`,
    // because `loaded.building.as_mut()` borrows the whole resource.
    let mut new_groups = 0usize;
    let mut items = 0usize;

    while (handed.bytes == 0 || handed.bytes < UPLOAD_BYTES) && items < UPLOAD_ITEMS {
        items += 1;
        // Start a tile: the root entity and the texture handles, which
        // costs no bytes at all. The pixels behind them are instalments like
        // everything else — see [`TileBuild::pending_images`].
        if loaded.building.is_none() {
            let Some(index) =
                nearest_index(loaded.ready.iter().map(|data| data.coord), loaded.centre)
            else {
                break;
            };
            let TileData {
                coord,
                triangles,
                groups,
                atlas,
                textures,
                doodads,
                wmos,
                liquids,
                foliage,
            } = loaded.ready.remove(index);

            let mut pending_images = Vec::with_capacity(1 + textures.len());
            let atlas_handle = images.reserve_handle();
            pending_images.push((atlas_handle.clone(), atlas));
            // One texel per chunk, transparent, which costs a kilobyte and
            // draws nothing: `TerrainParams::tint` is zero unless a host raises
            // it. Reserved beside the atlas rather than made lazily, so that a
            // material naming it is valid from the first instalment.
            let tint_handle = images.reserve_handle();
            pending_images.push((tint_handle.clone(), tint_image(live.0)));
            let ground: Vec<Handle<Image>> = textures
                .into_iter()
                .map(|decoded| match decoded {
                    Some(image) => {
                        let handle = images.reserve_handle();
                        pending_images.push((handle.clone(), image));
                        handle
                    }
                    // The magenta stand-in is small and shared with every other
                    // pass that fails to resolve a texture, so it goes in
                    // outright rather than through the queue.
                    None => images.add(crate::render::models::missing_image()),
                })
                .collect();

            let placements = doodads.len();
            let buildings = wmos.len();
            let liquid_draws = liquids.len();
            // A tile read again by a host (`LoadedTiles::reload`) still holds
            // the hulls its first reading inserted, keyed by placement id, and
            // the passes below insert the current placements over them. A
            // placement the re-read lists no longer name would keep its hull
            // until the tile walked out of range, so those are dropped here,
            // before the current ones are handed over. A first arrival holds
            // nothing under this coordinate and this is one lookup.
            if let Some(solids) = &solids {
                let keep: std::collections::HashSet<u32> = doodads
                    .iter()
                    .map(|(placed, _)| placed.unique_id)
                    .chain(wmos.iter().map(|placed| placed.unique_id))
                    .collect();
                solids.0.retain_placements(focus.map_id, coord, &keep);
            }
            let tile = commands
                .spawn((
                    TerrainTile { coord },
                    // The doodad and WMO passes drain these as each model
                    // finishes loading. They hang off the tile so that
                    // despawning it takes them too — and so a building's own
                    // furniture, which the WMO pass appends to the doodad
                    // list, goes with it.
                    crate::render::doodads::PendingDoodads {
                        // The sun lights these; only the WMO pass's furniture
                        // arrives with a light of its own. The bool is the
                        // baked `MCSH` bit under each origin, which picks the
                        // sun scale.
                        outdoor: doodads
                            .into_iter()
                            .map(|(p, shadowed)| {
                                crate::render::doodads::PlacedDoodad::on_ground(p, shadowed)
                            })
                            .collect(),
                        // Filled by the WMO pass as each building arrives — a
                        // `MODF` names one doodad set by index, so which
                        // furniture exists is not known until the root has
                        // been read.
                        indoor: Vec::new(),
                    },
                    // What the tile ends up actually drawing, counted as it
                    // does — the cross-check against `vale models`.
                    crate::render::doodads::TileDoodads::default(),
                    crate::render::doodads::ResidentDoodads::default(),
                    crate::render::wmos::PendingWmos(wmos),
                    // …and the ground's own plants, which are neither a
                    // placement nor a building: one plan per chunk, built into
                    // merged meshes as the camera comes near each. See
                    // [`crate::render::foliage`].
                    crate::render::foliage::plans_of(
                        foliage,
                        &mut foliage_models,
                        &ground_effects,
                    ),
                    Transform::default(),
                    Visibility::default(),
                ))
                .id();
            // …and the atlas itself, for a host that repaints a chunk. Only
            // while the switch is on: see [`TileAlpha`], whose handle names
            // nothing once the image has been dropped after upload.
            if live.0 {
                commands.entity(tile).insert((
                    TileAlpha(atlas_handle.clone()),
                    TileTint(tint_handle.clone()),
                ));
            }

            loaded.building = Some(TileBuild {
                coord,
                tile,
                atlas: atlas_handle,
                tint: tint_handle,
                ground,
                pending_images,
                groups,
                liquids,
                triangles,
                placements,
                buildings,
                liquid_draws,
                spawned_groups: 0,
            });
            continue;
        }

        let build = loaded.building.as_mut().expect("just checked");
        // The pixels first: the ground under the character is what the
        // character is looking at, and a group whose textures have not arrived
        // is not drawn at all.
        if let Some((handle, image)) = build.pending_images.pop() {
            let bytes = image_bytes(&image);
            handed.bytes += bytes;
            handed.largest = handed.largest.max(bytes);
            handed.images += 1;
            // The handle was reserved by this system a few frames ago and
            // nothing else can have taken its index, so the only way this
            // fails is a bug here.
            let _ = images.insert(handle.id(), image);
            continue;
        }
        if let Some((draw, mesh, sources)) = build.groups.pop() {
            let pick = |slot: usize| -> Handle<Image> {
                draw.textures
                    .get(slot)
                    .and_then(|&i| build.ground.get(i as usize))
                    .cloned()
                    .unwrap_or_else(|| build.ground.first().cloned().unwrap_or_default())
            };
            let material = materials.add(TerrainMaterial {
                params: TerrainParams {
                    layer_count: draw.textures.len().min(4) as u32,
                    repeat: TEXTURE_REPEAT,
                    // Off. See [`TerrainParams::tint`].
                    tint: 0.0,
                    _pad: 0.0,
                    scroll_a: pair(&draw.scrolls, 0),
                    scroll_b: pair(&draw.scrolls, 2),
                },
                alpha: build.atlas.clone(),
                tint: build.tint.clone(),
                layer0: pick(0),
                layer1: pick(1),
                layer2: pick(2),
                layer3: pick(3),
            });
            let bytes = mesh_bytes(&mesh);
            handed.bytes += bytes;
            handed.largest = handed.largest.max(bytes);
            handed.meshes += 1;
            let ground = commands
                .spawn((
                    TerrainGround,
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(material),
                    // Filled by [`light_sheen`] on the frame after this one — the
                    // colour is the hour's, which this system does not hold. Zero
                    // until then, which is one frame of ground with no sheen on it.
                    bevy::mesh::MeshTag(0),
                    ChildOf(build.tile),
                ))
                .id();
            if !sources.is_empty() {
                commands.entity(ground).insert(GroundSources(sources));
            }
            build.spawned_groups += 1;
            new_groups += 1;
            continue;
        }

        // The tile's own water, drawn exactly as a WMO's is: the mesh carries
        // `MCLQ`'s depth byte as its vertex alpha, the texture is one frame of
        // the flipbook, and the colour comes off `Light.dbc`.
        if let Some(prepared) = build.liquids.pop() {
            let tile = build.tile;
            // The whole world's water is one colour, and it is the light where
            // the camera is — see [`crate::render::water`], which resolves it
            // and which keeps it current from here on.
            //
            // This used to be resolved per surface, at the draw's own mean
            // position: a `MCLQ` draw is per tile per kind, so a lake crossing an
            // ADT boundary was two draws 533 yards apart, and two centres either
            // side of a `Light.dbc` falloff sphere gave the two halves different
            // colours — a straight line down the middle of the water, on the tile
            // seam. Nothing else in the frame changes on a 533-yard grid.
            //
            // A draw with no kind on it cannot happen — `liquid_raw_draw` puts
            // one on every one of them — but the field is an `Option` for the
            // rest of the model pass, so water is the fallback rather than a
            // surface that quietly stops being liquid.
            let kind = prepared.liquid.unwrap_or(Liquid::Water);
            let tint = palette.as_ref().and_then(|p| p.tint(kind));
            // …and the flipbook's anchor frame, which every surface of the
            // kind is interned against so they come out as one material — see
            // [`crate::render::water::LiquidFlipbook::anchor`], which is where
            // the cost of using the frame that happens to be showing is.
            // `water::dress` puts the showing frame on that one material.
            // Magenta only when the pass is not there at all, which is the
            // headless harnesses: a reserved handle whose pixels have not landed
            // yet is not drawn rather than drawn wrong.
            let image = flipbook
                .as_ref()
                .and_then(|b| b.anchor(kind))
                .unwrap_or_else(|| images.add(crate::render::models::missing_image()));
            let bytes = prepared.mesh.get_vertex_buffer_size()
                + prepared
                    .mesh
                    .get_index_buffer_bytes()
                    .map_or(0, <[u8]>::len);
            let uploaded = crate::render::models::upload_prepared(
                prepared,
                image,
                crate::render::wmos::WMO_ALPHA_KEY,
                Vec3::ZERO,
                tint,
                &mut meshes,
                &mut liquid_materials,
            );
            handed.bytes += bytes;
            handed.largest = handed.largest.max(bytes);
            handed.meshes += 1;
            commands.spawn((
                TerrainWater(kind),
                Mesh3d(uploaded.mesh),
                MeshMaterial3d(uploaded.material),
                ChildOf(tile),
            ));
            continue;
        }

        // Nothing left of it: the line, and on to the next tile if the budget
        // has anything left.
        let build = loaded.building.take().expect("just checked");
        // Directly comparable with `vale textures <Map> <x> <y>`, which
        // reports the same grouping from the same function: a tile is 256
        // chunks and should come out as a few dozen draws, not 256.
        // `placements` is what `vale models <Map> <x> <y>` counts and
        // `buildings` what `vale wmos <Map> <x> <y>` counts, each minus the
        // ones this tile only overhangs — so both should be a little under the
        // CLI's numbers and never over them. `liquid_draws` is at most four —
        // one per kind the tile holds — and `vale water`'s terrain section
        // reports the same surfaces from the same `liquid_surface` call.
        info!(
            "tile ({}, {}): {} chunks -> {} draw groups, {} triangles, \
             {} doodads, {} buildings, {} liquid draws",
            build.coord.0,
            build.coord.1,
            256,
            build.spawned_groups,
            build.triangles,
            build.placements,
            build.buildings,
            build.liquid_draws,
        );
    }

    loaded.draw_groups += new_groups;
    loaded.peak_item = loaded.peak_item.max(handed.largest);
    loaded.handed = handed;
}

/// One terrain liquid surface as the model pass's [`crate::render::models::RawDraw`],
/// in Bevy's axes.
///
/// The parameters are the WMO liquid draw's, verbatim (`WmoModel::add_liquid`):
/// water and ocean blend (mode 2) and are lit, magma and slime are opaque
/// (mode 0) and unlit because they glow, everything is two-sided because a
/// surface is seen from underneath as soon as anything swims. The vertex
/// colours survive whole — their alpha is `MCLQ`'s depth byte, which the
/// fragment mixes the shallow and deep opacities with.
///
/// What is not here any more is a position. This used to compute the mean of
/// the draw's own vertices and resolve `Light.dbc`'s positional half against it,
/// which is what put a straight tint seam down the middle of every lake that
/// crosses an ADT boundary — see [`crate::render::water`], which resolves one
/// colour for the whole world at the camera instead.
fn liquid_raw_draw(draw: &TerrainLiquidDraw) -> crate::render::models::RawDraw {
    let translucent = matches!(draw.kind, Liquid::Water | Liquid::Ocean);
    crate::render::models::RawDraw {
        positions: draw
            .positions
            .iter()
            .map(|&p| axes::to_bevy(p).to_array())
            .collect(),
        // Flat and up: a liquid surface has no MCNR of its own.
        normals: vec![axes::to_bevy([0.0, 0.0, 1.0]).to_array(); draw.positions.len()],
        uvs: draw.uvs.clone(),
        colours: draw
            .colours
            .iter()
            .map(|c| c.map(|v| v as f32 / 255.0))
            .collect(),
        indices: draw.indices.clone(),
        joints: Vec::new(),
        weights: Vec::new(),
        // No skeleton, so no subset — see [`RawDraw::bones`].
        bones: Vec::new(),
        // …and no folded layers: an environment map over the same triangles
        // is an M2 authoring pattern. See [`RawDraw::overlays`].
        overlays: Default::default(),
        geoset: 0,
        // The handle is passed to `upload_prepared` directly; the index is only
        // honest bookkeeping here.
        texture: Some(0),
        blend: if translucent { 2 } else { 0 },
        unlit: !translucent,
        two_sided: true,
        no_depth_write: false,
        light: vale_assets::world::wmo::BatchLight::Sun,
        liquid: Some(draw.kind),
        // `M2Color` is an M2 block; a liquid surface has no batch to tint.
        tint: None,
        baked_tint: None,
        // `M2TextureTransform` is an M2 block; the ground has none.
        uv: None,
        // The ground does not need projecting onto itself.
        ground: None,
    }
}

/// One draw group as its own mesh, with the vertices it uses remapped to a
/// buffer of its own.
///
/// The remap is what lets each group carry an `Aabb` Bevy can cull, and it is
/// lossless: every index in the range appears exactly once in the output, so the
/// triangle count is unchanged. There is a test for that, because a wrong remap
/// draws some other part of the tile rather than failing.
///
/// The winding is reversed here, and it is not cosmetic. `Adt::to_mesh`
/// emits each cell as four `(centre, corner[i], corner[i+1])` triangles, and
/// since a chunk's cells run in decreasing x and y that order winds clockwise
/// seen from above — a geometric normal pointing at the ground. The WebGL
/// renderer never noticed because its terrain pass never enabled `CULL_FACE` and
/// so drew the ground double-sided. Bevy back-face culls by default, which culls
/// every surface facing the camera and leaves only the far slopes of hills
/// visible: the whole world reads as though it were being viewed from
/// underneath. Reversing costs nothing and makes the cull do useful work.
/// Does this tile carry any vertex shading at all?
///
/// `MCCV` is absent from every chunk of every shipped 1.12 tile, so the honest
/// answer for the client is always no and the attribute is never uploaded. A
/// host editing the ground uploads it regardless (`live`), because a stroke has
/// to have somewhere to write before it has written anything.
///
/// The walk is over the whole tile's colours once per draw group, which is
/// 80,000 comparisons on a tile that has none — measured against the alternative
/// of carrying a flag on `TerrainMesh`, it is lost in the tile load and it
/// cannot go stale.
fn shaded(source: &TerrainMesh) -> bool {
    source.colours.iter().any(|c| *c != [1.0, 1.0, 1.0, 1.0])
}

fn group_mesh(
    source: &TerrainMesh,
    index_start: u32,
    index_count: u32,
    live: bool,
) -> Option<(Mesh, Vec<u32>)> {
    let range = index_start as usize..(index_start + index_count) as usize;
    let indices = source.indices.get(range)?;
    if indices.is_empty() {
        return None;
    }
    // An index past the end of the vertex arrays would panic in the remap below,
    // and these are twenty-year-old files out of patched archives — the same
    // reason `ChunkReader` clamps an oversized size field. `models::batch_draw`
    // and `wmos::batch_draw` both already refuse a range they cannot index; this
    // is the third of the three and was the one without it.
    let vertices = source.positions.len();
    if indices.iter().any(|&i| i as usize >= vertices) {
        return None;
    }

    let mut remap: HashMap<u32, u32> = HashMap::default();
    // Which tile vertex each of this group's vertices came from, in group order.
    // Empty unless the host asked for it — see [`LiveEdits`].
    let mut sources: Vec<u32> = Vec::new();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut alpha_uvs: Vec<[f32; 2]> = Vec::new();
    // `MCCV`, and only when the tile has any — see [`shaded`]. A tile with
    // no vertex shading uploads no colour attribute at all, so the pipeline
    // never gets `VERTEX_COLORS` and the fragment takes the branch it always
    // did: the client pays nothing for a chunk that carries none, which is every
    // chunk of every shipped tile.
    let shade = live || shaded(source);
    let mut colours: Vec<[f32; 4]> = Vec::new();
    let mut out: Vec<u32> = Vec::with_capacity(indices.len());

    for &index in indices {
        let next = remap.len() as u32;
        let mapped = *remap.entry(index).or_insert_with(|| {
            let i = index as usize;
            // Positions and normals cross into Bevy's axes here — the only place
            // terrain geometry does. A change of basis is a rotation, so the
            // normals need the same treatment as the positions and no rescaling.
            if live {
                sources.push(index);
            }
            positions.push(axes::to_bevy(source.positions[i]).to_array());
            normals.push(axes::to_bevy(source.normals[i]).to_array());
            if shade {
                colours.push(source.colours.get(i).copied().unwrap_or([1.0; 4]));
            }
            uvs.push(source.uvs[i]);
            alpha_uvs.push(source.alpha_uvs[i]);
            next
        });
        out.push(mapped);
    }

    // Reverse each triangle — see the note above. Swapping the last two of every
    // three is the whole of it.
    for tri in out.chunks_exact_mut(3) {
        tri.swap(1, 2);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        // `RENDER_WORLD` alone drops the mesh from `Assets<Mesh>` once it is
        // uploaded, which is what the client wants — nothing writes to a tile's
        // geometry again — and exactly what a host editing the ground cannot
        // have. See [`LiveEdits`].
        if live {
            RenderAssetUsages::default()
        } else {
            RenderAssetUsages::RENDER_WORLD
        },
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    if shade {
        // Bevy's mesh pipeline turns the presence of this attribute into the
        // `VERTEX_COLORS` shader def and location 5 — see `terrain.wgsl`, which
        // is why nothing here has to specialize the material.
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, alpha_uvs);
    mesh.insert_indices(Indices::U32(out));
    Some((mesh, sources))
}

/// The alpha atlas as an image, with the mip chain `adt::alpha_atlas_mips`
/// builds for it.
///
/// Clamped, not repeated, and minified only as far as a cell is a texel. The
/// blend maps are 64 texels across a 33-yard chunk, so they are minified as hard
/// as the ground textures over them and alias just as badly without a chain —
/// into a lattice in the weights, which reads as a cross-hatched grid of
/// squares rather than as the shimmer an unmipped colour texture gives. Why the
/// chain is safe despite the cells sharing one texture, and why it stops where it
/// does, is on `adt::alpha_atlas_mips`.
///
/// `Rgba8Unorm`, not sRGB: these are blend weights, not colour.
///
/// `live` is [`LiveEdits`], and what it buys is the same thing it buys for the
/// meshes: an image with `RenderAssetUsages::RENDER_WORLD` alone is dropped from
/// `Assets<Image>` once it is uploaded and cannot be written to afterwards, so a
/// host that changes a chunk's blend map and needs the ground to follow within
/// the frame has to keep it. It costs the 5.3 MiB of the atlas and its chain,
/// per loaded tile, which is why it is the switch's and not the default.
/// The per-chunk tint image: 16x16 RGBA, transparent, filtered nearest.
///
/// Nearest is the whole of why one texel is one chunk. `uv_b` maps a vertex to
/// its own chunk's cell of the alpha atlas's 16x16 grid, inset by half a texel
/// of that cell — so at 16x16 the coordinate lands strictly inside its own
/// texel and nearest sampling can only ever return that chunk's own colour. Any
/// filtering at all would bleed a chunk's neighbours into its edges, which for
/// a field with hard boundaries is exactly the wrong picture.
fn tint_image(live: bool) -> Image {
    let mut image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: vale_assets::world::adt::CHUNKS_PER_SIDE as u32,
            height: vale_assets::world::adt::CHUNKS_PER_SIDE as u32,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        vec![0u8; vale_assets::world::adt::CHUNKS_PER_SIDE
            * vale_assets::world::adt::CHUNKS_PER_SIDE
            * 4],
        TextureFormat::Rgba8Unorm,
        match live {
            true => RenderAssetUsages::all(),
            false => RenderAssetUsages::RENDER_WORLD,
        },
    );
    image.sampler = bevy::image::ImageSampler::Descriptor(
        bevy::image::ImageSamplerDescriptor::nearest(),
    );
    image
}

fn atlas_image(rgba: &[u8], live: bool) -> Image {
    let side = vale_assets::world::adt::ATLAS_SIDE as u32;
    let levels = vale_assets::world::adt::alpha_atlas_mips(rgba);

    let mut data = Vec::with_capacity(rgba.len() * 2);
    data.extend_from_slice(rgba);
    for level in &levels {
        data.extend_from_slice(level);
    }

    let mut image = Image::new_uninit(
        bevy::render::render_resource::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        match live {
            true => RenderAssetUsages::all(),
            false => RenderAssetUsages::RENDER_WORLD,
        },
    );
    image.texture_descriptor.mip_level_count = 1 + levels.len() as u32;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        // Without this the chain is uploaded and never used — wgpu's default is
        // nearest, which snaps between levels and slides a visible band over the
        // ground as the camera moves. The same line the ground textures need.
        mipmap_filter: ImageFilterMode::Linear,
        // Isotropic, whatever the ground layers are. The reference sets
        // its alpha map's sampler once, at `MAXANISOTROPY 1`, while the layers
        // beside it take the `anisotropic` CVar (`models::anisotropy`); and the
        // one time this atlas was sampled anisotropically a lattice stood over
        // the ground, phase-changing at chunk edges — the atlas's own edges.
        anisotropy_clamp: 1,
        ..default()
    });
    image
}

// A ground texture is built by `models::model_image`, and the magenta stand-in
// for one the archive lacks by `models::missing_image` — the ground and the
// models want the same sampling, and a second copy of it here is a second place
// for it to drift.

#[cfg(test)]
mod tests {
    use super::*;

    /// The reach a host asks for is clamped, and everything derived from it
    /// is derived.
    ///
    /// The three numbers that have to agree are the block the streamer fills,
    /// the camera's far plane and the ground pick's walk, and all three go
    /// through [`TerrainReach::yards`]. What this pins is that the clamp is on
    /// the used value rather than on the stored one — a host storing 40 must
    /// not get a 9,000-yard far plane against an 11x11 block — and that the
    /// client's own default is unchanged.
    #[test]
    fn the_reach_is_clamped_and_its_yards_follow_it() {
        assert_eq!(TerrainReach::default().0, RADIUS, "the client streams the 3x3");
        assert_eq!(TerrainReach::default().tiles(), 1);
        assert_eq!(TerrainReach::default().yards(), LOADED_REACH);
        // 1,509 yards at the diagonal of the 3x3 — the number the camera's far
        // plane was written against.
        assert!((LOADED_REACH - 1508.5).abs() < 1.0, "{LOADED_REACH}");

        // A wider block reaches further, by whole tiles.
        assert_eq!(TerrainReach(3).tiles(), 3);
        assert!(TerrainReach(3).yards() > TerrainReach(1).yards());
        assert_eq!(TerrainReach(3).yards(), TerrainReach(1).yards() * 2.0);

        // …and neither end of the clamp can be got past.
        assert_eq!(TerrainReach(-4).tiles(), 0, "a negative radius is one tile");
        assert_eq!(TerrainReach(0).yards(), TerrainReach(-4).yards());
        assert_eq!(TerrainReach(40).tiles(), TerrainReach::HIGHEST);
        assert_eq!(TerrainReach(40).yards(), TerrainReach(TerrainReach::HIGHEST).yards());
    }

    /// A minimal two-triangle mesh with one draw covering both.
    fn sample() -> TerrainMesh {
        TerrainMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            colours: Vec::new(),
            uvs: vec![[0.0, 0.0]; 4],
            alpha_uvs: vec![[0.0, 0.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            draws: Vec::new(),
            centre: [0.0; 3],
            radius: 1.0,
        }
    }

    /// A tile nobody has shaded uploads no colour attribute at all.
    ///
    /// That is what keeps `MCCV` free in the client: bevy's mesh pipeline pushes
    /// the `VERTEX_COLORS` shader def from the presence of the attribute, so a
    /// mesh without one compiles and runs exactly the fragment it did before
    /// vertex shading existed. No shipped 1.12 tile carries the region, so
    /// this is the client's every frame.
    #[test]
    fn an_unshaded_tile_carries_no_colour_attribute() {
        let source = sample();
        let (mesh, _) = group_mesh(&source, 0, 6, false).expect("a mesh");
        assert!(!mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR));

        // …and a tile whose colours are all neutral is the same thing: the
        // region can exist and say nothing, and a chunk of 0x7F draws as a chunk
        // with no region.
        let mut neutral = sample();
        neutral.colours = vec![[1.0, 1.0, 1.0, 1.0]; 4];
        let (mesh, _) = group_mesh(&neutral, 0, 6, false).expect("a mesh");
        assert!(!mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR));
    }

    /// …and a shaded one does, with the values the file gave it.
    ///
    /// The half that says the multiplier actually reaches the shader. `MCCV` is
    /// a multiplier centred on 1.0 rather than a 0..1 colour, so the value in
    /// the buffer is asserted rather than only its presence: a decoder that
    /// divided by 255 instead of 127 would put 0.5 here and darken every painted
    /// vertex by half.
    #[test]
    fn a_shaded_tile_carries_the_multipliers_it_was_given() {
        use bevy::mesh::VertexAttributeValues;

        let mut source = sample();
        source.colours = vec![
            [0.5, 1.0, 2.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        ];
        let (mesh, _) = group_mesh(&source, 0, 6, false).expect("a mesh");
        let Some(VertexAttributeValues::Float32x4(colours)) =
            mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("a shaded tile should carry a colour attribute");
        };
        assert_eq!(colours.len(), 4, "one per vertex the group kept");
        assert!(colours.contains(&[0.5, 1.0, 2.0, 1.0]), "{colours:?}");
        assert!(colours.contains(&[0.0, 0.0, 0.0, 1.0]), "{colours:?}");
    }

    /// A host editing the ground gets the attribute whether the tile has any
    /// shading or not, because a stroke has to have somewhere to write before
    /// it has written anything. This is the one case where the client's rule —
    /// upload nothing unless the file says something — is deliberately not
    /// followed.
    #[test]
    fn a_live_tile_always_carries_one() {
        let source = sample();
        let (mesh, _) = group_mesh(&source, 0, 6, true).expect("a mesh");
        assert!(mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR));
    }

    /// The property the plan calls out: splitting a tile into per-group meshes
    /// must not lose or duplicate a triangle. A wrong remap draws some other
    /// part of the tile, which renders as plausible ground rather than failing.
    #[test]
    fn a_group_mesh_keeps_every_triangle_in_its_range() {
        let source = sample();
        let (mesh, _) = group_mesh(&source, 0, 6, false).expect("a mesh");
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("expected u32 indices");
        };
        assert_eq!(indices.len(), 6, "triangle count changed");
        // Four distinct vertices are referenced, so the remap must have kept
        // exactly four.
        assert_eq!(mesh.count_vertices(), 4);
    }

    /// A group covering half the tile must carry only the vertices it uses —
    /// that is what makes its bounding box, and therefore the culling, tight.
    #[test]
    fn a_partial_group_only_carries_the_vertices_it_uses() {
        let source = sample();
        let (mesh, _) = group_mesh(&source, 0, 3, false).expect("a mesh");
        assert_eq!(mesh.count_vertices(), 3);
    }

    /// The tile under the character is handed over first. The loads finish
    /// in whatever order the archive and the parse take, and a login or a
    /// teleport finishes all nine of the 3x3 at once — so without this the
    /// ground you are standing on can be the last thing to appear, which is
    /// exactly the frame the hand-off budget spreads work over.
    #[test]
    fn the_nearest_tile_is_handed_over_first() {
        let ready = [(31, 47), (32, 48), (33, 49), (31, 48)];
        // The centre itself, when it is in the list.
        assert_eq!(
            nearest_index(ready.iter().copied(), Some((32, 48))),
            Some(1)
        );
        // …and its neighbour when it is not: (31, 48) is one tile away where
        // the two diagonals are two.
        let without_centre = [(31, 47), (33, 49), (31, 48)];
        assert_eq!(
            nearest_index(without_centre.iter().copied(), Some((32, 48))),
            Some(2)
        );
        // A tie goes to arrival order rather than to whichever way the
        // comparison happens to fall.
        let tied = [(31, 48), (33, 48)];
        assert_eq!(nearest_index(tied.iter().copied(), Some((32, 48))), Some(0));
        // Before a 3x3 has been chosen there is nothing to be near.
        assert_eq!(nearest_index(ready.iter().copied(), None), Some(0));
        assert_eq!(nearest_index([].into_iter(), Some((32, 48))), None);
    }

    /// The budget is only a budget if the sizes are real. Both of these go
    /// through Bevy APIs that would answer zero for a mesh whose attributes it
    /// does not recognise — and a zero here does not fail, it silently restores
    /// the "one whole tile a frame" behaviour this round replaced, because the
    /// loop would never reach [`UPLOAD_BYTES`].
    #[test]
    fn a_group_mesh_reports_the_bytes_it_will_cost() {
        let (mesh, _) = group_mesh(&sample(), 0, 6, false).expect("a mesh");
        // Four vertices of position + normal + uv + alpha uv = 40 bytes each,
        // and six u32 indices.
        assert_eq!(mesh.get_vertex_buffer_size(), 4 * 40);
        assert_eq!(mesh.get_index_buffer_bytes().map(<[u8]>::len), Some(6 * 4));
        assert_eq!(mesh_bytes(&mesh), 4 * 40 + 6 * 4);
    }

    /// …and the same for the alpha atlas, which is the largest single item a
    /// tile holds and the floor under the whole scheme: 1024x1024 RGBA plus its
    /// chain, which must be counted rather than reported as an empty image.
    #[test]
    fn the_alpha_atlas_reports_its_own_size() {
        let side = vale_assets::world::adt::ATLAS_SIDE;
        let flat = vec![0u8; side * side * 4];
        let image = atlas_image(&flat, false);
        let mips: usize = vale_assets::world::adt::alpha_atlas_mips(&flat)
            .iter()
            .map(Vec::len)
            .sum();
        assert_eq!(image_bytes(&image), flat.len() + mips);
        assert!(
            image_bytes(&image) > UPLOAD_BYTES,
            "the atlas is the item the budget cannot split; if it ever fits, \
             the note on UPLOAD_BYTES is stale"
        );
    }

    #[test]
    fn an_empty_range_yields_no_mesh() {
        assert!(group_mesh(&sample(), 0, 0, false).is_none());
        assert!(group_mesh(&sample(), 99, 3, false).is_none());
    }

    /// One flat cell laid out exactly the way `Adt::to_mesh` lays one out: four
    /// outer corners and a centre, positions running in decreasing x and y,
    /// and four triangles fanned `(centre, corner[i], corner[i+1])`.
    fn one_cell() -> TerrainMesh {
        let unit = 1.0;
        let corner = |r: f32, c: f32| [-r * unit, -c * unit, 0.0];
        TerrainMesh {
            positions: vec![
                corner(0.0, 0.0),
                corner(0.0, 1.0),
                corner(1.0, 1.0),
                corner(1.0, 0.0),
                corner(0.5, 0.5),
            ],
            // MCNR normals are already world axes and point up out of the ground.
            colours: Vec::new(),
            normals: vec![[0.0, 0.0, 1.0]; 5],
            uvs: vec![[0.0, 0.0]; 5],
            alpha_uvs: vec![[0.0, 0.0]; 5],
            indices: (0..4u32)
                .flat_map(|i| [4, i, (i + 1) % 4])
                .collect(),
            draws: Vec::new(),
            centre: [0.0; 3],
            radius: 1.0,
        }
    }

    /// The regression this exists for. Every emitted triangle must face the
    /// way its own vertex normals face, or Bevy's back-face culling removes
    /// exactly the surfaces pointing at the camera and the world reads as though
    /// it were being viewed from underneath — hills as lone mounds, flat ground
    /// as slivers, and no error anywhere.
    ///
    /// `to_mesh` winds clockwise seen from above, which the WebGL renderer never
    /// noticed because its terrain pass never enabled `CULL_FACE`.
    #[test]
    fn every_triangle_faces_the_way_its_normals_do() {
        let source = one_cell();
        let count = source.indices.len() as u32;
        let (mesh, _) = group_mesh(&source, 0, count, false).expect("a mesh");

        let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::render::mesh::VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => panic!("expected f32x3 positions"),
        };
        let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(bevy::render::mesh::VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => panic!("expected f32x3 normals"),
        };
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("expected u32 indices");
        };

        assert_eq!(indices.len(), 12, "four triangles for one cell");
        for (t, tri) in indices.chunks_exact(3).enumerate() {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|&i| Vec3::from_array(positions[i as usize]))
                .collect();
            // Counter-clockwise winding seen from the front, which is what
            // wgpu's default front face and Bevy's back-face cull expect.
            let geometric = (p[1] - p[0]).cross(p[2] - p[0]);
            let shading = Vec3::from_array(normals[tri[0] as usize]);
            assert!(
                geometric.dot(shading) > 0.0,
                "triangle {t} is wound inside out: face {geometric:?} vs normal {shading:?}"
            );
        }
    }
}

#[cfg(test)]
mod sheen_tests {
    use super::*;

    /// Red high, the same packing `models::RoomLight` uses and the same
    /// human order the `Light.dbc` facts are written in. Pinned because a
    /// swapped channel is a plausible sheen of the wrong hue rather than a
    /// failure — a warm highlight would come out cold and nothing would say so.
    #[test]
    fn the_sheen_tag_is_the_bands_own_bytes_red_high() {
        // Map 0's band 9 at noon: 255/247/222, the warm near-white.
        assert_eq!(sheen_tag([1.0, 247.0 / 255.0, 222.0 / 255.0]), 0x00FF_F7DE);
        // Each channel on its own, so a transposition cannot pass.
        assert_eq!(sheen_tag([1.0, 0.0, 0.0]), 0x00FF_0000);
        assert_eq!(sheen_tag([0.0, 1.0, 0.0]), 0x0000_FF00);
        assert_eq!(sheen_tag([0.0, 0.0, 1.0]), 0x0000_00FF);
        // Black is the off switch, which is what `light_sheen` writes with
        // the subtraction on and what the shader tests for. Nothing else may
        // produce it from a real band, so the clamp matters: a negative
        // channel must not wrap into another one's byte.
        assert_eq!(sheen_tag([0.0, 0.0, 0.0]), 0);
        assert_eq!(sheen_tag([-1.0, -1.0, -1.0]), 0);
        assert_eq!(sheen_tag([2.0, 2.0, 2.0]), 0x00FF_FFFF);
    }
}

/// Two layers' scroll velocities as one `Vec4`, for [`TerrainParams::scroll_a`].
///
/// A layer the draw group has not got is still, which is what the missing half
/// of a two- or three-layer group is.
fn pair(scrolls: &[[f32; 2]], from: usize) -> Vec4 {
    let at = |i: usize| scrolls.get(i).copied().unwrap_or([0.0, 0.0]);
    let (a, b) = (at(from), at(from + 1));
    Vec4::new(a[0], a[1], b[0], b[1])
}
