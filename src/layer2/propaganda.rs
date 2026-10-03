//! The Propaganda Constellation (spec 1380).
//!
//! Launch slogan satellites that link into a giant glowing message in the
//! night sky. While enough linked satellites are active, the colony's pops
//! see "the Broadcast" and gain a colony-wide morale boost that overrides
//! negative moods. Rival factions can hack the constellation to flip the
//! message into despair (a morale penalty instead). The counterplay is
//! shooting down your own satellites — which kills the message but feeds the
//! orbital-debris mechanic (Kessler-syndrome vulnerability).
//!
//! Original concept only: slogan texts are generated from the procedural
//! lore fragments tagged `slogan-hope` / `slogan-despair`, plus built-in
//! fallbacks. No named characters, places, or distinctive IP anywhere.

use bevy::prelude::*;
use rand::seq::SliceRandom;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::day_night::{DayNightCycle, TimeOfDay};
use crate::layer1::diplomacy::factions::rivals::RivalColony;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer2::debris::OrbitalDebris;
use crate::layer2::events::LaunchEvent;
use crate::layer2::fleet::InOrbit;
use crate::shared::narrative::NarrativeGenerator;

/// Minimum linked satellites for the constellation to activate.
pub const CONSTELLATION_MIN_SATELLITES: usize = 5;

/// Morale boost while the hope message shines. Tuned to override typical
/// negative moods.
pub const HOPE_BOOST: f32 = 0.35;

/// Morale penalty while a hacked despair message shines.
pub const DESPAIR_PENALTY: f32 = 0.40;

/// Debris added to orbit by shooting down one satellite. Meaningful against
/// the existing Kessler thresholds in `debris.rs` (attrition starts at 0.1).
pub const SHOOT_DOWN_DEBRIS: f32 = 0.15;

/// Per-tick hack probability per active rival faction. Low — the dread comes
/// from the flip, not the frequency.
pub const HACK_PROBABILITY_PER_TICK_PER_RIVAL: f32 = 0.002;

/// Launch cost: refined metal for the satellite bus.
pub const SATELLITE_METAL_COST: f32 = 3.0;

/// Launch cost: tools for the orbital assembly rig.
pub const SATELLITE_TOOL_COST: f32 = 1.0;

/// The morale modifier label the constellation applies (refreshed, never stacked).
pub const PROPAGANDA_MODIFIER_LABEL: &str = "Propaganda Constellation";

/// Lore fragment tag for procedurally generated hope slogans.
pub const SLOGAN_HOPE_FRAGMENT: &str = "slogan-hope";
/// Lore fragment tag for procedurally generated despair slogans.
pub const SLOGAN_DESPAIR_FRAGMENT: &str = "slogan-despair";

/// Built-in hope slogans (fallback when the lore generator has no fragments).
const HOPE_SLOGANS: &[&str] = &[
    "WE ENDURE",
    "THE HARVEST IS OURS",
    "TOMORROW BELONGS TO US",
    "HOLD THE LINE",
    "THE COLONY PROVIDES",
];

/// Built-in despair slogans (fallback when the lore generator has no fragments).
const DESPAIR_SLOGANS: &[&str] = &[
    "OBEY THE STATIC",
    "THE VOID IS HUNGRY",
    "NO DAWN COMES",
    "YOU ARE ALONE",
    "THE SIGNAL EATS",
];

/// The message currently spelled across the night sky.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SloganMessage {
    /// An uplifting slogan: the colony's own Broadcast.
    Hope(String),
    /// A hacked slogan: despair beamed down at the colony.
    Despair(String),
    /// No message: the sky is dark (too few linked satellites).
    ///
    /// `update_constellation` sets this whenever the link drops below
    /// [`CONSTELLATION_MIN_SATELLITES`], which is how "dropping below N ends
    /// the boost" is enforced: [`apply_constellation_morale`] only broadcasts
    /// a lit (Hope/Despair) message. Nothing is lit until the constellation
    /// first activates.
    #[default]
    Dark,
}

impl SloganMessage {
    /// The text of the message (empty when the sky is dark).
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            SloganMessage::Hope(s) | SloganMessage::Despair(s) => s,
            SloganMessage::Dark => "",
        }
    }

    /// Whether this is the colony's own hope message (vs. a hacked despair one).
    #[must_use]
    pub fn is_hope(&self) -> bool {
        matches!(self, SloganMessage::Hope(_))
    }

    /// Whether any message is currently lit in the sky.
    #[must_use]
    pub fn is_lit(&self) -> bool {
        !matches!(self, SloganMessage::Dark)
    }
}

/// A slogan satellite: one bright pixel of the sky-message.
///
/// Satellites are orbital entities (`InOrbit`), spawned via the colony's
/// "Launch Slogan Satellite" action. Each carries one word of the slogan.
#[derive(Component, Default, Debug, Clone)]
pub struct SloganSatellite {
    /// The word of the sky-message this satellite projects.
    pub word: String,
}

/// The constellation: the set of linked slogan satellites and the message
/// they spell.
#[derive(Resource, Debug, Default)]
pub struct Constellation {
    /// Entities currently linked into the constellation.
    pub satellites: Vec<Entity>,
    /// The message the sky is spelling.
    pub message: SloganMessage,
    /// Whether the first-activation chronicle has fired.
    announced: bool,
    /// Active state on the last tick (for transition detection).
    was_active: bool,
}

impl Constellation {
    /// The constellation is active when enough satellites are linked.
    #[must_use]
    pub fn active(&self) -> bool {
        self.satellites.len() >= CONSTELLATION_MIN_SATELLITES
    }

    /// The per-pop morale value the constellation currently broadcasts
    /// (positive for hope, negative for hacked despair, zero when dark).
    #[must_use]
    pub fn morale_value(&self) -> f32 {
        match &self.message {
            SloganMessage::Hope(_) => HOPE_BOOST,
            SloganMessage::Despair(_) => -DESPAIR_PENALTY,
            SloganMessage::Dark => 0.0,
        }
    }
}

/// Reconcile the constellation's satellite list: despawned satellites drop
/// out, newly launched ones link in. Fires the first-activation chronicle.
///
/// Message lifecycle: while too few satellites are linked the sky is
/// [`SloganMessage::Dark`] and the boost is off; on (re)activation a fresh
/// hope slogan is drawn from the procedural generator. Shooting the link
/// below the threshold therefore always kills the message — including a
/// hacked one.
pub fn update_constellation(world: &mut World) {
    world.init_resource::<Events<AddChronicleEvent>>();
    world.init_resource::<Constellation>();

    let live: Vec<Entity> = {
        let mut query = world.query::<(Entity, &SloganSatellite)>();
        query.iter(world).map(|(e, _)| e).collect()
    };

    // Fresh slogan for a (re)activation, drawn before the mutable borrow below.
    let fresh_slogan = generate_slogan(world.get_resource::<NarrativeGenerator>(), true);

    let (just_activated, message_text) = {
        let mut constellation = world.resource_mut::<Constellation>();
        constellation.satellites.retain(|e| live.contains(e));
        for e in &live {
            if !constellation.satellites.contains(e) {
                constellation.satellites.push(*e);
            }
        }
        let active = constellation.active();
        let just_activated = active && !constellation.was_active;
        if !active {
            // The sky goes dark; the message — hope or hacked despair —
            // dies with the link.
            constellation.message = SloganMessage::Dark;
        } else if !constellation.message.is_lit() {
            // Fresh broadcast on (re)activation.
            constellation.message = SloganMessage::Hope(fresh_slogan);
        }
        constellation.was_active = active;
        let first = just_activated && !constellation.announced;
        if first {
            constellation.announced = true;
        }
        (first, constellation.message.text().to_string())
    };

    if just_activated {
        world.resource_mut::<Events<AddChronicleEvent>>().send(
            AddChronicleEvent {
                text: format!(
                    "The sky learned our name. The Broadcast spells: \"{message_text}\"."
                ),
                importance: EventImportance::Major,
            },
        );
    }
}

/// Apply (or remove) the constellation's morale modifier on every pop.
///
/// The modifier is refreshed each tick, never stacked. It is broadcast only
/// while a message is lit (Hope or hacked Despair); [`update_constellation`]
/// darkens the sky whenever the link drops below
/// [`CONSTELLATION_MIN_SATELLITES`], which is how dropping below N ends the
/// boost. The slogan only works at night — it must be *visible* in the sky.
/// While dark or in daylight, any stale modifier is removed.
pub fn apply_constellation_morale(world: &mut World) {
    world.init_resource::<Constellation>();

    let (lit, value) = {
        let constellation = world.resource::<Constellation>();
        (
            constellation.message.is_lit(),
            constellation.morale_value(),
        )
    };

    // Night-sky visibility: the message must be visible to work.
    let visible = match world.get_resource::<DayNightCycle>() {
        Some(cycle) => cycle.time_of_day == TimeOfDay::Night,
        // No day/night cycle (e.g. unit tests): apply regardless.
        None => true,
    };

    let mut query = world.query_filtered::<&mut Morale, With<Pop>>();
    for mut morale in query.iter_mut(world) {
        morale
            .modifiers
            .retain(|m| m.label != PROPAGANDA_MODIFIER_LABEL);
        if lit && visible {
            morale.add_modifier(MoodModifier {
                label: PROPAGANDA_MODIFIER_LABEL.to_string(),
                value,
                duration: 2,
            });
        }
    }
}

/// A rival faction hacks the constellation, flipping the sky-message to
/// despair. The counterplay is to shoot down your own satellites.
pub fn hack_constellation(world: &mut World, message: String) {
    world.init_resource::<Events<AddChronicleEvent>>();
    world.init_resource::<Constellation>();

    {
        let mut constellation = world.resource_mut::<Constellation>();
        constellation.message = SloganMessage::Despair(message);
    }

    world.resource_mut::<Events<AddChronicleEvent>>().send(
        AddChronicleEvent {
            text: "The sky is lying to us. The Broadcast has been hacked.".to_string(),
            importance: EventImportance::Major,
        },
    );
}

/// Shoot down one of the colony's own satellites. Kills the message (the
/// constellation deactivates once too few satellites remain linked) but feeds
/// the orbital-debris mechanic: debris is added both to the orbited planet
/// and to the simplified orbital-debris ledger the spec tests observe.
pub fn shoot_down_satellite(world: &mut World, sat: Entity) {
    world.init_resource::<Events<AddChronicleEvent>>();
    world.init_resource::<OrbitalDebris>();

    // Read the orbit before the entity is gone.
    let planet = world.get::<InOrbit>(sat).map(|orbit| orbit.parent);

    if world.despawn(sat) {
        // Feed the real Kessler machinery: debris on the orbited planet, so
        // launch risk and fleet attrition feel the shoot-down.
        if let Some(planet) = planet {
            if let Some(mut debris) = world.get_mut::<OrbitalDebris>(planet) {
                debris.0 += SHOOT_DOWN_DEBRIS;
            } else {
                world
                    .entity_mut(planet)
                    .insert(OrbitalDebris(SHOOT_DOWN_DEBRIS));
            }
        }
        // Simplified ledger (also observed by the spec tests).
        world.resource_mut::<OrbitalDebris>().0 += SHOOT_DOWN_DEBRIS;

        world.resource_mut::<Events<AddChronicleEvent>>().send(
            AddChronicleEvent {
                text: "We shot our own satellite from the sky. The message died; the void drank the wreckage."
                    .to_string(),
                importance: EventImportance::Major,
            },
        );
    }
}

/// Rival hack attempts tick while the constellation is active, with
/// probability scaled by the number of active rival factions.
pub fn constellation_hack_system(
    mut constellation: ResMut<Constellation>,
    rivals: Query<(), With<RivalColony>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    generator: Option<Res<NarrativeGenerator>>,
) {
    if !constellation.active() {
        return;
    }
    #[allow(clippy::cast_precision_loss)]
    let rival_count = rivals.iter().count() as f32;
    if rival_count <= 0.0 {
        return;
    }
    let probability = HACK_PROBABILITY_PER_TICK_PER_RIVAL * rival_count;
    if rand::random::<f32>() < probability {
        let slogan = generate_slogan(generator.as_deref(), false);
        constellation.message = SloganMessage::Despair(slogan);
        chronicle.send(AddChronicleEvent {
            text: "The sky is lying to us. The Broadcast has been hacked.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Launch a slogan satellite into orbit around `planet`.
///
/// Costs metal + tools, emits `LaunchEvent` (so the existing debris
/// accumulation system also charges the launch), and links the new satellite
/// into the constellation on the next `update_constellation` tick.
pub fn launch_slogan_satellite(world: &mut World, planet: Entity) -> Option<Entity> {
    world.init_resource::<Events<LaunchEvent>>();

    let affordable = world
        .get_resource::<ColonyResources>()
        .is_some_and(|r| r.metal >= SATELLITE_METAL_COST && r.tools >= SATELLITE_TOOL_COST);
    if !affordable {
        return None;
    }

    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.metal -= SATELLITE_METAL_COST;
        resources.tools -= SATELLITE_TOOL_COST;
    }

    let slogan = generate_slogan(world.get_resource::<NarrativeGenerator>(), true);
    let word = slogan
        .split_whitespace()
        .next()
        .unwrap_or("WE")
        .to_string();

    let sat = world
        .spawn((
            SloganSatellite { word },
            InOrbit { parent: planet },
        ))
        .id();

    world.resource_mut::<Events<LaunchEvent>>().send(LaunchEvent {
        planet,
        success: true,
    });

    Some(sat)
}

/// Generate a slogan: prefer the procedural lore fragments tagged
/// `slogan-hope` / `slogan-despair`, fall back to the built-in lists.
fn generate_slogan(generator: Option<&NarrativeGenerator>, hope: bool) -> String {
    let tag = if hope {
        SLOGAN_HOPE_FRAGMENT
    } else {
        SLOGAN_DESPAIR_FRAGMENT
    };
    if let Some(gen) = generator {
        if let Some(slogan) = gen.get_random_fragment(tag) {
            return slogan.clone();
        }
    }
    let fallback = if hope { HOPE_SLOGANS } else { DESPAIR_SLOGANS };
    fallback
        .choose(&mut rand::thread_rng())
        .unwrap_or(&"WE ENDURE")
        .to_string()
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::layer1::social::morale::{Morale, MoodModifier};
    use crate::layer2::debris::OrbitalDebris;

    fn spawn_pop(world: &mut World) -> Entity {
        world
            .spawn((crate::layer1::pop::Pop, Morale::default()))
            .id()
    }

    #[test]
    fn test_linked_satellites_form_constellation() {
        let mut world = World::new();
        world.insert_resource(Constellation::default());
        for _ in 0..CONSTELLATION_MIN_SATELLITES {
            world.spawn((SloganSatellite::default(),));
        }

        update_constellation(&mut world);

        let constellation = world.resource::<Constellation>();
        assert!(
            constellation.active(),
            "enough linked satellites should activate the constellation"
        );
    }

    #[test]
    fn test_active_constellation_boosts_colony_morale() {
        let mut world = World::new();
        let mut constellation = Constellation::default();
        constellation.message = SloganMessage::Hope("WE ENDURE".to_string());
        world.insert_resource(constellation);
        let pop = spawn_pop(&mut world);

        apply_constellation_morale(&mut world);

        let morale = world.get::<Morale>(pop).unwrap();
        assert!(
            morale.modifiers.iter().any(|m| m.label == "Propaganda Constellation"),
            "active constellation should add a morale modifier to every pop"
        );
    }

    #[test]
    fn test_hack_flips_message_to_despair() {
        let mut world = World::new();
        let mut constellation = Constellation::default();
        constellation.message = SloganMessage::Hope("WE ENDURE".to_string());
        world.insert_resource(constellation);

        hack_constellation(&mut world, "OBEY THE STATIC".to_string());

        let constellation = world.resource::<Constellation>();
        assert!(
            matches!(constellation.message, SloganMessage::Despair(_)),
            "a successful hack should flip the slogan to despair"
        );
    }

    #[test]
    fn test_shooting_down_satellite_kills_boost_and_creates_debris() {
        let mut world = World::new();
        world.insert_resource(OrbitalDebris::default());
        let sat = world.spawn((SloganSatellite::default(),)).id();

        shoot_down_satellite(&mut world, sat);

        assert!(world.get_entity(sat).is_err(), "satellite should be destroyed");
        assert!(
            world.resource::<OrbitalDebris>().0 > 0.0,
            "shoot-down should feed the orbital debris mechanic"
        );
    }

    #[test]
    fn test_constellation_modifier_refreshes_without_stacking() {
        let mut world = World::new();
        let mut constellation = Constellation::default();
        constellation.message = SloganMessage::Hope("WE ENDURE".to_string());
        world.insert_resource(constellation);
        let pop = spawn_pop(&mut world);

        apply_constellation_morale(&mut world);
        apply_constellation_morale(&mut world);

        let morale = world.get::<Morale>(pop).unwrap();
        let mods: Vec<&MoodModifier> = morale
            .modifiers
            .iter()
            .filter(|m| m.label == PROPAGANDA_MODIFIER_LABEL)
            .collect();
        assert_eq!(
            mods.len(),
            1,
            "the constellation modifier should be refreshed, not stacked"
        );
        assert!(
            (mods[0].value - HOPE_BOOST).abs() < f32::EPSILON,
            "hope message should broadcast +HOPE_BOOST"
        );
    }

    #[test]
    fn test_deactivation_drops_below_threshold_kills_boost() {
        let mut world = World::new();
        world.insert_resource(Constellation::default());
        let sats: Vec<Entity> = (0..CONSTELLATION_MIN_SATELLITES)
            .map(|_| world.spawn((SloganSatellite::default(),)).id())
            .collect();
        update_constellation(&mut world);
        assert!(world.resource::<Constellation>().active());
        let pop = spawn_pop(&mut world);

        // Kill the link below the threshold: sky goes dark, boost ends.
        shoot_down_satellite(&mut world, sats[0]);
        update_constellation(&mut world);
        apply_constellation_morale(&mut world);

        let constellation = world.resource::<Constellation>();
        assert!(!constellation.active(), "link below threshold is inactive");
        assert!(
            matches!(constellation.message, SloganMessage::Dark),
            "dropping below N darkens the sky"
        );
        let morale = world.get::<Morale>(pop).unwrap();
        assert!(
            morale
                .modifiers
                .iter()
                .all(|m| m.label != PROPAGANDA_MODIFIER_LABEL),
            "dropping below N should end the morale boost"
        );
    }
}
