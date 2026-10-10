//! The Empathy Broadcast (Spec 305).
//!
//! A cross-layer (Layer 3 -> Layer 1) feature: a desperate neighbouring
//! empire broadcasts its population's terror as a psychic distress wave.
//! Every colony pop receives an unavoidable [`ActiveEmpathyBroadcast`],
//! accumulating [`StressTracker::accumulated_stress`] every tick for the
//! broadcast's duration and posting a typed
//! [`GrievanceType::DemandIntervention`] grievance demanding military
//! intervention on behalf of the broadcasting faction. This forces the
//! player to weigh cold geopolitical neutrality against internal collapse.
//!
//! ## Design notes (adaptations of the spec's mock-style RED tests)
//!
//! * Spec 305's dependency (Spec 233) `GrievanceTracker`/`GrievanceType`
//!   never landed as typed components — 233 landed as the bulletin-board
//!   system in [`crate::layer1::social::grievances`]. The typed grievance
//!   tracker is therefore introduced here, in this module, with exactly the
//!   variant the spec needs ([`GrievanceType::DemandIntervention`]); future
//!   specs can extend the enum.
//! * The spec's `src/layer1/stress.rs` / `src/layer1/grievance.rs` paths are
//!   stale — the real stress module is `crate::layer1::stress` (the glob
//!   re-export of `crate::layer1::psychology::stress`).
//! * All names are original and generic; nothing is lifted from outside
//!   fiction (see the IP-guard test).

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::entities::pop::Pop;
use crate::layer1::stress::StressTracker;
use bevy::prelude::*;

/// Fired when a neighbouring empire's distress broadcast reaches the colony.
///
/// A rare, global event: every pop with a [`StressTracker`] is saturated
/// regardless of walls, acoustics, or cover.
#[derive(Event, Clone, Debug)]
pub struct EmpathyBroadcastEvent {
    /// How many ticks the broadcast persists on each affected pop.
    pub duration_ticks: u32,
    /// Stress added to each affected pop per tick (unavoidable).
    pub intensity: f32,
    /// The broadcasting faction — the target of the intervention demand.
    pub target_faction_id: u32,
}

/// Marks a pop currently saturated by an empathy broadcast.
#[derive(Component, Clone, Debug)]
pub struct ActiveEmpathyBroadcast {
    /// Ticks remaining before the saturation lifts.
    pub remaining_ticks: u32,
    /// Stress applied per tick while saturated.
    pub intensity: f32,
    /// The faction whose suffering is being felt.
    pub target_faction_id: u32,
}

/// A typed grievance a pop holds against the colony leadership.
///
/// Introduced for Spec 305 (Spec 233's typed system never landed); the enum
/// is intentionally general so future specs can add variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrievanceType {
    /// The pop demands military intervention on behalf of the given faction.
    DemandIntervention(u32),
}

/// Per-pop ledger of unresolved grievances against the colony leadership.
#[derive(Component, Debug, Default)]
pub struct GrievanceTracker {
    /// Grievances currently posted (no duplicates).
    pub grievances: Vec<GrievanceType>,
}

impl GrievanceTracker {
    /// Posts a grievance unless an identical one is already present.
    pub fn add_grievance(&mut self, kind: GrievanceType) {
        if !self.has_grievance(kind) {
            self.grievances.push(kind);
        }
    }

    /// True if an identical grievance is already posted.
    pub fn has_grievance(&self, kind: GrievanceType) -> bool {
        self.grievances.contains(&kind)
    }

    /// Clears one grievance kind; returns true if anything was removed.
    pub fn clear_grievance(&mut self, kind: GrievanceType) -> bool {
        let before = self.grievances.len();
        self.grievances.retain(|g| *g != kind);
        self.grievances.len() < before
    }
}

/// On broadcast arrival, saturate EVERY colony pop — the wave is psychic
/// and global, bypassing walls, acoustics, and cover. Pops that lack the
/// tracking components gain them first (fresh trackers, zeroed state), so
/// the stress and the grievance actually land. Then writes a Major
/// chronicle entry naming the crisis.
#[allow(clippy::type_complexity)]
pub fn process_empathy_broadcast_system(
    mut commands: Commands,
    mut events: EventReader<EmpathyBroadcastEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    query: Query<(Entity, Option<&StressTracker>, Option<&GrievanceTracker>), With<Pop>>,
) {
    for ev in events.read() {
        let mut affected = 0;
        for (entity, stress, tracker) in query.iter() {
            if stress.is_none() {
                commands.entity(entity).insert(StressTracker::default());
            }
            // Guarantee the grievance has somewhere to land: a pop with no
            // tracker gets a fresh one. (The update system only posts into
            // an existing tracker.)
            if tracker.is_none() {
                commands
                    .entity(entity)
                    .insert(GrievanceTracker::default());
            }
            commands.entity(entity).insert(ActiveEmpathyBroadcast {
                remaining_ticks: ev.duration_ticks,
                intensity: ev.intensity,
                target_faction_id: ev.target_faction_id,
            });
            affected += 1;
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "EMPATHY BROADCAST: a psychic distress wave from faction {} saturates {} colonists — every pop feels the terror of strangers. The people demand intervention.",
                ev.target_faction_id, affected
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Applies broadcast stress each tick, posts the intervention grievance
/// once per pop (no duplicates), ticks the broadcast down, and removes
/// it when it expires.
pub fn update_broadcast_stress_system(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &mut StressTracker,
        Option<&mut GrievanceTracker>,
        &mut ActiveEmpathyBroadcast,
    )>,
) {
    for (entity, mut stress, tracker, mut broadcast) in query.iter_mut() {
        stress.accumulated_stress += broadcast.intensity;

        if let Some(mut tracker) = tracker {
            tracker.add_grievance(GrievanceType::DemandIntervention(
                broadcast.target_faction_id,
            ));
        }

        broadcast.remaining_ticks = broadcast.remaining_ticks.saturating_sub(1);
        if broadcast.remaining_ticks == 0 {
            commands.entity(entity).remove::<ActiveEmpathyBroadcast>();
        }
    }
}

/// Number of pops currently saturated by an empathy broadcast.
pub fn empathy_broadcast_pop_count(world: &mut World) -> usize {
    world
        .query_filtered::<Entity, With<ActiveEmpathyBroadcast>>()
        .iter(world)
        .count()
}

/// One-line STATS label for the active broadcast, or "none".
pub fn empathy_broadcast_status_label(world: &mut World) -> String {
    let mut q = world.query::<&ActiveEmpathyBroadcast>();
    let broadcasts: Vec<(u32, u32)> = q
        .iter(world)
        .map(|b| (b.target_faction_id, b.remaining_ticks))
        .collect();
    if broadcasts.is_empty() {
        return "none".to_string();
    }
    let max_ticks = broadcasts.iter().map(|(_, t)| *t).max().unwrap_or(0);
    let faction = broadcasts[0].0;
    format!(
        "pops={} faction={} max_ticks={}",
        broadcasts.len(),
        faction,
        max_ticks
    )
}

/// Public-facing strings, for the IP-guard test.
pub fn empathy_broadcast_public_strings() -> Vec<String> {
    vec![
        "EMPATHY BROADCAST".to_string(),
        "psychic distress wave".to_string(),
        "every pop feels the terror of strangers".to_string(),
        "The people demand intervention.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_app() -> App {
        let mut app = App::new();
        app.add_event::<EmpathyBroadcastEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(
            Update,
            (
                process_empathy_broadcast_system,
                update_broadcast_stress_system,
            )
                .chain(),
        );
        app
    }

    #[test]
    fn test_broadcast_event_applies_component() {
        let mut app = setup_app();

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                StressTracker {
                    accumulated_stress: 10.0,
                },
            ))
            .id();

        app.world_mut()
            .resource_mut::<Events<EmpathyBroadcastEvent>>()
            .send(EmpathyBroadcastEvent {
                duration_ticks: 100,
                intensity: 2.0,
                target_faction_id: 42,
            });

        app.update();

        // Pop should now have the ActiveEmpathyBroadcast component.
        // Note: the systems are chained, so the update system already ran
        // once in the same update — one tick of stress applied and the
        // duration ticked down from 100 to 99.
        assert!(app
            .world()
            .get::<ActiveEmpathyBroadcast>(pop)
            .is_some());
        let broadcast = app.world().get::<ActiveEmpathyBroadcast>(pop).unwrap();
        assert_eq!(broadcast.remaining_ticks, 99);
        assert_eq!(broadcast.intensity, 2.0);
        assert_eq!(broadcast.target_faction_id, 42);
        let stress = app.world().get::<StressTracker>(pop).unwrap();
        assert_eq!(stress.accumulated_stress, 12.0);
    }

    #[test]
    fn test_broadcast_increases_stress_and_spawns_grievance() {
        let mut app = setup_app();

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                StressTracker {
                    accumulated_stress: 10.0,
                },
                GrievanceTracker::default(),
                ActiveEmpathyBroadcast {
                    remaining_ticks: 10,
                    intensity: 5.0,
                    target_faction_id: 42,
                },
            ))
            .id();

        app.update();

        // Stress increased by intensity
        let stress = app.world().get::<StressTracker>(pop).unwrap();
        assert_eq!(stress.accumulated_stress, 15.0);

        // Broadcast duration decreased
        let broadcast = app.world().get::<ActiveEmpathyBroadcast>(pop).unwrap();
        assert_eq!(broadcast.remaining_ticks, 9);

        // Should spawn a grievance
        let grievances = app.world().get::<GrievanceTracker>(pop).unwrap();
        assert!(grievances.has_grievance(GrievanceType::DemandIntervention(42)));
    }

    #[test]
    fn test_broadcast_expires() {
        let mut app = setup_app();

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                StressTracker {
                    accumulated_stress: 50.0,
                },
                ActiveEmpathyBroadcast {
                    remaining_ticks: 1,
                    intensity: 5.0,
                    target_faction_id: 42,
                },
            ))
            .id();

        app.update();

        // Component should be removed after remaining_ticks reaches 0
        assert!(app
            .world()
            .get::<ActiveEmpathyBroadcast>(pop)
            .is_none());
    }

    #[test]
    fn test_grievance_not_duplicated_across_ticks() {
        let mut app = setup_app();

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                StressTracker {
                    accumulated_stress: 0.0,
                },
                GrievanceTracker::default(),
                ActiveEmpathyBroadcast {
                    remaining_ticks: 5,
                    intensity: 2.0,
                    target_faction_id: 7,
                },
            ))
            .id();

        for _ in 0..3 {
            app.update();
        }

        let grievances = app.world().get::<GrievanceTracker>(pop).unwrap();
        let intervention = grievances
            .grievances
            .iter()
            .filter(|g| matches!(g, GrievanceType::DemandIntervention(7)))
            .count();
        assert_eq!(intervention, 1, "grievance must not duplicate");
        // Stress accumulated every tick.
        let stress = app.world().get::<StressTracker>(pop).unwrap();
        assert_eq!(stress.accumulated_stress, 6.0);
    }

    #[test]
    fn test_broadcast_guarantees_tracker_components_on_bare_pops() {
        let mut app = setup_app();

        // A bare pop with no tracker components at all (the common
        // real-world case — nothing spawns these trackers yet).
        let pop = app.world_mut().spawn(Pop).id();

        app.world_mut().send_event(EmpathyBroadcastEvent {
            duration_ticks: 5,
            intensity: 1.0,
            target_faction_id: 11,
        });

        app.update(); // process inserts, then update runs same tick
        assert!(
            app.world().get::<StressTracker>(pop).is_some(),
            "pop should have gained a stress tracker"
        );
        assert!(
            app.world().get::<GrievanceTracker>(pop).is_some(),
            "pop should have gained a grievance tracker"
        );

        // Stress applied and grievance posted in the same chained update.
        let tracker = app.world().get::<GrievanceTracker>(pop).unwrap();
        assert!(tracker.has_grievance(GrievanceType::DemandIntervention(11)));
        let stress = app.world().get::<StressTracker>(pop).unwrap();
        assert_eq!(stress.accumulated_stress, 1.0);
    }

    #[test]
    fn test_broadcast_writes_major_chronicle_entry() {
        let mut app = setup_app();

        app.world_mut().spawn((Pop, StressTracker::default()));

        app.world_mut().send_event(EmpathyBroadcastEvent {
            duration_ticks: 10,
            intensity: 1.0,
            target_faction_id: 99,
        });

        app.update();

        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let chronicles: Vec<_> = reader.read(events).collect();
        assert_eq!(chronicles.len(), 1);
        assert!(matches!(
            chronicles[0].importance,
            EventImportance::Major
        ));
        assert!(chronicles[0].text.contains("EMPATHY BROADCAST"));
        assert!(chronicles[0].text.contains("99"));
    }

    #[test]
    fn test_grievance_tracker_add_and_clear() {
        let mut tracker = GrievanceTracker::default();
        assert!(!tracker.has_grievance(GrievanceType::DemandIntervention(3)));
        tracker.add_grievance(GrievanceType::DemandIntervention(3));
        tracker.add_grievance(GrievanceType::DemandIntervention(3));
        assert_eq!(tracker.grievances.len(), 1);
        tracker.add_grievance(GrievanceType::DemandIntervention(4));
        assert_eq!(tracker.grievances.len(), 2);
        assert!(tracker.clear_grievance(GrievanceType::DemandIntervention(3)));
        assert!(!tracker.has_grievance(GrievanceType::DemandIntervention(3)));
        assert!(!tracker.clear_grievance(GrievanceType::DemandIntervention(3)));
    }

    // --- IP guard ---

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // The Empathy Broadcast is concept-only: generic "psychic distress
        // wave" / "faction" naming, nothing lifted from outside fiction.
        const BANNED: &[&str] = &[
            "dune",
            "bene gesserit",
            "arrakis",
            "atreides",
            "harkonnen",
            "fremen",
            "mentat",
            "sardaukar",
            "corrino",
            "imperium",
            "jedi",
            "sith",
            "midichlorian",
            "betazoid",
            "vulcan",
            "psyker",
            "warhammer",
            "sad king billy",
            "windsor",
        ];
        for s in empathy_broadcast_public_strings() {
            let lower = s.to_lowercase();
            for b in BANNED {
                assert!(
                    !lower.contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
