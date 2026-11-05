//! Workplace relationships and affinity decay (Spec 047 core, Spec 391 upgrade).
//!
//! Spec 047 gave pops a [`Relationships`] component and directional
//! [`AffinityChange`] events. Spec 391 completes the definition:
//!
//! * **Proximity aura** — mood effects come ONLY from nearby pops, via
//!   [`crate::layer1::social::proximity_social_system`]: it writes a
//!   [`crate::layer1::social::SocialBuff`], which
//!   `update_morale_cache_system` folds into `Morale.value` through
//!   `calculate_raw_morale` — the same modifier path every other social
//!   system uses. The pre-391 `calculate_relationship_mood_buff_system`
//!   in this module summed ALL affinities regardless of distance and
//!   mutated `Morale.value` directly on a 0–100 clamp (the morale cache
//!   is 0.0–1.0 and recomputed from scratch every tick), so it was
//!   removed in favour of the proximity system; its distant-friend
//!   regression test lives on below, aimed at the proximity system.
//! * **Affinity decay** — [`decay_affinities_system`] drifts every
//!   affinity toward 0.0 on a throttled cadence (see
//!   [`AFFINITY_DECAY_INTERVAL_TICKS`]); relationships that are not
//!   refreshed by interaction are forgotten, and entries that reach
//!   exactly 0.0 are pruned so the maps do not grow stale entries.
//!
//! Reciprocity audit (Spec 391): [`AffinityChange`] is DIRECTIONAL by
//! codebase convention — `modify_affinity_system` changes only the
//! source's opinion. Emitters that model a mutual interaction send both
//! directions explicitly (this module's workplace system and
//! `water_cooler_gossip` do; rumor gossip, grievances, and social debt
//! are deliberately one-way). Making the event itself reciprocal would
//! double-count the mutual emitters, so the convention is kept.

use crate::layer1::actions::AssignedTo;
use crate::layer1::social::{AffinityChange, Relationships, SOCIAL_AURA_THRESHOLD};
use crate::shared::time::SimulationTime;
use bevy::prelude::*;

/// Event fired when a work shift ends; co-workers grow closer.
#[derive(Event)]
pub struct ShiftEndEvent;

/// Ticks between affinity-decay steps (Spec 391).
///
/// Decay is throttled like the repo's other periodic social systems
/// (hedonic-treadmill living-standard decay, seniority friction polls):
/// the system early-returns unless the simulation tick lands on the
/// interval, so its cost is one pass per 100 ticks, not per tick.
pub const AFFINITY_DECAY_INTERVAL_TICKS: u64 = 100;

/// Affinity points shed per decay step, toward 0.0 (Spec 391).
///
/// The rate works out to 1.0 point per 1,000 ticks. That is
/// deliberately slower than the slowest regular affinity income:
/// workplace shifts add +1.0 per shared shift-end and a day is 250
/// ticks, so pops who keep working together net +0.75/day after decay
/// — active bonds grow. Neglected ones fade: a single-shift
/// acquaintance (+1) is forgotten in 1,000 ticks, a maxed +100
/// friendship takes 100,000 ticks of total silence.
pub const AFFINITY_DECAY_STEP: f32 = 0.1;

/// Builds workplace affinity: pops assigned to the same entity send each
/// other a +1.0 [`AffinityChange`] on [`ShiftEndEvent`] (both directions
/// explicitly — see the module reciprocity note).
pub fn update_workplace_relationships_system(
    mut events: EventReader<ShiftEndEvent>,
    query: Query<(Entity, &AssignedTo)>,
    mut affinity_events: EventWriter<AffinityChange>,
) {
    for _ in events.read() {
        // Collect all workers and their locations
        let mut location_map = bevy::utils::HashMap::with_capacity(query.iter().size_hint().0);
        for (entity, assigned_to) in query.iter() {
            location_map
                .entry(assigned_to.entity)
                .or_insert_with(Vec::new)
                .push(entity);
        }

        // Build relationships based on shared locations
        for (_, workers) in location_map.iter() {
            if workers.len() > 1 {
                for &worker in workers {
                    for &peer in workers {
                        if worker != peer {
                            affinity_events.send(AffinityChange {
                                source: worker,
                                target: peer,
                                amount: 1.0,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Drifts every affinity toward 0.0 (Spec 391 decay leg).
///
/// Runs only on ticks that are a multiple of
/// [`AFFINITY_DECAY_INTERVAL_TICKS`]; each step moves every stored
/// affinity [`AFFINITY_DECAY_STEP`] points closer to neutral without
/// overshooting, and entries that reach exactly 0.0 are removed.
pub fn decay_affinities_system(
    time: Res<SimulationTime>,
    mut query: Query<&mut Relationships>,
) {
    if !time.tick.is_multiple_of(AFFINITY_DECAY_INTERVAL_TICKS) {
        return;
    }
    for mut rels in query.iter_mut() {
        rels.affinities.retain(|_, value| {
            if *value > 0.0 {
                *value = (*value - AFFINITY_DECAY_STEP).max(0.0);
            } else if *value < 0.0 {
                *value = (*value + AFFINITY_DECAY_STEP).min(0.0);
            }
            *value != 0.0
        });
    }
}

/// Census of the colony's relationship web (Spec 391 headless observability).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RelationshipCensus {
    /// Total stored affinity entries across all pops.
    pub bonds: usize,
    /// Entries above the friend threshold ([`SOCIAL_AURA_THRESHOLD`]).
    pub friends: usize,
    /// Entries below the negative friend threshold.
    pub rivals: usize,
}

/// Counts bonds / friends / rivals across every pop's [`Relationships`].
#[must_use]
pub fn relationship_census(world: &mut World) -> RelationshipCensus {
    let mut census = RelationshipCensus::default();
    let mut query = world.query::<&Relationships>();
    for rels in query.iter(world) {
        for &value in rels.affinities.values() {
            census.bonds += 1;
            if value > SOCIAL_AURA_THRESHOLD {
                census.friends += 1;
            } else if value < -SOCIAL_AURA_THRESHOLD {
                census.rivals += 1;
            }
        }
    }
    census
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::actions::AssignmentType;
    use crate::layer1::entities::pop::Pop;
    use crate::layer1::map::GridPosition;
    use crate::layer1::needs::Needs;
    use crate::layer1::social::morale::{update_morale_cache_system, Morale};
    use crate::layer1::social::{modify_affinity_system, proximity_social_system, SocialBuff};

    #[test]
    fn test_pops_develop_relationships_working_together() {
        // Arrange
        let mut app = App::new();
        app.add_event::<ShiftEndEvent>();
        app.add_event::<AffinityChange>();
        app.add_systems(Update, update_workplace_relationships_system);

        let _entity1 = app
            .world_mut()
            .spawn((
                Pop,
                AssignedTo {
                    entity: Entity::from_raw(1),
                    assignment_type: AssignmentType::FarmWorker,
                },
                Relationships::default(),
            ))
            .id();

        let _entity2 = app
            .world_mut()
            .spawn((
                Pop,
                AssignedTo {
                    entity: Entity::from_raw(1),
                    assignment_type: AssignmentType::FarmWorker,
                }, // Same location
                Relationships::default(),
            ))
            .id();

        // Act
        app.world_mut().send_event(ShiftEndEvent);
        app.update();

        // Assert
        let events = app.world().resource::<Events<AffinityChange>>();
        let mut reader = events.get_cursor();
        let emitted: Vec<_> = reader.read(events).collect();

        assert_eq!(emitted.len(), 2); // 1->2 and 2->1
        assert_eq!(emitted[0].amount, 1.0);
    }

    /// Reciprocity audit (Spec 391): a single `AffinityChange` is
    /// directional — only the source's opinion moves. Mutual systems
    /// (workplace, water-cooler) send both directions themselves.
    #[test]
    fn test_affinity_change_is_directional() {
        let mut app = App::new();
        app.add_event::<AffinityChange>();
        app.add_systems(Update, modify_affinity_system);

        let pop_b = app
            .world_mut()
            .spawn((Pop, Relationships::default()))
            .id();
        let pop_a = app
            .world_mut()
            .spawn((Pop, Relationships::default()))
            .id();

        app.world_mut().send_event(AffinityChange {
            source: pop_a,
            target: pop_b,
            amount: 5.0,
        });
        app.update();

        assert_eq!(
            app.world().get::<Relationships>(pop_a).unwrap().get_affinity(pop_b),
            5.0
        );
        assert_eq!(
            app.world().get::<Relationships>(pop_b).unwrap().get_affinity(pop_a),
            0.0,
            "AffinityChange must stay directional; reciprocity is the emitter's job"
        );
    }

    fn aura_world() -> (World, Schedule) {
        let world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems(proximity_social_system);
        (world, schedule)
    }

    /// Spec 391 RED test, adapted to real types: the spec's
    /// `PopRelationships`/`Mood{current}`/Transform sketch maps to
    /// `Relationships` + `SocialBuff` + `GridPosition`. A high-affinity
    /// pop standing nearby buffs via the aura.
    #[test]
    fn test_near_friend_grants_aura_buff() {
        let (mut world, mut schedule) = aura_world();
        let friend = world.spawn((Pop, GridPosition { x: 10, y: 11 })).id();
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 10, y: 10 },
                Relationships::with_affinity(friend, 50.0),
            ))
            .id();

        schedule.run(&mut world);

        let buff = world.get::<SocialBuff>(pop);
        assert!(buff.is_some(), "nearby friend should grant a SocialBuff");
        assert!((buff.unwrap().value - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_near_rival_grants_aura_debuff() {
        let (mut world, mut schedule) = aura_world();
        let rival = world.spawn((Pop, GridPosition { x: 10, y: 11 })).id();
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 10, y: 10 },
                Relationships::with_affinity(rival, -50.0),
            ))
            .id();

        schedule.run(&mut world);

        let buff = world.get::<SocialBuff>(pop);
        assert!(buff.is_some(), "nearby rival should grant a SocialBuff");
        assert!((buff.unwrap().value + 0.1).abs() < f32::EPSILON);
    }

    /// The Spec 391 core regression: affinity alone is not an aura.
    /// A beloved friend twenty tiles away grants nothing (the pre-391
    /// mood system in this module buffed regardless of distance).
    #[test]
    fn test_distant_friend_grants_no_aura() {
        let (mut world, mut schedule) = aura_world();
        let friend = world.spawn((Pop, GridPosition { x: 30, y: 30 })).id();
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 10, y: 10 },
                Relationships::with_affinity(friend, 50.0),
            ))
            .id();

        schedule.run(&mut world);

        assert!(
            world.get::<SocialBuff>(pop).is_none(),
            "distant friend must not grant an aura buff"
        );
    }

    /// The aura flows through the real morale path: proximity writes
    /// `SocialBuff`, and `update_morale_cache_system` folds it into
    /// `Morale.value` (0.0–1.0 cache) — no direct Morale mutation.
    #[test]
    fn test_aura_flows_into_morale_cache() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems((
            proximity_social_system,
            update_morale_cache_system.after(proximity_social_system),
        ));

        let friend = world
            .spawn((Pop, GridPosition { x: 10, y: 11 }, Needs::default(), Morale::default()))
            .id();
        let befriended = world
            .spawn((
                Pop,
                GridPosition { x: 10, y: 10 },
                Needs::default(),
                Morale::default(),
                Relationships::with_affinity(friend, 50.0),
            ))
            .id();
        let lonely = world
            .spawn((
                Pop,
                GridPosition { x: 40, y: 40 },
                Needs::default(),
                Morale::default(),
                Relationships::default(),
            ))
            .id();

        schedule.run(&mut world);

        let with_friend = world.get::<Morale>(befriended).unwrap().value;
        let without = world.get::<Morale>(lonely).unwrap().value;
        assert!(
            (with_friend - without - 0.1).abs() < 0.001,
            "near friend should lift cached morale by the aura amount: {with_friend} vs {without}"
        );
    }

    fn decay_world(tick: u64) -> (World, Schedule) {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick,
            ..Default::default()
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(decay_affinities_system);
        (world, schedule)
    }

    #[test]
    fn test_affinity_decays_toward_zero() {
        let (mut world, mut schedule) = decay_world(AFFINITY_DECAY_INTERVAL_TICKS);
        let other = world.spawn(Pop).id();
        let third = world.spawn(Pop).id();
        let mut rels = Relationships::default();
        rels.set_affinity(other, 50.0);
        rels.set_affinity(third, -30.0);
        let pop = world.spawn((Pop, rels)).id();

        schedule.run(&mut world);

        let rels = world.get::<Relationships>(pop).unwrap();
        assert_eq!(rels.get_affinity(other), 50.0 - AFFINITY_DECAY_STEP);
        assert_eq!(rels.get_affinity(third), -30.0 + AFFINITY_DECAY_STEP);
    }

    #[test]
    fn test_affinity_decay_prunes_faded_bonds() {
        let (mut world, mut schedule) = decay_world(AFFINITY_DECAY_INTERVAL_TICKS);
        let other = world.spawn(Pop).id();
        let pop = world
            .spawn((Pop, Relationships::with_affinity(other, AFFINITY_DECAY_STEP / 2.0)))
            .id();

        schedule.run(&mut world);

        let rels = world.get::<Relationships>(pop).unwrap();
        assert!(
            rels.affinities.is_empty(),
            "a bond that decays to exactly 0.0 is forgotten, not stored"
        );
    }

    #[test]
    fn test_affinity_decay_is_throttled() {
        let (mut world, mut schedule) = decay_world(AFFINITY_DECAY_INTERVAL_TICKS - 1);
        let other = world.spawn(Pop).id();
        let pop = world
            .spawn((Pop, Relationships::with_affinity(other, 50.0)))
            .id();

        schedule.run(&mut world);

        assert_eq!(
            world.get::<Relationships>(pop).unwrap().get_affinity(other),
            50.0,
            "no decay between interval ticks"
        );
    }

    #[test]
    fn test_relationship_census_counts_bonds_friends_rivals() {
        let mut world = World::new();
        let a = world.spawn(Pop).id();
        let b = world.spawn(Pop).id();
        let c = world.spawn(Pop).id();
        let mut rels = Relationships::default();
        rels.set_affinity(a, 50.0); // friend
        rels.set_affinity(b, -50.0); // rival
        rels.set_affinity(c, 5.0); // mere acquaintance
        world.spawn((Pop, rels));

        let census = relationship_census(&mut world);
        assert_eq!(census.bonds, 3);
        assert_eq!(census.friends, 1);
        assert_eq!(census.rivals, 1);
    }
}
