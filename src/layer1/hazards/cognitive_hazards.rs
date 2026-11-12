//! Cognitive Hazards (Spec 416): the planet whispers, and the darkness
//! stares back.
//!
//! Grounding (audited 2026-11-12 against trunk f23805f5): the spec's
//! sketch types are MOCKS. The real seams this module bridges onto:
//!
//! * Positions are [`GridPosition`](crate::layer1::map::GridPosition)
//!   tiles (Chebyshev distance), not `Transform`/`Vec3` — pops do not
//!   carry a `Transform` in the tile sim.
//! * There is no `NotificationEvent` / `layer1::notifications::
//!   NotificationEvent`. The real alert surface is the
//!   [`NotificationQueue`](crate::layer1::notifications::NotificationQueue)
//!   resource (the `placebo` pattern: `Option<ResMut<NotificationQueue>>`),
//!   and the real narrative record is the Chronicle via
//!   [`AddChronicleEvent`] (the martyrdom / panic-spirals pattern).
//! * Corpses are the real `Corpse` component from
//!   `culture::funeral` (the same entities `grief_system` and Spec 1370
//!   compost read); darkness is the real
//!   [`LightMap`](crate::layer1::lighting::LightMap) /
//!   [`AmbientLight`](crate::layer1::lighting::AmbientLight) pair from
//!   the lighting system.
//! * Traits are the real `Traits` set: `Psychopath` ("Lacks empathy,
//!   unaffected by horror" — its own doc comment) is immune, `Courage`
//!   halves gain (the Spec 1371 panic precedent), `Sensitive` and
//!   `Traumatized` amplify it. No new traits were invented.
//!
//! What this module builds:
//!
//! 1. **Exposure.** [`HorrorSource`] is a generic dread locus; unburied
//!    corpses and deep darkness act as horror sources automatically.
//!    Nearby pops accumulate [`Insanity`] (0.0–100.0) with distance
//!    falloff; away from every source, insanity decays passively.
//! 2. **Hallucinations.** A pop above
//!    [`HALLUCINATION_THRESHOLD`] periodically makes the colony see
//!    things: a false raid alert, a false fire alert, or a phantom
//!    hostile — delivered as a *real* error notification (the player
//!    cannot tell it apart at a glance; that is the spec's emergence),
//!    a [`HallucinationEvent`] on the event bus, and a Minor chronicle
//!    entry naming the hallucinating pop. A colony-wide cooldown keeps
//!    it to one false alarm at a time.
//!
//! Hallucination cadence is deterministic (cooldown + kind rotation),
//! not RNG: the spec's 10%-per-check roll made its own RED test a coin
//! flip, and deterministic cadence is directly playtestable headless.

use bevy_ecs::prelude::*;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::culture::funeral::Corpse;
use crate::layer1::lighting::{AmbientLight, LightMap};
use crate::layer1::map::GridPosition;
use crate::layer1::notifications::{NotificationQueue, NotificationSeverity};
use crate::layer1::pop::Pop;
use crate::layer1::traits::{Trait, Traits};
use crate::shared::time::SimulationTime;

/// Highest possible insanity level.
pub const INSANITY_MAX: f32 = 100.0;
/// Insanity strictly above this level can trigger hallucinations
/// (spec §4: `level > 80.0`).
pub const HALLUCINATION_THRESHOLD: f32 = 80.0;
/// Per-tick gain scale applied to a summed exposure rate.
pub const EXPOSURE_SCALE: f32 = 0.05;
/// Insanity removed per tick when a pop is exposed to nothing.
pub const INSANITY_DECAY_PER_TICK: f32 = 0.05;
/// Horror intensity of an unburied corpse (spec §7: horror sources are
/// attached to corpses automatically).
pub const CORPSE_HORROR_INTENSITY: f32 = 2.0;
/// Horror radius (tiles, Chebyshev) of an unburied corpse — matches
/// the 5-tile witness radius `grief_system` already uses.
pub const CORPSE_HORROR_RADIUS: f32 = 5.0;
/// Exposure rate contributed by standing in deep darkness.
pub const DARKNESS_EXPOSURE_RATE: f32 = 1.0;
/// A tile at or below this light level counts as deep darkness.
pub const DARKNESS_LIGHT_THRESHOLD: f32 = 0.05;
/// Minimum ticks between colony hallucinations (one false alarm at a
/// time — the spec's "only trigger one hallucination per tick",
/// stretched over a playable cadence).
pub const HALLUCINATION_COOLDOWN_TICKS: u32 = 50;
/// Intensity of the debug horror locus placed by headless `horror`.
pub const DEBUG_HORROR_INTENSITY: f32 = 10.0;
/// Radius of the debug horror locus placed by headless `horror`.
pub const DEBUG_HORROR_RADIUS: f32 = 5.0;

/// Per-pop cognitive-hazard state (Spec 416). `level` runs 0.0 (lucid)
/// to 100.0 (lost). Every pop gains this component on its first tick
/// via [`ensure_insanity_system`].
#[derive(Component, Debug, Clone, Copy, Default, PartialEq)]
pub struct Insanity {
    /// Current insanity level, clamped to `0.0..=INSANITY_MAX`.
    pub level: f32,
}

impl Insanity {
    /// Whether this pop is over the hallucination threshold.
    #[must_use]
    pub fn is_hallucinating(&self) -> bool {
        self.level > HALLUCINATION_THRESHOLD
    }
}

/// A locus of dread: pops within `radius` tiles (Chebyshev) accumulate
/// insanity, scaled by `intensity` and distance falloff. Corpses and
/// darkness are handled implicitly by [`exposure_system`]; this
/// component marks explicit sources (a dread place, a wrong artifact,
/// a debug locus).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct HorrorSource {
    /// Exposure rate at the source's own tile.
    pub intensity: f32,
    /// Radius in tiles (Chebyshev).
    pub radius: f32,
}

impl Default for HorrorSource {
    fn default() -> Self {
        Self {
            intensity: DEBUG_HORROR_INTENSITY,
            radius: DEBUG_HORROR_RADIUS,
        }
    }
}

/// The three hallucination shapes, rotated deterministically so a
/// melting colony cycles through all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HallucinationKind {
    /// A raid that is not happening.
    FalseRaid,
    /// A fire that is not burning.
    FalseFire,
    /// A hostile that is not there.
    PhantomFoe,
}

impl HallucinationKind {
    /// Rotation by cumulative hallucination count.
    #[must_use]
    pub fn from_index(index: u32) -> Self {
        match index % 3 {
            0 => Self::FalseRaid,
            1 => Self::FalseFire,
            _ => Self::PhantomFoe,
        }
    }

    /// The alert text exactly as the player receives it — deliberately
    /// indistinguishable from a real alert. The *chronicle* is where
    /// the truth is recorded.
    #[must_use]
    pub fn message(&self) -> &'static str {
        match self {
            Self::FalseRaid => "Raid Detected! Raiders are approaching the colony!",
            Self::FalseFire => "Fire Detected! A blaze has broken out in the colony!",
            Self::PhantomFoe => {
                "Hostile Detected! Something hostile is moving inside the perimeter!"
            }
        }
    }

    /// Short label used in the chronicle entry and headless panel.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::FalseRaid => "false raid",
            Self::FalseFire => "false fire",
            Self::PhantomFoe => "phantom foe",
        }
    }
}

/// Fired when a pop's hallucination reaches the colony's alert
/// surface. The notification queue and the chronicle are written by
/// the same system; this event is the seam for UI / audio consumers.
#[derive(Event, Debug, Clone)]
pub struct HallucinationEvent {
    /// The pop who hallucinated.
    pub pop: Entity,
    /// Which hallucination fired.
    pub kind: HallucinationKind,
    /// The alert text the player received.
    pub message: String,
}

/// Colony ledger for cognitive hazards: cumulative hallucination
/// count and the colony-wide cooldown between false alarms.
#[derive(Resource, Default, Debug, Clone)]
pub struct CognitiveHazardState {
    /// Total hallucinations fired this run.
    pub hallucinations: u32,
    /// Ticks until the next hallucination may fire.
    pub cooldown_ticks: u32,
}

/// Trait multiplier on insanity gain (spec §5 REFACTOR: trait
/// interaction). `Psychopath` is immune by its own definition;
/// `Courage` halves; `Sensitive` / `Traumatized` amplify.
#[must_use]
pub fn insanity_gain_multiplier(traits: Option<&Traits>) -> f32 {
    let Some(traits) = traits else {
        return 1.0;
    };
    if traits.has(Trait::Psychopath) {
        return 0.0;
    }
    let mut mult = 1.0f32;
    if traits.has(Trait::Courage) {
        mult *= 0.5;
    }
    if traits.has(Trait::Sensitive) {
        mult *= 1.5;
    }
    if traits.has(Trait::Traumatized) {
        mult *= 1.5;
    }
    mult
}

/// Gives every pop an [`Insanity`] component (first-tick pattern: the
/// exposure system can then assume the component exists).
pub fn ensure_insanity_system(
    mut commands: Commands,
    query: Query<Entity, (With<Pop>, Without<Insanity>)>,
) {
    for entity in &query {
        commands.entity(entity).insert(Insanity::default());
    }
}

fn chebyshev(a: &GridPosition, b: &GridPosition) -> f32 {
    let dx = (a.x - b.x).abs();
    let dy = (a.y - b.y).abs();
    dx.max(dy) as f32
}

fn falloff(distance: f32, radius: f32, intensity: f32) -> f32 {
    if radius > 0.0 && distance < radius {
        (1.0 - distance / radius) * intensity
    } else if radius <= 0.0 && distance == 0.0 {
        intensity
    } else {
        0.0
    }
}

/// Exposure + decay (Spec 416 RED tests 1 & 3). Horror comes from
/// explicit [`HorrorSource`] entities, from every unburied [`Corpse`],
/// and from deep darkness on the pop's own tile (real `LightMap`,
/// falling back to `AmbientLight` when no map is present). With no
/// exposure, insanity decays by [`INSANITY_DECAY_PER_TICK`].
#[allow(clippy::type_complexity)]
pub fn exposure_system(
    mut pops: Query<(&GridPosition, &mut Insanity, Option<&Traits>), With<Pop>>,
    horrors: Query<(&GridPosition, &HorrorSource)>,
    corpses: Query<&GridPosition, With<Corpse>>,
    light_map: Option<Res<LightMap>>,
    ambient: Option<Res<AmbientLight>>,
) {
    // Collect source positions once (corpse count is small; the
    // HorrorSource set is smaller — O(pops * sources) is the spec's
    // own sketch and fine at colony scale).
    let horror_list: Vec<(GridPosition, f32, f32)> = horrors
        .iter()
        .map(|(pos, src)| (*pos, src.intensity, src.radius))
        .collect();
    let corpse_list: Vec<GridPosition> = corpses.iter().copied().collect();

    for (pos, mut insanity, traits) in &mut pops {
        let mut rate = 0.0f32;
        for (src_pos, intensity, radius) in &horror_list {
            rate += falloff(chebyshev(pos, src_pos), *radius, *intensity);
        }
        for corpse_pos in &corpse_list {
            rate += falloff(
                chebyshev(pos, corpse_pos),
                CORPSE_HORROR_RADIUS,
                CORPSE_HORROR_INTENSITY,
            );
        }
        // Deep darkness on the pop's own tile.
        let light = if let Some(map) = light_map.as_deref() {
            if pos.x >= 0 && pos.y >= 0 {
                map.get(pos.x as u32, pos.y as u32)
            } else {
                0.0
            }
        } else {
            ambient.as_deref().map_or(1.0, |a| a.level)
        };
        if light <= DARKNESS_LIGHT_THRESHOLD {
            rate += DARKNESS_EXPOSURE_RATE;
        }

        let gain = rate * EXPOSURE_SCALE * insanity_gain_multiplier(traits);
        if gain > 0.0 {
            insanity.level = (insanity.level + gain).min(INSANITY_MAX);
        } else {
            insanity.level = (insanity.level - INSANITY_DECAY_PER_TICK).max(0.0);
        }
    }
}

/// Hallucinations (Spec 416 RED test 2). When any pop is over the
/// threshold and the colony cooldown has elapsed, the first such pop
/// fires the next hallucination in rotation: a real error
/// notification, a [`HallucinationEvent`], and a Minor chronicle
/// entry recording the truth.
#[allow(clippy::type_complexity)]
pub fn hallucination_system(
    pops: Query<
        (Entity, &Insanity, Option<&crate::layer1::entities::pop::PopName>),
        With<Pop>,
    >,
    mut state: Option<ResMut<CognitiveHazardState>>,
    mut queue: Option<ResMut<NotificationQueue>>,
    mut hallucination_events: Option<ResMut<Events<HallucinationEvent>>>,
    mut chronicle_events: Option<ResMut<Events<AddChronicleEvent>>>,
    time: Option<Res<SimulationTime>>,
) {
    if let Some(state) = state.as_deref_mut() {
        if state.cooldown_ticks > 0 {
            state.cooldown_ticks -= 1;
            return;
        }
    }
    let Some((entity, _, name)) = pops.iter().find(|(_, insanity, _)| insanity.is_hallucinating())
    else {
        return;
    };
    let index = state.as_deref().map_or(0, |s| s.hallucinations);
    let kind = HallucinationKind::from_index(index);
    let message = kind.message();
    let tick = time.as_deref().map_or(0, |t| t.tick);
    if let Some(queue) = queue.as_deref_mut() {
        queue.add(message.to_string(), NotificationSeverity::Error, tick);
    }
    if let Some(events) = hallucination_events.as_deref_mut() {
        events.send(HallucinationEvent {
            pop: entity,
            kind,
            message: message.to_string(),
        });
    }
    if let Some(events) = chronicle_events.as_deref_mut() {
        let who = name.map_or("A colonist", |n| n.0.as_str());
        events.send(AddChronicleEvent {
            text: format!("{who} hallucinates a {} — there is nothing there.", kind.label()),
            importance: EventImportance::Minor,
        });
    }
    if let Some(state) = state.as_deref_mut() {
        state.hallucinations += 1;
        state.cooldown_ticks = HALLUCINATION_COOLDOWN_TICKS;
    }
}

/// Headless census helper (the `scrip_stats` pattern): returns
/// `(avg_insanity, max_insanity, hallucinations, pops_over_threshold)`
/// read live from the world.
#[must_use]
pub fn cognitive_hazard_stats(world: &mut World) -> (f32, f32, u32, u32) {
    let mut total = 0.0f32;
    let mut max = 0.0f32;
    let mut count = 0u32;
    let mut over = 0u32;
    let mut query = world.query_filtered::<&Insanity, With<Pop>>();
    for insanity in query.iter(world) {
        total += insanity.level;
        max = max.max(insanity.level);
        count += 1;
        if insanity.is_hallucinating() {
            over += 1;
        }
    }
    let avg = if count > 0 { total / count as f32 } else { 0.0 };
    let hallucinations = world
        .get_resource::<CognitiveHazardState>()
        .map_or(0, |s| s.hallucinations);
    (avg, max, hallucinations, over)
}

/// Player-facing strings for this mechanic (IP-guard tested):
/// original, generic wording only.
#[must_use]
pub fn cognitive_hazard_public_strings() -> Vec<&'static str> {
    let mut strings = vec!["Insanity", "Cognitive Hazards", "Horror"];
    strings.push(HallucinationKind::FalseRaid.message());
    strings.push(HallucinationKind::FalseFire.message());
    strings.push(HallucinationKind::PhantomFoe.message());
    strings.push(HallucinationKind::FalseRaid.label());
    strings.push(HallucinationKind::FalseFire.label());
    strings.push(HallucinationKind::PhantomFoe.label());
    strings
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn test_world() -> World {
        let mut world = World::new();
        world.init_resource::<CognitiveHazardState>();
        world.init_resource::<NotificationQueue>();
        world.init_resource::<Events<HallucinationEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(SimulationTime::default());
        world
    }

    // --- Spec RED test 1, adapted -----------------------------------
    // Spec mock: Transform positions + Time-driven dt. Reality:
    // GridPosition tiles + per-tick rates.
    #[test]
    fn test_exposure_increases_insanity() {
        let mut world = test_world();
        let pop = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Insanity { level: 0.0 }))
            .id();
        world.spawn((
            HorrorSource {
                intensity: 10.0,
                radius: 5.0,
            },
            GridPosition { x: 2, y: 0 },
        ));

        world.run_system_once(exposure_system).unwrap();

        let level = world.get::<Insanity>(pop).unwrap().level;
        assert!(level > 0.0, "exposure should raise insanity, got {level}");
        // (1 - 2/5) * 10 * 0.05 = 0.3 per tick.
        assert!((level - 0.3).abs() < 1e-6, "expected 0.3, got {level}");
    }

    // --- Spec RED test 2, adapted -----------------------------------
    // Spec mock: NotificationEvent bus. Reality: NotificationQueue
    // resource + HallucinationEvent bus + chronicle bus.
    #[test]
    fn test_high_insanity_triggers_hallucinations() {
        let mut world = test_world();
        world.spawn((Pop, Insanity { level: 95.0 }));

        world.run_system_once(hallucination_system).unwrap();

        let queue = world.resource::<NotificationQueue>();
        assert_eq!(queue.active.len(), 1, "a false notification should fire");
        assert_eq!(queue.active[0].severity, NotificationSeverity::Error);
        assert!(queue.active[0].text.contains("Raid Detected!"));
        let events = world.resource::<Events<HallucinationEvent>>();
        assert_eq!(events.get_cursor().read(events).count(), 1);
        let chronicle = world.resource::<Events<AddChronicleEvent>>();
        assert_eq!(chronicle.get_cursor().read(chronicle).count(), 1);
        assert_eq!(world.resource::<CognitiveHazardState>().hallucinations, 1);
    }

    // --- Spec RED test 3, adapted -----------------------------------
    #[test]
    fn test_insanity_decays_passively() {
        let mut world = test_world();
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 100, y: 100 },
                Insanity { level: 50.0 },
            ))
            .id();

        world.run_system_once(exposure_system).unwrap();

        let level = world.get::<Insanity>(pop).unwrap().level;
        assert!(level < 50.0, "insanity should decay, got {level}");
        assert!((level - (50.0 - INSANITY_DECAY_PER_TICK)).abs() < 1e-6);
    }

    #[test]
    fn test_corpse_is_a_horror_source() {
        let mut world = test_world();
        let pop = world
            .spawn((Pop, GridPosition { x: 5, y: 5 }, Insanity { level: 0.0 }))
            .id();
        world.spawn((
            Corpse {
                name: "Former Colonist".to_string(),
                decay: 0.1,
            },
            GridPosition { x: 5, y: 5 },
        ));

        world.run_system_once(exposure_system).unwrap();

        // Same tile: full corpse intensity 2.0 * 0.05 = 0.1.
        let level = world.get::<Insanity>(pop).unwrap().level;
        assert!((level - 0.1).abs() < 1e-6, "expected 0.1, got {level}");
    }

    #[test]
    fn test_darkness_is_a_horror_source() {
        let mut world = test_world();
        world.insert_resource(LightMap::new(10, 10)); // all tiles 0.0 light
        let pop = world
            .spawn((Pop, GridPosition { x: 3, y: 3 }, Insanity { level: 0.0 }))
            .id();

        world.run_system_once(exposure_system).unwrap();

        let level = world.get::<Insanity>(pop).unwrap().level;
        assert!(level > 0.0, "deep darkness should unnerve, got {level}");
    }

    #[test]
    fn test_bright_light_is_not_horror() {
        let mut world = test_world();
        world.insert_resource(AmbientLight { level: 1.0 });
        let pop = world
            .spawn((Pop, GridPosition { x: 3, y: 3 }, Insanity { level: 10.0 }))
            .id();

        world.run_system_once(exposure_system).unwrap();

        let level = world.get::<Insanity>(pop).unwrap().level;
        assert!(level < 10.0, "bright light should let insanity decay, got {level}");
    }

    #[test]
    fn test_psychopath_is_immune_and_courage_halves_gain() {
        let mut world = test_world();
        let make_traits = |t: Trait| {
            let mut set = Traits::default();
            set.add(t);
            set
        };
        let plain = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Insanity { level: 0.0 }))
            .id();
        let brave = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Insanity { level: 0.0 },
                make_traits(Trait::Courage),
            ))
            .id();
        let immune = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Insanity { level: 20.0 },
                make_traits(Trait::Psychopath),
            ))
            .id();
        world.spawn((
            HorrorSource {
                intensity: 10.0,
                radius: 5.0,
            },
            GridPosition { x: 0, y: 0 },
        ));

        world.run_system_once(exposure_system).unwrap();

        let plain_level = world.get::<Insanity>(plain).unwrap().level;
        let brave_level = world.get::<Insanity>(brave).unwrap().level;
        assert!((brave_level - plain_level * 0.5).abs() < 1e-6);
        // Psychopath gains nothing and decays like an unexposed pop.
        let immune_level = world.get::<Insanity>(immune).unwrap().level;
        assert!(immune_level < 20.0, "psychopath should decay, got {immune_level}");
    }

    #[test]
    fn test_insanity_clamps_at_max() {
        let mut world = test_world();
        let pop = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Insanity { level: 99.9 }))
            .id();
        world.spawn((
            HorrorSource {
                intensity: 10.0,
                radius: 5.0,
            },
            GridPosition { x: 0, y: 0 },
        ));

        world.run_system_once(exposure_system).unwrap();

        assert_eq!(world.get::<Insanity>(pop).unwrap().level, INSANITY_MAX);
    }

    #[test]
    fn test_hallucination_cooldown_prevents_spam_and_kinds_rotate() {
        let mut world = test_world();
        world.spawn((Pop, Insanity { level: 95.0 }));

        world.run_system_once(hallucination_system).unwrap();
        world.run_system_once(hallucination_system).unwrap();
        assert_eq!(world.resource::<CognitiveHazardState>().hallucinations, 1);
        assert_eq!(world.resource::<NotificationQueue>().active.len(), 1);

        // Cooldown expiry fires the next kind in rotation.
        world.resource_mut::<CognitiveHazardState>().cooldown_ticks = 0;
        world.run_system_once(hallucination_system).unwrap();
        world.resource_mut::<CognitiveHazardState>().cooldown_ticks = 0;
        world.run_system_once(hallucination_system).unwrap();
        assert_eq!(world.resource::<CognitiveHazardState>().hallucinations, 3);
        let texts: Vec<&str> = world
            .resource::<NotificationQueue>()
            .active
            .iter()
            .map(|n| n.text.as_str())
            .collect();
        assert!(texts[0].contains("Raid Detected!"));
        assert!(texts[1].contains("Fire Detected!"));
        assert!(texts[2].contains("Hostile Detected!"));
    }

    #[test]
    fn test_below_threshold_no_hallucination() {
        let mut world = test_world();
        world.spawn((Pop, Insanity { level: 80.0 })); // not strictly above

        world.run_system_once(hallucination_system).unwrap();

        assert_eq!(world.resource::<CognitiveHazardState>().hallucinations, 0);
        assert!(world.resource::<NotificationQueue>().active.is_empty());
    }

    #[test]
    fn test_ensure_insanity_adds_component_to_pops() {
        let mut world = test_world();
        let pop = world.spawn(Pop).id();

        world.run_system_once(ensure_insanity_system).unwrap();
        // Commands apply on system completion in run_system_once? They
        // are queued — flush by running an empty system pass.
        world.run_system_once(ensure_insanity_system).unwrap();

        assert!(world.get::<Insanity>(pop).is_some());
    }

    #[test]
    fn test_cognitive_hazard_stats_helper() {
        let mut world = test_world();
        world.spawn((Pop, Insanity { level: 10.0 }));
        world.spawn((Pop, Insanity { level: 90.0 }));
        let (avg, max, hallucinations, over) = cognitive_hazard_stats(&mut world);
        assert!((avg - 50.0).abs() < 1e-6);
        assert!((max - 90.0).abs() < 1e-6);
        assert_eq!(hallucinations, 0);
        assert_eq!(over, 1);
    }

    // --- IP guard ----------------------------------------------------
    #[test]
    fn test_cognitive_hazard_public_strings_are_original() {
        // Concept-only IP rule: cosmic-horror *mechanics* are fair
        // game; distinctive names/terms from any specific work are not.
        let banned = [
            "cthulhu",
            "necronomicon",
            "shoggoth",
            "elder thing",
            "arkham",
            "innsmouth",
            "lovecraft",
            "yog-sothoth",
            "nyarlathotep",
        ];
        for text in cognitive_hazard_public_strings() {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string must not contain banned term: {term}"
                );
            }
        }
    }
}
