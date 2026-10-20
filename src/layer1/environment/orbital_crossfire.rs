//! Spec 356: Orbital Crossfire — live wiring (Item 109).
//!
//! Warring fleets in orbit miss shots and drop debris; the fallout lands
//! on Layer 1. Spec 206 built the impact lane ([`OrbitalEvent`] component
//! + [`impact_system`] + mineable [`ImpactSite`] scrap), but nothing ever
//! spawned an `OrbitalEvent` outside tests — crossfire was dead in live
//! play. This module now carries the live wiring:
//!
//! * [`crossfire_director_system`] — an organic director that spawns
//!   strikes at a low tuned rate. The seam is the real orbital-debris
//!   ledger ([`OrbitalDebris`][crate::layer2::debris::OrbitalDebris]):
//!   a clean sky is nearly safe; a debris-choked sky (Kessler territory,
//!   fed by shoot-downs, deorbit impacts, launches) rains crossfire.
//!   With no debris the base chance still applies — stray shots from
//!   distant fleet actions — but it is deliberately rare.
//! * [`impact_system`] feedback — every resolved strike increments
//!   [`CrossfireStats`], writes a Major chronicle entry, and logs to the
//!   message log, so the player can see the hazard happened and where.
//! * Crater fix — impacts now deform terrain to
//!   [`TerrainType::Crater`][crate::layer1::terrain::TerrainType::Crater]
//!   (the meteor lane's terrain), not the `Rock` placeholder. Verified
//!   safe: `Rock` and `Crater` are both unwalkable with identical heat
//!   retention (0.5) and movement cost, so no pathfinding or thermal
//!   consumer changes behavior.
//!
//! Spec adaptations (§8 pattern): the sketch's `OrbitalStrikeEvent`,
//! `Map`/`TileType`, `ResourceNode`, and `ResourceType::HighTechScrap`
//! do not exist. The real seams are the pre-existing `OrbitalEvent`
//! component, `TerrainGrid`, and `ImpactSite` -> `ResourceType::Scrap`
//! via [`mine_scrap`]; no duplicate strike system and no new resource
//! type were created. Spec 1386 fleet orders are out of scope.

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::resources::{MiningProgress, ResourceItem, ResourceType};
use crate::layer1::terrain::TerrainGrid;
use crate::layer2::debris::OrbitalDebris;
use crate::shared::log::MessageLog;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Ticks between organic crossfire rolls.
pub const CROSSFIRE_CHECK_INTERVAL_TICKS: u64 = 200;
/// Strike chance per check in a clean sky (stray shots only).
pub const CROSSFIRE_BASE_CHANCE: f64 = 0.06;
/// Extra chance per unit of orbital debris density.
pub const CROSSFIRE_DEBRIS_CHANCE_PER_UNIT: f64 = 0.20;
/// Hard cap on strike chance per check, however dirty the sky gets.
pub const CROSSFIRE_MAX_CHANCE: f64 = 0.35;
/// Damage of an organic strike (a standing 100 HP building is destroyed;
/// reinforced structures survive, damaged).
pub const CROSSFIRE_DAMAGE: f32 = 150.0;
/// Heat dumped into the struck tile by an organic strike.
pub const CROSSFIRE_HEAT: f32 = 250.0;
/// Chebyshev scatter around the colony heart for organic strike targets.
pub const CROSSFIRE_SCATTER_RADIUS: i32 = 12;
/// Map-center fallback when there are no pops to average.
pub const CROSSFIRE_MAP_CENTER: GridPosition = GridPosition { x: 40, y: 25 };

/// Event representing an orbital strike or meteor impact.
#[derive(Component, Debug, Clone)]
pub struct OrbitalEvent {
    /// Target grid position.
    pub target: GridPosition,
    /// Damage dealt to structures.
    pub damage: f32,
    /// Heat added to the tile.
    pub heat: f32,
}

/// Component marking a location with mineable scrap from an impact.
#[derive(Component, Debug, Clone)]
pub struct ImpactSite {
    /// Amount of scrap available.
    pub scrap_amount: f32,
    /// Difficulty modifier for mining.
    pub harvest_difficulty: f32,
}

/// Tuning knobs for the organic crossfire director. Tests override these
/// to force deterministic outcomes.
#[derive(Resource, Debug, Clone)]
pub struct CrossfireDirector {
    /// Master switch for organic strikes (debug strikes always work).
    pub enabled: bool,
    /// Ticks between strike rolls.
    pub check_interval_ticks: u64,
    /// Strike chance per check in a clean sky.
    pub base_chance: f64,
    /// Extra chance per unit of orbital debris density.
    pub debris_chance_per_unit: f64,
    /// Hard cap on strike chance per check.
    pub max_chance: f64,
    /// Damage dealt by an organic strike.
    pub damage: f32,
    /// Heat dumped into the struck tile by an organic strike.
    pub heat: f32,
    /// Chebyshev scatter around the colony heart for strike targets.
    pub scatter_radius: i32,
}

impl Default for CrossfireDirector {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_ticks: CROSSFIRE_CHECK_INTERVAL_TICKS,
            base_chance: CROSSFIRE_BASE_CHANCE,
            debris_chance_per_unit: CROSSFIRE_DEBRIS_CHANCE_PER_UNIT,
            max_chance: CROSSFIRE_MAX_CHANCE,
            damage: CROSSFIRE_DAMAGE,
            heat: CROSSFIRE_HEAT,
            scatter_radius: CROSSFIRE_SCATTER_RADIUS,
        }
    }
}

/// Running ledger of resolved crossfire strikes (STATS / panels read it).
#[derive(Resource, Debug, Clone, Default)]
pub struct CrossfireStats {
    /// Total strikes resolved since colony founding.
    pub total_strikes: u32,
    /// Buildings destroyed by crossfire across all strikes.
    pub buildings_destroyed: u32,
    /// Target of the most recent strike, if any.
    pub last_target: Option<GridPosition>,
    /// Tick of the most recent strike.
    pub last_tick: u64,
}

/// Strike chance for one director check at the given debris density.
#[must_use]
pub fn crossfire_chance(director: &CrossfireDirector, debris: f32) -> f64 {
    (director.base_chance + f64::from(debris.max(0.0)) * director.debris_chance_per_unit)
        .clamp(0.0, director.max_chance)
}

/// Mean position of living pops: the colony heart. Falls back to
/// [`CROSSFIRE_MAP_CENTER`] when there are no pops.
fn colony_heart(world: &mut World) -> GridPosition {
    let mut query = world.query_filtered::<&GridPosition, With<Pop>>();
    let positions: Vec<GridPosition> = query.iter(world).copied().collect();
    if positions.is_empty() {
        return CROSSFIRE_MAP_CENTER;
    }
    let (sx, sy) = positions
        .iter()
        .fold((0i64, 0i64), |(x, y), p| (x + p.x as i64, y + p.y as i64));
    #[allow(clippy::cast_possible_truncation)]
    GridPosition {
        x: (sx / positions.len() as i64) as i32,
        y: (sy / positions.len() as i64) as i32,
    }
}

/// Spawn a crossfire strike at `target` using the director's damage/heat
/// (or the module defaults when no director is installed). The strike
/// resolves through [`impact_system`] on the live schedule. Used by the
/// organic director and the headless `crossfire` debug command alike.
pub fn trigger_crossfire(world: &mut World, target: GridPosition) {
    let (damage, heat) = world
        .get_resource::<CrossfireDirector>()
        .map_or((CROSSFIRE_DAMAGE, CROSSFIRE_HEAT), |d| (d.damage, d.heat));
    world.spawn(OrbitalEvent {
        target,
        damage,
        heat,
    });
}

/// System: the organic crossfire director. Every
/// `check_interval_ticks`, roll against the debris-scaled chance; on a
/// hit, a strike falls near the colony heart (fleet actions are aimed at
/// the colony's sky, misses scatter around it). Exclusive `World` system
/// in the deorbit/sabotage director style.
pub fn crossfire_director_system(world: &mut World) {
    let Some(director) = world.get_resource::<CrossfireDirector>().cloned() else {
        return;
    };
    if !director.enabled || director.check_interval_ticks == 0 {
        return;
    }
    let tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);
    if tick == 0 || tick % director.check_interval_ticks != 0 {
        return;
    }
    let debris = world.get_resource::<OrbitalDebris>().map_or(0.0, |d| d.0);
    let chance = crossfire_chance(&director, debris);
    let mut rng = rand::thread_rng();
    {
        use rand::Rng;
        if !rng.gen_bool(chance) {
            return;
        }
    }
    let heart = colony_heart(world);
    let (tx, ty) = {
        use rand::Rng;
        let dx = rng.gen_range(-director.scatter_radius..=director.scatter_radius);
        let dy = rng.gen_range(-director.scatter_radius..=director.scatter_radius);
        (heart.x + dx, heart.y + dy)
    };
    // Clamp into the terrain grid when one is installed.
    let target = if let Some(grid) = world.get_resource::<TerrainGrid>() {
        GridPosition {
            x: tx.clamp(0, grid.width.saturating_sub(1) as i32),
            y: ty.clamp(0, grid.height.saturating_sub(1) as i32),
        }
    } else {
        GridPosition {
            x: tx.max(0),
            y: ty.max(0),
        }
    };
    trigger_crossfire(world, target);
}

/// Number of unharvested impact sites standing in the colony.
pub fn impact_site_count(world: &mut World) -> usize {
    let mut query = world.query::<&ImpactSite>();
    query.iter(world).count()
}

/// Short status for headless STATS: total resolved strikes.
#[must_use]
pub fn crossfire_stats_label(world: &World) -> String {
    world
        .get_resource::<CrossfireStats>()
        .map_or_else(|| "0".to_string(), |s| s.total_strikes.to_string())
}

/// Mines scrap from an impact site.
pub fn mine_scrap(world: &mut World, designation_entity: Entity, work_amount: f32) {
    // 1. Get position and verify ImpactSite
    let (pos, site_entity) = {
        let pos = if let Some(p) = world.get::<GridPosition>(designation_entity) {
            *p
        } else {
            return;
        };

        // Find ImpactSite at pos
        let mut target = None;
        let mut query = world.query::<(Entity, &ImpactSite, &GridPosition)>();
        for (e, _, p) in query.iter(world) {
            if *p == pos {
                target = Some(e);
                break;
            }
        }
        (pos, target)
    };

    let Some(site_entity) = site_entity else {
        return;
    };

    // 2. Update Progress (on designation)
    // Ensure MiningProgress exists
    if world.get::<MiningProgress>(designation_entity).is_none() {
        world.entity_mut(designation_entity).insert(MiningProgress {
            current: 0.0,
            max: 20.0,
        }); // 20 work for scrap
    }

    let completed = if let Some(mut progress) = world.get_mut::<MiningProgress>(designation_entity)
    {
        progress.current += work_amount;
        progress.current >= progress.max
    } else {
        false
    };

    // 3. Completion
    if completed {
        // Get scrap yield
        let yield_amount = world
            .get::<ImpactSite>(site_entity)
            .map_or(1.0, |s| s.scrap_amount);

        // Despawn ImpactSite
        world.despawn(site_entity);

        // Spawn Scrap Item
        world.spawn((
            ResourceItem {
                resource_type: ResourceType::Scrap,
                amount: yield_amount,
            },
            pos,
        ));

        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add("Salvaged Scrap from Impact Site.");
        }

        // Remove designation
        world.despawn(designation_entity);
    }
}

/// System that processes `OrbitalEvent` components, damaging buildings and creating `ImpactSite`s.
pub fn impact_system(world: &mut World) {
    // ⚡ Bolt Optimization: Pre-allocated vector capacity based on query size to eliminate dynamic heap reallocation.
    let mut query = world.query::<(Entity, &OrbitalEvent)>();
    let mut events = Vec::with_capacity(query.iter(world).len());
    for (entity, event) in query.iter(world) {
        events.push((entity, event.target, event.damage, event.heat));
    }

    for (event_entity, target, damage, heat) in events {
        // 1. Destroy/Damage Buildings
        let mut destroyed = Vec::new();
        {
            let mut buildings = world.query::<(
                Entity,
                &GridPosition,
                &mut crate::layer1::structure::Structure,
            )>();
            for (b_entity, pos, mut structure) in buildings.iter_mut(world) {
                if *pos == target {
                    structure.current_hp -= damage;
                    if structure.current_hp <= 0.0 {
                        destroyed.push(b_entity);
                    }
                }
            }
        }
        let destroyed_count = destroyed.len() as u32;
        for e in destroyed {
            world.despawn(e);
        }

        // 2. Deform Terrain — a real crater (spec 356), not the Rock
        // placeholder. Rock and Crater are both unwalkable with the same
        // heat retention, so this changes no pathfinding/thermal behavior.
        if let Some(mut grid) = world.get_resource_mut::<crate::layer1::terrain::TerrainGrid>() {
            // Need to handle bounds check or assume grid.set does it (it does)
            // But target.x might be negative (GridPosition is i32).
            if target.x >= 0 && target.y >= 0 {
                grid.set(
                    target.x as usize,
                    target.y as usize,
                    crate::layer1::terrain::TerrainType::Crater,
                );
            }
        }

        // 3. Add Heat
        if let Some(mut temp) =
            world.get_resource_mut::<crate::layer1::temperature::TemperatureGrid>()
        {
            temp.add(target.x, target.y, heat);
        }

        // 4. Spawn Scrap (ImpactSite)
        world.spawn((
            ImpactSite {
                scrap_amount: 50.0,
                harvest_difficulty: 1.0,
            },
            target,
        ));

        // 5. Feedback: stats ledger, message log, and a Major chronicle
        // entry so the player can see the hazard happened (and where).
        // All guarded: isolated unit-test worlds may not install these.
        let tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);
        if let Some(mut stats) = world.get_resource_mut::<CrossfireStats>() {
            stats.total_strikes += 1;
            stats.buildings_destroyed += destroyed_count;
            stats.last_target = Some(target);
            stats.last_tick = tick;
        }
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add(format!(
                "ORBITAL CROSSFIRE: a stray shot from the fighting above has struck ({}, {})! \
                 {destroyed_count} building(s) destroyed; salvage crews can mine the wreckage for scrap.",
                target.x, target.y
            ));
        }
        if let Some(mut events) =
            world.get_resource_mut::<bevy_ecs::event::Events<AddChronicleEvent>>()
        {
            events.send(AddChronicleEvent {
                text: format!(
                    "ORBITAL CROSSFIRE: the fleets fighting overhead missed — a stray shot \
                     cratered the ground at ({}, {}), destroying {destroyed_count} building(s). \
                     The wreckage smoulders, rich with salvageable scrap.",
                    target.x, target.y
                ),
                importance: EventImportance::Major,
            });
        }

        // 6. Cleanup event
        world.despawn(event_entity);
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::environment::orbital_crossfire::{impact_system, ImpactSite, OrbitalEvent};
    use crate::layer1::map::GridPosition;
    use crate::layer1::structure::Structure;
    use crate::layer1::temperature::TemperatureGrid;
    use crate::layer1::terrain::{TerrainGrid, TerrainType};
    use bevy_ecs::prelude::*;

    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });
        world.insert_resource(TemperatureGrid::new(10, 10, 20.0));
        world
    }

    #[test]
    fn test_impact_destroys_building() {
        let mut world = setup_world();

        // Spawn a building at (5,5)
        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 5, y: 5 },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
            ))
            .id();

        // Trigger Impact Event at (5,5)
        world.spawn(OrbitalEvent {
            target: GridPosition { x: 5, y: 5 },
            damage: 500.0,
            heat: 1000.0,
        });

        // Run system
        impact_system(&mut world);

        // Building should be gone
        assert!(world.get_entity(building).is_err());
    }

    #[test]
    fn test_impact_creates_crater() {
        let mut world = setup_world();
        let mut grid = world.resource_mut::<TerrainGrid>();
        grid.set(5, 5, TerrainType::Grass);

        // Trigger Impact
        world.spawn(OrbitalEvent {
            target: GridPosition { x: 5, y: 5 },
            damage: 100.0,
            heat: 100.0,
        });

        impact_system(&mut world);

        let grid = world.resource::<TerrainGrid>();
        // Spec 356: converted to a real Crater (Rock was a placeholder).
        assert_eq!(grid.get(5, 5).unwrap(), TerrainType::Crater);
    }

    #[test]
    fn test_impact_generates_heat() {
        let mut world = setup_world();

        world.spawn(OrbitalEvent {
            target: GridPosition { x: 5, y: 5 },
            damage: 100.0,
            heat: 500.0,
        });

        impact_system(&mut world);

        let temp = world.resource::<TemperatureGrid>();
        assert!(temp.get(5, 5) >= 500.0);
    }

    #[test]
    fn test_impact_spawns_harvestable_scrap() {
        let mut world = setup_world();

        world.spawn(OrbitalEvent {
            target: GridPosition { x: 5, y: 5 },
            damage: 100.0,
            heat: 100.0,
        });

        impact_system(&mut world);

        // Check for ImpactSite entity with Scrap
        let (_, site, pos) = world
            .query::<(Entity, &ImpactSite, &GridPosition)>()
            .single(&world);

        assert_eq!(pos.x, 5);
        assert_eq!(pos.y, 5);
        assert!(site.scrap_amount > 0.0);
    }

    #[test]
    fn test_mine_scrap() {
        let mut world = setup_world();
        world.insert_resource(crate::shared::log::MessageLog::default());

        // Spawn Impact Site
        world.spawn((
            ImpactSite {
                scrap_amount: 20.0,
                harvest_difficulty: 1.0,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Spawn Designation
        let designation = world
            .spawn((
                crate::layer1::designation::Designation {
                    designation_type: crate::layer1::designation::DesignationType::Mine,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        // Mine
        super::mine_scrap(&mut world, designation, 20.0);

        // ImpactSite should be gone
        let mut site_query = world.query::<&ImpactSite>();
        assert_eq!(site_query.iter(&world).count(), 0);

        // Scrap Item should be spawned
        let mut item_query = world.query::<&crate::layer1::resources::ResourceItem>();
        let item = item_query.iter(&world).next().unwrap();
        assert_eq!(
            item.resource_type,
            crate::layer1::resources::ResourceType::Scrap
        );

        // Designation should be gone
        assert!(world.get_entity(designation).is_err());
    }
}

#[cfg(test)]
mod live_wiring_tests {
    //! Spec 356 live-wiring tests (Item 109). These exercise the REAL
    //! production path: a full live world from `setup_world_with_config`,
    //! the real live schedule via `run_simulation_tick`, and the real
    //! `impact_system` registered in it. A strike spawned as an
    //! `OrbitalEvent` component must resolve end-to-end: crater terrain,
    //! building destruction, harvestable ImpactSite, chronicle feedback.
    use super::*;
    use crate::layer1::building::{try_place_building, BuildingType};
    use crate::layer1::chronicle::Chronicle;
    use crate::layer1::terrain::{TerrainGrid, TerrainType};

    fn place_and_strike(world: &mut World) -> (i32, i32) {
        let mut placed = None;
        'outer: for x in 4..60 {
            for y in 4..60 {
                if try_place_building(world, x, y, BuildingType::Housing) {
                    placed = Some((x, y));
                    break 'outer;
                }
            }
        }
        let (x, y) = placed.expect("a house should be placeable in a live world");
        // Let the building settle (era stamping etc.) before the strike:
        // destroying a building in the same tick it was placed trips a
        // pre-existing deferred-insert race in the era stamper that every
        // despawn path shares; strikes in live play hit standing buildings.
        for _ in 0..2 {
            crate::simulation::run_simulation_tick(world);
        }
        world.spawn(OrbitalEvent {
            target: GridPosition { x, y },
            damage: 500.0,
            heat: 100.0,
        });
        (x, y)
    }

    #[test]
    fn live_strike_craters_terrain_destroys_building_spawns_scrap_and_chronicles() {
        let mut world = crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        let (x, y) = place_and_strike(&mut world);
        for _ in 0..3 {
            crate::simulation::run_simulation_tick(&mut world);
        }
        // Crater terrain (spec 356 acceptance; Rock was a placeholder).
        let grid = world.resource::<TerrainGrid>();
        assert_eq!(
            grid.get(x as usize, y as usize),
            Some(TerrainType::Crater),
            "live strike must crater the terrain"
        );
        // Harvestable impact site at the strike tile.
        let mut q = world.query::<(&ImpactSite, &GridPosition)>();
        assert!(
            q.iter(&world).any(|(_, p)| p.x == x && p.y == y),
            "live strike must spawn a harvestable ImpactSite"
        );
        // Building at the strike tile destroyed (500 dmg vs 100 HP).
        let mut buildings = world.query::<(&crate::layer1::building::Building, &GridPosition)>();
        assert!(
            !buildings.iter(&world).any(|(_, p)| p.x == x && p.y == y),
            "live strike must destroy the building on its tile"
        );
        // Chronicle feedback the player can see.
        let chronicle = world.resource::<Chronicle>();
        assert!(
            chronicle
                .events
                .iter()
                .any(|e| e.text.to_ascii_lowercase().contains("crossfire")),
            "live strike must write a crossfire chronicle entry, got: {:?}",
            chronicle
                .events
                .iter()
                .map(|e| e.text.clone())
                .collect::<Vec<_>>()
        );
        // Stats ledger counted the strike.
        let stats = world
            .get_resource::<CrossfireStats>()
            .expect("live world must carry CrossfireStats after a tick");
        assert!(
            stats.total_strikes >= 1,
            "live strike must increment CrossfireStats, got {}",
            stats.total_strikes
        );
    }

    #[test]
    fn live_director_spawns_organic_strikes_end_to_end() {
        let mut world = crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        // Force the organic director: roll every tick, always hits.
        world.insert_resource(CrossfireDirector {
            check_interval_ticks: 1,
            base_chance: 1.0,
            max_chance: 1.0,
            ..CrossfireDirector::default()
        });
        for _ in 0..5 {
            crate::simulation::run_simulation_tick(&mut world);
        }
        let stats = world.resource::<CrossfireStats>();
        assert!(
            stats.total_strikes >= 1,
            "forced organic director must produce strikes on the live schedule, got {}",
            stats.total_strikes
        );
        assert!(
            impact_site_count(&mut world) >= 1,
            "organic strikes must leave harvestable impact sites"
        );
        let chronicle = world.resource::<Chronicle>();
        assert!(
            chronicle
                .events
                .iter()
                .any(|e| e.text.to_ascii_lowercase().contains("crossfire")),
            "organic strikes must be chronicled"
        );
    }

    #[test]
    fn live_director_silent_when_chance_is_zero() {
        let mut world = crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        world.insert_resource(CrossfireDirector {
            check_interval_ticks: 1,
            base_chance: 0.0,
            debris_chance_per_unit: 0.0,
            ..CrossfireDirector::default()
        });
        for _ in 0..10 {
            crate::simulation::run_simulation_tick(&mut world);
        }
        let stats = world.resource::<CrossfireStats>();
        assert_eq!(
            stats.total_strikes, 0,
            "a zero-chance director must never strike"
        );
        assert_eq!(impact_site_count(&mut world), 0);
    }

    #[test]
    fn chance_scales_with_debris_and_caps() {
        let director = CrossfireDirector::default();
        let clean = crossfire_chance(&director, 0.0);
        assert!((clean - CROSSFIRE_BASE_CHANCE).abs() < f64::EPSILON);
        let dirty = crossfire_chance(&director, 1.0);
        assert!(dirty > clean, "debris must raise the strike chance");
        let filthy = crossfire_chance(&director, 100.0);
        assert!(
            (filthy - CROSSFIRE_MAX_CHANCE).abs() < f64::EPSILON,
            "chance must cap at the director maximum, got {filthy}"
        );
    }

    #[test]
    fn ip_guard_public_strings_are_original() {
        // Every public string this module emits must be original/generic:
        // no named franchise, vessel, weapon, or place may appear.
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });
        world.insert_resource(crate::layer1::temperature::TemperatureGrid::new(
            10, 10, 20.0,
        ));
        world.init_resource::<bevy_ecs::event::Events<AddChronicleEvent>>();
        world.init_resource::<MessageLog>();
        world.init_resource::<CrossfireStats>();
        world.spawn(OrbitalEvent {
            target: GridPosition { x: 5, y: 5 },
            damage: 100.0,
            heat: 100.0,
        });
        impact_system(&mut world);
        let events = world.resource::<bevy_ecs::event::Events<AddChronicleEvent>>();
        let texts: Vec<String> = events
            .get_cursor()
            .read(events)
            .map(|e| e.text.clone())
            .collect();
        assert!(!texts.is_empty(), "a strike must emit a chronicle entry");
        let banned = [
            "death star",
            "star destroyer",
            "turbolaser",
            "photon torpedo",
            "enterprise",
            "starfleet",
            "imperium",
            "lasgun",
            "macross",
            "yamato",
            "galactica",
        ];
        for text in &texts {
            let lower = text.to_ascii_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string contains banned IP term {term:?}: {text}"
                );
            }
        }
    }
}
