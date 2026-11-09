//! Shipbreaking System (Spec 409) — "Shipbreaking".
//!
//! Fantasy: scavenging the bones of giants; living in the wreckage.
//! A crashed ship spawns as a multi-tile ring of indestructible **Hull**
//! terrain. Mining a hull tile yields refined alloys and high-tech
//! components, but only for a miner whose tools are up to the plate.
//! The un-mined ring is shelter: buildings can be placed *inside* it,
//! and room-quality flood fill treats hull as an enclosing wall.
//! Tension: rapid wealth (break the hull down for scrap) vs.
//! defense/shelter (keep the ring intact).
//!
//! Grounding (audit of trunk 5a8ed49d, following the specs 360-408
//! pattern — the spec's sketch types are mocks until proven real):
//! - `HullTile` / `Mineable` / `Miner` / `MineEvent` /
//!   `SpawnCrashedShipEvent` / `YieldsOnMine` already existed in this
//!   module, and the three systems were already scheduled
//!   (`systems/execution.rs`) with events initialized in `setup.rs` —
//!   but the implementation was a hollow mock: nothing in production
//!   ever spawned a `Miner` or sent a `MineEvent` at a hull tile, the
//!   spawn ignored `TerrainGrid`/`OccupiedTiles` entirely (hull tiles
//!   were walk-through and build-on-able), yields were `Scrap` only,
//!   there was no ship type, no chronicle, no headless surface, and
//!   room calculation did not know hull existed.
//! - Real terrain is `TerrainGrid` + `TerrainType`; adding a `Hull`
//!   variant would churn ~886 `TerrainType` uses across the tree, so
//!   hull stays entity-based (the `ImpactSite`/`MotherLode` pattern)
//!   and bridges onto the real seams instead: `OccupiedTiles` (the
//!   placement/pathing occupancy ledger), `can_designate` (the Mine
//!   designation gate), and `room_quality::is_boundary_enclosed`.
//! - Real tools have NO tier: `Tool { tool_type, durability }` behind
//!   `Equipment.tool` (spec 058/392). The spec's `required_tool_tier: 3`
//!   is therefore bridged, not invented as a parallel economy: a pop's
//!   effective tier is derived from real state — see
//!   [`effective_tool_tier`]. The `Miner` component remains as the
//!   schedule-native seam the spec's RED tests (and the three other
//!   `MineEvent` consumers: ancient ruins, megafauna terrain,
//!   biosphere inversion) already use, and it takes precedence when
//!   present.
//! - Real yields: there is no `RefinedAlloy` / `HyperAlloy`
//!   `ResourceType`. The refined-alloy analogue is `ResourceType::Metal`
//!   ("refined metal from ore") and the high-tech-component analogue is
//!   `ResourceType::AncientTech` (spec 370's deliberately untradeable
//!   pristine-tech pattern). Drops are real `ResourceItem` ground
//!   drops, the same seam the Mother Lode uses.
//! - Real placement: `try_place_building` rejects tiles in
//!   `OccupiedTiles` with "Location occupied", so registering hull
//!   tiles there makes building ON hull impossible while building
//!   INSIDE the ring (interior tiles are never registered) works.
//! - "Indestructible" means: no damage path except gated mining.
//!   Hull `Health` is only ever reduced by [`mine_system`]; explosion /
//!   demolish / destroy designations do not target bare entities.

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::spatial::OccupiedTiles;
use crate::layer1::health::Health;
use crate::layer1::items::{Equipment, Item, ItemType, Tool};
use crate::layer1::map::GridPosition;
use crate::layer1::resources::{ResourceItem, ResourceType};
use crate::layer1::skills::{SkillType, Skills};

/// Tool tier a hull tile demands. Hull plate is starship-grade: a
/// fresh colonist with a standard Smithy pickaxe (tier 1) cannot dent
/// it — see [`effective_tool_tier`] for how a pop reaches tier 3.
pub const HULL_REQUIRED_TOOL_TIER: u8 = 3;

/// Health of one hull tile. With [`HULL_MINE_DAMAGE`] per successful
/// mine event, a tile takes ten qualified hits to break.
pub const HULL_TILE_HEALTH: f32 = 100.0;

/// Damage one qualified [`MineEvent`] deals to a hull tile.
pub const HULL_MINE_DAMAGE: f32 = 10.0;

/// Refined-alloy (Metal) amount dropped when a hull tile breaks.
pub const HULL_METAL_YIELD: f32 = 5.0;

/// High-tech-component (AncientTech) amount dropped when a hull
/// tile breaks.
pub const HULL_TECH_YIELD: f32 = 1.0;

/// Chronicle line written when a crashed ship comes down.
pub const CRASH_CHRONICLE_TEXT: &str =
    "A crashed ship has come to rest nearby — its hull could shelter the colony, or feed it.";

/// The class of crashed ship. Class names are generic naval terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShipType {
    /// A mid-size warship: a 3x3 hull ring (8 tiles) around one
    /// sheltered interior tile.
    #[default]
    Cruiser,
    /// A bulk hauler: a 5x4 hull perimeter (14 tiles) around a
    /// 3x2 sheltered hold.
    Freighter,
}

impl ShipType {
    /// Player-facing class name (generic terms only).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cruiser => "Cruiser",
            Self::Freighter => "Freighter",
        }
    }

    /// Hull-tile offsets (the perimeter ring) for this class,
    /// relative to the spawn location (the ring's minimum corner).
    #[must_use]
    pub fn ring_offsets(self) -> Vec<(i32, i32)> {
        let (w, h) = match self {
            Self::Cruiser => (3, 3),
            Self::Freighter => (5, 4),
        };
        let mut offsets = Vec::new();
        for dy in 0..h {
            for dx in 0..w {
                if dx == 0 || dy == 0 || dx == w - 1 || dy == h - 1 {
                    offsets.push((dx, dy));
                }
            }
        }
        offsets
    }
}

// --- Components ---

/// Represents a piece of a crashed ship hull. It's indestructible by normal means.
#[derive(Component)]
pub struct HullTile;

/// Indicates this entity can be mined with a specific tool tier.
#[derive(Component)]
pub struct Mineable {
    pub required_tool_tier: u8,
}

/// What resources this entity drops when destroyed by mining.
#[derive(Component)]
pub struct YieldsOnMine(pub Vec<ResourceType>);

/// A miner capable of mining.
#[derive(Component)]
pub struct Miner {
    pub tool_tier: u8,
}

// --- Events ---

/// Event to spawn a crashed ship at a location.
#[derive(Event, Debug, Clone)]
pub struct SpawnCrashedShipEvent {
    pub location: crate::layer1::map::GridPosition,
    pub ship_type: ShipType,
}

impl SpawnCrashedShipEvent {
    /// Spawn event for the default (Cruiser) class.
    #[must_use]
    pub const fn new(location: crate::layer1::map::GridPosition) -> Self {
        Self {
            location,
            ship_type: ShipType::Cruiser,
        }
    }
}

/// Event representing a mining action.
#[derive(Event, Debug, Clone)]
pub struct MineEvent {
    pub miner: Entity,
    pub target: Entity,
}

// --- Tool-tier bridge ---

/// A pop's effective tool tier, derived from REAL tool/skill state.
///
/// The real economy has no tool tiers (`Tool` carries only a type and
/// durability), so the tier is a derived measure of "can this pop work
/// starship plate":
/// - No `Miner` seam and no usable equipped tool: tier 0.
/// - An equipped, unbroken pickaxe (the Smithy's generic mining tool,
///   per `tool_economy::tool_satisfies` a Pickaxe satisfies mining):
///   tier 1, plus the pop's Mining skill level — a veteran of the
///   rock face (Mining level 2+, i.e. 400+ Mining XP) with a pickaxe
///   reaches the tier-3 hull requirement.
/// - A broken tool (durability <= 0.0) counts as no tool.
///
/// The explicit [`Miner`] component, when present, is authoritative
/// (it is the seam the spec's tests and the other `MineEvent`
/// consumers drive); this derivation is how real pops qualify.
#[must_use]
pub fn effective_tool_tier(
    miner: Option<&Miner>,
    equipment: Option<&Equipment>,
    skills: Option<&Skills>,
    tools: &Query<&Tool>,
) -> u8 {
    if let Some(miner) = miner {
        return miner.tool_tier;
    }
    let Some(tool_entity) = equipment.and_then(|e| e.tool) else {
        return 0;
    };
    let Ok(tool) = tools.get(tool_entity) else {
        return 0;
    };
    if tool.durability <= 0.0 || tool.tool_type != crate::layer1::items::ToolType::Pickaxe {
        return 0;
    }
    let level = skills.map_or(0, |s| s.get_level(SkillType::Mining));
    #[allow(clippy::cast_possible_truncation)]
    let tier = 1 + level.min(u32::from(u8::MAX - 1));
    tier as u8
}

/// World-based form of [`effective_tool_tier`] for World-driven
/// callers (headless, designation execution).
#[must_use]
pub fn effective_tool_tier_world(world: &World, pop: Entity) -> u8 {
    if let Some(miner) = world.get::<Miner>(pop) {
        return miner.tool_tier;
    }
    let Some(tool_entity) = world.get::<Equipment>(pop).and_then(|e| e.tool) else {
        return 0;
    };
    let Some(tool) = world.get::<Tool>(tool_entity) else {
        return 0;
    };
    if tool.durability <= 0.0 || tool.tool_type != crate::layer1::items::ToolType::Pickaxe {
        return 0;
    }
    let level = world
        .get::<Skills>(pop)
        .map_or(0, |s| s.get_level(SkillType::Mining));
    #[allow(clippy::cast_possible_truncation)]
    let tier = 1 + level.min(u32::from(u8::MAX - 1));
    tier as u8
}

/// Amount dropped for a yielded resource type.
#[must_use]
pub const fn yield_amount(resource: ResourceType) -> f32 {
    match resource {
        ResourceType::Metal => HULL_METAL_YIELD,
        ResourceType::AncientTech => HULL_TECH_YIELD,
        _ => 5.0,
    }
}

/// Default yields for a freshly crashed hull tile: refined alloys
/// plus a high-tech component.
#[must_use]
pub fn default_hull_yields() -> Vec<ResourceType> {
    vec![ResourceType::Metal, ResourceType::AncientTech]
}

// --- Systems ---

pub fn spawn_crashed_ship_system(
    mut events: EventReader<SpawnCrashedShipEvent>,
    mut commands: Commands,
    mut occupied: Option<ResMut<OccupiedTiles>>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    for event in events.read() {
        let base_x = event.location.x;
        let base_y = event.location.y;
        for (dx, dy) in event.ship_type.ring_offsets() {
            let x = base_x + dx;
            let y = base_y + dy;
            if let Some(occ) = occupied.as_deref_mut() {
                occ.0.insert((x, y));
            }
            commands.spawn((
                HullTile,
                Mineable {
                    required_tool_tier: HULL_REQUIRED_TOOL_TIER,
                },
                Health {
                    current: HULL_TILE_HEALTH,
                    max: HULL_TILE_HEALTH,
                    has_rust_lung: false,
                },
                YieldsOnMine(default_hull_yields()),
                GridPosition { x, y },
            ));
        }
        if let Some(writer) = chronicle.as_deref_mut() {
            writer.send(AddChronicleEvent {
                text: format!(
                    "{} — a {} has come down at ({}, {}).",
                    CRASH_CHRONICLE_TEXT,
                    event.ship_type.name(),
                    base_x,
                    base_y
                ),
                importance: EventImportance::Major,
            });
        }
    }
}

pub fn mine_system(
    mut events: EventReader<MineEvent>,
    mut target_query: Query<(&Mineable, &mut Health)>,
    miner_query: Query<&Miner>,
    equipment_query: Query<&Equipment>,
    skills_query: Query<&Skills>,
    tools: Query<&Tool>,
) {
    for event in events.read() {
        let tier = effective_tool_tier(
            miner_query.get(event.miner).ok(),
            equipment_query.get(event.miner).ok(),
            skills_query.get(event.miner).ok(),
            &tools,
        );
        if let Ok((mineable, mut health)) = target_query.get_mut(event.target) {
            if tier >= mineable.required_tool_tier {
                health.take_damage(HULL_MINE_DAMAGE);
            }
        }
    }
}

pub fn hull_destroyed_system(
    mut commands: Commands,
    query: Query<(Entity, &Health, &YieldsOnMine, &GridPosition), With<HullTile>>,
    mut occupied: Option<ResMut<OccupiedTiles>>,
) {
    for (entity, health, yields, pos) in query.iter() {
        if health.current <= 0.0 {
            for res_type in &yields.0 {
                commands.spawn((
                    ResourceItem {
                        resource_type: *res_type,
                        amount: yield_amount(*res_type),
                    },
                    Item {
                        item_type: ItemType::None,
                    },
                    *pos,
                ));
            }
            if let Some(occ) = occupied.as_deref_mut() {
                occ.0.remove(&(pos.x, pos.y));
            }
            commands.entity(entity).despawn();
        }
    }
}

// --- Census (headless STATS / panel) ---

/// Live census of hull tiles for headless STATS and panels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HullCensus {
    /// Hull tiles still standing.
    pub tiles: usize,
    /// Sum of remaining hull health across all tiles.
    pub health: i64,
}

/// Count hull tiles in a raw World (headless / tests).
#[must_use]
pub fn hull_census(world: &mut World) -> HullCensus {
    let mut q = world.query_filtered::<&Health, With<HullTile>>();
    let mut tiles = 0usize;
    let mut health = 0.0f32;
    for h in q.iter(world) {
        tiles += 1;
        health += h.current.max(0.0);
    }
    HullCensus {
        tiles,
        #[allow(clippy::cast_possible_truncation)]
        health: health.round() as i64,
    }
}

/// Compact STATS label: `tiles/health`, e.g. `8/800`.
#[must_use]
pub fn hull_stats_label(world: &mut World) -> String {
    let c = hull_census(world);
    format!("{}/{}", c.tiles, c.health)
}

/// Whether a tile is covered by a standing hull tile (World form,
/// for room-quality enclosure and other World-driven callers).
#[must_use]
pub fn is_hull_tile(world: &World, x: i32, y: i32) -> bool {
    world.iter_entities().any(|e| {
        e.contains::<HullTile>()
            && e.get::<GridPosition>()
                .is_some_and(|p| p.x == x && p.y == y)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::BuildingMap;
    use crate::layer1::building::OccupiedTiles;
    use crate::layer1::building::{try_place_building, BuildingType};
    use crate::layer1::terrain::{TerrainGrid, TerrainType};

    fn hull_world() -> World {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 100,
            height: 100,
            tiles: vec![TerrainType::Grass; 10000],
        });
        world.insert_resource(OccupiedTiles::default());
        world.init_resource::<Events<SpawnCrashedShipEvent>>();
        world.init_resource::<Events<MineEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    fn run_spawn(world: &mut World, x: i32, y: i32, ship: ShipType) {
        world.send_event(SpawnCrashedShipEvent {
            location: GridPosition { x, y },
            ship_type: ship,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(spawn_crashed_ship_system);
        schedule.run(world);
    }

    // Spec RED test 1, adapted: SpawnCrashedShipEvent mock location
    // IVec2 -> real GridPosition; ShipType::Cruiser is real here.
    #[test]
    fn test_shipbreaking_hull_generation() {
        let mut world = hull_world();
        run_spawn(&mut world, 10, 10, ShipType::Cruiser);
        let hull_tiles: Vec<_> = world.query::<&HullTile>().iter(&world).collect();
        assert_eq!(hull_tiles.len(), 8, "Cruiser ring should spawn 8 Hull tiles");
        // Hull tiles occupy their tiles (placement/pathing ledger).
        let occ = world.resource::<OccupiedTiles>();
        assert!(occ.0.contains(&(10, 10)));
        assert!(occ.0.contains(&(12, 12)));
        assert!(!occ.0.contains(&(11, 11)), "interior must stay buildable");
    }

    #[test]
    fn test_freighter_ring_is_bigger() {
        let mut world = hull_world();
        run_spawn(&mut world, 10, 10, ShipType::Freighter);
        let n = world.query::<&HullTile>().iter(&world).count();
        assert_eq!(n, 14, "Freighter 5x4 perimeter should spawn 14 tiles");
    }

    // Spec RED test 2, verbatim intent: low-tier miner cannot damage hull.
    #[test]
    fn test_mining_hull_requires_tool() {
        let mut world = World::new();
        world.init_resource::<Events<MineEvent>>();
        let hull_entity = world
            .spawn((
                HullTile,
                Mineable { required_tool_tier: 3 },
                Health { current: 100.0, max: 100.0, has_rust_lung: false },
            ))
            .id();
        let miner_entity = world.spawn(Miner { tool_tier: 1 }).id();
        world.send_event(MineEvent { miner: miner_entity, target: hull_entity });
        let mut schedule = Schedule::default();
        schedule.add_systems(mine_system);
        schedule.run(&mut world);
        let health = world.get::<Health>(hull_entity).unwrap();
        assert_eq!(health.current, 100.0, "Miner with low tier tool should not damage Hull tile");
    }

    #[test]
    fn test_real_pop_tier_derivation() {
        // No tool -> 0; pickaxe -> 1; pickaxe + Mining lvl 2 -> 3 (qualifies).
        let mut world = World::new();
        world.init_resource::<Events<MineEvent>>();
        let hull = world
            .spawn((
                HullTile,
                Mineable { required_tool_tier: 3 },
                Health { current: 100.0, max: 100.0, has_rust_lung: false },
            ))
            .id();
        // Bare pop: no damage.
        let bare = world.spawn(Equipment::default()).id();
        // Pickaxe novice: tier 1, no damage.
        let pick = world
            .spawn(crate::layer1::items::Tool {
                tool_type: crate::layer1::items::ToolType::Pickaxe,
                durability: 100.0,
                max_durability: 100.0,
            })
            .id();
        let novice = world
            .spawn(Equipment { tool: Some(pick), ..Default::default() })
            .id();
        // Veteran: pickaxe + 400 Mining XP (level 2) -> tier 3, damages.
        let pick2 = world
            .spawn(crate::layer1::items::Tool {
                tool_type: crate::layer1::items::ToolType::Pickaxe,
                durability: 100.0,
                max_durability: 100.0,
            })
            .id();
        let mut skills = Skills::default();
        skills.xp.insert(SkillType::Mining, 400.0);
        let veteran = world
            .spawn((Equipment { tool: Some(pick2), ..Default::default() }, skills))
            .id();
        for miner in [bare, novice, veteran] {
            world.send_event(MineEvent { miner, target: hull });
        }
        let mut schedule = Schedule::default();
        schedule.add_systems(mine_system);
        schedule.run(&mut world);
        let health = world.get::<Health>(hull).unwrap();
        assert_eq!(health.current, 90.0, "only the veteran (tier 3) should damage hull");
    }

    // Spec RED test 3, adapted: YieldsOnMine mock Resource::RefinedAlloy
    // -> real ResourceType::Metal + AncientTech ground drops.
    #[test]
    fn test_mining_hull_yields_alloys() {
        let mut world = World::new();
        world.init_resource::<Events<MineEvent>>();
        world.insert_resource(OccupiedTiles::default());
        let hull_entity = world
            .spawn((
                HullTile,
                Mineable { required_tool_tier: 3 },
                Health { current: 10.0, max: 100.0, has_rust_lung: false },
                YieldsOnMine(vec![ResourceType::Metal, ResourceType::AncientTech]),
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        world.resource_mut::<OccupiedTiles>().0.insert((5, 5));
        let miner_entity = world.spawn(Miner { tool_tier: 3 }).id();
        world.send_event(MineEvent { miner: miner_entity, target: hull_entity });
        let mut schedule = Schedule::default();
        schedule.add_systems((mine_system, hull_destroyed_system).chain());
        schedule.run(&mut world);
        let drops: Vec<_> = world
            .query::<&ResourceItem>()
            .iter(&world)
            .map(|r| r.resource_type)
            .collect();
        assert!(drops.contains(&ResourceType::Metal), "Mining Hull should yield refined alloys (Metal)");
        assert!(drops.contains(&ResourceType::AncientTech), "Mining Hull should yield high-tech components (AncientTech)");
        assert!(world.get::<HullTile>(hull_entity).is_none(), "broken hull tile despawns");
        assert!(!world.resource::<OccupiedTiles>().0.contains(&(5, 5)), "mined-out tile frees its occupancy");
    }

    // Spec RED test 4, adapted: try_place_building mock (World, type,
    // Vec3) -> real (world, x, y, BuildingType) -> bool.
    #[test]
    fn test_building_inside_hull() {
        let mut world = hull_world();
        world.insert_resource(BuildingMap::default());
        world.insert_resource(crate::layer1::resources::ColonyResources {
            wood: 100.0,
            ..Default::default()
        });
        run_spawn(&mut world, 10, 10, ShipType::Cruiser);
        // Inside the ring (the sheltered interior tile) succeeds.
        let inside = try_place_building(&mut world, 11, 11, BuildingType::Housing);
        assert!(inside, "Should be able to place buildings inside hull boundaries");
    }

    #[test]
    fn test_building_on_hull_is_blocked() {
        let mut world = hull_world();
        world.insert_resource(BuildingMap::default());
        world.insert_resource(crate::layer1::resources::ColonyResources {
            wood: 100.0,
            ..Default::default()
        });
        run_spawn(&mut world, 10, 10, ShipType::Cruiser);
        let on_hull = try_place_building(&mut world, 10, 10, BuildingType::Housing);
        assert!(!on_hull, "Cannot build ON a hull tile");
    }

    #[test]
    fn test_hull_census_and_ip_guard() {
        let mut world = hull_world();
        run_spawn(&mut world, 10, 10, ShipType::Cruiser);
        let c = hull_census(&mut world);
        assert_eq!(c.tiles, 8);
        assert_eq!(c.health, 800);
        assert_eq!(hull_stats_label(&mut world), "8/800");
        // IP guard: player-facing strings use generic terms only.
        for s in [ShipType::Cruiser.name(), ShipType::Freighter.name(), CRASH_CHRONICLE_TEXT] {
            for banned in ["Star Wars", "Star Trek", "Battlestar", "Serenity", "Firefly", "Warhammer", "Halo"] {
                assert!(!s.contains(banned), "banned IP term {banned} in {s}");
            }
        }
    }
}
