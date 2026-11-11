//! Cryo-Shock system (Spec 413).
//!
//! Implements the Cryo-Shock status effect applied to Pops that spawn from cryo,
//! severely debuffing their movement and work speed. The effect naturally decays over time.
//!
//! Grounding (audited 2026-11-10 against the real seams, specs 360-412 pattern):
//! - Movement: there is no `MovementSpeed`/`EffectiveMovementSpeed` component
//!   in the real code (those were spec mocks). The unified pipeline is
//!   `pop::Speed { base, current }`: `reset_speed_system` restores
//!   `current = base` every tick, then modifier systems multiply `current`.
//!   [`apply_cryo_debuff_system`] is one of those modifiers (scheduled
//!   `.after(reset_speed_system)` in execution.rs) — no parallel component.
//! - Work: `work_execution_system` (execution/general_work.rs) multiplies a
//!   chain of per-module helper multipliers (traits, morale, `WorkSpeedBuff`,
//!   `MentalFog`, nostalgia, mundane cult). [`cryo_shock_work_multiplier`]
//!   is this module's leg of that same chain.
//! - Status effects: there is no central `StatusEffect` system in the code
//!   base — timed conditions are bespoke components with decay systems
//!   (e.g. `WorkSpeedBuff`, `MentalFog`). CryoShock follows that pattern.
//! - Spawns genuinely "from cryo": the beacon migrant bridge in
//!   core/integration.rs (pops arriving via ship — spec §1/§2) and the
//!   cryo-prison thaw in cryo_prison.rs (pops literally thawed out of
//!   suspension pods). Both attach `CryoShock::default()` at spawn.
//!   Starter pops, clone-vat growths, and awake passenger arrivals
//!   (pleasure-cruiser guests, couriered scions, refugee fleets) are NOT
//!   suspension arrivals and were deliberately left alone.
//! - Recovery beds: the real triage seam is a pop `AssignedTo` a `Hospital`
//!   entity with `AssignmentType::Patient` (the same seam
//!   `medical::healing_system` consumes). Patients recover
//!   [`CRYO_RECOVERY_DECAY_MULTIPLIER`]x faster in [`decay_cryo_shock_system`]
//!   (spec §5).

use bevy_ecs::prelude::*;

use crate::layer1::actions::{AssignedTo, AssignmentType};
use crate::layer1::medical::Hospital;
use crate::layer1::pop::{Pop, Speed};

/// Ticks a fresh cryo arrival stays shocked without treatment.
pub const CRYO_SHOCK_DURATION_TICKS: u64 = 250;
/// Fraction of movement/work speed a fresh cryo arrival loses (spec RED:
/// severity 0.5 halves speed).
pub const CRYO_SHOCK_SEVERITY: f32 = 0.5;
/// Spec 413 §5: being triaged in a medical bed multiplies the decay rate
/// (the spec's example is `delta * 5.0`).
pub const CRYO_RECOVERY_DECAY_MULTIPLIER: u64 = 5;

/// Applied to Pops that have just emerged from cryo suspension.
/// Causes severe debuffs to movement speed and efficiency that decay over time.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct CryoShock {
    /// Remaining duration of the shock in simulation ticks.
    pub duration_ticks: u64,
    /// Severity multiplier (0.0 to 1.0), where 1.0 reduces speed to 0.
    pub severity: f32,
}

impl Default for CryoShock {
    /// A fresh suspension arrival: full duration at standard severity.
    fn default() -> Self {
        Self {
            duration_ticks: CRYO_SHOCK_DURATION_TICKS,
            severity: CRYO_SHOCK_SEVERITY,
        }
    }
}

impl CryoShock {
    /// Remaining-speed fraction while shocked: severity 0.5 → half speed.
    #[must_use]
    pub fn speed_multiplier(&self) -> f32 {
        (1.0 - self.severity).clamp(0.0, 1.0)
    }

    /// Work-efficiency fraction while shocked (same severity curve as
    /// movement — one grogginess, two pipelines).
    #[must_use]
    pub fn work_speed_multiplier(&self) -> f32 {
        self.speed_multiplier()
    }
}

/// The work-pipeline leg: `work_execution_system` multiplies a worker's
/// speed by this factor (1.0 when the pop is not shocked).
#[must_use]
pub fn cryo_shock_work_multiplier(shock: Option<&CryoShock>) -> f32 {
    shock.map_or(1.0, CryoShock::work_speed_multiplier)
}

impl Pop {
    /// Spawns a new Pop with the CryoShock component applied.
    #[must_use]
    pub fn from_cryo() -> (Self, CryoShock) {
        (Self, CryoShock::default())
    }
}

/// Applies the movement speed debuff from the `CryoShock` component to Pops.
/// Runs after `reset_speed_system` in the real schedule, so the multiply
/// starts from `base` every tick and never compounds.
pub fn apply_cryo_debuff_system(mut query: Query<(&mut Speed, &CryoShock)>) {
    for (mut speed, shock) in query.iter_mut() {
        speed.current *= shock.speed_multiplier();
    }
}

/// Decays the duration of the `CryoShock` effect and removes it once it
/// expires. A pop triaged as a `Patient` at a real `Hospital` (the
/// recovery-bed seam `medical::healing_system` uses) decays
/// [`CRYO_RECOVERY_DECAY_MULTIPLIER`] ticks per tick; everyone else
/// decays naturally, one tick per tick (spec 413 §5).
pub fn decay_cryo_shock_system(
    mut commands: Commands,
    hospitals: Query<(), With<Hospital>>,
    mut query: Query<(Entity, &mut CryoShock, Option<&AssignedTo>)>,
) {
    for (entity, mut shock, assigned) in query.iter_mut() {
        let in_recovery_bed = assigned.is_some_and(|a| {
            a.assignment_type == AssignmentType::Patient && hospitals.get(a.entity).is_ok()
        });
        let decay = if in_recovery_bed {
            CRYO_RECOVERY_DECAY_MULTIPLIER
        } else {
            1
        };
        if shock.duration_ticks <= decay {
            commands.entity(entity).remove::<CryoShock>();
        } else {
            shock.duration_ticks -= decay;
        }
    }
}

/// Live census of pops still shaking off cryo shock (STATS `cryo=`,
/// full panel: headless `cryoshock`).
#[must_use]
pub fn cryo_shock_count(world: &mut World) -> usize {
    world.query::<&CryoShock>().iter(world).count()
}

/// STATS label for the cryo-shock census: the live count as a string.
#[must_use]
pub fn cryo_shock_stats_label(world: &mut World) -> String {
    cryo_shock_count(world).to_string()
}

/// Player-facing strings for this mechanic (IP-guard tested): original,
/// generic wording only.
#[must_use]
pub fn cryo_shock_public_strings() -> Vec<&'static str> {
    vec![
        "Cryo Shock",
        "cryo shock",
        "A new arrival stumbles out of suspension, groggy and half-frozen.",
        "Still shaking off cryo shock",
        "Recovered from cryo shock",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::{App, Update};

    #[test]
    fn test_cryo_shock_applied_on_spawn() {
        let mut app = App::new();

        let pop = app.world_mut().spawn(Pop::from_cryo()).id();
        app.update();

        assert!(
            app.world().get::<CryoShock>(pop).is_some(),
            "Pops spawned from cryo must have CryoShock"
        );
    }

    #[test]
    fn test_cryo_shock_applies_debuffs() {
        let mut app = App::new();

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                Speed {
                    base: 10.0,
                    current: 10.0,
                    accumulator: 0.0,
                },
                CryoShock {
                    duration_ticks: 100,
                    severity: 0.5,
                },
            ))
            .id();

        app.add_systems(Update, apply_cryo_debuff_system);
        app.update();

        let speed = app.world().get::<Speed>(pop).unwrap().current;
        assert!(
            speed < 10.0,
            "Cryo shock must reduce effective movement speed"
        );
        assert_eq!(speed, 5.0, "Cryo shock severity 0.5 halves speed");
    }

    #[test]
    fn test_cryo_shock_duration_decays() {
        let mut app = App::new();

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                CryoShock {
                    duration_ticks: 10,
                    severity: 0.5,
                },
            ))
            .id();

        app.add_systems(Update, decay_cryo_shock_system);

        app.update();

        let shock = app.world().get::<CryoShock>(pop).unwrap();
        assert!(
            shock.duration_ticks < 10,
            "Cryo shock duration must decay over time"
        );
        assert_eq!(shock.duration_ticks, 9);
    }

    #[test]
    fn test_cryo_shock_removed() {
        let mut app = App::new();

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                CryoShock {
                    duration_ticks: 1,
                    severity: 0.5,
                },
            ))
            .id();

        app.add_systems(Update, decay_cryo_shock_system);

        app.update();

        assert!(
            app.world().get::<CryoShock>(pop).is_none(),
            "CryoShock should be removed when duration reaches 0"
        );
    }

    // --- Spec 413 missing legs (2026-11-10) ---

    #[test]
    fn test_cryo_shock_work_speed_multiplier() {
        // Spec 413 §6: CryoShock severely debuffs WORK speed as well as
        // movement. The work pipeline multiplies by this factor.
        let shock = CryoShock {
            duration_ticks: 100,
            severity: 0.5,
        };
        assert_eq!(shock.work_speed_multiplier(), 0.5);
        assert_eq!(cryo_shock_work_multiplier(Some(&shock)), 0.5);
        assert_eq!(cryo_shock_work_multiplier(None), 1.0);
        // Severity is a fraction: a 0.9 shock leaves a tenth of work speed.
        let severe = CryoShock {
            duration_ticks: 100,
            severity: 0.9,
        };
        assert!((severe.work_speed_multiplier() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn test_cryo_shock_recovery_bed_accelerates_decay() {
        // Spec 413 §5: a pop triaged as a Patient at a real Hospital
        // (the recovery-bed seam) recovers faster than natural decay.
        use crate::layer1::actions::{AssignedTo, AssignmentType};
        use crate::layer1::medical::Hospital;

        let mut app = App::new();
        let hospital = app
            .world_mut()
            .spawn(Hospital::default())
            .id();
        let patient = app
            .world_mut()
            .spawn((
                Pop,
                CryoShock {
                    duration_ticks: 100,
                    severity: 0.5,
                },
                AssignedTo {
                    entity: hospital,
                    assignment_type: AssignmentType::Patient,
                },
            ))
            .id();
        let untreated = app
            .world_mut()
            .spawn((
                Pop,
                CryoShock {
                    duration_ticks: 100,
                    severity: 0.5,
                },
            ))
            .id();

        app.add_systems(Update, decay_cryo_shock_system);
        app.update();

        let patient_shock = app.world().get::<CryoShock>(patient).unwrap();
        assert_eq!(
            patient_shock.duration_ticks,
            100 - CRYO_RECOVERY_DECAY_MULTIPLIER,
            "Patient in a hospital bed must decay at the recovery multiplier"
        );
        let untreated_shock = app.world().get::<CryoShock>(untreated).unwrap();
        assert_eq!(
            untreated_shock.duration_ticks, 99,
            "Untreated pops decay naturally, one tick per tick"
        );
    }

    #[test]
    fn test_cryo_shock_patient_elsewhere_decays_normally() {
        // Guard: being a Patient of a non-Hospital entity (or a worker
        // assigned to a hospital) must NOT accelerate recovery.
        use crate::layer1::actions::{AssignedTo, AssignmentType};

        let mut app = App::new();
        let not_a_hospital = app.world_mut().spawn_empty().id();
        let pop = app
            .world_mut()
            .spawn((
                Pop,
                CryoShock {
                    duration_ticks: 100,
                    severity: 0.5,
                },
                AssignedTo {
                    entity: not_a_hospital,
                    assignment_type: AssignmentType::Patient,
                },
            ))
            .id();

        app.add_systems(Update, decay_cryo_shock_system);
        app.update();

        assert_eq!(
            app.world().get::<CryoShock>(pop).unwrap().duration_ticks,
            99,
            "Assignment to a non-hospital must not accelerate cryo recovery"
        );
    }

    #[test]
    fn test_cryo_shock_recovery_bed_can_finish_recovery() {
        // A patient whose remaining shock is within one accelerated
        // step fully recovers (component removed).
        use crate::layer1::actions::{AssignedTo, AssignmentType};
        use crate::layer1::medical::Hospital;

        let mut app = App::new();
        let hospital = app.world_mut().spawn(Hospital::default()).id();
        let patient = app
            .world_mut()
            .spawn((
                Pop,
                CryoShock {
                    duration_ticks: 3,
                    severity: 0.5,
                },
                AssignedTo {
                    entity: hospital,
                    assignment_type: AssignmentType::Patient,
                },
            ))
            .id();

        app.add_systems(Update, decay_cryo_shock_system);
        app.update();

        assert!(
            app.world().get::<CryoShock>(patient).is_none(),
            "Accelerated decay must be able to finish recovery"
        );
    }

    #[test]
    fn test_migrant_arrival_spawns_with_cryo_shock() {
        // Spec 413 §6: pops arriving via ship (the beacon migrant
        // bridge) spawn with CryoShock.
        use crate::layer1::core::chronicle::AddChronicleEvent;
        use crate::layer1::economy::remittances::MigrantArrivalEvent;
        use crate::layer1::integration::{beacon_migrant_arrival_bridge, RecentArrival};
        use crate::layer1::map::GridPosition;

        let mut app = App::new();
        app.add_event::<MigrantArrivalEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(Update, beacon_migrant_arrival_bridge);
        // A living colonist so migrants land on a colony tile.
        app.world_mut()
            .spawn(crate::layer1::PopBundle::random(5, 5, &mut rand::thread_rng()));
        app.world_mut()
            .resource_mut::<bevy_ecs::event::Events<MigrantArrivalEvent>>()
            .send(MigrantArrivalEvent {
                count: 2,
                ..Default::default()
            });
        app.update();
        app.update();

        let mut query = app
            .world_mut()
            .query_filtered::<&CryoShock, With<RecentArrival>>();
        let shocked: Vec<_> = query.iter(app.world()).collect();
        assert_eq!(
            shocked.len(),
            2,
            "Every beacon migrant must arrive with CryoShock"
        );
        assert!(shocked.iter().all(|s| s.severity == 0.5));
        let _ = GridPosition { x: 0, y: 0 };
    }

    #[test]
    fn test_thawed_cryo_prisoner_spawns_with_cryo_shock() {
        // Spec 413 §6: a pop thawed out of a cryo-prison pod is
        // genuinely "from cryo" and spawns with CryoShock.
        use crate::layer1::cryo_prison::{thaw_cryo_pod_system, CriminalSpecialty, CryoPod};
        use crate::layer1::social::unrest::Unrest;

        let mut app = App::new();
        app.insert_resource(Unrest::default());
        app.add_systems(Update, thaw_cryo_pod_system);
        app.world_mut().spawn(CryoPod {
            thaw_progress: 1.0,
            is_criminal: true,
            specialty: CriminalSpecialty::Engineer,
        });
        app.update();

        let mut query = app.world_mut().query::<&CryoShock>();
        assert_eq!(
            query.iter(app.world()).count(),
            1,
            "Thawed cryo-prisoner must spawn with CryoShock"
        );
    }

    #[test]
    fn test_cryo_shock_count_and_label() {
        use bevy_ecs::world::World;
        let mut world = World::new();
        assert_eq!(cryo_shock_count(&mut world), 0);
        world.spawn((
            Pop,
            CryoShock {
                duration_ticks: 10,
                severity: 0.5,
            },
        ));
        world.spawn(Pop);
        assert_eq!(cryo_shock_count(&mut world), 1);
        assert_eq!(cryo_shock_stats_label(&mut world), "1");
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Cryo Shock is a generic suspension-aftermath trope: player-
        // facing strings use original/generic wording only.
        let banned = [
            "sad king billy",
            "windsor",
            "hibernation sickness",
            "carbonite",
            "han solo",
            "ripley",
            "nostromo",
            "buck rogers",
            "fry",
            "futurama",
            "demolition man",
            "stallone",
        ];
        for s in cryo_shock_public_strings() {
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
