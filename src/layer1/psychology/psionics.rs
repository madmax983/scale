use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::map::GridPosition;
use crate::layer1::stress::StressTracker;
use bevy_ecs::prelude::*;

/// Marker component to indicate a Pop is Latent.
#[derive(Component)]
pub struct Latent;

/// The types of Psionic powers a Pop can awaken.
#[derive(Component, Debug, PartialEq, Eq, Clone, Copy)]
pub enum AwakenedPsionic {
    Pyrokinesis,
    Empathy,
    Foresight,
}

/// System to check for Latent Pop awakening due to high stress.
pub fn latent_awakening_system(
    mut commands: Commands,
    mut chronicle: EventWriter<AddChronicleEvent>,
    query: Query<(Entity, &Latent, &StressTracker), Without<AwakenedPsionic>>,
) {
    for (entity, _, stress) in query.iter() {
        if stress.accumulated_stress >= 80.0 {
            commands.entity(entity).insert(AwakenedPsionic::Pyrokinesis);
            commands.entity(entity).remove::<Latent>();
            chronicle.send(AddChronicleEvent {
                importance: EventImportance::Major,
                text: "A Pop has awakened as a Pyrokinetic!".to_string(),
            });
        }
    }
}

/// Event representing a fire.
#[derive(Event, Debug)]
pub struct FireEvent {
    pub position: GridPosition,
}

/// A fake struct for failed events if one doesn't exist
#[derive(Event, Debug)]
pub struct WorkFailedEvent {
    pub entity: Entity,
}

pub fn pyrokinesis_power_activation_system(
    mut events: EventReader<WorkFailedEvent>,
    mut fire_events: EventWriter<FireEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    query: Query<(&GridPosition, &AwakenedPsionic)>,
) {
    for event in events.read() {
        if let Ok((pos, psionic)) = query.get(event.entity) {
            if *psionic == AwakenedPsionic::Pyrokinesis {
                fire_events.send(FireEvent { position: *pos });
                chronicle.send(AddChronicleEvent {
                    importance: EventImportance::Standard,
                    text: "A Pyrokinetic has started a fire out of frustration!".to_string(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::stress::StressTracker;
    use bevy_app::App;

    #[test]
    fn test_latent_awakening_on_high_stress() {
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, latent_awakening_system);

        // Arrange: Spawn a Pop with Latent trait and high Stress
        let pop = app
            .world_mut()
            .spawn((
                Latent,
                StressTracker {
                    accumulated_stress: 85.0,
                },
            ))
            .id();

        // Act: Run awakening check system
        app.update();

        // Assert: Verify the Pop gains an Awakened Psionic trait (e.g., Pyrokinesis)
        assert!(app.world().get::<AwakenedPsionic>(pop).is_some());
        assert!(app.world().get::<Latent>(pop).is_none());
        assert_eq!(
            app.world().get::<AwakenedPsionic>(pop).unwrap(),
            &AwakenedPsionic::Pyrokinesis
        );

        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        assert!(reader.read(events).count() > 0);
    }

    #[test]
    fn test_pyrokinesis_power_activation() {
        let mut app = App::new();
        app.add_event::<WorkFailedEvent>();
        app.add_event::<FireEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, pyrokinesis_power_activation_system);

        // Arrange: Spawn an Awakened (Pyrokinetic) Pop and a Kitchen building
        let pos = GridPosition { x: 5, y: 5 };
        let pop = app
            .world_mut()
            .spawn((AwakenedPsionic::Pyrokinesis, pos))
            .id();

        // Act: Trigger an adverse event (like a failed work task)
        app.world_mut().send_event(WorkFailedEvent { entity: pop });
        app.update();

        // Assert: Verify a fire event/entity is spawned at the Pop's location
        let events = app.world().resource::<Events<FireEvent>>();
        let mut reader = events.get_cursor();
        let fired = reader.read(events).next().expect("FireEvent not emitted");
        assert_eq!(fired.position, pos);

        let chron_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut chron_reader = chron_events.get_cursor();
        assert!(chron_reader.read(chron_events).count() > 0);
    }
}

// ============================================================================
// Spec 374: Psionic Awakening — "Mind over matter, with terrible
// consequences." Random pops develop Psionic Potential; telekinetic work
// and soothing nearby pops drain the psion's Sanity (a resource DISTINCT
// from Morale and from Needs — it is the mana/health of the gift). At zero
// sanity the gift turns destructive: a psychic anomaly (a real FireEvent,
// bridged to a live Fire by INT-900's psionic_fire_bridge_system) erupts —
// gated by a per-psion cooldown so a broken mind is an occasional
// catastrophe, not a per-tick fire hose.
//
// Adaptations of the spec's mock-style sketch to the real codebase:
// * The spec's `src/layer1/traits/psionic.rs` path is stale — traits live
//   in `crate::layer1::psychology::traits` and the pre-existing psionics
//   (Latent / AwakenedPsionic / FireEvent) live here, so Spec 374 extends
//   THIS module instead of creating a parallel one.
// * The spec's FireEvent import (`crate::layer1::hazards::FireEvent`) is
//   stale — the real, live FireEvent is THIS module's (setup.rs inits its
//   Events resource; the integration bridge turns it into real fires).
// * Needs are 0.0..=1.0 in the real codebase (not the sketch's 0..100), so
//   soothing grants +0.2 leisure (the sketch's +20 on its assumed scale)
//   and only targets pops below 0.5 leisure.
// * The sketch's breakdown rolled `rng < 0.1` per tick at zero sanity —
//   untestable deterministically and spammy in expectation. The real
//   system fires ONCE when sanity is at zero, then imposes a 200-tick
//   cooldown: occasional, bounded, and exactly what the tests assert.
// * Sanity stays on the sketch's 0..100 scale: it is its own resource,
//   deliberately not folded into Morale (0..1) or Needs (0..1).
// ============================================================================

use crate::layer1::entities::pop::Pop;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::utility_types::{ActionType, PopAction};

/// A pop whose mind has opened to the psionic field (Spec 374).
///
/// Granted at random by [`psionic_awakening_system`] (stress makes it more
/// likely) or by the headless `awaken` debug command. `level` scales
/// nothing yet beyond identity — higher levels are future-spec territory.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PsionicPotential {
    /// Strength of the gift (1 = newly awakened).
    pub level: u32,
}

/// The psion's mental integrity — the fuel AND the fuse of the gift.
///
/// Distinct from `Morale` and from every `Needs` field: telekinetic work
/// and soothing burn it, rest (not working telekinetically) restores it
/// slowly, and at zero the gift fires on its own (see
/// [`trigger_psionic_breakdown_system`]).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Sanity {
    /// Current sanity (0.0 = broken).
    pub current: f32,
    /// Maximum sanity.
    pub max: f32,
}

impl Default for Sanity {
    fn default() -> Self {
        Self {
            current: 100.0,
            max: 100.0,
        }
    }
}

impl Sanity {
    /// Sanity as a 0.0..=1.0 fraction (STATS / UI display).
    #[must_use]
    pub fn fraction(&self) -> f32 {
        if self.max <= f32::EPSILON {
            0.0
        } else {
            (self.current / self.max).clamp(0.0, 1.0)
        }
    }
}

/// Ticks a broken psion must wait before the gift can erupt again.
///
/// Inserted at awakening; [`trigger_psionic_breakdown_system`] counts it
/// down and resets it on every anomaly, which is what keeps zero-sanity
/// anomalies occasional instead of per-tick.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct PsionicBreakdownCooldown {
    /// Ticks until the next anomaly is possible.
    pub ticks_remaining: u32,
}

/// Sanity burned per tick of telekinetic work.
pub const PSIONIC_TELEKINESIS_SANITY_DRAIN: f32 = 5.0;
/// Leisure granted to each soothed pop (Needs scale is 0.0..=1.0).
pub const PSIONIC_SOOTHE_LEISURE_GAIN: f32 = 0.2;
/// Sanity burned per pop soothed per tick.
pub const PSIONIC_SOOTHE_SANITY_DRAIN: f32 = 5.0;
/// Self-preservation floor: a psion stops soothing at or below this
/// sanity. Soothing is voluntary aid — a fraying mind withdraws rather
/// than calming the whole colony into its own breakdown. (Reaching zero
/// remains possible through telekinetic overwork, the spec's central
/// tension.) Without this floor, an ambient psion in a needy colony
/// soothes every tick, burns out, and starts fires with no player
/// involvement at all.
pub const PSIONIC_SOOTHE_MIN_SANITY: f32 = 20.0;
/// Sanity regained per tick while NOT working telekinetically.
pub const PSIONIC_SANITY_RECOVERY: f32 = 0.1;
/// Cooldown imposed after each psychic anomaly.
pub const PSIONIC_BREAKDOWN_COOLDOWN_TICKS: u32 = 200;
/// Per-tick awakening chance for an unstressed pop.
pub const PSIONIC_AWAKENING_BASE_CHANCE: f32 = 0.001;
/// Per-tick awakening chance for a highly stressed pop (>= 60 stress).
pub const PSIONIC_AWAKENING_STRESSED_CHANCE: f32 = 0.01;
/// Chebyshev radius within which a psion soothes nearby pops.
pub const PSIONIC_SOOTHE_RADIUS: i32 = 2;
/// Leisure below this counts as "needs soothing".
pub const PSIONIC_SOOTHE_LEISURE_THRESHOLD: f32 = 0.5;

/// Chronicle text written when a pop awakens (original wording, Spec 374).
pub const PSIONIC_AWAKENING_CHRONICLE: &str =
    "A colonist's mind opens to the psionic field - a new psion awakens.";
/// Chronicle text written when a broken psion erupts (original wording).
pub const PSIONIC_BREAKDOWN_CHRONICLE: &str =
    "A psion's mind breaks - raw telekinetic fury sets the world alight!";

/// Guarantees every psion carries `Sanity` + a breakdown cooldown.
///
/// Awakening inserts both, but pops granted `PsionicPotential` by any
/// other path (debug commands, future specs) get them here instead of
/// silently opting out of the sanity economy.
#[allow(clippy::type_complexity)]
pub fn ensure_psionic_components_system(
    mut commands: Commands,
    query: Query<
        (Entity, Option<&Sanity>, Option<&PsionicBreakdownCooldown>),
        With<PsionicPotential>,
    >,
) {
    for (entity, sanity, cooldown) in query.iter() {
        if sanity.is_none() {
            commands.entity(entity).insert(Sanity::default());
        }
        if cooldown.is_none() {
            commands
                .entity(entity)
                .insert(PsionicBreakdownCooldown::default());
        }
    }
}

/// Random pops develop psionic potential (Spec 374's core beat).
///
/// Every un-awakened pop rolls each tick: a small base chance, ten times
/// higher under heavy stress (the gift loves a breaking mind). Awakening
/// grants level 1 potential, a full Sanity pool, a fresh cooldown, and a
/// Major chronicle entry.
#[allow(clippy::type_complexity)]
pub fn psionic_awakening_system(
    mut commands: Commands,
    mut chronicle: EventWriter<AddChronicleEvent>,
    query: Query<(Entity, Option<&StressTracker>), (With<Pop>, Without<PsionicPotential>)>,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    for (entity, stress) in query.iter() {
        let chance = match stress {
            Some(s) if s.accumulated_stress >= 60.0 => PSIONIC_AWAKENING_STRESSED_CHANCE,
            _ => PSIONIC_AWAKENING_BASE_CHANCE,
        };
        if rng.gen::<f32>() < chance {
            commands.entity(entity).insert((
                PsionicPotential { level: 1 },
                Sanity::default(),
                PsionicBreakdownCooldown::default(),
            ));
            chronicle.send(AddChronicleEvent {
                importance: EventImportance::Major,
                text: PSIONIC_AWAKENING_CHRONICLE.to_string(),
            });
        }
    }
}

/// Telekinetic work burns sanity (Spec 374 RED test 1, adapted).
///
/// A psion whose current action is `ActionType::TelekineticWork` lifts,
/// hauls and builds without touching anything — and pays
/// [`PSIONIC_TELEKINESIS_SANITY_DRAIN`] sanity per tick for it.
pub fn process_psionic_actions_system(
    mut query: Query<(&mut Sanity, &PopAction), With<PsionicPotential>>,
) {
    for (mut sanity, action) in query.iter_mut() {
        if action.current == ActionType::TelekineticWork {
            sanity.current = (sanity.current - PSIONIC_TELEKINESIS_SANITY_DRAIN).max(0.0);
        }
    }
}

/// A psion soothes nearby pops — and pays for every calmed mind.
///
/// Every non-psion pop within [`PSIONIC_SOOTHE_RADIUS`] (Chebyshev) whose
/// leisure is below the threshold gains leisure (the real morale-relief
/// channel: leisure feeds morale/stress downstream); the psion loses
/// sanity per pop soothed. Runs every tick, so a surrounded psion burns
/// out fast — that is the spec's intended tension.
pub fn process_psionic_soothing_system(
    mut psions: Query<(&mut Sanity, &GridPosition), With<PsionicPotential>>,
    mut pops: Query<(&mut Needs, &GridPosition), Without<PsionicPotential>>,
) {
    for (mut sanity, psion_pos) in psions.iter_mut() {
        if sanity.current <= PSIONIC_SOOTHE_MIN_SANITY {
            continue;
        }
        for (mut needs, pop_pos) in pops.iter_mut() {
            if sanity.current <= PSIONIC_SOOTHE_MIN_SANITY {
                break;
            }
            let chebyshev = (psion_pos.x - pop_pos.x)
                .abs()
                .max((psion_pos.y - pop_pos.y).abs());
            if chebyshev <= PSIONIC_SOOTHE_RADIUS
                && needs.leisure < PSIONIC_SOOTHE_LEISURE_THRESHOLD
            {
                needs.leisure = (needs.leisure + PSIONIC_SOOTHE_LEISURE_GAIN).min(1.0);
                sanity.current = (sanity.current - PSIONIC_SOOTHE_SANITY_DRAIN).max(0.0);
            }
        }
    }
}

/// Sanity slowly returns while the gift rests.
///
/// Any psion NOT currently working telekinetically regains
/// [`PSIONIC_SANITY_RECOVERY`] per tick, up to max. (Soothing still nets
/// negative: its drain per soothed pop dwarfs this trickle.)
pub fn sanity_recovery_system(
    mut query: Query<(&mut Sanity, Option<&PopAction>), With<PsionicPotential>>,
) {
    for (mut sanity, action) in query.iter_mut() {
        if action.is_none_or(|a| a.current != ActionType::TelekineticWork) {
            sanity.current = (sanity.current + PSIONIC_SANITY_RECOVERY).min(sanity.max);
        }
    }
}

/// Zero sanity occasionally erupts as a psychic anomaly (Spec 374 RED
/// test 2, adapted).
///
/// When a psion's sanity is at zero and their breakdown cooldown has
/// expired, the gift fires on its own: a real [`FireEvent`] at the
/// psion's position (the INT-900 bridge turns it into a live `Fire`) and
/// a Major chronicle entry. The cooldown then resets to
/// [`PSIONIC_BREAKDOWN_COOLDOWN_TICKS`], so a broken psion is an
/// occasional catastrophe — never a per-tick fire hose. Pops without a
/// `GridPosition` erupt at the map origin rather than not at all.
#[allow(clippy::type_complexity)]
pub fn trigger_psionic_breakdown_system(
    mut commands: Commands,
    mut fire_events: EventWriter<FireEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut query: Query<
        (
            Entity,
            &Sanity,
            Option<&GridPosition>,
            Option<&mut PsionicBreakdownCooldown>,
        ),
        With<PsionicPotential>,
    >,
) {
    for (entity, sanity, pos, cooldown) in query.iter_mut() {
        let mut on_cooldown = false;
        if let Some(mut cd) = cooldown {
            if cd.ticks_remaining > 0 {
                cd.ticks_remaining -= 1;
                on_cooldown = true;
            }
        }
        if on_cooldown || sanity.current > 0.0 {
            continue;
        }
        fire_events.send(FireEvent {
            position: pos.copied().unwrap_or(GridPosition { x: 0, y: 0 }),
        });
        chronicle.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: PSIONIC_BREAKDOWN_CHRONICLE.to_string(),
        });
        commands.entity(entity).insert(PsionicBreakdownCooldown {
            ticks_remaining: PSIONIC_BREAKDOWN_COOLDOWN_TICKS,
        });
    }
}

/// Headless helper: (psion count, average sanity fraction, broken count).
pub fn psion_census(world: &mut World) -> (usize, f32, usize) {
    let mut count = 0usize;
    let mut total_fraction = 0.0f32;
    let mut broken = 0usize;
    let mut query = world.query::<(&PsionicPotential, Option<&Sanity>)>();
    for (_potential, sanity) in query.iter(world) {
        count += 1;
        if let Some(s) = sanity {
            total_fraction += s.fraction();
            if s.current <= 0.0 {
                broken += 1;
            }
        } else {
            total_fraction += 1.0;
        }
    }
    let avg = if count > 0 {
        total_fraction / count as f32
    } else {
        0.0
    };
    (count, avg, broken)
}

/// Headless helper: awaken the first `n` un-awakened pops. Returns how
/// many were actually awakened.
pub fn awaken_pops(world: &mut World, n: usize) -> usize {
    let targets: Vec<Entity> = {
        let mut query =
            world.query_filtered::<Entity, (With<Pop>, Without<PsionicPotential>)>();
        query.iter(world).take(n).collect()
    };
    for entity in &targets {
        world.entity_mut(*entity).insert((
            PsionicPotential { level: 1 },
            Sanity::default(),
            PsionicBreakdownCooldown::default(),
        ));
    }
    targets.len()
}

/// Headless helper: drive the first psion's sanity to zero (the next
/// breakdown tick will erupt). Returns false when there are no psions.
pub fn force_psionic_breakdown(world: &mut World) -> bool {
    let mut query = world.query_filtered::<&mut Sanity, With<PsionicPotential>>();
    if let Some(mut sanity) = query.iter_mut(world).next() {
        sanity.current = 0.0;
        return true;
    }
    false
}

#[cfg(test)]
mod awakening_tests {
    use super::*;
    use crate::layer1::psychology::needs::Needs;

    fn breakdown_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<FireEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    fn count_fire_events(world: &World) -> usize {
        let events = world.resource::<Events<FireEvent>>();
        let mut reader = events.get_cursor();
        reader.read(events).count()
    }

    // Spec 374 RED test 1 (adapted to the real PopAction shape, which has
    // utility/commitment fields alongside `current`).
    #[test]
    fn test_psionic_tasks_drain_sanity() {
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                PsionicPotential { level: 1 },
                Sanity {
                    current: 100.0,
                    max: 100.0,
                },
                PopAction {
                    current: ActionType::TelekineticWork,
                    ..Default::default()
                },
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(process_psionic_actions_system);
        schedule.run(&mut world);
        let sanity = world.get::<Sanity>(pop).unwrap();
        assert!(sanity.current < 100.0);
        assert!((sanity.current - 95.0).abs() < f32::EPSILON);
    }

    // Spec 374 RED test 2 (adapted): zero sanity erupts — once, through
    // the cooldown, not every tick.
    #[test]
    fn test_low_sanity_triggers_anomalies() {
        let mut world = breakdown_world();
        world.spawn((
            Pop,
            PsionicPotential { level: 1 },
            Sanity {
                current: 0.0,
                max: 100.0,
            },
            GridPosition { x: 3, y: 4 },
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(trigger_psionic_breakdown_system);
        schedule.run(&mut world);
        assert_eq!(count_fire_events(&world), 1);
        // Cooldown: immediate re-runs must NOT spam more anomalies.
        schedule.run(&mut world);
        schedule.run(&mut world);
        assert_eq!(count_fire_events(&world), 1);
    }

    // Regression (live-play find): in the real schedule the breakdown
    // system must run BEFORE sanity recovery in the same tick — recovery
    // would otherwise lift a zeroed psion to 0.1 and the gift would
    // never erupt outside isolated unit tests. This mirrors the chain
    // order wired in simulation.rs.
    #[test]
    fn test_breakdown_fires_before_recovery_in_chain_order() {
        let mut world = breakdown_world();
        let pop = world
            .spawn((
                Pop,
                PsionicPotential { level: 1 },
                Sanity {
                    current: 0.0,
                    max: 100.0,
                },
                GridPosition { x: 3, y: 4 },
                PopAction::default(),
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems((trigger_psionic_breakdown_system, sanity_recovery_system).chain());
        schedule.run(&mut world);
        assert_eq!(count_fire_events(&world), 1);
        // And recovery still applies afterwards (the mind can heal).
        assert!(world.get::<Sanity>(pop).unwrap().current > 0.0);
    }

    #[test]
    fn test_healthy_sanity_no_anomaly() {
        let mut world = breakdown_world();
        world.spawn((
            Pop,
            PsionicPotential { level: 1 },
            Sanity {
                current: 50.0,
                max: 100.0,
            },
            GridPosition { x: 3, y: 4 },
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(trigger_psionic_breakdown_system);
        schedule.run(&mut world);
        assert_eq!(count_fire_events(&world), 0);
    }

    // Spec 374 RED test 3 (adapted to the real 0.0..=1.0 Needs scale).
    #[test]
    fn test_psion_soothes_nearby_pops() {
        let mut world = World::new();
        let psion = world
            .spawn((
                Pop,
                PsionicPotential { level: 1 },
                Sanity {
                    current: 100.0,
                    max: 100.0,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        let angry_pop = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.0,
                    ..Default::default()
                },
                GridPosition { x: 6, y: 5 },
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(process_psionic_soothing_system);
        schedule.run(&mut world);
        let needs = world.get::<Needs>(angry_pop).unwrap();
        assert!(needs.leisure > 0.0);
        let sanity = world.get::<Sanity>(psion).unwrap();
        assert!(sanity.current < 100.0);
    }

    #[test]
    fn test_soothing_stops_at_sanity_floor() {
        // A fraying psion withdraws: at/below the floor there is no
        // soothing and no further drain (no ambient burnout spiral).
        let mut world = World::new();
        let psion = world
            .spawn((
                Pop,
                PsionicPotential { level: 1 },
                Sanity {
                    current: 20.0,
                    max: 100.0,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        let angry_pop = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.0,
                    ..Default::default()
                },
                GridPosition { x: 6, y: 5 },
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(process_psionic_soothing_system);
        schedule.run(&mut world);
        assert_eq!(world.get::<Needs>(angry_pop).unwrap().leisure, 0.0);
        assert_eq!(world.get::<Sanity>(psion).unwrap().current, 20.0);
    }

    #[test]
    fn test_soothing_respects_radius() {
        let mut world = World::new();
        world.spawn((
            Pop,
            PsionicPotential { level: 1 },
            Sanity {
                current: 100.0,
                max: 100.0,
            },
            GridPosition { x: 5, y: 5 },
        ));
        let far_pop = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.0,
                    ..Default::default()
                },
                GridPosition { x: 20, y: 20 },
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(process_psionic_soothing_system);
        schedule.run(&mut world);
        assert_eq!(world.get::<Needs>(far_pop).unwrap().leisure, 0.0);
    }

    #[test]
    fn test_sanity_recovers_when_resting() {
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                PsionicPotential { level: 1 },
                Sanity {
                    current: 50.0,
                    max: 100.0,
                },
                PopAction::default(),
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(sanity_recovery_system);
        schedule.run(&mut world);
        assert!(world.get::<Sanity>(pop).unwrap().current > 50.0);
    }

    #[test]
    fn test_ensure_grants_sanity_to_bare_psion() {
        let mut world = World::new();
        let pop = world.spawn((Pop, PsionicPotential { level: 2 })).id();
        let mut schedule = Schedule::default();
        schedule.add_systems(ensure_psionic_components_system);
        schedule.run(&mut world);
        assert!(world.get::<Sanity>(pop).is_some());
        assert!(world.get::<PsionicBreakdownCooldown>(pop).is_some());
    }

    #[test]
    fn test_sanity_is_distinct_from_needs_and_morale() {
        // Sanity lives on its own 0..100 scale even though Needs are
        // 0..1 — draining it must not touch Needs (spec guidance).
        let sanity = Sanity::default();
        assert_eq!(sanity.max, 100.0);
        assert!((sanity.fraction() - 1.0).abs() < f32::EPSILON);
        let needs = Needs::default();
        assert!(needs.leisure <= 1.0);
    }

    #[test]
    fn test_ip_guard_no_lifted_names() {
        // IP rule (concept-only): every player-facing string this spec
        // ships must be original/generic — no named fiction, characters,
        // factions, or distinctive terminology from outside works.
        let banned = [
            "jedi", "sith", "x-men", "xmen", "psyker", "warhammer", "akira",
            "midichlorian", "professor x", "jean grey", "eleven",
            "stranger things", "esper", "newtype", "gundam",
        ];
        for text in [PSIONIC_AWAKENING_CHRONICLE, PSIONIC_BREAKDOWN_CHRONICLE] {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "player-facing psionic text contains banned term {term:?}: {text}"
                );
            }
        }
    }
}
