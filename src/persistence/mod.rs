//! Spec 1382 — Save System: persistent colonies.
//!
//! # The persistence boundary (READ THIS before adding state)
//!
//! This module is the ONE place where persistent simulation state is
//! enumerated. When a future spec adds a new persistent component or
//! resource, it must be registered here (capture in [`capture_snapshot`],
//! restore in [`apply_snapshot`]) or it will silently reset on load.
//!
//! ## What is captured (v1 census)
//!
//! * Header: `format_version` (first serialized field), tick, colony name,
//!   world seed, scenario id, adventurer origin choice, save timestamp.
//! * Resources: `SimulationTime` tick, `WorldSeed`, `GlobalRng`
//!   (reseeded deterministically as `seed ^ tick` on load — rand 0.8's
//!   `StdRng` stream wrapper has no serde impl, so its exact stream
//!   position is not capturable; see the determinism note below),
//!   `ColonyResources` (every field), `ColonyName`, `Chronicle` (all
//!   events), `TechState`, `SeasonState`, `DayNightCycle`, `WeatherState`,
//!   `TerrainGrid`, `FertilityGrid`.
//! * Pops (stable order, sorted by entity bits): name, position, health,
//!   needs, morale value + mood modifiers, wallet, skills XP, traits, age,
//!   current `PopAction`, utility weights, memories, `Job` (workplace
//!   remapped), `Relationships` (affinities remapped), `MovementTarget`
//!   (target remapped).
//! * Buildings: type, position, material, health, `Farm` (crop, capacity,
//!   workers remapped), `Housing` (capacity, residents remapped),
//!   `Stockpile` bonuses. Buildings are respawned through the game's own
//!   `spawn_building`, so every auxiliary component a building type needs
//!   is recreated by the same code path that creates it in play.
//! * Loose `Item` entities and `Designation` entities (type + position).
//!
//! ## What is explicitly NOT captured (v1)
//!
//! * The hundreds of secondary resources owned by individual mechanics
//!   (grids such as temperature/pressure/noise, faction ledgers, origin
//!   state machines, layer-2/3 state, event queues). They re-derive or
//!   re-initialize on the next ticks. Gameplay consequence: transient
//!   mechanic state (in-flight crises, origin arcs, market prices) resets
//!   to its resource default on load; the colony's people, buildings,
//!   stockpiles, terrain, research and history survive. Named examples
//!   verified in the headless save-point comparison: the Rumor Web
//!   ledger (`rumors=` in STATS resets), the seniority ledger
//!   (`seniority=` resets), material-provenance building stamps
//!   (`provenance=` resets), and adventurer-origin state machines
//!   (e.g. the Corsair's `crew=`/`loyalty=` reset; the origin *roster*
//!   and the player's origin *choice* are captured).
//! * Pop components beyond the census above (equipment, appearance,
//!   ethics, faction, and the long tail of mechanic-specific components).
//!   Restored pops are respawned from a full `PopBundle` (seeded
//!   deterministically from the snapshot) and then have every captured
//!   field overwritten, so they are fully-formed pops — never ghosts —
//!   but uncaptured flavor components re-randomize from that seed.
//!   **Spec 1384 exception:** a pop's *worn EVA suit* IS captured
//!   (class, air, durability, colony-issued flag) and restored into
//!   the body slot — losing it on load would strip the bottle off a
//!   pop standing in vacuum and kill them on the first tick. Loose
//!   physical suits are captured as items. All other equipment stays
//!   uncaptured.
//! * Transient render/UI state (selection, camera, menus) — out by spec.
//!
//! ## Determinism honesty
//!
//! The save-point round-trip is exact: capture → apply → capture yields an
//! identical snapshot, so STATS at the save point matches exactly after a
//! load. Continuation after load is NOT bit-identical to an uninterrupted
//! run in the general case: much of the simulation draws from
//! `rand::thread_rng()` (260+ call sites), which no save system can
//! snapshot. Restoring `GlobalRng`/`WorldSeed` covers the seeded paths;
//! the rest is documented here rather than silently promised.
//!
//! ## Format
//!
//! `serde_json` (bincode is not in the dependency tree; spec §4 leaves the
//! choice to the builder and JSON is debuggable). `format_version` is the
//! first field of [`SaveSnapshot`]; a version mismatch fails with the
//! typed [`SaveError::VersionMismatch`] — never a panic, never a silent
//! garbage load.

use bevy_ecs::prelude::*;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use strum::IntoEnumIterator;

use crate::layer1::architecture::building::{spawn_building, Material};
use crate::layer1::culture::origins::{OriginChoice, OriginKind};
use crate::layer1::economy::items::Item;
use crate::layer1::economy::Wallet;
use crate::layer1::entities::pop::{Job, PopName};
use crate::layer1::execution::MovementTarget;
use crate::layer1::health::Health;
use crate::layer1::memory::{ActiveMemory, Memories};
use crate::layer1::morale::{MoodModifier, Morale};
use crate::layer1::needs::Needs;
use crate::layer1::pop::Pop;
use crate::layer1::skills::{SkillType, Skills};
use crate::layer1::social::Relationships;
use crate::layer1::traits::{Trait, Traits};
use crate::layer1::utility_types::{ActionType, PopAction, UtilityWeights};
use crate::layer1::{
    Building, BuildingType, Chronicle, ColonyResources, Designation, DesignationType, EventImportance,
    Farm, GridPosition, Housing, ItemType, MaterialType, Stockpile, TechState, TechStatus,
    TerrainGrid, TerrainType,
};
use crate::layer1::day_night::{DayNightCycle, TimeOfDay};
use crate::layer1::lifecycle::{Age, LifeStage};
use crate::layer1::nature::fertility::FertilityGrid;
use crate::layer1::nature::seasons::{Season, SeasonState};
use crate::layer1::nature::weather::{WeatherState, WeatherType};
use crate::layer1::psychology::memory::MemoryType;
use crate::layer1::tech::Tech;
use crate::shared::colony::ColonyName;
use crate::shared::random::{GlobalRng, WorldSeed};
use crate::shared::scenario::{start_scenario_definition, ActiveStartScenario, StartScenarioId};
use crate::shared::time::SimulationTime;

#[cfg(target_arch = "wasm32")]
pub mod wasm_idb;

pub mod settings;
pub use settings::{load_settings, store_settings, SaveSettings, SETTINGS_KEY};

/// Current on-disk save format version. Bump on any snapshot shape change.
/// v2 (Spec 1384): EVA suit pool fields, worn-suit payloads, loose-suit items.
pub const CURRENT_SAVE_VERSION: u32 = 2;
/// Reserved slot that only the autosave path may write.
pub const AUTOSAVE_SLOT: &str = "autosave";
/// Autosave cadence (ticks).
pub const AUTOSAVE_INTERVAL_TICKS: u64 = 1000;

/// Typed save-system errors. Every failure surfaces as one of these —
/// never a panic.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveError {
    /// Snapshot format version differs from [`CURRENT_SAVE_VERSION`].
    VersionMismatch { found: u32, expected: u32 },
    /// Slot name failed sanitization (empty, too long, or bad characters).
    InvalidSlot(String),
    /// The requested slot does not exist in the store.
    SlotNotFound(String),
    /// The reserved autosave slot was targeted by a manual save.
    ReservedSlot(String),
    /// Underlying I/O failure (native file backend).
    Io(String),
    /// Serialization / deserialization failure.
    Serialize(String),
    /// Storage-backend failure (IndexedDB bridge, etc.).
    Backend(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VersionMismatch { found, expected } => write!(
                f,
                "save format version mismatch: found {found}, expected {expected}"
            ),
            Self::InvalidSlot(slot) => write!(f, "invalid save slot name: '{slot}'"),
            Self::SlotNotFound(slot) => write!(f, "no save in slot '{slot}'"),
            Self::ReservedSlot(slot) => {
                write!(f, "slot '{slot}' is reserved for autosaves")
            }
            Self::Io(msg) => write!(f, "save I/O error: {msg}"),
            Self::Serialize(msg) => write!(f, "save serialization error: {msg}"),
            Self::Backend(msg) => write!(f, "save backend error: {msg}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// Metadata about one stored save — returned by `list_saves` without
/// loading the world it describes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveMetadata {
    /// Slot name the save lives in.
    pub slot: String,
    /// Snapshot format version.
    pub format_version: u32,
    /// Simulation tick at the save point.
    pub tick: u64,
    /// Colony name at the save point.
    pub colony_name: String,
    /// Wall-clock save time (unix seconds; 0 when unavailable).
    pub saved_at_unix: u64,
    /// Blob size in bytes.
    pub size_bytes: usize,
}

/// Lightweight header parsed from a blob to build [`SaveMetadata`]
/// without deserializing the whole snapshot.
#[derive(Debug, Deserialize)]
struct SaveHeader {
    format_version: u32,
    tick: u64,
    #[serde(default)]
    colony_name: String,
    #[serde(default)]
    saved_at_unix: u64,
}

fn metadata_from_blob(slot: &str, data: &[u8]) -> Result<SaveMetadata, SaveError> {
    let header: SaveHeader = serde_json::from_slice(data)
        .map_err(|e| SaveError::Serialize(format!("reading save header: {e}")))?;
    if header.format_version != CURRENT_SAVE_VERSION {
        return Err(SaveError::VersionMismatch {
            found: header.format_version,
            expected: CURRENT_SAVE_VERSION,
        });
    }
    Ok(SaveMetadata {
        slot: slot.to_string(),
        format_version: header.format_version,
        tick: header.tick,
        colony_name: header.colony_name,
        saved_at_unix: header.saved_at_unix,
        size_bytes: data.len(),
    })
}

/// Sanitize a slot name: 1–64 chars of `[A-Za-z0-9_-]`. Anything else
/// (including path separators / `..`) is rejected — no path traversal.
pub fn sanitize_slot(slot: &str) -> Result<String, SaveError> {
    let ok = !slot.is_empty()
        && slot.len() <= 64
        && slot
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(slot.to_string())
    } else {
        Err(SaveError::InvalidSlot(slot.to_string()))
    }
}

/// Storage backend for save blobs (spec §4).
pub trait SaveStore {
    /// Store (or overwrite) the blob in `slot`.
    fn put(&mut self, slot: &str, data: &[u8]) -> Result<(), SaveError>;
    /// Fetch the blob in `slot`, if any.
    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, SaveError>;
    /// Metadata for every stored save.
    fn list(&self) -> Result<Vec<SaveMetadata>, SaveError>;
    /// Remove the save in `slot` (missing slot is not an error).
    fn delete(&mut self, slot: &str) -> Result<(), SaveError>;
}

/// In-memory backend — headless default and all unit tests.
#[derive(Debug, Default)]
pub struct MemorySaveStore {
    slots: HashMap<String, Vec<u8>>,
}

impl MemorySaveStore {
    /// Create an empty in-memory store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SaveStore for MemorySaveStore {
    fn put(&mut self, slot: &str, data: &[u8]) -> Result<(), SaveError> {
        let slot = sanitize_slot(slot)?;
        self.slots.insert(slot, data.to_vec());
        Ok(())
    }

    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, SaveError> {
        let slot = sanitize_slot(slot)?;
        Ok(self.slots.get(&slot).cloned())
    }

    fn list(&self) -> Result<Vec<SaveMetadata>, SaveError> {
        let mut out = Vec::new();
        for (slot, data) in &self.slots {
            out.push(metadata_from_blob(slot, data)?);
        }
        out.sort_by(|a, b| a.slot.cmp(&b.slot));
        Ok(out)
    }

    fn delete(&mut self, slot: &str) -> Result<(), SaveError> {
        let slot = sanitize_slot(slot)?;
        self.slots.remove(&slot);
        Ok(())
    }
}

/// Native file backend: one `<slot>.json` file per slot under a
/// `saves/` directory (spec §4). Not compiled for wasm32.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct FileSaveStore {
    dir: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileSaveStore {
    /// Store rooted at the default `saves/` directory.
    #[must_use]
    pub fn default_dir() -> Self {
        Self::new(std::path::PathBuf::from("saves"))
    }

    /// Store rooted at an explicit directory (tests use a temp dir).
    #[must_use]
    pub fn new(dir: std::path::PathBuf) -> Self {
        Self { dir }
    }

    fn path_for(&self, slot: &str) -> Result<std::path::PathBuf, SaveError> {
        let slot = sanitize_slot(slot)?;
        Ok(self.dir.join(format!("{slot}.json")))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SaveStore for FileSaveStore {
    fn put(&mut self, slot: &str, data: &[u8]) -> Result<(), SaveError> {
        let path = self.path_for(slot)?;
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| SaveError::Io(format!("creating {}: {e}", self.dir.display())))?;
        // Write-then-rename so a crash mid-write cannot truncate a good save.
        let tmp = self.dir.join(".tmp-save.json");
        std::fs::write(&tmp, data)
            .map_err(|e| SaveError::Io(format!("writing {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| SaveError::Io(format!("renaming to {}: {e}", path.display())))?;
        Ok(())
    }

    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, SaveError> {
        let path = self.path_for(slot)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(SaveError::Io(format!("reading {}: {e}", path.display()))),
        }
    }

    fn list(&self) -> Result<Vec<SaveMetadata>, SaveError> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => {
                return Err(SaveError::Io(format!(
                    "listing {}: {e}",
                    self.dir.display()
                )))
            }
        };
        for entry in entries {
            let entry = entry.map_err(|e| SaveError::Io(e.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem.starts_with('.') {
                continue;
            }
            let data = std::fs::read(&path)
                .map_err(|e| SaveError::Io(format!("reading {}: {e}", path.display())))?;
            // A corrupt/foreign file in saves/ is skipped in listings
            // rather than failing the whole list; loading it directly
            // still reports the typed error.
            if let Ok(meta) = metadata_from_blob(stem, &data) {
                out.push(meta);
            }
        }
        out.sort_by(|a, b| a.slot.cmp(&b.slot));
        Ok(out)
    }

    fn delete(&mut self, slot: &str) -> Result<(), SaveError> {
        let path = self.path_for(slot)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(SaveError::Io(format!("deleting {}: {e}", path.display()))),
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot DTOs
// ---------------------------------------------------------------------------

/// Full colony snapshot (format v1). Field order matters: `format_version`
/// is serialized first, per spec §4.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveSnapshot {
    /// Snapshot format version — MUST stay the first field.
    pub format_version: u32,
    /// Simulation tick at the save point.
    pub tick: u64,
    /// Wall-clock save time (unix seconds; supplied by the caller so unit
    /// tests can be deterministic).
    pub saved_at_unix: u64,
    /// Colony name at the save point.
    pub colony_name: String,
    /// World-generation seed.
    pub world_seed: u64,
    /// Scenario id (Debug name of `StartScenarioId`), if known.
    pub scenario: Option<String>,
    /// Adventurer origin choice (origin id), if one was chosen.
    pub origin_choice: Option<String>,
    /// Deterministic GlobalRng reseed value (`world_seed ^ tick`), set
    /// when the GlobalRng resource exists. rand 0.8's `StdRng` has no
    /// serde impl for its stream state, so load reseeds from this
    /// instead of restoring the exact stream position.
    pub global_rng_reseed: Option<u64>,
    /// Colony resource ledger (every field).
    pub resources: ColonyResourcesSave,
    /// Chronicle events, oldest first.
    pub chronicle: Vec<ChronicleEventSave>,
    /// Research state.
    pub tech: TechSave,
    /// Season (Debug name).
    pub season: String,
    /// Day/night cycle state.
    pub day_night: DayNightSave,
    /// Weather state.
    pub weather: WeatherSave,
    /// Terrain grid (tile Debug names, row-major).
    pub terrain: TerrainSave,
    /// Fertility grid values, when the resource exists.
    pub fertility: Option<Vec<f32>>,
    /// All pops, in stable (entity-bits) order.
    pub pops: Vec<PopSave>,
    /// All buildings, in stable order.
    pub buildings: Vec<BuildingSave>,
    /// Loose item entities.
    pub items: Vec<ItemSave>,
    /// Designation entities.
    pub designations: Vec<DesignationSave>,
}

/// Every `ColonyResources` field, mirrored 1:1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct ColonyResourcesSave {
    pub food: f32,
    pub wood: f32,
    pub stone: f32,
    pub planks: f32,
    pub blocks: f32,
    pub ore: f32,
    pub metal: f32,
    pub tools: f32,
    pub knowledge: f32,
    pub fiber: f32,
    pub cloth: f32,
    pub hyper_alloys: f32,
    pub clothing: f32,
    pub waste: f32,
    pub biological_waste: f32,
    pub nutrient_paste: f32,
    pub max_biological_waste: f32,
    pub max_nutrient_paste: f32,
    pub rations: f32,
    pub fuel: f32,
    pub water: f32,
    pub alcohol: f32,
    pub scrap: f32,
    pub building_permits: f32,
    pub credits: f32,
    pub max_food: f32,
    pub max_wood: f32,
    pub max_stone: f32,
    pub max_planks: f32,
    pub max_blocks: f32,
    pub max_ore: f32,
    pub max_metal: f32,
    pub max_tools: f32,
    pub max_knowledge: f32,
    pub max_fiber: f32,
    pub max_cloth: f32,
    pub max_clothing: f32,
    pub max_waste: f32,
    pub max_rations: f32,
    pub max_fuel: f32,
    pub max_water: f32,
    pub max_alcohol: f32,
    pub max_scrap: f32,
    pub max_building_permits: f32,
    pub max_credits: f32,
    pub memory_cores: f32,
    pub max_memory_cores: f32,
    pub void_ale: f32,
    pub max_void_ale: f32,
    pub art: f32,
    pub luxury: f32,
    pub max_art: f32,
    pub max_luxury: f32,
    pub hyper_valuable: f32,
    pub max_hyper_valuable: f32,
    pub organs: f32,
    pub max_organs: f32,
    pub dark_motes: f32,
    pub max_dark_motes: f32,
    /// Spec 1384: colony EVA suit pool.
    pub eva_suits: f32,
    /// Spec 1384: colony EVA suit pool cap.
    pub max_eva_suits: f32,
}

impl From<&ColonyResources> for ColonyResourcesSave {
    fn from(r: &ColonyResources) -> Self {
        Self {
            food: r.food,
            wood: r.wood,
            stone: r.stone,
            planks: r.planks,
            blocks: r.blocks,
            ore: r.ore,
            metal: r.metal,
            tools: r.tools,
            knowledge: r.knowledge,
            fiber: r.fiber,
            cloth: r.cloth,
            hyper_alloys: r.hyper_alloys,
            clothing: r.clothing,
            waste: r.waste,
            biological_waste: r.biological_waste,
            nutrient_paste: r.nutrient_paste,
            max_biological_waste: r.max_biological_waste,
            max_nutrient_paste: r.max_nutrient_paste,
            rations: r.rations,
            fuel: r.fuel,
            water: r.water,
            alcohol: r.alcohol,
            scrap: r.scrap,
            building_permits: r.building_permits,
            credits: r.credits,
            max_food: r.max_food,
            max_wood: r.max_wood,
            max_stone: r.max_stone,
            max_planks: r.max_planks,
            max_blocks: r.max_blocks,
            max_ore: r.max_ore,
            max_metal: r.max_metal,
            max_tools: r.max_tools,
            max_knowledge: r.max_knowledge,
            max_fiber: r.max_fiber,
            max_cloth: r.max_cloth,
            max_clothing: r.max_clothing,
            max_waste: r.max_waste,
            max_rations: r.max_rations,
            max_fuel: r.max_fuel,
            max_water: r.max_water,
            max_alcohol: r.max_alcohol,
            max_scrap: r.max_scrap,
            max_building_permits: r.max_building_permits,
            max_credits: r.max_credits,
            memory_cores: r.memory_cores,
            max_memory_cores: r.max_memory_cores,
            void_ale: r.void_ale,
            max_void_ale: r.max_void_ale,
            art: r.art,
            luxury: r.luxury,
            max_art: r.max_art,
            max_luxury: r.max_luxury,
            hyper_valuable: r.hyper_valuable,
            max_hyper_valuable: r.max_hyper_valuable,
            organs: r.organs,
            max_organs: r.max_organs,
            dark_motes: r.dark_motes,
            max_dark_motes: r.max_dark_motes,
            eva_suits: r.eva_suits,
            max_eva_suits: r.max_eva_suits,
        }
    }
}

impl From<&ColonyResourcesSave> for ColonyResources {
    fn from(s: &ColonyResourcesSave) -> Self {
        Self {
            food: s.food,
            wood: s.wood,
            stone: s.stone,
            planks: s.planks,
            blocks: s.blocks,
            ore: s.ore,
            metal: s.metal,
            tools: s.tools,
            knowledge: s.knowledge,
            fiber: s.fiber,
            cloth: s.cloth,
            hyper_alloys: s.hyper_alloys,
            clothing: s.clothing,
            waste: s.waste,
            biological_waste: s.biological_waste,
            nutrient_paste: s.nutrient_paste,
            max_biological_waste: s.max_biological_waste,
            max_nutrient_paste: s.max_nutrient_paste,
            rations: s.rations,
            fuel: s.fuel,
            water: s.water,
            alcohol: s.alcohol,
            scrap: s.scrap,
            building_permits: s.building_permits,
            credits: s.credits,
            max_food: s.max_food,
            max_wood: s.max_wood,
            max_stone: s.max_stone,
            max_planks: s.max_planks,
            max_blocks: s.max_blocks,
            max_ore: s.max_ore,
            max_metal: s.max_metal,
            max_tools: s.max_tools,
            max_knowledge: s.max_knowledge,
            max_fiber: s.max_fiber,
            max_cloth: s.max_cloth,
            max_clothing: s.max_clothing,
            max_waste: s.max_waste,
            max_rations: s.max_rations,
            max_fuel: s.max_fuel,
            max_water: s.max_water,
            max_alcohol: s.max_alcohol,
            max_scrap: s.max_scrap,
            max_building_permits: s.max_building_permits,
            max_credits: s.max_credits,
            memory_cores: s.memory_cores,
            max_memory_cores: s.max_memory_cores,
            void_ale: s.void_ale,
            max_void_ale: s.max_void_ale,
            art: s.art,
            luxury: s.luxury,
            max_art: s.max_art,
            max_luxury: s.max_luxury,
            hyper_valuable: s.hyper_valuable,
            max_hyper_valuable: s.max_hyper_valuable,
            organs: s.organs,
            max_organs: s.max_organs,
            dark_motes: s.dark_motes,
            max_dark_motes: s.max_dark_motes,
            eva_suits: s.eva_suits,
            max_eva_suits: s.max_eva_suits,
        }
    }
}

/// One chronicle entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChronicleEventSave {
    /// Tick the event fired.
    pub tick: u64,
    /// Derived year.
    pub year: u32,
    /// Event text.
    pub text: String,
    /// Importance (Debug name).
    pub importance: String,
}

/// Research state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TechSave {
    /// (tech Debug name, status Debug name) pairs, sorted by tech name.
    pub entries: Vec<(String, String)>,
    /// Total data capacity.
    pub total_capacity: f32,
    /// Used data capacity.
    pub used_capacity: f32,
}

/// Day/night cycle state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayNightSave {
    /// Phase (Debug name).
    pub time_of_day: String,
    /// Days elapsed.
    pub day_count: u32,
    /// Ticks per full day.
    pub ticks_per_day: u64,
}

/// Weather state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherSave {
    /// Weather type (Debug name).
    pub current: String,
    /// Ticks remaining.
    pub duration_remaining: u32,
}

/// Terrain grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainSave {
    /// Grid width.
    pub width: usize,
    /// Grid height.
    pub height: usize,
    /// Tile Debug names, row-major (`index = y * width + x`).
    pub tiles: Vec<String>,
}

/// One saved pop. `old_bits` is the source entity's `to_bits()` value;
/// cross-references are stored in the same key space and remapped on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PopSave {
    /// Source entity bits.
    pub old_bits: u64,
    /// Name.
    pub name: String,
    /// Position.
    pub x: i32,
    /// Position.
    pub y: i32,
    /// Health current / max / rust-lung flag.
    pub health: Option<(f32, f32, bool)>,
    /// (hunger, rest, leisure, hygiene).
    pub needs: Option<(f32, f32, f32, f32)>,
    /// Morale value.
    pub morale_value: Option<f32>,
    /// Mood modifiers (label, value, duration).
    pub morale_modifiers: Vec<(String, f32, u32)>,
    /// Wallet credits.
    pub wallet_credits: Option<f32>,
    /// Skill XP by skill Debug name.
    pub skills: Vec<(String, f32)>,
    /// Trait Debug names.
    pub traits: Vec<String>,
    /// (ticks alive, life stage Debug name).
    pub age: Option<(u64, String)>,
    /// (action Debug name, utility, ticks committed).
    pub action: Option<(String, f32, u32)>,
    /// (distance weight, availability weight).
    pub weights: Option<(f32, f32)>,
    /// (memory type Debug name, added_at, intensity, forged).
    pub memories: Vec<(String, u64, f32, bool)>,
    /// (workplace old_bits, job type Debug name).
    pub job: Option<(u64, String)>,
    /// (target old_bits, affinity) pairs.
    pub relationships: Vec<(u64, f32)>,
    /// (target old_bits, target x, target y, action Debug name).
    pub movement_target: Option<(u64, i32, i32, String)>,
    /// Spec 1384 (§4/§10): the pop's worn EVA suit, if any — the one
    /// deliberate exception to the v1 "equipment is not captured"
    /// rule. A load without a suit would strip the bottle off a pop
    /// standing in vacuum and kill them on the first tick.
    pub worn_suit: Option<SuitSave>,
}

/// One saved EVA suit (worn payload or loose-item payload, Spec 1384).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuitSave {
    /// Suit class save name (`base` / `mk2` / `expedition`).
    pub class: String,
    /// Remaining bottle days.
    pub air: f32,
    /// Current durability.
    pub durability: f32,
    /// Whether the suit was colony-issued (pool-backed) or physical.
    pub colony_issued: bool,
}

/// One saved building.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildingSave {
    /// Source entity bits.
    pub old_bits: u64,
    /// Building type (Debug name).
    pub building_type: String,
    /// Position.
    pub x: i32,
    /// Position.
    pub y: i32,
    /// Material (Debug name).
    pub material: Option<String>,
    /// Health (current, max, rust-lung) when the building has Health.
    pub health: Option<(f32, f32, bool)>,
    /// (capacity, crop Debug name, worker old_bits).
    pub farm: Option<(usize, String, Vec<u64>)>,
    /// (capacity, resident old_bits).
    pub housing: Option<(usize, Vec<u64>)>,
    /// (food, wood, stone, waste) capacity bonuses.
    pub stockpile: Option<(f32, f32, f32, f32)>,
}

/// One loose item entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemSave {
    /// Item type (Debug name). For the data-carrying variants this is
    /// the bare variant name (`Corpse`, `MemoryCore`, `Curio`) and the
    /// payload travels in the fields below.
    pub item_type: String,
    /// Position.
    pub x: i32,
    /// Position.
    pub y: i32,
    /// Referenced entity bits for `Corpse(Entity)` / `MemoryCore(Entity)`
    /// — remapped on load like every other cross-reference.
    pub item_ref: Option<u64>,
    /// Display name for `Curio(&'static str)`.
    pub curio_name: Option<String>,
    /// Spec 1384: loose EVA suit payload when this item is a suit.
    pub suit: Option<SuitSave>,
}

/// One designation entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignationSave {
    /// Designation type (Debug name).
    pub designation_type: String,
    /// Position.
    pub x: i32,
    /// Position.
    pub y: i32,
}

// ---------------------------------------------------------------------------
// Enum name <-> value helpers. The game's enums do not derive serde, so the
// snapshot stores Debug names and restores by name. Enums with `EnumIter`
// are parsed generically; the rest use explicit match tables (compiler
// exhaustiveness on the capture side keeps these honest as variants grow).
// ---------------------------------------------------------------------------

fn name_of<T: Debug>(v: &T) -> String {
    format!("{v:?}")
}

fn parse_iter<T>(name: &str) -> Option<T>
where
    T: Debug + IntoEnumIterator,
{
    T::iter().find(|v| name_of(v) == name)
}

fn parse_terrain(name: &str) -> Option<TerrainType> {
    use TerrainType::{
        Artifact, Ash, Bridge, Crater, DeepRock, Dirt, Grass, IndestructibleStump, MagmaRock,
        Path, Rock, Sapling, Shrub, SporeBloom, Tree, Void, Water,
    };
    Some(match name {
        "Grass" => Grass,
        "Bridge" => Bridge,
        "Void" => Void,
        "Dirt" => Dirt,
        "Rock" => Rock,
        "Water" => Water,
        "Tree" => Tree,
        "Path" => Path,
        "Shrub" => Shrub,
        "Sapling" => Sapling,
        "DeepRock" => DeepRock,
        "Crater" => Crater,
        "IndestructibleStump" => IndestructibleStump,
        "MagmaRock" => MagmaRock,
        "SporeBloom" => SporeBloom,
        "Ash" => Ash,
        "Artifact" => Artifact,
        "FaultLine(true)" => TerrainType::FaultLine(true),
        "FaultLine(false)" => TerrainType::FaultLine(false),
        _ => return None,
    })
}

fn parse_item_type(name: &str) -> Option<ItemType> {
    // Curio carries a &'static str name: Curio("Wooden Duck"). The name
    // string is leaked to satisfy the 'static lifetime (one tiny leak
    // per restored curio per load — bounded by the item count).
    if let Some(inner) = name
        .strip_prefix("Curio(\"")
        .and_then(|rest| rest.strip_suffix("\")"))
    {
        let leaked: &'static str = Box::leak(inner.to_string().into_boxed_str());
        return Some(ItemType::Curio(leaked));
    }
    use ItemType::{
        AgonyExtract, Alcohol, AlienMeatA, AlienMeatB, Artifact, BuildingPermit, CalciumAlloy,
        Clothing, Corn, Fish, Fruit, GeneticSample, GlowMushroom, LivingStone,
        LostTech, LuxuryMeal, Manual, Meat, MysteryMeal, None as ItemNone, NutrientPaste,
        Potato, Prosthetic, Rations, Rebreather, Rice, Scrap, Sedative, ShadowCrystal, Soy, Stim,
        Tool, VitalOrgans, VoidAle, Waste, Wheat, Xenoflora,
    };
    Some(match name {
        "None" => ItemNone,
        "Tool" => Tool,
        "Clothing" => Clothing,
        "Prosthetic" => Prosthetic,
        "Manual" => Manual,
        "CalciumAlloy" => CalciumAlloy,
        "Potato" => Potato,
        "Wheat" => Wheat,
        "Rice" => Rice,
        "Corn" => Corn,
        "NutrientPaste" => NutrientPaste,
        "Soy" => Soy,
        "Meat" => Meat,
        "Fish" => Fish,
        "Fruit" => Fruit,
        "LuxuryMeal" => LuxuryMeal,
        "AlienMeatA" => AlienMeatA,
        "AlienMeatB" => AlienMeatB,
        "GlowMushroom" => GlowMushroom,
        "MysteryMeal" => MysteryMeal,
        "Xenoflora" => Xenoflora,
        "Stim" => Stim,
        "Sedative" => Sedative,
        "Alcohol" => Alcohol,
        "GeneticSample" => GeneticSample,
        "Rations" => Rations,
        "Waste" => Waste,
        "Rebreather" => Rebreather,
        "BuildingPermit" => BuildingPermit,
        "ShadowCrystal" => ShadowCrystal,
        "VitalOrgans" => VitalOrgans,
        "LivingStone" => LivingStone,
        "Scrap" => Scrap,
        "AgonyExtract" => AgonyExtract,
        "VoidAle" => VoidAle,
        "LostTech" => LostTech,
        "Artifact" => Artifact,
        _ => return None,
    })
}

fn parse_designation_type(name: &str) -> Option<DesignationType> {
    // SetZone carries a ZoneType: SetZone(Storage), etc.
    if let Some(inner) = name
        .strip_prefix("SetZone(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return parse_zone_type(inner).map(DesignationType::SetZone);
    }
    use DesignationType::{
        Cannibalize, Chop, ClearFlora, CollectSample, Consume, Demolish, Destroy, Douse,
        ExcavateDust, JuryRig, Mine, Repair, Tame,
    };
    Some(match name {
        "Mine" => Mine,
        "Demolish" => Demolish,
        "Chop" => Chop,
        "Repair" => Repair,
        "Tame" => Tame,
        "ClearFlora" => ClearFlora,
        "JuryRig" => JuryRig,
        "Cannibalize" => Cannibalize,
        "Destroy" => Destroy,
        "Consume" => Consume,
        "CollectSample" => CollectSample,
        "ExcavateDust" => ExcavateDust,
        "Douse" => Douse,
        _ => return None,
    })
}

fn parse_zone_type(name: &str) -> Option<crate::layer1::zone::ZoneType> {
    use crate::layer1::zone::ZoneType::{
        Bedroom, Customs, Dezoned, Dining, Extraterritorial, Hospital, Jail, None as ZoneNone,
        Office, Pasture, Penal, Sanctuary, Storage,
    };
    Some(match name {
        "None" => ZoneNone,
        "Bedroom" => Bedroom,
        "Dining" => Dining,
        "Hospital" => Hospital,
        "Office" => Office,
        "Storage" => Storage,
        "Jail" => Jail,
        "Pasture" => Pasture,
        "Penal" => Penal,
        "Sanctuary" => Sanctuary,
        "Extraterritorial" => Extraterritorial,
        "Customs" => Customs,
        "Dezoned" => Dezoned,
        _ => return None,
    })
}

fn parse_skill_type(name: &str) -> Option<SkillType> {
    use SkillType::{
        Construction, Crafting, Engineering, Farming, Forestry, Husbandry, Mining, Science,
    };
    Some(match name {
        "Mining" => Mining,
        "Forestry" => Forestry,
        "Farming" => Farming,
        "Construction" => Construction,
        "Crafting" => Crafting,
        "Husbandry" => Husbandry,
        "Engineering" => Engineering,
        "Science" => Science,
        _ => return None,
    })
}

fn parse_assignment_type(
    name: &str,
) -> Option<crate::layer1::utility_types::AssignmentType> {
    use crate::layer1::utility_types::AssignmentType::{
        Administrator, DeepMining, Doctor, FarmWorker, Funeral, HousingResident, LibraryWorker,
        ObservatoryWorker, Patient, RationingBureaucrat, Sheriff, Surgery, TavernVisitor,
    };
    Some(match name {
        "Sheriff" => Sheriff,
        "FarmWorker" => FarmWorker,
        "HousingResident" => HousingResident,
        "TavernVisitor" => TavernVisitor,
        "LibraryWorker" => LibraryWorker,
        "Patient" => Patient,
        "Funeral" => Funeral,
        "ObservatoryWorker" => ObservatoryWorker,
        "Administrator" => Administrator,
        "RationingBureaucrat" => RationingBureaucrat,
        "Surgery" => Surgery,
        "DeepMining" => DeepMining,
        "Doctor" => Doctor,
        _ => return None,
    })
}

fn parse_memory_type(name: &str) -> Option<MemoryType> {
    use MemoryType::{
        AdmiredArt, AteFineMeal, AteInAwfulRoom, AteInDecentRoom, AteInDullRoom, AteInGreatRoom,
        AteInLegendaryRoom, AttendedFuneral, CannibalismTaboo, CannibalismTrauma, ColonyFounder,
        DisgustedByVermin, FakePositive, FleshTaxTrauma, InspectorDisappointed, InspectorImpressed,
        LearnedFromMentor, LostLimb, MascotDeath, SawCorpse, SleptInAwfulRoom, SleptInDecentRoom,
        SleptInDullRoom, SleptInGreatRoom, SleptInLegendaryRoom, StarvationTrauma, SurvivedFamine,
        TaughtNovice, WitnessedDeath, WonFight,
    };
    Some(match name {
        "WitnessedDeath" => WitnessedDeath,
        "StarvationTrauma" => StarvationTrauma,
        "AteFineMeal" => AteFineMeal,
        "WonFight" => WonFight,
        "SawCorpse" => SawCorpse,
        "AttendedFuneral" => AttendedFuneral,
        "FleshTaxTrauma" => FleshTaxTrauma,
        "SleptInAwfulRoom" => SleptInAwfulRoom,
        "SleptInDullRoom" => SleptInDullRoom,
        "SleptInDecentRoom" => SleptInDecentRoom,
        "SleptInGreatRoom" => SleptInGreatRoom,
        "SleptInLegendaryRoom" => SleptInLegendaryRoom,
        "AteInAwfulRoom" => AteInAwfulRoom,
        "AteInDullRoom" => AteInDullRoom,
        "AteInDecentRoom" => AteInDecentRoom,
        "AteInGreatRoom" => AteInGreatRoom,
        "AteInLegendaryRoom" => AteInLegendaryRoom,
        "AdmiredArt" => AdmiredArt,
        "DisgustedByVermin" => DisgustedByVermin,
        "InspectorImpressed" => InspectorImpressed,
        "InspectorDisappointed" => InspectorDisappointed,
        "FakePositive" => FakePositive,
        "MascotDeath" => MascotDeath,
        "LostLimb" => LostLimb,
        "CannibalismTaboo" => CannibalismTaboo,
        "CannibalismTrauma" => CannibalismTrauma,
        "LearnedFromMentor" => LearnedFromMentor,
        "TaughtNovice" => TaughtNovice,
        "SurvivedFamine" => SurvivedFamine,
        "ColonyFounder" => ColonyFounder,
        _ => return None,
    })
}

fn parse_weather(name: &str) -> Option<WeatherType> {
    use WeatherType::{
        BlissStorm, Clear, Fog, Heatwave, MagneticStorm, MutagenicRain, Rain, Snow, SporeStorm,
        Storm, ThermalInversion,
    };
    Some(match name {
        "Clear" => Clear,
        "Rain" => Rain,
        "Storm" => Storm,
        "Fog" => Fog,
        "Heatwave" => Heatwave,
        "Snow" => Snow,
        "ThermalInversion" => ThermalInversion,
        "MagneticStorm" => MagneticStorm,
        "MutagenicRain" => MutagenicRain,
        "BlissStorm" => BlissStorm,
        "SporeStorm" => SporeStorm,
        _ => return None,
    })
}

fn parse_time_of_day(name: &str) -> Option<TimeOfDay> {
    Some(match name {
        "Dawn" => TimeOfDay::Dawn,
        "Day" => TimeOfDay::Day,
        "Dusk" => TimeOfDay::Dusk,
        "Night" => TimeOfDay::Night,
        _ => return None,
    })
}

fn parse_season(name: &str) -> Option<Season> {
    Some(match name {
        "Spring" => Season::Spring,
        "Summer" => Season::Summer,
        "Autumn" => Season::Autumn,
        "Winter" => Season::Winter,
        _ => return None,
    })
}

fn parse_life_stage(name: &str) -> Option<LifeStage> {
    Some(match name {
        "Child" => LifeStage::Child,
        "Adult" => LifeStage::Adult,
        "Elder" => LifeStage::Elder,
        _ => return None,
    })
}

fn parse_importance(name: &str) -> Option<EventImportance> {
    Some(match name {
        "Minor" => EventImportance::Minor,
        "Standard" => EventImportance::Standard,
        "Major" => EventImportance::Major,
        "Legendary" => EventImportance::Legendary,
        _ => return None,
    })
}

fn parse_tech_status(name: &str) -> Option<TechStatus> {
    Some(match name {
        "Active" => TechStatus::Active,
        "Corrupted" => TechStatus::Corrupted,
        _ => return None,
    })
}

fn parse_material(name: &str) -> Option<MaterialType> {
    Some(match name {
        "Wood" => MaterialType::Wood,
        "Stone" => MaterialType::Stone,
        "Metal" => MaterialType::Metal,
        "Gold" => MaterialType::Gold,
        _ => return None,
    })
}

fn parse_scenario(name: &str) -> Option<StartScenarioId> {
    Some(match name {
        "Classic" => StartScenarioId::Classic,
        "GroundSurvival" => StartScenarioId::GroundSurvival,
        "SocialDrama" => StartScenarioId::SocialDrama,
        "Layer2Ready" => StartScenarioId::Layer2Ready,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// Capture a full snapshot of `world`. `saved_at_unix` is supplied by the
/// caller (wall clock in production, a fixed value in tests).
#[must_use]
pub fn capture_snapshot(world: &mut World, saved_at_unix: u64) -> SaveSnapshot {
    let tick = world
        .get_resource::<SimulationTime>()
        .map_or(0, |t| t.tick);
    let colony_name = world
        .get_resource::<ColonyName>()
        .map_or_else(|| "The Colony".to_string(), |c| c.name.clone());
    let world_seed = world.get_resource::<WorldSeed>().map_or(0, |s| s.0);
    let scenario = world
        .get_resource::<ActiveStartScenario>()
        .map(|s| name_of(&s.id));
    let origin_choice = world
        .get_resource::<OriginChoice>()
        .and_then(|c| c.chosen)
        .map(|k| k.id().to_string());
    let global_rng_reseed = world
        .get_resource::<GlobalRng>()
        .map(|_| world_seed ^ tick);
    let resources = world
        .get_resource::<ColonyResources>()
        .copied()
        .unwrap_or_default();
    let chronicle = world.get_resource::<Chronicle>().map_or_else(
        Vec::new,
        |c| {
            c.events
                .iter()
                .map(|e| ChronicleEventSave {
                    tick: e.tick,
                    year: e.year,
                    text: e.text.clone(),
                    importance: name_of(&e.importance),
                })
                .collect()
        },
    );
    let tech = world.get_resource::<TechState>().map_or(
        TechSave {
            entries: Vec::new(),
            total_capacity: 0.0,
            used_capacity: 0.0,
        },
        |t| {
            let mut entries: Vec<(String, String)> = t
                .techs
                .iter()
                .map(|(tech, status)| (name_of(tech), name_of(status)))
                .collect();
            entries.sort();
            TechSave {
                entries,
                total_capacity: t.total_capacity,
                used_capacity: t.used_capacity,
            }
        },
    );
    let season = world
        .get_resource::<SeasonState>()
        .map_or_else(|| "Spring".to_string(), |s| name_of(&s.current_season));
    let day_night = world.get_resource::<DayNightCycle>().map_or(
        DayNightSave {
            time_of_day: "Day".to_string(),
            day_count: 0,
            ticks_per_day: 250,
        },
        |d| DayNightSave {
            time_of_day: name_of(&d.time_of_day),
            day_count: d.day_count,
            ticks_per_day: d.ticks_per_day,
        },
    );
    let weather = world.get_resource::<WeatherState>().map_or(
        WeatherSave {
            current: "Clear".to_string(),
            duration_remaining: 100,
        },
        |w| WeatherSave {
            current: name_of(&w.current_weather),
            duration_remaining: w.duration_remaining,
        },
    );
    let terrain = world.get_resource::<TerrainGrid>().map_or(
        TerrainSave {
            width: 0,
            height: 0,
            tiles: Vec::new(),
        },
        |t| TerrainSave {
            width: t.width,
            height: t.height,
            tiles: t.tiles.iter().map(name_of).collect(),
        },
    );
    let fertility = world
        .get_resource::<FertilityGrid>()
        .map(|f| f.values.clone());

    // --- Entities, in stable order (sorted by entity bits). ---
    let mut pop_ids: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, With<Pop>>();
        q.iter(world).collect()
    };
    pop_ids.sort_by_key(|e| e.to_bits());
    let pop_set: HashSet<Entity> = pop_ids.iter().copied().collect();

    let mut pops = Vec::new();
    for e in &pop_ids {
        let e = *e;
        let name = world
            .get::<PopName>(e)
            .map_or_else(String::new, |n| n.0.clone());
        let (x, y) = world.get::<GridPosition>(e).map_or((0, 0), |p| (p.x, p.y));
        let health = world
            .get::<Health>(e)
            .map(|h| (h.current, h.max, h.has_rust_lung));
        let needs = world
            .get::<Needs>(e)
            .map(|n| (n.hunger, n.rest, n.leisure, n.hygiene));
        let (morale_value, morale_modifiers) = world.get::<Morale>(e).map_or_else(
            || (None, Vec::new()),
            |m| {
                (
                    Some(m.value),
                    m.modifiers
                        .iter()
                        .map(|mo| (mo.label.clone(), mo.value, mo.duration))
                        .collect(),
                )
            },
        );
        let wallet_credits = world.get::<Wallet>(e).map(|w| w.credits);
        let mut skills: Vec<(String, f32)> = world.get::<Skills>(e).map_or_else(
            Vec::new,
            |s| {
                s.xp.iter()
                    .map(|(k, v)| (name_of(k), *v))
                    .collect()
            },
        );
        skills.sort_by(|a, b| a.0.cmp(&b.0));
        let mut traits: Vec<String> = world.get::<Traits>(e).map_or_else(Vec::new, |t| {
            t.0.iter().map(name_of).collect()
        });
        traits.sort();
        let age = world.get::<Age>(e).map(|a| (a.ticks_alive, name_of(&a.stage)));
        let action = world
            .get::<PopAction>(e)
            .map(|a| (name_of(&a.current), a.current_utility, a.ticks_committed));
        let weights = world
            .get::<UtilityWeights>(e)
            .map(|w| (w.distance_weight, w.availability_weight));
        let memories = world.get::<Memories>(e).map_or_else(Vec::new, |m| {
            m.items
                .iter()
                .map(|mem| {
                    (
                        name_of(&mem.memory_type),
                        mem.added_at,
                        mem.intensity,
                        mem.forged,
                    )
                })
                .collect()
        });
        let job = world
            .get::<Job>(e)
            .map(|j| (j.workplace.to_bits(), name_of(&j.job_type)));
        let mut relationships: Vec<(u64, f32)> = world.get::<Relationships>(e).map_or_else(
            Vec::new,
            |r| {
                r.affinities
                    .iter()
                    .map(|(target, v)| (target.to_bits(), *v))
                    .collect()
            },
        );
        relationships.sort_by_key(|(bits, _)| *bits);
        let movement_target = world.get::<MovementTarget>(e).map(|mt| {
            (
                mt.target_entity.to_bits(),
                mt.target_position.x,
                mt.target_position.y,
                name_of(&mt.for_action),
            )
        });
        // Spec 1384: worn EVA suit (the persistence exception, §4/§10).
        let worn_suit = world
            .get::<crate::layer1::items::Equipment>(e)
            .and_then(|eq| eq.body)
            .and_then(|body| world.get::<crate::layer1::eva_suit::EvaSuit>(body))
            .map(|suit| SuitSave {
                class: suit.class.save_name().to_string(),
                air: suit.air,
                durability: suit.durability,
                colony_issued: suit.colony_issued,
            });
        pops.push(PopSave {
            old_bits: e.to_bits(),
            name,
            x,
            y,
            health,
            needs,
            morale_value,
            morale_modifiers,
            wallet_credits,
            skills,
            traits,
            age,
            action,
            weights,
            memories,
            job,
            relationships,
            movement_target,
            worn_suit,
        });
    }

    let mut building_ids: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, With<Building>>();
        q.iter(world).collect()
    };
    building_ids.sort_by_key(|e| e.to_bits());
    let building_set: HashSet<Entity> = building_ids.iter().copied().collect();

    let mut buildings = Vec::new();
    for e in &building_ids {
        let e = *e;
        let Some(building) = world.get::<Building>(e) else {
            continue;
        };
        let building_type = name_of(&building.building_type);
        let (x, y) = world.get::<GridPosition>(e).map_or((0, 0), |p| (p.x, p.y));
        let material = world.get::<Material>(e).map(|m| name_of(&m.0));
        let health = world
            .get::<Health>(e)
            .map(|h| (h.current, h.max, h.has_rust_lung));
        let farm = world.get::<Farm>(e).map(|f| {
            (
                f.capacity,
                name_of(&f.selected_crop),
                f.workers.iter().map(|w| w.to_bits()).collect(),
            )
        });
        let housing = world.get::<Housing>(e).map(|h| {
            (
                h.capacity,
                h.residents.iter().map(|r| r.to_bits()).collect(),
            )
        });
        let stockpile = world.get::<Stockpile>(e).map(|s| {
            (s.food_bonus, s.wood_bonus, s.stone_bonus, s.waste_bonus)
        });
        buildings.push(BuildingSave {
            old_bits: e.to_bits(),
            building_type,
            x,
            y,
            material,
            health,
            farm,
            housing,
            stockpile,
        });
    }

    // Spec 1384: worn suit entities are captured via PopSave.worn_suit;
    // they must not double-save as loose items.
    let worn_suit_entities: HashSet<Entity> = {
        let mut q = world.query_filtered::<&crate::layer1::items::Equipment, With<Pop>>();
        q.iter(world).filter_map(|eq| eq.body).collect()
    };
    let mut items = Vec::new();
    {
        let mut q = world.query::<(Entity, &Item, &GridPosition)>();
        for (e, item, pos) in q.iter(world) {
            if pop_set.contains(&e) || building_set.contains(&e) {
                continue;
            }
            if worn_suit_entities.contains(&e) {
                continue;
            }
            let (item_type, item_ref, curio_name) = match &item.item_type {
                ItemType::Corpse(target) => {
                    ("Corpse".to_string(), Some(target.to_bits()), None)
                }
                ItemType::MemoryCore(target) => {
                    ("MemoryCore".to_string(), Some(target.to_bits()), None)
                }
                ItemType::Curio(name) => {
                    ("Curio".to_string(), None, Some((*name).to_string()))
                }
                other => (name_of(other), None, None),
            };
            // Spec 1384: loose physical EVA suits ride as items with a
            // suit payload (class/air/durability ride along exactly).
            let suit = world
                .get::<crate::layer1::eva_suit::EvaSuit>(e)
                .map(|s| SuitSave {
                    class: s.class.save_name().to_string(),
                    air: s.air,
                    durability: s.durability,
                    colony_issued: s.colony_issued,
                });
            items.push(ItemSave {
                item_type,
                x: pos.x,
                y: pos.y,
                item_ref,
                curio_name,
                suit,
            });
        }
    }
    items.sort_by(|a, b| {
        (a.x, a.y, &a.item_type, a.item_ref)
            .cmp(&(b.x, b.y, &b.item_type, b.item_ref))
    });

    let mut designations = Vec::new();
    {
        let mut q = world.query::<(Entity, &Designation, &GridPosition)>();
        for (e, designation, pos) in q.iter(world) {
            if pop_set.contains(&e) || building_set.contains(&e) {
                continue;
            }
            designations.push(DesignationSave {
                designation_type: name_of(&designation.designation_type),
                x: pos.x,
                y: pos.y,
            });
        }
    }
    designations.sort_by(|a, b| {
        (a.x, a.y, &a.designation_type).cmp(&(b.x, b.y, &b.designation_type))
    });

    SaveSnapshot {
        format_version: CURRENT_SAVE_VERSION,
        tick,
        saved_at_unix,
        colony_name,
        world_seed,
        scenario,
        origin_choice,
        global_rng_reseed,
        resources: ColonyResourcesSave::from(&resources),
        chronicle,
        tech,
        season,
        day_night,
        weather,
        terrain,
        fertility,
        pops,
        buildings,
        items,
        designations,
    }
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

/// Outcome of applying a snapshot: counts plus non-fatal warnings
/// (unparseable enum names, dropped cross-references, …).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LoadReport {
    /// Pops restored.
    pub pops: usize,
    /// Buildings restored.
    pub buildings: usize,
    /// Items restored.
    pub items: usize,
    /// Designations restored.
    pub designations: usize,
    /// Non-fatal restore warnings.
    pub warnings: Vec<String>,
}

/// Apply `snap` to `world`, replacing the captured state. Existing pops,
/// buildings, items and designations are despawned; restored pops and
/// buildings are respawned through the game's own bundle/spawn paths so
/// they carry every component those paths normally attach.
///
/// # Errors
/// [`SaveError::VersionMismatch`] if the snapshot version differs.
pub fn apply_snapshot(world: &mut World, snap: &SaveSnapshot) -> Result<LoadReport, SaveError> {
    if snap.format_version != CURRENT_SAVE_VERSION {
        return Err(SaveError::VersionMismatch {
            found: snap.format_version,
            expected: CURRENT_SAVE_VERSION,
        });
    }
    let mut report = LoadReport::default();

    // Despawn everything the snapshot owns.
    let mut to_despawn: Vec<Entity> = Vec::new();
    {
        let mut q = world.query_filtered::<Entity, With<Pop>>();
        to_despawn.extend(q.iter(world));
    }
    {
        let mut q = world.query_filtered::<Entity, With<Building>>();
        to_despawn.extend(q.iter(world));
    }
    {
        let mut q = world.query_filtered::<Entity, With<Item>>();
        to_despawn.extend(q.iter(world));
    }
    {
        let mut q = world.query_filtered::<Entity, With<Designation>>();
        to_despawn.extend(q.iter(world));
    }
    let despawn_set: HashSet<Entity> = to_despawn.iter().copied().collect();
    for e in &despawn_set {
        world.despawn(*e);
    }

    // Old entity bits -> new entity, for cross-reference remapping.
    let mut remap: HashMap<u64, Entity> = HashMap::new();

    // Buildings first: jobs / movement targets may point at them.
    for b in &snap.buildings {
        let Some(building_type) = parse_iter::<BuildingType>(&b.building_type) else {
            report
                .warnings
                .push(format!("unknown building type '{}'", b.building_type));
            continue;
        };
        let material = b
            .material
            .as_deref()
            .and_then(parse_material)
            .unwrap_or_default();
        let entity = spawn_building(world, b.x, b.y, building_type, material);
        remap.insert(b.old_bits, entity);
        report.buildings += 1;
        if let Some((current, max, rust)) = b.health {
            world.entity_mut(entity).insert(Health {
                current,
                max,
                has_rust_lung: rust,
            });
        }
        if let Some((capacity, crop_name, _)) = &b.farm {
            if let Some(mut farm) = world.get_mut::<Farm>(entity) {
                farm.capacity = *capacity;
                if let Some(crop) = parse_item_type(crop_name) {
                    farm.selected_crop = crop;
                }
            }
        }
        if let Some((capacity, _)) = &b.housing {
            if let Some(mut housing) = world.get_mut::<Housing>(entity) {
                housing.capacity = *capacity;
            }
        }
        if let Some((food, wood, stone, waste)) = b.stockpile {
            if let Some(mut stockpile) = world.get_mut::<Stockpile>(entity) {
                stockpile.food_bonus = food;
                stockpile.wood_bonus = wood;
                stockpile.stone_bonus = stone;
                stockpile.waste_bonus = waste;
            }
        }
        // Faithful restore: a component the snapshot says was ABSENT must
        // not be invented by spawn_building's per-type defaults.
        if b.health.is_none() {
            world.entity_mut(entity).remove::<Health>();
        }
        if b.farm.is_none() {
            world.entity_mut(entity).remove::<Farm>();
        }
        if b.housing.is_none() {
            world.entity_mut(entity).remove::<Housing>();
        }
        if b.stockpile.is_none() {
            world.entity_mut(entity).remove::<Stockpile>();
        }
    }

    // Pops: full PopBundle (seeded deterministically from the snapshot),
    // then every captured field overwritten.
    let mut new_pop_ids: Vec<Entity> = Vec::new();
    for p in &snap.pops {
        let mut rng = rand::rngs::StdRng::seed_from_u64(
            snap.world_seed ^ p.old_bits.wrapping_mul(0x9E37_79B9_7F4A_7C15),
        );
        let mut bundle = crate::layer1::pop::PopBundle::random(p.x, p.y, &mut rng);
        bundle.name = PopName(p.name.clone());
        if let Some((current, max, rust)) = p.health {
            bundle.health = Health {
                current,
                max,
                has_rust_lung: rust,
            };
        }
        if let Some((hunger, rest, leisure, hygiene)) = p.needs {
            bundle.needs = Needs {
                hunger,
                rest,
                leisure,
                hygiene,
            };
        }
        if let Some(value) = p.morale_value {
            bundle.morale = Morale {
                value,
                modifiers: p
                    .morale_modifiers
                    .iter()
                    .map(|(label, value, duration)| MoodModifier {
                        label: label.clone(),
                        value: *value,
                        duration: *duration,
                    })
                    .collect(),
            };
        }
        if let Some(credits) = p.wallet_credits {
            bundle.wallet = Wallet { credits };
        }
        // Unconditional (even when empty): the bundle's random seed must
        // never leak through. A pop saved with NO traits/skills/memories
        // must restore with none — guarding on is_empty() here was the
        // persistence round-trip flake (an empty-traits pop regained
        // random bundle traits on load, flipping the canonical digest
        // on roughly 40% of live-colony runs).
        {
            let mut xp = HashMap::new();
            for (name, value) in &p.skills {
                if let Some(skill) = parse_skill_type(name) {
                    xp.insert(skill, *value);
                }
            }
            bundle.skills = Skills { xp };
        }
        {
            let mut set = std::collections::HashSet::new();
            for name in &p.traits {
                if let Some(t) = parse_iter::<Trait>(name) {
                    set.insert(t);
                }
            }
            bundle.traits = Traits(set.into_iter().collect());
        }
        if let Some((ticks_alive, stage_name)) = &p.age {
            if let Some(stage) = parse_life_stage(stage_name) {
                bundle.age = Age {
                    ticks_alive: *ticks_alive,
                    stage,
                };
            }
        }
        if let Some((action_name, utility, ticks)) = &p.action {
            if let Some(current) = parse_iter::<ActionType>(action_name) {
                bundle.action = PopAction {
                    current,
                    current_utility: *utility,
                    ticks_committed: *ticks,
                };
            }
        }
        if let Some((distance, availability)) = p.weights {
            bundle.weights = UtilityWeights {
                distance_weight: distance,
                availability_weight: availability,
            };
        }
        {
            let items: Vec<ActiveMemory> = p
                .memories
                .iter()
                .filter_map(|(name, added_at, intensity, forged)| {
                    parse_memory_type(name).map(|memory_type| ActiveMemory {
                        memory_type,
                        added_at: *added_at,
                        intensity: *intensity,
                        forged: *forged,
                    })
                })
                .collect();
            bundle.memories = Memories { items };
        }
        let entity = world.spawn(bundle).id();
        // Faithful restore for absent components: PopBundle always carries
        // Health/Needs/Morale/Wallet/Age/PopAction/UtilityWeights (some
        // randomized), so a field captured as None must REMOVE the
        // bundle's component instead of letting invented values through.
        if p.health.is_none() {
            world.entity_mut(entity).remove::<Health>();
        }
        if p.needs.is_none() {
            world.entity_mut(entity).remove::<Needs>();
        }
        if p.morale_value.is_none() {
            world.entity_mut(entity).remove::<Morale>();
        }
        if p.wallet_credits.is_none() {
            world.entity_mut(entity).remove::<Wallet>();
        }
        if p.age.is_none() {
            world.entity_mut(entity).remove::<Age>();
        }
        if p.action.is_none() {
            world.entity_mut(entity).remove::<PopAction>();
        }
        if p.weights.is_none() {
            world.entity_mut(entity).remove::<UtilityWeights>();
        }
        remap.insert(p.old_bits, entity);
        new_pop_ids.push(entity);
        report.pops += 1;
    }

    // Cross-references, now that every entity map entry exists.
    for (p, &entity) in snap.pops.iter().zip(new_pop_ids.iter()) {
        if let Some((workplace_bits, job_name)) = &p.job {
            if let (Some(&workplace), Some(job_type)) = (
                remap.get(workplace_bits),
                parse_assignment_type(job_name),
            ) {
                world.entity_mut(entity).insert(Job {
                    workplace,
                    job_type,
                });
            }
        }
        if !p.relationships.is_empty() {
            let affinities: HashMap<Entity, f32> = p
                .relationships
                .iter()
                .filter_map(|(bits, v)| remap.get(bits).map(|&target| (target, *v)))
                .collect();
            if !affinities.is_empty() {
                world
                    .entity_mut(entity)
                    .insert(Relationships { affinities });
            }
        }
        if let Some((target_bits, tx, ty, action_name)) = &p.movement_target {
            if let (Some(&target), Some(for_action)) = (
                remap.get(target_bits),
                parse_iter::<ActionType>(action_name),
            ) {
                world.entity_mut(entity).insert(MovementTarget {
                    target_entity: target,
                    target_position: GridPosition { x: *tx, y: *ty },
                    for_action,
                });
            }
        }
        // Spec 1384: restore the worn suit into the body slot (the
        // persistence exception — a suited pop in vacuum must not
        // come back naked).
        if let Some(suit_save) = &p.worn_suit {
            if let Some(class) =
                crate::layer1::eva_suit::SuitClass::from_save_name(&suit_save.class)
            {
                let suit_entity = world
                    .spawn((
                        Item::default(),
                        crate::layer1::eva_suit::EvaSuit {
                            class,
                            air: suit_save.air,
                            durability: suit_save.durability,
                            max_durability: 100.0,
                            colony_issued: suit_save.colony_issued,
                        },
                        GridPosition { x: p.x, y: p.y },
                    ))
                    .id();
                if let Some(mut eq) =
                    world.get_mut::<crate::layer1::items::Equipment>(entity)
                {
                    eq.body = Some(suit_entity);
                } else {
                    world.entity_mut(entity).insert(crate::layer1::items::Equipment {
                        body: Some(suit_entity),
                        ..Default::default()
                    });
                }
            }
        }
    }
    // Farm workers / housing residents.
    for b in &snap.buildings {
        let Some(&entity) = remap.get(&b.old_bits) else {
            continue;
        };
        if let Some((_, _, workers)) = &b.farm {
            if let Some(mut farm) = world.get_mut::<Farm>(entity) {
                farm.workers = workers.iter().filter_map(|w| remap.get(w).copied()).collect();
            }
        }
        if let Some((_, residents)) = &b.housing {
            if let Some(mut housing) = world.get_mut::<Housing>(entity) {
                housing.residents =
                    residents.iter().filter_map(|r| remap.get(r).copied()).collect();
            }
        }
    }

    // Items and designations.
    for item in &snap.items {
        // Spec 1384: loose EVA suits restore with their suit payload,
        // regardless of how the base item type name resolves.
        if let Some(suit_save) = &item.suit {
            if let Some(class) =
                crate::layer1::eva_suit::SuitClass::from_save_name(&suit_save.class)
            {
                world.spawn((
                    Item::default(),
                    crate::layer1::eva_suit::EvaSuit {
                        class,
                        air: suit_save.air,
                        durability: suit_save.durability,
                        max_durability: 100.0,
                        colony_issued: suit_save.colony_issued,
                    },
                    GridPosition {
                        x: item.x,
                        y: item.y,
                    },
                ));
                report.items += 1;
                continue;
            }
        }
        let resolved = match item.item_type.as_str() {
            "Corpse" | "MemoryCore" => item.item_ref.and_then(|bits| remap.get(&bits)).map(
                |&target| {
                    if item.item_type == "Corpse" {
                        ItemType::Corpse(target)
                    } else {
                        ItemType::MemoryCore(target)
                    }
                },
            ),
            "Curio" => item.curio_name.as_deref().map(|name| {
                let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
                ItemType::Curio(leaked)
            }),
            _ => parse_item_type(&item.item_type),
        };
        if let Some(item_type) = resolved {
            world.spawn((
                Item { item_type },
                GridPosition {
                    x: item.x,
                    y: item.y,
                },
            ));
            report.items += 1;
        } else {
            report
                .warnings
                .push(format!("unknown item type '{}'", item.item_type));
        }
    }
    for designation in &snap.designations {
        if let Some(designation_type) = parse_designation_type(&designation.designation_type) {
            world.spawn((
                Designation { designation_type },
                GridPosition {
                    x: designation.x,
                    y: designation.y,
                },
            ));
            report.designations += 1;
        } else {
            report.warnings.push(format!(
                "unknown designation type '{}'",
                designation.designation_type
            ));
        }
    }

    // Resources.
    let speed = world
        .get_resource::<SimulationTime>()
        .map_or(crate::shared::time::SimSpeed::Normal, |t| t.speed);
    world.insert_resource(SimulationTime {
        tick: snap.tick,
        speed,
    });
    world.insert_resource(WorldSeed(snap.world_seed));
    world.insert_resource(ColonyName {
        name: snap.colony_name.clone(),
    });
    world.insert_resource(ColonyResources::from(&snap.resources));
    world.insert_resource(Chronicle {
        events: snap
            .chronicle
            .iter()
            .map(|e| crate::layer1::ChronicleEvent {
                tick: e.tick,
                year: e.year,
                text: e.text.clone(),
                importance: parse_importance(&e.importance).unwrap_or(EventImportance::Standard),
            })
            .collect(),
    });
    {
        let mut techs = HashMap::new();
        for (tech_name, status_name) in &snap.tech.entries {
            if let (Some(tech), Some(status)) = (
                parse_iter::<Tech>(tech_name),
                parse_tech_status(status_name),
            ) {
                techs.insert(tech, status);
            }
        }
        world.insert_resource(TechState {
            techs,
            total_capacity: snap.tech.total_capacity,
            used_capacity: snap.tech.used_capacity,
        });
    }
    if let Some(season) = parse_season(&snap.season) {
        world.insert_resource(SeasonState {
            current_season: season,
        });
    }
    if let Some(time_of_day) = parse_time_of_day(&snap.day_night.time_of_day) {
        world.insert_resource(DayNightCycle {
            time_of_day,
            day_count: snap.day_night.day_count,
            ticks_per_day: snap.day_night.ticks_per_day,
        });
    }
    if let Some(current_weather) = parse_weather(&snap.weather.current) {
        world.insert_resource(WeatherState {
            current_weather,
            duration_remaining: snap.weather.duration_remaining,
        });
    }
    if snap.terrain.width > 0 {
        let tiles: Vec<TerrainType> = snap
            .terrain
            .tiles
            .iter()
            .map(|name| parse_terrain(name).unwrap_or(TerrainType::Grass))
            .collect();
        world.insert_resource(TerrainGrid {
            width: snap.terrain.width,
            height: snap.terrain.height,
            tiles,
        });
        if let Some(values) = &snap.fertility {
            if values.len() == snap.terrain.width * snap.terrain.height {
                world.insert_resource(FertilityGrid {
                    width: snap.terrain.width,
                    height: snap.terrain.height,
                    values: values.clone(),
                });
            }
        }
    }
    if let Some(reseed) = snap.global_rng_reseed {
        world.insert_resource(GlobalRng(rand::rngs::StdRng::seed_from_u64(reseed)));
    }
    match &snap.origin_choice {
        Some(id) => {
            if let Some(kind) = OriginKind::parse(id) {
                world.insert_resource(OriginChoice { chosen: Some(kind) });
            }
        }
        None => {
            world.insert_resource(OriginChoice { chosen: None });
        }
    }
    if let Some(name) = &snap.scenario {
        if let Some(id) = parse_scenario(name) {
            let def = start_scenario_definition(id);
            world.insert_resource(ActiveStartScenario {
                id,
                name: def.name,
                difficulty: def.difficulty,
            });
        }
    }

    // Rebuild derived spatial indices so the next tick sees the restored
    // world, not the pre-load one.
    if let Some(mut map) = world.get_resource_mut::<crate::layer1::building::BuildingMap>() {
        map.0.clear();
        for b in &snap.buildings {
            if let Some(&entity) = remap.get(&b.old_bits) {
                map.0.insert((b.x, b.y), entity);
            }
        }
    }
    if let Some(mut occupied) = world.get_resource_mut::<crate::layer1::OccupiedTiles>() {
        occupied.0.clear();
        for b in &snap.buildings {
            occupied.0.insert((b.x, b.y));
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Serialization + store-level helpers
// ---------------------------------------------------------------------------

/// Serialize a snapshot to its stored blob form (JSON).
///
/// # Errors
/// [`SaveError::Serialize`] on encoder failure.
pub fn snapshot_to_bytes(snap: &SaveSnapshot) -> Result<Vec<u8>, SaveError> {
    serde_json::to_vec(snap).map_err(|e| SaveError::Serialize(e.to_string()))
}

/// Parse a stored blob back into a snapshot. The format version is
/// checked from the header FIRST, so a mismatched save fails with the
/// typed [`SaveError::VersionMismatch`] instead of a confusing
/// deserialization error or a panic.
///
/// # Errors
/// [`SaveError::VersionMismatch`] or [`SaveError::Serialize`].
pub fn snapshot_from_bytes(data: &[u8]) -> Result<SaveSnapshot, SaveError> {
    // Header pass: surfaces VersionMismatch before full parsing.
    let _ = metadata_from_blob("probe", data)?;
    serde_json::from_slice(data).map_err(|e| SaveError::Serialize(e.to_string()))
}

/// Canonical digest of a snapshot: two snapshots of the same colony
/// state digest identically even though a load respawns entities under
/// new ids. Entity identity is replaced by a content key
/// (`pop:<name>:<x>:<y>:<occurrence>` / `bld:<type>:<x>:<y>`), cross-
/// references are replaced by the target's content key, records are
/// sorted by content, and `saved_at_unix` is zeroed. Occurrence indices
/// disambiguate same-name/same-tile pops by sorting the colliding group
/// by content, so the digest is invariant under respawn ordering (up to
/// isomorphism between truly identical records, which are
/// interchangeable by definition).
#[must_use]
pub fn snapshot_digest(snap: &SaveSnapshot) -> String {
    use std::collections::BTreeMap;
    let mut value = serde_json::to_value(snap).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("saved_at_unix".to_string(), serde_json::json!(0));
    }
    // Content keys for every entity, computed from a ref-free rendering.
    let ref_free = |record: &serde_json::Value, is_pop: bool| -> String {
        let mut record = record.clone();
        if let Some(obj) = record.as_object_mut() {
            obj.insert("old_bits".to_string(), serde_json::json!(0));
            if is_pop {
                obj.insert("job".to_string(), serde_json::Value::Null);
                obj.insert("relationships".to_string(), serde_json::json!([]));
                if let Some(mt) = obj.get_mut("movement_target") {
                    if let Some(arr) = mt.as_array_mut() {
                        if !arr.is_empty() {
                            arr[0] = serde_json::json!(0);
                        }
                    }
                }
            } else {
                if let Some(farm) = obj.get_mut("farm") {
                    if let Some(arr) = farm.as_array_mut() {
                        if arr.len() > 2 {
                            arr[2] = serde_json::json!([]);
                        }
                    }
                }
                if let Some(housing) = obj.get_mut("housing") {
                    if let Some(arr) = housing.as_array_mut() {
                        if arr.len() > 1 {
                            arr[1] = serde_json::json!([]);
                        }
                    }
                }
            }
        }
        serde_json::to_string(&record).unwrap_or_default()
    };
    let mut key_of: BTreeMap<u64, String> = BTreeMap::new();
    if let Some(pops) = value.get("pops").and_then(|v| v.as_array()) {
        let mut groups: BTreeMap<String, Vec<(String, u64)>> = BTreeMap::new();
        for pop in pops {
            let name = pop.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let x = pop.get("x").and_then(serde_json::Value::as_i64).unwrap_or(0);
            let y = pop.get("y").and_then(serde_json::Value::as_i64).unwrap_or(0);
            let bits = pop
                .get("old_bits")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            groups
                .entry(format!("pop:{name}:{x}:{y}"))
                .or_default()
                .push((ref_free(pop, true), bits));
        }
        for (base, mut members) in groups {
            members.sort();
            for (occ, (_, bits)) in members.into_iter().enumerate() {
                key_of.insert(bits, format!("{base}:{occ}"));
            }
        }
    }
    if let Some(buildings) = value.get("buildings").and_then(|v| v.as_array()) {
        // Same occurrence disambiguation as pops: two buildings of the same
        // type on the same tile must not collide onto one key.
        let mut groups: BTreeMap<String, Vec<(String, u64)>> = BTreeMap::new();
        for b in buildings {
            let ty = b
                .get("building_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let x = b.get("x").and_then(serde_json::Value::as_i64).unwrap_or(0);
            let y = b.get("y").and_then(serde_json::Value::as_i64).unwrap_or(0);
            let bits = b
                .get("old_bits")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            groups
                .entry(format!("bld:{ty}:{x}:{y}"))
                .or_default()
                .push((ref_free(b, false), bits));
        }
        for (base, mut members) in groups {
            members.sort();
            for (occ, (_, bits)) in members.into_iter().enumerate() {
                key_of.insert(bits, format!("{base}:{occ}"));
            }
        }
    }
    let key_json = |bits: u64| -> serde_json::Value {
        key_of
            .get(&bits)
            .map_or_else(|| serde_json::json!("missing"), |k| serde_json::json!(k))
    };
    if let Some(pops) = value.get_mut("pops").and_then(|v| v.as_array_mut()) {
        for pop in pops.iter_mut() {
            let bits = pop
                .get("old_bits")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if let Some(obj) = pop.as_object_mut() {
                obj.insert("old_bits".to_string(), key_json(bits));
                if let Some(job) = obj.get_mut("job") {
                    if let Some(arr) = job.as_array_mut() {
                        if let Some(target) = arr.first().and_then(serde_json::Value::as_u64) {
                            arr[0] = key_json(target);
                        }
                    }
                }
                if let Some(rels) = obj.get_mut("relationships") {
                    if let Some(arr) = rels.as_array_mut() {
                        for rel in arr.iter_mut() {
                            if let Some(pair) = rel.as_array_mut() {
                                if let Some(target) =
                                    pair.first().and_then(serde_json::Value::as_u64)
                                {
                                    pair[0] = key_json(target);
                                }
                            }
                        }
                        // Capture sorts affinities by RAW entity bits, whose
                        // order a load legitimately reshuffles; the canonical
                        // form sorts by the replacement content key instead.
                        arr.sort_by_key(|rel| serde_json::to_string(rel).unwrap_or_default());
                    }
                }
                if let Some(mt) = obj.get_mut("movement_target") {
                    if let Some(arr) = mt.as_array_mut() {
                        if let Some(target) = arr.first().and_then(serde_json::Value::as_u64) {
                            arr[0] = key_json(target);
                        }
                    }
                }
            }
        }
        pops.sort_by_key(|pop| serde_json::to_string(pop).unwrap_or_default());
    }
    if let Some(buildings) = value.get_mut("buildings").and_then(|v| v.as_array_mut()) {
        for b in buildings.iter_mut() {
            let bits = b
                .get("old_bits")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if let Some(obj) = b.as_object_mut() {
                obj.insert("old_bits".to_string(), key_json(bits));
                if let Some(farm) = obj.get_mut("farm") {
                    if let Some(arr) = farm.as_array_mut() {
                        if let Some(workers) = arr.get_mut(2).and_then(|w| w.as_array_mut()) {
                            for w in workers.iter_mut() {
                                if let Some(target) = w.as_u64() {
                                    *w = key_json(target);
                                }
                            }
                            workers
                                .sort_by_key(|w| serde_json::to_string(w).unwrap_or_default());
                        }
                    }
                }
                if let Some(housing) = obj.get_mut("housing") {
                    if let Some(arr) = housing.as_array_mut() {
                        if let Some(residents) = arr.get_mut(1).and_then(|r| r.as_array_mut()) {
                            for r in residents.iter_mut() {
                                if let Some(target) = r.as_u64() {
                                    *r = key_json(target);
                                }
                            }
                            residents
                                .sort_by_key(|r| serde_json::to_string(r).unwrap_or_default());
                        }
                    }
                }
            }
        }
        buildings.sort_by_key(|b| serde_json::to_string(b).unwrap_or_default());
    }
    if let Some(items) = value.get_mut("items").and_then(|v| v.as_array_mut()) {
        // ItemSave.item_ref is a raw entity reference (Corpse/MemoryCore
        // payloads): canonicalize it like every other cross-reference and
        // sort records by content. Previously items entered the digest
        // with raw bits, so any corpse/memory-core item flipped the
        // digest on every load.
        for item in items.iter_mut() {
            if let Some(obj) = item.as_object_mut() {
                if let Some(bits) = obj
                    .get("item_ref")
                    .and_then(serde_json::Value::as_u64)
                {
                    obj.insert("item_ref".to_string(), key_json(bits));
                }
            }
        }
        items.sort_by_key(|i| serde_json::to_string(i).unwrap_or_default());
    }
    if let Some(designations) = value
        .get_mut("designations")
        .and_then(|v| v.as_array_mut())
    {
        designations.sort_by_key(|d| serde_json::to_string(d).unwrap_or_default());
    }
    let json = serde_json::to_string(&value).unwrap_or_default();
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    json.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Save the world into `store` under `slot`. The reserved `autosave`
/// slot is rejected here — only [`save_world_autosave`] may write it.
///
/// # Errors
/// Typed [`SaveError`] on sanitize/serialize/backend failure.
pub fn save_world(
    world: &mut World,
    store: &mut dyn SaveStore,
    slot: &str,
    saved_at_unix: u64,
) -> Result<SaveMetadata, SaveError> {
    let slot = sanitize_slot(slot)?;
    if slot == AUTOSAVE_SLOT {
        return Err(SaveError::ReservedSlot(slot));
    }
    save_world_inner(world, store, &slot, saved_at_unix)
}

/// Save the world into the reserved autosave slot.
///
/// # Errors
/// Typed [`SaveError`] on serialize/backend failure.
pub fn save_world_autosave(
    world: &mut World,
    store: &mut dyn SaveStore,
    saved_at_unix: u64,
) -> Result<SaveMetadata, SaveError> {
    save_world_inner(world, store, AUTOSAVE_SLOT, saved_at_unix)
}

fn save_world_inner(
    world: &mut World,
    store: &mut dyn SaveStore,
    slot: &str,
    saved_at_unix: u64,
) -> Result<SaveMetadata, SaveError> {
    let snap = capture_snapshot(world, saved_at_unix);
    let bytes = snapshot_to_bytes(&snap)?;
    store.put(slot, &bytes)?;
    Ok(SaveMetadata {
        slot: slot.to_string(),
        format_version: snap.format_version,
        tick: snap.tick,
        colony_name: snap.colony_name.clone(),
        saved_at_unix: snap.saved_at_unix,
        size_bytes: bytes.len(),
    })
}

/// Load the save in `slot` from `store` into `world`.
///
/// # Errors
/// [`SaveError::SlotNotFound`] if the slot is empty; version/parse
/// errors from [`snapshot_from_bytes`].
pub fn load_world(
    world: &mut World,
    store: &dyn SaveStore,
    slot: &str,
) -> Result<LoadReport, SaveError> {
    let slot = sanitize_slot(slot)?;
    let data = store
        .get(&slot)?
        .ok_or_else(|| SaveError::SlotNotFound(slot.clone()))?;
    let snap = snapshot_from_bytes(&data)?;
    apply_snapshot(world, &snap)
}

/// List metadata for every save in `store`.
///
/// # Errors
/// Typed [`SaveError`] on backend failure.
pub fn list_saves(store: &dyn SaveStore) -> Result<Vec<SaveMetadata>, SaveError> {
    store.list()
}

/// Export a stored save to a plain file (the player's permanent backup;
/// on native this is also all "export" means — saves are files already).
///
/// # Errors
/// Typed [`SaveError`] on missing slot or I/O failure.
#[cfg(not(target_arch = "wasm32"))]
pub fn export_save(
    store: &dyn SaveStore,
    slot: &str,
    path: &std::path::Path,
) -> Result<(), SaveError> {
    let slot = sanitize_slot(slot)?;
    let data = store
        .get(&slot)?
        .ok_or_else(|| SaveError::SlotNotFound(slot.clone()))?;
    std::fs::write(path, &data)
        .map_err(|e| SaveError::Io(format!("writing {}: {e}", path.display())))
}

/// Import a `.scale-save` file into `slot`, validating its header first.
///
/// # Errors
/// Typed [`SaveError`] on unreadable file, bad header, or version mismatch.
#[cfg(not(target_arch = "wasm32"))]
pub fn import_save(
    store: &mut dyn SaveStore,
    slot: &str,
    path: &std::path::Path,
) -> Result<SaveMetadata, SaveError> {
    let slot = sanitize_slot(slot)?;
    let data = std::fs::read(path)
        .map_err(|e| SaveError::Io(format!("reading {}: {e}", path.display())))?;
    let meta = metadata_from_blob(&slot, &data)?;
    store.put(&slot, &data)?;
    Ok(meta)
}

// ---------------------------------------------------------------------------
// Autosave + request/completion events (the sync ECS <-> async store seam)
// ---------------------------------------------------------------------------

/// Per-world autosave bookkeeping.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct AutosaveState {
    /// Tick of the last autosave (0 = never).
    pub last_autosave_tick: u64,
}

/// Is an autosave due at `tick`, given the last autosave tick?
#[must_use]
pub fn autosave_due(tick: u64, last_autosave_tick: u64) -> bool {
    tick > 0 && tick.is_multiple_of(AUTOSAVE_INTERVAL_TICKS) && tick != last_autosave_tick
}

/// Write an autosave if one is due, tracking state in the world's
/// [`AutosaveState`] resource (no-op when the resource is absent).
/// Autosave writes ONLY the reserved `autosave` slot.
///
/// # Errors
/// Typed [`SaveError`] on serialize/backend failure.
pub fn maybe_autosave(
    world: &mut World,
    store: &mut dyn SaveStore,
    saved_at_unix: u64,
) -> Result<bool, SaveError> {
    let tick = world
        .get_resource::<SimulationTime>()
        .map_or(0, |t| t.tick);
    let last = world
        .get_resource::<AutosaveState>()
        .map_or(0, |s| s.last_autosave_tick);
    if world.get_resource::<AutosaveState>().is_none() || !autosave_due(tick, last) {
        return Ok(false);
    }
    save_world_autosave(world, store, saved_at_unix)?;
    if let Some(mut state) = world.get_resource_mut::<AutosaveState>() {
        state.last_autosave_tick = tick;
    }
    Ok(true)
}

/// Kind of a save/load request (bridge events, spec §4 WASM design).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveLoadKind {
    /// Write a slot.
    Save,
    /// Read a slot.
    Load,
    /// List slots.
    List,
    /// Delete a slot.
    Delete,
}

/// Request event: the (sync) game asks for a store operation. Native and
/// memory backends may service it immediately; the WASM IndexedDB bridge
/// services it asynchronously and answers with [`SaveLoadCompleted`].
#[derive(Event, Debug, Clone)]
pub struct SaveLoadRequest {
    /// Operation requested.
    pub kind: SaveLoadKind,
    /// Slot the operation targets (empty for List).
    pub slot: String,
}

/// Completion event delivered by a store bridge when an operation
/// finishes (immediately for sync backends, on IDB callback for WASM).
#[derive(Event, Debug, Clone)]
pub struct SaveLoadCompleted {
    /// Operation that finished.
    pub kind: SaveLoadKind,
    /// Slot the operation targeted (empty for List).
    pub slot: String,
    /// Whether it succeeded.
    pub ok: bool,
    /// Human-readable outcome, surfaced to the player — never a panic.
    pub message: String,
}

/// Initialize the save system's world resources (events + autosave
/// state). Called from world setup.
pub fn init_save_system(world: &mut World) {
    world.init_resource::<Events<SaveLoadRequest>>();
    world.init_resource::<Events<SaveLoadCompleted>>();
    world.init_resource::<AutosaveState>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::{setup_world_with_config, SetupConfig};
    use crate::simulation::run_simulation_tick;

    fn bare_world(tick: u64, food: f32) -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick,
            speed: crate::shared::time::SimSpeed::Normal,
        });
        world.insert_resource(ColonyResources {
            food,
            ..ColonyResources::default()
        });
        world.insert_resource(ColonyName {
            name: "Test Colony".to_string(),
        });
        world.insert_resource(WorldSeed(7));
        world
    }

    #[test]
    fn test_save_carries_format_version() {
        let mut world = bare_world(12, 10.0);
        let snap = capture_snapshot(&mut world, 42);
        let bytes = snapshot_to_bytes(&snap).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        // format_version is the FIRST field of the serialized snapshot.
        assert!(
            text.starts_with("{\"format_version\":2"),
            "snapshot JSON must start with format_version, got: {}",
            &text[..text.len().min(60)]
        );
        let parsed = snapshot_from_bytes(&bytes).unwrap();
        assert_eq!(parsed.format_version, CURRENT_SAVE_VERSION);

        // A mismatched version fails with the typed error, never a panic.
        let mut bad = snap.clone();
        bad.format_version = CURRENT_SAVE_VERSION + 99;
        let bad_bytes = snapshot_to_bytes(&bad).unwrap();
        let err = snapshot_from_bytes(&bad_bytes).unwrap_err();
        assert_eq!(
            err,
            SaveError::VersionMismatch {
                found: CURRENT_SAVE_VERSION + 99,
                expected: CURRENT_SAVE_VERSION,
            }
        );
        let err = apply_snapshot(&mut world, &bad).unwrap_err();
        assert!(matches!(err, SaveError::VersionMismatch { .. }));
    }

    #[test]
    fn test_save_slots_are_independent() {
        let mut store = MemorySaveStore::new();
        let mut world_a = bare_world(100, 33.0);
        let mut world_b = bare_world(200, 77.0);
        save_world(&mut world_a, &mut store, "slot1", 1).unwrap();
        save_world(&mut world_b, &mut store, "slot2", 2).unwrap();

        let mut world = bare_world(0, 0.0);
        load_world(&mut world, &store, "slot1").unwrap();
        assert_eq!(world.resource::<SimulationTime>().tick, 100);
        assert!((world.resource::<ColonyResources>().food - 33.0).abs() < f32::EPSILON);

        load_world(&mut world, &store, "slot2").unwrap();
        assert_eq!(world.resource::<SimulationTime>().tick, 200);
        assert!((world.resource::<ColonyResources>().food - 77.0).abs() < f32::EPSILON);

        // Missing slot is a typed error, not a panic.
        let err = load_world(&mut world, &store, "nope").unwrap_err();
        assert_eq!(err, SaveError::SlotNotFound("nope".to_string()));
    }

    #[test]
    fn test_autosave_does_not_clobber_manual_slots() {
        let mut store = MemorySaveStore::new();
        let mut world = bare_world(500, 10.0);
        world.init_resource::<AutosaveState>();
        save_world(&mut world, &mut store, "manual", 5).unwrap();
        let manual_before = store.get("manual").unwrap().unwrap();

        // Manual saves may not target the reserved slot.
        let err = save_world(&mut world, &mut store, AUTOSAVE_SLOT, 5).unwrap_err();
        assert_eq!(err, SaveError::ReservedSlot(AUTOSAVE_SLOT.to_string()));

        // Autosave is not due at tick 500; due at 1000, writes only autosave.
        assert!(!maybe_autosave(&mut world, &mut store, 6).unwrap());
        world.resource_mut::<SimulationTime>().tick = 1000;
        assert!(maybe_autosave(&mut world, &mut store, 6).unwrap());
        assert!(store.get(AUTOSAVE_SLOT).unwrap().is_some());
        assert_eq!(store.get("manual").unwrap().unwrap(), manual_before);
        // Not due again at the same tick.
        assert!(!maybe_autosave(&mut world, &mut store, 6).unwrap());
    }

    #[test]
    fn test_list_saves_returns_metadata() {
        let mut store = MemorySaveStore::new();
        let mut world = bare_world(321, 10.0);
        save_world(&mut world, &mut store, "alpha", 1234).unwrap();
        world.resource_mut::<SimulationTime>().tick = 654;
        save_world(&mut world, &mut store, "beta", 5678).unwrap();

        let list = list_saves(&store).unwrap();
        assert_eq!(list.len(), 2);
        let alpha = list.iter().find(|m| m.slot == "alpha").unwrap();
        assert_eq!(alpha.tick, 321);
        assert_eq!(alpha.colony_name, "Test Colony");
        assert_eq!(alpha.format_version, CURRENT_SAVE_VERSION);
        assert_eq!(alpha.saved_at_unix, 1234);
        assert!(alpha.size_bytes > 0);
        let beta = list.iter().find(|m| m.slot == "beta").unwrap();
        assert_eq!(beta.tick, 654);
    }

    #[test]
    fn test_slot_sanitization_rejects_traversal() {
        assert!(sanitize_slot("../evil").is_err());
        assert!(sanitize_slot("a/b").is_err());
        assert!(sanitize_slot("").is_err());
        assert!(sanitize_slot("ok-slot_1").is_ok());
        let mut store = MemorySaveStore::new();
        assert!(store.put("../evil", b"{}").is_err());
    }

    #[test]
    fn test_save_load_round_trip_preserves_state() {
        // Live world: run 250 ticks, snapshot, advance, load, and the
        // snapshot digest must match the save point exactly.
        // Colonies are stochastic (random origin roster, thread-rng
        // events) and a run can occasionally wipe before tick 250;
        // retry with a fresh world until one is alive at the save
        // point. The round-trip assertions below are exact either way.
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        });
        for _ in 0..3 {
            *world.resource_mut::<crate::shared::state::GameState>() =
                crate::shared::state::GameState::Running;
            for _ in 0..250 {
                run_simulation_tick(&mut world);
            }
            let alive = {
                let mut q = world.query_filtered::<Entity, With<Pop>>();
                q.iter(&world).count()
            };
            if alive > 0 {
                break;
            }
            world = setup_world_with_config(SetupConfig {
                headless: true,
                ..Default::default()
            });
        }
        let mut store = MemorySaveStore::new();
        let meta = save_world(&mut world, &mut store, "test", 42).unwrap();
        assert_eq!(meta.tick, 250);
        let at_save = capture_snapshot(&mut world, 42);
        let digest_at_save = snapshot_digest(&at_save);
        assert!(!at_save.pops.is_empty(), "round-trip needs a live colony");

        for _ in 0..100 {
            run_simulation_tick(&mut world);
        }
        assert_eq!(world.resource::<SimulationTime>().tick, 350);

        let report = load_world(&mut world, &store, "test").unwrap();
        assert_eq!(report.pops, at_save.pops.len());
        assert_eq!(report.buildings, at_save.buildings.len());
        assert_eq!(world.resource::<SimulationTime>().tick, 250);

        let after_load = capture_snapshot(&mut world, 42);
        assert_eq!(
            snapshot_digest(&after_load),
            digest_at_save,
            "snapshot after load must equal snapshot at save point"
        );

        // Continuing after load: the restored colony keeps simulating.
        // (Bit-identical continuation vs an uninterrupted run is NOT
        // asserted: much of the sim draws from thread_rng, which no save
        // can snapshot — see module docs. The save-point state is exact.)
        for _ in 0..100 {
            run_simulation_tick(&mut world);
        }
        assert_eq!(world.resource::<SimulationTime>().tick, 350);
        let continued = capture_snapshot(&mut world, 42);
        assert!(!continued.pops.is_empty());
    }

    #[test]
    fn test_save_load_round_trip_empty_and_absent_fields() {
        // Regression for the round-trip flake: a pop saved with EMPTY
        // traits/skills/memories and ABSENT optional components must
        // restore exactly so — the random PopBundle seed and spawn
        // defaults must not leak invented values into the recapture.
        let snap = SaveSnapshot {
            format_version: CURRENT_SAVE_VERSION,
            tick: 7,
            saved_at_unix: 42,
            colony_name: "Test Colony".to_string(),
            world_seed: 7,
            scenario: None,
            origin_choice: None,
            global_rng_reseed: None,
            resources: ColonyResourcesSave::from(&ColonyResources::default()),
            chronicle: Vec::new(),
            tech: TechSave {
                entries: Vec::new(),
                total_capacity: 0.0,
                used_capacity: 0.0,
            },
            season: "Spring".to_string(),
            day_night: DayNightSave {
                time_of_day: "Day".to_string(),
                day_count: 0,
                ticks_per_day: 250,
            },
            weather: WeatherSave {
                current: "Clear".to_string(),
                duration_remaining: 100,
            },
            terrain: TerrainSave {
                width: 0,
                height: 0,
                tiles: Vec::new(),
            },
            fertility: None,
            pops: vec![PopSave {
                old_bits: 4242,
                name: "Solo".to_string(),
                x: 3,
                y: 4,
                health: None,
                needs: None,
                morale_value: None,
                morale_modifiers: Vec::new(),
                wallet_credits: None,
                skills: Vec::new(),
                traits: Vec::new(),
                age: None,
                action: None,
                weights: None,
                memories: Vec::new(),
                job: None,
                relationships: Vec::new(),
                movement_target: None,
                worn_suit: None,
            }],
            buildings: vec![BuildingSave {
                old_bits: 4343,
                building_type: "Wall".to_string(),
                x: 1,
                y: 1,
                material: Some("Wood".to_string()),
                health: None,
                farm: None,
                housing: None,
                stockpile: None,
            }],
            items: vec![ItemSave {
                item_type: "Corpse".to_string(),
                x: 3,
                y: 4,
                item_ref: Some(4242),
                curio_name: None,
                suit: None,
            }],
            designations: vec![DesignationSave {
                designation_type: "Mine".to_string(),
                x: 5,
                y: 5,
            }],
        };
        let digest_before = snapshot_digest(&snap);
        let mut world = bare_world(0, 0.0);
        let report = apply_snapshot(&mut world, &snap).unwrap();
        assert_eq!(report.pops, 1);
        assert_eq!(report.buildings, 1);
        assert_eq!(report.items, 1);
        let after = capture_snapshot(&mut world, 42);
        assert_eq!(after.pops.len(), 1);
        assert!(after.pops[0].traits.is_empty(), "empty traits must stay empty, got {:?}", after.pops[0].traits);
        assert!(after.pops[0].health.is_none(), "absent Health must stay absent");
        assert!(after.pops[0].age.is_none(), "absent Age must stay absent");
        assert_eq!(
            snapshot_digest(&after),
            digest_before,
            "snapshot with empty/absent fields must round-trip exactly"
        );
    }

    #[test]
    fn test_suits_round_trip_through_save() {
        // Spec 1384 §3/§10: pool count, a worn Mk II at 63.0 air, and
        // a loose expedition suit at 141.0 air all survive
        // capture -> apply -> capture identically.
        use crate::layer1::eva_suit::{EvaSuit, SuitClass};
        let mut world = bare_world(0, 0.0);
        {
            let mut resources = world.resource_mut::<ColonyResources>();
            resources.eva_suits = 2.0;
            resources.max_eva_suits = 50.0;
        }
        let worn = world
            .spawn((
                Item::default(),
                EvaSuit {
                    class: SuitClass::Mk2,
                    air: 63.0,
                    durability: 100.0,
                    max_durability: 100.0,
                    colony_issued: true,
                },
                GridPosition { x: 1, y: 1 },
            ))
            .id();
        world.spawn((
            crate::layer1::pop::Pop,
            PopName("Suited".to_string()),
            GridPosition { x: 1, y: 1 },
            crate::layer1::items::Equipment {
                body: Some(worn),
                ..Default::default()
            },
        ));
        world.spawn((
            Item::default(),
            EvaSuit {
                class: SuitClass::Expedition,
                air: 141.0,
                durability: 100.0,
                max_durability: 100.0,
                colony_issued: false,
            },
            GridPosition { x: 4, y: 4 },
        ));
        let snap = capture_snapshot(&mut world, 42);
        assert_eq!(snap.resources.eva_suits, 2.0);
        let pop_save = snap.pops.iter().find(|p| p.name == "Suited").unwrap();
        let worn_save = pop_save.worn_suit.clone().expect("worn suit captured");
        assert_eq!(worn_save.class, "mk2");
        assert_eq!(worn_save.air, 63.0);
        let loose_save = snap
            .items
            .iter()
            .find_map(|i| i.suit.clone())
            .expect("loose suit captured as item");
        assert_eq!(loose_save.class, "expedition");
        assert_eq!(loose_save.air, 141.0);

        let mut world2 = bare_world(0, 0.0);
        apply_snapshot(&mut world2, &snap).unwrap();
        let after = capture_snapshot(&mut world2, 42);
        assert_eq!(after.resources.eva_suits, 2.0);
        let pop_after = after.pops.iter().find(|p| p.name == "Suited").unwrap();
        assert_eq!(pop_after.worn_suit, pop_save.worn_suit);
        let loose_after = after
            .items
            .iter()
            .find_map(|i| i.suit.clone())
            .expect("loose suit restored");
        assert_eq!(loose_after, loose_save);
    }
}
