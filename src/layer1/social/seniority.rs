//! Spec 333 — The Old Guard.
//!
//! The struggle between the hardened founders and the soft new arrivals.
//! Pops carry an [`Arrival`] tick (spec 078); this module layers a
//! three-tier [`Seniority`] on top of it:
//!
//! - [`SeniorityStatus::Founder`] — arrived inside the founding window
//!   (first 100 ticks: the original crew). Authority multiplier 1.5.
//! - [`SeniorityStatus::Veteran`] — arrived later but has at least two
//!   in-game years of tenure. Authority multiplier 1.2.
//! - [`SeniorityStatus::NewBlood`] — recent arrivals. Authority 1.0.
//!
//! NewBlood graduates to Veteran automatically (tenure is re-checked once
//! per in-game year, not every tick). Founders take decaying morale
//! friction when NewBlood outnumbers them — capped and temporary per the
//! spec's REFACTOR phase, via the real [`Morale`] mood-modifier path (the
//! same pattern as `generational_dissonance`). Authority is wired into
//! something real: founders and veterans are better mentors — their
//! authority multiplier scales the apprentice XP bonus in
//! `check_mentorship_system` (see [`mentor_multiplier_for`]).
//!
//! Note: the founder window here is deliberately narrower than spec 078's
//! `Generation::Founder` (arrival < 5 years). Seniority founders are the
//! *original crew*; a year-3 arrival is `Generation::Founder` but
//! `Seniority::Veteran`. The two splits coexist for different fantasies.

use crate::layer1::balance::TICKS_PER_YEAR;
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::old_guard::Arrival;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Arrival ticks before this count as Founders (the original crew).
/// Spec's `day < 10` mapped onto the real tick scale: the founding generation
/// is the setup batch plus the first moments of the sim. Later arrivals are
/// NewBlood until they earn Veteran tenure — so fresh immigrants always read
/// as new blood, which is the spec's core fantasy.
pub const FOUNDER_WINDOW_TICKS: u64 = 100;

/// Tenure in ticks after which NewBlood graduates to Veteran.
/// Spec's `current_day - day > 50` mapped onto the real tick scale.
pub const VETERAN_TENURE_TICKS: u64 = 2 * TICKS_PER_YEAR;

/// Graduation re-check cadence — once per in-game year ("seasonal"), not every tick.
pub const GRADUATION_CHECK_INTERVAL_TICKS: u64 = TICKS_PER_YEAR;

/// Demographic friction poll cadence.
pub const FRICTION_CHECK_INTERVAL_TICKS: u64 = 100;

/// How long a friction mood modifier lasts (capped/temporary per spec §5).
pub const FRICTION_MODIFIER_TICKS: u32 = 600;

/// Friction mood hit (same scale as generational_dissonance's -0.05).
pub const FRICTION_MODIFIER_VALUE: f32 = -0.05;

/// Mood label for a direct Founder × NewBlood encounter.
pub const FRICTION_LABEL: &str = "New Blood Friction";

/// Mood label for the demographic "outnumbered" state.
pub const OUTNUMBERED_LABEL: &str = "Outnumbered by New Blood";

/// One-time Major chronicle when NewBlood first outnumbers the founders.
pub const DEADLOCK_CHRONICLE_TEXT: &str =
    "The old guard is outnumbered. The founders mutter that the colony no longer feels like theirs.";

/// Seniority tier of a pop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Component)]
pub enum SeniorityStatus {
    Founder,
    Veteran,
    NewBlood,
}

/// Seniority component: tier plus the authority multiplier the spec requires.
#[derive(Debug, Clone, Copy, Component)]
pub struct Seniority {
    pub status: SeniorityStatus,
    pub authority_multiplier: f32,
}

/// Authority multiplier per tier (spec §4 GREEN mapping).
#[must_use]
pub const fn authority_for(status: SeniorityStatus) -> f32 {
    match status {
        SeniorityStatus::Founder => 1.5,
        SeniorityStatus::Veteran => 1.2,
        SeniorityStatus::NewBlood => 1.0,
    }
}

/// Pure tier evaluation from arrival tick and "now" tick. Unit-testable.
#[must_use]
pub const fn status_for_arrival(arrival_tick: u64, now_tick: u64) -> SeniorityStatus {
    if arrival_tick < FOUNDER_WINDOW_TICKS {
        SeniorityStatus::Founder
    } else if now_tick.saturating_sub(arrival_tick) >= VETERAN_TENURE_TICKS {
        SeniorityStatus::Veteran
    } else {
        SeniorityStatus::NewBlood
    }
}

/// Mentorship XP multiplier for an apprentice whose master has `master_seniority`.
/// Base teaching bonus is 1.5x; the master's authority scales it, so founders
/// (1.5) and veterans (1.2) pass on their craft better than new blood (1.0).
/// Pops without a Seniority component yet teach at the base rate.
#[must_use]
pub fn mentor_multiplier_for(master_seniority: Option<&Seniority>) -> f32 {
    const BASE_TEACHING_MULTIPLIER: f32 = 1.5;
    BASE_TEACHING_MULTIPLIER * master_seniority.map_or(1.0, |s| s.authority_multiplier)
}

/// Pair-interaction event: a founder and a new-blood pop had a rankling encounter.
/// (Spec's `SocialInteractionEvent`, adapted to the real codebase — there is no
/// global pair-interaction event, so this one is seniority-specific.)
#[derive(Event, Debug, Clone, Copy)]
pub struct SeniorityFrictionEvent {
    /// One side of the encounter.
    pub a: Entity,
    /// The other side of the encounter.
    pub b: Entity,
}

/// Demographic snapshot, refreshed by the friction system. Also feeds headless STATS.
#[derive(Resource, Default, Debug)]
pub struct SeniorityDemographics {
    pub founders: usize,
    pub veterans: usize,
    pub new_blood: usize,
    /// Whether the outnumbered chronicle has fired (one-time, like spec 078's Turning Point).
    pub deadlock_announced: bool,
}

/// Assigns [`Seniority`] to pops that lack it, and graduates NewBlood to
/// Veteran on a throttled (yearly) schedule. Spec §5: one system handling
/// both fresh assignment (`Without<Seniority>`) and mutable updates.
#[allow(clippy::type_complexity)]
pub fn assign_seniority_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    newcomers: Query<(Entity, &Arrival), (With<Pop>, Without<Seniority>)>,
    mut tenured: Query<(&Arrival, &mut Seniority), (With<Pop>, With<Seniority>)>,
) {
    let now = time.tick;
    for (entity, arrival) in &newcomers {
        let status = status_for_arrival(arrival.tick, now);
        commands.entity(entity).insert(Seniority {
            status,
            authority_multiplier: authority_for(status),
        });
    }

    // Graduation is throttled: re-check tenure once per in-game year, not every tick.
    if !now.is_multiple_of(GRADUATION_CHECK_INTERVAL_TICKS) {
        return;
    }
    for (arrival, mut seniority) in &mut tenured {
        if seniority.status == SeniorityStatus::NewBlood
            && now.saturating_sub(arrival.tick) >= VETERAN_TENURE_TICKS
        {
            seniority.status = SeniorityStatus::Veteran;
            seniority.authority_multiplier = authority_for(SeniorityStatus::Veteran);
        }
    }
}

fn friction_modifier(label: &str) -> MoodModifier {
    MoodModifier {
        label: label.to_string(),
        value: FRICTION_MODIFIER_VALUE,
        duration: FRICTION_MODIFIER_TICKS,
    }
}

/// Applies (or refreshes, never stacks) a decaying friction mood modifier on
/// `target` through the real [`Morale`] path. Spec §5 REFACTOR: capped and
/// temporary, not a permanent deduction.
fn apply_friction_mood(commands: &mut Commands, target: Entity, morale: Option<Mut<Morale>>, label: &str) {
    match morale {
        Some(mut m) => {
            if let Some(existing) = m.modifiers.iter_mut().find(|e| e.label == label) {
                existing.duration = FRICTION_MODIFIER_TICKS; // refresh, don't stack
            } else {
                m.add_modifier(friction_modifier(label));
            }
        }
        None => {
            let mut fresh = Morale::default();
            fresh.add_modifier(friction_modifier(label));
            commands.entity(target).insert(fresh);
        }
    }
}

/// Event-driven friction: exactly one side must be a Founder and the other
/// NewBlood (either order); the founder takes the morale hit.
pub fn seniority_friction_event_system(
    mut commands: Commands,
    mut events: EventReader<SeniorityFrictionEvent>,
    mut pops: Query<(&Seniority, Option<&mut Morale>), With<Pop>>,
) {
    for event in events.read() {
        let status_a = pops.get(event.a).ok().map(|(s, _)| s.status);
        let status_b = pops.get(event.b).ok().map(|(s, _)| s.status);
        let founder_entity = match (status_a, status_b) {
            (Some(SeniorityStatus::Founder), Some(SeniorityStatus::NewBlood)) => event.a,
            (Some(SeniorityStatus::NewBlood), Some(SeniorityStatus::Founder)) => event.b,
            _ => continue,
        };
        if let Ok((_, morale)) = pops.get_mut(founder_entity) {
            apply_friction_mood(&mut commands, founder_entity, morale, FRICTION_LABEL);
        }
    }
}

/// Demographic friction, polled every [`FRICTION_CHECK_INTERVAL_TICKS`] ticks:
/// when NewBlood outnumbers the founders, every founder gets the (capped,
/// decaying) "Outnumbered" mood modifier, and a one-time Major chronicle fires.
/// Also refreshes the [`SeniorityDemographics`] resource.
pub fn seniority_demographic_friction_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut demographics: ResMut<SeniorityDemographics>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut pops: Query<(Entity, &Seniority, Option<&mut Morale>), With<Pop>>,
) {
    if !time.tick.is_multiple_of(FRICTION_CHECK_INTERVAL_TICKS) {
        return;
    }

    let (mut founders, mut veterans, mut new_blood) = (0usize, 0usize, 0usize);
    for (_, seniority, _) in &pops {
        match seniority.status {
            SeniorityStatus::Founder => founders += 1,
            SeniorityStatus::Veteran => veterans += 1,
            SeniorityStatus::NewBlood => new_blood += 1,
        }
    }
    demographics.founders = founders;
    demographics.veterans = veterans;
    demographics.new_blood = new_blood;

    if !demographics.deadlock_announced && founders > 0 && new_blood > founders {
        demographics.deadlock_announced = true;
        chronicle.send(AddChronicleEvent {
            text: DEADLOCK_CHRONICLE_TEXT.to_string(),
            importance: EventImportance::Major,
        });
    }

    if founders == 0 || new_blood <= founders {
        return;
    }
    for (entity, seniority, morale) in &mut pops {
        if seniority.status == SeniorityStatus::Founder {
            apply_friction_mood(&mut commands, entity, morale, OUTNUMBERED_LABEL);
        }
    }
}

/// Counts pops per seniority tier. Headless STATS helper.
#[must_use]
pub fn seniority_counts(world: &mut World) -> (usize, usize, usize) {
    let mut query = world.query_filtered::<&Seniority, With<Pop>>();
    let (mut founders, mut veterans, mut new_blood) = (0usize, 0usize, 0usize);
    for seniority in query.iter(world) {
        match seniority.status {
            SeniorityStatus::Founder => founders += 1,
            SeniorityStatus::Veteran => veterans += 1,
            SeniorityStatus::NewBlood => new_blood += 1,
        }
    }
    (founders, veterans, new_blood)
}

/// Player-facing strings, for the IP-guard test.
#[must_use]
pub fn seniority_public_strings() -> Vec<&'static str> {
    vec![FRICTION_LABEL, OUTNUMBERED_LABEL, DEADLOCK_CHRONICLE_TEXT]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::time::SimSpeed;
    use bevy_ecs::system::RunSystemOnce;

    fn sim_time(tick: u64) -> SimulationTime {
        SimulationTime {
            tick,
            speed: SimSpeed::Normal,
        }
    }

    #[test]
    fn test_founder_gets_authority_bonus() {
        // Adapted RED test 1: spec's GlobalTime/ArrivalDate mapped onto the
        // real SimulationTime/Arrival types.
        let mut world = World::new();
        world.insert_resource(sim_time(100_000)); // ~year 100

        let founder = world.spawn((Pop, Arrival { tick: 5 })).id();

        let _ = world.run_system_once(assign_seniority_system);

        let seniority = world.get::<Seniority>(founder).unwrap();
        assert_eq!(seniority.status, SeniorityStatus::Founder);
        assert!(seniority.authority_multiplier > 1.0);
        assert_eq!(seniority.authority_multiplier, 1.5);
    }

    #[test]
    fn test_tier_assignment_and_graduation() {
        let mut world = World::new();
        world.insert_resource(sim_time(6_000)); // year 6

        // Arrived year 5, only 1 year of tenure -> NewBlood.
        let newcomer = world.spawn((Pop, Arrival { tick: 5_000 })).id();
        // Arrived year 3, 3 years of tenure -> Veteran immediately.
        let old_hand = world.spawn((Pop, Arrival { tick: 3_000 })).id();

        let _ = world.run_system_once(assign_seniority_system);

        assert_eq!(
            world.get::<Seniority>(newcomer).unwrap().status,
            SeniorityStatus::NewBlood
        );
        assert_eq!(
            world.get::<Seniority>(old_hand).unwrap().status,
            SeniorityStatus::Veteran
        );
        assert_eq!(
            world
                .get::<Seniority>(old_hand)
                .unwrap()
                .authority_multiplier,
            1.2
        );

        // Advance to year 8 (graduation cadence tick): the newcomer now has
        // 3 years of tenure and graduates to Veteran.
        world.resource_mut::<SimulationTime>().tick = 8_000;
        let _ = world.run_system_once(assign_seniority_system);
        let seniority = world.get::<Seniority>(newcomer).unwrap();
        assert_eq!(seniority.status, SeniorityStatus::Veteran);
        assert_eq!(seniority.authority_multiplier, 1.2);
    }

    #[test]
    fn test_graduation_throttled_between_years() {
        let mut world = World::new();
        // Year-8 tick that is NOT on the graduation cadence: no graduation.
        world.insert_resource(sim_time(8_500));
        let newcomer = world
            .spawn((
                Pop,
                Arrival { tick: 5_000 },
                Seniority {
                    status: SeniorityStatus::NewBlood,
                    authority_multiplier: 1.0,
                },
            ))
            .id();

        let _ = world.run_system_once(assign_seniority_system);
        assert_eq!(
            world.get::<Seniority>(newcomer).unwrap().status,
            SeniorityStatus::NewBlood
        );
    }

    #[test]
    fn test_founder_friction_with_new_blood() {
        // Adapted RED test 2: spec's direct Morale deduction becomes a
        // capped/decaying mood modifier per the REFACTOR phase.
        let mut world = World::new();
        world.init_resource::<Events<SeniorityFrictionEvent>>();

        let founder = world
            .spawn((
                Pop,
                Seniority {
                    status: SeniorityStatus::Founder,
                    authority_multiplier: 1.5,
                },
                Morale::default(),
            ))
            .id();
        let new_blood = world
            .spawn((
                Pop,
                Seniority {
                    status: SeniorityStatus::NewBlood,
                    authority_multiplier: 1.0,
                },
                Morale::default(),
            ))
            .id();

        world.send_event(SeniorityFrictionEvent {
            a: founder,
            b: new_blood,
        });
        let _ = world.run_system_once(seniority_friction_event_system);

        let morale = world.get::<Morale>(founder).unwrap();
        let friction = morale
            .modifiers
            .iter()
            .find(|m| m.label == FRICTION_LABEL)
            .expect("founder should carry the friction modifier");
        assert!(friction.value < 0.0, "founder morale should drop");
        assert_eq!(friction.duration, FRICTION_MODIFIER_TICKS);

        // The new blood is unbothered.
        let nb_morale = world.get::<Morale>(new_blood).unwrap();
        assert!(nb_morale
            .modifiers
            .iter()
            .all(|m| m.label != FRICTION_LABEL));

        // Swapped argument order still lands on the founder.
        world.send_event(SeniorityFrictionEvent {
            a: new_blood,
            b: founder,
        });
        let _ = world.run_system_once(seniority_friction_event_system);
        let morale = world.get::<Morale>(founder).unwrap();
        assert_eq!(
            morale.modifiers.iter().filter(|m| m.label == FRICTION_LABEL).count(),
            1,
            "friction must refresh, not stack"
        );
    }

    #[test]
    fn test_friction_ignores_non_pairs() {
        let mut world = World::new();
        world.init_resource::<Events<SeniorityFrictionEvent>>();

        let vet_a = world
            .spawn((
                Pop,
                Seniority {
                    status: SeniorityStatus::Veteran,
                    authority_multiplier: 1.2,
                },
                Morale::default(),
            ))
            .id();
        let vet_b = world
            .spawn((
                Pop,
                Seniority {
                    status: SeniorityStatus::Veteran,
                    authority_multiplier: 1.2,
                },
                Morale::default(),
            ))
            .id();

        world.send_event(SeniorityFrictionEvent { a: vet_a, b: vet_b });
        let _ = world.run_system_once(seniority_friction_event_system);

        for e in [vet_a, vet_b] {
            assert!(world
                .get::<Morale>(e)
                .unwrap()
                .modifiers
                .is_empty());
        }
    }

    #[test]
    fn test_demographic_friction_when_outnumbered() {
        let mut world = World::new();
        world.insert_resource(sim_time(100)); // on the 100-tick poll cadence
        world.insert_resource(SeniorityDemographics::default());
        world.init_resource::<Events<AddChronicleEvent>>();

        let founder = world
            .spawn((
                Pop,
                Seniority {
                    status: SeniorityStatus::Founder,
                    authority_multiplier: 1.5,
                },
                Morale::default(),
            ))
            .id();
        for _ in 0..3 {
            world.spawn((
                Pop,
                Seniority {
                    status: SeniorityStatus::NewBlood,
                    authority_multiplier: 1.0,
                },
                Morale::default(),
            ));
        }

        let _ = world.run_system_once(seniority_demographic_friction_system);

        let morale = world.get::<Morale>(founder).unwrap();
        assert!(
            morale.modifiers.iter().any(|m| m.label == OUTNUMBERED_LABEL),
            "outnumbered founder should carry the modifier"
        );

        let demo = world.resource::<SeniorityDemographics>();
        assert_eq!((demo.founders, demo.veterans, demo.new_blood), (1, 0, 3));
        assert!(demo.deadlock_announced);

        let chronicles = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicles.get_cursor();
        let texts: Vec<String> = reader
            .read(chronicles)
            .map(|e| e.text.clone())
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("old guard")),
            "deadlock chronicle should fire, got {texts:?}"
        );
    }

    #[test]
    fn test_no_friction_when_founders_hold_majority() {
        let mut world = World::new();
        world.insert_resource(sim_time(200));
        world.insert_resource(SeniorityDemographics::default());
        world.init_resource::<Events<AddChronicleEvent>>();

        for _ in 0..2 {
            world.spawn((
                Pop,
                Seniority {
                    status: SeniorityStatus::Founder,
                    authority_multiplier: 1.5,
                },
                Morale::default(),
            ));
        }
        world.spawn((
            Pop,
            Seniority {
                status: SeniorityStatus::NewBlood,
                authority_multiplier: 1.0,
            },
            Morale::default(),
        ));

        let _ = world.run_system_once(seniority_demographic_friction_system);

        let mut query = world.query::<&Morale>();
        assert!(query
            .iter(&world)
            .flat_map(|m| &m.modifiers)
            .all(|m| m.label != OUTNUMBERED_LABEL));
        assert!(!world.resource::<SeniorityDemographics>().deadlock_announced);
    }

    #[test]
    fn test_mentor_multiplier_scales_with_authority() {
        let founder = Seniority {
            status: SeniorityStatus::Founder,
            authority_multiplier: 1.5,
        };
        let veteran = Seniority {
            status: SeniorityStatus::Veteran,
            authority_multiplier: 1.2,
        };
        let new_blood = Seniority {
            status: SeniorityStatus::NewBlood,
            authority_multiplier: 1.0,
        };
        assert!((mentor_multiplier_for(Some(&founder)) - 2.25).abs() < 1e-6);
        assert!((mentor_multiplier_for(Some(&veteran)) - 1.8).abs() < 1e-6);
        assert!((mentor_multiplier_for(Some(&new_blood)) - 1.5).abs() < 1e-6);
        assert!((mentor_multiplier_for(None) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn test_status_for_arrival_pure() {
        // Founder window: first in-game year.
        assert_eq!(
            status_for_arrival(0, 50_000),
            SeniorityStatus::Founder
        );
        assert_eq!(
            status_for_arrival(FOUNDER_WINDOW_TICKS - 1, 50_000),
            SeniorityStatus::Founder
        );
        // Boundary: exactly at the window edge is no longer a founder.
        assert_eq!(
            status_for_arrival(FOUNDER_WINDOW_TICKS, FOUNDER_WINDOW_TICKS + 100),
            SeniorityStatus::NewBlood
        );
        // Tenure past the veteran threshold graduates at assignment time.
        assert_eq!(
            status_for_arrival(5_000, 5_000 + VETERAN_TENURE_TICKS),
            SeniorityStatus::Veteran
        );
        // One tick short stays NewBlood.
        assert_eq!(
            status_for_arrival(5_000, 5_000 + VETERAN_TENURE_TICKS - 1),
            SeniorityStatus::NewBlood
        );
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Concept-only inspirations: no named characters, places, or
        // distinctive entities from outside fiction may appear in
        // player-facing strings.
        const BANNED: &[&str] = &[
            "arrakis", "dune", "atreides", "harkonnen", "fremen", "sandworm",
            "tatooine", "coruscant", "jedi", "sith", "wookiee", "mordor",
            "gondor", "hobbit", "shire", "sauron", "gandalf", "rivendell",
            "westeros", "stark", "lannister", "targaryen", "winterfell",
            "klingon", "vulcan", "enterprise", "borg", "romulan",
            "xenomorph", "ripley", "weyland", "nostromo",
            "megatron", "optimus", "cybertron",
        ];
        for s in seniority_public_strings() {
            let lower = s.to_lowercase();
            for banned in BANNED {
                assert!(
                    !lower.contains(banned),
                    "public string {s:?} contains banned term {banned:?}"
                );
            }
        }
    }
}
