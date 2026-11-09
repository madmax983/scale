//! Spec 408: Galactic Tourism — beautiful colonies attract visitor ships.
//!
//! # Grounding audit (spec §8 adaptation pattern, specs 360–407)
//!
//! * `layer1::visitor::{Visitor, VisitorSpawning}` (spec sketch) —
//!   **PARTLY MOCK.** No `VisitorSpawning` exists and the real module is
//!   `layer1::entities::visitor`: [`Visitor`] + [`VisitorSource`] are
//!   REAL (spawn/lifecycle/behavior systems; visitors deliberately spawn
//!   WITHOUT `UtilityWeights`, so the main utility AI never assigns them
//!   colony jobs — spec §7's requirement is already the real seam's
//!   behavior). Tourists are real visitor entities carrying [`Visitor`]
//!   plus the new [`Tourist`] component; pending arrivals live in the
//!   new [`TourismState`] ledger instead of a parallel spawning system.
//! * `layer1::colony::ColonyStats { global_beauty, credits }` — **MOCK,
//!   does not exist.** The real `ColonyStats` (`economy::black_market`)
//!   carries only `unmet_luxury` + `corruption`. The real beauty seam is
//!   the spatial [`BeautyGrid`]; the colony beauty score is its tile sum
//!   ([`global_beauty`]). The real credits ledger is
//!   [`ColonyResources::credits`] with `add_credits` — arrival payments
//!   land there, never in a mock stats struct.
//! * Food consumption — **REAL** via `ColonyResources::food`, the same
//!   stockpile every pop eats from. Tourists eat from it on a cadence;
//!   that shared stockpile IS the spec §1 dilemma: when it runs dry
//!   while a liner is docked, the [`TourismPolicy`] choice bites —
//!   `DenyVisitors` starves the tourists (early departure + reputation
//!   penalty, boom ends) while `FeedVisitors` keeps feeding them first.
//! * Quarantine screening (§5) — **REAL seam, cheap leg taken.**
//!   Regular immigrants arrive as `ImmigrationStatus::Pending`
//!   (screened); tourists bypass screening entirely (no
//!   `ImmigrationStatus` is ever attached) and a deterministic fraction
//!   arrive carrying the real [`MindSporeInfection`] — the spec 406
//!   quarantine zones are then the colony's only containment tool.
//! * Morale/reputation — reputation is a tourism-local ledger in
//!   [`TourismState`] (no colony-wide reputation resource exists to
//!   reuse); needs cratering uses the real [`Needs`] component visitors
//!   already carry.
//!
//! # New constants (no existing balance constants were retuned)
//!
//! * [`BEAUTY_THRESHOLD`] `300.0` (spec §4), [`TOURIST_WEALTH`] `500.0`
//!   (spec RED test), [`TOURIST_STAY_TICKS`] `800`, [`EAT_INTERVAL`]
//!   `100`, [`EAT_AMOUNT`] `1.0`, [`REP_PENALTY_EARLY_DEPARTURE`] `0.25`,
//!   [`DISEASE_CARRIER_FRACTION`] `0.10`.

use crate::layer1::beauty::BeautyGrid;
use crate::layer1::biology::symbiotic_insurgency::MindSporeInfection;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::entities::visitor::{Visitor, VisitorSource, VisitorState};
use crate::layer1::map::GridPosition;
use crate::layer1::pop::PopName;
use crate::layer1::psychology::needs::Needs;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Beauty score (sum of `BeautyGrid` tiles) at or above which visitor
/// ships start queueing tourists (spec §4 threshold).
pub const BEAUTY_THRESHOLD: f32 = 300.0;
/// Credits a tourist pays the colony on landing (spec RED test wealth).
pub const TOURIST_WEALTH: f32 = 500.0;
/// How long a tourist stays before departing normally.
pub const TOURIST_STAY_TICKS: u64 = 800;
/// Ticks between tourist meals.
pub const EAT_INTERVAL: u64 = 100;
/// Food drawn from the shared stockpile per tourist meal.
pub const EAT_AMOUNT: f32 = 1.0;
/// Hunger restored by one tourist meal.
pub const EAT_HUNGER_GAIN: f32 = 0.5;
/// Hunger lost when a meal is missed (no food, or policy denies it).
pub const MISSED_MEAL_HUNGER_LOSS: f32 = 0.3;
/// Reputation lost per early (needs-cratered) departure.
pub const REP_PENALTY_EARLY_DEPARTURE: f32 = 0.25;
/// Fraction of tourists who bypass screening carrying a mind-spore
/// infection (deterministic per-arrival roll, see
/// [`tourist_carries_disease`]).
pub const DISEASE_CARRIER_FRACTION: f32 = 0.10;
/// Ticks between attraction evaluations.
pub const ATTRACTION_INTERVAL: u64 = 200;
/// Player-facing chronicle text when the first liner of a boom docks.
pub const TOURISM_BOOM_TEXT: &str =
    "A visitor liner has docked. Word of the colony's beauty is spreading.";
/// Player-facing chronicle text when a tourist leaves early, unhappy.
pub const TOURISM_EARLY_DEPARTURE_TEXT: &str =
    "A visitor has departed early, unimpressed. Word travels fast out there.";
/// Player-facing chronicle text when a tourist is found carrying spores.
pub const TOURISM_DISEASE_TEXT: &str =
    "A visitor who bypassed screening has been found carrying spores.";

/// The colony's food policy toward docked tourists (spec §1 dilemma).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TourismPolicy {
    /// Tourists eat from the shared stockpile like everyone else —
    /// workers may go hungry so the rich do not.
    #[default]
    FeedVisitors,
    /// Tourists are denied food: they crater, leave early, and the
    /// reputation hit ends the boom.
    DenyVisitors,
}

/// A paying guest. Always paired with the real [`Visitor`] tag, so the
/// visitor lifecycle owns arrival/departure states and the main utility
/// AI (which visitors never join) never assigns colony jobs.
#[derive(Component, Debug, Clone, Copy)]
pub struct Tourist {
    /// Credits paid to the colony on landing.
    pub wealth: f32,
    /// Tick the tourist landed.
    pub arrival_tick: u64,
    /// Tick the tourist plans to leave.
    pub departure_tick: u64,
}

/// Fired when a tourist entity lands; the arrival system pays its
/// wealth into the real credits ledger.
#[derive(Event, Debug, Clone, Copy)]
pub struct TouristArrivalEvent {
    /// The tourist entity that landed.
    pub tourist: Entity,
}

/// Tourism ledger + policy (first-tick pattern: initialized in
/// `simulation.rs`).
#[derive(Resource, Debug)]
pub struct TourismState {
    /// Tourists queued by attraction, waiting to land.
    pub pending_tourists: u32,
    /// Lifetime tourists landed.
    pub total_arrived: u32,
    /// Lifetime tourists who left early with cratered needs.
    pub total_departed_early: u32,
    /// Lifetime tourists who completed their stay.
    pub total_departed: u32,
    /// Colony tourism reputation, 0.0..=1.0. Below 0.5 the boom ends.
    pub reputation: f32,
    /// Whether a tourism boom is currently active.
    pub boom_active: bool,
    /// Lifetime meals denied by policy or an empty stockpile.
    pub denied_meals: u32,
    /// Lifetime tourists found carrying disease past screening.
    pub disease_imports: u32,
    /// Lifetime credits earned from tourism.
    pub credits_earned: f32,
    /// Last tick attraction was evaluated (cadence bookkeeping).
    pub last_attraction_tick: u64,
    /// Current food policy toward tourists.
    pub policy: TourismPolicy,
}

impl Default for TourismState {
    fn default() -> Self {
        Self {
            pending_tourists: 0,
            total_arrived: 0,
            total_departed_early: 0,
            total_departed: 0,
            reputation: 1.0,
            boom_active: false,
            denied_meals: 0,
            disease_imports: 0,
            credits_earned: 0.0,
            last_attraction_tick: 0,
            policy: TourismPolicy::FeedVisitors,
        }
    }
}

/// The colony beauty score: sum of all `BeautyGrid` tiles. A handful of
/// statues (10.0 each) plus gardens clears the spec threshold; a bare
/// dirt landing pad does not.
#[must_use]
pub fn global_beauty(grid: &BeautyGrid) -> f32 {
    grid.values.iter().sum()
}

/// Deterministic disease roll for the screening bypass: `roll` in
/// 0.0..1.0 carries iff it falls under the carrier fraction.
#[must_use]
pub fn tourist_carries_disease(roll: f32) -> bool {
    roll < DISEASE_CARRIER_FRACTION
}

/// Spec §6: high beauty queues tourists. Evaluated on a cadence; each
/// evaluation queues one tourist per full threshold multiple, capped,
/// and only while reputation holds the boom together.
pub fn evaluate_tourist_attraction_system(
    grid: Option<Res<BeautyGrid>>,
    time: Res<SimulationTime>,
    mut state: ResMut<TourismState>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    let Some(grid) = grid else { return };
    if time.tick < state.last_attraction_tick + ATTRACTION_INTERVAL {
        return;
    }
    state.last_attraction_tick = time.tick;
    if state.reputation < 0.5 {
        state.boom_active = false;
        return;
    }
    let beauty = global_beauty(&grid);
    if beauty < BEAUTY_THRESHOLD {
        return;
    }
    let queued = ((beauty / BEAUTY_THRESHOLD).floor() as u32).clamp(1, 5);
    state.pending_tourists += queued;
    if !state.boom_active {
        state.boom_active = true;
        if let Some(events) = chronicle.as_deref_mut() {
            events.send(AddChronicleEvent {
                text: TOURISM_BOOM_TEXT.to_string(),
                importance: EventImportance::Major,
            });
        }
    }
}

/// Lands queued tourists as real visitor entities and fires
/// [`TouristArrivalEvent`] for each. Tourists spawn with `Visitor` +
/// `Needs` + `PopName` (the real visitor bundle shape) and NO
/// `ImmigrationStatus` — the screening bypass of spec §5.
pub fn spawn_tourist_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    source: Option<Res<VisitorSource>>,
    mut state: ResMut<TourismState>,
    mut arrivals: EventWriter<TouristArrivalEvent>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    if state.pending_tourists == 0 {
        return;
    }
    let spawn_pos = source
        .as_deref()
        .and_then(|s| s.spawn_points.first().copied())
        .unwrap_or(GridPosition { x: 0, y: 0 });
    let mut rng = rand::thread_rng();
    while state.pending_tourists > 0 {
        state.pending_tourists -= 1;
        let entity = commands
            .spawn((
                Visitor {
                    state: VisitorState::Arriving,
                    arrival_tick: time.tick,
                    departure_tick: time.tick + TOURIST_STAY_TICKS,
                },
                Tourist {
                    wealth: TOURIST_WEALTH,
                    arrival_tick: time.tick,
                    departure_tick: time.tick + TOURIST_STAY_TICKS,
                },
                spawn_pos,
                PopName::random(&mut rng),
                Needs::default(),
            ))
            .id();
        arrivals.send(TouristArrivalEvent { tourist: entity });
        // Screening bypass: a fraction slip through carrying spores.
        if tourist_carries_disease(rng.gen::<f32>()) {
            commands
                .entity(entity)
                .insert(MindSporeInfection { active: true });
            state.disease_imports += 1;
            if let Some(events) = chronicle.as_deref_mut() {
                events.send(AddChronicleEvent {
                    text: TOURISM_DISEASE_TEXT.to_string(),
                    importance: EventImportance::Major,
                });
            }
        }
    }
}

/// Spec RED test 2, bridged: arrival pays the tourist's wealth into
/// the real `ColonyResources` credits ledger.
pub fn process_tourist_arrival_system(
    mut events: EventReader<TouristArrivalEvent>,
    tourists: Query<&Tourist>,
    mut resources: ResMut<ColonyResources>,
    mut state: ResMut<TourismState>,
) {
    for event in events.read() {
        if let Ok(tourist) = tourists.get(event.tourist) {
            resources.add_credits(tourist.wealth);
            state.credits_earned += tourist.wealth;
            state.total_arrived += 1;
        }
    }
}

/// Tourists eat from the shared stockpile on a cadence. Under
/// `DenyVisitors`, or when the stockpile is empty, the meal is missed
/// and hunger craters — the feed-the-rich dilemma of spec §1.
pub fn tourist_consumption_system(
    time: Res<SimulationTime>,
    mut resources: ResMut<ColonyResources>,
    mut state: ResMut<TourismState>,
    mut tourists: Query<&mut Needs, With<Tourist>>,
) {
    if time.tick == 0 || time.tick % EAT_INTERVAL != 0 {
        return;
    }
    for mut needs in &mut tourists {
        let fed = state.policy == TourismPolicy::FeedVisitors && resources.food >= EAT_AMOUNT;
        if fed {
            resources.food -= EAT_AMOUNT;
            needs.hunger = (needs.hunger + EAT_HUNGER_GAIN).min(1.0);
        } else {
            needs.hunger = (needs.hunger - MISSED_MEAL_HUNGER_LOSS).max(0.0);
            state.denied_meals += 1;
        }
    }
}

/// Spec §5 anger leg: a tourist whose worst need craters departs early
/// and costs reputation (ending the boom under 0.5); a tourist who
/// reaches their departure tick leaves satisfied.
pub fn tourist_departure_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut state: ResMut<TourismState>,
    tourists: Query<(Entity, &Tourist, &Needs)>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    for (entity, tourist, needs) in &tourists {
        if needs.worst() < 0.2 {
            state.total_departed_early += 1;
            state.reputation = (state.reputation - REP_PENALTY_EARLY_DEPARTURE).max(0.0);
            if state.reputation < 0.5 {
                state.boom_active = false;
            }
            if let Some(events) = chronicle.as_deref_mut() {
                events.send(AddChronicleEvent {
                    text: TOURISM_EARLY_DEPARTURE_TEXT.to_string(),
                    importance: EventImportance::Major,
                });
            }
            commands.entity(entity).despawn();
        } else if time.tick >= tourist.departure_tick {
            state.total_departed += 1;
            commands.entity(entity).despawn();
        }
    }
}

/// Census for the headless panel / STATS.
#[derive(Debug, Clone, Copy, Default)]
pub struct TourismCensus {
    /// Tourists currently in the colony.
    pub present: usize,
    /// Tourists queued to land.
    pub pending: u32,
    /// Current reputation.
    pub reputation: f32,
    /// Lifetime credits earned.
    pub credits_earned: f32,
    /// Lifetime early departures.
    pub departed_early: u32,
    /// Lifetime disease imports.
    pub disease_imports: u32,
}

/// Read the tourism census from a raw world (headless helper pattern).
pub fn tourism_census(world: &mut World) -> TourismCensus {
    let present = {
        let mut q = world.query::<&Tourist>();
        q.iter(world).count()
    };
    let (pending, reputation, credits_earned, departed_early, disease_imports) = world
        .get_resource::<TourismState>()
        .map_or((0, 1.0, 0.0, 0, 0), |s| {
            (
                s.pending_tourists,
                s.reputation,
                s.credits_earned,
                s.total_departed_early,
                s.disease_imports,
            )
        });
    TourismCensus {
        present,
        pending,
        reputation,
        credits_earned,
        departed_early,
        disease_imports,
    }
}

/// Compact STATS label: `present/pending/rep`.
pub fn tourism_stats_label(world: &mut World) -> String {
    let c = tourism_census(world);
    format!("{}/{}/{:.2}", c.present, c.pending, c.reputation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn beauty_grid_with_total(total: f32) -> BeautyGrid {
        let mut grid = BeautyGrid::new(10, 10);
        grid.set(0, 0, total);
        grid
    }

    /// Spec RED test 1, adapted: `ColonyStats{global_beauty}` mock →
    /// real `BeautyGrid` sum; `VisitorSpawning` mock → `TourismState`.
    #[test]
    fn test_high_beauty_triggers_tourist_spawn() {
        let mut world = World::new();
        world.insert_resource(beauty_grid_with_total(500.0));
        world.insert_resource(SimulationTime {
            tick: 1000,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        // Pre-age the cadence so the first evaluation fires.
        world.resource_mut::<TourismState>().last_attraction_tick = 0;
        world.run_system_once(evaluate_tourist_attraction_system).unwrap();
        let state = world.resource::<TourismState>();
        assert!(state.pending_tourists > 0, "High beauty should queue tourists");
        assert!(state.boom_active);
    }

    #[test]
    fn test_low_beauty_queues_nothing() {
        let mut world = World::new();
        world.insert_resource(beauty_grid_with_total(50.0));
        world.insert_resource(SimulationTime {
            tick: 1000,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        world.run_system_once(evaluate_tourist_attraction_system).unwrap();
        assert_eq!(world.resource::<TourismState>().pending_tourists, 0);
    }

    /// Spec RED test 2, adapted: credits land in the real ledger.
    #[test]
    fn test_tourist_arrival_grants_credits() {
        let mut world = World::new();
        world.init_resource::<Events<TouristArrivalEvent>>();
        world.insert_resource(ColonyResources {
            credits: 100.0,
            max_credits: 1_000_000.0,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        let tourist_ent = world
            .spawn(Tourist {
                wealth: 500.0,
                arrival_tick: 0,
                departure_tick: 800,
            })
            .id();
        world
            .resource_mut::<Events<TouristArrivalEvent>>()
            .send(TouristArrivalEvent { tourist: tourist_ent });
        world.run_system_once(process_tourist_arrival_system).unwrap();
        assert_eq!(world.resource::<ColonyResources>().credits, 600.0);
        assert_eq!(world.resource::<TourismState>().total_arrived, 1);
    }

    #[test]
    fn test_spawned_tourist_carries_real_visitor_tag() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.init_resource::<Events<TouristArrivalEvent>>();
        world.init_resource::<TourismState>();
        world.resource_mut::<TourismState>().pending_tourists = 1;
        world.run_system_once(spawn_tourist_system).unwrap();
        let mut q = world.query::<(&Tourist, &Visitor, &Needs)>();
        assert_eq!(q.iter(&world).count(), 1, "tourist must be a real Visitor");
    }

    #[test]
    fn test_deny_policy_starves_tourist_and_costs_reputation() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 100,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 50.0,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        world.resource_mut::<TourismState>().policy = TourismPolicy::DenyVisitors;
        let ent = world
            .spawn((
                Tourist { wealth: 500.0, arrival_tick: 0, departure_tick: 800 },
                Needs { hunger: 0.4, rest: 0.9, leisure: 0.9, hygiene: 0.9 },
            ))
            .id();
        world.run_system_once(tourist_consumption_system).unwrap();
        assert_eq!(world.resource::<ColonyResources>().food, 50.0, "denied meal draws nothing");
        world.run_system_once(tourist_departure_system).unwrap();
        // hunger 0.4 - 0.3 = 0.1 < 0.2 → early departure + penalty.
        assert!(world.get::<Tourist>(ent).is_none());
        let state = world.resource::<TourismState>();
        assert_eq!(state.total_departed_early, 1);
        assert!((state.reputation - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn test_feed_policy_consumes_shared_food() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 100,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 10.0,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        world.spawn((
            Tourist { wealth: 500.0, arrival_tick: 0, departure_tick: 800 },
            Needs { hunger: 0.3, rest: 0.9, leisure: 0.9, hygiene: 0.9 },
        ));
        world.run_system_once(tourist_consumption_system).unwrap();
        assert_eq!(world.resource::<ColonyResources>().food, 9.0);
    }

    #[test]
    fn test_disease_roll_fraction() {
        assert!(tourist_carries_disease(0.0));
        assert!(tourist_carries_disease(0.09));
        assert!(!tourist_carries_disease(0.10));
        assert!(!tourist_carries_disease(0.99));
    }

    /// IP guard: every new player-facing string is original/generic.
    #[test]
    fn test_ip_guard_tourism_strings() {
        let banned = [
            "star wars", "star trek", "firefly", "serenity", "babylon 5",
            "mass effect", "citadel", "coruscant", "tatooine", "enterprise",
            "fhloston", "fifth element", "total recall", "mars",
        ];
        let strings = [TOURISM_BOOM_TEXT, TOURISM_EARLY_DEPARTURE_TEXT, TOURISM_DISEASE_TEXT];
        for s in strings {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(!lower.contains(b), "IP-guard: '{b}' in '{s}'");
            }
        }
    }
}
