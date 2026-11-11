//! The Black Box (Spec 414) — cross-run knowledge preservation.
//!
//! Fantasy: "This civilization may fall, but our knowledge will remain."
//! A buildable Archive structure saves the colony's Tech/Lore state. If
//! the colony fails (Game Over), a future colony on the same world can
//! discover and decrypt a RuinedArchive to regain lost progress.
//!
//! Grounding (audited against the real seams, specs 360-413 pattern):
//! - Tech: the spec's `unlock_tech("fusion_power")` / `is_unlocked(str)` /
//!   `unlocked: Vec<String>` API is a sketch. The real API is
//!   `TechState { techs: HashMap<Tech, TechStatus>, total_capacity,
//!   used_capacity }` with `unlock(Tech)` / `is_unlocked(Tech)` over the
//!   closed `Tech` enum (there is no fusion tech). The Black Box keeps
//!   the spec's portable `Vec<String>` shape and translates through a
//!   stable name codec ([`tech_black_box_name`] /
//!   [`tech_from_black_box_name`]); unknown names are skipped on decrypt
//!   so boxes written by other versions never crash a run.
//! - Capacity trap (the deorbit/tether lesson): `TechState::unlock`
//!   re-runs corruption, so restoring onto a zero-capacity TechState
//!   would instantly re-corrupt everything. A decrypted Archive is
//!   itself a data store (its building carries `DataStorage`), so the
//!   decrypt system first raises `total_capacity` to fit the recovered
//!   set, then unlocks.
//! - Game Over: there is NO pre-existing colony-failure detection or
//!   `GameOverEvent` anywhere in the codebase (audited: no matches).
//!   [`GameOverEvent`] is defined here per spec §7, and
//!   [`colony_collapse_detection_system`] is the detector: a colony that
//!   once had living pops and now has none has fallen. It fires once.
//! - Chronicle: the real lore ledger is `layer1::chronicle::Chronicle`
//!   (events: tick/year/text/importance). The box captures the event
//!   texts as its lore payload; saves/decrypts announce themselves
//!   through the real `AddChronicleEvent` bus.
//! - Buildings: `BuildingType::Archive` joins the real building enum
//!   (cost/label/glyph/wind), and `spawn_building` attaches the
//!   [`Archive`] + [`UploadProgress`] components plus `DataStorage`.
//! - Persistence (spec §5): [`BlackBoxData`] is a plain serde resource —
//!   unit tests stay resource-only (mocked boundary). The actual file
//!   I/O lives in separate platform/startup seams:
//!   [`write_black_box_to_path`] / [`read_black_box_from_path`] (pure
//!   path functions), [`flush_black_box_to_disk_system`] (native-only
//!   schedule system that writes when the save counter moves), and the
//!   disk seed in `simulation::init_simulation_resources` (loads a
//!   previous run's box at world init, native only).
//!
//! Follow-ons (spec §5, deliberately NOT in this pass):
//! - Partial recovery: decrypt currently restores full unlocks; a
//!   corruption model (e.g. 50% research progress instead of unlock)
//!   needs a research-progress representation TechState does not have.
//! - Physical upload job: upload accrues automatically while an Archive
//!   stands ([`archive_upload_system`]); a pop-carried work job at the
//!   Archive (utility-AI integration) is the gameplay follow-on.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use strum::IntoEnumIterator;

use crate::layer1::architecture::building::Building;
use crate::layer1::chronicle::{AddChronicleEvent, Chronicle, EventImportance};
use crate::layer1::entities::pop::Pop;
use crate::layer1::map::GridPosition;
use crate::layer1::tech::{Tech, TechState};
use crate::shared::time::SimulationTime;

/// Ticks of standing upload work before an Archive's data is secured.
pub const UPLOAD_TICKS: u32 = 50;
/// Max chronicle texts a black box carries (oldest dropped first).
pub const MAX_BLACK_BOX_LORE: usize = 100;

/// Marker + state for a standing Archive building: whether its copy of
/// the colony's knowledge has finished uploading.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Archive {
    /// True once the upload completed — only uploaded Archives save
    /// anything on Game Over (spec RED test 1/2 seam).
    pub data_uploaded: bool,
}

/// Upload progress for an Archive that has not finished yet. Kept as a
/// separate component so `Archive { data_uploaded: true }` stays a
/// one-field literal, exactly as the spec's RED tests construct it.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UploadProgress {
    /// Ticks of upload work accrued so far (target: [`UPLOAD_TICKS`]).
    pub progress_ticks: u32,
}

/// Marker for the ruin of an Archive from a fallen colony. Decrypting
/// it (see [`DecryptArchiveEvent`]) restores the black box's tech.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct RuinedArchive;

/// Marker left on a ruin whose black box has been decrypted and drained.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct DecryptedArchive;

/// The saved legacy itself: portable across runs and processes.
/// Stored as tech *names* (not the `Tech` enum) so the on-disk format
/// survives enum changes; unknown names are skipped on decrypt.
#[derive(Resource, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlackBoxData {
    /// Black-box names of every researched tech at save time.
    pub unlocked_techs: Vec<String>,
    /// Chronicle event texts captured as the colony's lore.
    pub chronicle_texts: Vec<String>,
    /// Simulation tick of the colony that saved this box.
    pub saved_at_tick: u64,
    /// How many Game Over saves have written this box (lineage depth).
    pub saves: u32,
}

/// Per-run ledger for the black-box lifecycle (not persisted).
#[derive(Resource, Debug, Clone, Default)]
pub struct BlackBoxState {
    /// A living pop has been observed this run (arms collapse detection).
    pub ever_had_pops: bool,
    /// Game Over has already fired this run (fire-once latch).
    pub game_over_fired: bool,
    /// A RuinedArchive has been placed for discovery this run.
    pub ruin_spawned: bool,
}

/// Native disk-sync ledger: which save count has been flushed to disk.
#[derive(Resource, Debug, Clone, Default)]
pub struct BlackBoxDiskSync {
    /// The `BlackBoxData::saves` value last written to disk.
    pub synced_saves: u32,
}

/// The colony has fallen. Defined here per spec §7 — no pre-existing
/// colony-failure event exists in the codebase to hook into.
#[derive(Event, Debug, Clone, Copy)]
pub struct GameOverEvent;

/// A future colony interacts with a RuinedArchive to recover its box.
#[derive(Event, Debug, Clone, Copy)]
pub struct DecryptArchiveEvent {
    /// The ruin entity being decrypted.
    pub entity: Entity,
}

/// Stable black-box name for a tech (the on-disk / cross-run format).
#[must_use]
pub const fn tech_black_box_name(tech: Tech) -> &'static str {
    match tech {
        Tech::Masonry => "masonry",
        Tech::MetalWorking => "metal_working",
        Tech::SocialStructures => "social_structures",
        Tech::Astronomy => "astronomy",
        Tech::Hydroponics => "hydroponics",
        Tech::Militia => "militia",
        Tech::Medical => "medical",
        Tech::Electromagnetism => "electromagnetism",
        Tech::VoidWhispers => "void_whispers",
        Tech::Terraforming => "terraforming",
        Tech::ClosedCycleBreathing => "closed_cycle_breathing",
        Tech::ExpeditionSystems => "expedition_systems",
    }
}

/// Resolves a black-box name back to a tech. Unknown names (written by
/// other versions, or fictional in a spec sketch) return `None` and are
/// skipped by the decrypt system — never a panic, never a fake unlock.
#[must_use]
pub fn tech_from_black_box_name(name: &str) -> Option<Tech> {
    Tech::iter().find(|t| tech_black_box_name(*t) == name)
}

/// Pure capture: snapshot researched techs + chronicle lore into a box.
/// Kept system-free so the save policy is unit-testable without an App.
#[must_use]
pub fn capture_black_box(tech_state: &TechState, chronicle: &Chronicle, tick: u64) -> BlackBoxData {
    let mut names: Vec<String> = Tech::iter()
        .filter(|t| tech_state.is_researched(*t))
        .map(|t| tech_black_box_name(t).to_string())
        .collect();
    names.sort();
    names.dedup();
    let skip = chronicle.events.len().saturating_sub(MAX_BLACK_BOX_LORE);
    let texts = chronicle
        .events
        .iter()
        .skip(skip)
        .map(|e| e.text.clone())
        .collect();
    BlackBoxData {
        unlocked_techs: names,
        chronicle_texts: texts,
        saved_at_tick: tick,
        saves: 0,
    }
}

/// Upload accrual: a standing Archive steadily secures the colony's
/// knowledge; once [`UPLOAD_TICKS`] of work accrues, its data counts as
/// uploaded and will be saved if the colony falls. (The pop-carried
/// physical upload job is the spec §5 follow-on.)
pub fn archive_upload_system(
    mut archives: Query<(&mut Archive, &mut UploadProgress)>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for (mut archive, mut progress) in archives.iter_mut() {
        if archive.data_uploaded {
            continue;
        }
        progress.progress_ticks = progress.progress_ticks.saturating_add(1);
        if progress.progress_ticks >= UPLOAD_TICKS {
            archive.data_uploaded = true;
            chronicle_events.send(AddChronicleEvent {
                text: "The Archive finished uploading the colony's knowledge into its black box."
                    .to_string(),
                importance: EventImportance::Standard,
            });
        }
    }
}

/// Colony-failure detection (the Game Over hook the spec asked us to
/// find or build — none existed): once a colony that had living pops
/// has none left, it has fallen. Fires [`GameOverEvent`] exactly once.
pub fn colony_collapse_detection_system(
    pops: Query<(), With<Pop>>,
    mut state: ResMut<BlackBoxState>,
    mut events: EventWriter<GameOverEvent>,
) {
    if pops.iter().next().is_some() {
        state.ever_had_pops = true;
        return;
    }
    if state.ever_had_pops && !state.game_over_fired {
        state.game_over_fired = true;
        events.send(GameOverEvent);
    }
}

/// On Game Over: if at least one Archive finished its upload, seal the
/// colony's tech + lore into the BlackBoxData resource. Without an
/// uploaded Archive, nothing is saved (spec RED test 2).
pub fn save_black_box_system(
    mut events: EventReader<GameOverEvent>,
    archives: Query<&Archive>,
    tech_state: Res<TechState>,
    chronicle: Res<Chronicle>,
    time: Res<SimulationTime>,
    mut black_box: ResMut<BlackBoxData>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in events.read() {
        let can_save = archives.iter().any(|a| a.data_uploaded);
        if !can_save {
            continue;
        }
        let mut captured = capture_black_box(&tech_state, &chronicle, time.tick);
        captured.saves = black_box.saves + 1;
        let count = captured.unlocked_techs.len();
        *black_box = captured;
        chronicle_events.send(AddChronicleEvent {
            text: format!(
                "The colony has fallen. Its black box was sealed with {count} technologies for whoever comes next."
            ),
            importance: EventImportance::Major,
        });
    }
}

/// On decrypt: restore every recoverable tech from the box into the
/// current run's TechState, then drain the ruin (RuinedArchive removed,
/// DecryptedArchive marker left). The Archive's own data store covers
/// the recovered set, so capacity is raised first — otherwise
/// `TechState::unlock`'s corruption pass would instantly re-corrupt
/// the recovery on a low-capacity colony.
pub fn load_black_box_system(
    mut events: EventReader<DecryptArchiveEvent>,
    ruins: Query<&RuinedArchive>,
    mut commands: Commands,
    black_box: Res<BlackBoxData>,
    mut tech_state: ResMut<TechState>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        if ruins.get(event.entity).is_err() {
            continue;
        }
        let mut recovered: Vec<Tech> = Vec::new();
        for name in &black_box.unlocked_techs {
            if let Some(tech) = tech_from_black_box_name(name) {
                if !tech_state.is_researched(tech) {
                    recovered.push(tech);
                }
            }
        }
        if !recovered.is_empty() {
            let needed: f32 = recovered.iter().map(|t| t.storage_cost()).sum();
            let floor = tech_state.used_capacity + needed;
            if tech_state.total_capacity < floor {
                tech_state.total_capacity = floor;
            }
            for tech in &recovered {
                tech_state.unlock(*tech);
            }
            let labels: Vec<&str> = recovered.iter().map(|t| t.label()).collect();
            chronicle_events.send(AddChronicleEvent {
                text: format!(
                    "A black box from a fallen colony was decrypted: {} technologies recovered ({}).",
                    recovered.len(),
                    labels.join(", ")
                ),
                importance: EventImportance::Major,
            });
        } else {
            chronicle_events.send(AddChronicleEvent {
                text: "A ruined Archive was opened, but its black box held nothing this colony did not already know.".to_string(),
                importance: EventImportance::Standard,
            });
        }
        commands
            .entity(event.entity)
            .remove::<RuinedArchive>()
            .insert(DecryptedArchive);
    }
}

/// Discovery: a run that inherited a non-empty black box (seeded from
/// disk at world init) finds the fallen colony's RuinedArchive near
/// its own buildings, once, ready to decrypt.
pub fn spawn_ruined_archive_system(
    black_box: Res<BlackBoxData>,
    mut state: ResMut<BlackBoxState>,
    ruins: Query<(), With<RuinedArchive>>,
    buildings: Query<&GridPosition, With<Building>>,
    time: Res<SimulationTime>,
    mut commands: Commands,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    if state.ruin_spawned || black_box.unlocked_techs.is_empty() || time.tick < 1 {
        return;
    }
    if ruins.iter().next().is_some() {
        state.ruin_spawned = true;
        return;
    }
    let Some(heart) = buildings.iter().next().copied() else {
        return;
    };
    state.ruin_spawned = true;
    commands.spawn((
        RuinedArchive,
        GridPosition {
            x: heart.x + 3,
            y: heart.y,
        },
    ));
    chronicle_events.send(AddChronicleEvent {
        text: "Scouts found a ruined Archive from a fallen colony nearby — its black box may still hold their knowledge.".to_string(),
        importance: EventImportance::Major,
    });
}

/// Where the black box lives on disk (native). `SCALE_BLACK_BOX_PATH`
/// overrides it (tests, tooling, sandboxed runs).
#[must_use]
pub fn black_box_file_path() -> PathBuf {
    std::env::var_os("SCALE_BLACK_BOX_PATH")
        .map_or_else(|| PathBuf::from("black_box.json"), PathBuf::from)
}

/// Pure file write (the spec §5 disk-I/O seam): serialize the box as
/// JSON to an explicit path. Errors surface as strings, never panics.
///
/// # Errors
/// Returns `Err` with a message on serialization or I/O failure.
pub fn write_black_box_to_path(data: &BlackBoxData, path: &std::path::Path) -> Result<(), String> {
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// Pure file read: a missing or corrupt file yields `None` (a fresh
/// lineage), never a panic and never garbage state.
#[must_use]
pub fn read_black_box_from_path(path: &std::path::Path) -> Option<BlackBoxData> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Native platform system: flush the box to disk whenever a new save
/// lands. Registered only on non-wasm targets; unit tests never touch
/// it (they exercise the pure path functions above instead).
pub fn flush_black_box_to_disk_system(
    black_box: Res<BlackBoxData>,
    mut sync: ResMut<BlackBoxDiskSync>,
) {
    if black_box.saves > sync.synced_saves {
        if write_black_box_to_path(&black_box, &black_box_file_path()).is_ok() {
            sync.synced_saves = black_box.saves;
        }
    }
}

/// Live census for STATS `blackbox=` (full panel: headless `blackbox`):
/// `{archives}a/{saved_techs}s/{ruins}r` — archives standing, techs in
/// the box, ruins awaiting decrypt. No spaces (STATS is space-parsed).
#[must_use]
pub fn black_box_stats_label(world: &mut World) -> String {
    let archives = world.query::<&Archive>().iter(world).count();
    let ruins = world.query::<&RuinedArchive>().iter(world).count();
    let saved = world
        .get_resource::<BlackBoxData>()
        .map_or(0, |b| b.unlocked_techs.len());
    format!("{archives}a/{saved}s/{ruins}r")
}

/// Player-facing strings for this mechanic (IP-guard tested): original,
/// generic wording only.
#[must_use]
pub fn black_box_public_strings() -> Vec<&'static str> {
    vec![
        "Archive",
        "The Black Box",
        "black box",
        "Ruined Archive",
        "The colony has fallen. Its black box was sealed for whoever comes next.",
        "A black box from a fallen colony was decrypted.",
        "Scouts found a ruined Archive from a fallen colony nearby.",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::architecture::building::BuildingType;
    use bevy_app::{App, Update};
    use bevy_ecs::schedule::IntoSystemConfigs;

    fn setup_app() -> App {
        // Spec RED setup, adapted to real types: bevy_app App, real
        // TechState/Chronicle resources, events registered explicitly.
        let mut app = App::new();
        app.add_event::<GameOverEvent>();
        app.add_event::<DecryptArchiveEvent>();
        app.add_event::<AddChronicleEvent>();
        app.init_resource::<TechState>();
        app.init_resource::<Chronicle>();
        app.init_resource::<BlackBoxData>();
        app.init_resource::<BlackBoxState>();
        app.init_resource::<SimulationTime>();
        app
    }

    fn unlock(app: &mut App, tech: Tech) {
        let mut state = app.world_mut().resource_mut::<TechState>();
        state.total_capacity = state.total_capacity.max(200.0);
        state.unlock(tech);
    }

    fn send_game_over(app: &mut App) {
        app.world_mut()
            .resource_mut::<Events<GameOverEvent>>()
            .send(GameOverEvent);
    }

    #[test]
    fn test_black_box_preserves_tech_on_game_over() {
        // Spec RED test 1, adapted: "fusion_power" is not a real tech —
        // Astronomy stands in (same codec path, real enum variant).
        let mut app = setup_app();
        app.add_systems(Update, save_black_box_system);
        unlock(&mut app, Tech::Astronomy);
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::Archive,
            },
            Archive {
                data_uploaded: true,
            },
        ));
        send_game_over(&mut app);
        app.update();
        let black_box = app.world().resource::<BlackBoxData>();
        assert!(black_box.unlocked_techs.contains(&"astronomy".to_string()));
        assert_eq!(black_box.saves, 1);
    }

    #[test]
    fn test_black_box_requires_archive_building() {
        // Spec RED test 2, verbatim intent: no Archive, nothing saved.
        let mut app = setup_app();
        app.add_systems(Update, save_black_box_system);
        unlock(&mut app, Tech::Astronomy);
        send_game_over(&mut app);
        app.update();
        let black_box = app.world().resource::<BlackBoxData>();
        assert!(!black_box.unlocked_techs.contains(&"astronomy".to_string()));
        assert_eq!(black_box.saves, 0);
    }

    #[test]
    fn test_archive_without_upload_saves_nothing() {
        // An Archive that never finished uploading is just a building.
        let mut app = setup_app();
        app.add_systems(Update, save_black_box_system);
        unlock(&mut app, Tech::Astronomy);
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::Archive,
            },
            Archive {
                data_uploaded: false,
            },
        ));
        send_game_over(&mut app);
        app.update();
        assert!(app
            .world()
            .resource::<BlackBoxData>()
            .unlocked_techs
            .is_empty());
    }

    #[test]
    fn test_new_run_can_discover_black_box() {
        // Spec RED test 3, adapted to the real TechState API.
        let mut app = setup_app();
        app.add_systems(Update, load_black_box_system);
        app.world_mut()
            .resource_mut::<BlackBoxData>()
            .unlocked_techs
            .push("astronomy".to_string());
        let ruin_id = app.world_mut().spawn(RuinedArchive).id();
        app.world_mut()
            .resource_mut::<Events<DecryptArchiveEvent>>()
            .send(DecryptArchiveEvent { entity: ruin_id });
        app.update();
        let tech_state = app.world().resource::<TechState>();
        assert!(tech_state.is_unlocked(Tech::Astronomy));
        assert!(app.world().get::<RuinedArchive>(ruin_id).is_none());
        assert!(app.world().get::<DecryptedArchive>(ruin_id).is_some());
    }

    #[test]
    fn test_decrypt_grants_capacity_so_recovery_stays_active() {
        // The corruption trap: a fresh TechState has zero capacity, and
        // unlock() alone would instantly corrupt the recovered tech.
        let mut app = setup_app();
        app.add_systems(Update, load_black_box_system);
        assert_eq!(app.world().resource::<TechState>().total_capacity, 0.0);
        app.world_mut()
            .resource_mut::<BlackBoxData>()
            .unlocked_techs
            .push("terraforming".to_string());
        let ruin_id = app.world_mut().spawn(RuinedArchive).id();
        app.world_mut()
            .resource_mut::<Events<DecryptArchiveEvent>>()
            .send(DecryptArchiveEvent { entity: ruin_id });
        app.update();
        let tech_state = app.world().resource::<TechState>();
        assert!(tech_state.is_active(Tech::Terraforming));
        assert!(tech_state.total_capacity >= Tech::Terraforming.storage_cost());
    }

    #[test]
    fn test_decrypt_skips_unknown_tech_names() {
        // Forward/backward compatibility: names with no real Tech
        // (including the spec sketch's fictional "fusion_power") are
        // skipped, known names still restore, nothing panics.
        let mut app = setup_app();
        app.add_systems(Update, load_black_box_system);
        {
            let mut black_box = app.world_mut().resource_mut::<BlackBoxData>();
            black_box.unlocked_techs.push("fusion_power".to_string());
            black_box.unlocked_techs.push("masonry".to_string());
        }
        let ruin_id = app.world_mut().spawn(RuinedArchive).id();
        app.world_mut()
            .resource_mut::<Events<DecryptArchiveEvent>>()
            .send(DecryptArchiveEvent { entity: ruin_id });
        app.update();
        assert!(app.world().resource::<TechState>().is_unlocked(Tech::Masonry));
        assert!(app.world().get::<RuinedArchive>(ruin_id).is_none());
    }

    #[test]
    fn test_tech_name_codec_roundtrips_all_techs() {
        for tech in Tech::iter() {
            assert_eq!(tech_from_black_box_name(tech_black_box_name(tech)), Some(tech));
        }
        assert_eq!(tech_from_black_box_name("fusion_power"), None);
        assert_eq!(tech_from_black_box_name(""), None);
    }

    #[test]
    fn test_save_captures_chronicle_lore() {
        let mut app = setup_app();
        app.add_systems(Update, save_black_box_system);
        unlock(&mut app, Tech::Masonry);
        app.world_mut()
            .resource_mut::<Chronicle>()
            .add_event(7, "The colony was founded.".to_string(), EventImportance::Legendary);
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::Archive,
            },
            Archive {
                data_uploaded: true,
            },
        ));
        send_game_over(&mut app);
        app.update();
        let black_box = app.world().resource::<BlackBoxData>();
        assert!(black_box
            .chronicle_texts
            .contains(&"The colony was founded.".to_string()));
    }

    #[test]
    fn test_upload_progress_completes_upload() {
        let mut app = setup_app();
        app.add_systems(Update, archive_upload_system);
        let archive_id = app
            .world_mut()
            .spawn((
                Archive {
                    data_uploaded: false,
                },
                UploadProgress {
                    progress_ticks: UPLOAD_TICKS - 1,
                },
            ))
            .id();
        app.update();
        assert!(app.world().get::<Archive>(archive_id).unwrap().data_uploaded);
    }

    #[test]
    fn test_collapse_detection_fires_game_over_once_and_saves() {
        // End-to-end hook: pops vanish -> detection fires -> the same
        // chained update seals the box. A second update must not
        // re-fire or double-count the save.
        let mut app = setup_app();
        app.add_systems(
            Update,
            (colony_collapse_detection_system, save_black_box_system).chain(),
        );
        unlock(&mut app, Tech::Masonry);
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::Archive,
            },
            Archive {
                data_uploaded: true,
            },
        ));
        let pop = app.world_mut().spawn(Pop).id();
        app.update();
        assert_eq!(app.world().resource::<BlackBoxData>().saves, 0);
        app.world_mut().despawn(pop);
        app.update();
        assert!(app.world().resource::<BlackBoxState>().game_over_fired);
        assert_eq!(app.world().resource::<BlackBoxData>().saves, 1);
        app.update();
        assert_eq!(app.world().resource::<BlackBoxData>().saves, 1);
    }

    #[test]
    fn test_collapse_detection_needs_a_living_colony_first() {
        // A world that never had pops (menus, fixtures) must not be
        // declared fallen.
        let mut app = setup_app();
        app.add_systems(Update, colony_collapse_detection_system);
        app.update();
        app.update();
        assert!(!app.world().resource::<BlackBoxState>().game_over_fired);
    }

    #[test]
    fn test_ruin_spawned_for_inherited_black_box() {
        let mut app = setup_app();
        app.add_systems(Update, spawn_ruined_archive_system);
        app.world_mut()
            .resource_mut::<BlackBoxData>()
            .unlocked_techs
            .push("masonry".to_string());
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::Housing,
            },
            GridPosition { x: 10, y: 10 },
        ));
        app.update();
        let mut q = app.world_mut().query::<&GridPosition>();
        let positions: Vec<GridPosition> = q
            .iter(app.world())
            .copied()
            .filter(|p| p.x == 13 && p.y == 10)
            .collect();
        // Building at (10,10) + ruin at (13,10): the ruin query above
        // counts both GridPositions; assert the ruin exists by component.
        let ruins = app.world_mut().query::<&RuinedArchive>().iter(app.world()).count();
        assert_eq!(ruins, 1);
        assert_eq!(positions.len(), 1);
        assert!(app.world().resource::<BlackBoxState>().ruin_spawned);
    }

    #[test]
    fn test_black_box_data_serde_roundtrip() {
        let data = BlackBoxData {
            unlocked_techs: vec!["astronomy".to_string(), "masonry".to_string()],
            chronicle_texts: vec!["The colony was founded.".to_string()],
            saved_at_tick: 4242,
            saves: 2,
        };
        let json = serde_json::to_string(&data).unwrap();
        let back: BlackBoxData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, back);
    }

    #[test]
    fn test_black_box_file_roundtrip() {
        // The §5 disk seam, exercised against an explicit temp path —
        // unit tests never touch the real black_box.json.
        let path = std::env::temp_dir().join(format!(
            "scale-black-box-test-{}.json",
            std::process::id()
        ));
        let data = BlackBoxData {
            unlocked_techs: vec!["medical".to_string()],
            chronicle_texts: Vec::new(),
            saved_at_tick: 99,
            saves: 1,
        };
        write_black_box_to_path(&data, &path).unwrap();
        assert_eq!(read_black_box_from_path(&path), Some(data));
        let _ = std::fs::remove_file(&path);
        assert_eq!(read_black_box_from_path(&path), None);
    }

    #[test]
    fn test_ip_guard_public_strings() {
        // IP rule: concept-only. Player-facing black-box strings and the
        // building label must carry no named characters, places, or
        // distinctive borrowed IP.
        let banned = [
            "Sad King Billy",
            "Windsor",
            "Foundation",
            "Dune",
            "Warhammer",
            "Star Wars",
            "Star Trek",
            "Hitchhiker",
            "Ender",
            "Annihilation",
            "Shimmer",
        ];
        let mut strings = black_box_public_strings();
        strings.push(BuildingType::Archive.label());
        strings.push(tech_black_box_name(Tech::Astronomy));
        for text in strings {
            for term in banned {
                assert!(
                    !text.contains(term),
                    "public string {text:?} contains banned IP term {term:?}"
                );
            }
        }
    }
}
