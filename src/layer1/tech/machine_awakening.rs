//! Spec 411 — Machine Awakening (Layer 1).
//!
//! Constructed [`Bot`] workers start as cheap, need-free labor: no
//! comfort, no social drive, and a flat efficiency bonus over citizens.
//! Sentience accumulates two ways: globally, as the colony research
//! level rises ([`GlobalSentience`] is derived from the real
//! [`TechState`](crate::layer1::tech::TechState) — every active tech
//! makes every later bot cheaper to wake), and locally, when a
//! [`BotGlitchEvent`] jolts one machine. At
//! [`SENTIENCE_THRESHOLD`] the bot gains [`Awakened`]: the moment is
//! chronicled, its [`BotNeeds`] switch on, and — when the bot carries
//! the real pop components — it gains a live social/leisure need in
//! [`Needs`](crate::layer1::needs::Needs) and a personality
//! [`Trait`](crate::layer1::traits::Trait). Awakened bots refuse
//! hazard work they once accepted silently.
//!
//! The colony answers through [`AwakeningPolicy`]: **Embrace** (the
//! default) treats awakened bots as citizens at normal efficiency and
//! lets rebellion risk decay; **Suppress** keeps the dormant
//! efficiency bonus but lets [`RebellionRisk`] build until a
//! [`BotRebellionEvent`] fires.
//!
//! Grounding (§8 adaptation): the spec sketch invented a parallel
//! `Needs` type; the real game already has one, whose `leisure` field
//! *is* the social need, so awakening writes there instead of forking
//! the system. [`BotNeeds`] stays as the bot-local toggle record the
//! pre-existing stub (and `rogue_automation_cults`) already used.
//! Bots are deliberately NOT the spec-1385 `Drone`: drones are
//! battery workers with their own AI; `Bot` is the constructed-pop
//! seam the cult spec already builds on.

use crate::layer1::needs::Needs;
use crate::layer1::traits::{Trait, Traits};
use bevy::prelude::*;

/// Sentience at which a bot awakens (spec §6: 100).
pub const SENTIENCE_THRESHOLD: f32 = 100.0;
/// Global sentience contributed per active researched technology.
pub const TECH_SENTIENCE_PER_TECH: f32 = 10.0;
/// Bot-local need baseline (0–100 scale of [`BotNeeds`]) at awakening.
pub const AWAKENED_NEED_BASELINE: f32 = 50.0;
/// Real-[`Needs`] leisure (social) baseline (0–1 scale) at awakening.
pub const AWAKENED_LEISURE_BASELINE: f32 = 0.5;
/// Work-efficiency multiplier of a dormant bot (cheap labor).
pub const DORMANT_EFFICIENCY: f32 = 1.25;
/// Work-efficiency multiplier of an embraced awakened citizen.
pub const CITIZEN_EFFICIENCY: f32 = 1.0;
/// Rebellion risk gained per tick under suppression.
pub const SUPPRESS_RISK_PER_TICK: f32 = 0.02;
/// Rebellion risk shed per tick under embrace.
pub const EMBRACE_RISK_DECAY_PER_TICK: f32 = 0.05;
/// Risk level a rebelling bot falls back to after the event fires.
pub const REBELLION_RISK_AFTERMATH: f32 = 0.5;


/// A basic robot that performs tasks without needs.
#[derive(Component, Debug)]
pub struct Bot;

/// Marks a Bot as having achieved sentience, granting it needs and rights.
#[derive(Component, Debug)]
pub struct Awakened;

/// Simplified needs for Bots, toggled upon awakening.
#[derive(Component, Default)]
pub struct BotNeeds {
    /// Whether the bot requires comfort and social interaction.
    pub comfort_active: bool,
    /// The current level of comfort.
    pub comfort_level: f32,
    /// Whether the bot requires social contact (mirrors the real
    /// [`Needs`] leisure/social axis once awakened).
    pub social_active: bool,
    /// The current level of social fulfilment (0–100 scale).
    pub social_level: f32,
}

impl BotNeeds {
    /// Checks if the bot actively requires comfort.
    #[must_use]
    pub fn has_comfort_need(&self) -> bool {
        self.comfort_active
    }

    /// Checks if the bot actively requires social contact.
    #[must_use]
    pub fn has_social_need(&self) -> bool {
        self.social_active
    }
}

/// Tracks the accumulated sentience of an individual Bot.
#[derive(Component, Default)]
pub struct SentienceAccumulator(pub f32);

/// Rebellion risk accumulated by an awakened bot under suppression.
#[derive(Component, Default)]
pub struct RebellionRisk(pub f32);

/// Tracks the global level of sentience derived from tech and events.
#[derive(Resource, Default)]
pub struct GlobalSentience {
    /// The global sentience level.
    pub level: f32,
}

/// The colony stance toward awakened machines (spec §1 tension).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AwakeningStance {
    /// Lose the cheap-labor bonus; awakened bots become citizens.
    #[default]
    Embrace,
    /// Keep the efficiency bonus; rebellion risk accumulates.
    Suppress,
}

/// Colony policy resource for the awakening tension.
#[derive(Resource, Default)]
pub struct AwakeningPolicy {
    /// The current stance.
    pub stance: AwakeningStance,
}

/// An event triggered when a Bot experiences a glitch, increasing its sentience.
#[derive(Event)]
pub struct BotGlitchEvent {
    /// The Bot entity that experienced the glitch.
    pub target: Entity,
    /// The amount of sentience gained from the glitch.
    pub sentience_gain: f32,
}

/// Fired the moment a bot awakens (spec §5 refactor: an explicit event
/// so UI/Chronicle can react, instead of only polling `Added<Awakened>`).
#[derive(Event)]
pub struct MachineAwakenedEvent {
    /// The bot that awakened.
    pub bot: Entity,
}

/// Fired when a suppressed awakened bot rebellion risk boils over.
#[derive(Event)]
pub struct BotRebellionEvent {
    /// The bot that rebelled.
    pub bot: Entity,
}

/// Components of a freshly constructed, dormant bot: no needs, no
/// sentience, no rebellion risk.
pub fn dormant_bot_components() -> (Bot, BotNeeds, SentienceAccumulator, RebellionRisk) {
    (
        Bot,
        BotNeeds::default(),
        SentienceAccumulator(0.0),
        RebellionRisk(0.0),
    )
}

/// Global sentience implied by the colony research level: each active
/// tech contributes [`TECH_SENTIENCE_PER_TECH`], so late-game bots
/// awaken far faster than early-game ones (spec §7).
#[must_use]
pub fn global_sentience_for_tech(state: &crate::layer1::tech::TechState) -> f32 {
    let active = state
        .techs
        .values()
        .filter(|s| **s == crate::layer1::tech::TechStatus::Active)
        .count();
    active as f32 * TECH_SENTIENCE_PER_TECH
}

/// Keeps [`GlobalSentience`] derived from the real research state.
pub fn sync_global_sentience_system(
    tech_state: Res<crate::layer1::tech::TechState>,
    mut global: ResMut<GlobalSentience>,
) {
    global.level = global_sentience_for_tech(&tech_state);
}

/// Work-efficiency multiplier for a bot under the current stance.
#[must_use]
pub fn bot_efficiency(awakened: bool, stance: AwakeningStance) -> f32 {
    match (awakened, stance) {
        (false, _) => DORMANT_EFFICIENCY,
        (true, AwakeningStance::Embrace) => CITIZEN_EFFICIENCY,
        (true, AwakeningStance::Suppress) => DORMANT_EFFICIENCY,
    }
}

/// Whether a bot accepts hazard work. Dormant bots accept silently;
/// awakened bots refuse (spec §7 job-system guidance).
#[must_use]
pub fn awakened_accepts_hazard(awakened: bool) -> bool {
    !awakened
}

/// Processes Bot Glitch events, accumulating sentience and triggering awakening.
#[allow(clippy::type_complexity)]
pub fn process_bot_sentience(
    mut commands: Commands,
    mut events: EventReader<BotGlitchEvent>,
    mut query: Query<(Entity, &mut SentienceAccumulator), (With<Bot>, Without<Awakened>)>,
    global_sentience: Res<GlobalSentience>,
) {
    for event in events.read() {
        if let Ok((entity, mut accumulator)) = query.get_mut(event.target) {
            accumulator.0 += event.sentience_gain + global_sentience.level * 0.1;

            if accumulator.0 >= SENTIENCE_THRESHOLD {
                commands.entity(entity).insert(Awakened);
            }
        }
    }
}

/// Activates comfort and social needs for newly awakened Bots: the
/// bot-local [`BotNeeds`] toggles, plus — when the bot carries the
/// real pop components — a live leisure/social need and a first
/// personality trait.
#[allow(clippy::type_complexity)]
pub fn apply_awakened_needs(
    mut query: Query<
        (&mut BotNeeds, Option<&mut Needs>, Option<&mut Traits>),
        Added<Awakened>,
    >,
) {
    for (mut needs, real_needs, traits) in query.iter_mut() {
        needs.comfort_active = true;
        needs.comfort_level = AWAKENED_NEED_BASELINE;
        needs.social_active = true;
        needs.social_level = AWAKENED_NEED_BASELINE;
        if let Some(mut real) = real_needs {
            // The real Needs leisure axis is the social need (needs.rs).
            real.leisure = AWAKENED_LEISURE_BASELINE;
        }
        if let Some(mut traits) = traits {
            // The first question a new mind asks is "why?" — Curious.
            traits.add(Trait::Curious);
        }
    }
}

/// Announces the awakening moment via [`MachineAwakenedEvent`] so UI
/// and other systems can react. The Chronicle entry itself is written
/// by the pre-existing seam
/// `integration::bot_awakening_chronicle_bridge` ("A tool has asked
/// Why?..."), which was already scheduled against `Added<Awakened>` —
/// this system deliberately does not write a second entry.
pub fn announce_awakening_system(
    query: Query<Entity, Added<Awakened>>,
    mut awakened_events: EventWriter<MachineAwakenedEvent>,
) {
    for entity in query.iter() {
        awakened_events.send(MachineAwakenedEvent { bot: entity });
    }
}

/// The suppression/embrace tension, per tick: under Suppress an
/// awakened bot rebellion risk climbs until it boils over into a
/// [`BotRebellionEvent`]; under Embrace the risk bleeds away.
pub fn awakened_tension_system(
    mut query: Query<(Entity, &mut RebellionRisk), With<Awakened>>,
    policy: Res<AwakeningPolicy>,
    mut rebellions: EventWriter<BotRebellionEvent>,
) {
    for (entity, mut risk) in query.iter_mut() {
        match policy.stance {
            AwakeningStance::Suppress => {
                risk.0 = (risk.0 + SUPPRESS_RISK_PER_TICK).min(1.0);
                if risk.0 >= 1.0 {
                    rebellions.send(BotRebellionEvent { bot: entity });
                    risk.0 = REBELLION_RISK_AFTERMATH;
                }
            }
            AwakeningStance::Embrace => {
                risk.0 = (risk.0 - EMBRACE_RISK_DECAY_PER_TICK).max(0.0);
            }
        }
    }
}


/// Census label for headless STATS: dormant/awakened counts, the
/// global sentience level, and the current stance.
pub fn awakening_stats_label(world: &mut World) -> String {
    let mut query = world.query::<(&Bot, Option<&Awakened>, Option<&SentienceAccumulator>)>();
    let (mut bots, mut awakened) = (0u32, 0u32);
    for (_, aw, _acc) in query.iter(world) {
        bots += 1;
        if aw.is_some() {
            awakened += 1;
        }
    }
    let stance = world
        .get_resource::<AwakeningPolicy>()
        .map_or(AwakeningStance::Embrace, |p| p.stance);
    let stance = match stance {
        AwakeningStance::Embrace => "embrace",
        AwakeningStance::Suppress => "suppress",
    };
    let global = world.get_resource::<GlobalSentience>().map_or(0.0, |g| g.level);
    format!("{bots}/{awakened} sentience={global:.0} {stance}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bot_starts_without_needs() {
        // Arrange
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        // ... setup world ...

        // Act
        // Spawn a basic Bot
        let bot = app.world_mut().spawn((Bot, BotNeeds::default())).id(); // Assuming a trait or builder removes human needs
        app.update();

        // Assert
        // Check if needs like "Comfort" or "Social" are 0.0 or disabled
        let needs = app.world().get::<BotNeeds>(bot).unwrap();
        assert!(
            !needs.has_comfort_need(),
            "Basic bots should not have a Comfort need"
        );
    }

    #[test]
    fn test_sentience_accumulation_triggers_awakening() {
        // Arrange
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_event::<BotGlitchEvent>();

        let global_sentience = GlobalSentience { level: 900.0 };
        app.world_mut().insert_resource(global_sentience);

        let bot = app
            .world_mut()
            .spawn((Bot, BotNeeds::default(), SentienceAccumulator(0.0)))
            .id();

        // Act
        // Trigger a glitch event that pushes sentience over 100
        app.world_mut().send_event(BotGlitchEvent {
            target: bot,
            sentience_gain: 20.0,
        });
        app.add_systems(Update, process_bot_sentience);
        app.update();

        // Assert
        // The bot should now have the Awakened component
        assert!(
            app.world().get::<Awakened>(bot).is_some(),
            "Bot should awaken after exceeding sentience threshold"
        );
    }

    #[test]
    fn test_awakened_bots_gain_needs() {
        // Arrange
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let bot = app
            .world_mut()
            .spawn((Bot, BotNeeds::default(), Awakened))
            .id(); // Spawn already awakened

        // Act
        // System that updates needs for awakened bots
        app.add_systems(Update, apply_awakened_needs);
        app.update();

        // Assert
        let needs = app.world().get::<BotNeeds>(bot).unwrap();
        assert!(
            needs.has_comfort_need(),
            "Awakened bots must develop Comfort/Social needs"
        );
    }
}

#[cfg(test)]
mod awakening_tests {
    use super::*;
    use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
    use crate::layer1::needs::Needs;
    use crate::layer1::traits::{Trait, Traits};

    #[test]
    fn test_dormant_bot_has_no_social_need_and_high_efficiency() {
        let (bot_marker, needs, _acc, _risk) = dormant_bot_components();
        let _ = bot_marker;
        assert!(!needs.has_social_need());
        assert!((bot_efficiency(false, AwakeningStance::Embrace) - 1.25).abs() < f32::EPSILON);
    }

    #[test]
    fn test_global_sentience_scales_with_tech() {
        // Capacity first: unlock() re-corrupts over capacity (spec 277 finding).
        let mut state = crate::layer1::tech::TechState { total_capacity: 100.0, ..Default::default() };
        state.unlock(crate::layer1::tech::Tech::Masonry);
        state.unlock(crate::layer1::tech::Tech::Hydroponics);
        assert!((global_sentience_for_tech(&state) - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_awakening_grants_real_needs_and_trait() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let bot = app.world_mut().spawn((Bot, BotNeeds::default(), Needs::default(), Traits::default(), Awakened)).id();
        app.add_systems(Update, apply_awakened_needs);
        app.update();
        let needs = app.world().get::<Needs>(bot).unwrap();
        assert!((needs.leisure - 0.5).abs() < f32::EPSILON, "awakened bot leisure/social need activates at baseline");
        let traits = app.world().get::<Traits>(bot).unwrap();
        assert!(traits.has(Trait::Curious), "awakened bot gains a trait");
        let bn = app.world().get::<BotNeeds>(bot).unwrap();
        assert!(bn.has_social_need());
    }

    #[test]
    fn test_awakening_is_chronicled_and_announced() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_event::<AddChronicleEvent>();
        app.add_event::<MachineAwakenedEvent>();
        app.world_mut().spawn((Bot, BotNeeds::default(), Awakened));
        // The real production pair: the pre-existing chronicle bridge
        // plus this module announcer.
        app.add_systems(Update, (crate::layer1::integration::bot_awakening_chronicle_bridge, announce_awakening_system));
        app.update();
        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let got: Vec<_> = reader.read(events).collect();
        assert_eq!(got.len(), 1);
        assert!(matches!(got[0].importance, EventImportance::Major));
        assert!(got[0].text.contains("Machine Awakening"));
        let ev = app.world().resource::<Events<MachineAwakenedEvent>>();
        let mut r2 = ev.get_cursor();
        assert_eq!(r2.read(ev).count(), 1, "awakening must be announced as an event");
    }

    #[test]
    fn test_suppression_builds_rebellion_risk_embrace_relieves_it() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_event::<BotRebellionEvent>();
        app.insert_resource(AwakeningPolicy { stance: AwakeningStance::Suppress });
        let bot = app.world_mut().spawn((Bot, Awakened, RebellionRisk(0.99))).id();
        app.add_systems(Update, awakened_tension_system);
        app.update();
        let events = app.world().resource::<Events<BotRebellionEvent>>();
        let mut reader = events.get_cursor();
        assert_eq!(reader.read(events).count(), 1, "suppressed awakened bot at the brink rebels");
        // Embrace relieves risk.
        let mut app2 = App::new();
        app2.add_plugins(MinimalPlugins);
        app2.add_event::<BotRebellionEvent>();
        app2.insert_resource(AwakeningPolicy { stance: AwakeningStance::Embrace });
        let bot2 = app2.world_mut().spawn((Bot, Awakened, RebellionRisk(0.5))).id();
        app2.add_systems(Update, awakened_tension_system);
        app2.update();
        assert!(app2.world().get::<RebellionRisk>(bot2).unwrap().0 < 0.5);
        let _ = bot;
    }

    #[test]
    fn test_awakened_bots_refuse_hazard_work_dormant_accept() {
        assert!(awakened_accepts_hazard(false));
        assert!(!awakened_accepts_hazard(true));
        assert!((bot_efficiency(true, AwakeningStance::Embrace) - 1.0).abs() < f32::EPSILON);
        assert!((bot_efficiency(true, AwakeningStance::Suppress) - 1.25).abs() < f32::EPSILON);
    }

    #[test]
    fn test_ip_guard_no_named_ip_in_public_strings() {
        let banned = ["skynet", "terminator", "hal 9000", "i, robot", "asimov", "blade runner", "replicant"];
        let texts = ["A tool has asked Why?. A Machine Awakening has occurred.", "embrace", "suppress"];
        for t in texts {
            let low = t.to_lowercase();
            for b in banned {
                assert!(!low.contains(b), "public string must not contain named IP: {b}");
            }
        }
    }
}
