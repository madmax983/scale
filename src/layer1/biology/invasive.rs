//! Invasive Biology (Spec 326): alien spores hitchhike on trade ships,
//! take root in living soil, spread across the map, and can be burned out
//! with quarantine burns.
//!
//! Design notes:
//! - The spec sketch assumed `Ship { origin: BiomeType::Alien }`,
//!   `TerrainType::Xenofungus`, and `DesignationType::Burn`; none exist.
//!   Adapted: the real [`TradeShipArrivalEvent`] (faction-string heuristic
//!   for "alien biome"), the existing [`TerrainType::SporeBloom`] for the
//!   infestation, a new [`TerrainType::Ash`] only for burn aftermath, and a
//!   new [`BurnOrder`] component instead of the nonexistent designation.
//! - Burns jump the work queue (processed immediately at high priority per
//!   spec section 7) rather than routing through the slow job system.

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::nature::terrain::{TerrainGrid, TerrainType};
use crate::layer1::director::DirectorState;
use crate::layer2::trade::blockade::TradeShipArrivalEvent;

/// Alien flora strain carried by a spore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SporeStrain {
    /// Slow, relentless spreader.
    Xenofungus,
    /// Fast, aggressive spreader.
    TerranKudzu,
}

impl SporeStrain {
    /// Ticks between spread attempts for this strain.
    #[must_use]
    pub const fn spread_interval(self) -> u32 {
        match self {
            Self::Xenofungus => 40,
            Self::TerranKudzu => 25,
        }
    }
}

/// An alien spore that has taken root (or is about to) at a grid position.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spore {
    /// Where the spore sits.
    pub position: GridPosition,
    /// Which strain it carries.
    pub strain: SporeStrain,
    /// Ticks until the next spread attempt.
    pub spread_timer: u32,
    /// Ticks since this spore took root (Spec 1388 senescence).
    pub age: u32,
}

/// Player-placed quarantine burn designation. Consumed on execution.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BurnOrder {
    /// Tile to scorch.
    pub position: GridPosition,
}

/// Fired when a spore bloom takes root on a tile.
#[derive(Event, Debug, Clone, Copy)]
pub struct SporeBloomEvent {
    /// Where the bloom rooted.
    pub position: GridPosition,
    /// Which strain bloomed.
    pub strain: SporeStrain,
}

/// Fired when a quarantine burn resolves.
#[derive(Event, Debug, Clone, Copy)]
pub struct QuarantineBurnEvent {
    /// Tile that was scorched.
    pub position: GridPosition,
}

/// Tuning knobs for invasive biology.
#[derive(Resource, Debug, Clone)]
pub struct InvasiveConfig {
    /// Hitchhiker chance per ordinary trade ship arrival.
    pub arrival_chance: f32,
    /// Hitchhiker chance per arrival from an exotic-looking faction.
    pub exotic_arrival_chance: f32,
    /// Hard cap on live spores (spread stops spawning past this).
    pub max_spores: usize,
    /// Spec 1388 senescence: a spore older than this (ticks) despawns,
    /// so the live count can fall below the cap and outbreaks recur.
    pub spore_lifespan: u32,
}

impl Default for InvasiveConfig {
    fn default() -> Self {
        Self {
            arrival_chance: 0.25,
            exotic_arrival_chance: 0.6,
            max_spores: 200,
            spore_lifespan: 1500,
        }
    }
}

/// Substrings that mark a trade faction as plausibly xeno-origin.
const EXOTIC_MARKERS: &[&str] = &[
    "xeno", "void", "outer", "rim", "deep", "alien", "mycel", "spore",
];

/// Returns true when a faction name looks like it hails from alien biomes.
#[must_use]
pub fn is_exotic_faction(faction: &str) -> bool {
    let lowered = faction.to_lowercase();
    EXOTIC_MARKERS.iter().any(|m| lowered.contains(m))
}

/// Returns true for terrain a spore can take root in.
#[must_use]
pub const fn is_infectable(terrain: TerrainType) -> bool {
    matches!(
        terrain,
        TerrainType::Grass
            | TerrainType::Dirt
            | TerrainType::Shrub
            | TerrainType::Sapling
            | TerrainType::Path
    )
}

/// Neighbor offsets in deterministic scan order: east first, so replays agree.
const SPREAD_NEIGHBORS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// Trade ships can carry alien spores in their holds. Exotic-looking
/// factions are far more likely to be contaminated.
#[allow(clippy::too_many_arguments)]
pub fn spore_hitchhiker_system(
    mut commands: Commands,
    mut arrivals: EventReader<TradeShipArrivalEvent>,
    terrain: Res<TerrainGrid>,
    config: Res<InvasiveConfig>,
    spores: Query<&Spore>,
    mut bloom_events: EventWriter<SporeBloomEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    director: Option<ResMut<DirectorState>>,
) {
    // SPEC 1388 (§4.6): seeding is token-gated by the Director. No token
    // (or no DirectorState at all) -> arrivals NEVER seed. The token is
    // consumed by the first arrival that actually seeds a spore.
    let mut token_live = director.as_ref().is_some_and(|d| d.outbreak_token);
    let mut token_consumed = false;
    let mut rng = rand::thread_rng();
    for arrival in arrivals.read() {
        if !token_live {
            continue;
        }
        if spores.iter().count() >= config.max_spores {
            continue;
        }
        let chance = if is_exotic_faction(&arrival.faction) {
            config.exotic_arrival_chance
        } else {
            config.arrival_chance
        };
        if rng.gen::<f32>() >= chance {
            continue;
        }
        // 32 random probes, then a deterministic linear-scan fallback.
        let mut target: Option<(usize, usize)> = None;
        for _ in 0..32 {
            let x = rng.gen_range(0..terrain.width);
            let y = rng.gen_range(0..terrain.height);
            if matches!(terrain.get(x, y), Some(t) if is_infectable(t)) {
                target = Some((x, y));
                break;
            }
        }
        if target.is_none() {
            'scan: for y in 0..terrain.height {
                for x in 0..terrain.width {
                    if matches!(terrain.get(x, y), Some(t) if is_infectable(t)) {
                        target = Some((x, y));
                        break 'scan;
                    }
                }
            }
        }
        if let Some((x, y)) = target {
            let strain = if rng.gen_bool(0.5) {
                SporeStrain::Xenofungus
            } else {
                SporeStrain::TerranKudzu
            };
            let position = GridPosition {
                x: x as i32,
                y: y as i32,
            };
            commands.spawn(Spore {
                position,
                strain,
                spread_timer: strain.spread_interval(),
                age: 0,
            });
            token_live = false;
            token_consumed = true;
            bloom_events.send(SporeBloomEvent { position, strain });
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "[SPORE_OUTBREAK] A trade ship of the {} seeded {:?} spores near ({x}, {y})!",
                    arrival.faction, strain
                ),
                importance: EventImportance::Major,
            });
        }
    }
    if token_consumed {
        if let Some(mut state) = director {
            state.outbreak_token = false;
        }
    }
}

/// Each tick, spores take root (converting their home tile to SporeBloom)
/// and, when their timer fires, infect the first infectable neighbor in
/// deterministic east-first scan order.
pub fn spore_spread_system(
    mut commands: Commands,
    mut terrain: ResMut<TerrainGrid>,
    config: Res<InvasiveConfig>,
    mut spores: Query<(Entity, &mut Spore)>,
) {
    let mut live = spores.iter().count();
    for (entity, mut spore) in spores.iter_mut() {
        // Take root: convert the home tile to SporeBloom.
        let (hx, hy) = (spore.position.x, spore.position.y);
        if hx >= 0 && hy >= 0 {
            let (x, y) = (hx as usize, hy as usize);
            if matches!(terrain.get(x, y), Some(t) if is_infectable(t)) {
                terrain.set(x, y, TerrainType::SporeBloom);
            }
        }
        // SPEC 1388 senescence (§4.6): an aged spore despawns. Terrain
        // stays SporeBloom (the scar remains); the live count falls
        // below the cap so the outbreak class can fire again.
        spore.age = spore.age.saturating_add(1);
        if config.spore_lifespan > 0 && spore.age >= config.spore_lifespan {
            commands.entity(entity).despawn();
            live = live.saturating_sub(1);
            continue;
        }
        if spore.spread_timer > 0 {
            spore.spread_timer -= 1;
            continue;
        }
        spore.spread_timer = spore.strain.spread_interval();
        if live >= config.max_spores {
            continue;
        }
        for (dx, dy) in SPREAD_NEIGHBORS {
            let (nx, ny) = (hx + dx, hy + dy);
            if nx < 0 || ny < 0 {
                continue;
            }
            let (x, y) = (nx as usize, ny as usize);
            if matches!(terrain.get(x, y), Some(t) if is_infectable(t)) {
                commands.spawn(Spore {
                    position: GridPosition { x: nx, y: ny },
                    strain: spore.strain,
                    spread_timer: spore.strain.spread_interval(),
                    age: 0,
                });
                live += 1;
                break;
            }
        }
    }
}

/// Quarantine burns resolve immediately at high priority: every spore on
/// the designated tile is destroyed and the ground is scorched to Ash.
pub fn quarantine_burn_system(
    mut commands: Commands,
    mut terrain: ResMut<TerrainGrid>,
    burns: Query<(Entity, &BurnOrder)>,
    spores: Query<(Entity, &Spore)>,
    mut burn_events: EventWriter<QuarantineBurnEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for (burn_entity, order) in burns.iter() {
        // Destroy every spore on the designated tile.
        for (spore_entity, spore) in spores.iter() {
            if spore.position == order.position {
                commands.entity(spore_entity).despawn();
            }
        }
        // Scorch bloom or infectable ground to Ash.
        let (bx, by) = (order.position.x, order.position.y);
        if bx >= 0 && by >= 0 {
            let (x, y) = (bx as usize, by as usize);
            if matches!(terrain.get(x, y), Some(t) if t == TerrainType::SporeBloom || is_infectable(t))
            {
                terrain.set(x, y, TerrainType::Ash);
            }
        }
        burn_events.send(QuarantineBurnEvent {
            position: order.position,
        });
        chronicle.send(AddChronicleEvent {
            text: format!(
                "[QUARANTINE_BURN] Quarantine burn at ({}, {}): ground scorched to ash.",
                order.position.x, order.position.y
            ),
            importance: EventImportance::Minor,
        });
        commands.entity(burn_entity).despawn();
    }
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
        app.insert_resource(InvasiveConfig::default());
        // Spec 1388: seeding requires a Director outbreak token. The
        // legacy seeding tests run with a token granted; the dedicated
        // no-token test below revokes it.
        app.insert_resource(DirectorState {
            outbreak_token: true,
            ..DirectorState::default()
        });
        app.init_resource::<Events<TradeShipArrivalEvent>>();
        app.init_resource::<Events<SporeBloomEvent>>();
        app.init_resource::<Events<QuarantineBurnEvent>>();
        app.init_resource::<Events<AddChronicleEvent>>();
        app.add_systems(
            Update,
            (
                spore_hitchhiker_system,
                spore_spread_system,
                quarantine_burn_system,
            ),
        );
        app
    }

    fn event_count<T: Event>(app: &App) -> usize {
        let events = app.world().resource::<Events<T>>();
        events.get_reader().read(events).count()
    }

    #[test]
    fn test_exotic_faction_heuristic() {
        for faction in [
            "Xeno Traders",
            "Void Collective",
            "Outer Rim Exchange",
            "Deep Mycel Network",
            "Alien Spore Caravan",
            "RIMWARD SPORELINGS",
        ] {
            assert!(
                is_exotic_faction(faction),
                "{faction} should read as exotic"
            );
        }
        for faction in ["Sol Standard Freight", "Terran Freight Lines", "Inner Markets Guild"] {
            assert!(
                !is_exotic_faction(faction),
                "{faction} should NOT read as exotic"
            );
        }
    }

    #[test]
    fn test_spore_arrival_on_ship() {
        let mut app = setup_test_app();
        // 50 exotic arrivals at 0.6 chance: failure odds ~1e-20.
        for _ in 0..50 {
            app.world_mut()
                .resource_mut::<Events<TradeShipArrivalEvent>>()
                .send(TradeShipArrivalEvent {
                    cargo_value: 100.0,
                    faction: "Xeno Traders".to_string(),
                });
        }
        app.update();
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert!(
            spore_count > 0,
            "Ship arrival should occasionally spawn spores."
        );
    }

    #[test]
    fn test_spore_arrival_respects_max_cap() {
        let mut app = setup_test_app();
        app.insert_resource(InvasiveConfig {
            max_spores: 0,
            ..Default::default()
        });
        app.world_mut()
            .resource_mut::<Events<TradeShipArrivalEvent>>()
            .send(TradeShipArrivalEvent {
                cargo_value: 100.0,
                faction: "Xeno Traders".to_string(),
            });
        app.update();
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 0, "No spores may root past the cap.");
    }

    #[test]
    fn test_spore_spreads_and_replaces_terrain() {
        let mut app = setup_test_app();
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::Xenofungus,
            spread_timer: 0,
            age: 0,
        });
        app.update(); // Trigger spread: daughter spore spawns east.
        let mut daughter_at_east = false;
        for spore in app.world_mut().query::<&Spore>().iter(app.world()) {
            if spore.position == (GridPosition { x: 6, y: 5 }) {
                daughter_at_east = true;
            }
        }
        assert!(
            daughter_at_east,
            "Spread must spawn a daughter spore on the east neighbor first (deterministic scan order)."
        );
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 2, "Spread must spawn exactly one daughter spore.");
        app.update(); // Daughter takes root on its home tile.
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(6, 5),
            Some(TerrainType::SporeBloom),
            "The daughter spore must convert its home tile."
        );
    }

    #[test]
    fn test_spore_takes_root_on_home_tile() {
        let mut app = setup_test_app();
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::TerranKudzu,
            spread_timer: 999,
            age: 0,
        });
        app.update();
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::SporeBloom),
            "A spore must convert its home tile even before spreading."
        );
    }

    #[test]
    fn test_terrankudzu_spreads_faster() {
        assert!(
            SporeStrain::TerranKudzu.spread_interval() < SporeStrain::Xenofungus.spread_interval(),
            "TerranKudzu must be the more aggressive strain."
        );
        assert_eq!(SporeStrain::Xenofungus.spread_interval(), 40);
        assert_eq!(SporeStrain::TerranKudzu.spread_interval(), 25);
    }

    #[test]
    fn test_spore_quarantine_burn() {
        let mut app = setup_test_app();
        let tile = GridPosition { x: 5, y: 5 };
        app.world_mut().spawn(Spore {
            position: tile,
            strain: SporeStrain::Xenofungus,
            spread_timer: 999,
            age: 0,
        });
        app.world_mut().spawn(BurnOrder { position: tile });
        app.update();
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 0, "Burn must destroy every spore on the tile.");
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::Ash),
            "Burned ground must be scorched to Ash."
        );
        let order_count = app
            .world_mut()
            .query::<&BurnOrder>()
            .iter(app.world())
            .count();
        assert_eq!(order_count, 0, "Burn orders are consumed on execution.");
        assert!(
            event_count::<QuarantineBurnEvent>(&app) > 0,
            "Burn must fire a QuarantineBurnEvent."
        );
    }

    #[test]
    fn test_no_outbreak_without_director_token() {
        // Spec 1388 §4.6: without a token, arrivals never seed — even
        // exotic arrivals at guaranteed chance.
        let mut app = setup_test_app();
        app.insert_resource(InvasiveConfig {
            arrival_chance: 1.0,
            exotic_arrival_chance: 1.0,
            ..Default::default()
        });
        app.world_mut().resource_mut::<DirectorState>().outbreak_token = false;
        for _ in 0..10 {
            app.world_mut()
                .resource_mut::<Events<TradeShipArrivalEvent>>()
                .send(TradeShipArrivalEvent {
                    cargo_value: 100.0,
                    faction: "Xeno Traders".to_string(),
                });
        }
        app.update();
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 0, "No token -> arrivals never seed.");
    }

    #[test]
    fn test_spore_senescence_despawns_aged_spore() {
        // Spec 1388 §4.6: at spore_lifespan the entity despawns; the
        // terrain scar (SporeBloom) remains.
        let mut app = setup_test_app();
        app.insert_resource(InvasiveConfig {
            spore_lifespan: 3,
            ..Default::default()
        });
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::Xenofungus,
            spread_timer: 999,
            age: 0,
        });
        for _ in 0..4 {
            app.update();
        }
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 0, "Aged spore must senesce and despawn.");
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::SporeBloom),
            "Terrain stays SporeBloom after senescence."
        );
    }

    #[test]
    fn test_burn_clears_bloom_without_spore() {
        let mut app = setup_test_app();
        app.world_mut()
            .resource_mut::<TerrainGrid>()
            .set(5, 5, TerrainType::SporeBloom);
        app.world_mut().spawn(BurnOrder {
            position: GridPosition { x: 5, y: 5 },
        });
        app.update();
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::Ash),
            "Burns must scorch spore-free bloom too."
        );
    }
}
