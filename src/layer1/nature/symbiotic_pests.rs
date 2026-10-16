//! # Symbiotic Pests (Spec 337)
//!
//! Food storage and organic waste naturally generate "Vermin". Vermin are not
//! strictly bad — that's the whole point of the spec:
//!
//! - [`VerminSpecies::GrainRat`]: food thieves that spawn at unprotected
//!   stockpiles and drain the colony food ledger.
//! - [`VerminSpecies::BrainBorerBeetle`]: predators that hunt GrainRats
//!   within a hunt radius. Each kill heals the beetle; every third kill it
//!   reproduces (documented choice for the spec's "health or reproduction"
//!   fork: it does both, capped).
//! - [`VerminSpecies::SilkWeaverSpider`]: scavenges a little food, weaves
//!   silk over time ([`Vermin::silk_ready`]). Silk is banked into
//!   [`SilkLedger`] by the headless `vermin harvest` command, or
//!   automatically when spiders are culled. Documented choice for the
//!   spec's "predates rats too, or just scavenges": spiders just scavenge —
//!   the predator niche belongs to the beetle.
//!
//! Disease is a light touch: pops near vermin can catch "Vermin Fever" (a
//! mood modifier plus 1.0 health damage) through a timer-gated, low-chance
//! roll. No needs-module refactor.
//!
//! Symbiosis mechanics (the interesting tradeoffs):
//! - Exterminating rats converts them to meat (+[`RAT_MEAT_YIELD`] food each).
//! - Spiders are passive silk producers — culling them harvests the silk
//!   they've woven.
//! - Beetles keep rats in check; rats arriving at all means food is
//!   unprotected.
//! - Counterplay: stockpiles can be tagged [`PestProof`] ("Pest Safe" per
//!   spec §7) — the spawn system skips them; `vermin protect` toggles it.
//!
//! What was deliberately left alone (see spec §5 + "Questions" style):
//! - Vermin do not move. Spec §7 suggests Utility-AI pathing for vermin;
//!   stationary lurkers are the sane minimal version — a movement pass is a
//!   future workbench item.
//! - Vermin do not age or starve to death. Population control is
//!   predation + the `exterminate` console command, plus a per-stockpile
//!   spawn cap.
//!
//! Bevy lessons honored: spawn checks are timer-gated (every
//! [`VERMIN_SPAWN_WINDOW_TICKS`] ticks, not every tick); predation uses one
//! pass plus an eaten-set instead of O(N^2) pairwise systems; RNG is a
//! tick-seeded [`StdRng`] (deterministic per tick, so tests are stable).
//!
//! Vermin entities carry no [`Pop`] component, so no needs/health/morale
//! system treats them as colonists. [`collect_pop_data`] additionally
//! carries an explicit `Without<Vermin>` filter for defense in depth (the
//! spec's requirement, honoring the 8-origins + panic pattern).

use std::collections::HashSet;

use bevy_ecs::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::layer1::biology::health::Health;
use crate::layer1::building::{Building, BuildingType};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::economy::stockpile::Stockpile;
use crate::layer1::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::shared::time::SimulationTime;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Spawn checks run every this many ticks (spec §5: timer-gated, not every tick).
pub const VERMIN_SPAWN_WINDOW_TICKS: u64 = 50;
/// Per-window spawn chance per unprotected stockpile, per unit of colony food.
pub const VERMIN_SPAWN_CHANCE_PER_FOOD: f32 = 0.0015;
/// Cap on vermin lurking near one stockpile (radius 4) — populations can't explode.
pub const VERMIN_MAX_PER_STOCKPILE: usize = 8;
/// Radius (Chebyshev) around a stockpile that counts as "near" for the cap.
pub const VERMIN_STOCKPILE_RADIUS: u32 = 4;
/// When this many rats are alive colony-wide, beetles get a bonus spawn chance.
pub const BEETLE_RAT_DENSITY_THRESHOLD: usize = 6;
/// Bonus beetle spawn chance per window once rats cross the density threshold.
pub const BEETLE_BONUS_SPAWN_CHANCE: f32 = 0.25;
/// Food drain windows.
pub const VERMIN_DRAIN_WINDOW_TICKS: u64 = 10;
/// Food drained per window per GrainRat.
pub const RAT_FOOD_DRAIN: f32 = 0.05;
/// Food drained per window per SilkWeaverSpider (they just scavenge).
pub const SPIDER_FOOD_DRAIN: f32 = 0.02;
/// Beetles eat no stored food at all.
pub const BEETLE_FOOD_DRAIN: f32 = 0.0;
/// Disease rolls happen every this many ticks.
pub const DISEASE_WINDOW_TICKS: u64 = 200;
/// Pops within this Chebyshev radius of a vermin risk infection.
pub const DISEASE_RADIUS: u32 = 3;
/// Per-window infection chance for a pop near vermin.
pub const DISEASE_CHANCE: f32 = 0.10;
/// Health damage from one bout of Vermin Fever.
pub const DISEASE_HEALTH_DAMAGE: f32 = 1.0;
/// Mood penalty for Vermin Fever.
pub const DISEASE_MOOD_PENALTY: f32 = -0.10;
/// How long the fever mood modifier lasts (ticks).
pub const DISEASE_MOOD_DURATION: u32 = 1200;
/// Mood modifier label.
pub const VERMIN_FEVER_LABEL: &str = "Vermin Fever";
/// Beetle hunt radius (Chebyshev tiles).
pub const BEETLE_HUNT_RADIUS: u32 = 8;
/// Health a beetle regains per kill (capped at max).
pub const BEETLE_HEAL_PER_KILL: f32 = 2.0;
/// Every this many kills, a beetle reproduces.
pub const BEETLE_KILLS_TO_REPRODUCE: u32 = 3;
/// Silk woven per weave window per spider.
pub const VERMIN_WEAVE_WINDOW_TICKS: u64 = 10;
/// Silk woven per window (silk_ready caps at 1.0).
pub const SPIDER_WEAVE_RATE: f32 = 0.05;
/// Food gained per culled rat (dressed for meat).
pub const RAT_MEAT_YIELD: f32 = 0.5;

// ---------------------------------------------------------------------------
// Components / resources
// ---------------------------------------------------------------------------

/// The three vermin species. Original names, concept-only (Spec 337 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerminSpecies {
    /// Food thief: spawns at unprotected food, drains the food ledger.
    GrainRat,
    /// Predator: hunts GrainRats within [`BEETLE_HUNT_RADIUS`] tiles.
    BrainBorerBeetle,
    /// Weaver: scavenges a little food, weaves harvestable silk.
    SilkWeaverSpider,
}

impl VerminSpecies {
    /// Public display name (used in console output + the IP-guard test).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::GrainRat => "Grain Rat",
            Self::BrainBorerBeetle => "Brain Borer Beetle",
            Self::SilkWeaverSpider => "Silk Weaver Spider",
        }
    }

    /// Short console alias.
    #[must_use]
    pub const fn short(&self) -> &'static str {
        match self {
            Self::GrainRat => "rat",
            Self::BrainBorerBeetle => "beetle",
            Self::SilkWeaverSpider => "spider",
        }
    }

    /// Parse a console token (case-insensitive, accepts label or alias).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let lower = s.to_ascii_lowercase();
        let lower = lower.replace([' ', '-', '_'], "");
        match lower.as_str() {
            "rat" | "rats" | "grainrat" => Some(Self::GrainRat),
            "beetle" | "beetles" | "brainborerbeetle" | "brainborer" => {
                Some(Self::BrainBorerBeetle)
            }
            "spider" | "spiders" | "silkweaverspider" | "silkweaver" => {
                Some(Self::SilkWeaverSpider)
            }
            _ => None,
        }
    }

    /// All three species in stable order.
    #[must_use]
    pub const fn all() -> [Self; 3] {
        [Self::GrainRat, Self::BrainBorerBeetle, Self::SilkWeaverSpider]
    }
}

/// A vermin entity. Carries NO [`Pop`] component — vermin are wildlife, not
/// colonists, so no needs/health/morale/utility system treats them as pops.
#[derive(Component, Debug, Clone, Copy)]
pub struct Vermin {
    /// Which species this vermin is.
    pub species: VerminSpecies,
    /// Hit points (only beetles currently gain/lose it via predation).
    pub health: f32,
    /// Max hit points.
    pub max_health: f32,
    /// Beetles only: kills toward the next reproduction.
    pub kills: u32,
    /// Spiders only: woven silk ready for harvest (0.0–1.0).
    pub silk_ready: f32,
}

impl Vermin {
    /// A fresh GrainRat.
    #[must_use]
    pub const fn grain_rat() -> Self {
        Self {
            species: VerminSpecies::GrainRat,
            health: 10.0,
            max_health: 10.0,
            kills: 0,
            silk_ready: 0.0,
        }
    }

    /// A fresh BrainBorerBeetle.
    #[must_use]
    pub const fn brain_borer() -> Self {
        Self {
            species: VerminSpecies::BrainBorerBeetle,
            health: 20.0,
            max_health: 20.0,
            kills: 0,
            silk_ready: 0.0,
        }
    }

    /// A fresh SilkWeaverSpider.
    #[must_use]
    pub const fn silk_weaver() -> Self {
        Self {
            species: VerminSpecies::SilkWeaverSpider,
            health: 12.0,
            max_health: 12.0,
            kills: 0,
            silk_ready: 0.0,
        }
    }

    /// Fresh vermin of the given species.
    #[must_use]
    pub const fn of(species: VerminSpecies) -> Self {
        match species {
            VerminSpecies::GrainRat => Self::grain_rat(),
            VerminSpecies::BrainBorerBeetle => Self::brain_borer(),
            VerminSpecies::SilkWeaverSpider => Self::silk_weaver(),
        }
    }
}

/// Marker for a stockpile building sealed against vermin (spec §7 "Pest Safe"
/// — refrigerated or sealed silos). The spawn system skips these; the
/// headless `vermin protect` command toggles the marker.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct PestProof;

/// Silk bank. No `ResourceType::Silk` exists, so woven silk lives in this
/// dedicated ledger (wired consistently: `vermin harvest` and spider culling
/// add to it; console `vermin` reports it).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct SilkLedger {
    /// Banked silk units.
    pub silk: f32,
}

// ---------------------------------------------------------------------------
// Pure helpers (deterministic, unit-testable)
// ---------------------------------------------------------------------------

/// Per-window spawn chance for one unprotected stockpile, scaling with the
/// amount of unprotected food (spec §5 refactor: not a flat probability).
/// `food` is the colony-wide food level.
#[must_use]
pub fn spawn_chance(food: f32) -> f32 {
    if food <= 0.0 {
        0.0
    } else {
        (food * VERMIN_SPAWN_CHANCE_PER_FOOD).min(1.0)
    }
}

/// Pick the spawned species from a roll in [0, 1): rats are common, spiders
/// occasional, beetles rare.
#[must_use]
pub fn pick_species(roll: f32) -> VerminSpecies {
    if roll < 0.70 {
        VerminSpecies::GrainRat
    } else if roll < 0.90 {
        VerminSpecies::SilkWeaverSpider
    } else {
        VerminSpecies::BrainBorerBeetle
    }
}

/// Tick-seeded RNG for a system run — deterministic per tick, so playthroughs
/// and tests are reproducible.
fn tick_rng(tick: u64, salt: u64) -> StdRng {
    StdRng::seed_from_u64(tick.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(salt))
}

fn send_vermin_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    world.init_resource::<Events<AddChronicleEvent>>();
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: text.to_string(),
            importance,
        });
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Query alias for unprotected stockpile buildings (keeps clippy's
/// `type_complexity` lint quiet).
type UnprotectedStockpiles<'w, 's> = Query<
    'w,
    's,
    (&'static Building, &'static GridPosition),
    (With<Stockpile>, Without<PestProof>),
>;

/// Timer-gated vermin spawns: unprotected stockpile buildings with colony food
/// in the ledger attract vermin. Chance scales with food (see
/// [`spawn_chance`]); capped per stockpile; beetles get a bonus spawn when
/// rats are dense (predator symbiosis).
pub fn vermin_spawn_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    resources: Res<ColonyResources>,
    stockpiles: UnprotectedStockpiles<'_, '_>,
    vermin: Query<(&Vermin, &GridPosition)>,
) {
    if !time.tick.is_multiple_of(VERMIN_SPAWN_WINDOW_TICKS) {
        return;
    }
    let mut rng = tick_rng(time.tick, 0x337);
    let chance = spawn_chance(resources.food);
    if chance <= 0.0 {
        return;
    }
    let rat_total = vermin
        .iter()
        .filter(|(v, _)| v.species == VerminSpecies::GrainRat)
        .count();

    for (building, pos) in &stockpiles {
        if building.building_type != BuildingType::Stockpile {
            continue;
        }
        let nearby = vermin
            .iter()
            .filter(|(_, vpos)| pos.distance_chebyshev(**vpos) <= VERMIN_STOCKPILE_RADIUS)
            .count();
        if nearby >= VERMIN_MAX_PER_STOCKPILE {
            continue;
        }
        if rng.gen::<f32>() < chance {
            let species = pick_species(rng.gen::<f32>());
            commands.spawn((Vermin::of(species), *pos));
        }
        // Predator symbiosis: rats plentiful → a beetle may move in.
        if rat_total >= BEETLE_RAT_DENSITY_THRESHOLD
            && rng.gen::<f32>() < BEETLE_BONUS_SPAWN_CHANCE
        {
            commands.spawn((Vermin::brain_borer(), *pos));
        }
    }
}

/// Timer-gated food drain: rats (and, lightly, scavenging spiders) eat from
/// the colony food ledger.
pub fn vermin_food_drain_system(
    time: Res<SimulationTime>,
    mut resources: ResMut<ColonyResources>,
    vermin: Query<&Vermin>,
) {
    if !time.tick.is_multiple_of(VERMIN_DRAIN_WINDOW_TICKS) {
        return;
    }
    let mut drain = 0.0;
    for v in &vermin {
        drain += match v.species {
            VerminSpecies::GrainRat => RAT_FOOD_DRAIN,
            VerminSpecies::SilkWeaverSpider => SPIDER_FOOD_DRAIN,
            VerminSpecies::BrainBorerBeetle => BEETLE_FOOD_DRAIN,
        };
    }
    if drain > 0.0 {
        resources.food = (resources.food - drain).max(0.0);
    }
}

/// Beetles hunt GrainRats within [`BEETLE_HUNT_RADIUS`]. Each kill heals the
/// beetle (+[`BEETLE_HEAL_PER_KILL`], capped); every
/// [`BEETLE_KILLS_TO_REPRODUCE`] kills it reproduces (one pass + an eaten-set,
/// no O(N^2) pairwise systems).
pub fn vermin_predation_system(
    mut commands: Commands,
    mut vermin: Query<(Entity, &mut Vermin, &GridPosition)>,
) {
    let beetles: Vec<(Entity, GridPosition)> = vermin
        .iter()
        .filter(|(_, v, _)| v.species == VerminSpecies::BrainBorerBeetle)
        .map(|(e, _, p)| (e, *p))
        .collect();
    if beetles.is_empty() {
        return;
    }
    let mut eaten: HashSet<Entity> = HashSet::new();
    let mut new_beetles: Vec<GridPosition> = Vec::new();
    for (beetle, bpos) in beetles {
        let mut best: Option<(Entity, u32)> = None;
        for (e, v, p) in vermin.iter() {
            if v.species != VerminSpecies::GrainRat || eaten.contains(&e) {
                continue;
            }
            let d = bpos.distance_chebyshev(*p);
            if d <= BEETLE_HUNT_RADIUS && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((e, d));
            }
        }
        let Some((rat, _)) = best else { continue };
        eaten.insert(rat);
        commands.entity(rat).despawn();
        if let Ok((_, mut bv, _)) = vermin.get_mut(beetle) {
            bv.health = (bv.health + BEETLE_HEAL_PER_KILL).min(bv.max_health);
            bv.kills += 1;
            if bv.kills % BEETLE_KILLS_TO_REPRODUCE == 0 {
                new_beetles.push(bpos);
            }
        }
    }
    for pos in new_beetles {
        commands.spawn((Vermin::brain_borer(), pos));
    }
}

/// Timer-gated silk weaving: living spiders accumulate [`Vermin::silk_ready`]
/// up to 1.0, harvestable via the console.
pub fn vermin_weave_system(time: Res<SimulationTime>, mut vermin: Query<&mut Vermin>) {
    if !time.tick.is_multiple_of(VERMIN_WEAVE_WINDOW_TICKS) {
        return;
    }
    for mut v in &mut vermin {
        if v.species == VerminSpecies::SilkWeaverSpider {
            v.silk_ready = (v.silk_ready + SPIDER_WEAVE_RATE).min(1.0);
        }
    }
}

/// Timer-gated disease: pops near vermin roll a small infection chance and,
/// on success, gain the "Vermin Fever" mood modifier plus a little health
/// damage (light touch — no needs-module refactor).
pub fn vermin_disease_system(world: &mut World) {
    let tick = world.resource::<SimulationTime>().tick;
    if !tick.is_multiple_of(DISEASE_WINDOW_TICKS) {
        return;
    }
    let vermin_positions: Vec<GridPosition> = world
        .query_filtered::<&GridPosition, With<Vermin>>()
        .iter(world)
        .copied()
        .collect();
    if vermin_positions.is_empty() {
        return;
    }
    let mut rng = tick_rng(tick, 0xD15EA5E);
    let mut new_infections = 0u32;
    let pops: Vec<Entity> = world
        .query_filtered::<Entity, (With<Pop>, With<GridPosition>)>()
        .iter(world)
        .collect();
    for pop in pops {
        let Some(pos) = world.get::<GridPosition>(pop).copied() else {
            continue;
        };
        let near = vermin_positions
            .iter()
            .any(|vp| pos.distance_chebyshev(*vp) <= DISEASE_RADIUS);
        if !near {
            continue;
        }
        if rng.gen::<f32>() < DISEASE_CHANCE {
            apply_vermin_fever(world, pop);
            new_infections += 1;
        }
    }
    if new_infections > 0 {
        send_vermin_chronicle(
            world,
            &format!(
                "Vermin fever is spreading through the colony ({new_infections} new case{}).",
                if new_infections == 1 { "" } else { "s" }
            ),
            EventImportance::Minor,
        );
    }
}

/// Apply one bout of Vermin Fever to a pop (idempotent on the mood modifier).
pub fn apply_vermin_fever(world: &mut World, pop: Entity) {
    if let Some(mut morale) = world.get_mut::<Morale>(pop) {
        if !morale
            .modifiers
            .iter()
            .any(|m| m.label == VERMIN_FEVER_LABEL)
        {
            morale.modifiers.push(MoodModifier {
                label: VERMIN_FEVER_LABEL.to_string(),
                value: DISEASE_MOOD_PENALTY,
                duration: DISEASE_MOOD_DURATION,
            });
        }
    }
    if let Some(mut health) = world.get_mut::<Health>(pop) {
        health.current = (health.current - DISEASE_HEALTH_DAMAGE).max(0.0);
    }
    // Item 84: attribute the damage source at the moment of damage.
    crate::layer1::biology::death_attribution::note_damage_world(
        world,
        pop,
        crate::layer1::biology::death_attribution::CAUSE_VERMIN_FEVER,
    );
}

// ---------------------------------------------------------------------------
// Console API (used by the headless bin)
// ---------------------------------------------------------------------------

/// Spawn one vermin of the given species at a tile. Returns the entity.
pub fn spawn_vermin_at(world: &mut World, species: VerminSpecies, x: i32, y: i32) -> Entity {
    world
        .spawn((Vermin::of(species), GridPosition { x, y }))
        .id()
}

/// Count vermin by species: (rats, beetles, spiders).
pub fn vermin_counts(world: &mut World) -> (usize, usize, usize) {
    let mut rats = 0;
    let mut beetles = 0;
    let mut spiders = 0;
    let mut q = world.query::<&Vermin>();
    for v in q.iter(world) {
        match v.species {
            VerminSpecies::GrainRat => rats += 1,
            VerminSpecies::BrainBorerBeetle => beetles += 1,
            VerminSpecies::SilkWeaverSpider => spiders += 1,
        }
    }
    (rats, beetles, spiders)
}

/// Total vermin alive (for the STATS `vermin=` field).
pub fn vermin_total(world: &mut World) -> usize {
    world.query::<&Vermin>().iter(world).count()
}

/// Report from an extermination.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExterminateReport {
    /// Rats culled.
    pub rats: usize,
    /// Beetles culled.
    pub beetles: usize,
    /// Spiders culled.
    pub spiders: usize,
    /// Food gained from culled rats (meat).
    pub food_gained: f32,
    /// Silk banked from culled spiders' woven silk.
    pub silk_gained: f32,
}

/// Cull vermin (player population control, spec §7). Rats are dressed for
/// meat (+[`RAT_MEAT_YIELD`] food each into the colony ledger); spiders have
/// their woven silk banked into [`SilkLedger`]; beetles yield nothing.
/// `species = None` exterminates everything.
pub fn exterminate_vermin(world: &mut World, species: Option<VerminSpecies>) -> ExterminateReport {
    world.init_resource::<SilkLedger>();
    world.init_resource::<ColonyResources>();
    let mut report = ExterminateReport::default();
    let targets: Vec<(Entity, Vermin)> = {
        let mut q = world.query::<(Entity, &Vermin)>();
        q.iter(world)
            .filter(|(_, v)| species.is_none_or(|s| v.species == s))
            .map(|(e, v)| (e, *v))
            .collect()
    };
    let mut silk = 0.0;
    let mut meat = 0.0;
    for (entity, v) in targets {
        match v.species {
            VerminSpecies::GrainRat => {
                report.rats += 1;
                meat += RAT_MEAT_YIELD;
            }
            VerminSpecies::BrainBorerBeetle => report.beetles += 1,
            VerminSpecies::SilkWeaverSpider => {
                report.spiders += 1;
                silk += v.silk_ready;
            }
        }
        world.despawn(entity);
    }
    report.food_gained = meat;
    report.silk_gained = silk;
    if meat > 0.0 {
        world.resource_mut::<ColonyResources>().food += meat;
    }
    if silk > 0.0 {
        world.resource_mut::<SilkLedger>().silk += silk;
    }
    if report.rats + report.beetles + report.spiders > 0 {
        send_vermin_chronicle(
            world,
            &format!(
                "Vermin extermination: {} culled{}",
                report.rats + report.beetles + report.spiders,
                if meat > 0.0 {
                    format!(" (+{meat:.1} food from dressed rats)")
                } else {
                    String::new()
                }
            ),
            EventImportance::Minor,
        );
    }
    report
}

/// Harvest woven silk from all living spiders into [`SilkLedger`]; resets
/// each spider's `silk_ready` to 0. Returns the amount banked.
pub fn harvest_silk(world: &mut World) -> f32 {
    world.init_resource::<SilkLedger>();
    let mut banked = 0.0;
    let mut q = world.query::<&mut Vermin>();
    for mut v in q.iter_mut(world) {
        if v.species == VerminSpecies::SilkWeaverSpider && v.silk_ready > 0.0 {
            banked += v.silk_ready;
            v.silk_ready = 0.0;
        }
    }
    if banked > 0.0 {
        world.resource_mut::<SilkLedger>().silk += banked;
    }
    banked
}

/// Banked silk (for console status).
pub fn banked_silk(world: &mut World) -> f32 {
    world.init_resource::<SilkLedger>();
    world.resource::<SilkLedger>().silk
}

/// Total woven-but-unharvested silk on living spiders.
pub fn unharvested_silk(world: &mut World) -> f32 {
    let mut q = world.query::<&Vermin>();
    q.iter(world)
        .filter(|v| v.species == VerminSpecies::SilkWeaverSpider)
        .map(|v| v.silk_ready)
        .sum()
}

/// Toggle the [`PestProof`] marker on the stockpile building nearest to
/// (x, y) (spec §7: tag stockpiles "Pest Safe"). Returns `None` when no
/// stockpile building exists; otherwise the new protected state.
pub fn toggle_pest_proof(world: &mut World, x: i32, y: i32) -> Option<bool> {
    let target = GridPosition { x, y };
    let best: Option<(Entity, u32)> = {
        let mut q = world.query::<(Entity, &Building, &GridPosition)>();
        q.iter(world)
            .filter(|(_, b, _)| b.building_type == BuildingType::Stockpile)
            .map(|(e, _, p)| (e, target.distance_chebyshev(*p)))
            .min_by_key(|(_, d)| *d)
    };
    let (entity, _) = best?;
    if world.get::<PestProof>(entity).is_some() {
        world.entity_mut(entity).remove::<PestProof>();
        Some(false)
    } else {
        world.entity_mut(entity).insert(PestProof);
        Some(true)
    }
}

// ---------------------------------------------------------------------------
// Tests (spec RED phase, adapted to the real codebase types)
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::layer1::nature::terrain::generate_terrain;
    use crate::setup::init_task_pools;
    use bevy_ecs::system::RunSystemOnce;

    fn setup() -> World {
        init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(SilkLedger::default());
        world.insert_resource(generate_terrain(60, 60));
        world
    }

    fn spawn_stockpile(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Building {
                    building_type: BuildingType::Stockpile,
                },
                Stockpile::default(),
                GridPosition { x, y },
            ))
            .id()
    }

    fn spawn_pop(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Pop,
                GridPosition { x, y },
                Morale::default(),
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id()
    }

    #[test]
    fn test_spawn_chance_scales_with_food() {
        assert_eq!(spawn_chance(0.0), 0.0);
        assert_eq!(spawn_chance(-5.0), 0.0);
        assert!(spawn_chance(100.0) > 0.0);
        assert!(spawn_chance(100.0) < spawn_chance(200.0));
        assert_eq!(spawn_chance(100_000.0), 1.0, "chance caps at 1.0");
    }

    #[test]
    fn test_pick_species_distribution() {
        assert_eq!(pick_species(0.0), VerminSpecies::GrainRat);
        assert_eq!(pick_species(0.69), VerminSpecies::GrainRat);
        assert_eq!(pick_species(0.70), VerminSpecies::SilkWeaverSpider);
        assert_eq!(pick_species(0.89), VerminSpecies::SilkWeaverSpider);
        assert_eq!(pick_species(0.90), VerminSpecies::BrainBorerBeetle);
        assert_eq!(pick_species(0.999), VerminSpecies::BrainBorerBeetle);
    }

    #[test]
    fn test_vermin_spawn_near_unprotected_food() {
        let mut world = setup();
        spawn_stockpile(&mut world, 10, 10);
        world.resource_mut::<ColonyResources>().food = 100_000.0; // chance = 1.0
        world.resource_mut::<SimulationTime>().tick = VERMIN_SPAWN_WINDOW_TICKS;

        world.run_system_once(vermin_spawn_system).unwrap();

        let count = vermin_total(&mut world);
        assert!(
            count > 0,
            "vermin should spawn near unprotected food over time"
        );
    }

    #[test]
    fn test_no_spawn_when_food_zero() {
        let mut world = setup();
        spawn_stockpile(&mut world, 10, 10);
        world.resource_mut::<ColonyResources>().food = 0.0;
        world.resource_mut::<SimulationTime>().tick = VERMIN_SPAWN_WINDOW_TICKS;

        world.run_system_once(vermin_spawn_system).unwrap();

        assert_eq!(vermin_total(&mut world), 0, "no food, no vermin");
    }

    #[test]
    fn test_pest_proof_stockpile_spawns_nothing() {
        let mut world = setup();
        let stockpile = spawn_stockpile(&mut world, 10, 10);
        world.entity_mut(stockpile).insert(PestProof);
        world.resource_mut::<ColonyResources>().food = 100_000.0;
        world.resource_mut::<SimulationTime>().tick = VERMIN_SPAWN_WINDOW_TICKS;

        world.run_system_once(vermin_spawn_system).unwrap();

        assert_eq!(
            vermin_total(&mut world),
            0,
            "PestProof stockpiles must not attract vermin"
        );
    }

    #[test]
    fn test_spawn_gated_to_window() {
        let mut world = setup();
        spawn_stockpile(&mut world, 10, 10);
        world.resource_mut::<ColonyResources>().food = 100_000.0;
        // Tick is NOT on a spawn window boundary.
        world.resource_mut::<SimulationTime>().tick = VERMIN_SPAWN_WINDOW_TICKS + 1;

        world.run_system_once(vermin_spawn_system).unwrap();

        assert_eq!(vermin_total(&mut world), 0, "spawn must be timer-gated");
    }

    #[test]
    fn test_predator_vermin_consume_prey_vermin() {
        let mut world = setup();
        let prey = spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 5, 5);
        let predator = spawn_vermin_at(&mut world, VerminSpecies::BrainBorerBeetle, 6, 6);
        world.get_mut::<Vermin>(predator).unwrap().health = 10.0;

        world.run_system_once(vermin_predation_system).unwrap();

        assert!(
            world.get_entity(prey).is_err(),
            "the rat should be eaten"
        );
        let bv = world.get::<Vermin>(predator).unwrap();
        assert_eq!(bv.kills, 1, "the kill must be recorded");
        assert!(
            bv.health > 10.0,
            "the predator must heal on a kill, got {}",
            bv.health
        );
    }

    #[test]
    fn test_predator_reproduces_every_third_kill() {
        let mut world = setup();
        let predator = spawn_vermin_at(&mut world, VerminSpecies::BrainBorerBeetle, 5, 5);
        world.get_mut::<Vermin>(predator).unwrap().kills = 2;
        spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 5, 6);

        world.run_system_once(vermin_predation_system).unwrap();

        let (rats, beetles, _) = vermin_counts(&mut world);
        assert_eq!(rats, 0, "the rat was eaten");
        assert_eq!(beetles, 2, "third kill reproduces a beetle");
        assert_eq!(
            world.get::<Vermin>(predator).unwrap().kills,
            3,
            "kill counter must hit 3"
        );
    }

    #[test]
    fn test_predator_ignores_distant_prey() {
        let mut world = setup();
        let prey = spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 50, 50);
        spawn_vermin_at(&mut world, VerminSpecies::BrainBorerBeetle, 5, 5);

        world.run_system_once(vermin_predation_system).unwrap();

        assert!(
            world.get_entity(prey).is_ok(),
            "prey outside the hunt radius must survive"
        );
    }

    #[test]
    fn test_food_drain() {
        let mut world = setup();
        spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 5, 5);
        spawn_vermin_at(&mut world, VerminSpecies::SilkWeaverSpider, 5, 6);
        world.resource_mut::<ColonyResources>().food = 10.0;
        world.resource_mut::<SimulationTime>().tick = VERMIN_DRAIN_WINDOW_TICKS;

        world.run_system_once(vermin_food_drain_system).unwrap();

        let food = world.resource::<ColonyResources>().food;
        let expected = 10.0 - RAT_FOOD_DRAIN - SPIDER_FOOD_DRAIN;
        assert!(
            (food - expected).abs() < 1e-5,
            "food should drain by rat + spider amounts, got {food}"
        );
    }

    #[test]
    fn test_food_drain_never_negative() {
        let mut world = setup();
        spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 5, 5);
        world.resource_mut::<ColonyResources>().food = 0.01;
        world.resource_mut::<SimulationTime>().tick = VERMIN_DRAIN_WINDOW_TICKS;

        world.run_system_once(vermin_food_drain_system).unwrap();

        assert_eq!(world.resource::<ColonyResources>().food, 0.0);
    }

    #[test]
    fn test_weave_accumulates_silk() {
        let mut world = setup();
        let spider = spawn_vermin_at(&mut world, VerminSpecies::SilkWeaverSpider, 5, 5);
        world.resource_mut::<SimulationTime>().tick = VERMIN_WEAVE_WINDOW_TICKS;

        world.run_system_once(vermin_weave_system).unwrap();

        let ready = world.get::<Vermin>(spider).unwrap().silk_ready;
        assert!(
            (ready - SPIDER_WEAVE_RATE).abs() < 1e-5,
            "spiders weave silk each window, got {ready}"
        );
    }

    #[test]
    fn test_harvest_silk_banks_and_resets() {
        let mut world = setup();
        let spider = spawn_vermin_at(&mut world, VerminSpecies::SilkWeaverSpider, 5, 5);
        world.get_mut::<Vermin>(spider).unwrap().silk_ready = 0.6;

        let banked = harvest_silk(&mut world);
        assert!((banked - 0.6).abs() < 1e-5);
        assert!((banked_silk(&mut world) - 0.6).abs() < 1e-5);
        assert_eq!(
            world.get::<Vermin>(spider).unwrap().silk_ready,
            0.0,
            "harvest resets the woven silk"
        );
    }

    #[test]
    fn test_exterminate_rats_yields_meat() {
        let mut world = setup();
        spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 5, 5);
        spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 5, 6);
        spawn_vermin_at(&mut world, VerminSpecies::SilkWeaverSpider, 5, 7);
        let food_before = world.resource::<ColonyResources>().food;

        let report = exterminate_vermin(&mut world, Some(VerminSpecies::GrainRat));

        assert_eq!(report.rats, 2);
        assert_eq!(report.spiders, 0);
        assert!(
            (report.food_gained - 2.0 * RAT_MEAT_YIELD).abs() < 1e-5,
            "culled rats are dressed for meat"
        );
        let food_after = world.resource::<ColonyResources>().food;
        assert!((food_after - food_before - report.food_gained).abs() < 1e-5);
        let (rats, _, spiders) = vermin_counts(&mut world);
        assert_eq!(rats, 0, "all rats culled");
        assert_eq!(spiders, 1, "spider left alone");
    }

    #[test]
    fn test_exterminate_spiders_banks_silk() {
        let mut world = setup();
        let spider = spawn_vermin_at(&mut world, VerminSpecies::SilkWeaverSpider, 5, 5);
        world.get_mut::<Vermin>(spider).unwrap().silk_ready = 0.7;

        let report = exterminate_vermin(&mut world, Some(VerminSpecies::SilkWeaverSpider));

        assert_eq!(report.spiders, 1);
        assert!((report.silk_gained - 0.7).abs() < 1e-5);
        assert!((banked_silk(&mut world) - 0.7).abs() < 1e-5);
        assert_eq!(vermin_total(&mut world), 0);
    }

    #[test]
    fn test_exterminate_all() {
        let mut world = setup();
        spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 5, 5);
        spawn_vermin_at(&mut world, VerminSpecies::BrainBorerBeetle, 5, 6);
        spawn_vermin_at(&mut world, VerminSpecies::SilkWeaverSpider, 5, 7);

        let report = exterminate_vermin(&mut world, None);

        assert_eq!(report.rats + report.beetles + report.spiders, 3);
        assert_eq!(vermin_total(&mut world), 0);
    }

    #[test]
    fn test_vermin_fever_application() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 5, 5);

        apply_vermin_fever(&mut world, pop);

        let morale = world.get::<Morale>(pop).unwrap();
        assert!(
            morale
                .modifiers
                .iter()
                .any(|m| m.label == VERMIN_FEVER_LABEL),
            "pop must gain the Vermin Fever mood modifier"
        );
        assert_eq!(world.get::<Health>(pop).unwrap().current, 99.0);

        // Idempotent: no duplicate modifier.
        apply_vermin_fever(&mut world, pop);
        let morale = world.get::<Morale>(pop).unwrap();
        assert_eq!(
            morale
                .modifiers
                .iter()
                .filter(|m| m.label == VERMIN_FEVER_LABEL)
                .count(),
            1,
            "fever modifier must not stack"
        );
    }

    #[test]
    fn test_disease_system_needs_vermin_nearby() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 5, 5);
        // Vermin far away — no infection possible.
        spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 50, 50);
        world.resource_mut::<SimulationTime>().tick = DISEASE_WINDOW_TICKS;

        vermin_disease_system(&mut world);

        let morale = world.get::<Morale>(pop).unwrap();
        assert!(
            !morale
                .modifiers
                .iter()
                .any(|m| m.label == VERMIN_FEVER_LABEL),
            "distant vermin must not infect"
        );
    }

    #[test]
    fn test_vermin_are_not_pops() {
        let mut world = setup();
        spawn_vermin_at(&mut world, VerminSpecies::GrainRat, 5, 5);
        spawn_pop(&mut world, 5, 6);

        let vermin_with_pop = world
            .query_filtered::<Entity, (With<Vermin>, With<Pop>)>()
            .iter(&world)
            .count();
        assert_eq!(
            vermin_with_pop, 0,
            "no vermin entity may carry Pop — utility AI can never pick one up"
        );
    }

    #[test]
    fn test_species_parse() {
        assert_eq!(VerminSpecies::parse("rat"), Some(VerminSpecies::GrainRat));
        assert_eq!(VerminSpecies::parse("RATS"), Some(VerminSpecies::GrainRat));
        assert_eq!(
            VerminSpecies::parse("brain borer"),
            Some(VerminSpecies::BrainBorerBeetle)
        );
        assert_eq!(
            VerminSpecies::parse("silk-weaver"),
            Some(VerminSpecies::SilkWeaverSpider)
        );
        assert_eq!(VerminSpecies::parse("dragon"), None);
    }

    #[test]
    fn test_toggle_pest_proof() {
        let mut world = setup();
        spawn_stockpile(&mut world, 10, 10);

        assert_eq!(toggle_pest_proof(&mut world, 12, 12), Some(true));
        assert_eq!(toggle_pest_proof(&mut world, 12, 12), Some(false));
        assert_eq!(
            toggle_pest_proof(&mut world, 10, 10),
            Some(true),
            "toggle works repeatedly"
        );
    }

    #[test]
    fn test_toggle_pest_proof_no_stockpile() {
        let mut world = setup();
        assert_eq!(toggle_pest_proof(&mut world, 10, 10), None);
    }

    #[test]
    fn test_ip_guard_no_lifted_names() {
        // Concept-only rule: public vermin strings must not contain distinctive
        // IP (there is no real IP here — this is the generic guard).
        let banned = [
            "xenomorph", "facehugger", "skaven", "tyranid", "zerg", "ratling", "wererat",
            "gloomspite", "nurgle",
        ];
        let public_text: Vec<String> = VerminSpecies::all()
            .iter()
            .flat_map(|s| [s.label().to_string(), s.short().to_string()])
            .chain(std::iter::once(VERMIN_FEVER_LABEL.to_string()))
            .collect();
        for text in &public_text {
            let lower = text.to_ascii_lowercase();
            for banned_term in banned {
                assert!(
                    !lower.contains(banned_term),
                    "public string '{text}' contains banned term '{banned_term}'"
                );
            }
        }
    }
}
