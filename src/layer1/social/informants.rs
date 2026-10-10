//! Spec 323: The Informant's Dilemma.
//!
//! The "Citizen Informant" edict ( [`crate::layer1::administration::edicts::Policy::CitizenInformant`] )
//! pays pops a small credit bounty for reporting dissent in their neighbors. Reports suppress
//! dissent and drop global [`Unrest`], but every report raises colony-wide [`Paranoia`].
//! Highly paranoid pops stop socializing and start levelling false treason accusations
//! (consumed by the justice system as [`Wanted`] markers) — perfect internal security at
//! the cost of all social cohesion.
//!
//! Dissent itself breeds from low morale (see [`dissent_breeds_system`]) so the edict has
//! something to act on in a live colony.

use crate::layer1::administration::edicts::{ColonyPolicies, Policy};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::Wallet;
use crate::layer1::law::justice::Wanted;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::social::morale::Morale;
use crate::layer1::unrest::{Unrest, UnrestModifier};
use bevy_ecs::prelude::*;

// --- Tuning ---------------------------------------------------------------

/// Dissent level above which a pop gets reported by informants.
pub const DISSENT_REPORT_THRESHOLD: f32 = 50.0;
/// Unrest reduction per informant report (Unrest runs 0.0–1.0).
pub const UNREST_DROP_PER_REPORT: f32 = 0.05;
/// Ticks an informant-report unrest modifier lasts.
pub const INFORMANT_UNREST_DURATION: u32 = 500;
/// Society-wide paranoia gained per report filed (the chilling effect).
pub const PARANOIA_PER_REPORT: f32 = 5.0;
/// Extra paranoia the reporting informant gains (snitches fear each other most).
pub const INFORMANT_PARANOIA_BONUS: f32 = 10.0;
/// Credit bounty paid to the informant per verified report.
pub const INFORMANT_CREDIT_REWARD: f32 = 25.0;
/// Paranoia above which a pop stops socializing entirely.
pub const PARANOIA_SOCIAL_CUTOFF: f32 = 80.0;
/// Paranoia above which a pop starts levelling false treason accusations.
pub const PARANOIA_ACCUSATION_CUTOFF: f32 = 90.0;
/// Social desire recovered per tick below the cutoff.
pub const SOCIAL_DESIRE_RECOVERY: f32 = 2.0;
/// Paranoia decayed per tick while the edict is inactive.
pub const PARANOIA_DECAY_PER_TICK: f32 = 0.05;
/// Chebyshev radius within which informants spot dissent.
pub const INFORMANT_SPOT_RADIUS: i32 = 8;
/// Chebyshev radius within which false accusations land (trust breaks locally).
pub const ACCUSATION_RADIUS: i32 = 8;
/// Ticks before a pop that made a false accusation may accuse again.
pub const ACCUSATION_COOLDOWN_TICKS: u32 = 500;
/// Wanted severity applied to a falsely accused pop.
pub const FALSE_ACCUSATION_SEVERITY: f32 = 0.6;
/// Wanted severity applied to a reported dissenter (they are "arrested").
pub const REPORTED_DISSENTER_SEVERITY: f32 = 0.8;
/// Morale below which dissent starts breeding.
pub const DISSENT_MORALE_THRESHOLD: f32 = 0.35;
/// Dissent gained per tick by a miserable pop.
pub const DISSENT_GROWTH_PER_TICK: f32 = 0.2;
/// Dissent shed per tick by a content pop.
pub const DISSENT_SHED_PER_TICK: f32 = 0.1;
/// Dissent level seeded on a pop that just turned miserable.
pub const DISSENT_SEED: f32 = 5.0;

// --- Components -------------------------------------------------------------

/// Hidden paranoia stat (0–100). Rises with every informant report filed
/// while the edict is active; decays slowly once it is repealed.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Paranoia {
    pub level: f32,
}

/// Dissident sentiment (0–100). Breeds from low morale; pops above
/// [`DISSENT_REPORT_THRESHOLD`] get reported while the edict is active.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Dissent {
    pub level: f32,
}

/// Willingness to mingle (0–100). Crushed to zero by high paranoia, which
/// makes the pop socially invisible (see `proximity_social_system`).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct SocialDesire {
    pub score: f32,
}

/// Rate-limiter on false accusations: ticks remaining before this pop may
/// accuse again.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct AccusationCooldown {
    pub ticks: u32,
}

// --- Events -----------------------------------------------------------------

/// A paranoid pop accused a neighbor of treason. The accusation may be
/// entirely false — the justice system does not check.
#[derive(Event, Debug, Clone, Copy)]
pub struct AccusationEvent {
    pub accuser: Entity,
    pub target: Entity,
}

/// An informant's report was verified: the reporter earned a bounty.
#[derive(Event, Debug, Clone, Copy)]
pub struct InformantReportEvent {
    pub reporter: Entity,
    pub target: Entity,
}

// --- Systems ----------------------------------------------------------------

/// Ensures every pop carries the informant-system components so the feature
/// works on pops spawned before (or without) them.
pub fn ensure_informant_components_system(
    mut commands: Commands,
    pops: Query<
        Entity,
        (
            With<Pop>,
            Without<Paranoia>,
        ),
    >,
) {
    for entity in pops.iter() {
        commands.entity(entity).insert((
            Paranoia::default(),
            Dissent::default(),
            SocialDesire { score: 100.0 },
        ));
    }
}

/// Dissent breeds where morale rots: miserable pops accumulate dissent,
/// content pops shed it. Gives the Citizen Informant edict live targets.
pub fn dissent_breeds_system(
    mut commands: Commands,
    mut pops: Query<(Entity, &Morale, Option<&mut Dissent>), With<Pop>>,
) {
    for (entity, morale, dissent_opt) in pops.iter_mut() {
        if morale.value < DISSENT_MORALE_THRESHOLD {
            match dissent_opt {
                Some(mut d) => {
                    d.level = (d.level + DISSENT_GROWTH_PER_TICK).min(100.0);
                }
                None => {
                    commands
                        .entity(entity)
                        .insert(Dissent { level: DISSENT_SEED });
                }
            }
        } else if let Some(mut d) = dissent_opt {
            d.level = (d.level - DISSENT_SHED_PER_TICK).max(0.0);
        }
    }
}

/// The heart of the dilemma. While the Citizen Informant edict is active,
/// pops with heavy dissent are reported by their nearest neighbor: the
/// dissenter is suppressed (dissent cleared, marked Wanted for arrest),
/// the informant gains paranoia plus a bounty event, global unrest drops,
/// and the whole colony gains paranoia — everyone is watching everyone.
pub fn process_informant_reports(
    mut commands: Commands,
    policies: Res<ColonyPolicies>,
    mut unrest: ResMut<Unrest>,
    mut report_events: EventWriter<InformantReportEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut pops: Query<(Entity, &GridPosition, &mut Paranoia, Option<&mut Dissent>), With<Pop>>,
) {
    if !policies.is_active(Policy::CitizenInformant) {
        return;
    }

    // Two-phase: collect first (borrowck), mutate second.
    let mut positions: Vec<(Entity, GridPosition)> = Vec::new();
    let mut dissenters: Vec<(Entity, GridPosition)> = Vec::new();
    for (entity, pos, _, dissent_opt) in pops.iter() {
        positions.push((entity, *pos));
        if dissent_opt.as_ref().map_or(false, |d| d.level > DISSENT_REPORT_THRESHOLD) {
            dissenters.push((entity, *pos));
        }
    }

    let mut reports = 0u32;
    for (dissenter, dpos) in &dissenters {
        // The nearest neighbor within spotting radius files the report.
        let mut best: Option<(Entity, i32)> = None;
        for (cand, cpos) in &positions {
            if cand == dissenter {
                continue;
            }
            let dx = dpos.x.abs_diff(cpos.x).min(i32::MAX as u32) as i32;
            let dy = dpos.y.abs_diff(cpos.y).min(i32::MAX as u32) as i32;
            let dist = dx.max(dy);
            if dist <= INFORMANT_SPOT_RADIUS && best.map_or(true, |(_, bd)| dist < bd) {
                best = Some((*cand, dist));
            }
        }
        let Some((reporter, _)) = best else {
            continue; // No snitch in earshot — the dissenter goes unreported.
        };

        // Suppress the dissenter: "arrested".
        if let Ok((_, _, _, dissent_opt)) = pops.get_mut(*dissenter) {
            if let Some(mut d) = dissent_opt {
                d.level = 0.0;
            }
        }
        commands
            .entity(*dissenter)
            .insert(Wanted { severity: REPORTED_DISSENTER_SEVERITY });

        // The informant's reward: paranoia (snitches fear each other most).
        if let Ok((_, _, mut paranoia, _)) = pops.get_mut(reporter) {
            paranoia.level = (paranoia.level + INFORMANT_PARANOIA_BONUS).min(100.0);
        }
        report_events.send(InformantReportEvent {
            reporter,
            target: *dissenter,
        });
        reports += 1;
    }

    if reports > 0 {
        unrest.modifiers.push(UnrestModifier {
            value: -(UNREST_DROP_PER_REPORT * reports as f32),
            duration: INFORMANT_UNREST_DURATION,
            label: "Informant reports".to_string(),
        });
        // The chilling effect: the whole colony learns to watch itself.
        for (_, _, mut paranoia, _) in pops.iter_mut() {
            paranoia.level =
                (paranoia.level + PARANOIA_PER_REPORT * reports as f32).min(100.0);
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "Informants denounced {reports} dissenter(s); the colony watches itself now."
            ),
            importance: EventImportance::Standard,
        });
    }
}

/// Pays the credit bounty for each verified informant report.
pub fn reward_informants(
    mut events: EventReader<InformantReportEvent>,
    mut wallets: Query<&mut Wallet>,
) {
    for ev in events.read() {
        if let Ok(mut wallet) = wallets.get_mut(ev.reporter) {
            wallet.credits += INFORMANT_CREDIT_REWARD;
        }
    }
}

/// Paranoia crushes the will to mingle; calm pops recover it.
pub fn evaluate_social_action(mut pops: Query<(&Paranoia, &mut SocialDesire)>) {
    for (paranoia, mut desire) in pops.iter_mut() {
        if paranoia.level > PARANOIA_SOCIAL_CUTOFF {
            desire.score = 0.0; // Too afraid to talk.
        } else {
            desire.score = (desire.score + SOCIAL_DESIRE_RECOVERY).min(100.0);
        }
    }
}

/// Highly paranoid pops accuse a nearby neighbor of treason — innocent or
/// not. Rate-limited by [`AccusationCooldown`] so a broken colony accuses in
/// waves rather than every tick.
pub fn generate_false_accusations(
    mut commands: Commands,
    mut events: EventWriter<AccusationEvent>,
    accusers: Query<(Entity, &GridPosition, &Paranoia), (With<Pop>, Without<AccusationCooldown>)>,
    targets: Query<(Entity, &GridPosition), With<Pop>>,
) {
    for (accuser, apos, paranoia) in accusers.iter() {
        if paranoia.level <= PARANOIA_ACCUSATION_CUTOFF {
            continue;
        }
        // Trust breaks locally and personally: the nearest neighbor.
        let mut best: Option<(Entity, i32)> = None;
        for (target, tpos) in targets.iter() {
            if target == accuser {
                continue;
            }
            let dx = apos.x.abs_diff(tpos.x).min(i32::MAX as u32) as i32;
            let dy = apos.y.abs_diff(tpos.y).min(i32::MAX as u32) as i32;
            let dist = dx.max(dy);
            if dist <= ACCUSATION_RADIUS && best.map_or(true, |(_, bd)| dist < bd) {
                best = Some((target, dist));
            }
        }
        if let Some((target, _)) = best {
            events.send(AccusationEvent { accuser, target });
            commands
                .entity(accuser)
                .insert(AccusationCooldown { ticks: ACCUSATION_COOLDOWN_TICKS });
        }
    }
}

/// Justice integration (spec §5): accusations are consumed as [`Wanted`]
/// markers whether or not they are true — wardens will arrest the accused.
/// This is the dilemma made mechanical: essential workers can be jailed on
/// a paranoid neighbor's word.
pub fn process_accusations_system(
    mut commands: Commands,
    mut events: EventReader<AccusationEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    targets: Query<(), With<Pop>>,
) {
    for ev in events.read() {
        if targets.get(ev.target).is_err() {
            continue;
        }
        commands.entity(ev.target).insert(Wanted {
            severity: FALSE_ACCUSATION_SEVERITY,
        });
        chronicle.send(AddChronicleEvent {
            text: "A citizen accused a neighbor of treason. The wardens are not asking questions."
                .to_string(),
            importance: EventImportance::Minor,
        });
    }
}

/// Ticks down accusation cooldowns.
pub fn tick_accusation_cooldowns(
    mut commands: Commands,
    mut pops: Query<(Entity, &mut AccusationCooldown)>,
) {
    for (entity, mut cd) in pops.iter_mut() {
        if cd.ticks > 0 {
            cd.ticks -= 1;
        }
        if cd.ticks == 0 {
            commands.entity(entity).remove::<AccusationCooldown>();
        }
    }
}

/// Paranoia decays once the edict is repealed — trust rebuilds slowly.
pub fn decay_paranoia_system(
    policies: Res<ColonyPolicies>,
    mut pops: Query<&mut Paranoia>,
) {
    if policies.is_active(Policy::CitizenInformant) {
        return;
    }
    for mut paranoia in pops.iter_mut() {
        paranoia.level = (paranoia.level - PARANOIA_DECAY_PER_TICK).max(0.0);
    }
}

// --- Headless helpers ---------------------------------------------------------

/// Average paranoia across all pops (for STATS).
#[must_use]
pub fn average_paranoia(world: &mut World) -> f32 {
    let mut total = 0.0;
    let mut count = 0u32;
    let mut query = world.query::<&Paranoia>();
    for paranoia in query.iter(world) {
        total += paranoia.level;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

/// `active` / `off` label for the Citizen Informant edict (for STATS).
#[must_use]
pub fn informant_edict_label(world: &World) -> &'static str {
    if world
        .get_resource::<ColonyPolicies>()
        .map_or(false, |p| p.is_active(Policy::CitizenInformant))
    {
        "active"
    } else {
        "off"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::economy::Wallet;
    use crate::layer1::law::justice::Wanted;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::unrest::Unrest;
    use bevy::app::App;
    use bevy::MinimalPlugins;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<ColonyPolicies>();
        app.init_resource::<Unrest>();
        app.add_event::<AccusationEvent>();
        app.add_event::<InformantReportEvent>();
        app.add_event::<AddChronicleEvent>();
        app
    }

    #[test]
    fn test_edict_activates_informant_system() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, process_informant_reports);

        let pop_a = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Paranoia { level: 0.0 },
                Dissent { level: 0.0 },
                Wallet { credits: 0.0 },
            ))
            .id();
        let pop_b = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 2, y: 0 },
                Paranoia { level: 0.0 },
                Dissent { level: 80.0 },
                Wallet { credits: 0.0 },
            ))
            .id();

        // Edict active (real ColonyPolicies integration, dep 054)
        app.world_mut()
            .resource_mut::<ColonyPolicies>()
            .toggle(Policy::CitizenInformant);

        // Act
        app.update();

        // Assert: dissent suppressed, unrest drops, informant gains paranoia
        let dissent_b = app.world().get::<Dissent>(pop_b).unwrap();
        assert!(
            dissent_b.level < DISSENT_REPORT_THRESHOLD,
            "Dissenter should be suppressed after an informant report"
        );
        let unrest = app.world().get_resource::<Unrest>().unwrap();
        assert!(
            unrest.modifiers.iter().any(|m| m.value < 0.0),
            "Global unrest should drop when dissent is reported"
        );
        let paranoia_a = app.world().get::<Paranoia>(pop_a).unwrap();
        assert!(
            paranoia_a.level > 0.0,
            "Informant should gain paranoia after reporting"
        );
    }

    #[test]
    fn test_edict_inactive_no_reports() {
        // Arrange: edict NOT enacted
        let mut app = test_app();
        app.add_systems(bevy::app::Update, process_informant_reports);

        let pop_b = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 2, y: 0 },
                Paranoia { level: 0.0 },
                Dissent { level: 80.0 },
                Wallet { credits: 0.0 },
            ))
            .id();
        app.world_mut().spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            Paranoia { level: 0.0 },
            Dissent { level: 0.0 },
            Wallet { credits: 0.0 },
        ));

        // Act
        app.update();

        // Assert: nothing happens without the edict
        let dissent_b = app.world().get::<Dissent>(pop_b).unwrap();
        assert!(
            (dissent_b.level - 80.0).abs() < f32::EPSILON,
            "Dissent must be untouched while the edict is inactive"
        );
        let unrest = app.world().get_resource::<Unrest>().unwrap();
        assert!(
            unrest.modifiers.is_empty(),
            "No unrest modifiers without the edict"
        );
    }

    #[test]
    fn test_high_paranoia_stops_socializing() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, evaluate_social_action);

        let paranoid_pop = app
            .world_mut()
            .spawn((Paranoia { level: 90.0 }, SocialDesire { score: 100.0 }))
            .id();
        let normal_pop = app
            .world_mut()
            .spawn((Paranoia { level: 10.0 }, SocialDesire { score: 100.0 }))
            .id();

        // Act
        app.update();

        // Assert
        let p_desire = app.world().get::<SocialDesire>(paranoid_pop).unwrap();
        let n_desire = app.world().get::<SocialDesire>(normal_pop).unwrap();
        assert!(
            p_desire.score < 10.0,
            "High paranoia should suppress social desire"
        );
        assert!(
            n_desire.score > 50.0,
            "Normal paranoia should not suppress social desire"
        );
    }

    #[test]
    fn test_high_paranoia_causes_false_accusations() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, generate_false_accusations);

        // Pop A is paranoid, Pop B is innocent (no Dissent)
        let _pop_a = app
            .world_mut()
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Paranoia { level: 95.0 }))
            .id();
        let pop_b = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 1, y: 0 },
                Paranoia { level: 0.0 },
                Dissent { level: 0.0 },
            ))
            .id();

        // Act
        app.update();

        // Assert
        let events = app.world().get_resource::<Events<AccusationEvent>>().unwrap();
        let mut cursor = events.get_cursor();
        let mut accusation_found = false;
        for ev in cursor.read(events) {
            if ev.target == pop_b {
                accusation_found = true;
            }
        }
        assert!(
            accusation_found,
            "Highly paranoid pops should generate false accusations against innocent targets"
        );
    }

    #[test]
    fn test_accusation_marks_target_wanted() {
        // Arrange: justice integration — accusations become Wanted markers even when false
        let mut app = test_app();
        app.add_systems(bevy::app::Update, process_accusations_system);

        let accuser = app.world_mut().spawn(Pop).id();
        let target = app.world_mut().spawn(Pop).id();
        app.world_mut()
            .resource_mut::<Events<AccusationEvent>>()
            .send(AccusationEvent { accuser, target });

        // Act
        app.update();

        // Assert
        let wanted = app.world().get::<Wanted>(target);
        assert!(
            wanted.is_some(),
            "False accusation must still mark the target Wanted (justice integration)"
        );
    }

    #[test]
    fn test_informant_gets_credit_reward() {
        // Arrange
        let mut app = test_app();
        app.add_systems(
            bevy::app::Update,
            (process_informant_reports, reward_informants).chain(),
        );

        let pop_a = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Paranoia { level: 0.0 },
                Dissent { level: 0.0 },
                Wallet { credits: 0.0 },
            ))
            .id();
        app.world_mut().spawn((
            Pop,
            GridPosition { x: 2, y: 0 },
            Paranoia { level: 0.0 },
            Dissent { level: 80.0 },
            Wallet { credits: 0.0 },
        ));
        app.world_mut()
            .resource_mut::<ColonyPolicies>()
            .toggle(Policy::CitizenInformant);

        // Act
        app.update();

        // Assert
        let wallet = app.world().get::<Wallet>(pop_a).unwrap();
        assert!(
            wallet.credits > 0.0,
            "Informant should receive a credit bounty for a verified report"
        );
    }

    #[test]
    fn test_paranoia_decays_when_edict_off() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, decay_paranoia_system);

        let pop = app.world_mut().spawn((Pop, Paranoia { level: 50.0 })).id();
        // Edict NOT active

        // Act
        app.update();

        // Assert
        let paranoia = app.world().get::<Paranoia>(pop).unwrap();
        assert!(
            paranoia.level < 50.0,
            "Paranoia should decay while the edict is inactive"
        );
    }

    #[test]
    fn test_dissent_breeds_from_low_morale() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, dissent_breeds_system);

        let miserable = app
            .world_mut()
            .spawn((Pop, Morale { value: 0.1, modifiers: vec![] }))
            .id();

        // Act: run enough ticks for dissent to seed and grow
        for _ in 0..10 {
            app.update();
        }

        // Assert
        let dissent = app.world().get::<Dissent>(miserable);
        assert!(
            dissent.map_or(false, |d| d.level > 0.0),
            "Miserable pops should accumulate dissent"
        );
    }

    #[test]
    fn test_full_dilemma_cascade_over_time() {
        // End-to-end: misery breeds dissent -> edict reports it -> unrest drops,
        // paranoia spreads -> social desire collapses.
        let mut app = test_app();
        app.add_systems(
            bevy::app::Update,
            (
                dissent_breeds_system,
                process_informant_reports,
                evaluate_social_action,
            )
                .chain(),
        );

        // A troubled colony: 8 miserable pops with dissent about to boil over.
        // Two report waves fit in 300 ticks (dissent regrows 0.2/tick after
        // each suppression), driving paranoia past the social-collapse cutoff.
        for i in 0..8 {
            app.world_mut().spawn((
                Pop,
                GridPosition { x: i, y: 0 },
                Paranoia { level: 0.0 },
                Dissent { level: 49.0 },
                SocialDesire { score: 100.0 },
                Wallet { credits: 0.0 },
                Morale { value: 0.1, modifiers: vec![] },
            ));
        }
        app.world_mut()
            .resource_mut::<ColonyPolicies>()
            .toggle(Policy::CitizenInformant);

        for _ in 0..300 {
            app.update();
        }

        // The cascade fired: unrest suppressed, paranoia up, socializing dead.
        let unrest = app.world().get_resource::<Unrest>().unwrap();
        assert!(
            unrest.modifiers.iter().any(|m| m.value < 0.0),
            "Sustained misery + edict should produce unrest-dropping reports"
        );
        let avg: f32 = {
            let mut q = app.world_mut().query::<&Paranoia>();
            let vs: Vec<f32> = q.iter(app.world()).map(|p| p.level).collect();
            vs.iter().sum::<f32>() / vs.len() as f32
        };
        assert!(avg > 0.0, "Paranoia should spread colony-wide, got {avg}");
        let mut q = app.world_mut().query::<&SocialDesire>();
        let desires: Vec<f32> = q.iter(app.world()).map(|d| d.score).collect();
        assert!(
            desires.iter().any(|s| *s < 10.0),
            "Paranoid pops should stop wanting to socialize"
        );
    }

    #[test]
    fn test_accusation_cooldown_rate_limits() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, generate_false_accusations);

        app.world_mut()
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Paranoia { level: 95.0 }));
        app.world_mut()
            .spawn((Pop, GridPosition { x: 1, y: 0 }, Paranoia { level: 0.0 }));

        // Act: two ticks — the second must not produce a second accusation
        app.update();
        app.update();

        // Assert
        let events = app.world().get_resource::<Events<AccusationEvent>>().unwrap();
        let mut cursor = events.get_cursor();
        let count = cursor.read(events).count();
        assert_eq!(
            count, 1,
            "Accusation cooldown must rate-limit paranoid pops to one accusation"
        );
    }
}
