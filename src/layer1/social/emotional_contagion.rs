//! # Emotional Contagion (Spec 346)
//!
//! Panic spreads like wildfire. Laughter is infectious. The mood of the
//! crowd overpowers the individual: extreme emotions — Terror, Joy, Rage —
//! radiate from the pops feeling them, and nearby pops receive a temporary
//! [`MoodModifier`] on their real [`Morale`] component.
//!
//! ## Grounding (this is a rewrite of two earlier sketches)
//!
//! * The spec's RED tests sketched `Needs`/`Position` and mutating leisure
//!   directly. The live sim has a real morale system instead: [`Morale`]
//!   (cached 0.0–1.0 value) + [`MoodModifier`] (label/value/duration,
//!   summed by `update_morale_cache_system`, decayed by
//!   `morale_decay_system`). Contagion writes *modifiers*, never needs.
//! * An earlier draft of this module keyed off Bevy `Transform` positions
//!   with 0–100 morale strengths (±15.0). Real pops carry [`GridPosition`]
//!   and morale lives on the 0.0–1.0 scale, so that draft could never fire
//!   in the live sim. Everything here is `GridPosition` (Chebyshev) and
//!   0.0–1.0-scale strengths.
//!
//! ## Composition with Panic Spirals (Spec 1371)
//!
//! Panic is a *state* (flee, drop items, no work) owned by
//! `panic_spirals`; contagion is a *mood aura*. The two compose, they do
//! not duplicate: a pop carrying [`Panic`] radiates Terror (see
//! [`trigger_emotional_contagion_system`]), and Terror contagion only ever
//! writes a mood modifier — it never inserts `Panic`. Whether a terrified
//! bystander actually panics stays the panic system's call.
//!
//! ## Crowd overpower & anti-ping-pong
//!
//! A target in range of several sources feels the *crowd*, not a stack of
//! independent whispers: per emotion, the strongest source sets the base
//! strength and each additional source amplifies it by
//! [`CROWD_BONUS_PER_EXTRA_SOURCE`] (capped), then the single dominant
//! emotion (largest absolute value; ties break toward Terror, then Rage —
//! fear shouts louder than joy) is applied as ONE modifier. A pop carries
//! at most one contagion modifier at a time; re-exposure refreshes its
//! value/duration instead of stacking, and when sources leave, the
//! modifier decays through the ordinary morale decay system.
//!
//! ## Spatial lookup
//!
//! Neighbor lookup uses the spatial-hash pattern from the Spec 343 rumor
//! web (`layer1/social/rumor.rs`): sources are bucketed by tile into a
//! `HashMap`, and each target only probes the cells inside the widest
//! source radius — never an O(N²) all-pairs scan.

use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::layer1::map::GridPosition;
use crate::layer1::psychology::panic_spirals::Panic;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::stress::StressTracker;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Chebyshev radius (tiles) an extreme emotion radiates.
pub const CONTAGION_RADIUS: i32 = 3;
/// Ticks a contagion mood modifier lasts before ordinary morale decay
/// removes it (refreshed while the target stays in the crowd).
pub const CONTAGION_DURATION_TICKS: u32 = 50;
/// Cached morale at or below this is Terror (spec GREEN: morale < 0.2).
pub const TERROR_MORALE_THRESHOLD: f32 = 0.2;
/// Cached morale at or above this is Joy (spec GREEN: morale > 0.8).
pub const JOY_MORALE_THRESHOLD: f32 = 0.8;
/// Accumulated stress at or above this turns a pop into a Rage source
/// (same 50.0 line where stressed pops start fabricating rumors, Spec 343;
/// breakdown itself is at 100.0).
pub const RAGE_STRESS_THRESHOLD: f32 = 50.0;
/// Each source past the first amplifies its emotion's crowd strength by
/// this fraction — the crowd overpowers the individual.
pub const CROWD_BONUS_PER_EXTRA_SOURCE: f32 = 0.25;
/// Crowd amplification never exceeds this multiplier.
pub const CROWD_BONUS_CAP: f32 = 2.0;
/// Absolute cap on any single contagion modifier (real morale scale).
pub const CONTAGION_VALUE_CAP: f32 = 0.30;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// An extreme emotion radiating from a pop (Spec 346).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContagionType {
    /// Terror: rock-bottom morale, or a pop in outright Panic (Spec 1371).
    Terror,
    /// Joy: soaring morale.
    Joy,
    /// Rage: stress past the rumor-fabrication line, short of breakdown.
    Rage,
}

impl ContagionType {
    /// Mood-modifier label written onto affected pops.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Terror => "Contagion: Terror",
            Self::Joy => "Contagion: Joy",
            Self::Rage => "Contagion: Rage",
        }
    }

    /// Base modifier strength on the real 0.0–1.0 morale scale.
    #[must_use]
    pub const fn base_strength(&self) -> f32 {
        match self {
            Self::Terror => -0.15,
            Self::Joy => 0.12,
            Self::Rage => -0.10,
        }
    }

    /// Tie-break priority when two crowds are equally loud: fear first.
    const fn priority(&self) -> u8 {
        match self {
            Self::Terror => 2,
            Self::Rage => 1,
            Self::Joy => 0,
        }
    }
}

/// Component marking a pop as a contagion source: it is feeling
/// `contagion_type` strongly enough to radiate it `radius` tiles.
#[derive(Component, Debug, Clone, Copy)]
pub struct EmotionalContagion {
    /// Which extreme emotion is radiating.
    pub contagion_type: ContagionType,
    /// Chebyshev radius in tiles.
    pub radius: i32,
    /// Base modifier strength (0.0–1.0 morale scale).
    pub strength: f32,
}

impl EmotionalContagion {
    /// A source with the default radius and the type's base strength.
    #[must_use]
    pub const fn new(contagion_type: ContagionType) -> Self {
        Self {
            contagion_type,
            radius: CONTAGION_RADIUS,
            strength: contagion_type.base_strength(),
        }
    }
}

/// Ledger of contagion activity for the headless dashboard (Spec 346),
/// mirroring the Spec 343 `RumorWebLedger` pattern.
#[derive(Resource, Default, Debug)]
pub struct ContagionLedger {
    /// Total first-time modifier applications (refreshes don't count).
    pub total_spreads: u64,
    /// Pops carrying a contagion modifier after the most recent run.
    pub last_affected: usize,
    /// High-water mark of simultaneously affected pops.
    pub peak_affected: usize,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Trigger: decide which pops are radiating an extreme emotion.
///
/// * [`Panic`] (Spec 1371) or cached morale ≤ [`TERROR_MORALE_THRESHOLD`]
///   → Terror. (Panic composes here; this system never inserts `Panic`.)
/// * Otherwise, stress ≥ [`RAGE_STRESS_THRESHOLD`] → Rage.
/// * Otherwise, cached morale ≥ [`JOY_MORALE_THRESHOLD`] → Joy.
/// * Otherwise the pop radiates nothing and any stale source is removed.
///
/// Pops already radiating the right emotion are left untouched (no
/// insert churn); a changed emotion swaps the component.
#[allow(clippy::type_complexity)]
pub fn trigger_emotional_contagion_system(
    mut commands: Commands,
    query: Query<(
        Entity,
        &Morale,
        Option<&StressTracker>,
        Option<&Panic>,
        Option<&EmotionalContagion>,
    )>,
) {
    for (entity, morale, stress, panic, existing) in &query {
        let desired = if panic.is_some() || morale.value <= TERROR_MORALE_THRESHOLD {
            Some(ContagionType::Terror)
        } else if stress.is_some_and(|s| s.accumulated_stress >= RAGE_STRESS_THRESHOLD) {
            Some(ContagionType::Rage)
        } else if morale.value >= JOY_MORALE_THRESHOLD {
            Some(ContagionType::Joy)
        } else {
            None
        };

        match desired {
            Some(kind) => {
                if existing.is_none_or(|c| c.contagion_type != kind) {
                    commands.entity(entity).insert(EmotionalContagion::new(kind));
                }
            }
            None => {
                if existing.is_some() {
                    commands.entity(entity).remove::<EmotionalContagion>();
                }
            }
        }
    }
}

/// Spread: nearby pops receive the crowd's dominant mood as a temporary
/// [`MoodModifier`] on their [`Morale`].
///
/// Sources are bucketed into a spatial hash by tile (Spec 343 pattern);
/// each target probes only the cells within the widest source radius.
/// Sources never receive contagion while radiating. A target carries at
/// most one contagion modifier — the dominant crowd emotion — refreshed,
/// never stacked.
pub fn contagion_system(
    sources: Query<(Entity, &GridPosition, &EmotionalContagion)>,
    mut targets: Query<(Entity, &GridPosition, &mut Morale), Without<EmotionalContagion>>,
    mut ledger: Option<ResMut<ContagionLedger>>,
) {
    // Snapshot phase: source tiles, types, radii, strengths.
    struct SourceSnapshot {
        x: i32,
        y: i32,
        kind: ContagionType,
        radius: i32,
        strength: f32,
    }
    let mut snapshots: Vec<SourceSnapshot> = Vec::new();
    let mut max_radius: i32 = 0;
    for (_entity, pos, contagion) in &sources {
        max_radius = max_radius.max(contagion.radius);
        snapshots.push(SourceSnapshot {
            x: pos.x,
            y: pos.y,
            kind: contagion.contagion_type,
            radius: contagion.radius,
            strength: contagion.strength,
        });
    }
    if snapshots.is_empty() {
        if let Some(ledger) = ledger.as_mut() {
            ledger.last_affected = 0;
        }
        return;
    }

    // Spatial hash grid: cell -> snapshot indices (Spec 343 pattern).
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (idx, snap) in snapshots.iter().enumerate() {
        grid.entry((snap.x, snap.y)).or_default().push(idx);
    }

    let all_labels = [
        ContagionType::Terror.label(),
        ContagionType::Joy.label(),
        ContagionType::Rage.label(),
    ];

    let mut new_spreads: u64 = 0;
    let mut affected: usize = 0;

    for (_entity, pos, mut morale) in &mut targets {
        // Accumulate the crowd per emotion: count + strongest base.
        let mut crowd: HashMap<ContagionType, (usize, f32)> = HashMap::new();
        for dx in -max_radius..=max_radius {
            for dy in -max_radius..=max_radius {
                let Some(cell) = grid.get(&(pos.x + dx, pos.y + dy)) else {
                    continue;
                };
                for &idx in cell {
                    let snap = &snapshots[idx];
                    let chebyshev = (pos.x - snap.x).abs().max((pos.y - snap.y).abs());
                    if chebyshev == 0 || chebyshev > snap.radius {
                        continue;
                    }
                    let entry = crowd.entry(snap.kind).or_insert((0, snap.strength));
                    entry.0 += 1;
                    if snap.strength.abs() > entry.1.abs() {
                        entry.1 = snap.strength;
                    }
                }
            }
        }
        if crowd.is_empty() {
            continue;
        }

        // The crowd overpowers the individual: one dominant emotion,
        // amplified by crowd size, capped.
        let mut best: Option<(ContagionType, f32)> = None;
        for (kind, (count, strongest)) in &crowd {
            let bonus = (1.0 + CROWD_BONUS_PER_EXTRA_SOURCE * (*count as f32 - 1.0))
                .min(CROWD_BONUS_CAP);
            let value = (strongest * bonus).clamp(-CONTAGION_VALUE_CAP, CONTAGION_VALUE_CAP);
            let wins = match best {
                None => true,
                Some((best_kind, best_value)) => {
                    value.abs() > best_value.abs() + f32::EPSILON
                        || ((value.abs() - best_value.abs()).abs() <= f32::EPSILON
                            && kind.priority() > best_kind.priority())
                }
            };
            if wins {
                best = Some((*kind, value));
            }
        }
        let Some((kind, value)) = best else {
            continue;
        };
        let label = kind.label();

        // One contagion mood at a time: drop other contagion labels, then
        // refresh (never stack) the dominant one.
        let had = morale.modifiers.iter().any(|m| m.label == label);
        morale
            .modifiers
            .retain(|m| m.label == label || !all_labels.contains(&m.label.as_str()));
        if had {
            if let Some(existing) = morale.modifiers.iter_mut().find(|m| m.label == label) {
                existing.value = value;
                existing.duration = CONTAGION_DURATION_TICKS;
            }
        } else {
            morale.modifiers.push(MoodModifier {
                label: label.to_string(),
                value,
                duration: CONTAGION_DURATION_TICKS,
            });
            new_spreads += 1;
        }
        affected += 1;
    }

    if let Some(ledger) = ledger.as_mut() {
        ledger.total_spreads += new_spreads;
        ledger.last_affected = affected;
        ledger.peak_affected = ledger.peak_affected.max(affected);
    }
}

// ---------------------------------------------------------------------------
// Headless helpers
// ---------------------------------------------------------------------------

/// Pops currently carrying a contagion-sourced mood modifier (STATS).
#[must_use]
pub fn contagion_affected_count(world: &mut World) -> usize {
    let mut query = world.query::<&Morale>();
    query
        .iter(world)
        .filter(|m| {
            m.modifiers.iter().any(|modifier| {
                modifier.label == ContagionType::Terror.label()
                    || modifier.label == ContagionType::Joy.label()
                    || modifier.label == ContagionType::Rage.label()
            })
        })
        .count()
}

/// Live contagion sources by emotion: (terror, joy, rage).
#[must_use]
pub fn contagion_source_counts(world: &mut World) -> (usize, usize, usize) {
    let mut query = world.query::<&EmotionalContagion>();
    let (mut terror, mut joy, mut rage) = (0, 0, 0);
    for source in query.iter(world) {
        match source.contagion_type {
            ContagionType::Terror => terror += 1,
            ContagionType::Joy => joy += 1,
            ContagionType::Rage => rage += 1,
        }
    }
    (terror, joy, rage)
}

// ---------------------------------------------------------------------------
// Tests (Spec 346 RED tests, adapted from Needs/Position sketches to the
// real Morale / MoodModifier / GridPosition types)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::pop::Pop;

    fn run_spread(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(contagion_system);
        schedule.run(world);
    }

    fn run_trigger(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(trigger_emotional_contagion_system);
        schedule.run(world);
    }

    fn morale_with(value: f32) -> Morale {
        Morale {
            value,
            modifiers: Vec::new(),
        }
    }

    fn contagion_modifier(morale: &Morale) -> Option<&MoodModifier> {
        morale
            .modifiers
            .iter()
            .find(|m| m.label.starts_with("Contagion:"))
    }

    #[test]
    fn test_emotional_contagion_spreads_joy() {
        let mut world = World::new();
        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            EmotionalContagion::new(ContagionType::Joy),
        ));
        let neutral = world
            .spawn((Pop, GridPosition { x: 1, y: 0 }, morale_with(0.5)))
            .id();

        run_spread(&mut world);

        let morale = world.get::<Morale>(neutral).unwrap();
        let modifier = contagion_modifier(morale).expect("nearby pop should catch Joy");
        assert_eq!(modifier.label, "Contagion: Joy");
        assert!(modifier.value > 0.0, "Joy modifier should be positive");
        assert_eq!(modifier.duration, CONTAGION_DURATION_TICKS);
    }

    #[test]
    fn test_emotional_contagion_spreads_terror() {
        let mut world = World::new();
        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            EmotionalContagion::new(ContagionType::Terror),
        ));
        let neutral = world
            .spawn((Pop, GridPosition { x: 1, y: 0 }, morale_with(0.8)))
            .id();

        run_spread(&mut world);

        let morale = world.get::<Morale>(neutral).unwrap();
        let modifier = contagion_modifier(morale).expect("nearby pop should catch Terror");
        assert_eq!(modifier.label, "Contagion: Terror");
        assert!(modifier.value < 0.0, "Terror modifier should be negative");
    }

    #[test]
    fn test_emotional_contagion_spreads_rage() {
        let mut world = World::new();
        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            EmotionalContagion::new(ContagionType::Rage),
        ));
        let neutral = world
            .spawn((Pop, GridPosition { x: 0, y: 2 }, morale_with(0.5)))
            .id();

        run_spread(&mut world);

        let morale = world.get::<Morale>(neutral).unwrap();
        let modifier = contagion_modifier(morale).expect("nearby pop should catch Rage");
        assert_eq!(modifier.label, "Contagion: Rage");
        assert!(modifier.value < 0.0, "Rage modifier should be negative");
    }

    #[test]
    fn test_emotional_contagion_range_limit() {
        let mut world = World::new();
        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            EmotionalContagion::new(ContagionType::Joy),
        ));
        let far = world
            .spawn((Pop, GridPosition { x: 10, y: 10 }, morale_with(0.5)))
            .id();

        run_spread(&mut world);

        let morale = world.get::<Morale>(far).unwrap();
        assert!(
            contagion_modifier(morale).is_none(),
            "Far pop should be unaffected"
        );
    }

    #[test]
    fn test_contagion_crowd_amplifies() {
        // Three joyful pops in a crowd move a bystander more than one does.
        let mut world = World::new();
        for x in -1..=1 {
            world.spawn((
                Pop,
                GridPosition { x, y: 1 },
                EmotionalContagion::new(ContagionType::Joy),
            ));
        }
        let bystander = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, morale_with(0.5)))
            .id();

        run_spread(&mut world);

        let morale = world.get::<Morale>(bystander).unwrap();
        let modifier = contagion_modifier(morale).unwrap();
        assert!(
            modifier.value > ContagionType::Joy.base_strength(),
            "crowd of 3 should amplify past the base strength, got {}",
            modifier.value
        );
        assert!(modifier.value <= CONTAGION_VALUE_CAP);
    }

    #[test]
    fn test_contagion_crowd_overpowers_individual() {
        // One joyful pop vs. a terror crowd: the crowd's mood wins.
        let mut world = World::new();
        world.spawn((
            Pop,
            GridPosition { x: 1, y: 0 },
            EmotionalContagion::new(ContagionType::Joy),
        ));
        for x in -1..=1 {
            world.spawn((
                Pop,
                GridPosition { x, y: -1 },
                EmotionalContagion::new(ContagionType::Terror),
            ));
        }
        let bystander = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, morale_with(0.5)))
            .id();

        run_spread(&mut world);

        let morale = world.get::<Morale>(bystander).unwrap();
        let modifier = contagion_modifier(morale).unwrap();
        assert_eq!(modifier.label, "Contagion: Terror");
        // Exactly one contagion modifier: no stacking whispers.
        assert_eq!(
            morale
                .modifiers
                .iter()
                .filter(|m| m.label.starts_with("Contagion:"))
                .count(),
            1
        );
    }

    #[test]
    fn test_contagion_refreshes_instead_of_stacking() {
        let mut world = World::new();
        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            EmotionalContagion::new(ContagionType::Joy),
        ));
        let neutral = world
            .spawn((Pop, GridPosition { x: 1, y: 0 }, morale_with(0.5)))
            .id();

        run_spread(&mut world);
        run_spread(&mut world);
        run_spread(&mut world);

        let morale = world.get::<Morale>(neutral).unwrap();
        assert_eq!(
            morale
                .modifiers
                .iter()
                .filter(|m| m.label == "Contagion: Joy")
                .count(),
            1,
            "repeated exposure must refresh, not stack"
        );
    }

    #[test]
    fn test_trigger_marks_extreme_morale() {
        let mut world = World::new();
        let terrified = world.spawn((Pop, morale_with(0.1))).id();
        let joyful = world.spawn((Pop, morale_with(0.95))).id();
        let neutral = world.spawn((Pop, morale_with(0.5))).id();

        run_trigger(&mut world);

        assert_eq!(
            world.get::<EmotionalContagion>(terrified).unwrap().contagion_type,
            ContagionType::Terror
        );
        assert_eq!(
            world.get::<EmotionalContagion>(joyful).unwrap().contagion_type,
            ContagionType::Joy
        );
        assert!(world.get::<EmotionalContagion>(neutral).is_none());
    }

    #[test]
    fn test_trigger_clears_when_morale_recovers() {
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                morale_with(0.5),
                EmotionalContagion::new(ContagionType::Terror),
            ))
            .id();

        run_trigger(&mut world);

        assert!(
            world.get::<EmotionalContagion>(pop).is_none(),
            "a pop back to neutral morale stops radiating"
        );
    }

    #[test]
    fn test_trigger_keeps_existing_same_type() {
        let mut world = World::new();
        let custom = EmotionalContagion {
            contagion_type: ContagionType::Terror,
            radius: 5,
            strength: -0.2,
        };
        let pop = world.spawn((Pop, morale_with(0.05), custom)).id();

        run_trigger(&mut world);

        let kept = world.get::<EmotionalContagion>(pop).unwrap();
        assert_eq!(kept.radius, 5, "same-type source must not be re-inserted");
        assert!((kept.strength - -0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn test_trigger_rage_from_stress() {
        let mut world = World::new();
        let stressed = world
            .spawn((
                Pop,
                morale_with(0.45),
                StressTracker {
                    accumulated_stress: RAGE_STRESS_THRESHOLD + 10.0,
                },
            ))
            .id();

        run_trigger(&mut world);

        assert_eq!(
            world.get::<EmotionalContagion>(stressed).unwrap().contagion_type,
            ContagionType::Rage
        );
    }

    #[test]
    fn test_trigger_panicked_pop_radiates_terror_not_panic() {
        // Composition with Spec 1371: a panicking pop is a Terror source,
        // and contagion spread must never insert Panic itself.
        let mut world = World::new();
        let panicking = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                morale_with(0.5),
                Panic::new(GridPosition { x: 0, y: 0 }),
            ))
            .id();
        let bystander = world
            .spawn((Pop, GridPosition { x: 1, y: 0 }, morale_with(0.6)))
            .id();

        run_trigger(&mut world);
        assert_eq!(
            world.get::<EmotionalContagion>(panicking).unwrap().contagion_type,
            ContagionType::Terror
        );

        run_spread(&mut world);
        let morale = world.get::<Morale>(bystander).unwrap();
        assert_eq!(contagion_modifier(morale).unwrap().label, "Contagion: Terror");
        assert!(
            world.get::<Panic>(bystander).is_none(),
            "contagion writes mood only; panic state belongs to Spec 1371"
        );
    }

    #[test]
    fn test_affected_count_helper() {
        let mut world = World::new();
        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            EmotionalContagion::new(ContagionType::Joy),
        ));
        world.spawn((Pop, GridPosition { x: 1, y: 0 }, morale_with(0.5)));
        world.spawn((Pop, GridPosition { x: 9, y: 9 }, morale_with(0.5)));

        run_spread(&mut world);

        assert_eq!(contagion_affected_count(&mut world), 1);
        assert_eq!(contagion_source_counts(&mut world), (0, 1, 0));
    }

    #[test]
    fn test_ip_guard_contagion_strings_are_original() {
        // Player-visible strings must be generic/original (IP rule):
        // no lifted franchise terms in labels.
        let banned = [
            "hitchhiker",
            "improbability drive",
            "shrike",
            "hyperion",
            "area x",
            "annihilation",
            "foundation",
            "scp",
            "warhammer",
            "grimdark",
        ];
        for kind in [
            ContagionType::Terror,
            ContagionType::Joy,
            ContagionType::Rage,
        ] {
            let label = kind.label().to_lowercase();
            for term in banned {
                assert!(!label.contains(term), "label {label} contains {term}");
            }
        }
    }
}
