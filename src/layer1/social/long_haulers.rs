//! The Long-Haulers (Spec 351).
//!
//! Your colony is just a stopover for some. Not everyone calls this rock
//! home: some pops are **Long-Haulers** — temporary colonists who work,
//! learn, and contribute like everyone else, but who signed on for a
//! fixed tour. Each carries a [`Traveler`] component with the tick they
//! are due to leave. When a passenger-capable ship arrives at or after
//! that tick, they board it and leave the colony permanently, taking
//! their accumulated skills with them.
//!
//! ## Tension
//!
//! Invest in a brilliant worker who is guaranteed to leave, or build
//! around the permanent but less skilled residents? The colony feels
//! every departure — most of all when the departing hauler was good.
//!
//! ## Design notes (spec adaptation)
//!
//! * The spec's RED phase names a mock `ShipArrivalEvent { ship_type }`
//!   in `layer2::trade`. The real codebase has exactly one ship-arrival
//!   event: [`TradeShipArrivalEvent`] (`cargo_value`, `faction`) in
//!   `layer2::trade::blockade`, fired by the colony beacon and consumed
//!   by the stowaway / merchant bridges. It carries no ship-type field,
//!   and every trade ship that reaches this colony demonstrably carries
//!   people (migrants, stowaways, merchants ride the same hulls), so
//!   **every `TradeShipArrivalEvent` is passenger-capable** here. If a
//!   future ship-type field lands, gate the departure query on it.
//! * The spec's `crate::layer1::time::SimulationTime` is really
//!   `crate::shared::time::SimulationTime`, and `PopBundle` lives in
//!   `crate::layer1::pop` (re-exported from `entities::pop`).
//! * Departure despawns the pop entity outright. Everything a pop owns
//!   that matters (job, action, equipment, wallet) is a component *on*
//!   the pop, so despawning is the cleanup — there is no separate bed /
//!   job registry holding a bare `Entity` that outlives the pop in this
//!   codebase (assignments are `AssignedTo` components on the pop).
//! * REFACTOR beats taken: a skilled departure writes a **Major**
//!   chronicle entry naming the skill the colony lost; an ordinary
//!   departure writes a Minor one. A [`TravelerDepartedEvent`] is also
//!   emitted for any future mood / relationship hooks. Detention and
//!   pre-departure UI warnings are left alone (future work).

use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::pop::{Pop, PopBundle, PopName};
use crate::layer1::skills::{SkillType, Skills};
use crate::layer1::social::old_guard::Arrival;
use crate::layer2::trade::blockade::TradeShipArrivalEvent;
use crate::shared::time::SimulationTime;

/// Default tour length for a Long-Hauler, in ticks, when an arrival
/// event does not specify one.
pub const DEFAULT_STAY_TICKS: u64 = 600;

/// A skill level at (or above) which a departing hauler counts as
/// "highly skilled" for chronicle purposes. Level = floor(sqrt(xp/100)),
/// so level 3 means 900+ XP in one craft — a real investment walking
/// out of the airlock.
pub const SKILLED_LEVEL_THRESHOLD: u32 = 3;

/// Marker for a temporary colonist: a normal working pop who will
/// board the first passenger-capable ship arriving at or after
/// [`Self::departure_tick`] and leave the colony permanently.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Traveler {
    /// Tick at/after which this pop is due to depart.
    pub departure_tick: u64,
}

/// Event: a batch of Long-Haulers has signed on for a tour.
#[derive(Event, Debug, Clone)]
pub struct TravelerArrivalEvent {
    /// How many haulers are coming down.
    pub count: u32,
    /// How many ticks they will stay before they are due to leave.
    pub stay_ticks: u64,
}

/// Event: a Long-Hauler has boarded an outbound ship and left.
#[derive(Event, Debug, Clone)]
pub struct TravelerDepartedEvent {
    /// The (now despawned) pop entity.
    pub entity: Entity,
    /// The departing pop's name.
    pub name: String,
    /// Tick on which they departed.
    pub tick: u64,
    /// Whether they counted as highly skilled (Major chronicle).
    pub skilled: bool,
}

/// The hauler's best skill and its level, if they have any XP at all.
#[must_use]
pub fn best_skill(skills: &Skills) -> Option<(SkillType, u32)> {
    [
        SkillType::Mining,
        SkillType::Forestry,
        SkillType::Farming,
        SkillType::Construction,
        SkillType::Crafting,
        SkillType::Husbandry,
        SkillType::Engineering,
        SkillType::Science,
    ]
    .into_iter()
    .map(|s| (s, skills.get_level(s)))
    .filter(|(_, level)| *level > 0)
    .max_by_key(|(_, level)| *level)
}

/// Whether this skill set makes a departure sting: one craft mastered
/// to [`SKILLED_LEVEL_THRESHOLD`], per the spec's REFACTOR note.
#[must_use]
pub fn is_skilled(skills: Option<&Skills>) -> bool {
    skills.is_some_and(|s| best_skill(s).is_some_and(|(_, level)| level >= SKILLED_LEVEL_THRESHOLD))
}

/// Spawns Long-Haulers: full pops (real workers — no skill or trait
/// overrides; that is the whole point of the tension) carrying a
/// [`Traveler`] due-date, landing at colony tiles like other arrivals.
pub fn process_traveler_arrival_system(
    mut commands: Commands,
    mut events: EventReader<TravelerArrivalEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    sim_time: Option<Res<SimulationTime>>,
    pop_positions: Query<&GridPosition, With<Pop>>,
) {
    let tick = sim_time.map_or(0, |t| t.tick);
    // Haulers land where the colony already is, not in the vacuum.
    let colony_tiles: Vec<GridPosition> = pop_positions.iter().copied().collect();

    for event in events.read() {
        let mut rng = rand::thread_rng();

        for _ in 0..event.count {
            let spawn_pos = colony_tiles
                .choose(&mut rng)
                .copied()
                .unwrap_or(GridPosition { x: 0, y: 0 });

            let mut bundle = PopBundle::random(spawn_pos.x, spawn_pos.y, &mut rng);
            bundle.arrival = Arrival { tick };

            commands.spawn((
                bundle,
                Traveler {
                    departure_tick: tick.saturating_add(event.stay_ticks),
                },
            ));
        }

        chronicle.send(AddChronicleEvent {
            text: format!(
                "Long-Haulers have signed on: {} temporary colonists will work the colony for a fixed tour, then ship out for good.",
                event.count
            ),
            importance: EventImportance::Standard,
        });
    }
}

/// The departure gate (Spec 351 core): when a passenger-capable ship
/// arrives (any [`TradeShipArrivalEvent`] — see module docs), every
/// [`Traveler`] whose `departure_tick` has passed boards it and is
/// despawned permanently. Travelers not yet due, and travelers with no
/// ship to catch, stay and keep working.
#[allow(clippy::type_complexity)]
pub fn process_traveler_departures_system(
    mut commands: Commands,
    time: Option<Res<SimulationTime>>,
    mut ship_events: EventReader<TradeShipArrivalEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut departed: EventWriter<TravelerDepartedEvent>,
    traveler_query: Query<(Entity, &Traveler, Option<&PopName>, Option<&Skills>), With<Pop>>,
) {
    // Drain the queue first: only sail if a ship actually arrived.
    let ship_arrived = ship_events.read().count() > 0;
    if !ship_arrived {
        return;
    }
    let tick = time.map_or(0, |t| t.tick);

    for (entity, traveler, name, skills) in traveler_query.iter() {
        if tick < traveler.departure_tick {
            continue;
        }
        let pop_name = name.map_or_else(|| "A Long-Hauler".to_string(), |n| n.0.clone());
        let skilled = is_skilled(skills);
        if skilled {
            // best_skill is Some by construction when skilled.
            let (skill, level) = best_skill(skills.expect("skilled implies skills present"))
                .expect("skilled implies a best skill");
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "{pop_name} has shipped out for good. The colony loses a level-{level} {skill:?} hand — the Long-Haulers keep their own schedule."
                ),
                importance: EventImportance::Major,
            });
        } else {
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "{pop_name} has shipped out — a Long-Hauler whose tour was up, gone with the outbound ship."
                ),
                importance: EventImportance::Minor,
            });
        }
        departed.send(TravelerDepartedEvent {
            entity,
            name: pop_name,
            tick,
            skilled,
        });
        commands.entity(entity).despawn();
    }
}

/// Counts living Long-Haulers (headless STATS).
pub fn traveler_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<Entity, With<Traveler>>();
    query.iter(world).count()
}

/// Counts Long-Haulers whose tour is up and who are only waiting on a
/// ship (headless `travelers` panel).
pub fn traveler_due_count(world: &mut World) -> usize {
    let tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);
    let mut query = world.query::<&Traveler>();
    query
        .iter(world)
        .filter(|t| tick >= t.departure_tick)
        .count()
}

/// Public-facing strings, for the IP-guard test.
pub fn long_hauler_public_strings() -> Vec<String> {
    vec![
        "Long-Haulers have signed on: temporary colonists will work the colony for a fixed tour, then ship out for good.".to_string(),
        "has shipped out for good. The colony loses a skilled hand — the Long-Haulers keep their own schedule.".to_string(),
        "has shipped out — a Long-Hauler whose tour was up, gone with the outbound ship.".to_string(),
        "Long-Hauler".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 1000,
            ..Default::default()
        });
        world.insert_resource(Events::<TradeShipArrivalEvent>::default());
        world.insert_resource(Events::<TravelerArrivalEvent>::default());
        world.insert_resource(Events::<TravelerDepartedEvent>::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world
    }

    fn ship_arrives(world: &mut World) {
        world
            .resource_mut::<Events<TradeShipArrivalEvent>>()
            .send(TradeShipArrivalEvent {
                cargo_value: 1000.0,
                faction: "Independent Merchants".to_string(),
            });
    }

    fn run_departures(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(process_traveler_departures_system);
        schedule.run(world);
    }

    fn spawn_traveler(world: &mut World, departure_tick: u64) -> Entity {
        world
            .spawn((
                Pop,
                PopName("Test Hauler".to_string()),
                Skills::default(),
                Traveler { departure_tick },
            ))
            .id()
    }

    // --- Spec 351 RED phase, adapted to the real types ---

    #[test]
    fn test_traveler_departs_when_ship_arrives() {
        let mut world = setup();
        let pop = spawn_traveler(&mut world, 950); // due date has passed
        ship_arrives(&mut world);
        run_departures(&mut world);
        assert!(
            world.get_entity(pop).is_err(),
            "due Traveler should be gone once a ship arrives"
        );
    }

    #[test]
    fn test_traveler_stays_if_departure_date_not_reached() {
        let mut world = setup();
        let pop = spawn_traveler(&mut world, 1500); // still on tour
        ship_arrives(&mut world);
        run_departures(&mut world);
        assert!(world.get_entity(pop).is_ok(), "Traveler on tour stays");
    }

    #[test]
    fn test_traveler_stays_if_no_ship_arrives() {
        let mut world = setup();
        let pop = spawn_traveler(&mut world, 950); // due, but no ship
        run_departures(&mut world);
        assert!(world.get_entity(pop).is_ok(), "no ship, no departure");
    }

    #[test]
    fn test_traveler_departs_exactly_at_departure_tick() {
        let mut world = setup();
        let pop = spawn_traveler(&mut world, 1000); // due this very tick
        ship_arrives(&mut world);
        run_departures(&mut world);
        assert!(
            world.get_entity(pop).is_err(),
            "at/after departure_tick departs"
        );
    }

    #[test]
    fn test_permanent_pop_never_departs() {
        let mut world = setup();
        let resident = world
            .spawn((Pop, PopName("Lifer".to_string()), Skills::default()))
            .id();
        let hauler = spawn_traveler(&mut world, 950);
        ship_arrives(&mut world);
        run_departures(&mut world);
        assert!(
            world.get_entity(resident).is_ok(),
            "residents are not haulers"
        );
        assert!(world.get_entity(hauler).is_err());
    }

    #[test]
    fn test_departure_emits_event_and_chronicle() {
        let mut world = setup();
        let pop = spawn_traveler(&mut world, 950);
        ship_arrives(&mut world);
        run_departures(&mut world);
        assert!(world.get_entity(pop).is_err());

        let departed = world.resource::<Events<TravelerDepartedEvent>>();
        let mut reader = departed.get_cursor();
        let events: Vec<_> = reader.read(departed).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "Test Hauler");
        assert!(!events[0].skilled);

        let chronicle = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle.get_cursor();
        let entries: Vec<_> = reader.read(chronicle).collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].text.contains("Test Hauler"));
        assert_eq!(entries[0].importance, EventImportance::Minor);
    }

    #[test]
    fn test_skilled_departure_is_major_chronicle() {
        let mut world = setup();
        // Level 3 in Mining = 900 XP.
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Mining, 900.0);
        let pop = world
            .spawn((
                Pop,
                PopName("Master Miner".to_string()),
                skills,
                Traveler {
                    departure_tick: 950,
                },
            ))
            .id();
        ship_arrives(&mut world);
        run_departures(&mut world);
        assert!(world.get_entity(pop).is_err());

        let departed = world.resource::<Events<TravelerDepartedEvent>>();
        let mut reader = departed.get_cursor();
        let events: Vec<_> = reader.read(departed).collect();
        assert!(events[0].skilled, "level-3 hand counts as skilled");

        let chronicle = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle.get_cursor();
        let entries: Vec<_> = reader.read(chronicle).collect();
        assert_eq!(entries[0].importance, EventImportance::Major);
        assert!(
            entries[0].text.contains("Mining"),
            "chronicle names the lost craft: {}",
            entries[0].text
        );
    }

    #[test]
    fn test_arrival_spawns_full_worker_pops() {
        use crate::layer1::psychology::needs::Needs;

        let mut world = setup();
        // One existing pop so haulers have a colony tile to land on.
        world.spawn((Pop, GridPosition { x: 10, y: 10 }));

        world
            .resource_mut::<Events<TravelerArrivalEvent>>()
            .send(TravelerArrivalEvent {
                count: 3,
                stay_ticks: 250,
            });
        let mut schedule = Schedule::default();
        schedule.add_systems(process_traveler_arrival_system);
        schedule.run(&mut world);

        let mut query = world.query_filtered::<(&PopName, &Needs, &Traveler), With<Pop>>();
        let rows: Vec<_> = query.iter(&world).collect();
        assert_eq!(rows.len(), 3, "arrival spawns full pops, not ghosts");
        for (_, _, traveler) in &rows {
            assert_eq!(traveler.departure_tick, 1250, "due = arrival tick + stay");
        }
        assert_eq!(traveler_count(&mut world), 3);
        assert_eq!(traveler_due_count(&mut world), 0);
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Spec 351 is concept-only: generic long-haul language, nothing
        // lifted from outside fiction.
        let banned = [
            "sad king billy",
            "windsor",
            "dune",
            "arrakis",
            "hitchhiker",
            "foundation",
            "psychohistory",
            "area x",
            "shimmer",
        ];
        for s in long_hauler_public_strings() {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(
                    !lower.contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
