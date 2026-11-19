//! Spec 432: Quantum Entanglement Commute.
//!
//! Quantum Pads are linked pairs of pads: a pop standing on one pad whose
//! destination lies far away is instantly entangled to the linked pad.
//! The jump is not free:
//!
//! * **Stress**: +10.0 [`StressTracker::accumulated_stress`] per jump
//!   (the spec's `Stress` maps to the real `StressTracker`).
//! * **Dissociation**: +5.0 [`Dissociation::level`] per jump, reusing the
//!   existing teleport-psychosis machinery in
//!   [`crate::layer1::psychology::teleport_psychosis`] — this module does
//!   NOT duplicate it, and the gated `hunger_decay_system` there is
//!   untouched (`With<Dissociation>`).
//! * **Energy**: each use discharges [`ENERGY_COST_PER_JUMP`] from the
//!   departure pad's [`Battery`]; a pad that cannot pay refuses the jump.
//! * **TeleportCount**: jumps are counted per pop and decay by one per
//!   in-game day, so infrequent jumps are safe.
//!
//! A pop at high stress (>= [`DISSOCIATION_STRESS_THRESHOLD`]) with
//! >= [`DISSOCIATION_JUMP_THRESHOLD`] recent jumps gains
//! [`TemporalDissociation`]: like a stress breakdown it overrides normal
//! job AI (a `BreakdownType::Dazing` breakdown of the same duration, so
//! the existing mental-break evaluator makes the pop wander aimlessly).
//! Per spec §8 it lasts ~12 in-game hours — half of the default
//! 250-tick day, i.e. [`DISSOCIATION_DURATION_TICKS`] ticks — cannot be
//! sped up by medical care in the MVP, and clears faster while the pop
//! is rested (rest need >= 0.8 ticks the timer down twice as fast).

use crate::layer1::energy::Battery;
use crate::layer1::map::GridPosition;
use crate::layer1::execution::MovementTarget;
use crate::layer1::pop::Pop;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::psychology::teleport_psychosis::Dissociation;
use crate::layer1::stress::{Breakdown, BreakdownType, StressTracker};
use bevy::prelude::*;

/// Stress applied per quantum jump (spec §3/§6: +10.0).
pub const QUANTUM_STRESS_PER_JUMP: f32 = 10.0;
/// Dissociation applied per quantum jump (matches the existing
/// `TELEPORT_DISSOCIATION_COST` of the legacy teleporter).
pub const QUANTUM_DISSOCIATION_PER_JUMP: f32 = 5.0;
/// Battery charge discharged from the departure pad per jump.
pub const ENERGY_COST_PER_JUMP: f32 = 10.0;
/// Recent jumps at/above which dissociation can trigger.
pub const DISSOCIATION_JUMP_THRESHOLD: u32 = 5;
/// Accumulated stress at/above which dissociation can trigger.
/// Five fresh jumps alone contribute 50.0, so a pop who jumps five
/// times in quick succession dissociates on the fifth jump.
pub const DISSOCIATION_STRESS_THRESHOLD: f32 = 50.0;
/// Duration of Temporal Dissociation: ~12 in-game hours
/// (half of the default 250-tick day). Spec §8.
pub const DISSOCIATION_DURATION_TICKS: u32 = 125;
/// One counted jump is forgiven every this many ticks (one day), so
/// infrequent jumps never accumulate toward dissociation.
pub const TELEPORT_COUNT_DECAY_TICKS: u32 = 250;
/// Manhattan distance at/above which the utility AI prefers a pad
/// over walking.
pub const QUANTUM_PAD_FAR_DISTANCE: u64 = 20;

/// Per-pop ledger of recent quantum jumps (spec's `TeleportCount`).
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TeleportCount {
    /// Jumps in the current decay window.
    pub jumps: u32,
    /// Ticks since the most recent jump (drives decay).
    pub ticks_since_jump: u32,
}

/// Marker + timer: the pop believes it is in two places at once and
/// wanders aimlessly until the timer clears (spec's
/// `TemporalDissociation`).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalDissociation {
    /// Ticks remaining until the state clears.
    pub remaining_ticks: u32,
}

impl Default for TemporalDissociation {
    fn default() -> Self {
        Self {
            remaining_ticks: DISSOCIATION_DURATION_TICKS,
        }
    }
}

/// A Quantum Pad building component. Pads work in linked pairs.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuantumPad {
    /// The pad at the other end of the entanglement link.
    pub linked_pad: Option<Entity>,
    /// Lifetime successful jumps from this pad.
    pub uses: u64,
}

/// Colony-wide ledger for STATS / headless reporting.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct QuantumCommuteLedger {
    /// Total successful quantum jumps.
    pub total_jumps: u64,
    /// Jumps refused because the departure pad could not pay energy.
    pub refused_jumps: u64,
    /// Times Temporal Dissociation has triggered.
    pub dissociations: u64,
    /// Total battery energy spent on jumps.
    pub energy_spent: f32,
}

/// Request a quantum jump for `entity` to `target_pos`.
/// `pad` is the departure Quantum Pad paying the energy cost, if any;
/// pad-less jumps (debug / legacy teleporter bridges) are free.
#[derive(Event, Debug, Clone, Copy)]
pub struct QuantumTeleportEvent {
    /// The pop to move.
    pub entity: Entity,
    /// Destination tile.
    pub target_pos: GridPosition,
    /// Departure pad entity, if the jump starts on a pad.
    pub pad: Option<Entity>,
}

/// Pure utility-AI scoring for pad use (spec §7): pops choose pads for
/// far pathing, and avoid them as their recent jump count climbs.
/// Returns 0.0 when walking is preferred.
#[must_use]
pub fn quantum_pad_score(distance_manhattan: u64, count: &TeleportCount) -> f32 {
    if distance_manhattan < QUANTUM_PAD_FAR_DISTANCE {
        return 0.0;
    }
    if count.jumps >= DISSOCIATION_JUMP_THRESHOLD {
        return 0.0; // One more jump risks dissociation: walk instead.
    }
    // Farther is worth more; each recent jump discounts the benefit.
    let distance_bonus = (distance_manhattan as f32).min(100.0) / 100.0;
    let risk_discount = 1.0 - (count.jumps as f32 * 0.15);
    (distance_bonus * risk_discount).max(0.0)
}

/// Whether the utility AI should route this pop via a Quantum Pad.
#[must_use]
pub fn should_use_quantum_pad(distance_manhattan: u64, count: &TeleportCount) -> bool {
    quantum_pad_score(distance_manhattan, count) > 0.0
}

/// GREEN (spec §4, adapted to real types): apply quantum jumps —
/// instant move, +10 stress, +5 dissociation, +1 TeleportCount, and
/// the departure pad pays the energy cost.
#[allow(clippy::type_complexity)]
pub fn quantum_commute_system(
    mut events: EventReader<QuantumTeleportEvent>,
    mut commands: Commands,
    mut pops: Query<(
        &mut GridPosition,
        Option<&mut StressTracker>,
        Option<&mut Dissociation>,
        Option<&mut TeleportCount>,
    )>,
    mut pads: Query<(&mut QuantumPad, Option<&mut Battery>)>,
    mut ledger: Option<ResMut<QuantumCommuteLedger>>,
) {
    for event in events.read() {
        // Energy gate: a pad that cannot pay refuses the jump.
        if let Some(pad_entity) = event.pad {
            if let Ok((_, Some(battery))) = pads.get(pad_entity) {
                {
                    if battery.charge < ENERGY_COST_PER_JUMP {
                        if let Some(ledger) = ledger.as_deref_mut() {
                            ledger.refused_jumps += 1;
                        }
                        continue;
                    }
                }
            }
        }

        let Ok((mut pos, stress, dissociation, count)) = pops.get_mut(event.entity) else {
            continue;
        };
        *pos = event.target_pos;

        match stress {
            Some(mut stress) => stress.accumulated_stress += QUANTUM_STRESS_PER_JUMP,
            None => {
                commands.entity(event.entity).insert(StressTracker {
                    accumulated_stress: QUANTUM_STRESS_PER_JUMP,
                });
            }
        }
        match dissociation {
            Some(mut dissociation) => dissociation.level += QUANTUM_DISSOCIATION_PER_JUMP,
            None => {
                commands.entity(event.entity).insert(Dissociation {
                    level: QUANTUM_DISSOCIATION_PER_JUMP,
                });
            }
        }
        match count {
            Some(mut count) => {
                count.jumps += 1;
                count.ticks_since_jump = 0;
            }
            None => {
                commands.entity(event.entity).insert(TeleportCount {
                    jumps: 1,
                    ticks_since_jump: 0,
                });
            }
        }

        if let Some(pad_entity) = event.pad {
            if let Ok((mut pad, battery)) = pads.get_mut(pad_entity) {
                pad.uses += 1;
                if let Some(mut battery) = battery {
                    let _ = battery.discharge(ENERGY_COST_PER_JUMP);
                }
            }
        }
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.total_jumps += 1;
            ledger.energy_spent += ENERGY_COST_PER_JUMP;
        }
    }
}

/// Spec §3/§6: high stress + frequent jumps -> Temporal Dissociation.
/// Also inserts a Dazing breakdown of the same duration so the
/// existing mental-break evaluator overrides job AI with aimless
/// wandering (spec §5/§7), exactly like stress breakdowns.
pub fn check_dissociation_system(
    mut commands: Commands,
    mut query: Query<
        (Entity, &StressTracker, &mut TeleportCount),
        Without<TemporalDissociation>,
    >,
    mut ledger: Option<ResMut<QuantumCommuteLedger>>,
) {
    for (entity, stress, mut count) in query.iter_mut() {
        if stress.accumulated_stress >= DISSOCIATION_STRESS_THRESHOLD
            && count.jumps >= DISSOCIATION_JUMP_THRESHOLD
        {
            commands.entity(entity).insert((
                TemporalDissociation::default(),
                Breakdown {
                    breakdown_type: BreakdownType::Dazing,
                    duration_remaining: DISSOCIATION_DURATION_TICKS,
                },
            ));
            // The counted jumps are spent triggering the episode;
            // the pop starts fresh after recovery.
            count.jumps = 0;
            count.ticks_since_jump = 0;
            if let Some(ledger) = ledger.as_deref_mut() {
                ledger.dissociations += 1;
            }
        }
    }
}

/// Counts Temporal Dissociation down. Rested pops (rest >= 0.8)
/// recover twice as fast — it "clears naturally with rest" (spec §8).
/// Medical care does not affect it in the MVP.
pub fn temporal_dissociation_timer_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut TemporalDissociation, Option<&Needs>)>,
) {
    for (entity, mut dissociation, needs) in query.iter_mut() {
        let rested = needs.is_some_and(|n| n.rest >= 0.8);
        let step = if rested { 2 } else { 1 };
        dissociation.remaining_ticks = dissociation.remaining_ticks.saturating_sub(step);
        if dissociation.remaining_ticks == 0 {
            commands.entity(entity).remove::<TemporalDissociation>();
        }
    }
}

/// REFACTOR (spec §5): TeleportCount decays — one jump forgiven per
/// day without a jump, so infrequent jumps are safe.
pub fn teleport_count_decay_system(mut query: Query<&mut TeleportCount>) {
    for mut count in query.iter_mut() {
        if count.jumps == 0 {
            count.ticks_since_jump = 0;
            continue;
        }
        count.ticks_since_jump = count.ticks_since_jump.saturating_add(1);
        if count.ticks_since_jump >= TELEPORT_COUNT_DECAY_TICKS {
            count.jumps -= 1;
            count.ticks_since_jump = 0;
        }
    }
}

/// Utility-AI integration (spec §7): a pop with a far MovementTarget
/// who is standing on a linked Quantum Pad is routed through the pad
/// when [`should_use_quantum_pad`] approves. Dissociating and broken
/// pops never use pads.
#[allow(clippy::type_complexity)]
pub fn quantum_commute_autonomous_system(
    pops: Query<
        (Entity, &GridPosition, &MovementTarget, Option<&TeleportCount>),
        (
            With<Pop>,
            Without<TemporalDissociation>,
            Without<Breakdown>,
        ),
    >,
    pads: Query<(Entity, &GridPosition, &QuantumPad)>,
    mut events: EventWriter<QuantumTeleportEvent>,
) {
    for (entity, pos, target, count) in pops.iter() {
        let distance = pos.distance_manhattan(target.target_position);
        let default_count = TeleportCount::default();
        let count = count.unwrap_or(&default_count);
        if !should_use_quantum_pad(distance, count) {
            continue;
        }
        for (pad_entity, pad_pos, pad) in pads.iter() {
            if *pad_pos != *pos {
                continue;
            }
            let Some(linked) = pad.linked_pad else { continue };
            let Ok((_, linked_pos, _)) = pads.get(linked) else {
                continue;
            };
            // Only jump if the far end actually helps.
            if linked_pos.distance_manhattan(target.target_position) < distance {
                events.send(QuantumTeleportEvent {
                    entity,
                    target_pos: *linked_pos,
                    pad: Some(pad_entity),
                });
            }
            break;
        }
    }
}

/// Census helper for STATS/headless: pops currently dissociating.
#[must_use]
pub fn dissociated_pop_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<(), With<TemporalDissociation>>();
    query.iter(world).count()
}

/// Census helper for STATS/headless: placed Quantum Pads.
#[must_use]
pub fn quantum_pad_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<(), With<QuantumPad>>();
    query.iter(world).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_systems() -> App {
        let mut app = App::new();
        app.add_event::<QuantumTeleportEvent>();
        app.insert_resource(QuantumCommuteLedger::default());
        app.add_systems(
            Update,
            (
                quantum_commute_system,
                check_dissociation_system,
                temporal_dissociation_timer_system,
                teleport_count_decay_system,
            )
                .chain(),
        );
        app
    }

    /// Spec §3 RED test 1, adapted: Position->GridPosition,
    /// Stress->{value}->StressTracker{accumulated_stress}.
    #[test]
    fn test_teleportation_applies_disorientation() {
        let mut app = app_with_systems();
        let entity = app
            .world_mut()
            .spawn((
                GridPosition { x: 0, y: 0 },
                StressTracker {
                    accumulated_stress: 0.0,
                },
                TeleportCount::default(),
            ))
            .id();

        app.world_mut().send_event(QuantumTeleportEvent {
            entity,
            target_pos: GridPosition { x: 100, y: 0 },
            pad: None,
        });
        app.update();

        let pos = app.world().get::<GridPosition>(entity).unwrap();
        assert_eq!(*pos, GridPosition { x: 100, y: 0 });
        let stress = app.world().get::<StressTracker>(entity).unwrap();
        assert_eq!(stress.accumulated_stress, 10.0); // +10 stress per jump
        let count = app.world().get::<TeleportCount>(entity).unwrap();
        assert_eq!(count.jumps, 1);
        let dissociation = app.world().get::<Dissociation>(entity).unwrap();
        assert_eq!(dissociation.level, QUANTUM_DISSOCIATION_PER_JUMP);
    }

    /// Spec §3 RED test 2, adapted to real types.
    #[test]
    fn test_temporal_dissociation_trigger() {
        let mut app = app_with_systems();
        let entity = app
            .world_mut()
            .spawn((
                StressTracker {
                    accumulated_stress: 100.0,
                },
                TeleportCount {
                    jumps: 5,
                    ticks_since_jump: 0,
                },
            ))
            .id();

        app.update();

        assert!(app.world().get::<TemporalDissociation>(entity).is_some());
        // Wander-aimlessly override rides the existing breakdown lane.
        let breakdown = app.world().get::<Breakdown>(entity).unwrap();
        assert_eq!(breakdown.breakdown_type, BreakdownType::Dazing);
        assert_eq!(
            breakdown.duration_remaining,
            DISSOCIATION_DURATION_TICKS
        );
    }

    #[test]
    fn test_no_dissociation_below_either_threshold() {
        let mut app = app_with_systems();
        let calm_frequent = app
            .world_mut()
            .spawn((
                StressTracker {
                    accumulated_stress: 10.0,
                },
                TeleportCount {
                    jumps: 9,
                    ticks_since_jump: 0,
                },
            ))
            .id();
        let stressed_rare = app
            .world_mut()
            .spawn((
                StressTracker {
                    accumulated_stress: 100.0,
                },
                TeleportCount {
                    jumps: 2,
                    ticks_since_jump: 0,
                },
            ))
            .id();
        app.update();
        assert!(app.world().get::<TemporalDissociation>(calm_frequent).is_none());
        assert!(app.world().get::<TemporalDissociation>(stressed_rare).is_none());
    }

    #[test]
    fn test_five_quick_jumps_trigger_dissociation() {
        let mut app = app_with_systems();
        let entity = app
            .world_mut()
            .spawn((
                GridPosition { x: 0, y: 0 },
                StressTracker::default(),
                TeleportCount::default(),
            ))
            .id();
        for i in 0..5 {
            app.world_mut().send_event(QuantumTeleportEvent {
                entity,
                target_pos: GridPosition { x: 10 + i, y: 0 },
                pad: None,
            });
            app.update();
        }
        assert!(app.world().get::<TemporalDissociation>(entity).is_some());
    }

    #[test]
    fn test_teleport_count_decay_makes_infrequent_jumps_safe() {
        let mut app = App::new();
        app.add_systems(Update, teleport_count_decay_system);
        let entity = app
            .world_mut()
            .spawn(TeleportCount {
                jumps: 3,
                ticks_since_jump: 0,
            })
            .id();
        for _ in 0..TELEPORT_COUNT_DECAY_TICKS {
            app.update();
        }
        assert_eq!(app.world().get::<TeleportCount>(entity).unwrap().jumps, 2);
        for _ in 0..(TELEPORT_COUNT_DECAY_TICKS * 2) {
            app.update();
        }
        assert_eq!(app.world().get::<TeleportCount>(entity).unwrap().jumps, 0);
    }

    #[test]
    fn test_pad_energy_cost_and_refusal() {
        let mut app = app_with_systems();
        let pad = app
            .world_mut()
            .spawn((
                GridPosition { x: 0, y: 0 },
                QuantumPad::default(),
                Battery {
                    capacity: 100.0,
                    charge: 100.0,
                    max_throughput: 100.0,
                },
            ))
            .id();
        let pop = app
            .world_mut()
            .spawn((
                GridPosition { x: 0, y: 0 },
                StressTracker::default(),
                TeleportCount::default(),
            ))
            .id();

        app.world_mut().send_event(QuantumTeleportEvent {
            entity: pop,
            target_pos: GridPosition { x: 50, y: 0 },
            pad: Some(pad),
        });
        app.update();

        assert_eq!(
            app.world().get::<GridPosition>(pop).unwrap(),
            &GridPosition { x: 50, y: 0 }
        );
        let battery = app.world().get::<Battery>(pad).unwrap();
        assert_eq!(battery.charge, 100.0 - ENERGY_COST_PER_JUMP);
        assert_eq!(app.world().get::<QuantumPad>(pad).unwrap().uses, 1);

        // Drain the pad: the next jump is refused and the pop stays put.
        app.world_mut().get_mut::<Battery>(pad).unwrap().charge = 0.0;
        app.world_mut().send_event(QuantumTeleportEvent {
            entity: pop,
            target_pos: GridPosition { x: 0, y: 0 },
            pad: Some(pad),
        });
        app.update();
        assert_eq!(
            app.world().get::<GridPosition>(pop).unwrap(),
            &GridPosition { x: 50, y: 0 }
        );
        let ledger = app.world().resource::<QuantumCommuteLedger>();
        assert_eq!(ledger.total_jumps, 1);
        assert_eq!(ledger.refused_jumps, 1);
    }

    #[test]
    fn test_dissociation_expires_and_rest_speeds_recovery() {
        let mut app = App::new();
        app.add_systems(Update, temporal_dissociation_timer_system);
        let tired = app
            .world_mut()
            .spawn((
                TemporalDissociation { remaining_ticks: 3 },
                Needs {
                    rest: 0.2,
                    ..Default::default()
                },
            ))
            .id();
        let rested = app
            .world_mut()
            .spawn((
                TemporalDissociation { remaining_ticks: 3 },
                Needs {
                    rest: 0.9,
                    ..Default::default()
                },
            ))
            .id();
        app.update();
        app.update();
        // Rested pop recovered after 2 ticks (2/tick); tired has 1 left.
        assert!(app.world().get::<TemporalDissociation>(rested).is_none());
        assert_eq!(
            app.world()
                .get::<TemporalDissociation>(tired)
                .unwrap()
                .remaining_ticks,
            1
        );
        app.update();
        assert!(app.world().get::<TemporalDissociation>(tired).is_none());
    }

    #[test]
    fn test_utility_scoring_far_low_count_uses_pad_high_count_avoids() {
        let fresh = TeleportCount::default();
        assert!(should_use_quantum_pad(50, &fresh));
        assert!(!should_use_quantum_pad(5, &fresh)); // near: just walk
        let frequent = TeleportCount {
            jumps: DISSOCIATION_JUMP_THRESHOLD,
            ticks_since_jump: 0,
        };
        assert!(!should_use_quantum_pad(50, &frequent));
        assert_eq!(quantum_pad_score(50, &frequent), 0.0);
    }

    #[test]
    fn test_autonomous_system_routes_far_pop_through_pad() {
        let mut app = App::new();
        app.add_event::<QuantumTeleportEvent>();
        app.add_systems(
            Update,
            (
                quantum_commute_autonomous_system,
                quantum_commute_system,
            )
                .chain(),
        );
        let pad_a = app
            .world_mut()
            .spawn((GridPosition { x: 0, y: 0 }, QuantumPad::default()))
            .id();
        let pad_b = app
            .world_mut()
            .spawn((GridPosition { x: 40, y: 0 }, QuantumPad::default()))
            .id();
        app.world_mut().get_mut::<QuantumPad>(pad_a).unwrap().linked_pad = Some(pad_b);
        app.world_mut().get_mut::<QuantumPad>(pad_b).unwrap().linked_pad = Some(pad_a);

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                MovementTarget {
                    target_entity: Entity::PLACEHOLDER,
                    target_position: GridPosition { x: 42, y: 0 },
                    for_action: crate::layer1::utility_types::ActionType::Work,
                },
                StressTracker::default(),
                TeleportCount::default(),
            ))
            .id();

        app.update();
        assert_eq!(
            app.world().get::<GridPosition>(pop).unwrap(),
            &GridPosition { x: 40, y: 0 }
        );
        assert_eq!(app.world().get::<TeleportCount>(pop).unwrap().jumps, 1);
    }

    #[test]
    fn test_ip_guard_no_borrowed_terms() {
        // Concept-only (IP rule): public labels must be original.
        let banned = ["star trek", "stargate", "portal gun", "tardis"];
        let label = crate::layer1::architecture::building::BuildingType::QuantumPad.label();
        for term in banned {
            assert!(!label.to_lowercase().contains(term));
        }
        assert_eq!(label, "Quantum Pad");
    }
}
