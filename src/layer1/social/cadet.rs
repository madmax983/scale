//! The Cadet Branch (Spec 1208).
//!
//! Babysitting the Homeworld's noble scions: useless, exquisite, and
//! enormously well-funded. The colony accepts "Noble Scion" pops from the
//! Homeworld — they arrive with terrible stats (zero skills) and the Noble
//! trait (which bars them from all manual labor, enforced by the utility
//! AI), but each living scion pays a monthly Allowance into the colony's
//! credits, funded by their rich families. If a scion dies, the funding
//! stops on its own (the dead scion leaves the allowance query) and the
//! Homeworld's faction relation tanks.
//!
//! ## Tension
//!
//! Free money vs. incompetent, high-maintenance population: you build a
//! luxurious, safe playground for the idiots just to keep the funding
//! flowing, while the real workers live in squalor.
//!
//! ## Design notes
//!
//! * The spec's RED phase names `NobleScion { allowance, faction_id }` and
//!   mock `Economy` / `FactionRelations` types; the GREEN implementation
//!   adapts them to the real types: [`ColonyResources`] (credits) and the
//!   layer-3 [`FactionRelations`] (u32 faction ids, i32 scores from -100 to
//!   100).
//! * The spec's REFACTOR phase asks for a "Snob" trait that prevents manual
//!   labor: the [`Trait::Noble`] variant (Spec 263) already does exactly
//!   this — the utility AI skips all work and logistics evaluation for
//!   nobles — so scions carry `Trait::Noble` rather than a duplicate
//!   variant.
//! * Allowance is periodic (every [`ALLOWANCE_PERIOD_TICKS`] ticks,
//!   "monthly") and scaled by the scion's morale, per the REFACTOR phase.
//! * All names are original and generic ("the Homeworld", "Noble Scion");
//!   nothing is lifted from outside fiction (see the IP-guard test).

use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::pop::{Pop, PopBundle, PopDied};
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::skills::Skills;
use crate::layer1::social::morale::Morale;
use crate::layer1::social::old_guard::Arrival;
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::time::SimulationTime;

/// Faction id of the Homeworld in the layer-3 [`FactionRelations`] map.
///
/// External factions are keyed by u32; this id is reserved for the
/// scion-sending Homeworld.
pub const HOMEWORLD_FACTION_ID: u32 = 7;

/// Credits paid per living scion each allowance period.
pub const DEFAULT_SCION_ALLOWANCE: f32 = 50.0;

/// Ticks between allowance payouts ("monthly").
pub const ALLOWANCE_PERIOD_TICKS: u64 = 1000;

/// Relation points lost with the Homeworld when a scion dies.
pub const SCION_DEATH_RELATION_PENALTY: i32 = 100;

/// Marker for a noble scion of the Homeworld's cadet branch.
///
/// The pop is useless (zero skills; [`Trait::Noble`] bars all labor) but
/// each living scion pays [`Self::allowance`] credits every
/// [`ALLOWANCE_PERIOD_TICKS`] ticks while alive.
#[derive(Component, Debug, Clone, Copy)]
pub struct NobleScion {
    /// Credits paid per allowance period.
    pub allowance: f32,
    /// Homeworld faction id in [`FactionRelations`]; tanked on death.
    pub faction_id: u32,
}

/// Event: a courier from the Homeworld has delivered noble scions.
#[derive(Event, Debug, Clone)]
pub struct ScionArrivalEvent {
    /// How many scions are coming down.
    pub count: u32,
}

/// Spawns noble scions: full pops with zero skills and the Noble trait
/// (total labor refusal via the utility AI), landing at colony tiles like
/// other arrivals.
pub fn process_scion_arrival_system(
    mut commands: Commands,
    mut events: EventReader<ScionArrivalEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    sim_time: Option<Res<SimulationTime>>,
    pop_positions: Query<&GridPosition, With<Pop>>,
) {
    let tick = sim_time.map_or(0, |t| t.tick);
    // Scions land where the colony already is, not in the vacuum.
    let colony_tiles: Vec<GridPosition> = pop_positions.iter().copied().collect();

    for event in events.read() {
        let mut rng = rand::thread_rng();

        for _ in 0..event.count {
            let spawn_pos = colony_tiles
                .choose(&mut rng)
                .copied()
                .unwrap_or(GridPosition { x: 0, y: 0 });

            let mut bundle = PopBundle::random(spawn_pos.x, spawn_pos.y, &mut rng);
            // Scions have terrible stats: zero skills...
            bundle.skills = Skills::default();
            // ...and exactly one trait: Noble (no manual labor, ever).
            let mut traits = Traits::default();
            traits.add(Trait::Noble);
            bundle.traits = traits;
            bundle.arrival = Arrival { tick };

            commands.spawn((
                bundle,
                NobleScion {
                    allowance: DEFAULT_SCION_ALLOWANCE,
                    faction_id: HOMEWORLD_FACTION_ID,
                },
            ));
        }

        chronicle.send(AddChronicleEvent {
            text: format!(
                "A courier from the Homeworld has delivered {} noble scions — useless, exquisite, and enormously well-funded. Their families' allowances will flow as long as the little darlings survive.",
                event.count
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Pays each living scion's allowance into colony credits, once per
/// [`ALLOWANCE_PERIOD_TICKS`] ticks, scaled by the scion's morale.
/// Dead scions leave the query, so funding stops automatically.
pub fn process_allowance_system(
    mut resources: ResMut<ColonyResources>,
    query: Query<(&NobleScion, Option<&Morale>)>,
    time: Option<Res<SimulationTime>>,
) {
    if let Some(time) = time {
        if time.tick == 0 || !time.tick.is_multiple_of(ALLOWANCE_PERIOD_TICKS) {
            return;
        }
    } else {
        return;
    }

    for (scion, morale) in query.iter() {
        // Scale allowance by Morale (0.0 to 1.0)
        let modifier = morale.map_or(1.0, |m| m.value.clamp(0.0, 1.0));
        resources.add_credits(scion.allowance * modifier);
    }
}

/// When a noble scion dies, the Homeworld's fury tanks faction relations.
///
/// (Funding stops on its own: the dead scion leaves the allowance query.)
/// This supersedes the old spec-263 behavior (colony Unrest) — per spec
/// 1208, a scion's death is a diplomatic catastrophe, not a domestic one.
pub fn death_consequence_system(
    mut events: EventReader<PopDied>,
    query: Query<&NobleScion>,
    mut relations: ResMut<FactionRelations>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        let Ok(scion) = query.get(event.entity) else {
            continue;
        };
        let score = relations.scores.entry(scion.faction_id).or_insert(0);
        *score = (*score - SCION_DEATH_RELATION_PENALTY).max(-100);

        chronicle.send(AddChronicleEvent {
            text: format!(
                "A noble scion has died ({}). The Homeworld is furious — relations have collapsed.",
                event.name
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Counts living noble scions (headless STATS).
pub fn scion_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<Entity, With<NobleScion>>();
    query.iter(world).count()
}

/// Public-facing strings, for the IP-guard test.
pub fn cadet_public_strings() -> Vec<String> {
    vec![
        "A courier from the Homeworld has delivered noble scions — useless, exquisite, and enormously well-funded. Their families' allowances will flow as long as the little darlings survive.".to_string(),
        "A noble scion has died. The Homeworld is furious — relations have collapsed.".to_string(),
        Trait::Noble.label().to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::skills::SkillType;

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(FactionRelations::default());
        world.insert_resource(Events::<PopDied>::default());
        world.insert_resource(Events::<ScionArrivalEvent>::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world
    }

    fn spawn_scion(world: &mut World, allowance: f32, morale: f32) -> Entity {
        let mut traits = Traits::default();
        traits.add(Trait::Noble);
        world
            .spawn((
                Pop,
                traits,
                NobleScion {
                    allowance,
                    faction_id: HOMEWORLD_FACTION_ID,
                },
                Morale {
                    value: morale,
                    ..Default::default()
                },
            ))
            .id()
    }

    fn run_allowance(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(process_allowance_system);
        schedule.run(world);
    }

    fn run_deaths(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(death_consequence_system);
        schedule.run(world);
    }

    fn kill(world: &mut World, entity: Entity) {
        world.send_event(PopDied {
            entity,
            name: "Test Scion".to_string(),
            tick: 100,
            reason: "Mock death".to_string(),
        });
    }

    // --- Spec 1208 RED phase, adapted to the real types ---

    #[test]
    fn test_noble_scion_grants_allowance() {
        let mut world = setup();
        world.resource_mut::<SimulationTime>().tick = ALLOWANCE_PERIOD_TICKS;

        spawn_scion(&mut world, 50.0, 1.0);
        run_allowance(&mut world);

        let resources = world.resource::<ColonyResources>();
        assert_eq!(resources.credits, 50.0);
    }

    #[test]
    fn test_noble_scion_death_tanks_relations() {
        let mut world = setup();
        world
            .resource_mut::<FactionRelations>()
            .scores
            .insert(HOMEWORLD_FACTION_ID, 50);

        let scion = spawn_scion(&mut world, 50.0, 1.0);
        kill(&mut world, scion);
        run_deaths(&mut world);

        let relations = world.resource::<FactionRelations>();
        let relation = relations.scores[&HOMEWORLD_FACTION_ID];
        // Huge relations penalty
        assert!(relation < 0, "relation should tank, got {relation}");
        assert_eq!(relation, -50);
    }

    // --- REFACTOR phase: periodic allowance ---

    #[test]
    fn test_allowance_only_on_monthly_tick() {
        let mut world = setup();
        spawn_scion(&mut world, 50.0, 1.0);

        world.resource_mut::<SimulationTime>().tick = ALLOWANCE_PERIOD_TICKS - 1;
        run_allowance(&mut world);
        assert_eq!(world.resource::<ColonyResources>().credits, 0.0);

        world.resource_mut::<SimulationTime>().tick = ALLOWANCE_PERIOD_TICKS + 1;
        run_allowance(&mut world);
        assert_eq!(world.resource::<ColonyResources>().credits, 0.0);

        world.resource_mut::<SimulationTime>().tick = ALLOWANCE_PERIOD_TICKS;
        run_allowance(&mut world);
        assert_eq!(world.resource::<ColonyResources>().credits, 50.0);
    }

    #[test]
    fn test_noble_allowance_income_scales_with_morale() {
        let mut world = setup();
        world.resource_mut::<SimulationTime>().tick = ALLOWANCE_PERIOD_TICKS;

        spawn_scion(&mut world, 100.0, 0.5);
        run_allowance(&mut world);

        let resources = world.resource::<ColonyResources>();
        assert_eq!(resources.credits, 50.0);
    }

    #[test]
    fn test_noble_allowance_not_on_tick_zero() {
        let mut world = setup();
        world.resource_mut::<SimulationTime>().tick = 0;

        spawn_scion(&mut world, 100.0, 1.0);
        run_allowance(&mut world);

        let resources = world.resource::<ColonyResources>();
        assert_eq!(resources.credits, 0.0);
    }

    // --- Death consequences ---

    #[test]
    fn test_scion_death_penalty_floors_at_hostile() {
        let mut world = setup();
        world
            .resource_mut::<FactionRelations>()
            .scores
            .insert(HOMEWORLD_FACTION_ID, 50);

        let scion = spawn_scion(&mut world, 50.0, 1.0);
        kill(&mut world, scion);
        run_deaths(&mut world);
        // Second death (another scion) must not push below -100.
        let scion2 = spawn_scion(&mut world, 50.0, 1.0);
        kill(&mut world, scion2);
        run_deaths(&mut world);

        let relations = world.resource::<FactionRelations>();
        assert_eq!(relations.scores[&HOMEWORLD_FACTION_ID], -100);
    }

    #[test]
    fn test_non_scion_death_ignores_relations() {
        let mut world = setup();
        world
            .resource_mut::<FactionRelations>()
            .scores
            .insert(HOMEWORLD_FACTION_ID, 50);

        let pleb = world.spawn(Pop).id();
        kill(&mut world, pleb);
        run_deaths(&mut world);

        let relations = world.resource::<FactionRelations>();
        assert_eq!(relations.scores[&HOMEWORLD_FACTION_ID], 50);
        let events = world.resource::<Events<AddChronicleEvent>>();
        assert_eq!(events.len(), 0, "no chronicle for commoner deaths");
    }

    #[test]
    fn test_scion_death_sends_chronicle() {
        let mut world = setup();
        let scion = spawn_scion(&mut world, 50.0, 1.0);
        kill(&mut world, scion);
        run_deaths(&mut world);

        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let entries: Vec<_> = reader.read(events).collect();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].text.contains("furious"),
            "chronicle should mention the Homeworld's fury"
        );
    }

    // --- Arrival ---

    #[test]
    fn test_scion_arrival_spawns_useless_nobles() {
        let mut world = setup();
        // One existing pop so the scions have a colony tile to land on.
        world.spawn((Pop, GridPosition { x: 10, y: 10 }));

        world
            .resource_mut::<Events<ScionArrivalEvent>>()
            .send(ScionArrivalEvent { count: 3 });

        let mut schedule = Schedule::default();
        schedule.add_systems(process_scion_arrival_system);
        schedule.run(&mut world);

        let mut query =
            world.query_filtered::<(&Skills, &Traits, &NobleScion, &Arrival), With<Pop>>();
        let mut count = 0;
        for (skills, traits, scion, arrival) in query.iter(&world) {
            count += 1;
            for skill in [
                SkillType::Mining,
                SkillType::Forestry,
                SkillType::Farming,
                SkillType::Construction,
                SkillType::Crafting,
                SkillType::Husbandry,
                SkillType::Engineering,
                SkillType::Science,
            ] {
                assert_eq!(
                    skills.get_xp(skill),
                    0.0,
                    "Scions should have terrible stats (zero skills)"
                );
            }
            assert!(
                traits.has(Trait::Noble),
                "Scions should carry the Noble (snob) trait"
            );
            assert_eq!(scion.allowance, DEFAULT_SCION_ALLOWANCE);
            assert_eq!(scion.faction_id, HOMEWORLD_FACTION_ID);
            assert_eq!(arrival.tick, 0);
        }
        assert_eq!(count, 3, "Should spawn exact number of scions");
    }

    #[test]
    fn test_scion_arrival_sends_chronicle() {
        let mut world = setup();
        world.spawn((Pop, GridPosition { x: 10, y: 10 }));

        world
            .resource_mut::<Events<ScionArrivalEvent>>()
            .send(ScionArrivalEvent { count: 2 });

        let mut schedule = Schedule::default();
        schedule.add_systems(process_scion_arrival_system);
        schedule.run(&mut world);

        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let entries: Vec<_> = reader.read(events).collect();
        assert_eq!(entries.len(), 1, "One chronicle entry per arrival");
        assert!(
            entries[0].text.contains("Homeworld"),
            "Chronicle mentions the Homeworld"
        );
    }

    #[test]
    fn test_scion_count_helper() {
        let mut world = setup();
        assert_eq!(scion_count(&mut world), 0);
        spawn_scion(&mut world, 50.0, 1.0);
        spawn_scion(&mut world, 50.0, 1.0);
        world.spawn(Pop);
        assert_eq!(scion_count(&mut world), 2);
    }

    // --- IP guard ---

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // The Cadet Branch is concept-only: generic "Homeworld" / "Noble
        // Scion" naming, nothing lifted from outside fiction.
        let banned = [
            // Distinctive emperor-fiction terms (staying generic on purpose)
            "emperor",
            "dune",
            "corrino",
            "arrakis",
            "atreides",
            "imperium",
            "padishah",
            "sardaukar",
            "mentat",
            "fremen",
            // Standing rules
            "sad king billy",
            "windsor",
            "windsor-in-exile",
        ];
        for s in cadet_public_strings() {
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
