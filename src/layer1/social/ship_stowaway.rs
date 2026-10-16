//! The Stowaway (Spec 327).
//!
//! Every trade ship that docks might carry one passenger too many. On each
//! [`TradeShipArrivalEvent`][crate::layer2::trade::blockade::TradeShipArrivalEvent]
//! a stowaway can slip off the ship and melt into the colony: a real,
//! full-bundle pop carrying a [`Stowaway`] marker with a hidden
//! [`StowawayAgenda`]. The census never counts them (see
//! [`collect_pop_data`]'s `Without<Stowaway>` filter and the headless
//! `pops`/STATS exclusions), but the `ghosts` audit still counts every
//! `Pop` entity — the discrepancy is the deliberate tell.
//!
//! NOTE: this module is deliberately named `ship_stowaway` (not
//! `stowaway`): the legacy `crate::layer1::stowaway` module (an old,
//! unscheduled visitor-infiltration mechanic) owns the `stowaway` module
//! name at the `layer1` level via `pub use social::*`, so a
//! `social::stowaway` module would collide. The [`Stowaway`] component
//! here is likewise only reachable via its full path, never glob
//! re-exported, to avoid an ambiguous-glob collision with the legacy
//! `crate::layer1::Stowaway` type.
//!
//! Agendas: [`StowawayAgenda::Saboteur`] damages buildings through the
//! real [`ExplosionEvent`][crate::layer1::environment::volatile::ExplosionEvent]
//! pipeline; [`StowawayAgenda::Spy`] sends intel packets that eventually
//! trigger a real [`PirateRaidEvent`][crate::layer1::void_weed::PirateRaidEvent];
//! [`StowawayAgenda::Sympathizer`] reveals themselves and asks to belong.
//!
//! Counterplay is the colony muster ([`muster_sweep`]): a census sweep
//! that can expose hidden stowaways. Exposed stowaways are resolved with
//! [`pardon_stowaway`] (they join the colony; a spy defects) or
//! [`expel_stowaway`] (despawned; faction relations take an agenda-scaled
//! hit through the real [`FactionRelations`]).
//!
//! No new `Events<...>` type is introduced — the module reuses
//! `AddChronicleEvent`, `ExplosionEvent`, and `PirateRaidEvent`, all
//! initialized by `simulation.rs` from the start (fresh-world panic class
//! avoided by construction). All names are original and generic; nothing
//! is lifted from outside fiction (see the IP-guard test).

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::architecture::building::Building;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::environment::volatile::ExplosionEvent;
use crate::layer1::pop::{Pop, PopBundle, PopName};
use crate::layer1::social::unrest::{Unrest, UnrestModifier};
use crate::layer1::void_weed::PirateRaidEvent;
use crate::layer2::trade::blockade::TradeShipArrivalEvent;
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;

/// What the stowaway is secretly here to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StowawayAgenda {
    /// Damages colony buildings on a timer.
    Saboteur,
    /// Sends intel packets; enough packets call in a pirate raid.
    Spy,
    /// No malice — reveals themselves after long enough and asks to stay.
    Sympathizer,
}

impl StowawayAgenda {
    /// Human-readable label for console panels and chronicles.
    pub fn label(self) -> &'static str {
        match self {
            StowawayAgenda::Saboteur => "saboteur",
            StowawayAgenda::Spy => "spy",
            StowawayAgenda::Sympathizer => "sympathizer",
        }
    }

    /// Parse a console agenda argument.
    pub fn parse(s: &str) -> Option<StowawayAgenda> {
        match s.to_lowercase().as_str() {
            "saboteur" => Some(StowawayAgenda::Saboteur),
            "spy" => Some(StowawayAgenda::Spy),
            "sympathizer" => Some(StowawayAgenda::Sympathizer),
            _ => None,
        }
    }

    /// Ticks between agenda actions (sympathizers act on a reveal clock).
    pub fn interval(self, config: &StowawayConfig) -> u32 {
        match self {
            StowawayAgenda::Saboteur => config.sabotage_interval,
            StowawayAgenda::Spy => config.intel_interval,
            StowawayAgenda::Sympathizer => u32::MAX,
        }
    }
}

/// Marker component on an unregistered pop hiding in the colony.
#[derive(Component, Debug)]
pub struct Stowaway {
    /// The hidden agenda, rolled at spawn.
    pub agenda: StowawayAgenda,
    /// Trade faction whose ship they slipped off.
    pub home_faction: String,
    /// 0.0..0.5 — reduces muster detection odds.
    pub elusiveness: f32,
    /// Ticks spent hidden in the colony.
    pub ticks_hidden: u32,
    /// True once exposed by a muster or self-reveal.
    pub exposed: bool,
    /// Countdown to the next agenda action.
    pub agenda_timer: u32,
    /// Spy only: intel packets sent toward the raid threshold.
    pub intel_packets: u32,
}

impl Stowaway {
    /// Build a fresh hidden stowaway.
    pub fn new(
        agenda: StowawayAgenda,
        home_faction: &str,
        elusiveness: f32,
        first_interval: u32,
    ) -> Self {
        Self {
            agenda,
            home_faction: home_faction.to_string(),
            elusiveness: elusiveness.clamp(0.0, 0.5),
            ticks_hidden: 0,
            exposed: false,
            agenda_timer: first_interval,
            intel_packets: 0,
        }
    }
}

/// Tuning knobs for the stowaway mechanic.
#[derive(Resource, Debug, Clone)]
pub struct StowawayConfig {
    /// Chance per `TradeShipArrivalEvent` that a stowaway slips off.
    pub arrival_chance: f32,
    /// No new arrivals while this many stowaways hide unexposed.
    pub max_hidden: u32,
    /// Base muster detection chance, scaled by (1 - elusiveness).
    pub muster_base_detection: f32,
    /// Ticks between saboteur strikes.
    pub sabotage_interval: u32,
    /// Ticks between spy intel packets.
    pub intel_interval: u32,
    /// Intel packets that trigger a pirate raid.
    pub intel_raid_threshold: u32,
    /// Ticks hidden before a sympathizer reveals themselves.
    pub sympathizer_reveal_ticks: u32,
    /// Per-tick chance of a "signs" chronicle hint while any stowaway hides.
    pub signs_chance: f32,
}

impl Default for StowawayConfig {
    fn default() -> Self {
        Self {
            // Trade ships arrive ~5%/tick; ~1 stowaway per 1000 ticks.
            arrival_chance: 0.02,
            max_hidden: 3,
            muster_base_detection: 0.60,
            sabotage_interval: 400,
            intel_interval: 300,
            intel_raid_threshold: 3,
            sympathizer_reveal_ticks: 1500,
            signs_chance: 0.002,
        }
    }
}

/// Deterministic FNV-1a-32 hash of a faction name — the colony's informal
/// faction registry, mapping trade-ship faction strings onto the
/// [`FactionRelations`] id space.
pub fn faction_id_for(name: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for b in name.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Flavor pool for the "signs" chronicle hints — original lines, no IP.
const SIGNS_POOL: &[&str] = &[
    "Someone's been in the stores — the count is off again.",
    "A bunk that was empty shows signs of use.",
    "The night watch reports footsteps with no one attached.",
    "Two crew swear they served an extra mouth at mess.",
];

/// Reads `TradeShipArrivalEvent`; occasionally a stowaway slips off the
/// ship and melts into the colony as a full-bundle pop.
pub fn stowaway_arrival_system(
    mut commands: Commands,
    mut arrivals: EventReader<TradeShipArrivalEvent>,
    config: Res<StowawayConfig>,
    stowaways: Query<&Stowaway>,
    pops: Query<&GridPosition, With<Pop>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut rng = rand::thread_rng();
    for arrival in arrivals.read() {
        let hidden = stowaways.iter().filter(|s| !s.exposed).count() as u32;
        if hidden >= config.max_hidden {
            continue;
        }
        if rng.gen::<f32>() >= config.arrival_chance {
            continue;
        }
        // Blend into the crowd: appear at a random living pop's tile.
        let anchors: Vec<&GridPosition> = pops.iter().collect();
        if anchors.is_empty() {
            continue;
        }
        let anchor = anchors[rng.gen_range(0..anchors.len())];
        let agenda = match rng.gen_range(0..3) {
            0 => StowawayAgenda::Saboteur,
            1 => StowawayAgenda::Spy,
            _ => StowawayAgenda::Sympathizer,
        };
        let bundle = PopBundle::random(anchor.x, anchor.y, &mut rng);
        commands.spawn((
            bundle,
            Stowaway::new(
                agenda,
                &arrival.faction,
                rng.gen_range(0.0..0.5),
                agenda.interval(&config),
            ),
        ));
        chronicle.send(AddChronicleEvent {
            text: format!(
                "A dockworker swears someone slipped off the {} trade ship without registering.",
                arrival.faction
            ),
            importance: EventImportance::Minor,
        });
    }
}

/// Damage dealt by a saboteur strike — a malfunction, not a bombing.
const SABOTEUR_DAMAGE: f32 = 10.0;

/// Ticks hidden stowaways: agenda timers, sympathizer self-reveal, and the
/// occasional "signs" chronicle hint.
pub fn stowaway_agenda_system(
    mut explosions: EventWriter<ExplosionEvent>,
    mut raids: EventWriter<PirateRaidEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    config: Res<StowawayConfig>,
    mut stowaways: Query<(Entity, &mut Stowaway, &GridPosition, &PopName)>,
    buildings: Query<(&Building, &GridPosition)>,
) {
    let mut rng = rand::thread_rng();
    let mut any_hidden = false;
    for (_entity, mut stowaway, _pos, _name) in stowaways.iter_mut() {
        if stowaway.exposed {
            continue;
        }
        any_hidden = true;
        stowaway.ticks_hidden += 1;

        let agenda = stowaway.agenda;
        let faction = stowaway.home_faction.clone();

        // Sympathizers run on the hidden clock, not the agenda timer.
        if agenda == StowawayAgenda::Sympathizer
            && stowaway.ticks_hidden >= config.sympathizer_reveal_ticks
        {
            stowaway.exposed = true;
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "A gaunt stranger steps forward: 'I hid aboard the {faction} ship. I only want to belong.'"
                ),
                importance: EventImportance::Major,
            });
            continue;
        }

        if stowaway.agenda_timer > 0 {
            stowaway.agenda_timer -= 1;
        }
        if stowaway.agenda_timer > 0 {
            continue;
        }

        match agenda {
            StowawayAgenda::Saboteur => {
                let targets: Vec<(&Building, i32, i32)> =
                    buildings.iter().map(|(b, p)| (b, p.x, p.y)).collect();
                if !targets.is_empty() {
                    let (building, bx, by) = targets[rng.gen_range(0..targets.len())];
                    explosions.send(ExplosionEvent {
                        center: GridPosition { x: bx, y: by },
                        damage: SABOTEUR_DAMAGE,
                        radius: 1,
                    });
                    chronicle.send(AddChronicleEvent {
                        text: format!(
                            "A strange malfunction rattles the {:?} at ({bx}, {by}).",
                            building.building_type
                        ),
                        importance: EventImportance::Minor,
                    });
                }
                stowaway.agenda_timer = config.sabotage_interval;
            }
            StowawayAgenda::Spy => {
                stowaway.intel_packets += 1;
                if stowaway.intel_packets >= config.intel_raid_threshold {
                    stowaway.intel_packets = 0;
                    raids.send(PirateRaidEvent);
                    chronicle.send(AddChronicleEvent {
                        text: "The raiders came too prepared — someone told them exactly where the stores are."
                            .to_string(),
                        importance: EventImportance::Minor,
                    });
                } else {
                    chronicle.send(AddChronicleEvent {
                        text:
                            "Encrypted static bursts from the comms array — gone before it can be traced."
                                .to_string(),
                        importance: EventImportance::Minor,
                    });
                }
                stowaway.agenda_timer = config.intel_interval;
            }
            StowawayAgenda::Sympathizer => {
                // Handled above via the reveal clock; nothing on the timer.
                stowaway.agenda_timer = u32::MAX;
            }
        }
    }

    if any_hidden && rng.gen::<f32>() < config.signs_chance {
        let hint = SIGNS_POOL[rng.gen_range(0..SIGNS_POOL.len())];
        chronicle.send(AddChronicleEvent {
            text: hint.to_string(),
            importance: EventImportance::Minor,
        });
    }
}

/// Colony-wide census sweep. Each hidden stowaway rolls detection against
/// their elusiveness; the exposed are dragged into the light (Major
/// chronicle + unrest shock). Returns the names of the newly exposed.
pub fn muster_sweep(world: &mut World) -> Vec<String> {
    // Item 87: callable as the FIRST command on a fresh world, before the
    // first tick runs init_simulation_resources — initialize everything
    // read below (the same guard the headless fools/scion handlers apply
    // to their event resources). init_resource never clobbers an
    // existing (possibly scenario-tuned) resource.
    world.init_resource::<StowawayConfig>();
    world.init_resource::<Events<AddChronicleEvent>>();
    world.init_resource::<Unrest>();
    let config = world.resource::<StowawayConfig>().clone();
    let mut rng = rand::thread_rng();
    let candidates: Vec<(Entity, String, f32)> = {
        let mut query = world.query::<(Entity, &Stowaway, &PopName)>();
        query
            .iter(world)
            .filter(|(_, s, _)| !s.exposed)
            .map(|(e, s, n)| (e, n.0.clone(), s.elusiveness))
            .collect()
    };
    let mut exposed_names = Vec::new();
    for (entity, name, elusiveness) in candidates {
        if rng.gen::<f32>() < config.muster_base_detection * (1.0 - elusiveness) {
            if let Some(mut stowaway) = world.get_mut::<Stowaway>(entity) {
                stowaway.exposed = true;
            }
            exposed_names.push(name.clone());
            world
                .resource_mut::<Events<AddChronicleEvent>>()
                .send(AddChronicleEvent {
                    text: format!("The muster drags {name} into the light — a stowaway!"),
                    importance: EventImportance::Major,
                });
        }
    }
    if !exposed_names.is_empty() {
        world.resource_mut::<Unrest>().modifiers.push(UnrestModifier {
            value: 0.15,
            duration: 500,
            label: "Stowaway exposed".to_string(),
        });
    }
    exposed_names
}

/// Read an exposed stowaway's identity, or explain why they can't be
/// resolved yet.
fn resolve_target(world: &mut World, target: Entity) -> Result<(String, String, StowawayAgenda), String> {
    let mut query = world.query::<(&Stowaway, &PopName)>();
    let (stowaway, pop_name) = query.get(world, target).map_err(|_| {
        format!(
            "No stowaway with id {}. List them with `stowaway`.",
            target.index()
        )
    })?;
    if !stowaway.exposed {
        return Err(format!(
            "{} is still hidden — run `muster` to expose them first.",
            pop_name.0
        ));
    }
    Ok((
        pop_name.0.clone(),
        stowaway.home_faction.clone(),
        stowaway.agenda,
    ))
}

/// Pardon an exposed stowaway: they join the colony as a registered pop.
/// A pardoned spy defects, improving relations with their home faction.
/// Returns the pop's name on success.
pub fn pardon_stowaway(world: &mut World, target: Entity) -> Result<String, String> {
    let (name, faction, agenda) = resolve_target(world, target)?;
    world.entity_mut(target).remove::<Stowaway>();
    // A pardoned spy defects, bringing the cipher book.
    if agenda == StowawayAgenda::Spy {
        let faction_id = faction_id_for(&faction);
        let mut relations = world.resource_mut::<FactionRelations>();
        let score = relations.scores.entry(faction_id).or_insert(0);
        *score = (*score + 25).min(100);
    }
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: format!("The colony takes in {name}, late of {faction}."),
            importance: EventImportance::Major,
        });
    Ok(name)
}

/// Relations hit by agenda when an exposed stowaway is expelled.
fn expel_relations_hit(agenda: StowawayAgenda) -> i32 {
    match agenda {
        StowawayAgenda::Saboteur => -50,
        StowawayAgenda::Spy => -60,
        StowawayAgenda::Sympathizer => -40,
    }
}

/// Expel an exposed stowaway: despawned, with an agenda-scaled relations
/// hit against their home faction and an unrest shock. Returns the pop's
/// name on success.
pub fn expel_stowaway(world: &mut World, target: Entity) -> Result<String, String> {
    let (name, faction, agenda) = resolve_target(world, target)?;
    let faction_id = faction_id_for(&faction);
    let mut relations = world.resource_mut::<FactionRelations>();
    let score = relations.scores.entry(faction_id).or_insert(0);
    *score = (*score + expel_relations_hit(agenda)).max(-100);
    world.despawn(target);
    world.resource_mut::<Unrest>().modifiers.push(UnrestModifier {
        value: 0.10,
        duration: 300,
        label: "Stowaway expelled".to_string(),
    });
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: format!(
                "The stowaway {name} is put off the colony — {faction} will hear of this."
            ),
            importance: EventImportance::Major,
        });
    Ok(name)
}

/// STATS hint field: `clear` (none), `signs` (hidden present),
/// `exposed:N` (N exposed, awaiting resolution).
pub fn stowaway_status_label(world: &mut World) -> String {
    let mut hidden = 0u32;
    let mut exposed = 0u32;
    let mut query = world.query::<&Stowaway>();
    for stowaway in query.iter(world) {
        if stowaway.exposed {
            exposed += 1;
        } else {
            hidden += 1;
        }
    }
    if exposed > 0 {
        format!("exposed:{exposed}")
    } else if hidden > 0 {
        "signs".to_string()
    } else {
        "clear".to_string()
    }
}

/// If exactly one stowaway is exposed, return its entity — the console
/// `pardon`/`expel` shortcut when no id is given.
pub fn single_exposed_stowaway(world: &mut World) -> Option<Entity> {
    let mut query = world.query::<(Entity, &Stowaway)>();
    let mut found = None;
    let mut count = 0;
    for (entity, s) in query.iter(world) {
        if s.exposed {
            found = Some(entity);
            count += 1;
        }
    }
    if count == 1 {
        found
    } else {
        None
    }
}

/// Debug helper: spawn a stowaway directly at a random living pop's tile,
/// bypassing the trade-ship arrival roll (console playtesting). Returns the
/// new entity, or `None` if the colony is empty.
pub fn debug_spawn_stowaway(
    world: &mut World,
    agenda_override: Option<StowawayAgenda>,
) -> Option<Entity> {
    // Item 87: first-command guard, as in muster_sweep — StowawayConfig
    // used to exist only after the first tick initialized it.
    world.init_resource::<StowawayConfig>();
    world.init_resource::<Events<AddChronicleEvent>>();
    let config = world.resource::<StowawayConfig>().clone();
    let mut rng = rand::thread_rng();
    let anchors: Vec<(i32, i32)> = {
        let mut query = world.query_filtered::<&GridPosition, With<Pop>>();
        query.iter(world).map(|p| (p.x, p.y)).collect()
    };
    if anchors.is_empty() {
        return None;
    }
    let (x, y) = anchors[rng.gen_range(0..anchors.len())];
    let agenda = agenda_override.unwrap_or_else(|| match rng.gen_range(0..3) {
        0 => StowawayAgenda::Saboteur,
        1 => StowawayAgenda::Spy,
        _ => StowawayAgenda::Sympathizer,
    });
    let bundle = PopBundle::random(x, y, &mut rng);
    let interval = agenda.interval(&config);
    let entity = world
        .spawn((
            bundle,
            Stowaway::new(agenda, "Independent Merchants", rng.gen_range(0.0..0.5), interval),
        ))
        .id();
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: "A dockworker swears someone slipped off the Independent Merchants trade ship without registering."
                .to_string(),
            importance: EventImportance::Minor,
        });
    Some(entity)
}

/// Concatenated public-facing text, for the IP-guard test.
#[cfg(test)]
fn all_public_text() -> String {
    let mut text = SIGNS_POOL.join(" ");
    text.push_str("The muster drags a stowaway into the light. ");
    text.push_str("A dockworker swears someone slipped off the trade ship without registering. ");
    text.push_str("A strange malfunction rattles the building. ");
    text.push_str("Encrypted static bursts from the comms array. ");
    text.push_str("The raiders came too prepared. ");
    text.push_str("A gaunt stranger steps forward. ");
    text.push_str("The colony takes in a stray. ");
    text.push_str("The stowaway is put off the colony. ");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::{App, Update};

    fn test_app() -> App {
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.add_event::<ExplosionEvent>();
        app.add_event::<PirateRaidEvent>();
        app.add_event::<TradeShipArrivalEvent>();
        app.init_resource::<StowawayConfig>();
        app.init_resource::<Unrest>();
        app.insert_resource(FactionRelations::default());
        app
    }

    fn spawn_pop(app: &mut App, x: i32, y: i32) -> Entity {
        let mut rng = rand::thread_rng();
        app.world_mut()
            .spawn(PopBundle::random(x, y, &mut rng))
            .id()
    }

    fn spawn_stowaway(
        app: &mut App,
        agenda: StowawayAgenda,
        exposed: bool,
        elusiveness: f32,
    ) -> Entity {
        let entity = spawn_pop(app, 5, 5);
        let config = app.world().resource::<StowawayConfig>().clone();
        app.world_mut().entity_mut(entity).insert(Stowaway::new(
            agenda,
            "Independent Merchants",
            elusiveness,
            agenda.interval(&config),
        ));
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Stowaway>()
            .unwrap()
            .exposed = exposed;
        entity
    }

    fn chronicle_texts(app: &App) -> Vec<(String, EventImportance)> {
        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        reader
            .read(events)
            .map(|e| (e.text.clone(), e.importance))
            .collect()
    }

    #[test]
    fn test_arrival_spawns_hidden_stowaway() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, stowaway_arrival_system);
        app.world_mut()
            .resource_mut::<StowawayConfig>()
            .arrival_chance = 1.0;
        spawn_pop(&mut app, 10, 10);

        // Act
        app.world_mut()
            .resource_mut::<Events<TradeShipArrivalEvent>>()
            .send(TradeShipArrivalEvent {
                cargo_value: 5000.0,
                faction: "Independent Merchants".to_string(),
            });
        app.update();

        // Assert
        let mut query = app
            .world_mut()
            .query_filtered::<(Entity, &Stowaway), With<Pop>>();
        let found: Vec<_> = query.iter(app.world()).collect();
        assert_eq!(found.len(), 1, "arrival should spawn one hidden stowaway");
        let (_, stowaway) = found[0];
        assert!(!stowaway.exposed, "new stowaway starts hidden");
        assert_eq!(stowaway.home_faction, "Independent Merchants");
        // Full bundle — a complete colonist, never an accidental ghost.
        let entity = found[0].0;
        assert!(app.world().get::<PopName>(entity).is_some());
        assert!(app.world().get::<crate::layer1::biology::health::Health>(entity).is_some());
        assert!(app.world().get::<crate::layer1::psychology::needs::Needs>(entity).is_some());
        // The dockworker saw something.
        let texts = chronicle_texts(&app);
        assert!(
            texts.iter().any(|(t, _)| t.contains("slipped off")),
            "arrival should write a Minor chronicle hint"
        );
    }

    #[test]
    fn test_stowaway_excluded_from_census() {
        // Arrange
        let mut app = test_app();
        spawn_pop(&mut app, 1, 1);
        spawn_stowaway(&mut app, StowawayAgenda::Spy, false, 0.0);

        // Act: the census query the colony uses (mirrors headless STATS).
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, (With<Pop>, Without<Stowaway>)>();

        // Assert
        assert_eq!(
            query.iter(app.world()).count(),
            1,
            "census counts registered pops only"
        );
    }

    #[test]
    fn test_muster_exposes_stowaway() {
        // Arrange
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<StowawayConfig>()
            .muster_base_detection = 1.0;
        spawn_stowaway(&mut app, StowawayAgenda::Saboteur, false, 0.0);

        // Act
        let exposed = muster_sweep(app.world_mut());

        // Assert
        assert_eq!(exposed.len(), 1, "muster should expose the stowaway");
        let mut query = app.world_mut().query::<&Stowaway>();
        assert!(
            query.iter(app.world()).next().unwrap().exposed,
            "stowaway marked exposed"
        );
        let texts = chronicle_texts(&app);
        assert!(
            texts
                .iter()
                .any(|(t, i)| t.contains("into the light") && *i == EventImportance::Major),
            "exposure writes a Major chronicle"
        );
        let unrest = app.world().resource::<Unrest>();
        assert!(
            unrest.modifiers.iter().any(|m| m.label == "Stowaway exposed"),
            "exposure shocks unrest"
        );
    }

    #[test]
    fn test_muster_can_miss() {
        // Arrange
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<StowawayConfig>()
            .muster_base_detection = 0.0;
        spawn_stowaway(&mut app, StowawayAgenda::Spy, false, 0.0);

        // Act
        let exposed = muster_sweep(app.world_mut());

        // Assert
        assert!(exposed.is_empty(), "a failed muster exposes nobody");
        let mut query = app.world_mut().query::<&Stowaway>();
        assert!(
            !query.iter(app.world()).next().unwrap().exposed,
            "stowaway stays hidden"
        );
    }

    #[test]
    fn test_saboteur_damages_building() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, stowaway_agenda_system);
        app.world_mut().spawn((
            Building::default(),
            GridPosition { x: 3, y: 3 },
        ));
        let entity = spawn_stowaway(&mut app, StowawayAgenda::Saboteur, false, 0.0);
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Stowaway>()
            .unwrap()
            .agenda_timer = 1;

        // Act
        app.update();

        // Assert
        let events = app.world().resource::<Events<ExplosionEvent>>();
        let mut reader = events.get_cursor();
        let fired: Vec<_> = reader.read(events).collect();
        assert!(!fired.is_empty(), "saboteur should fire an ExplosionEvent");
        assert!(
            fired.iter().any(|e| e.center.x == 3 && e.center.y == 3),
            "explosion centered on the building"
        );
        let texts = chronicle_texts(&app);
        assert!(
            texts.iter().any(|(t, _)| t.contains("strange malfunction")),
            "sabotage writes a chronicle hint"
        );
    }

    #[test]
    fn test_spy_triggers_raid_at_threshold() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, stowaway_agenda_system);
        let threshold = app
            .world()
            .resource::<StowawayConfig>()
            .intel_raid_threshold;
        let entity = spawn_stowaway(&mut app, StowawayAgenda::Spy, false, 0.0);
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Stowaway>()
            .unwrap()
            .intel_packets = threshold - 1;
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Stowaway>()
            .unwrap()
            .agenda_timer = 1;

        // Act
        app.update();

        // Assert
        let events = app.world().resource::<Events<PirateRaidEvent>>();
        let mut reader = events.get_cursor();
        assert!(
            reader.read(events).count() > 0,
            "spy at the intel threshold should trigger a raid"
        );
        let packets = app
            .world()
            .get::<Stowaway>(entity)
            .unwrap()
            .intel_packets;
        assert_eq!(packets, 0, "intel packets reset after the raid call");
    }

    #[test]
    fn test_sympathizer_self_reveals() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, stowaway_agenda_system);
        let reveal_at = app
            .world()
            .resource::<StowawayConfig>()
            .sympathizer_reveal_ticks;
        let entity = spawn_stowaway(&mut app, StowawayAgenda::Sympathizer, false, 0.0);
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Stowaway>()
            .unwrap()
            .ticks_hidden = reveal_at;

        // Act
        app.update();

        // Assert
        assert!(
            app.world().get::<Stowaway>(entity).unwrap().exposed,
            "sympathizer reveals themselves"
        );
        let texts = chronicle_texts(&app);
        assert!(
            texts
                .iter()
                .any(|(t, i)| t.contains("steps forward") && *i == EventImportance::Major),
            "self-reveal writes a Major chronicle"
        );
    }

    #[test]
    fn test_pardon_registers_pop() {
        // Arrange
        let mut app = test_app();
        let entity = spawn_stowaway(&mut app, StowawayAgenda::Sympathizer, true, 0.0);

        // Act
        let result = pardon_stowaway(app.world_mut(), entity);

        // Assert
        assert!(result.is_ok(), "pardon should succeed on an exposed stowaway");
        assert!(
            app.world().get::<Stowaway>(entity).is_none(),
            "Stowaway marker removed"
        );
        assert!(
            app.world().get::<Pop>(entity).is_some(),
            "pardoned pop is a registered colonist"
        );
        let texts = chronicle_texts(&app);
        assert!(
            texts.iter().any(|(t, i)| t.contains("takes in") && *i == EventImportance::Major),
            "pardon writes a Major chronicle"
        );
    }

    #[test]
    fn test_pardon_requires_exposure() {
        // Arrange
        let mut app = test_app();
        let entity = spawn_stowaway(&mut app, StowawayAgenda::Spy, false, 0.0);

        // Act
        let result = pardon_stowaway(app.world_mut(), entity);

        // Assert
        assert!(result.is_err(), "cannot pardon a hidden stowaway");
    }

    #[test]
    fn test_expel_hits_faction_relations() {
        // Arrange
        let mut app = test_app();
        let entity = spawn_stowaway(&mut app, StowawayAgenda::Spy, true, 0.0);
        let faction_id = faction_id_for("Independent Merchants");

        // Act
        let result = expel_stowaway(app.world_mut(), entity);

        // Assert
        assert!(result.is_ok(), "expel should succeed on an exposed stowaway");
        let mut pops = app.world_mut().query_filtered::<Entity, With<Pop>>();
        assert_eq!(
            pops.iter(app.world()).count(),
            0,
            "expelled stowaway is gone"
        );
        let relations = app.world().resource::<FactionRelations>();
        assert_eq!(
            relations.scores.get(&faction_id),
            Some(&-60),
            "expelling a spy hits relations by -60"
        );
        let unrest = app.world().resource::<Unrest>();
        assert!(
            unrest.modifiers.iter().any(|m| m.label == "Stowaway expelled"),
            "expulsion unsettles the colony"
        );
    }

    #[test]
    fn test_signs_hint_chronicle() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, stowaway_agenda_system);
        app.world_mut().resource_mut::<StowawayConfig>().signs_chance = 1.0;
        spawn_stowaway(&mut app, StowawayAgenda::Saboteur, false, 0.0);

        // Act
        app.update();

        // Assert
        let texts = chronicle_texts(&app);
        assert!(
            texts
                .iter()
                .any(|(t, i)| *i == EventImportance::Minor
                    && (t.contains("stores") || t.contains("bunk") || t.contains("footsteps") || t.contains("mess"))),
            "a hidden stowaway leaves signs"
        );
    }

    #[test]
    fn test_stowaway_status_label() {
        // Arrange
        let mut app = test_app();

        // Act / Assert: clear
        assert_eq!(stowaway_status_label(app.world_mut()), "clear");

        // Hidden -> signs
        spawn_stowaway(&mut app, StowawayAgenda::Spy, false, 0.0);
        assert_eq!(stowaway_status_label(app.world_mut()), "signs");

        // Exposed -> exposed:N
        let entity = spawn_stowaway(&mut app, StowawayAgenda::Saboteur, false, 0.0);
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Stowaway>()
            .unwrap()
            .exposed = true;
        assert_eq!(stowaway_status_label(app.world_mut()), "exposed:1");
    }

    #[test]
    fn test_debug_spawn_creates_stowaway() {
        // Arrange
        let mut app = test_app();
        spawn_pop(&mut app, 5, 5);

        // Act
        let entity = debug_spawn_stowaway(app.world_mut(), Some(StowawayAgenda::Spy))
            .expect("spawn should succeed with a living colony");

        // Assert
        let s = app.world().get::<Stowaway>(entity).unwrap();
        assert_eq!(s.agenda, StowawayAgenda::Spy);
        assert!(!s.exposed, "debug spawn starts hidden");
        assert_eq!(stowaway_status_label(app.world_mut()), "signs");
    }

    #[test]
    fn test_debug_spawn_empty_colony() {
        // Arrange
        let mut app = test_app();

        // Act / Assert
        assert!(
            debug_spawn_stowaway(app.world_mut(), None).is_none(),
            "no living pops to blend with — no spawn"
        );
    }

    /// Item 87 regression: a world in the exact state `setup_world_with_config`
    /// leaves it before the first simulation tick — the setup-provided
    /// dependencies (chronicle events, unrest, faction relations) exist, but
    /// `StowawayConfig` does NOT (it used to be created only by
    /// `init_simulation_resources` on the first tick). The headless
    /// `stowaway spawn` / `muster` commands dispatch into these helpers
    /// before any tick runs, so they must not panic here.
    fn fresh_world_without_config() -> World {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Unrest>();
        world.insert_resource(FactionRelations::default());
        // Deliberately NO StowawayConfig — see above.
        let mut rng = rand::thread_rng();
        world.spawn(PopBundle::random(5, 5, &mut rng));
        world
    }

    #[test]
    fn test_debug_spawn_stowaway_first_command_on_fresh_world() {
        // Arrange
        let mut world = fresh_world_without_config();
        assert!(!world.contains_resource::<StowawayConfig>());

        // Act — the very first headless command on a fresh world.
        let entity = debug_spawn_stowaway(&mut world, Some(StowawayAgenda::Spy));

        // Assert
        let entity = entity.expect("spawn should succeed with a living colony");
        assert!(world.get::<Stowaway>(entity).is_some());
        assert!(world.contains_resource::<StowawayConfig>());
    }

    #[test]
    fn test_muster_sweep_first_command_on_fresh_world() {
        // Arrange
        let mut world = fresh_world_without_config();

        // Act — muster with nobody hidden, then after a debug spawn.
        let exposed_empty = muster_sweep(&mut world);
        debug_spawn_stowaway(&mut world, None).expect("spawn should succeed");
        let _exposed_after_spawn = muster_sweep(&mut world);

        // Assert — no panic either way; the empty sweep exposes nobody.
        assert!(exposed_empty.is_empty());
        assert!(world.contains_resource::<StowawayConfig>());
    }

    #[test]
    fn test_stowaway_status_label_first_command_on_fresh_world() {
        // Arrange
        let mut world = fresh_world_without_config();

        // Act / Assert — the panel / STATS path is query-only and must
        // stay panic-free on a fresh world.
        assert_eq!(stowaway_status_label(&mut world), "clear");
    }

    #[test]
    fn test_first_command_debug_spawn_on_bare_world() {
        // Item 87: `stowaway spawn` as the FIRST headless command on a
        // fresh world panicked — StowawayConfig was only initialized by
        // init_simulation_resources on the first tick, and this helper
        // read it unconditionally. A bare World (no config, no chronicle
        // events, no tick ever run) must be enough.
        let mut world = World::new();
        let mut rng = rand::thread_rng();
        world.spawn(PopBundle::random(5, 5, &mut rng));

        // Act
        let spawned = debug_spawn_stowaway(&mut world, Some(StowawayAgenda::Spy));

        // Assert
        assert!(
            spawned.is_some(),
            "debug spawn must succeed as the first command on a fresh world"
        );
        assert_eq!(stowaway_status_label(&mut world), "signs");
    }

    #[test]
    fn test_first_command_muster_on_bare_world() {
        // Item 87: `muster` as the first command hit the same panic at
        // the same StowawayConfig read, even with no stowaways present.
        let mut world = World::new();

        // Act / Assert (must not panic)
        let exposed = muster_sweep(&mut world);
        assert!(exposed.is_empty());
        assert_eq!(stowaway_status_label(&mut world), "clear");
    }

    #[test]
    fn test_first_command_on_setup_world_before_any_tick() {
        // Item 87, end-to-end: a world from setup_world_with_config (the
        // headless entry path) must serve stowaway commands before the
        // first simulation tick runs init_simulation_resources.
        let mut world = crate::setup::setup_world_with_config(crate::setup::SetupConfig {
            headless: true,
            ..Default::default()
        });
        assert!(
            world.contains_resource::<StowawayConfig>(),
            "setup must initialize StowawayConfig before any tick/command"
        );

        // Act (no tick has run — these are the first commands)
        let spawned = debug_spawn_stowaway(&mut world, None);
        assert!(
            spawned.is_some(),
            "a setup world starts with living pops to blend with"
        );
        let _ = muster_sweep(&mut world);
        assert!(!stowaway_status_label(&mut world).is_empty());
    }

    #[test]
    fn test_agenda_parse() {
        assert_eq!(
            StowawayAgenda::parse("saboteur"),
            Some(StowawayAgenda::Saboteur)
        );
        assert_eq!(StowawayAgenda::parse("SPY"), Some(StowawayAgenda::Spy));
        assert_eq!(
            StowawayAgenda::parse("sympathizer"),
            Some(StowawayAgenda::Sympathizer)
        );
        assert_eq!(StowawayAgenda::parse("pirate"), None);
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // All public-facing text must be original — no drifting into
        // other fiction's distinctive vocabulary.
        let banned = [
            "vogon",
            "heart of gold",
            "ford prefect",
            "zaphod",
            "babel fish",
            "pan galactic",
            "mugwump",
        ];
        let text = all_public_text().to_lowercase();
        for term in banned {
            assert!(
                !text.contains(term),
                "public text must not contain banned term '{term}'"
            );
        }
        for agenda in [
            StowawayAgenda::Saboteur,
            StowawayAgenda::Spy,
            StowawayAgenda::Sympathizer,
        ] {
            let label = agenda.label().to_lowercase();
            for term in banned {
                assert!(
                    !label.contains(term),
                    "agenda label must not contain banned term '{term}'"
                );
            }
        }
    }
}
