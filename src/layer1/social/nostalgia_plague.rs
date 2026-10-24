//! Spec 362: The Nostalgia Plague.
//!
//! A civilization-wide crisis of collective melancholia: pops contract
//! [`NostalgiaInfection`] from viewing ancient relics and ruins, from
//! old chronologies embodied in historical artifacts and ancient
//! structures, and from social contact with infected pops, carriers of
//! the older [`Nostalgia`](crate::layer1::culture::nostalgia::Nostalgia)
//! affliction, and elder core-world pops.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **Exposure, not instant infection (spec §7).** Contact feeds a
//!   [`NostalgiaExposure`] accumulator; only crossing
//!   [`EXPOSURE_THRESHOLD`] inserts the infection. A single glimpse of a
//!   ruin does nothing.
//! * **One efficiency pass (spec §5 REFACTOR).**
//!   [`nostalgia_efficiency_multiplier`] is the single place both the
//!   high-tech penalty (0.1×) and the primitive bonus (2.0×) live. It is
//!   applied in exactly one production seam — the per-pop speed
//!   modifier assembled in `execution::general_work`'s worker
//!   collection, alongside the trait/job/buff/fog modifiers — keyed by
//!   the pop's [`TechTier`] classification
//!   ([`tech_tier_for_assignment`] / [`tech_tier_for_building`]).
//!   `BuildingType::tier_info` (the pre-existing Tech Envy tiers) is
//!   reused rather than duplicated.
//! * **Spread rides existing paths (spec §5).** Social spread uses the
//!   spatial-hash-by-tile neighbour pattern from Spec 346's emotional
//!   contagion (bucket sources by tile, probe only nearby cells — no
//!   O(N²) all-pairs scan, no duplicated distance system).
//! * **Sabotage plugs into the existing evaluator.** Infected pops do
//!   not get a new AI: `PopEvaluationQuery` now reports
//!   `is_nostalgic` for plague-infected pops too, so the pre-existing
//!   `actions::sabotage::evaluate_sabotage` (utility 100, nearest
//!   advanced-tech structure) and the real `sabotage_action_system`
//!   damage pipeline handle dismantling exactly as they already did
//!   for the Nostalgia affliction.
//! * **Cure (spec §5).** No modern-treatment system existed to hook, so
//!   the cure is the spec's own "reality anchor" broadcast kept to a
//!   minimal seam: a toggleable [`NostalgiaPlagueState::anchor_active`]
//!   flag (headless `nostalgia anchor on|off`). While the broadcast
//!   runs, infected pops accrue cure progress and shed the infection
//!   at [`CURE_THRESHOLD`]; while it runs, exposure cannot build at
//!   all and exposed pops shed exposure five times faster (without
//!   that, staggered cures ping-ponged reinfection in dense colonies). A building-gated anchor (e.g. requiring a powered media
//!   station) is a documented follow-on, not built here.

use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::layer1::building::{BuildingType, Tier};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::culture::nostalgia::Nostalgia;
use crate::layer1::economy::artifact_market::HistoricalArtifact;
use crate::layer1::entities::pop::Pop;
use crate::layer1::health::Dead;
use crate::layer1::heirloom::AncientStructure;
use crate::layer1::lifecycle::{Age, LifeStage};
use crate::layer1::architecture::ruins::Ruin;
use crate::layer1::utility_types::AssignmentType;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Exposure points a pop must accumulate before the plague takes hold.
pub const EXPOSURE_THRESHOLD: f32 = 30.0;
/// Exposure gained per tick near a relic source (ruin / artifact /
/// ancient structure).
pub const RELIC_EXPOSURE_PER_TICK: f32 = 1.0;
/// Exposure gained per tick in social contact with a carrier source
/// (infected pop, Nostalgia-afflicted pop, or elder pop).
pub const SOCIAL_EXPOSURE_PER_TICK: f32 = 1.0;
/// Chebyshev radius (tiles) for relic viewing.
pub const RELIC_EXPOSURE_RADIUS: i32 = 3;
/// Chebyshev radius (tiles) for social contact.
pub const SOCIAL_EXPOSURE_RADIUS: i32 = 2;
/// Exposure shed per tick with no source in range.
pub const EXPOSURE_DECAY_PER_TICK: f32 = 1.0;
/// Extra exposure shed per tick while the reality anchor broadcasts.
pub const ANCHOR_EXPOSURE_DECAY: f32 = 5.0;
/// Cure progress gained per tick under the reality anchor.
pub const CURE_PER_TICK: f32 = 2.0;
/// Cure progress at which the infection is shed.
pub const CURE_THRESHOLD: f32 = 100.0;
/// Efficiency multiplier for infected pops on high-tech work.
pub const HIGHTECH_EFFICIENCY: f32 = 0.1;
/// Efficiency multiplier for infected pops on modern work.
pub const MODERN_EFFICIENCY: f32 = 0.5;
/// Efficiency multiplier for infected pops on primitive work.
pub const PRIMITIVE_EFFICIENCY: f32 = 2.0;

// ---------------------------------------------------------------------------
// Tech tier classification (spec §7: queryable TechTier)
// ---------------------------------------------------------------------------

/// Queryable technology tier of a job or building (Spec 362).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TechTier {
    /// Manual, low-tech work: farming by hand, burial, hauling-grade labour.
    Primitive,
    /// Ordinary colony industry and administration.
    Modern,
    /// Advanced systems: research, medicine, machine intelligence.
    HighTech,
}

impl TechTier {
    /// Short label for panels and logs.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Primitive => "primitive",
            Self::Modern => "modern",
            Self::HighTech => "high-tech",
        }
    }
}

/// Classifies a pop assignment into a [`TechTier`].
#[must_use]
pub const fn tech_tier_for_assignment(job: AssignmentType) -> TechTier {
    match job {
        AssignmentType::FarmWorker | AssignmentType::Funeral | AssignmentType::DeepMining => {
            TechTier::Primitive
        }
        AssignmentType::LibraryWorker
        | AssignmentType::ObservatoryWorker
        | AssignmentType::Doctor
        | AssignmentType::Surgery => TechTier::HighTech,
        _ => TechTier::Modern,
    }
}

/// Classifies a building into a [`TechTier`], reusing the pre-existing
/// `BuildingType::tier_info` (Tech Envy tiers) where it speaks, with a
/// conservative fallback for unclassified types.
#[must_use]
pub const fn tech_tier_for_building(kind: BuildingType) -> TechTier {
    if let Some((_, tier)) = kind.tier_info() {
        return match tier {
            Tier::Basic => TechTier::Primitive,
            Tier::Advanced => TechTier::Modern,
            Tier::HighTech => TechTier::HighTech,
        };
    }
    match kind {
        BuildingType::Housing
        | BuildingType::Well
        | BuildingType::Stockpile
        | BuildingType::Grave
        | BuildingType::Wall
        | BuildingType::FlowerBed
        | BuildingType::PersonalGarden => TechTier::Primitive,
        BuildingType::AICore
        | BuildingType::CloneVat
        | BuildingType::GeneBank
        | BuildingType::AtmosphericProcessor
        | BuildingType::Nanoforge
        | BuildingType::ServerBank
        | BuildingType::Mainframe
        | BuildingType::LifeSupport => TechTier::HighTech,
        _ => TechTier::Modern,
    }
}

// ---------------------------------------------------------------------------
// The one efficiency pass (spec §5 REFACTOR)
// ---------------------------------------------------------------------------

/// THE centralized nostalgia efficiency modifier (Spec 362 §5).
///
/// Uninfected pops are always at 1.0×. Infected pops work high-tech
/// jobs at [`HIGHTECH_EFFICIENCY`] (10%), modern jobs at
/// [`MODERN_EFFICIENCY`], and primitive jobs at
/// [`PRIMITIVE_EFFICIENCY`] (200%) — they long for the old ways and
/// are suddenly very good at them.
#[must_use]
pub const fn nostalgia_efficiency_multiplier(infected: bool, tier: TechTier) -> f32 {
    if !infected {
        return 1.0;
    }
    match tier {
        TechTier::Primitive => PRIMITIVE_EFFICIENCY,
        TechTier::Modern => MODERN_EFFICIENCY,
        TechTier::HighTech => HIGHTECH_EFFICIENCY,
    }
}

/// Convenience: efficiency for an assignment.
#[must_use]
pub const fn nostalgia_efficiency_for_assignment(
    infected: bool,
    job: AssignmentType,
) -> f32 {
    nostalgia_efficiency_multiplier(infected, tech_tier_for_assignment(job))
}

/// Convenience: efficiency for work at a building.
#[must_use]
pub const fn nostalgia_efficiency_for_building(infected: bool, kind: BuildingType) -> f32 {
    nostalgia_efficiency_multiplier(infected, tech_tier_for_building(kind))
}

// ---------------------------------------------------------------------------
// Components, state, events
// ---------------------------------------------------------------------------

/// The plague itself: a pop lost to collective melancholia.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct NostalgiaInfection {
    /// Cure progress accrued under the reality anchor (0..[`CURE_THRESHOLD`]).
    pub cure_progress: f32,
}

/// Pre-infection exposure accumulator (spec §7: no instant infection).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct NostalgiaExposure {
    /// Accumulated exposure points; infection at [`EXPOSURE_THRESHOLD`].
    pub amount: f32,
}

/// Colony-level plague ledger + reality-anchor switch.
#[derive(Resource, Debug, Default)]
pub struct NostalgiaPlagueState {
    /// True once the first infection has been chronicled.
    pub outbreak_announced: bool,
    /// Total infections over the run (including cured pops).
    pub total_infections: u64,
    /// Reality-anchor broadcast currently running (the cure seam).
    pub anchor_active: bool,
}

/// Fired when a pop crosses the exposure threshold into infection.
#[derive(Event, Debug, Clone, Copy)]
pub struct NostalgiaInfectedEvent {
    /// The newly infected pop.
    pub pop: Entity,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

struct ExposureSource {
    x: i32,
    y: i32,
    radius: i32,
    rate: f32,
}

/// Exposure: pops near relics (ruins, historical artifacts, ancient
/// structures) or carrier pops (infected, Nostalgia-afflicted, elders)
/// accumulate [`NostalgiaExposure`]; crossing the threshold infects.
///
/// Sources are bucketed by tile into a spatial hash (Spec 346 pattern);
/// each pop probes only the cells inside the widest source radius. A
/// pop gains at the strongest single source's rate — sources do not
/// stack — and sheds exposure when no source is in range.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn nostalgia_exposure_system(
    mut commands: Commands,
    ruins: Query<&GridPosition, With<Ruin>>,
    artifacts: Query<&GridPosition, With<HistoricalArtifact>>,
    ancient: Query<&GridPosition, With<AncientStructure>>,
    infected: Query<&GridPosition, With<NostalgiaInfection>>,
    afflicted: Query<&GridPosition, With<Nostalgia>>,
    elders: Query<(&GridPosition, &Age)>,
    mut pops: Query<
        (Entity, &GridPosition, Option<&mut NostalgiaExposure>),
        (With<Pop>, Without<NostalgiaInfection>, Without<Dead>),
    >,
    mut state: Option<ResMut<NostalgiaPlagueState>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut infected_events: EventWriter<NostalgiaInfectedEvent>,
) {
    // Snapshot + bucket sources by tile.
    let mut buckets: HashMap<(i32, i32), Vec<ExposureSource>> = HashMap::new();
    let mut max_radius: i32 = 0;
    let push = |buckets: &mut HashMap<(i32, i32), Vec<ExposureSource>>,
                    max_radius: &mut i32,
                    pos: &GridPosition,
                    radius: i32,
                    rate: f32| {
        *max_radius = (*max_radius).max(radius);
        buckets
            .entry((pos.x, pos.y))
            .or_default()
            .push(ExposureSource {
                x: pos.x,
                y: pos.y,
                radius,
                rate,
            });
    };
    for pos in &ruins {
        push(&mut buckets, &mut max_radius, pos, RELIC_EXPOSURE_RADIUS, RELIC_EXPOSURE_PER_TICK);
    }
    for pos in &artifacts {
        push(&mut buckets, &mut max_radius, pos, RELIC_EXPOSURE_RADIUS, RELIC_EXPOSURE_PER_TICK);
    }
    for pos in &ancient {
        push(&mut buckets, &mut max_radius, pos, RELIC_EXPOSURE_RADIUS, RELIC_EXPOSURE_PER_TICK);
    }
    for pos in &infected {
        push(&mut buckets, &mut max_radius, pos, SOCIAL_EXPOSURE_RADIUS, SOCIAL_EXPOSURE_PER_TICK);
    }
    for pos in &afflicted {
        push(&mut buckets, &mut max_radius, pos, SOCIAL_EXPOSURE_RADIUS, SOCIAL_EXPOSURE_PER_TICK);
    }
    for (pos, age) in &elders {
        if age.stage == LifeStage::Elder {
            push(&mut buckets, &mut max_radius, pos, SOCIAL_EXPOSURE_RADIUS, SOCIAL_EXPOSURE_PER_TICK);
        }
    }

    let anchor_active = state.as_ref().is_some_and(|s| s.anchor_active);
    let mut newly_infected: Vec<Entity> = Vec::new();
    let mut inserts: Vec<(Entity, f32)> = Vec::new();

    for (entity, pos, exposure) in &mut pops {
        // Strongest source in range (Chebyshev), via bucket probe.
        let mut best_rate: f32 = 0.0;
        if !buckets.is_empty() {
            for cy in (pos.y - max_radius)..=(pos.y + max_radius) {
                for cx in (pos.x - max_radius)..=(pos.x + max_radius) {
                    if let Some(cell) = buckets.get(&(cx, cy)) {
                        for src in cell {
                            let dist = (pos.x - src.x).abs().max((pos.y - src.y).abs());
                            if dist <= src.radius && src.rate > best_rate {
                                best_rate = src.rate;
                            }
                        }
                    }
                }
            }
        }

        let current = exposure.as_ref().map_or(0.0, |e| e.amount);
        // While the reality anchor broadcasts, exposure cannot build:
        // the anchor is a counter-signal, so a cured pop standing next
        // to a still-infected neighbour is not re-infected mid-cure
        // (playtest: without this the anchor could never win in a
        // dense colony — cures staggered, exposure ping-ponged).
        let next = if anchor_active {
            (current - ANCHOR_EXPOSURE_DECAY).max(0.0)
        } else if best_rate > 0.0 {
            current + best_rate
        } else {
            (current - EXPOSURE_DECAY_PER_TICK).max(0.0)
        };

        if next >= EXPOSURE_THRESHOLD {
            newly_infected.push(entity);
        } else if exposure.is_some() {
            if let Some(mut e) = exposure {
                e.amount = next;
            }
        } else if next > 0.0 {
            inserts.push((entity, next));
        }
    }

    for (entity, amount) in inserts {
        commands.entity(entity).insert(NostalgiaExposure { amount });
    }

    if !newly_infected.is_empty() {
        for &entity in &newly_infected {
            commands.entity(entity).insert(NostalgiaInfection::default());
            commands.entity(entity).remove::<NostalgiaExposure>();
            infected_events.send(NostalgiaInfectedEvent { pop: entity });
        }
        if let Some(state) = state.as_mut() {
            state.total_infections += newly_infected.len() as u64;
            if !state.outbreak_announced {
                state.outbreak_announced = true;
                chronicle.send(AddChronicleEvent {
                    text: "The Nostalgia Plague takes hold: a colonist has "
                        .to_string()
                        + "succumbed to an overwhelming longing for the old days.",
                    importance: EventImportance::Major,
                });
            }
        }
    }
}

/// Reality-anchor cure: while the broadcast runs, infected pops accrue
/// cure progress and shed the infection at [`CURE_THRESHOLD`].
/// (Exposure shedding under the anchor lives in the exposure system.)
pub fn reality_anchor_system(
    mut commands: Commands,
    state: Option<Res<NostalgiaPlagueState>>,
    mut infected: Query<(Entity, &mut NostalgiaInfection)>,
) {
    if !state.is_some_and(|s| s.anchor_active) {
        return;
    }
    for (entity, mut infection) in &mut infected {
        infection.cure_progress += CURE_PER_TICK;
        if infection.cure_progress >= CURE_THRESHOLD {
            commands.entity(entity).remove::<NostalgiaInfection>();
        }
    }
}

// ---------------------------------------------------------------------------
// Headless helpers
// ---------------------------------------------------------------------------

/// Number of pops currently infected (STATS `nostalgia=`).
pub fn nostalgia_infected_count(world: &mut World) -> usize {
    world
        .query_filtered::<Entity, With<NostalgiaInfection>>()
        .iter(world)
        .count()
}

/// Number of pops carrying exposure but not yet infected.
pub fn nostalgia_exposed_count(world: &mut World) -> usize {
    world
        .query_filtered::<Entity, (With<NostalgiaExposure>, Without<NostalgiaInfection>)>()
        .iter(world)
        .count()
}

/// One-line plague summary for the headless `nostalgia` panel.
pub fn nostalgia_status_label(world: &mut World) -> String {
    let infected = nostalgia_infected_count(world);
    let exposed = nostalgia_exposed_count(world);
    let (total, anchor) = world
        .get_resource::<NostalgiaPlagueState>()
        .map_or((0, false), |s| (s.total_infections, s.anchor_active));
    format!("infected={infected} exposed={exposed} total={total} anchor={anchor}")
}

// ---------------------------------------------------------------------------
// Tests (spec §3 RED tests adapted to real types, plus cure + IP guard)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::actions::sabotage::evaluate_sabotage;
    use crate::layer1::map::GridPosition;
    use crate::layer1::mind::utility_eval_types::{
        PopEvalData, ScorableCandidate, UtilityAIBuffer,
    };
    use crate::layer1::utility_types::ActionType;
    use bevy::prelude::*;

    fn plague_app() -> App {
        let mut app = App::new();
        app.init_resource::<NostalgiaPlagueState>();
        app.add_event::<AddChronicleEvent>();
        app.add_event::<NostalgiaInfectedEvent>();
        app.add_systems(
            Update,
            (nostalgia_exposure_system, reality_anchor_system),
        );
        app
    }

    /// Spec test 1: viewing an ancient relic for X ticks infects — but
    /// only after the exposure accumulator crosses the threshold.
    #[test]
    fn test_exposure_to_ancient_relic_triggers_nostalgia_infection() {
        let mut app = plague_app();
        let pop = app
            .world_mut()
            .spawn((Pop, GridPosition { x: 5, y: 5 }))
            .id();
        // A ruin is the colony's standing "ancient relic".
        app.world_mut()
            .spawn((Ruin { original_type: BuildingType::Housing, material: crate::layer1::building::MaterialType::Wood }, GridPosition { x: 6, y: 5 }));

        // A brief glimpse must NOT infect (accumulator, spec §7).
        for _ in 0..5 {
            app.update();
        }
        assert!(
            app.world().get::<NostalgiaInfection>(pop).is_none(),
            "brief relic exposure must not instantly infect"
        );
        assert!(
            app.world().get::<NostalgiaExposure>(pop).is_some_and(|e| e.amount > 0.0),
            "exposure should be accumulating"
        );

        // Sustained viewing crosses the threshold.
        for _ in 0..40 {
            app.update();
        }
        assert!(
            app.world().get::<NostalgiaInfection>(pop).is_some(),
            "sustained relic viewing should infect"
        );
        let state = app.world().resource::<NostalgiaPlagueState>();
        assert!(state.outbreak_announced);
        assert_eq!(state.total_infections, 1);
    }

    /// Spec test 2: infected pops are crippled on high-tech jobs (10%).
    #[test]
    fn test_nostalgia_infected_pop_suffers_penalty_on_hightech_jobs() {
        assert!((nostalgia_efficiency_for_assignment(true, AssignmentType::LibraryWorker) - 0.1).abs() < f32::EPSILON);
        assert!((nostalgia_efficiency_for_building(true, BuildingType::AICore) - 0.1).abs() < f32::EPSILON);
        assert!((nostalgia_efficiency_multiplier(true, TechTier::HighTech) - 0.1).abs() < f32::EPSILON);
        // Uninfected pops are untouched.
        assert!((nostalgia_efficiency_for_assignment(false, AssignmentType::LibraryWorker) - 1.0).abs() < f32::EPSILON);
    }

    /// Spec test 3: infected pops excel at primitive jobs (200%).
    #[test]
    fn test_nostalgia_infected_pop_gains_bonus_on_primitive_jobs() {
        assert!((nostalgia_efficiency_for_assignment(true, AssignmentType::FarmWorker) - 2.0).abs() < f32::EPSILON);
        assert!((nostalgia_efficiency_for_building(true, BuildingType::Farm) - 2.0).abs() < f32::EPSILON);
        assert!((nostalgia_efficiency_multiplier(true, TechTier::Primitive) - 2.0).abs() < f32::EPSILON);
        // Modern work sits between the two poles.
        assert!((nostalgia_efficiency_multiplier(true, TechTier::Modern) - 0.5).abs() < f32::EPSILON);
    }

    /// Spec test 4: the plague spreads via social proximity.
    #[test]
    fn test_nostalgia_infection_spreads_via_social_interaction() {
        let mut app = plague_app();
        let _carrier = app
            .world_mut()
            .spawn((Pop, GridPosition { x: 5, y: 5 }, NostalgiaInfection::default()))
            .id();
        let target = app
            .world_mut()
            .spawn((Pop, GridPosition { x: 6, y: 5 }))
            .id();
        let bystander = app
            .world_mut()
            .spawn((Pop, GridPosition { x: 40, y: 40 }))
            .id();

        for _ in 0..40 {
            app.update();
        }
        assert!(
            app.world().get::<NostalgiaInfection>(target).is_some(),
            "close social contact with an infected pop should spread the plague"
        );
        assert!(
            app.world().get::<NostalgiaInfection>(bystander).is_none(),
            "distant pops must not be infected"
        );
    }

    /// Spec test 5: infected pops attempt to sabotage high-tech —
    /// through the REAL utility evaluator. `is_nostalgic` is exactly
    /// what `PopEvalData::from_query_item` now reports for a pop
    /// carrying `NostalgiaInfection`.
    #[test]
    fn test_nostalgia_infected_pops_attempt_to_sabotage_hightech() {
        let mut data = PopEvalData::test_instance();
        data.is_nostalgic = true; // as from_query_item reports for infected pops

        let mut buffer = UtilityAIBuffer::default();
        let basic = ScorableCandidate::new(Entity::from_raw(10), GridPosition { x: 2, y: 0 });
        let mut advanced =
            ScorableCandidate::new(Entity::from_raw(20), GridPosition { x: 9, y: 0 });
        advanced.is_advanced_tech = true;
        buffer.all_structures.push(basic);
        buffer.all_structures.push(advanced);

        let result = evaluate_sabotage(&data, &buffer);
        assert!(result.is_some(), "infected pop should select sabotage");
        let (action, _utility, target) = result.unwrap();
        assert_eq!(action, ActionType::Sabotage);
        assert_eq!(target, Some(Entity::from_raw(20)));
        // The sabotage target classifies as high-tech under the new tier map.
        assert_eq!(tech_tier_for_building(BuildingType::AICore), TechTier::HighTech);
    }

    /// Cure seam: the reality anchor sheds the infection.
    #[test]
    fn test_reality_anchor_cures_infection() {
        let mut app = plague_app();
        app.world_mut().resource_mut::<NostalgiaPlagueState>().anchor_active = true;
        let pop = app
            .world_mut()
            .spawn((Pop, GridPosition { x: 0, y: 0 }, NostalgiaInfection::default()))
            .id();
        for _ in 0..60 {
            app.update();
        }
        assert!(
            app.world().get::<NostalgiaInfection>(pop).is_none(),
            "sustained reality-anchor broadcast should cure the infection"
        );
    }

    /// Anchor semantics from the live playtest: while the broadcast
    /// runs, a healthy pop beside an infected one cannot be infected.
    #[test]
    fn test_reality_anchor_blocks_new_infections() {
        let mut app = plague_app();
        app.world_mut().resource_mut::<NostalgiaPlagueState>().anchor_active = true;
        let _carrier = app
            .world_mut()
            .spawn((Pop, GridPosition { x: 5, y: 5 }, NostalgiaInfection::default()))
            .id();
        let target = app
            .world_mut()
            .spawn((Pop, GridPosition { x: 6, y: 5 }))
            .id();
        for _ in 0..60 {
            app.update();
        }
        assert!(
            app.world().get::<NostalgiaInfection>(target).is_none(),
            "the reality anchor must block exposure build-up while it broadcasts"
        );
    }

    /// IP guard: every public string this feature introduces uses
    /// original names only — no lifted franchise terms.
    #[test]
    fn test_nostalgia_plague_ip_guard() {
        let public_strings = [
            TechTier::Primitive.label(),
            TechTier::Modern.label(),
            TechTier::HighTech.label(),
            "The Nostalgia Plague takes hold: a colonist has succumbed to an overwhelming longing for the old days.",
        ];
        let banned = [
            "star wars", "star trek", "dune", "foundation", "hyperion",
            "warhammer", "earth", "terra", "before times", "good old days squad",
        ];
        for s in public_strings {
            let lower = s.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string {s:?} contains lifted term {term:?}"
                );
            }
        }
    }
}
