//! The Martyrdom Effect (Spec 272).
//!
//! A single death can ignite a holy war. When a pop of high prestige or
//! leadership — a [`Leader`][crate::layer1::leader_ascension::Leader], an
//! [`AscensionCandidate`], an [`Officer`][crate::layer1::chain_of_command::Officer],
//! a [`NobleScion`][crate::layer1::social::cadet::NobleScion], a prophet or
//! noble, or a master of any skill (level 10+) — is killed by an enemy in a
//! highly visible area, the colony erupts:
//!
//! * Unrest is zeroed for [`MARTYRDOM_DURATION_TICKS`] ticks (via a
//!   colony-wide [`UnrestModifier`][crate::layer1::unrest::UnrestModifier],
//!   the real unrest mechanism).
//! * Every living pop works at maximum speed (a
//!   [`WorkSpeedBuff`][crate::layer1::agriculture::gastronomy::WorkSpeedBuff]
//!   at the game's cap, the same buff a HighEnergy meal grants).
//! * The colony gains an [`IdeologicalCasusBelli`] against the offending
//!   faction on layer 3: relations with the offender collapse to Hostile
//!   (-100) in [`FactionRelations`], and neutral third parties rally to the
//!   colony's side (+[`MARTYRDOM_RALLY_BOOST`]).
//! * A Major chronicle entry names the martyr.
//!
//! ## Design notes (adaptations of the spec's mock-style RED tests)
//!
//! * The spec's `DeathEvent`/`DamageSource` don't exist; the real death
//!   event is [`PopDied`][crate::layer1::pop::PopDied] with a free-text
//!   `reason`. Enemy attribution therefore works two ways:
//!   1. An explicit [`SlainByFaction`] marker on the victim (inserted by
//!      enemy kill systems, or by the headless `martyr` debug command).
//!   2. A raid-window bridge: if a [`PirateRaidEvent`] fired within
//!      [`MARTYRDOM_RAID_WINDOW_TICKS`] ticks and the death wasn't plainly
//!      natural (`reason == "Old Age"`), the offending faction is blamed.
//!      The spec's REFACTOR phase explicitly embraces this ambiguity —
//!      orchestrating a "convenient" martyrdom is the intended tension.
//! * The spec's `Unrest`-on-`Pop` / `Leadership` components don't exist;
//!   unrest is the global [`Unrest`][crate::layer1::unrest::Unrest] resource
//!   and leadership is the marker/trait/skill set above.
//! * "Highly visible area" uses real colony mechanisms: within
//!   [`MARTYRDOM_VISIBILITY_RADIUS`] tiles (Chebyshev) of the colony heart
//!   (mean position of living pops) or of any [`Tavern`] — the social hub.
//! * All names are original and generic; nothing is lifted from outside
//!   fiction (see the IP-guard test).

use bevy_ecs::prelude::*;
use std::collections::HashSet;

use crate::layer1::agriculture::gastronomy::WorkSpeedBuff;
use crate::layer1::chain_of_command::Officer;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::leader_ascension::{AscensionCandidate, Leader};
use crate::layer1::pop::{Pop, PopDied};
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::skills::Skills;
use crate::layer1::social::cadet::NobleScion;
use crate::layer1::social::Tavern;
use crate::layer1::unrest::{Unrest, UnrestModifier};
use crate::layer1::void_weed::PirateRaidEvent;
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::time::SimulationTime;

/// How long the martyrdom fervor lasts, in ticks (~2 in-game months).
pub const MARTYRDOM_DURATION_TICKS: u32 = 2000;
/// Work-speed multiplier during martyrdom: the game's cap (HighEnergy meal level).
pub const MARTYRDOM_WORK_SPEED: f32 = 1.5;
/// A leader dying within this many ticks of a pirate raid is blamed on the raiders.
pub const MARTYRDOM_RAID_WINDOW_TICKS: u64 = 150;
/// "Highly visible": Chebyshev tiles from the colony heart or a tavern.
pub const MARTYRDOM_VISIBILITY_RADIUS: i32 = 12;
/// Skill XP for level 10 — the same bar as ascension eligibility (prestige via mastery).
pub const LEADERSHIP_PRESTIGE_XP: f32 = 10_000.0;
/// Layer-3 faction id of the pirate raiders.
pub const PIRATE_FACTION_ID: u32 = 13;
/// Relation boost granted to neutral third parties (rallying them to the colony).
pub const MARTYRDOM_RALLY_BOOST: i32 = 20;
/// Factions with |score| <= this count as neutral for rally purposes.
pub const MARTYRDOM_NEUTRAL_BAND: i32 = 25;
/// Death reasons that can never be martyrdom (plainly natural causes).
pub const NATURAL_DEATH_REASON: &str = "Old Age";

/// Marker: this pop was killed by an enemy faction's action.
///
/// Inserted by enemy kill systems (or the headless `martyr` debug command);
/// the martyrdom system reads it off the corpse before despawn.
#[derive(Component, Debug, Clone, Copy)]
pub struct SlainByFaction {
    /// Layer-3 faction id of the killer, per [`FactionRelations`].
    pub faction_id: u32,
}

/// Last recorded enemy strike against the colony (pirate raid bridge).
#[derive(Resource, Default, Debug, Clone)]
pub struct LastEnemyStrike {
    /// Offending faction id, if any strike has been recorded.
    pub faction_id: Option<u32>,
    /// Tick the strike happened on.
    pub tick: u64,
}

/// Active martyrdom fervor: unrest silenced, work redoubled.
#[derive(Resource, Default, Debug, Clone)]
pub struct Martyrdom {
    /// Ticks of fervor remaining (0 = inactive).
    pub ticks_remaining: u32,
    /// Faction the casus belli is held against.
    pub faction_id: Option<u32>,
}

/// The colony's ideological justification for war against a faction.
///
/// Set when martyrdom triggers; read by layer-3 diplomacy. Expires with
/// the fervor.
#[derive(Resource, Default, Debug, Clone)]
pub struct IdeologicalCasusBelli {
    /// Faction the colony holds the casus belli against.
    pub faction_id: Option<u32>,
    /// Tick the justification expires.
    pub expires_tick: u64,
}

/// Generic layer-3 faction name for chronicles. Original names only.
#[must_use]
pub fn faction_name(faction_id: u32) -> &'static str {
    match faction_id {
        PIRATE_FACTION_ID => "the pirate raiders",
        7 => "the Homeworld",
        _ => "an unknown faction",
    }
}

/// Pure: does this pop count as a leader figure (prestige or leadership)?
///
/// Mirrors the component checks in [`handle_martyrdom_system`]; kept pure
/// so tests can exercise the rule without a world.
#[must_use]
pub fn is_leader_figure(
    is_leader: bool,
    is_candidate: bool,
    is_officer: bool,
    is_scion: bool,
    traits: Option<&Traits>,
    skills: Option<&Skills>,
) -> bool {
    if is_leader || is_candidate || is_officer || is_scion {
        return true;
    }
    if let Some(traits) = traits {
        if traits.has(Trait::Prophet) || traits.has(Trait::Noble) {
            return true;
        }
    }
    if let Some(skills) = skills {
        if skills.xp.values().any(|&xp| xp >= LEADERSHIP_PRESTIGE_XP) {
            return true;
        }
    }
    false
}

/// Pure: attribute a death to an enemy faction, if any.
///
/// Explicit [`SlainByFaction`] markers win; otherwise a recent enemy strike
/// (within [`MARTYRDOM_RAID_WINDOW_TICKS`]) blames the striker, unless the
/// death was plainly natural.
#[must_use]
pub fn enemy_attribution(
    slain_by: Option<u32>,
    strike: Option<(u32, u64)>,
    tick: u64,
    reason: &str,
) -> Option<u32> {
    if let Some(faction_id) = slain_by {
        return Some(faction_id);
    }
    if reason == NATURAL_DEATH_REASON {
        return None;
    }
    if let Some((faction_id, strike_tick)) = strike {
        if tick.saturating_sub(strike_tick) <= MARTYRDOM_RAID_WINDOW_TICKS {
            return Some(faction_id);
        }
    }
    None
}

/// Pure: is the death tile highly visible?
///
/// Visible when within [`MARTYRDOM_VISIBILITY_RADIUS`] (Chebyshev) of the
/// colony heart or of any tavern position.
#[must_use]
pub fn is_death_visible(
    death_pos: GridPosition,
    colony_heart: Option<GridPosition>,
    tavern_positions: &[GridPosition],
) -> bool {
    let within = |a: GridPosition, b: GridPosition| {
        (a.x - b.x).abs().max((a.y - b.y).abs()) <= MARTYRDOM_VISIBILITY_RADIUS
    };
    if let Some(heart) = colony_heart {
        if within(death_pos, heart) {
            return true;
        }
    }
    tavern_positions.iter().any(|&t| within(death_pos, t))
}

/// Mean position of living pops: the colony heart. `None` when empty.
fn colony_heart(positions: &[GridPosition]) -> Option<GridPosition> {
    if positions.is_empty() {
        return None;
    }
    let (sx, sy) = positions
        .iter()
        .fold((0i64, 0i64), |(x, y), p| (x + p.x as i64, y + p.y as i64));
    #[allow(clippy::cast_possible_truncation)]
    Some(GridPosition {
        x: (sx / positions.len() as i64) as i32,
        y: (sy / positions.len() as i64) as i32,
    })
}

/// Records pirate raids as enemy strikes for martyrdom attribution.
pub fn record_enemy_strike_system(
    mut events: EventReader<PirateRaidEvent>,
    mut strike: ResMut<LastEnemyStrike>,
    time: Option<Res<SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    for _ in events.read() {
        strike.faction_id = Some(PIRATE_FACTION_ID);
        strike.tick = tick;
    }
}

/// Leadership/attribution components read off a fresh corpse.
#[derive(bevy_ecs::query::QueryData)]
pub struct MartyrFigure {
    leader: Option<&'static Leader>,
    candidate: Option<&'static AscensionCandidate>,
    officer: Option<&'static Officer>,
    scion: Option<&'static NobleScion>,
    traits: Option<&'static Traits>,
    skills: Option<&'static Skills>,
    pos: Option<&'static GridPosition>,
    slain: Option<&'static SlainByFaction>,
}

/// Watches for leader-figure deaths by enemy hands and ignites martyrdom.
///
/// Must run after `handle_pop_death_system` (which emits [`PopDied`]) and
/// before `despawn_dead_entities_system` (the corpse's components are read
/// here).
#[allow(clippy::too_many_arguments)]
pub fn handle_martyrdom_system(
    mut commands: Commands,
    mut events: EventReader<PopDied>,
    figures: Query<MartyrFigure>,
    living: Query<(Entity, &GridPosition), With<Pop>>,
    taverns: Query<&GridPosition, With<Tavern>>,
    mut unrest: ResMut<Unrest>,
    mut relations: ResMut<FactionRelations>,
    mut martyrdom: ResMut<Martyrdom>,
    mut casus: ResMut<IdeologicalCasusBelli>,
    strike: Option<Res<LastEnemyStrike>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    time: Option<Res<SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    let strike_info = strike
        .as_ref()
        .and_then(|s| s.faction_id.map(|f| (f, s.tick)));
    let tavern_positions: Vec<GridPosition> = taverns.iter().copied().collect();
    // Collect first: the reader cannot be borrowed again inside the loop.
    let died: Vec<PopDied> = events.read().cloned().collect();
    let dead: HashSet<Entity> = died.iter().map(|e| e.entity).collect();

    for event in &died {
        let Ok(fig) = figures.get(event.entity) else {
            continue;
        };
        if !is_leader_figure(
            fig.leader.is_some(),
            fig.candidate.is_some(),
            fig.officer.is_some(),
            fig.scion.is_some(),
            fig.traits,
            fig.skills,
        ) {
            continue;
        }
        let Some(faction_id) = enemy_attribution(
            fig.slain.map(|s| s.faction_id),
            strike_info,
            tick,
            &event.reason,
        ) else {
            continue;
        };
        // Visibility: the colony heart excludes the freshly dead.
        let heart_positions: Vec<GridPosition> = living
            .iter()
            .filter(|(e, _)| !dead.contains(e))
            .map(|(_, p)| *p)
            .collect();
        let death_pos = match fig.pos.copied() {
            Some(p) => p,
            None => continue,
        };
        if !is_death_visible(death_pos, colony_heart(&heart_positions), &tavern_positions) {
            continue;
        }

        // --- MARTYRDOM ---
        martyrdom.ticks_remaining = MARTYRDOM_DURATION_TICKS;
        martyrdom.faction_id = Some(faction_id);
        casus.faction_id = Some(faction_id);
        casus.expires_tick = tick + u64::from(MARTYRDOM_DURATION_TICKS);

        // Zero unrest for the duration (real mechanism: a timed modifier).
        unrest.modifiers.push(UnrestModifier {
            value: -1.0,
            duration: MARTYRDOM_DURATION_TICKS,
            label: "Martyrdom".to_string(),
        });

        // Maximize work speed for every living pop (game's cap buff).
        for (entity, _) in living.iter() {
            if dead.contains(&entity) {
                continue;
            }
            commands.entity(entity).insert(WorkSpeedBuff {
                multiplier: MARTYRDOM_WORK_SPEED,
                duration: MARTYRDOM_DURATION_TICKS,
            });
        }

        // Ideological casus belli: offender goes Hostile, neutrals rally.
        relations.scores.insert(faction_id, -100);
        for score in relations.scores.values_mut() {
            if score.abs() <= MARTYRDOM_NEUTRAL_BAND {
                *score = (*score + MARTYRDOM_RALLY_BOOST).min(100);
            }
        }

        chronicle.send(AddChronicleEvent {
            text: format!(
                "MARTYRDOM: {} has fallen to {}! The colony burns with righteous fury — unrest silenced, work redoubled, and {} will answer for this.",
                event.name,
                faction_name(faction_id),
                faction_name(faction_id),
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Ticks down active martyrdom; clears the casus belli on expiry.
pub fn tick_martyrdom_system(
    mut martyrdom: ResMut<Martyrdom>,
    mut casus: ResMut<IdeologicalCasusBelli>,
    time: Option<Res<SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    if martyrdom.ticks_remaining > 0 {
        martyrdom.ticks_remaining -= 1;
        if martyrdom.ticks_remaining == 0 {
            martyrdom.faction_id = None;
        }
    }
    if let Some(faction_id) = casus.faction_id {
        if tick >= casus.expires_tick {
            let _ = faction_id;
            casus.faction_id = None;
        }
    }
}

/// Headless/debug helper: mark a pop as a leader figure if it isn't one
/// already. Returns true when it already was.
pub fn ensure_leader_figure(world: &mut World, entity: Entity) -> bool {
    let already = world.get::<Leader>(entity).is_some()
        || world.get::<AscensionCandidate>(entity).is_some()
        || world.get::<Officer>(entity).is_some()
        || world.get::<NobleScion>(entity).is_some()
        || world
            .get::<Traits>(entity)
            .is_some_and(|t| t.has(Trait::Prophet) || t.has(Trait::Noble))
        || world
            .get::<Skills>(entity)
            .is_some_and(|s| s.xp.values().any(|&xp| xp >= LEADERSHIP_PRESTIGE_XP));
    if !already {
        world.entity_mut(entity).insert(AscensionCandidate);
    }
    already
}

/// Ticks of martyrdom fervor remaining (headless STATS).
pub fn martyrdom_ticks_remaining(world: &mut World) -> u32 {
    world
        .get_resource::<Martyrdom>()
        .map_or(0, |m| m.ticks_remaining)
}

/// Public-facing strings, for the IP-guard test.
pub fn martyrdom_public_strings() -> Vec<String> {
    vec![
        "MARTYRDOM: a leader has fallen to the pirate raiders! The colony burns with righteous fury.".to_string(),
        "The colony holds an ideological casus belli against the pirate raiders.".to_string(),
        faction_name(PIRATE_FACTION_ID).to_string(),
        faction_name(7).to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::pop::PopName;
    use crate::layer1::biology::health::Health;

    fn setup() -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Unrest::default());
        world.insert_resource(FactionRelations::default());
        world.insert_resource(Martyrdom::default());
        world.insert_resource(LastEnemyStrike::default());
        world.insert_resource(IdeologicalCasusBelli::default());
        world.insert_resource(Events::<PopDied>::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(Events::<PirateRaidEvent>::default());
        world
    }

    fn spawn_pop(world: &mut World, name: &str, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Pop,
                PopName(name.to_string()),
                GridPosition { x, y },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                Traits::default(),
                Skills::default(),
            ))
            .id()
    }

    fn kill(world: &mut World, entity: Entity, name: &str, reason: &str) {
        world.send_event(PopDied {
            entity,
            name: name.to_string(),
            tick: 100,
            reason: reason.to_string(),
        });
    }

    fn run_martyrdom(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_martyrdom_system);
        schedule.run(world);
    }

    fn chronicle_texts(world: &mut World) -> Vec<String> {
        let events = world.resource::<Events<AddChronicleEvent>>();
        events
            .get_cursor()
            .read(events)
            .map(|e| e.text.clone())
            .collect()
    }

    // --- Spec 272 RED phase, adapted to the real types ---

    #[test]
    fn test_leader_death_by_enemy_triggers_martyrdom_effect() {
        let mut world = setup();
        // Colony heart near (5,5).
        spawn_pop(&mut world, "Witness One", 5, 5);
        spawn_pop(&mut world, "Witness Two", 6, 4);
        let leader = spawn_pop(&mut world, "Beloved Leader", 6, 6);
        world.entity_mut(leader).insert(AscensionCandidate);
        world.entity_mut(leader).insert(SlainByFaction {
            faction_id: PIRATE_FACTION_ID,
        });

        kill(&mut world, leader, "Beloved Leader", "Pirate raid");
        run_martyrdom(&mut world);

        let martyrdom = world.resource::<Martyrdom>();
        assert_eq!(martyrdom.ticks_remaining, MARTYRDOM_DURATION_TICKS);
        assert_eq!(martyrdom.faction_id, Some(PIRATE_FACTION_ID));

        // Unrest is zeroed for the duration via a timed modifier.
        let unrest = world.resource::<Unrest>();
        assert!(unrest
            .modifiers
            .iter()
            .any(|m| m.label == "Martyrdom" && m.value == -1.0));

        // Ideological casus belli: offender Hostile, neutrals rallied.
        let relations = world.resource::<FactionRelations>();
        assert_eq!(relations.scores[&PIRATE_FACTION_ID], -100);
        let casus = world.resource::<IdeologicalCasusBelli>();
        assert_eq!(casus.faction_id, Some(PIRATE_FACTION_ID));

        // Work speed maximized for the living.
        let mut living_buffed = 0;
        let mut query = world.query::<(Entity, &Pop, Option<&WorkSpeedBuff>)>();
        for (_, _, buff) in query.iter(&world) {
            if let Some(b) = buff {
                assert_eq!(b.multiplier, MARTYRDOM_WORK_SPEED);
                living_buffed += 1;
            }
        }
        assert_eq!(living_buffed, 2);

        // Chronicle names the martyr.
        let texts = chronicle_texts(&mut world);
        assert!(texts.iter().any(|t| t.contains("MARTYRDOM") && t.contains("Beloved Leader")));
    }

    #[test]
    fn test_normal_pop_death_does_not_trigger_martyrdom() {
        let mut world = setup();
        spawn_pop(&mut world, "Witness One", 5, 5);
        let normal = spawn_pop(&mut world, "Ordinary Pop", 6, 6);
        world.entity_mut(normal).insert(SlainByFaction {
            faction_id: PIRATE_FACTION_ID,
        });

        kill(&mut world, normal, "Ordinary Pop", "Pirate raid");
        run_martyrdom(&mut world);

        let martyrdom = world.resource::<Martyrdom>();
        assert_eq!(martyrdom.ticks_remaining, 0);
        assert_eq!(martyrdom.faction_id, None);
        let casus = world.resource::<IdeologicalCasusBelli>();
        assert_eq!(casus.faction_id, None);
        assert!(chronicle_texts(&mut world).is_empty());
    }

    #[test]
    fn test_leader_natural_death_does_not_trigger_martyrdom() {
        let mut world = setup();
        spawn_pop(&mut world, "Witness One", 5, 5);
        let leader = spawn_pop(&mut world, "Old Leader", 6, 6);
        world.entity_mut(leader).insert(Leader);
        // A raid happened recently — but old age is old age.
        world.resource_mut::<LastEnemyStrike>().faction_id = Some(PIRATE_FACTION_ID);
        world.resource_mut::<LastEnemyStrike>().tick = 50;
        world.resource_mut::<SimulationTime>().tick = 100;

        kill(&mut world, leader, "Old Leader", NATURAL_DEATH_REASON);
        run_martyrdom(&mut world);

        assert_eq!(world.resource::<Martyrdom>().ticks_remaining, 0);
        assert!(chronicle_texts(&mut world).is_empty());
    }

    #[test]
    fn test_leader_death_after_raid_triggers_via_window() {
        let mut world = setup();
        spawn_pop(&mut world, "Witness One", 5, 5);
        let leader = spawn_pop(&mut world, "Raid Victim", 6, 6);
        world.entity_mut(leader).insert(Officer {
            command_radius: 5.0,
        });
        // No explicit SlainByFaction marker — the raid window blames the raiders.
        world.resource_mut::<LastEnemyStrike>().faction_id = Some(PIRATE_FACTION_ID);
        world.resource_mut::<LastEnemyStrike>().tick = 50;
        world.resource_mut::<SimulationTime>().tick = 100;

        kill(&mut world, leader, "Raid Victim", "Causes unknown");
        run_martyrdom(&mut world);

        assert_eq!(
            world.resource::<Martyrdom>().ticks_remaining,
            MARTYRDOM_DURATION_TICKS
        );
        assert_eq!(
            world.resource::<FactionRelations>().scores[&PIRATE_FACTION_ID],
            -100
        );
    }

    #[test]
    fn test_raid_window_expired_no_martyrdom() {
        let mut world = setup();
        spawn_pop(&mut world, "Witness One", 5, 5);
        let leader = spawn_pop(&mut world, "Late Victim", 6, 6);
        world.entity_mut(leader).insert(Leader);
        world.resource_mut::<LastEnemyStrike>().faction_id = Some(PIRATE_FACTION_ID);
        world.resource_mut::<LastEnemyStrike>().tick = 0;
        world.resource_mut::<SimulationTime>().tick = 1000;

        kill(&mut world, leader, "Late Victim", "Causes unknown");
        run_martyrdom(&mut world);

        assert_eq!(world.resource::<Martyrdom>().ticks_remaining, 0);
    }

    #[test]
    fn test_death_outside_visible_area_no_martyrdom() {
        let mut world = setup();
        spawn_pop(&mut world, "Witness One", 5, 5);
        spawn_pop(&mut world, "Witness Two", 6, 4);
        // Far from the colony heart (5,5) and from any tavern.
        let leader = spawn_pop(&mut world, "Far Leader", 60, 40);
        world.entity_mut(leader).insert(AscensionCandidate);
        world.entity_mut(leader).insert(SlainByFaction {
            faction_id: PIRATE_FACTION_ID,
        });

        kill(&mut world, leader, "Far Leader", "Pirate raid");
        run_martyrdom(&mut world);

        assert_eq!(world.resource::<Martyrdom>().ticks_remaining, 0);
        assert!(chronicle_texts(&mut world).is_empty());
    }

    #[test]
    fn test_tavern_death_is_visible() {
        let mut world = setup();
        // Colony heart far away at (5,5)...
        spawn_pop(&mut world, "Witness One", 5, 5);
        spawn_pop(&mut world, "Witness Two", 6, 4);
        // ...but the leader dies next to a tavern at (60,60).
        world.spawn((Tavern::default(), GridPosition { x: 60, y: 60 }));
        let leader = spawn_pop(&mut world, "Tavern Leader", 61, 60);
        world.entity_mut(leader).insert(AscensionCandidate);
        world.entity_mut(leader).insert(SlainByFaction {
            faction_id: PIRATE_FACTION_ID,
        });

        kill(&mut world, leader, "Tavern Leader", "Pirate raid");
        run_martyrdom(&mut world);

        assert_eq!(
            world.resource::<Martyrdom>().ticks_remaining,
            MARTYRDOM_DURATION_TICKS
        );
    }

    #[test]
    fn test_neutral_factions_rally_to_colony() {
        let mut world = setup();
        spawn_pop(&mut world, "Witness One", 5, 5);
        world
            .resource_mut::<FactionRelations>()
            .scores
            .insert(5, 10); // neutral
        world
            .resource_mut::<FactionRelations>()
            .scores
            .insert(9, 60); // allied — untouched
        let leader = spawn_pop(&mut world, "Rally Leader", 6, 6);
        world.entity_mut(leader).insert(Leader);
        world.entity_mut(leader).insert(SlainByFaction {
            faction_id: PIRATE_FACTION_ID,
        });

        kill(&mut world, leader, "Rally Leader", "Pirate raid");
        run_martyrdom(&mut world);

        let relations = world.resource::<FactionRelations>();
        assert_eq!(relations.scores[&PIRATE_FACTION_ID], -100);
        assert_eq!(relations.scores[&5], 10 + MARTYRDOM_RALLY_BOOST);
        assert_eq!(relations.scores[&9], 60);
    }

    #[test]
    fn test_record_enemy_strike_system() {
        let mut world = setup();
        world.resource_mut::<SimulationTime>().tick = 42;
        world.send_event(PirateRaidEvent);
        let mut schedule = Schedule::default();
        schedule.add_systems(record_enemy_strike_system);
        schedule.run(&mut world);
        let strike = world.resource::<LastEnemyStrike>();
        assert_eq!(strike.faction_id, Some(PIRATE_FACTION_ID));
        assert_eq!(strike.tick, 42);
    }

    #[test]
    fn test_tick_martyrdom_expires_casus_belli() {
        let mut world = setup();
        world.resource_mut::<Martyrdom>().ticks_remaining = 1;
        world.resource_mut::<Martyrdom>().faction_id = Some(PIRATE_FACTION_ID);
        world.resource_mut::<IdeologicalCasusBelli>().faction_id = Some(PIRATE_FACTION_ID);
        world.resource_mut::<IdeologicalCasusBelli>().expires_tick = 1;
        world.resource_mut::<SimulationTime>().tick = 1;

        let mut schedule = Schedule::default();
        schedule.add_systems(tick_martyrdom_system);
        schedule.run(&mut world);

        assert_eq!(world.resource::<Martyrdom>().ticks_remaining, 0);
        assert_eq!(world.resource::<Martyrdom>().faction_id, None);
        assert_eq!(world.resource::<IdeologicalCasusBelli>().faction_id, None);
    }

    #[test]
    fn test_is_leader_figure_pure_rules() {
        assert!(is_leader_figure(true, false, false, false, None, None));
        assert!(is_leader_figure(false, true, false, false, None, None));
        assert!(is_leader_figure(false, false, true, false, None, None));
        assert!(is_leader_figure(false, false, false, true, None, None));
        assert!(!is_leader_figure(false, false, false, false, None, None));

        let mut traits = Traits::default();
        traits.add(Trait::Prophet);
        assert!(is_leader_figure(false, false, false, false, Some(&traits), None));

        let mut skills = Skills::default();
        skills.add_xp(crate::layer1::skills::SkillType::Science, LEADERSHIP_PRESTIGE_XP);
        assert!(is_leader_figure(
            false,
            false,
            false,
            false,
            None,
            Some(&skills)
        ));
    }

    #[test]
    fn test_enemy_attribution_pure_rules() {
        // Explicit marker always wins.
        assert_eq!(enemy_attribution(Some(13), None, 100, "Causes unknown"), Some(13));
        // Raid window blames the striker.
        assert_eq!(
            enemy_attribution(None, Some((13, 50)), 100, "Causes unknown"),
            Some(13)
        );
        // Natural death is never blamed.
        assert_eq!(
            enemy_attribution(None, Some((13, 50)), 100, NATURAL_DEATH_REASON),
            None
        );
        // Stale strike is not blamed.
        assert_eq!(enemy_attribution(None, Some((13, 0)), 1000, "Causes unknown"), None);
        // No evidence at all.
        assert_eq!(enemy_attribution(None, None, 100, "Causes unknown"), None);
    }

    // --- IP guard ---

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // The Martyrdom Effect is concept-only: generic "pirate raiders" /
        // "Homeworld" naming, nothing lifted from outside fiction.
        let banned = [
            "emperor",
            "dune",
            "corrino",
            "arrakis",
            "atreides",
            "imperium",
            "padishah",
            "sardaukar",
            "mentat",
            "fremen",
            "sad king billy",
            "windsor",
            "windsor-in-exile",
            "crusade",
            "jihad",
            "inquisition",
        ];
        for s in martyrdom_public_strings() {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(
                    !lower.contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
