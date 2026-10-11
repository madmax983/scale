//! Spec 330: Biome Aggression.
//!
//! Flora tiles have a Growth/Aggression rate and actively try to reclaim
//! "Civilized" tiles (buildings, roads). If maintenance is neglected, the
//! wilderness overtakes the colony.
//!
//! Design notes (adapted from the spec sketch — the mock types
//! `Building`/`Position`/`MaintenanceLevel` and `TerrainType::Forest` /
//! `Jungle` / `Road` do not exist in the real codebase):
//! - Encroachment sources are real [`Flora`] entities (the game's
//!   hostile-plant component). Maintenance level is the real
//!   [`Structure`] health fraction (`current_hp / max_hp`); the game's road
//!   analog is [`TerrainType::Path`].
//! - The spec's "encroachment rate by biome type (Jungle > Tundra)" maps onto
//!   [`FloraType`]: jungle-like constrictors
//!   ([`FloraType::StrangleVines`]) press hardest, pale slow-growing flora
//!   barely stir. See [`FloraType::encroachment_interval`].
//! - Throttling: each flora carries its own `encroach_timer` (40-200 ticks
//!   by type); the weeding directive sweeps every [`WEED_DIRECTIVE_INTERVAL`]
//!   ticks. No full-grid scans.
//! - Counterplay: [`biome_weeding_directive_system`] auto-issues
//!   [`DesignationType::ClearFlora`] work orders where flora presses against
//!   buildings (worked through the existing Work action); buildings damaged
//!   by overgrowth are picked up by the existing Repair action.
//! - IP: all flavor text is original and generic; see
//!   `test_ip_guard_no_banned_terms`.

use bevy::utils::{HashMap, HashSet};
use bevy_ecs::prelude::*;

use crate::layer1::agriculture::compost::ScrapHeap;
use crate::layer1::building::{Building, BuildingType, MaterialType};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::designation::{Designation, DesignationType};
use crate::layer1::flora::{Flora, FloraType};
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::ruins::{Ruin, RuinHistory};
use crate::layer1::structure::Structure;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::shared::time::SimulationTime;

/// 8-neighborhood scan order for encroachment attempts (deterministic).
pub const ENCROACH_NEIGHBORS: [(i32, i32); 8] = [
    (0, 1),
    (1, 0),
    (0, -1),
    (-1, 0),
    (1, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
];

/// Buildings at or above this maintenance fraction resist encroachment.
/// (Maintenance fraction = `Structure.current_hp / Structure.max_hp`.)
pub const ENCROACH_MAINTENANCE_THRESHOLD: f32 = 0.5;

/// Ticks between automatic weeding-directive sweeps.
pub const WEED_DIRECTIVE_INTERVAL: u64 = 50;

/// Chronicle tag for biome-aggression events.
pub const BIOME_ENCROACHMENT_TAG: &str = "[BIOME_ENCROACHMENT]";

/// Fired when the wilds claim a civilized tile (Spec 330).
#[derive(Event, Debug, Clone)]
pub struct BiomeEncroachmentEvent {
    /// Where the overgrowth won.
    pub position: GridPosition,
    /// Human-readable name of what fell ("Housing", "road", ...).
    pub target: String,
}

/// Heavily throttled encroachment: each flora's own `encroach_timer` gates
/// attempts, so at most a trickle of tiles is ever contested per tick.
/// Flora establishes on adjacent "civilized" tiles — buildings whose
/// maintenance has lapsed, or roads ([`TerrainType::Path`]). Buildings the
/// flora then grinds to 0 HP collapse into ruins (see destruction pass).
#[allow(clippy::too_many_arguments)]
pub fn biome_encroachment_system(
    mut commands: Commands,
    terrain: Res<TerrainGrid>,
    time: Res<SimulationTime>,
    mut flora: Query<(Entity, &mut Flora, &GridPosition)>,
    flora_tiles: Query<&GridPosition, With<Flora>>,
    buildings: Query<(Entity, &Building, &GridPosition, &Structure)>,
    mut encroach_events: EventWriter<BiomeEncroachmentEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    // Index flora-occupied tiles: the wilds never stack two flora on one tile.
    let mut occupied: HashSet<(i32, i32)> = HashSet::new();
    for pos in &flora_tiles {
        occupied.insert((pos.x, pos.y));
    }

    // Index buildings by tile with their maintenance fraction
    // (`current_hp / max_hp` — the real maintenance level).
    let mut building_at: HashMap<(i32, i32), (Entity, f32, BuildingType)> = HashMap::new();
    for (entity, building, pos, structure) in &buildings {
        let fraction = if structure.max_hp > 0.0 {
            (structure.current_hp / structure.max_hp).clamp(0.0, 1.0)
        } else {
            1.0
        };
        building_at
            .entry((pos.x, pos.y))
            .or_insert((entity, fraction, building.building_type));
    }

    // Encroachment sweep: each flora attempts on its own throttled timer, so
    // only a trickle of tiles is ever contested per tick.
    let mut new_flora: Vec<(GridPosition, FloraType)> = Vec::new();
    for (_, mut f, pos) in &mut flora {
        if f.encroach_timer > 0 {
            f.encroach_timer -= 1;
            continue;
        }
        f.encroach_timer = f.flora_type.encroachment_interval();
        for (dx, dy) in ENCROACH_NEIGHBORS {
            let (nx, ny) = (pos.x + dx, pos.y + dy);
            if nx < 0 || ny < 0 {
                continue;
            }
            if occupied.contains(&(nx, ny)) {
                continue;
            }
            #[allow(clippy::cast_sign_loss)]
            let (tx, ty) = (nx as usize, ny as usize);
            let civilized = match building_at.get(&(nx, ny)) {
                // Neglected buildings (maintenance lapsed) can be overgrown;
                // well-kept ones resist.
                Some((_, fraction, _)) => *fraction < ENCROACH_MAINTENANCE_THRESHOLD,
                // Roads are the game's other "civilized" tile.
                None => matches!(terrain.get(tx, ty), Some(TerrainType::Path)),
            };
            if civilized {
                new_flora.push((GridPosition { x: nx, y: ny }, f.flora_type));
                occupied.insert((nx, ny));
                break;
            }
        }
    }
    for (pos, flora_type) in new_flora {
        commands.spawn((
            Flora {
                flora_type,
                ..Default::default()
            },
            pos,
            Health {
                current: 20.0,
                max: 20.0,
                has_rust_lung: false,
            },
        ));
    }

    // Overgrowth destruction: a structure ground to 0 HP while flora holds
    // its tile is swallowed — despawn it, leave a ruin + scrap heap (the same
    // aftermath the fire-damage path leaves), and chronicle the fall.
    let mut fallen: Vec<(Entity, GridPosition, BuildingType)> = Vec::new();
    for (entity, building, pos, structure) in &buildings {
        if structure.current_hp <= 0.0 && occupied.contains(&(pos.x, pos.y)) {
            fallen.push((entity, *pos, building.building_type));
        }
    }
    for (entity, pos, building_type) in fallen {
        commands.entity(entity).despawn();
        commands.spawn((
            Ruin {
                original_type: building_type,
                material: MaterialType::default(),
            },
            RuinHistory {
                destruction_tick: time.tick,
                reason: "Overgrowth".to_string(),
            },
            pos,
        ));
        commands.spawn((ScrapHeap { metal: 8.0, wood: 8.0 }, pos));
        encroach_events.send(BiomeEncroachmentEvent {
            position: pos,
            target: format!("{building_type:?}"),
        });
        chronicle.send(AddChronicleEvent {
            text: format!(
                "{BIOME_ENCROACHMENT_TAG} The {building_type:?} at ({}, {}) has been swallowed by the wilds.",
                pos.x, pos.y
            ),
            importance: EventImportance::Standard,
        });
    }
}

/// Weeding/maintenance counterplay: every [`WEED_DIRECTIVE_INTERVAL`] ticks,
/// auto-issue [`DesignationType::ClearFlora`] work orders on flora tiles that
/// sit on (or directly adjacent to) a building — i.e. exactly where
/// encroachment is near. The existing Work action picks them up.
pub fn biome_weeding_directive_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    flora: Query<(&Flora, &GridPosition)>,
    building_tiles: Query<&GridPosition, With<Building>>,
    designations: Query<&GridPosition, With<Designation>>,
) {
    if time.tick == 0 || !time.tick.is_multiple_of(WEED_DIRECTIVE_INTERVAL) {
        return;
    }
    let designated: HashSet<(i32, i32)> =
        designations.iter().map(|p| (p.x, p.y)).collect();
    let buildings: HashSet<(i32, i32)> =
        building_tiles.iter().map(|p| (p.x, p.y)).collect();
    for (_, pos) in &flora {
        let tile = (pos.x, pos.y);
        if designated.contains(&tile) {
            continue;
        }
        let on_building = buildings.contains(&tile);
        let adjacent_to_building = !on_building
            && ENCROACH_NEIGHBORS
                .iter()
                .any(|(dx, dy)| buildings.contains(&(pos.x + dx, pos.y + dy)));
        if on_building || adjacent_to_building {
            commands.spawn((
                Designation {
                    designation_type: DesignationType::ClearFlora,
                },
                GridPosition { x: pos.x, y: pos.y },
            ));
        }
    }
}

/// Count flora entities currently pressing on civilization (on a building
/// tile or adjacent to one). Used by the headless `stats` readout.
pub fn encroaching_flora_count(world: &mut World) -> usize {
    let building_tiles: HashSet<(i32, i32)> = {
        let mut q = world.query_filtered::<&GridPosition, With<Building>>();
        q.iter(world).map(|p| (p.x, p.y)).collect()
    };
    let mut q = world.query::<(&Flora, &GridPosition)>();
    q.iter(world)
        .filter(|(_, pos)| {
            let tile = (pos.x, pos.y);
            building_tiles.contains(&tile)
                || ENCROACH_NEIGHBORS
                    .iter()
                    .any(|(dx, dy)| building_tiles.contains(&(pos.x + dx, pos.y + dy)))
        })
        .count()
}

/// Debug helper: plant aggressive flora at a tile (headless `overgrow`).
pub fn debug_spawn_aggressive_flora(world: &mut World, x: i32, y: i32) -> Entity {
    world
        .spawn((
            Flora {
                flora_type: FloraType::StrangleVines,
                encroach_timer: 0,
                ..Default::default()
            },
            GridPosition { x, y },
            Health {
                current: 20.0,
                max: 20.0,
                has_rust_lung: false,
            },
        ))
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::{App, Update};
    use bevy::MinimalPlugins;

    fn test_grid() -> TerrainGrid {
        TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        }
    }

    fn setup_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(test_grid());
        app.init_resource::<SimulationTime>();
        app.init_resource::<Events<BiomeEncroachmentEvent>>();
        app.init_resource::<Events<AddChronicleEvent>>();
        app.add_systems(
            Update,
            (
                biome_encroachment_system,
                crate::layer1::flora::flora_attack_system,
            )
                .chain(),
        );
        app
    }

    /// Mirrors the spec's `advance_time(&mut app, 1000.0)`: bump the clock
    /// tick-by-tick so timer-driven systems observe every tick.
    fn advance_time(app: &mut App, ticks: u64) {
        for _ in 0..ticks {
            app.world_mut().resource_mut::<SimulationTime>().tick += 1;
            app.update();
        }
    }

    fn spawn_flora(app: &mut App, x: i32, y: i32) -> Entity {
        app.world_mut()
            .spawn((
                Flora {
                    flora_type: FloraType::StrangleVines,
                    encroach_timer: 0,
                    ..Default::default()
                },
                GridPosition { x, y },
                Health {
                    current: 20.0,
                    max: 20.0,
                    has_rust_lung: false,
                },
            ))
            .id()
    }

    fn spawn_building(app: &mut App, x: i32, y: i32, hp: f32) -> Entity {
        app.world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x, y },
                Structure {
                    current_hp: hp,
                    max_hp: 100.0,
                },
            ))
            .id()
    }

    fn flora_at(app: &mut App, x: i32, y: i32) -> bool {
        let mut q = app.world_mut().query::<(&Flora, &GridPosition)>();
        q.iter(app.world()).any(|(_, p)| p.x == x && p.y == y)
    }

    fn building_alive(app: &mut App, entity: Entity) -> bool {
        app.world().get::<Building>(entity).is_some()
    }

    fn event_count<T: bevy_ecs::event::Event>(app: &App) -> usize {
        let events = app.world().resource::<Events<T>>();
        events.get_cursor().read(events).count()
    }

    fn chronicle_texts(app: &App) -> Vec<String> {
        let events = app.world().resource::<Events<AddChronicleEvent>>();
        events
            .get_cursor()
            .read(events)
            .map(|e| e.text.clone())
            .collect()
    }

    #[test]
    fn test_flora_encroaches_on_civilization() {
        let mut app = setup_test_app();

        // Adapted: the spec's `TerrainType::Road` does not exist; the game's
        // road analog is `TerrainType::Path`.
        app.world_mut()
            .resource_mut::<TerrainGrid>()
            .set(5, 6, TerrainType::Path);
        spawn_flora(&mut app, 5, 5);

        advance_time(&mut app, 1000);

        // Assert the wilds established on the road tile.
        assert!(
            flora_at(&mut app, 5, 6),
            "expected aggressive flora to encroach onto the Path tile at (5, 6)"
        );
    }

    #[test]
    fn test_maintenance_prevents_encroachment() {
        let mut app = setup_test_app();

        spawn_flora(&mut app, 5, 5);
        // Adapted: the spec's `MaintenanceLevel(100.0)` mock is the real
        // `Structure` health fraction — a fully maintained building.
        let building = spawn_building(&mut app, 5, 6, 100.0);

        advance_time(&mut app, 1000);

        // Assert the maintained road... building survived and was never overgrown.
        assert!(
            building_alive(&mut app, building),
            "maintained building should survive encroachment pressure"
        );
        assert!(
            !flora_at(&mut app, 5, 6),
            "flora must not establish on a well-maintained building tile"
        );
    }

    #[test]
    fn test_building_destroyed_by_overgrowth() {
        let mut app = setup_test_app();

        spawn_flora(&mut app, 5, 5);
        // Adapted: `MaintenanceLevel(0.0)` mock -> neglected `Structure`
        // (10/100 HP is below the encroachment threshold but not yet rubble).
        let building = spawn_building(&mut app, 5, 6, 10.0);

        advance_time(&mut app, 1000);

        // Assert the neglected building fell to the wilds.
        assert!(
            !building_alive(&mut app, building),
            "neglected building should be destroyed by overgrowth"
        );
        assert!(
            event_count::<BiomeEncroachmentEvent>(&app) >= 1,
            "expected a BiomeEncroachmentEvent when the building fell"
        );
        assert!(
            chronicle_texts(&app)
                .iter()
                .any(|t| t.contains(BIOME_ENCROACHMENT_TAG)),
            "expected a {BIOME_ENCROACHMENT_TAG} chronicle entry"
        );
    }

    #[test]
    fn test_encroachment_rate_varies_by_flora_type() {
        // Spec beat: encroachment rate by biome type (Jungle > Tundra).
        // Jungle-like constrictors press hardest; pale slow flora barely stir.
        assert!(
            FloraType::StrangleVines.encroachment_interval()
                < FloraType::FireWeed.encroachment_interval()
        );
        assert!(
            FloraType::FireWeed.encroachment_interval()
                < FloraType::XenoMoss.encroachment_interval()
        );
        assert!(
            FloraType::XenoMoss.encroachment_interval()
                < FloraType::SilentFlora.encroachment_interval()
        );
        assert!(
            FloraType::SilentFlora.encroachment_interval()
                <= FloraType::Ironwood.encroachment_interval()
        );
        assert!(
            FloraType::SilentFlora.encroachment_interval()
                <= FloraType::Lumiflora.encroachment_interval()
        );
    }

    #[test]
    fn test_weeding_directive_designates_threatened_flora() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(test_grid());
        app.init_resource::<SimulationTime>();
        app.add_systems(Update, biome_weeding_directive_system);

        // Flora ON a building tile: threatened.
        spawn_flora(&mut app, 5, 5);
        spawn_building(&mut app, 5, 5, 100.0);
        // Flora far from any building: not threatened.
        spawn_flora(&mut app, 8, 8);

        // The directive sweeps every WEED_DIRECTIVE_INTERVAL ticks.
        advance_time(&mut app, WEED_DIRECTIVE_INTERVAL + 10);

        let mut q = app
            .world_mut()
            .query::<(&Designation, &GridPosition)>();
        let designated: Vec<(i32, i32)> = q
            .iter(app.world())
            .filter(|(d, _)| d.designation_type == DesignationType::ClearFlora)
            .map(|(_, p)| (p.x, p.y))
            .collect();
        assert!(
            designated.contains(&(5, 5)),
            "expected a ClearFlora designation where flora presses on a building, got {designated:?}"
        );
        assert!(
            !designated.contains(&(8, 8)),
            "flora far from buildings must not be auto-designated, got {designated:?}"
        );
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // All flavor is concept-only: no named characters, places, or
        // distinctive external IP anywhere in public strings.
        let banned = [
            "Mordor",
            "Fangorn",
            "Mirkwood",
            "Lothlorien",
            "Pandora",
            "Na'vi",
            "Endor",
            "Kashyyyk",
            "Dagobah",
            "Arrakis",
            "Narnia",
            "Westeros",
            "Neverland",
        ];
        let public_strings = [
            BIOME_ENCROACHMENT_TAG.to_string(),
            format!(
                "{BIOME_ENCROACHMENT_TAG} The {:?} at (5, 6) has been swallowed by the wilds.",
                BuildingType::Housing
            ),
            "Overgrowth".to_string(),
        ];
        for s in &public_strings {
            for term in banned {
                assert!(
                    !s.contains(term),
                    "banned term '{term}' found in public string: {s}"
                );
            }
        }
    }
}
