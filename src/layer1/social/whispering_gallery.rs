//! Spec 429: The Whispering Gallery.
//!
//! A beautifully constructed Grand Dome (`BuildingType::GrandDome`)
//! provides massive morale but functions as a perfect acoustic
//! reflector: every Rumor Web rumor spoken inside it is broadcast to
//! the entire colony at once, instantly accelerating the spread of
//! both good news and terrifying false panics. The dome is strictly
//! neutral (spec §8) — it amplifies whatever is said, kind or cruel.
//!
//! Grounding (audited against the real seams, per the established
//! spec-worker pattern):
//! - The spec's `Room` component, `Transform` positions, and
//!   `rumor_id: String` events are sketches. The real homes are
//!   `Building { building_type }` + `GridPosition`
//!   (`layer1::architecture::building`), and the real Rumor Web of
//!   Specs 055/343 (`layer1::social::rumor`): a rumor is a
//!   [`Rumor`] carrying a [`RumorTopic`], held per-pop in
//!   [`Knowledge`]. [`RumorSpokenEvent`] / [`BroadcastRumorEvent`]
//!   therefore carry the real [`Rumor`]; `rumor_id()` keeps the spec's
//!   string handle as a derived label.
//! - A "room with an AcousticReflector" is a building entity carrying
//!   [`AcousticReflector`]. `BuildingType::GrandDome` buildings get
//!   one from building configuration (amplification 10.0, the spec's
//!   value). A spoken event names its room entity when known; when it
//!   does not, the gallery falls back to the nearest reflector within
//!   [`DOME_RADIUS`] of the speaker's tile — the dome hears everything
//!   said under it.
//! - Broadcast actually affects pops (spec §7): [`broadcast_rumor_system`]
//!   teaches the rumor to every pop in the colony in one tick (the
//!   walking-speed adjacency web of Spec 343 becomes instant), applying
//!   the real reaction economy — bad news costs leisure exactly as
//!   `process_rumor_reaction` charges it, good news (`EventNews`)
//!   restores a little, and Skeptics refuse false rumors at the door,
//!   exactly as in `spread_rumors_system`. The *panic* itself is not
//!   re-implemented here: a pop who newly believes a false rumor is
//!   `Changed<Knowledge>`, so Spec 343's `react_to_rumors_system`
//!   lands the stress spike through the ordinary breakdown machinery
//!   on the next pass. The gallery ledger counts those new believers
//!   as amplified panics.
//! - Echo safety (spec §7): a given rumor topic broadcasts at most
//!   once per [`BROADCAST_COOLDOWN_TICKS`] window, tracked in
//!   [`WhisperingGalleryState`]. Broadcast adoptions therefore cannot
//!   re-trigger an endless echo, and the dome-speech detector below
//!   can safely watch `Changed<Knowledge>`.
//! - REFACTOR hooks (spec §5): [`AcousticDampener`] is the counterplay
//!   — a dampener fitted to the dome itself, or standing within
//!   [`DAMPENER_RADIUS`] of it, silences broadcasts from that dome.
//!   The Grand Dome also grants a baseline mood buff with no rumor
//!   involved: [`grand_dome_morale_system`] refreshes a single
//!   persistent "Grand Dome" [`MoodModifier`] on pops under the dome
//!   (the archive-of-regret aura pattern; `update_morale_cache_system`
//!   folds it into `Morale.value`), scaled by the reflector's
//!   amplification, and the building's beauty feeds room quality
//!   through the ordinary `BeautySource` path.
//! - Organic speech: [`dome_speech_detection_system`] watches pops
//!   standing under a dome whose [`Knowledge`] changed (they just
//!   learned — or invented — a rumor, via Spec 343 generation or
//!   adjacency spread) and speaks it into the gallery. Headless can
//!   also speak directly with the `whisper` command.

use bevy_ecs::prelude::*;
use std::collections::HashMap;

use crate::layer1::chronicle::{Chronicle, EventImportance};
use crate::layer1::entities::pop::Pop;
use crate::layer1::map::GridPosition;
use crate::layer1::needs::Needs;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::rumor::{is_false_rumor, Rumor, RumorTopic, RumorWebLedger};
use crate::layer1::traits::{Trait, Traits};
use crate::shared::time::SimulationTime;

/// Amplification fitted to a Grand Dome by default (spec §4 value).
pub const DEFAULT_AMPLIFICATION: f32 = 10.0;
/// Chebyshev radius: a speaker this close to a reflector counts as
/// "inside" the gallery when the spoken event names no room.
pub const DOME_RADIUS: i32 = 5;
/// Chebyshev radius of the baseline morale aura under a dome.
pub const DOME_AURA_RADIUS: i32 = 4;
/// A dampener this close to a dome (Chebyshev) silences it.
pub const DAMPENER_RADIUS: i32 = 2;
/// Minimum ticks between broadcasts of the same rumor topic.
pub const BROADCAST_COOLDOWN_TICKS: u64 = 200;
/// Leisure restored when good news is broadcast (mirrors the 0.05
/// leisure cost `process_rumor_reaction` charges for bad news).
pub const GOOD_NEWS_LEISURE_GAIN: f32 = 0.03;
/// Morale modifier granted by broadcast good news.
pub const GOOD_NEWS_MORALE_BONUS: f32 = 0.05;
/// Duration (ticks) of broadcast mood modifiers.
pub const BROADCAST_MODIFIER_DURATION: u32 = 200;
/// Label of the persistent morale modifier the dome aura maintains.
pub const DOME_MORALE_LABEL: &str = "Grand Dome";
/// Label of the mood modifier granted by broadcast good news.
pub const GOOD_NEWS_LABEL: &str = "Good News in the Grand Dome";

/// An acoustic reflector fitted to a room/building (Spec 429).
///
/// Attached to `BuildingType::GrandDome` buildings by configuration;
/// any entity carrying one turns rumors spoken in it into colony-wide
/// broadcasts. `amplification` scales the dome's baseline morale aura.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct AcousticReflector {
    /// How strongly the room reflects sound (Grand Dome: 10.0).
    pub amplification: f32,
}

impl Default for AcousticReflector {
    fn default() -> Self {
        Self {
            amplification: DEFAULT_AMPLIFICATION,
        }
    }
}

/// Counterplay to a reflector (spec §5 REFACTOR): fitted to a dome, or
/// standing within [`DAMPENER_RADIUS`] of one, it silences broadcasts
/// from that dome. The rumor still spreads at walking speed — the
/// colony just has to hear it the slow way.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct AcousticDampener {
    /// Damping strength; any positive value fully silences MVP-era
    /// broadcasts (kept as a value so partial damping has a seam).
    pub strength: f32,
}

impl Default for AcousticDampener {
    fn default() -> Self {
        Self { strength: 1.0 }
    }
}

/// A rumor was spoken aloud somewhere (Spec 429, adapted: the spec's
/// `rumor_id: String` + `location: Vec3` become the real [`Rumor`]
/// and a [`GridPosition`]).
#[derive(Event, Debug, Clone)]
pub struct RumorSpokenEvent {
    /// The rumor that was spoken (real Rumor Web payload).
    pub rumor: Rumor,
    /// Who spoke it.
    pub speaker: Entity,
    /// Where it was spoken.
    pub position: GridPosition,
    /// The room/building it was spoken in, when known.
    pub room_entity: Option<Entity>,
}

impl RumorSpokenEvent {
    /// The spec's string handle for this rumor, derived from its topic.
    #[must_use]
    pub fn rumor_id(&self) -> String {
        rumor_topic_label(&self.rumor.topic)
    }
}

/// A rumor is being broadcast colony-wide by a whispering gallery.
#[derive(Event, Debug, Clone)]
pub struct BroadcastRumorEvent {
    /// The rumor being broadcast (real Rumor Web payload).
    pub rumor: Rumor,
    /// Always true for gallery broadcasts (spec acceptance criterion).
    pub colony_wide: bool,
    /// Amplification of the reflector that caught the rumor.
    pub amplification: f32,
    /// The dome that broadcast it.
    pub source_room: Option<Entity>,
}

impl BroadcastRumorEvent {
    /// The spec's string handle for this rumor, derived from its topic.
    #[must_use]
    pub fn rumor_id(&self) -> String {
        rumor_topic_label(&self.rumor.topic)
    }
}

/// Stable string label for a rumor topic (the spec's `rumor_id`).
#[must_use]
pub fn rumor_topic_label(topic: &RumorTopic) -> String {
    match topic {
        RumorTopic::ResourceShortage(_) => "food_shortage".to_string(),
        RumorTopic::FalseMutantAttack => "false_mutant_attack".to_string(),
        RumorTopic::DoomProphecy => "doom_prophecy".to_string(),
        RumorTopic::EventNews(news) => news.clone(),
        RumorTopic::CharacterGossip(_, _) => "character_gossip".to_string(),
        RumorTopic::JobOpening(_) => "job_opening".to_string(),
    }
}

/// Echo guard: last tick each rumor topic was broadcast (spec §7 —
/// one broadcast per unique rumor per window, no infinite echoes).
#[derive(Resource, Default, Debug)]
pub struct WhisperingGalleryState {
    /// Rumor label -> tick of its most recent broadcast.
    pub last_broadcast: HashMap<String, u64>,
    /// Whether the first-ever broadcast has been chronicled.
    pub first_broadcast_chronicled: bool,
}

impl WhisperingGalleryState {
    /// Whether `label` may broadcast at `tick` (cooldown elapsed).
    #[must_use]
    pub fn may_broadcast(&self, label: &str, tick: u64) -> bool {
        self.last_broadcast
            .get(label)
            .is_none_or(|&last| tick.saturating_sub(last) >= BROADCAST_COOLDOWN_TICKS)
    }

    /// Records a broadcast of `label` at `tick`.
    pub fn record_broadcast(&mut self, label: &str, tick: u64) {
        self.last_broadcast.insert(label.to_string(), tick);
    }
}

/// Ledger of whispering-gallery activity for the headless dashboard.
#[derive(Resource, Default, Debug)]
pub struct WhisperingGalleryLedger {
    /// Colony-wide broadcasts performed.
    pub broadcasts: u64,
    /// Pops who learned a rumor from a broadcast.
    pub adoptions: u64,
    /// Broadcasts silenced by an acoustic dampener.
    pub suppressed_dampened: u64,
    /// Spoken rumors swallowed by the echo cooldown.
    pub suppressed_cooldown: u64,
    /// Pops who newly believed a false rumor via broadcast (their
    /// panic lands through Spec 343's reaction system).
    pub panics_amplified: u64,
    /// Baseline dome morale applications.
    pub dome_morale_applications: u64,
}

/// A reflector that heard a spoken rumor: entity, reflector, position.
type HeardReflector<'a> = (Entity, &'a AcousticReflector, Option<&'a GridPosition>);

/// Chebyshev distance between two tiles.
fn chebyshev(a: GridPosition, b: GridPosition) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// Whether any dampener silences the reflector at `dome`/`dome_pos`:
/// fitted to the dome entity itself, or standing nearby.
fn is_dampened(
    dome: Entity,
    dome_pos: Option<GridPosition>,
    dampener_entities: &Query<(), With<AcousticDampener>>,
    dampener_positions: &Query<&GridPosition, With<AcousticDampener>>,
) -> bool {
    if dampener_entities.get(dome).is_ok() {
        return true;
    }
    let Some(dome_pos) = dome_pos else {
        return false;
    };
    dampener_positions
        .iter()
        .any(|pos| chebyshev(*pos, dome_pos) <= DAMPENER_RADIUS)
}

/// System: rumors spoken in a room with an [`AcousticReflector`]
/// become colony-wide broadcasts (Spec 429 core).
///
/// Resolution order for the reflector: the spoken event's named room
/// first; otherwise the nearest reflector within [`DOME_RADIUS`] of
/// the speaker. Dampened domes stay silent; the echo cooldown lets
/// each unique rumor through once per window.
#[allow(clippy::too_many_arguments)]
pub fn whispering_gallery_system(
    mut spoken: EventReader<RumorSpokenEvent>,
    mut broadcasts: EventWriter<BroadcastRumorEvent>,
    reflectors: Query<(Entity, &AcousticReflector, Option<&GridPosition>)>,
    dampener_entities: Query<(), With<AcousticDampener>>,
    dampener_positions: Query<&GridPosition, With<AcousticDampener>>,
    mut state: Option<ResMut<WhisperingGalleryState>>,
    mut ledger: Option<ResMut<WhisperingGalleryLedger>>,
    time: Option<Res<SimulationTime>>,
    mut chronicle: Option<ResMut<Chronicle>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    for event in spoken.read() {
        // Find the reflector that heard this: named room, else nearest.
        let mut heard: Option<HeardReflector<'_>> = None;
        if let Some(room) = event.room_entity {
            if let Ok(found) = reflectors.get(room) {
                heard = Some(found);
            }
        }
        if heard.is_none() {
            let mut best: Option<(i32, HeardReflector<'_>)> = None;
            for candidate in reflectors.iter() {
                let Some(pos) = candidate.2 else { continue };
                let dist = chebyshev(*pos, event.position);
                if dist <= DOME_RADIUS && best.as_ref().is_none_or(|(d, _)| dist < *d) {
                    best = Some((dist, candidate));
                }
            }
            heard = best.map(|(_, candidate)| candidate);
        }
        let Some((dome, reflector, dome_pos)) = heard else {
            continue; // Spoken elsewhere: not amplified (spec test 2).
        };

        if is_dampened(
            dome,
            dome_pos.copied(),
            &dampener_entities,
            &dampener_positions,
        ) {
            if let Some(ledger) = ledger.as_mut() {
                ledger.suppressed_dampened += 1;
            }
            continue;
        }

        let label = event.rumor_id();
        if let Some(state) = state.as_mut() {
            if !state.may_broadcast(&label, tick) {
                if let Some(ledger) = ledger.as_mut() {
                    ledger.suppressed_cooldown += 1;
                }
                continue;
            }
            state.record_broadcast(&label, tick);
        }

        broadcasts.send(BroadcastRumorEvent {
            rumor: event.rumor.clone(),
            colony_wide: true,
            amplification: reflector.amplification,
            source_room: Some(dome),
        });
        if let Some(ledger) = ledger.as_mut() {
            ledger.broadcasts += 1;
        }

        // Chronicle: the first broadcast ever, and any false rumor —
        // the dome makes sure *everyone* hears the scare story.
        if let Some(chronicle) = chronicle.as_mut() {
            let first = state
                .as_ref()
                .is_some_and(|s| !s.first_broadcast_chronicled);
            if first {
                if let Some(state) = state.as_mut() {
                    state.first_broadcast_chronicled = true;
                }
                chronicle.add_event(
                    tick,
                    "The Grand Dome catches a voice and gives it to the whole colony at once. Nothing said beneath it stays small. [WHISPERING_GALLERY]".to_string(),
                    EventImportance::Major,
                );
            } else if is_false_rumor(&event.rumor.topic) {
                chronicle.add_event(
                    tick,
                    "A baseless scare story, spoken under the Grand Dome, reaches every ear in the colony before anyone can shush it. [WHISPERING_GALLERY]".to_string(),
                    EventImportance::Major,
                );
            }
        }
    }
}

/// System: a colony-wide broadcast teaches the rumor to every pop
/// (Spec 429 §7 — the broadcast must actually land).
///
/// Adoptions dedup through [`Knowledge::add_rumor`], so a pop who
/// already knows a rumor is untouched (no double morale hit). Bad
/// news charges the same 0.05 leisure `process_rumor_reaction`
/// charges face-to-face; good news restores a little leisure and
/// leaves a mood modifier; Skeptics refuse false rumors at the door.
#[allow(clippy::type_complexity)]
pub fn broadcast_rumor_system(
    mut events: EventReader<BroadcastRumorEvent>,
    mut pops: Query<(
        &mut crate::layer1::social::rumor::Knowledge,
        Option<&mut Needs>,
        Option<&mut Morale>,
        Option<&Traits>,
    )>,
    mut ledger: Option<ResMut<WhisperingGalleryLedger>>,
    mut web_ledger: Option<ResMut<RumorWebLedger>>,
) {
    for event in events.read() {
        if !event.colony_wide {
            continue;
        }
        let mut adoptions: u64 = 0;
        let mut panics: u64 = 0;
        let mut debunked: u64 = 0;
        for (mut knowledge, needs, morale, traits) in &mut pops {
            let skeptic = traits.is_some_and(|t| t.has(Trait::Skeptic));
            if skeptic && is_false_rumor(&event.rumor.topic) {
                if !knowledge.knows(&event.rumor.topic) {
                    debunked += 1;
                }
                continue;
            }
            let before = knowledge.known_rumors.len();
            knowledge.add_rumor(event.rumor.clone());
            if knowledge.known_rumors.len() == before {
                continue; // Already knew: no re-reaction.
            }
            adoptions += 1;
            match &event.rumor.topic {
                RumorTopic::ResourceShortage(_)
                | RumorTopic::DoomProphecy
                | RumorTopic::FalseMutantAttack => {
                    if let Some(mut needs) = needs {
                        needs.leisure = (needs.leisure - 0.05).max(0.0);
                    }
                    if is_false_rumor(&event.rumor.topic) {
                        // The stress spike itself lands via Spec 343's
                        // react_to_rumors_system (Changed<Knowledge>).
                        panics += 1;
                    }
                }
                RumorTopic::EventNews(_) => {
                    if let Some(mut needs) = needs {
                        needs.leisure = (needs.leisure + GOOD_NEWS_LEISURE_GAIN).min(1.0);
                    }
                    if let Some(mut morale) = morale {
                        morale.modifiers.retain(|m| m.label != GOOD_NEWS_LABEL);
                        morale.add_modifier(MoodModifier {
                            label: GOOD_NEWS_LABEL.to_string(),
                            value: GOOD_NEWS_MORALE_BONUS,
                            duration: BROADCAST_MODIFIER_DURATION,
                        });
                    }
                }
                RumorTopic::CharacterGossip(_, _) | RumorTopic::JobOpening(_) => {
                    // Word travels without extra emotional weight; the
                    // affinity economy already ran when the gossip was
                    // first shared face-to-face (Spec 055).
                }
            }
        }
        if let Some(ledger) = ledger.as_mut() {
            ledger.adoptions += adoptions;
            ledger.panics_amplified += panics;
        }
        if let Some(web) = web_ledger.as_mut() {
            web.false_rumors_debunked += debunked;
        }
    }
}

/// System: pops standing under a Grand Dome receive its baseline
/// morale aura — the dome is beautiful even in silence (spec §5).
///
/// Refreshes one persistent [`DOME_MORALE_LABEL`] modifier (duration
/// 2, re-applied each tick while in range) scaled by the strongest
/// nearby reflector's amplification; `update_morale_cache_system`
/// folds it into `Morale.value`.
pub fn grand_dome_morale_system(
    reflectors: Query<(&GridPosition, &AcousticReflector)>,
    mut pops: Query<(&GridPosition, &mut Morale), With<Pop>>,
    mut ledger: Option<ResMut<WhisperingGalleryLedger>>,
) {
    if reflectors.is_empty() {
        return;
    }
    let mut applications: u64 = 0;
    for (pos, mut morale) in &mut pops {
        let mut best: Option<f32> = None;
        for (dome_pos, reflector) in reflectors.iter() {
            if chebyshev(*pos, *dome_pos) <= DOME_AURA_RADIUS {
                best = Some(best.map_or(reflector.amplification, |b: f32| {
                    b.max(reflector.amplification)
                }));
            }
        }
        let Some(amplification) = best else { continue };
        let value = 0.04 + amplification * 0.004;
        if let Some(existing) = morale
            .modifiers
            .iter_mut()
            .find(|m| m.label == DOME_MORALE_LABEL)
        {
            existing.value = value;
            existing.duration = 2;
        } else {
            morale.add_modifier(MoodModifier {
                label: DOME_MORALE_LABEL.to_string(),
                value,
                duration: 2,
            });
            applications += 1;
        }
    }
    if let Some(ledger) = ledger.as_mut() {
        ledger.dome_morale_applications += applications;
    }
}

/// System: pops under a dome whose knowledge just changed speak what
/// they learned into the gallery (the organic bridge).
///
/// Fires on `Changed<Knowledge>` for pops within [`DOME_RADIUS`] of a
/// reflector and emits one [`RumorSpokenEvent`] per known rumor; the
/// gallery's echo cooldown decides what actually broadcasts, so a
/// broadcast adoption under the dome cannot loop.
pub fn dome_speech_detection_system(
    changed: Query<(Entity, &GridPosition, &crate::layer1::social::rumor::Knowledge), Changed<crate::layer1::social::rumor::Knowledge>>,
    reflectors: Query<(Entity, &GridPosition), With<AcousticReflector>>,
    mut spoken: EventWriter<RumorSpokenEvent>,
) {
    if reflectors.is_empty() {
        return;
    }
    for (entity, pos, knowledge) in changed.iter() {
        let mut nearest: Option<(i32, Entity)> = None;
        for (dome, dome_pos) in reflectors.iter() {
            let dist = chebyshev(*pos, *dome_pos);
            if dist <= DOME_RADIUS && nearest.as_ref().is_none_or(|(d, _)| dist < *d) {
                nearest = Some((dist, dome));
            }
        }
        let Some((_, dome)) = nearest else { continue };
        for rumor in &knowledge.known_rumors {
            spoken.send(RumorSpokenEvent {
                rumor: rumor.clone(),
                speaker: entity,
                position: *pos,
                room_entity: Some(dome),
            });
        }
    }
}

/// Grand Domes currently standing (headless STATS / `gallery` panel).
#[must_use]
pub fn grand_dome_count(world: &mut World) -> usize {
    let mut query = world.query::<&AcousticReflector>();
    query.iter(world).count()
}

#[cfg(test)]
mod spec_429_tests {
    use super::*;
    use crate::layer1::architecture::building::BuildingType;
    use crate::layer1::pop::Pop;
    use crate::layer1::social::rumor::Knowledge;
    use crate::layer1::stress::StressTracker;

    fn rumor(topic: RumorTopic) -> Rumor {
        Rumor {
            topic,
            source: Entity::PLACEHOLDER,
            timestamp: 0,
            strength: 1.0,
        }
    }

    fn food_rumor() -> Rumor {
        rumor(RumorTopic::ResourceShortage("Food".to_string()))
    }

    fn run<Marker>(world: &mut World, system: impl IntoSystemConfigs<Marker>) {
        let mut schedule = Schedule::default();
        schedule.add_systems(system);
        schedule.run(world);
    }

    fn gallery_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<RumorSpokenEvent>>();
        world.init_resource::<Events<BroadcastRumorEvent>>();
        world.init_resource::<WhisperingGalleryState>();
        world.init_resource::<WhisperingGalleryLedger>();
        world.insert_resource(SimulationTime {
            tick: 0,
            ..Default::default()
        });
        world
    }

    fn count_broadcasts(world: &mut World) -> Vec<BroadcastRumorEvent> {
        // Drain via a fresh reader over the event buffer: run an
        // extractor system that moves events into a resource.
        #[derive(Resource, Default)]
        struct Collected(Vec<BroadcastRumorEvent>);
        world.init_resource::<Collected>();
        fn collect(
            mut events: EventReader<BroadcastRumorEvent>,
            mut out: ResMut<Collected>,
        ) {
            for e in events.read() {
                out.0.push(e.clone());
            }
        }
        run(world, collect);
        std::mem::take(&mut world.resource_mut::<Collected>().0)
    }

    // Spec 429 RED test 1 (spec section 3, adapted to real types:
    // Room -> Grand Dome building entity, Transform -> GridPosition,
    // rumor_id String -> real RumorTopic payload).
    #[test]
    fn test_grand_dome_amplifies_rumors() {
        let mut world = gallery_world();
        let dome = world
            .spawn((
                crate::layer1::building::Building {
                    building_type: BuildingType::GrandDome,
                },
                AcousticReflector {
                    amplification: 10.0,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        world.send_event(RumorSpokenEvent {
            rumor: food_rumor(),
            speaker: Entity::PLACEHOLDER,
            position: GridPosition { x: 0, y: 0 },
            room_entity: Some(dome),
        });
        run(&mut world, whispering_gallery_system);

        let events = count_broadcasts(&mut world);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].rumor.topic,
            RumorTopic::ResourceShortage("Food".to_string())
        );
        assert_eq!(events[0].rumor_id(), "food_shortage");
        assert!(events[0].colony_wide);
        assert_eq!(events[0].source_room, Some(dome));
    }

    // Spec 429 RED test 2: a room without a reflector stays local.
    #[test]
    fn test_rumor_outside_dome_not_amplified() {
        let mut world = gallery_world();
        let standard_room = world
            .spawn((
                crate::layer1::building::Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 10, y: 0 },
            ))
            .id();

        world.send_event(RumorSpokenEvent {
            rumor: rumor(RumorTopic::EventNews("mutiny".to_string())),
            speaker: Entity::PLACEHOLDER,
            position: GridPosition { x: 10, y: 0 },
            room_entity: Some(standard_room),
        });
        run(&mut world, whispering_gallery_system);

        assert!(count_broadcasts(&mut world).is_empty());
    }

    #[test]
    fn test_spatial_fallback_hears_speaker_under_dome() {
        let mut world = gallery_world();
        world.spawn((
            AcousticReflector::default(),
            GridPosition { x: 3, y: 3 },
        ));
        // No room named, but the speaker stands under the dome.
        world.send_event(RumorSpokenEvent {
            rumor: food_rumor(),
            speaker: Entity::PLACEHOLDER,
            position: GridPosition { x: 4, y: 3 },
            room_entity: None,
        });
        run(&mut world, whispering_gallery_system);
        assert_eq!(count_broadcasts(&mut world).len(), 1);

        // A speaker far from any dome is not heard.
        let mut world = gallery_world();
        world.spawn((
            AcousticReflector::default(),
            GridPosition { x: 3, y: 3 },
        ));
        world.send_event(RumorSpokenEvent {
            rumor: food_rumor(),
            speaker: Entity::PLACEHOLDER,
            position: GridPosition { x: 30, y: 30 },
            room_entity: None,
        });
        run(&mut world, whispering_gallery_system);
        assert!(count_broadcasts(&mut world).is_empty());
    }

    #[test]
    fn test_same_rumor_broadcasts_once_per_cooldown() {
        let mut world = gallery_world();
        let dome = world
            .spawn((AcousticReflector::default(), GridPosition { x: 0, y: 0 }))
            .id();
        for _ in 0..3 {
            world.send_event(RumorSpokenEvent {
                rumor: food_rumor(),
                speaker: Entity::PLACEHOLDER,
                position: GridPosition { x: 0, y: 0 },
                room_entity: Some(dome),
            });
        }
        // A different rumor in the same tick still gets through.
        world.send_event(RumorSpokenEvent {
            rumor: rumor(RumorTopic::DoomProphecy),
            speaker: Entity::PLACEHOLDER,
            position: GridPosition { x: 0, y: 0 },
            room_entity: Some(dome),
        });
        run(&mut world, whispering_gallery_system);
        let events = count_broadcasts(&mut world);
        assert_eq!(events.len(), 2, "one per unique rumor, got {events:?}");
        assert_eq!(
            world.resource::<WhisperingGalleryLedger>().suppressed_cooldown,
            2
        );
    }

    #[test]
    fn test_dampener_suppresses_broadcast() {
        let mut world = gallery_world();
        // Dampener fitted to the dome itself.
        let dome = world
            .spawn((
                AcousticReflector::default(),
                AcousticDampener::default(),
                GridPosition { x: 0, y: 0 },
            ))
            .id();
        world.send_event(RumorSpokenEvent {
            rumor: food_rumor(),
            speaker: Entity::PLACEHOLDER,
            position: GridPosition { x: 0, y: 0 },
            room_entity: Some(dome),
        });
        run(&mut world, whispering_gallery_system);
        assert!(count_broadcasts(&mut world).is_empty());
        assert_eq!(
            world.resource::<WhisperingGalleryLedger>().suppressed_dampened,
            1
        );

        // Dampener standing next to an unfitted dome also silences it.
        let mut world = gallery_world();
        let dome = world
            .spawn((AcousticReflector::default(), GridPosition { x: 0, y: 0 }))
            .id();
        world.spawn((AcousticDampener::default(), GridPosition { x: 1, y: 0 }));
        world.send_event(RumorSpokenEvent {
            rumor: food_rumor(),
            speaker: Entity::PLACEHOLDER,
            position: GridPosition { x: 0, y: 0 },
            room_entity: Some(dome),
        });
        run(&mut world, whispering_gallery_system);
        assert!(count_broadcasts(&mut world).is_empty());
    }

    #[test]
    fn test_broadcast_teaches_whole_colony_and_charges_bad_news() {
        let mut world = gallery_world();
        world.init_resource::<RumorWebLedger>();
        let near = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Knowledge::default(),
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
            ))
            .id();
        // Far away: unreachable by Spec 343 adjacency, reached here.
        let far = world
            .spawn((
                Pop,
                GridPosition { x: 90, y: 90 },
                Knowledge::default(),
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
            ))
            .id();
        // Already knows: untouched, no double reaction.
        let knower = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 5 },
                Knowledge {
                    known_rumors: vec![food_rumor()],
                },
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
            ))
            .id();

        world.send_event(BroadcastRumorEvent {
            rumor: food_rumor(),
            colony_wide: true,
            amplification: 10.0,
            source_room: None,
        });
        run(&mut world, broadcast_rumor_system);

        for pop in [near, far] {
            assert!(
                world
                    .get::<Knowledge>(pop)
                    .unwrap()
                    .knows(&RumorTopic::ResourceShortage("Food".to_string())),
                "broadcast should teach every pop"
            );
            assert!(
                world.get::<Needs>(pop).unwrap().leisure < 0.5,
                "bad news should cost leisure"
            );
        }
        assert!((world.get::<Needs>(knower).unwrap().leisure - 0.5).abs() < f32::EPSILON);
        assert_eq!(world.resource::<WhisperingGalleryLedger>().adoptions, 2);
    }

    #[test]
    fn test_broadcast_false_rumor_panics_via_rumor_web_reaction() {
        let mut world = gallery_world();
        world.init_resource::<RumorWebLedger>();
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 50, y: 50 },
                Knowledge::default(),
                Needs::default(),
                StressTracker::default(),
            ))
            .id();

        world.send_event(BroadcastRumorEvent {
            rumor: rumor(RumorTopic::FalseMutantAttack),
            colony_wide: true,
            amplification: 10.0,
            source_room: None,
        });
        run(&mut world, broadcast_rumor_system);
        assert_eq!(
            world.resource::<WhisperingGalleryLedger>().panics_amplified,
            1
        );
        // The panic stress itself lands through Spec 343's reaction
        // system on the newly-changed Knowledge.
        run(
            &mut world,
            crate::layer1::social::rumor::react_to_rumors_system,
        );
        assert!(
            world.get::<StressTracker>(pop).unwrap().accumulated_stress > 0.0,
            "broadcast false rumor should panic its new believer"
        );
    }

    #[test]
    fn test_skeptic_refuses_broadcast_false_rumor() {
        let mut world = gallery_world();
        world.init_resource::<RumorWebLedger>();
        let mut traits = Traits::default();
        traits.add(Trait::Skeptic);
        let skeptic = world
            .spawn((Pop, Knowledge::default(), traits))
            .id();
        let believer = world.spawn((Pop, Knowledge::default())).id();

        world.send_event(BroadcastRumorEvent {
            rumor: rumor(RumorTopic::FalseMutantAttack),
            colony_wide: true,
            amplification: 10.0,
            source_room: None,
        });
        run(&mut world, broadcast_rumor_system);

        assert!(!world
            .get::<Knowledge>(skeptic)
            .unwrap()
            .knows(&RumorTopic::FalseMutantAttack));
        assert!(world
            .get::<Knowledge>(believer)
            .unwrap()
            .knows(&RumorTopic::FalseMutantAttack));
        assert_eq!(world.resource::<RumorWebLedger>().false_rumors_debunked, 1);
    }

    #[test]
    fn test_broadcast_good_news_lifts_colony() {
        let mut world = gallery_world();
        let pop = world
            .spawn((
                Pop,
                Knowledge::default(),
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
                Morale::default(),
            ))
            .id();
        world.send_event(BroadcastRumorEvent {
            rumor: rumor(RumorTopic::EventNews("First harvest festival".to_string())),
            colony_wide: true,
            amplification: 10.0,
            source_room: None,
        });
        run(&mut world, broadcast_rumor_system);

        assert!(world.get::<Needs>(pop).unwrap().leisure > 0.5);
        let morale = world.get::<Morale>(pop).unwrap();
        assert!(morale
            .modifiers
            .iter()
            .any(|m| m.label == GOOD_NEWS_LABEL && m.value > 0.0));
    }

    #[test]
    fn test_dome_grants_baseline_morale_without_rumors() {
        let mut world = gallery_world();
        world.spawn((AcousticReflector::default(), GridPosition { x: 0, y: 0 }));
        let inside = world
            .spawn((Pop, GridPosition { x: 2, y: 1 }, Morale::default()))
            .id();
        let outside = world
            .spawn((Pop, GridPosition { x: 40, y: 40 }, Morale::default()))
            .id();

        run(&mut world, grand_dome_morale_system);

        let morale = world.get::<Morale>(inside).unwrap();
        assert!(morale
            .modifiers
            .iter()
            .any(|m| m.label == DOME_MORALE_LABEL && (m.value - 0.08).abs() < 0.001));
        assert!(world.get::<Morale>(outside).unwrap().modifiers.is_empty());
    }

    #[test]
    fn test_dome_speech_detection_speaks_for_pop_under_dome() {
        let mut world = gallery_world();
        let dome = world
            .spawn((AcousticReflector::default(), GridPosition { x: 0, y: 0 }))
            .id();
        world.spawn((
            Pop,
            GridPosition { x: 1, y: 0 },
            Knowledge {
                known_rumors: vec![food_rumor()],
            },
        ));
        // Far pop with the same knowledge: not under the dome, silent.
        world.spawn((
            Pop,
            GridPosition { x: 80, y: 80 },
            Knowledge {
                known_rumors: vec![food_rumor()],
            },
        ));

        run(&mut world, dome_speech_detection_system);
        run(&mut world, whispering_gallery_system);

        let events = count_broadcasts(&mut world);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source_room, Some(dome));
    }

    #[test]
    fn test_grand_dome_building_metadata() {
        assert_eq!(BuildingType::GrandDome.label(), "Grand Dome");
        let cost = BuildingType::GrandDome.cost(crate::layer1::building::MaterialType::Stone);
        assert!(cost.metal > 0.0 && cost.stone > 0.0);
        assert!(BuildingType::GrandDome.beauty_value() > 0.0);
        assert!(BuildingType::GrandDome.beauty_radius() > 0.0);
        assert_eq!(AcousticReflector::default().amplification, 10.0);
    }

    #[test]
    fn test_ip_guard_no_named_ip_in_public_strings() {
        // Concept-only: the dome is a generic architectural feature.
        // No real-world venue names or borrowed IP in public strings.
        let banned = [
            "st paul",
            "st. paul",
            "grand central",
            "hagia",
            "union station",
            "whispering gallery of",
        ];
        let public_strings = [
            BuildingType::GrandDome.label().to_string(),
            DOME_MORALE_LABEL.to_string(),
            GOOD_NEWS_LABEL.to_string(),
            rumor_topic_label(&RumorTopic::ResourceShortage("Food".to_string())),
            rumor_topic_label(&RumorTopic::DoomProphecy),
            rumor_topic_label(&RumorTopic::FalseMutantAttack),
        ];
        for text in &public_strings {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string {text:?} contains banned term {term:?}"
                );
            }
        }
    }
}
