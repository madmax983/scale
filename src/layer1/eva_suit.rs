//! Spec 1384 — EVA Suits: timed air for the world outside the hull.
//!
//! An EVA suit is equippable body-slot gear with a timed air supply.
//! While a worn suit has air, vacuum exposure (below
//! [`SUFFOCATION_PRESSURE_THRESHOLD`]) drains the suit's air instead of
//! the pop's HP (the drain decision lives inside
//! `pressure_damage_system`'s branch — see `physics::pressure`). When
//! the bottle runs dry, the existing suffocation rules resume,
//! unchanged. Air refills in pressurized space via [`suit_air_system`].
//!
//! Suits are physical: colony-issued suits are tracked as an abstract
//! pool (`ColonyResources::eva_suits`, mirroring `clothing`) and
//! materialize into [`EvaSuit`] entities on fetch; loose physical suits
//! (migrant expedition gear, death drops) are [`EvaSuit`] entities in
//! the world and are never pooled. Research never retrofits a bottle
//! already issued — capacity is stamped at issue from the colony's
//! current issue tier ([`issue_tier`]).
//!
//! Original names only (IP rule): no named characters, places, or
//! borrowed IP — a suit is a round trip, not a home.

use bevy_ecs::prelude::*;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::items::{Equipment, Item};
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, PopName};
use crate::layer1::pressure::{PressureGrid, SUFFOCATION_PRESSURE_THRESHOLD};
use crate::layer1::resources::ColonyResources;
use crate::layer1::tech::{Tech, TechState};

/// Air drained per tick spent in vacuum (mirrors 1 HP/tick suffocation:
/// a day out costs a day of air).
pub const EVA_AIR_DRAIN_PER_TICK: f32 = 1.0;
/// Fraction of capacity below which the low-air chronicle warning fires.
pub const EVA_AIR_LOW_FRACTION: f32 = 0.25;
/// Fraction of capacity refilled per tick in pressurized space
/// (a full refill takes ~10 ticks indoors).
pub const EVA_REFILL_FRACTION_PER_TICK: f32 = 0.1;
/// Transit drain band for arriving migrants, in days (Spec 1384 §4.6).
pub const EVA_MIGRANT_TRANSIT_MIN: u32 = 30;
/// Transit drain band for arriving migrants, in days (Spec 1384 §4.6).
pub const EVA_MIGRANT_TRANSIT_MAX: u32 = 60;

/// Class of an EVA suit. Capacity is stamped at issue; research never
/// retrofits an existing bottle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SuitClass {
    /// Base suit — 50 days of air.
    Base,
    /// Mk II suit (Closed-Cycle Breathing) — 100 days of air.
    Mk2,
    /// Expedition suit (Expedition Systems / migrant gear) — 200 days.
    Expedition,
}

impl SuitClass {
    /// Air capacity in days (1 tick = 1 day). Mark's ladder, 2026-10-14.
    #[must_use]
    pub const fn capacity(self) -> f32 {
        match self {
            Self::Base => 50.0,
            Self::Mk2 => 100.0,
            Self::Expedition => 200.0,
        }
    }

    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Base => "Base",
            Self::Mk2 => "Mk II",
            Self::Expedition => "Expedition",
        }
    }

    /// Stable save-file name (lowercase, per the persistence module's
    /// `SuitSave` convention: `base` / `mk2` / `expedition`).
    #[must_use]
    pub const fn save_name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Mk2 => "mk2",
            Self::Expedition => "expedition",
        }
    }

    /// Parse a save-file / label name back into a class.
    /// Case-insensitive; also accepts the label spellings.
    #[must_use]
    pub fn from_save_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "base" => Some(Self::Base),
            "mk2" | "mk ii" | "mkii" => Some(Self::Mk2),
            "expedition" => Some(Self::Expedition),
            _ => None,
        }
    }

    /// A fresh suit of this class, full bottle, full durability.
    #[must_use]
    pub fn fresh(self, colony_issued: bool) -> EvaSuit {
        EvaSuit {
            class: self,
            air: self.capacity(),
            durability: 100.0,
            max_durability: 100.0,
            colony_issued,
        }
    }
}

/// A worn or loose EVA suit (an item entity, like `Clothing`).
#[derive(Component, Debug, Clone)]
pub struct EvaSuit {
    /// Suit class; capacity was stamped at issue.
    pub class: SuitClass,
    /// Days of air remaining. Drains in vacuum, refills in air.
    pub air: f32,
    /// Follows clothing wear (`clothing_wear_system` rates).
    pub durability: f32,
    /// Maximum durability.
    pub max_durability: f32,
    /// True when the suit came from the colony pool: on doff or death
    /// it returns to the pool. False for physical suits (migrant
    /// expedition gear): those drop as loose entities instead.
    pub colony_issued: bool,
}

impl EvaSuit {
    /// Air capacity for this suit's class.
    #[must_use]
    pub fn capacity(&self) -> f32 {
        self.class.capacity()
    }

    /// Remaining air as a fraction of capacity (0.0–1.0).
    #[must_use]
    pub fn air_fraction(&self) -> f32 {
        let cap = self.capacity();
        if cap <= 0.0 {
            0.0
        } else {
            (self.air / cap).clamp(0.0, 1.0)
        }
    }
}

/// Per-pop alert state for suit-air chronicle warnings.
///
/// Mirrors the `SuffocationAlerted` pattern: each warning fires once
/// per excursion, and the state re-arms when the suit refills above
/// the low-air threshold.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SuitAirAlerted {
    /// The low-air warning has fired for this excursion.
    pub low_warned: bool,
    /// The run-dry warning has fired for this excursion.
    pub dry_warned: bool,
}

/// Scenario-set bottle floor: the lowest class the colony issues,
/// regardless of research (Layer 2 Ready starts at Mk II).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScenarioSuitFloor(pub SuitClass);

impl Default for ScenarioSuitFloor {
    fn default() -> Self {
        Self(SuitClass::Base)
    }
}

impl ScenarioSuitFloor {
    /// The floor class.
    #[must_use]
    pub fn class(self) -> SuitClass {
        self.0
    }
}

/// Colony-level suit snapshot, refreshed per tick by
/// [`update_colony_suit_status_system`] (mirrors `ColonyAtmosphere`).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ColonySuitStatus {
    /// Suits sitting in the abstract colony pool.
    pub pool: usize,
    /// Suits currently worn by living pops.
    pub worn: usize,
    /// Lowest air among suited pops currently below the pressure
    /// threshold (`None` when nobody suited is in vacuum).
    pub lowest_air_in_vacuum: Option<f32>,
    /// Air fraction (of its own class capacity) of that same
    /// lowest-air bottle — the class-aware low-air check the status
    /// bar needs (25% of an Expedition bottle is 50 days, not 12.5).
    pub lowest_air_fraction_in_vacuum: Option<f32>,
}

/// The colony's current issue tier: the highest of the scenario bottle
/// floor and the researched tier. Pure and unit-tested against all
/// combinations of floor × tech state.
#[must_use]
pub fn issue_tier(tech: Option<&TechState>, floor: SuitClass) -> SuitClass {
    let researched = match tech {
        Some(t) if t.is_unlocked(Tech::ExpeditionSystems) => SuitClass::Expedition,
        Some(t) if t.is_unlocked(Tech::ClosedCycleBreathing) => SuitClass::Mk2,
        _ => SuitClass::Base,
    };
    researched.max(floor)
}

/// Issue capacity with no scenario floor (Base floor).
#[must_use]
pub fn issue_capacity(tech: &TechState) -> f32 {
    issue_tier(Some(tech), SuitClass::Base).capacity()
}

/// Issue capacity for a colony with the given scenario floor.
#[must_use]
pub fn issue_capacity_with_floor(tech: Option<&TechState>, floor: SuitClass) -> f32 {
    issue_tier(tech, floor).capacity()
}

/// Remaining air for a newly arrived migrant's expedition suit:
/// a full 200-day bottle drained by 30–60 days of transit.
#[must_use]
pub fn migrant_suit_air<R: rand::Rng>(rng: &mut R) -> f32 {
    let transit = rng.gen_range(EVA_MIGRANT_TRANSIT_MIN..=EVA_MIGRANT_TRANSIT_MAX) as f32;
    SuitClass::Expedition.capacity() - transit
}

/// Refill system: a suited pop on a tile at or above the suffocation
/// threshold regains air, capped at capacity, and their suit-air
/// alerts re-arm once the bottle is back above the low threshold.
/// Loose (unworn) suits never refill in v1.
///
/// Registered in the real Layer 1 schedule before
/// `pressure_damage_system` (see `systems::consumption`).
#[allow(clippy::type_complexity)]
pub fn suit_air_system(
    mut commands: Commands,
    pops: Query<
        (
            Entity,
            &GridPosition,
            Option<&Equipment>,
            Option<&SuitAirAlerted>,
        ),
        With<Pop>,
    >,
    mut suits: Query<&mut EvaSuit>,
    grid_opt: Option<Res<PressureGrid>>,
) {
    let Some(grid) = grid_opt.as_deref() else {
        return;
    };
    for (entity, pos, equipment, alerted) in pops.iter() {
        let Some(body) = equipment.and_then(|eq| eq.body) else {
            continue;
        };
        let Ok(mut suit) = suits.get_mut(body) else {
            continue;
        };
        if grid.get(pos.x, pos.y) < SUFFOCATION_PRESSURE_THRESHOLD {
            continue;
        }
        let cap = suit.capacity();
        if suit.air < cap {
            suit.air = (suit.air + cap * EVA_REFILL_FRACTION_PER_TICK).min(cap);
        }
        // Re-arm the chronicle warnings once the bottle is healthy.
        if suit.air_fraction() > EVA_AIR_LOW_FRACTION {
            if let Some(state) = alerted {
                if state.low_warned || state.dry_warned {
                    commands
                        .entity(entity)
                        .try_insert(SuitAirAlerted::default());
                }
            }
        }
    }
}

/// Refresh the [`ColonySuitStatus`] snapshot from the live world.
#[allow(clippy::type_complexity)]
pub fn update_colony_suit_status_system(
    mut snapshot: ResMut<ColonySuitStatus>,
    resources: Option<Res<ColonyResources>>,
    pops: Query<(&GridPosition, Option<&Equipment>), With<Pop>>,
    suits: Query<&EvaSuit>,
    grid_opt: Option<Res<PressureGrid>>,
) {
    let pool = resources.map_or(0, |r| r.eva_suits.max(0.0) as usize);
    let mut worn = 0usize;
    let mut lowest: Option<f32> = None;
    let mut lowest_fraction: Option<f32> = None;
    for (pos, equipment) in pops.iter() {
        let Some(body) = equipment.and_then(|eq| eq.body) else {
            continue;
        };
        let Ok(suit) = suits.get(body) else {
            continue;
        };
        worn += 1;
        if let Some(grid) = grid_opt.as_deref() {
            if grid.get(pos.x, pos.y) < SUFFOCATION_PRESSURE_THRESHOLD
                && lowest.is_none_or(|l| suit.air < l)
            {
                lowest = Some(suit.air);
                lowest_fraction = Some(suit.air_fraction());
            }
        }
    }
    *snapshot = ColonySuitStatus {
        pool,
        worn,
        lowest_air_in_vacuum: lowest,
        lowest_air_fraction_in_vacuum: lowest_fraction,
    };
}

/// Marker inserted once a dead pop's suit has been recovered, so the
/// recovery system never double-counts before the corpse despawns.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SuitRecovered;

/// Death rule (Spec 1384 §4.9): a suit is never silently deleted by
/// its wearer's death. A colony-issued suit returns to the pool; a
/// physical (expedition) suit stays in the world as a loose entity at
/// the death tile, air preserved.
#[allow(clippy::type_complexity)]
pub fn suit_death_recovery_system(
    mut commands: Commands,
    dead: Query<
        (Entity, &GridPosition, Option<&Equipment>),
        (
            With<Pop>,
            With<crate::layer1::health::Dead>,
            Without<SuitRecovered>,
        ),
    >,
    suits: Query<&EvaSuit>,
    mut resources: Option<ResMut<ColonyResources>>,
) {
    for (entity, pos, equipment) in dead.iter() {
        commands.entity(entity).try_insert(SuitRecovered);
        let Some(body) = equipment.and_then(|eq| eq.body) else {
            continue;
        };
        let Ok(suit) = suits.get(body) else {
            continue;
        };
        if suit.colony_issued {
            if let Some(res) = resources.as_deref_mut() {
                res.eva_suits += 1.0;
            }
            commands.entity(body).despawn();
        } else {
            // Physical suit: leave the entity in the world, re-anchored
            // at the death tile — a rescuer can wear a dead pop's
            // bottle, and in vacuum that pickup is itself an expedition.
            commands.entity(body).insert((Item::default(), *pos));
        }
        // The corpse does not wear the suit.
        commands.entity(entity).insert(Equipment {
            body: None,
            ..equipment.copied().unwrap_or_default()
        });
    }
}

/// Count of loose physical suits in a world (not worn, not pooled).
/// Used by the headless `suits` panel and STATS helpers.
#[must_use]
pub fn loose_suit_count(world: &mut World) -> usize {
    suit_rows(world).1.len()
}

/// Rows for the headless `suits` panel: (pool, worn rows, loose rows).
/// Each row is (label, class, air) where label is the wearer name or a
/// position string.
#[must_use]
#[allow(clippy::type_complexity)]
pub fn suit_rows(
    world: &mut World,
) -> (
    usize,
    Vec<(String, SuitClass, f32)>,
    Vec<(String, SuitClass, f32)>,
) {
    let pool = world
        .get_resource::<ColonyResources>()
        .map_or(0, |r| r.eva_suits.max(0.0) as usize);
    let worn_entities: std::collections::HashSet<Entity> = {
        let mut q = world.query_filtered::<&Equipment, With<Pop>>();
        q.iter(world).filter_map(|eq| eq.body).collect()
    };
    let mut worn_rows = Vec::new();
    {
        let mut q = world.query_filtered::<(Option<&PopName>, &Equipment), With<Pop>>();
        for (name, eq) in q.iter(world) {
            if let Some(body) = eq.body {
                if let Some(suit) = world.get::<EvaSuit>(body) {
                    worn_rows.push((
                        name.map_or_else(|| "A colonist".to_string(), |n| n.0.clone()),
                        suit.class,
                        suit.air,
                    ));
                }
            }
        }
    }
    let mut loose_rows = Vec::new();
    {
        let mut q = world.query::<(Entity, &EvaSuit, Option<&GridPosition>)>();
        for (e, suit, pos) in q.iter(world) {
            if worn_entities.contains(&e) {
                continue;
            }
            let label = pos.map_or_else(
                || "loose".to_string(),
                |p| format!("loose at ({}, {})", p.x, p.y),
            );
            loose_rows.push((label, suit.class, suit.air));
        }
    }
    (pool, worn_rows, loose_rows)
}

/// STATS helper: (pool, worn, lowest air in vacuum or None).
///
/// Computed live from the world rather than read from the
/// [`ColonySuitStatus`] snapshot: the snapshot only refreshes on the
/// first tick, so a tick-0 STATS would report `suits=0` for a Classic
/// colony whose pool grant (3) is already in `ColonyResources`
/// (Spec 1384 §6 acceptance). The snapshot still feeds the status bar.
#[must_use]
pub fn suit_stats(world: &mut World) -> (usize, usize, Option<f32>) {
    let (pool, worn_rows, _) = suit_rows(world);
    // Two passes: collect suited pops' positions first (the query
    // borrows the world), then read the grid and bottles.
    let suited_positions: Vec<(GridPosition, Entity)> = {
        let mut q = world.query_filtered::<(&GridPosition, &Equipment), With<Pop>>();
        q.iter(world)
            .filter_map(|(pos, eq)| eq.body.map(|body| (*pos, body)))
            .collect()
    };
    let mut lowest: Option<f32> = None;
    for (pos, body) in suited_positions {
        let in_vacuum = world
            .get_resource::<PressureGrid>()
            .is_some_and(|grid| grid.get(pos.x, pos.y) < SUFFOCATION_PRESSURE_THRESHOLD);
        if !in_vacuum {
            continue;
        }
        if let Some(suit) = world.get::<EvaSuit>(body) {
            lowest = Some(lowest.map_or(suit.air, |l: f32| l.min(suit.air)));
        }
    }
    (pool, worn_rows.len(), lowest)
}

/// Chronicle helper retained for callers outside the pressure system
/// (kept tiny: the live warnings fire inside `pressure_damage_system`).
pub fn send_suit_chronicle(events: &mut EventWriter<AddChronicleEvent>, text: String) {
    events.send(AddChronicleEvent {
        text,
        importance: EventImportance::Major,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::health::Health;
    use crate::layer1::pressure::pressure_damage_system;
    use bevy_ecs::system::RunSystemOnce;

    fn vacuum_world() -> World {
        let mut world = World::new();
        world.insert_resource(PressureGrid::new(10, 10));
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    fn spawn_suited_pop(
        world: &mut World,
        air: f32,
        class: SuitClass,
        x: i32,
        y: i32,
    ) -> (Entity, Entity) {
        let suit = world
            .spawn((
                Item::default(),
                EvaSuit {
                    class,
                    air,
                    durability: 100.0,
                    max_durability: 100.0,
                    colony_issued: true,
                },
                GridPosition { x, y },
            ))
            .id();
        let pop = world
            .spawn((
                Pop,
                GridPosition { x, y },
                Health::default(),
                PopName("Ada".to_string()),
                Equipment {
                    body: Some(suit),
                    ..Default::default()
                },
            ))
            .id();
        (pop, suit)
    }

    // --- Air drain / protection ---

    #[test]
    fn test_suited_pop_in_vacuum_drains_air_not_hp() {
        let mut world = vacuum_world();
        let (pop, suit) = spawn_suited_pop(&mut world, 50.0, SuitClass::Base, 5, 5);
        world.run_system_once(pressure_damage_system).unwrap();
        world.flush();
        assert_eq!(world.get::<EvaSuit>(suit).unwrap().air, 49.0);
        assert_eq!(world.get::<Health>(pop).unwrap().current, 100.0);
    }

    #[test]
    fn test_dry_suit_resumes_suffocation() {
        let mut world = vacuum_world();
        let (pop, _suit) = spawn_suited_pop(&mut world, 0.0, SuitClass::Base, 5, 5);
        world.run_system_once(pressure_damage_system).unwrap();
        world.flush();
        assert_eq!(world.get::<Health>(pop).unwrap().current, 99.0);
        assert!(world
            .get::<crate::layer1::pressure::SuffocationAlerted>(pop)
            .is_some());
    }

    #[test]
    fn test_air_refills_in_pressurized_space_and_caps() {
        let mut world = World::new();
        let mut grid = PressureGrid::new(10, 10);
        grid.fill(1.0);
        world.insert_resource(grid);
        let (_pop, suit) = spawn_suited_pop(&mut world, 48.0, SuitClass::Base, 5, 5);
        for _ in 0..10 {
            world.run_system_once(suit_air_system).unwrap();
            world.flush();
        }
        assert_eq!(world.get::<EvaSuit>(suit).unwrap().air, 50.0);
    }

    #[test]
    fn test_air_low_and_dry_chronicle_fire_once_per_excursion() {
        let mut world = vacuum_world();
        // 26% of a 50-day bottle.
        let (_pop, _suit) = spawn_suited_pop(&mut world, 13.0, SuitClass::Base, 5, 5);
        for _ in 0..20 {
            world.run_system_once(pressure_damage_system).unwrap();
            world.flush();
        }
        // The alert state proves once-per-excursion firing —
        // both warnings latched, and health only started dropping after
        // the bottle hit zero (13 ticks of air, then 7 ticks of damage).
        // (The exact chronicle strings are verified again in the
        // headless playtest, where the Chronicle resource is readable.)
        let pop_entity = world.query_filtered::<Entity, With<Pop>>().single(&world);
        let alert = world
            .get::<SuitAirAlerted>(pop_entity)
            .expect("alert state latched");
        assert!(alert.low_warned, "low warning fired");
        assert!(alert.dry_warned, "dry warning fired");
        let health = world.get::<Health>(pop_entity).unwrap().current;
        assert_eq!(health, 93.0, "7 unprotected ticks after 13 suited ticks");
    }

    // --- Tiers & tech ---

    #[test]
    fn test_suit_capacity_follows_issue_tier() {
        assert_eq!(issue_capacity(&TechState::default()), 50.0);
        let mut tech = TechState::default();
        tech.techs.insert(
            Tech::ClosedCycleBreathing,
            crate::layer1::tech::TechStatus::Active,
        );
        assert_eq!(issue_capacity(&tech), 100.0);
        tech.techs.insert(
            Tech::ExpeditionSystems,
            crate::layer1::tech::TechStatus::Active,
        );
        assert_eq!(issue_capacity(&tech), 200.0);
        // Scenario floor alone (Layer 2 Ready) issues Mk II.
        assert_eq!(issue_capacity_with_floor(None, SuitClass::Mk2), 100.0);
        // Floor never lowers a researched tier.
        assert_eq!(
            issue_capacity_with_floor(Some(&tech), SuitClass::Base),
            200.0
        );
    }

    #[test]
    fn test_research_does_not_retrofit_worn_suits() {
        let suit = SuitClass::Base.fresh(true);
        assert_eq!(suit.capacity(), 50.0);
        let mut tech = TechState::default();
        tech.techs.insert(
            Tech::ClosedCycleBreathing,
            crate::layer1::tech::TechStatus::Active,
        );
        // The pure function moved on; the physical bottle did not.
        assert_eq!(issue_capacity(&tech), 100.0);
        assert_eq!(suit.capacity(), 50.0);
        assert_eq!(suit.air, 50.0);
    }

    #[test]
    fn test_tech_node_costs_match_spec() {
        assert_eq!(Tech::ClosedCycleBreathing.cost(), 40.0);
        assert_eq!(Tech::ClosedCycleBreathing.storage_cost(), 20.0);
        assert_eq!(Tech::ClosedCycleBreathing.label(), "Closed-Cycle Breathing");
        assert_eq!(Tech::ExpeditionSystems.cost(), 75.0);
        assert_eq!(Tech::ExpeditionSystems.storage_cost(), 30.0);
        assert_eq!(Tech::ExpeditionSystems.label(), "Expedition Systems");
    }

    // --- Migrants, scenarios, death ---

    #[test]
    fn test_migrant_suit_air_band() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        for _ in 0..50 {
            let air = migrant_suit_air(&mut rng);
            assert!(
                (140.0..=170.0).contains(&air),
                "air {air} outside transit band"
            );
        }
    }

    #[test]
    fn test_death_returns_colony_suit_to_pool() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            eva_suits: 0.0,
            max_eva_suits: 50.0,
            ..Default::default()
        });
        let (pop, suit) = spawn_suited_pop(&mut world, 30.0, SuitClass::Base, 3, 3);
        world.entity_mut(pop).insert(crate::layer1::health::Dead);
        world.run_system_once(suit_death_recovery_system).unwrap();
        world.flush();
        assert_eq!(world.resource::<ColonyResources>().eva_suits, 1.0);
        assert!(
            world.get_entity(suit).is_err(),
            "pooled suit entity consumed"
        );
    }

    #[test]
    fn test_death_leaves_expedition_suit_loose() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        let suit = world
            .spawn((
                Item::default(),
                EvaSuit {
                    class: SuitClass::Expedition,
                    air: 141.0,
                    durability: 90.0,
                    max_durability: 100.0,
                    colony_issued: false,
                },
            ))
            .id();
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 7, y: 8 },
                Health::default(),
                PopName("Bryn".to_string()),
                Equipment {
                    body: Some(suit),
                    ..Default::default()
                },
                crate::layer1::health::Dead,
            ))
            .id();
        let _ = pop;
        world.run_system_once(suit_death_recovery_system).unwrap();
        world.flush();
        let suit_comp = world
            .get::<EvaSuit>(suit)
            .expect("expedition suit survives death");
        assert_eq!(suit_comp.air, 141.0);
        let pos = world
            .get::<GridPosition>(suit)
            .expect("loose suit anchored at death tile");
        assert_eq!((pos.x, pos.y), (7, 8));
        assert_eq!(world.resource::<ColonyResources>().eva_suits, 0.0);
    }

    #[test]
    fn test_panicked_pop_is_never_assigned_fetch_suit() {
        // The utility AI's pop collection excludes Panic pops entirely
        // (panic_spirals contract), so a panicking pop is never even
        // evaluated for FetchSuit — it will not stop to dress.
        use crate::layer1::mind::utility_ai_population::collect_pop_data;
        use crate::layer1::psychology::panic_spirals::Panic;
        use crate::layer1::utility_eval_types::UtilityAIBuffer;
        use crate::layer1::utility_types::{ActionType, PopAction, UtilityConfig};
        let mut world = World::new();
        let mut rng = rand::thread_rng();
        let mut bundle = crate::layer1::pop::PopBundle::random(0, 0, &mut rng);
        bundle.action = PopAction {
            current: ActionType::Idle,
            current_utility: 0.0,
            ticks_committed: 10,
        };
        let panicked = world
            .spawn((bundle, Panic::new(GridPosition { x: 0, y: 0 })))
            .id();
        let mut bundle2 = crate::layer1::pop::PopBundle::random(1, 1, &mut rng);
        bundle2.action = PopAction {
            current: ActionType::Idle,
            current_utility: 0.0,
            ticks_committed: 10,
        };
        let calm = world.spawn(bundle2).id();
        let mut buffer = UtilityAIBuffer::default();
        collect_pop_data(&mut world, &mut buffer, &UtilityConfig::default());
        assert!(
            buffer.pop_data.iter().all(|d| d.entity != panicked),
            "panicked pop must be excluded from utility evaluation"
        );
        assert!(
            buffer.pop_data.iter().any(|d| d.entity == calm),
            "calm pop is evaluated (control)"
        );
    }

    #[test]
    fn test_scenario_suit_grants() {
        // Spec 1384 §3/§4.8: Classic 3 pool/Base floor, Ground
        // Survival 2/Base, Social Drama 3/Base, Layer 2 Ready 6 pool
        // + Mk II floor.
        use crate::setup::{setup_world_with_config, SetupConfig};
        use crate::shared::scenario::StartScenarioId;
        for (scenario, expected_pool) in [
            (StartScenarioId::Classic, 3.0),
            (StartScenarioId::GroundSurvival, 2.0),
            (StartScenarioId::SocialDrama, 3.0),
            (StartScenarioId::Layer2Ready, 6.0),
        ] {
            let world = setup_world_with_config(SetupConfig {
                headless: true,
                scenario,
            });
            let pool = world.get_resource::<ColonyResources>().unwrap().eva_suits;
            assert_eq!(pool, expected_pool, "scenario {scenario:?} pool");
            let floor = world.get_resource::<ScenarioSuitFloor>().unwrap().0;
            let expected_floor = if scenario == StartScenarioId::Layer2Ready {
                SuitClass::Mk2
            } else {
                SuitClass::Base
            };
            assert_eq!(floor, expected_floor, "scenario {scenario:?} floor");
        }
    }

    #[test]
    fn test_save_name_round_trips_all_classes() {
        // Canonical save names are lowercase (persistence convention);
        // parsing is case-insensitive so older capitalized saves load.
        for class in [SuitClass::Base, SuitClass::Mk2, SuitClass::Expedition] {
            assert_eq!(SuitClass::from_save_name(class.save_name()), Some(class));
        }
        assert_eq!(SuitClass::from_save_name("Mk2"), Some(SuitClass::Mk2));
        assert_eq!(
            SuitClass::from_save_name("Expedition"),
            Some(SuitClass::Expedition)
        );
        assert_eq!(SuitClass::from_save_name("nonsense"), None);
    }

    #[test]
    fn test_ip_guard_no_borrowed_names() {
        // Concept-only (IP rule): public suit strings must be original.
        let banned = [
            "biosuit", "eva-01", "nasa", "spacex", "ripley", "gordon", "shepard",
        ];
        let haystacks = [
            SuitClass::Base.label(),
            SuitClass::Mk2.label(),
            SuitClass::Expedition.label(),
            Tech::ClosedCycleBreathing.label(),
            Tech::ClosedCycleBreathing.description(),
            Tech::ExpeditionSystems.label(),
            Tech::ExpeditionSystems.description(),
        ];
        for text in haystacks {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(!lower.contains(term), "banned term {term} in {text}");
            }
        }
    }

    #[test]
    fn test_schedule_suited_pop_survives_30_ticks_of_vacuum() {
        use crate::setup::{setup_world_with_config, SetupConfig};
        use crate::simulation::run_simulation_tick;
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: crate::shared::scenario::StartScenarioId::Classic,
        });
        // Classic grant check rides along: pool starts at 3.
        assert_eq!(world.resource::<ColonyResources>().eva_suits, 3.0);
        let mut rng = rand::thread_rng();
        // Suited pop far outside the habitat envelope.
        let suit = world
            .spawn((
                Item::default(),
                EvaSuit {
                    class: SuitClass::Base,
                    air: 50.0,
                    durability: 100.0,
                    max_durability: 100.0,
                    colony_issued: true,
                },
                GridPosition { x: 2, y: 2 },
            ))
            .id();
        let mut suited_bundle = crate::layer1::pop::PopBundle::random(2, 2, &mut rng);
        suited_bundle.equipment.body = Some(suit);
        let suited = world.spawn(suited_bundle).id();
        let control_bundle = crate::layer1::pop::PopBundle::random(3, 2, &mut rng);
        let control = world.spawn(control_bundle).id();
        for _ in 0..30 {
            run_simulation_tick(&mut world);
        }
        let suited_hp = world.get::<Health>(suited).map_or(0.0, |h| h.current);
        let control_hp = world.get::<Health>(control).map_or(0.0, |h| h.current);
        let air = world.get::<EvaSuit>(suit).map_or(50.0, |s| s.air);
        assert!(
            air < 49.0,
            "schedule must drain suit air in vacuum, air={air}"
        );
        assert!(
            suited_hp >= 99.0,
            "suited pop protected by bottle, hp={suited_hp}"
        );
        assert!(
            control_hp < suited_hp,
            "control pop must fare worse: {control_hp} vs {suited_hp}"
        );
        assert!(
            control_hp <= 85.0,
            "control pop suffocates ~1 HP/tick, hp={control_hp}"
        );
    }
}
