use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::map::GridPosition;
use crate::layer1::morale::{MoodModifier, Morale};
use crate::layer1::needs::Needs;
use crate::layer1::pop::{Job, JobType, Pop};
use crate::layer1::traits::{Trait, Traits};
use crate::layer1::zone::{ZoneGrid, ZoneType};
use bevy_ecs::prelude::*;
use std::collections::HashMap;

/// Component representing the social standing of a Pop.
#[derive(Component, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Prestige {
    /// The numeric prestige value (0-10+).
    pub value: u8,
}

impl Prestige {
    /// Calculate prestige from a job type.
    #[must_use]
    pub const fn from_job(job: JobType) -> Self {
        // Static per-JobType prestige (spec 388 section 8, architect's
        // answer; spec 113's tier sketch adapted to the real 13-variant
        // AssignmentType — there is no Miner/Scientist/Governor job in
        // the real enum, so DeepMining is the miner analog, Library /
        // Observatory work is the scholar tier, and Administrator is
        // the governor analog).
        let value = match job {
            JobType::Administrator => 10,
            JobType::LibraryWorker | JobType::ObservatoryWorker => 7,
            JobType::Doctor | JobType::RationingBureaucrat => 5,
            JobType::Sheriff => 3,
            JobType::Funeral | JobType::TavernVisitor => 2,
            // FarmWorker, DeepMining, HousingResident, Patient, Surgery
            // and any future variant default to 1 (Labor).
            _ => 1,
        };
        Self { value }
    }
}

/// The social class of a Pop, derived from Prestige.
#[derive(Component, Debug, Clone, PartialEq, Eq, Copy)]
pub enum SocialClass {
    /// Low prestige (0-3).
    Labor,
    /// Medium prestige (4-7).
    Middle,
    /// High prestige (8+).
    Elite,
}

impl SocialClass {
    /// Derives the class for a raw prestige value (Labor 0-3,
    /// Middle 4-7, Elite 8+).
    #[must_use]
    pub const fn from_prestige_value(value: u8) -> Self {
        if value >= 8 {
            Self::Elite
        } else if value >= 4 {
            Self::Middle
        } else {
            Self::Labor
        }
    }
}

/// Component tracking stress accumulated from class friction.
#[derive(Component, Debug, Clone)]
pub struct ClassFriction {
    /// Accumulated stress amount.
    pub amount: f32,
}

/// Calculate the social class for a given entity based on its Prestige.
#[must_use]
pub fn calculate_social_class(world: &World, entity: Entity) -> SocialClass {
    world
        .get::<Prestige>(entity)
        .map_or(SocialClass::Labor, |prestige| {
            SocialClass::from_prestige_value(prestige.value)
        })
}

/// System to update `SocialClass` component when Prestige changes.
pub fn update_social_class_system(
    mut commands: Commands,
    query: Query<(Entity, &Prestige), Changed<Prestige>>,
) {
    for (entity, prestige) in query.iter() {
        let class = SocialClass::from_prestige_value(prestige.value);
        commands.entity(entity).insert(class);
    }
}

/// System to generate friction when pops of different classes are near each other.
pub fn class_friction_system(
    mut commands: Commands,
    mut pops: Query<(
        Entity,
        &crate::layer1::map::GridPosition,
        &SocialClass,
        Option<&mut Morale>,
    )>,
    other_pops: Query<(Entity, &crate::layer1::map::GridPosition, &SocialClass)>,
) {
    // Naive O(N^2) for Green phase - optimize later
    for (entity, pos, class, mut morale_opt) in &mut pops {
        let mut friction_score = 0.0;

        for (other_entity, other_pos, other_class) in other_pops.iter() {
            if entity == other_entity {
                continue;
            }

            // Check if neighbors (radius 1 for housing friction)
            // Ideally, check if their *assigned beds* are near, but using position for now (assuming they are home/sleeping)
            let dist = pos
                .x
                .abs_diff(other_pos.x)
                .saturating_add(pos.y.abs_diff(other_pos.y))
                .min(i32::MAX as u32) as i32;

            if dist <= 2 {
                // Close proximity
                if class != other_class {
                    // Friction!
                    // Elite <-> Labor = High friction
                    // Elite <-> Middle = Medium
                    // Middle <-> Labor = Low
                    friction_score += 0.05;
                }
            }
        }

        if friction_score > 0.0 {
            commands.entity(entity).insert(ClassFriction {
                amount: friction_score,
            });
            if let Some(ref mut morale) = morale_opt {
                morale.add_modifier(MoodModifier {
                    label: "Class Friction".to_string(),
                    value: -friction_score * 10.0,
                    duration: 1,
                });
            }
        } else {
            commands.entity(entity).remove::<ClassFriction>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::map::GridPosition;
    use crate::layer1::morale::Morale;
    use crate::layer1::pop::{Job, JobType, Pop};

    #[test]
    fn test_job_prestige_mapping() {
        // Verify jobs map to correct prestige
        assert_eq!(Prestige::from_job(JobType::FarmWorker).value, 1); // Low
        assert_eq!(Prestige::from_job(JobType::LibraryWorker).value, 7); // Medium/High
        assert_eq!(Prestige::from_job(JobType::Administrator).value, 10); // High
    }

    #[test]
    fn test_social_class_calculation() {
        let mut world = World::new();

        // Low Prestige Pop
        let farmer = world
            .spawn((
                Pop,
                Job {
                    workplace: Entity::PLACEHOLDER,
                    job_type: JobType::FarmWorker,
                },
                Prestige { value: 1 },
            ))
            .id();

        // High Prestige Pop
        let admin = world
            .spawn((
                Pop,
                Job {
                    workplace: Entity::PLACEHOLDER,
                    job_type: JobType::Administrator,
                },
                Prestige { value: 10 },
            ))
            .id();

        // Run calculation logic (or system)
        let farmer_class = calculate_social_class(&world, farmer);
        let admin_class = calculate_social_class(&world, admin);

        assert_eq!(farmer_class, SocialClass::Labor);
        assert_eq!(admin_class, SocialClass::Elite);
    }

    #[test]
    fn test_class_friction_housing() {
        let mut world = World::new();

        // Create two pops living next to each other
        let labor_pop = world
            .spawn((
                Pop,
                SocialClass::Labor,
                GridPosition { x: 10, y: 10 }, // Home position (abstracted)
                Morale::default(),
            ))
            .id();

        let elite_pop = world
            .spawn((
                Pop,
                SocialClass::Elite,
                GridPosition { x: 10, y: 11 }, // Adjacent home
                Morale::default(),
            ))
            .id();

        // Run friction system
        let mut schedule = Schedule::default();
        schedule.add_systems(class_friction_system);
        schedule.run(&mut world);

        // Check for ClassFriction component or Morale penalty
        let friction = world.get::<ClassFriction>(labor_pop);
        assert!(friction.is_some());
        assert!(friction.unwrap().amount > 0.0);

        let friction_elite = world.get::<ClassFriction>(elite_pop);
        assert!(friction_elite.is_some());
    }

    #[test]
    fn test_no_friction_same_class() {
        let mut world = World::new();

        let pop1 = world
            .spawn((
                Pop,
                SocialClass::Labor,
                GridPosition { x: 10, y: 10 },
                Morale::default(),
            ))
            .id();

        let _pop2 = world
            .spawn((
                Pop,
                SocialClass::Labor,
                GridPosition { x: 10, y: 11 },
                Morale::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(class_friction_system);
        schedule.run(&mut world);

        assert!(world.get::<ClassFriction>(pop1).is_none());
    }
}

// ---------------------------------------------------------------------------
// Spec 388 Social Stratification: class friction from job prestige gaps.
//
// Grounding note: spec 113 already built the prestige leg in this module
// (`Prestige` / `Prestige::from_job` / `SocialClass` / `ClassFriction`)
// and spec 386 extended its use (mimicry's effective-status fallback).
// The missing legs — and the whole of spec 388 — were: the old
// `class_friction_system` above is O(N^2), keys off the `SocialClass`
// component alone (which the live schedule never populates), touches
// Morale only, and is registered in NO production schedule. This section
// builds the live system on the same components instead of duplicating
// them: status derives from `Prestige`, falling back to
// `Prestige::from_job` via the pop's real `Job` (the mimicry pattern);
// the penalty lands on the real `Needs.leisure` (0.0..=1.0 scale,
// clamped at 0.0 per spec section 7 — the sketch's 100.0 scale and
// `Job { prestige }` / `Transform` fields do not exist in this codebase;
// position is `GridPosition`).
//
// Spec section 5 REFACTOR items, implemented:
// * Spatial hash over pops instead of the O(N^2) pair scan.
// * Friction primarily in shared zones: the real shared-zone seam is
//   `ZoneGrid` (per-tile designations — Dining / Bedroom / ...; the
//   `RoomZone` entity layer paints into it, room_zones.rs). Two pops
//   standing on tiles of the same non-None zone type count as sharing
//   a zone and friction doubles; elsewhere plain proximity applies.
// * `Trait::Egalitarian` (new variant on the real trait seam) negates
//   friction for the pop who holds it — the other pop still feels it.
// ---------------------------------------------------------------------------

/// Chebyshev radius (tiles) within which class friction applies.
pub const FRICTION_RADIUS: u32 = 3;
/// Minimum prestige gap that produces friction (spec sketch: diff > 5).
pub const FRICTION_MIN_GAP: i32 = 6;
/// Leisure lost per tick per point of prestige gap, per neighbour.
pub const FRICTION_LEISURE_PER_GAP: f32 = 0.001;
/// Multiplier when both pops share a designated zone tile type.
pub const SHARED_ZONE_MULTIPLIER: f32 = 2.0;

/// Chronicle text the first time class friction bites colony-wide.
pub const STRATIFICATION_CHRONICLE_TEXT: &str =
    "Class friction sharpens in the colony: high-station and low-station pops are wearing on each other in the shared rooms.";

/// Tuning for class friction (resource; defaults are the spec values).
#[derive(Resource, Debug, Clone, Copy)]
pub struct StratificationConfig {
    /// Chebyshev radius (tiles) within which friction applies.
    pub radius: u32,
    /// Minimum prestige gap that produces friction.
    pub min_gap: i32,
    /// Leisure lost per tick per gap point per neighbour.
    pub leisure_per_gap: f32,
    /// Multiplier for pairs sharing a designated zone.
    pub shared_zone_multiplier: f32,
}

impl Default for StratificationConfig {
    fn default() -> Self {
        Self {
            radius: FRICTION_RADIUS,
            min_gap: FRICTION_MIN_GAP,
            leisure_per_gap: FRICTION_LEISURE_PER_GAP,
            shared_zone_multiplier: SHARED_ZONE_MULTIPLIER,
        }
    }
}

/// Running ledger of class friction (feeds STATS `friction=` / `strat=`).
#[derive(Resource, Debug, Default, Clone)]
pub struct StratificationStats {
    /// Total pop-ticks that took a friction penalty.
    pub friction_events: u64,
    /// Pops penalized on the most recent tick the system ran.
    pub affected_now: usize,
    /// Largest prestige gap among penalized pairs on that tick.
    pub max_gap_now: i32,
    /// Whether the one-shot friction chronicle entry has been sent.
    pub chronicle_sent: bool,
}

/// Pure friction for one ordered pair: the leisure `target` loses this
/// tick because of one neighbour at prestige gap `gap`. Deterministic —
/// no probability anywhere in spec 388.
#[must_use]
pub fn pair_friction(gap: i32, shared_zone: bool, egalitarian: bool) -> f32 {
    friction_amount(
        gap,
        shared_zone,
        egalitarian,
        FRICTION_MIN_GAP,
        FRICTION_LEISURE_PER_GAP,
        SHARED_ZONE_MULTIPLIER,
    )
}

/// Config-aware form of [`pair_friction`].
#[must_use]
pub fn friction_amount(
    gap: i32,
    shared_zone: bool,
    egalitarian: bool,
    min_gap: i32,
    leisure_per_gap: f32,
    shared_zone_multiplier: f32,
) -> f32 {
    if egalitarian || gap < min_gap {
        return 0.0;
    }
    let base = gap as f32 * leisure_per_gap;
    if shared_zone {
        base * shared_zone_multiplier
    } else {
        base
    }
}

/// Public-facing strings for spec 388 (IP-guard test scans these).
#[must_use]
pub fn stratification_public_strings() -> Vec<String> {
    vec![
        STRATIFICATION_CHRONICLE_TEXT.to_string(),
        "Class Friction".to_string(),
        "Social Stratification".to_string(),
    ]
}

/// Effective prestige of a pop: explicit `Prestige`, else derived from
/// its job, else the Labor floor (1) — the spec 386 mimicry pattern.
fn effective_prestige(prestige: Option<&Prestige>, job: Option<&Job>) -> i32 {
    if let Some(p) = prestige {
        return i32::from(p.value);
    }
    if let Some(j) = job {
        return i32::from(Prestige::from_job(j.job_type).value);
    }
    1
}

/// Census for STATS / headless: (pops currently carrying `ClassFriction`,
/// total friction pop-ticks in the ledger).
#[must_use]
pub fn stratification_census(world: &mut World) -> (usize, u64) {
    let affected = {
        let mut query = world.query::<&ClassFriction>();
        query.iter(world).count()
    };
    let events = world
        .get_resource::<StratificationStats>()
        .map_or(0, |stats| stats.friction_events);
    (affected, events)
}

/// STATS label for spec 388: pops currently in friction.
#[must_use]
pub fn stratification_stats_label(world: &mut World) -> String {
    let (affected, _) = stratification_census(world);
    affected.to_string()
}

/// STATS label for spec 388: total friction pop-ticks so far.
#[must_use]
pub fn stratification_events_label(world: &mut World) -> String {
    let (_, events) = stratification_census(world);
    events.to_string()
}

/// Spec 388 live system: nearby pops whose prestige differs sharply
/// wear on each other's leisure. Snapshot-first with a spatial hash
/// (cell = radius), deterministic pure pair math, `Needs.leisure`
/// clamped at 0.0, `ClassFriction` maintained as the visible marker
/// (amount = this tick's total leisure loss), and a one-shot chronicle
/// entry the first time two or more pops are caught in friction.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn apply_class_friction_system(
    mut commands: Commands,
    config: Option<Res<StratificationConfig>>,
    zone_grid: Option<Res<ZoneGrid>>,
    mut stats: Option<ResMut<StratificationStats>>,
    mut chronicle_events: Option<ResMut<Events<AddChronicleEvent>>>,
    mut query: Query<
        (
            Entity,
            &GridPosition,
            &mut Needs,
            Option<&Prestige>,
            Option<&Job>,
            Option<&Traits>,
        ),
        With<Pop>,
    >,
) {
    let cfg = config.map_or_else(StratificationConfig::default, |c| *c);
    let radius = cfg.radius.max(1);

    struct Snap {
        entity: Entity,
        pos: GridPosition,
        prestige: i32,
        egalitarian: bool,
        zone: ZoneType,
    }
    let mut snaps: Vec<Snap> = Vec::new();
    for (entity, pos, _needs, prestige, job, traits) in query.iter() {
        snaps.push(Snap {
            entity,
            pos: *pos,
            prestige: effective_prestige(prestige, job),
            egalitarian: traits.is_some_and(|t| t.has(Trait::Egalitarian)),
            zone: zone_grid
                .as_deref()
                .map_or(ZoneType::None, |grid| grid.get(pos.x, pos.y)),
        });
    }

    // Spatial hash: bucket index by cell, then check the 3x3 cells.
    let cell = i32::try_from(radius).unwrap_or(i32::MAX).max(1);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (idx, snap) in snaps.iter().enumerate() {
        grid.entry((snap.pos.x.div_euclid(cell), snap.pos.y.div_euclid(cell)))
            .or_default()
            .push(idx);
    }

    let mut totals: HashMap<Entity, f32> = HashMap::new();
    let mut max_gap = 0i32;
    for snap in &snaps {
        let cx = snap.pos.x.div_euclid(cell);
        let cy = snap.pos.y.div_euclid(cell);
        let mut total = 0.0f32;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(bucket) = grid.get(&(cx + dx, cy + dy)) else {
                    continue;
                };
                for &other_idx in bucket {
                    let other = &snaps[other_idx];
                    if other.entity == snap.entity
                        || other.pos.distance_chebyshev(snap.pos) > radius
                    {
                        continue;
                    }
                    let gap = (snap.prestige - other.prestige).abs();
                    let shared_zone =
                        snap.zone != ZoneType::None && snap.zone == other.zone;
                    let amount = friction_amount(
                        gap,
                        shared_zone,
                        snap.egalitarian,
                        cfg.min_gap,
                        cfg.leisure_per_gap,
                        cfg.shared_zone_multiplier,
                    );
                    if amount > 0.0 {
                        total += amount;
                        max_gap = max_gap.max(gap);
                    }
                }
            }
        }
        if total > 0.0 {
            totals.insert(snap.entity, total);
        }
    }

    let affected_now = totals.len();
    for (entity, _pos, mut needs, _prestige, _job, _traits) in query.iter_mut() {
        if let Some(total) = totals.get(&entity) {
            needs.leisure = (needs.leisure - total).max(0.0);
            commands.entity(entity).insert(ClassFriction { amount: *total });
        } else {
            commands.entity(entity).remove::<ClassFriction>();
        }
    }

    if let Some(stats) = stats.as_deref_mut() {
        stats.affected_now = affected_now;
        stats.max_gap_now = if affected_now > 0 { max_gap } else { 0 };
        stats.friction_events += affected_now as u64;
        if affected_now >= 2 && !stats.chronicle_sent {
            stats.chronicle_sent = true;
            if let Some(events) = chronicle_events.as_deref_mut() {
                events.send(AddChronicleEvent {
                    text: STRATIFICATION_CHRONICLE_TEXT.to_string(),
                    importance: EventImportance::Standard,
                });
            }
        }
    }
}


#[cfg(test)]
mod tests388 {
    use super::*;
    use crate::layer1::map::GridPosition;
    use crate::layer1::needs::Needs;
    use crate::layer1::pop::Pop;
    use crate::layer1::traits::{Trait, Traits};
    use crate::layer1::zone::{ZoneGrid, ZoneType};

    fn run(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(apply_class_friction_system);
        schedule.run(world);
    }

    fn spawn_needy(world: &mut World, x: i32, y: i32, prestige: u8) -> Entity {
        world
            .spawn((
                Pop,
                GridPosition { x, y },
                Needs::default(),
                Prestige { value: prestige },
            ))
            .id()
    }

    #[test]
    fn test388_friction_between_different_prestige() {
        let mut world = World::new();
        let high = spawn_needy(&mut world, 0, 0, 10);
        let low = spawn_needy(&mut world, 1, 0, 1);
        run(&mut world);
        let low_needs = world.get::<Needs>(low).unwrap();
        assert!(low_needs.leisure < 0.8, "low pop leisure should drop, got {}", low_needs.leisure);
        let high_needs = world.get::<Needs>(high).unwrap();
        assert!(high_needs.leisure < 0.8, "friction is mutual, got {}", high_needs.leisure);
        assert!(world.get::<ClassFriction>(low).unwrap().amount > 0.0);
    }

    #[test]
    fn test388_no_friction_same_prestige() {
        let mut world = World::new();
        let a = spawn_needy(&mut world, 0, 0, 1);
        let _b = spawn_needy(&mut world, 1, 0, 1);
        run(&mut world);
        assert_eq!(world.get::<Needs>(a).unwrap().leisure, 0.8);
        assert!(world.get::<ClassFriction>(a).is_none());
    }

    #[test]
    fn test388_small_gap_below_threshold_no_friction() {
        // Doctor-tier (5) beside Labor (1): gap 4, under the spec's >5 line.
        assert_eq!(pair_friction(4, false, false), 0.0);
        let mut world = World::new();
        let a = spawn_needy(&mut world, 0, 0, 5);
        let _b = spawn_needy(&mut world, 1, 0, 1);
        run(&mut world);
        assert_eq!(world.get::<Needs>(a).unwrap().leisure, 0.8);
    }

    #[test]
    fn test388_pair_friction_pure_helper() {
        assert_eq!(pair_friction(0, false, false), 0.0);
        assert!(pair_friction(9, false, false) > pair_friction(6, false, false));
        assert!(pair_friction(6, false, false) > 0.0);
        // Shared zone doubles; egalitarian negates.
        assert!((pair_friction(9, true, false) - 2.0 * pair_friction(9, false, false)).abs() < 1e-6);
        assert_eq!(pair_friction(9, true, true), 0.0);
        assert_eq!(pair_friction(9, false, true), 0.0);
    }

    #[test]
    fn test388_out_of_radius_no_friction() {
        let mut world = World::new();
        let a = spawn_needy(&mut world, 0, 0, 10);
        let _b = spawn_needy(&mut world, 10, 0, 1);
        run(&mut world);
        assert_eq!(world.get::<Needs>(a).unwrap().leisure, 0.8);
    }

    #[test]
    fn test388_egalitarian_negates_own_friction() {
        let mut world = World::new();
        let mut traits = Traits::default();
        traits.add(Trait::Egalitarian);
        let egal = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Needs::default(), Prestige { value: 1 }, traits))
            .id();
        let high = spawn_needy(&mut world, 1, 0, 10);
        run(&mut world);
        assert_eq!(world.get::<Needs>(egal).unwrap().leisure, 0.8, "egalitarian pop feels no friction");
        assert!(world.get::<Needs>(high).unwrap().leisure < 0.8, "the other pop still does");
    }

    #[test]
    fn test388_leisure_clamped_at_zero() {
        let mut world = World::new();
        let low = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Needs { hunger: 0.8, rest: 0.8, leisure: 0.001, hygiene: 0.8 }, Prestige { value: 1 }))
            .id();
        let _high = spawn_needy(&mut world, 1, 0, 10);
        for _ in 0..10 {
            run(&mut world);
        }
        let leisure = world.get::<Needs>(low).unwrap().leisure;
        assert_eq!(leisure, 0.0, "leisure clamps at 0.0, got {leisure}");
    }

    #[test]
    fn test388_shared_zone_doubles_friction() {
        // Pure-proximity world.
        let mut w1 = World::new();
        let a1 = spawn_needy(&mut w1, 0, 0, 1);
        let _b1 = spawn_needy(&mut w1, 1, 0, 10);
        run(&mut w1);
        let drop1 = 0.8 - w1.get::<Needs>(a1).unwrap().leisure;
        // Same pair, both standing in one designated Dining zone.
        let mut w2 = World::new();
        let mut grid = ZoneGrid::new(10, 10);
        grid.set(0, 0, ZoneType::Dining);
        grid.set(1, 0, ZoneType::Dining);
        w2.insert_resource(grid);
        let a2 = spawn_needy(&mut w2, 0, 0, 1);
        let _b2 = spawn_needy(&mut w2, 1, 0, 10);
        run(&mut w2);
        let drop2 = 0.8 - w2.get::<Needs>(a2).unwrap().leisure;
        assert!(drop1 > 0.0 && (drop2 - 2.0 * drop1).abs() < 1e-6, "shared-zone drop {drop2} should be 2x proximity drop {drop1}");
    }

    #[test]
    fn test388_prestige_mapping_expanded() {
        assert_eq!(Prestige::from_job(JobType::DeepMining).value, 1);
        assert_eq!(Prestige::from_job(JobType::Sheriff).value, 3);
        assert_eq!(Prestige::from_job(JobType::Doctor).value, 5);
        assert_eq!(Prestige::from_job(JobType::RationingBureaucrat).value, 5);
        assert_eq!(Prestige::from_job(JobType::LibraryWorker).value, 7);
        assert_eq!(Prestige::from_job(JobType::Administrator).value, 10);
        assert_eq!(SocialClass::from_prestige_value(2), SocialClass::Labor);
        assert_eq!(SocialClass::from_prestige_value(5), SocialClass::Middle);
        assert_eq!(SocialClass::from_prestige_value(9), SocialClass::Elite);
    }

    #[test]
    fn test388_job_fallback_without_prestige_component() {
        // Pops with a Job but no Prestige component derive status from the job.
        let mut world = World::new();
        let low = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Needs::default(), crate::layer1::pop::Job { workplace: Entity::PLACEHOLDER, job_type: JobType::FarmWorker }))
            .id();
        let _high = world
            .spawn((Pop, GridPosition { x: 1, y: 0 }, Needs::default(), crate::layer1::pop::Job { workplace: Entity::PLACEHOLDER, job_type: JobType::Administrator }))
            .id();
        run(&mut world);
        assert!(world.get::<Needs>(low).unwrap().leisure < 0.8);
    }

    #[test]
    fn test388_stats_ledger_counts_affected() {
        let mut world = World::new();
        world.init_resource::<StratificationStats>();
        let _a = spawn_needy(&mut world, 0, 0, 10);
        let _b = spawn_needy(&mut world, 1, 0, 1);
        run(&mut world);
        let stats = world.resource::<StratificationStats>();
        assert_eq!(stats.affected_now, 2);
        assert_eq!(stats.max_gap_now, 9);
        assert!(stats.friction_events >= 2);
    }

    #[test]
    fn test388_ip_guard_public_strings_are_original() {
        let banned = [
            "dwarf fortress", "stellaris", "rimworld", "clean coats", "dusty boots",
            "windsor", "billy", "ghetto",
        ];
        for s in stratification_public_strings() {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(!lower.contains(b), "banned term '{b}' in public string: {s}");
            }
        }
    }
}
