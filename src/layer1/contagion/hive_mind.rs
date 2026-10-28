//! Spec 371: The Hive Mind Resurgence.
//!
//! A dormant hive-mind spore infects pops. Latent infections ripen to
//! Active, Active to Critical. Active/Critical pops gain
//! `Trait::HiveCoordination`, which the real work-speed consumer
//! (`psychology::traits::get_trait_work_speed_modifier`) turns into a
//! +25% efficiency buff — the infection is subtly beneficial, at
//! first. Infected pops spread the spore by proximity
//! (`GridPosition`, Chebyshev). Critical pops quietly sabotage nearby
//! defenses (turret attack power, and turret `Health` where present).
//! At critical mass (>= 10 Critical) a `HiveMindRebellionEvent`
//! fires once; its handler transforms the Critical pops into a
//! coordinated hostile sub-faction by reusing the existing seams —
//! `FactionId::HiveMind` + `FactionMember` + `Factions`
//! (`FactionState::Striking`, the same hostile stance spec 1022's
//! split uses) and spec 1022's `IntegratedCollective` marker, so the
//! existing `trigger_hive_mind_faction_split_system` recognises them.
//! A Major chronicle entry records the rebellion.
//!
//! Grounding / deviations from the mock-style spec:
//! * Spec 1022 already landed `layer1::hive_mind_integration` with
//!   `IntegratedCollective`, a surgery-driven integration path, and
//!   `trigger_hive_mind_faction_split_system`. That is a voluntary /
//!   surgical collective, not a contagion: it has no spore, stages,
//!   spread, sabotage, or rebellion event. This module builds only
//!   those missing legs and reuses 1022's faction/collective seams
//!   for the transformation — no parallel faction system.
//! * `Trait` lives in `layer1::psychology::traits`, not
//!   `layer1::traits`; `Pop` in `layer1::entities::pop`, not
//!   `layer1::pop` (both are re-exported at `layer1` level).
//! * The mind-spore precedent is `MindSporeInfection` +
//!   `SymbiontFaction` in `layer1::biology::symbiotic_insurgency`
//!   (a flat active flag + faction counter, no stages); the staged
//!   spore here is new, as the spec requires.
//! * Sabotage reuses the real defense component
//!   `architecture::turret::Turret` (its `AttackProperties.damage`)
//!   instead of inventing a defense-health type; turret entities
//!   that also carry `Health` lose health too.

use bevy_ecs::prelude::*;

use crate::layer1::architecture::turret::Turret;
use crate::layer1::biology::health::Health;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::entities::pop::Pop;
use crate::layer1::hive_mind_integration::IntegratedCollective;
use crate::layer1::map::GridPosition;
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::social::factions::{FactionData, FactionId, FactionMember, FactionState, Factions};

/// Critical-stage infections required to trigger the rebellion.
pub const CRITICAL_MASS: usize = 10;
/// Chebyshev range (tiles) over which the spore spreads.
pub const SPREAD_RANGE: u32 = 2;
/// Cooldown (ticks/runs) a stage must age before ripening.
pub const STAGE_COOLDOWN: f32 = 100.0;
/// Chebyshev range within which Critical pops sabotage defenses.
pub const SABOTAGE_RANGE: u32 = 3;
/// Turret attack damage shaved per sabotage tick.
pub const SABOTAGE_TURRET_DAMAGE: f32 = 0.25;
/// Health damage per sabotage tick to a turret that carries Health.
pub const SABOTAGE_HEALTH_DAMAGE: f32 = 1.0;
/// Work-speed bonus granted by `Trait::HiveCoordination`.
pub const HIVE_COORDINATION_WORK_BONUS: f32 = 0.25;

/// Stage of a hive-spore infection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InfectionStage {
    /// Dormant: ripens to Active, spreads quietly.
    Latent,
    /// Coordinated: gains the efficiency trait, spreads.
    Active,
    /// Terminal: sabotages defenses; counts toward critical mass.
    Critical,
}

impl InfectionStage {
    /// Human-readable label.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Latent => "Latent",
            Self::Active => "Active",
            Self::Critical => "Critical",
        }
    }
}

/// A dormant hive-mind spore riding a pop (Spec 371).
#[derive(Component, Debug, Clone)]
pub struct HiveSpore {
    /// Current infection stage.
    pub stage: InfectionStage,
    /// Ticks until the stage ripens (also the spec's spread field).
    pub spread_cooldown: f32,
}

impl HiveSpore {
    /// A fresh latent infection.
    #[must_use]
    pub fn latent() -> Self {
        Self { stage: InfectionStage::Latent, spread_cooldown: STAGE_COOLDOWN }
    }
}

/// Fired once when Critical infections reach critical mass.
#[derive(Event, Debug, Clone)]
pub struct HiveMindRebellionEvent;

/// Ledger for the resurgence (prevents rebellion spam).
#[derive(Resource, Default, Debug)]
pub struct HiveMindState {
    /// Whether the (single) rebellion has already fired.
    pub rebellion_fired: bool,
    /// Total rebellions handled.
    pub rebellions: u32,
    /// Total sabotage ticks applied to defenses.
    pub sabotage_ticks: u32,
}

/// Pure helper: efficiency multiplier from hive coordination.
/// Wired into `get_trait_work_speed_modifier`.
#[must_use]
pub fn hive_efficiency_multiplier(traits: &Traits) -> f32 {
    if traits.has(Trait::HiveCoordination) {
        1.0 + HIVE_COORDINATION_WORK_BONUS
    } else {
        1.0
    }
}

/// Progression + trait grant. Runs in `Layer1SystemSet::Observation`.
pub fn process_hive_infection_system(
    mut query: Query<(&mut HiveSpore, Option<&mut Traits>), With<Pop>>,
) {
    for (mut spore, mut traits) in query.iter_mut() {
        if spore.spread_cooldown > 0.0 {
            spore.spread_cooldown = (spore.spread_cooldown - 1.0).max(0.0);
        } else {
            match spore.stage {
                InfectionStage::Latent => {
                    spore.stage = InfectionStage::Active;
                    spore.spread_cooldown = STAGE_COOLDOWN;
                }
                InfectionStage::Active => {
                    spore.stage = InfectionStage::Critical;
                    spore.spread_cooldown = STAGE_COOLDOWN;
                }
                InfectionStage::Critical => {}
            }
        }
        if let Some(traits) = traits.as_deref_mut() {
            if matches!(spore.stage, InfectionStage::Active | InfectionStage::Critical)
                && !traits.has(Trait::HiveCoordination)
            {
                traits.add(Trait::HiveCoordination);
            }
        }
    }
}

/// Proximity spread: any infected pop infects uninfected pops
/// within `SPREAD_RANGE` (Chebyshev) as Latent.
#[allow(clippy::type_complexity)]
pub fn spread_hive_infection_system(
    mut commands: Commands,
    sources: Query<(&HiveSpore, &GridPosition), With<Pop>>,
    targets: Query<(Entity, &GridPosition), (With<Pop>, Without<HiveSpore>)>,
) {
    let source_positions: Vec<GridPosition> = sources.iter().map(|(_, p)| *p).collect();
    if source_positions.is_empty() { return; }
    for (entity, pos) in targets.iter() {
        if source_positions.iter().any(|s| s.distance_chebyshev(*pos) <= SPREAD_RANGE) {
            commands.entity(entity).insert(HiveSpore::latent());
        }
    }
}

/// Critical pops sabotage nearby defenses: shave turret attack
/// damage (floor 0) and damage turret Health where present.
pub fn hive_sabotage_system(
    critical: Query<(&HiveSpore, &GridPosition), With<Pop>>,
    mut turrets: Query<(&mut Turret, &GridPosition, Option<&mut Health>)>,
    state: Option<ResMut<HiveMindState>>,
) {
    let positions: Vec<GridPosition> = critical.iter()
        .filter(|(s, _)| s.stage == InfectionStage::Critical)
        .map(|(_, p)| *p).collect();
    if positions.is_empty() { return; }
    let mut hits = 0u32;
    for (mut turret, pos, health) in turrets.iter_mut() {
        if positions.iter().any(|p| p.distance_chebyshev(*pos) <= SABOTAGE_RANGE) {
            turret.attack.damage = (turret.attack.damage - SABOTAGE_TURRET_DAMAGE).max(0.0);
            if let Some(mut h) = health { h.current = (h.current - SABOTAGE_HEALTH_DAMAGE).max(0.0); }
            hits += 1;
        }
    }
    if hits > 0 {
        if let Some(mut s) = state { s.sabotage_ticks += hits; }
    }
}

/// Fire the rebellion event at critical mass (once, via state).
pub fn trigger_hive_rebellion_system(
    query: Query<&HiveSpore, With<Pop>>,
    mut events: EventWriter<HiveMindRebellionEvent>,
    state: Option<Res<HiveMindState>>,
) {
    if let Some(s) = state.as_ref() { if s.rebellion_fired { return; } }
    let critical = query.iter().filter(|s| s.stage == InfectionStage::Critical).count();
    if critical >= CRITICAL_MASS {
        events.send(HiveMindRebellionEvent);
    }
}

/// Rebellion transformation: Critical pops become the HiveMind
/// sub-faction (existing FactionId/FactionMember/Factions seams +
/// spec 1022's IntegratedCollective), with a Major chronicle entry.
pub fn handle_hive_rebellion_system(
    mut events: EventReader<HiveMindRebellionEvent>,
    mut commands: Commands,
    mut query: Query<(Entity, &HiveSpore, Option<&mut FactionMember>), With<Pop>>,
    factions: Option<ResMut<Factions>>,
    state: Option<ResMut<HiveMindState>>,
    chronicle_events: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    let mut fired = false;
    for _ in events.read() { fired = true; }
    if !fired { return; }
    // Idempotent: a re-delivered / re-read event after the rebellion
    // has already fired must not transform or count twice.
    if let Some(s) = state.as_ref() {
        if s.rebellion_fired { return; }
    }

    let mut transformed = 0usize;
    for (entity, spore, member) in query.iter_mut() {
        if spore.stage != InfectionStage::Critical { continue; }
        commands.entity(entity).insert(IntegratedCollective);
        if let Some(mut m) = member { m.faction_id = Some(FactionId::HiveMind); }
        transformed += 1;
    }
    if let Some(mut factions) = factions {
        factions.map.entry(FactionId::HiveMind).or_insert_with(|| FactionData {
            name: "The Collective".to_string(),
            satisfaction: 0.0,
            members_count: transformed,
            state: FactionState::Striking,
            active_demand: None,
        });
        if let Some(data) = factions.map.get_mut(&FactionId::HiveMind) {
            data.members_count = transformed;
            data.state = FactionState::Striking;
        }
    }
    if let Some(mut s) = state { s.rebellion_fired = true; s.rebellions += 1; }
    if let Some(mut ev) = chronicle_events {
        ev.send(AddChronicleEvent {
            text: format!("THE HIVE AWAKENS: {transformed} colonists move as one and turn on the colony. The resurgence has begun."),
            importance: EventImportance::Major,
        });
    }
}

/// Census: (latent, active, critical).
#[must_use]
pub fn hive_counts(world: &mut World) -> (usize, usize, usize) {
    let mut q = world.query::<&HiveSpore>();
    let (mut l, mut a, mut c) = (0usize, 0usize, 0usize);
    for s in q.iter(world) {
        match s.stage { InfectionStage::Latent => l += 1, InfectionStage::Active => a += 1, InfectionStage::Critical => c += 1 }
    }
    (l, a, c)
}

/// STATS label: `<latent>/<active>/<critical>`.
#[must_use]
pub fn hive_stats_label(world: &mut World) -> String {
    let (l, a, c) = hive_counts(world);
    format!("{l}/{a}/{c}")
}

/// Headless debug: infect up to `n` uninfected pops (Latent).
/// Returns how many were infected.
pub fn infect_pops(world: &mut World, n: usize) -> usize {
    let mut q = world.query_filtered::<Entity, (With<Pop>, Without<HiveSpore>)>();
    let targets: Vec<Entity> = q.iter(world).take(n).collect();
    let count = targets.len();
    for e in targets { world.entity_mut(e).insert(HiveSpore::latent()); }
    count
}

/// Public-facing strings (IP-guard test scans these).
#[must_use]
pub fn hive_public_strings() -> Vec<String> {
    vec![
        "The Hive Mind Resurgence".to_string(),
        "THE HIVE AWAKENS".to_string(),
        "hive coordination".to_string(),
    ]
}

#[cfg(test)]
#[path = "hive_mind_tests.rs"]
mod tests;
