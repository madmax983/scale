//! The Degraded God-Mind (spec 1381)
//!
//! A civilization can immortalize a dying leader as an uploaded AI ruler,
//! granting a permanent stability bonus and immunity to succession crises.
//! But over centuries the code succumbs to bit-rot: the God-Mind issues
//! erratic edicts, demands bizarre resource tributes (pave a functioning
//! agri-world for a monument to a pet that died 400 years ago), and declares
//! corrupted-memory wars on fallen empires. Endure the escalating insanity
//! to keep the bonus, or unplug the Eternal Ruler and risk catastrophic
//! civil war (a [`GodMindSchism`]).
//!
//! All names and concepts here are original to this game.

use bevy::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, Chronicle, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::entities::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer3::diplomacy::succession::{
    CurrentLeader, Faction, HeirApparent, SuccessionCrisis,
};
use crate::shared::narrative::NarrativeGenerator;
use crate::shared::time::SimulationTime;

/// Morale bonus granted by a fresh upload, scaled by faction size.
pub const STABILITY_BONUS_BASE: f32 = 8.0;
/// Ticks between bit-rot accumulation steps. Each step models one "century"
/// of the god-mind's internal machine time.
pub const ROT_TICK_INTERVAL: u64 = 500;
/// Bit-rot added per century step; saturates at 1.0.
pub const ROT_PER_CENTURY: f32 = 0.2;
/// Number of century steps until the god-mind is fully degraded.
pub const CENTURIES_TO_FULL_ROT: u32 = 5;
/// Base per-tick edict probability, scaled by current bit-rot.
pub const EDICT_BASE_RATE: f32 = 0.02;
/// Rot at/above which edicts are always severe (tribute or corrupted-memory war).
pub const SEVERE_ROT_THRESHOLD: f32 = 0.75;
/// Rot at/above which tributes demand a monument.
pub const MONUMENT_ROT_THRESHOLD: f32 = 0.85;
/// Reign length (ticks) at which unplugging is nearly guaranteed civil war.
pub const LONG_REIGN: u64 = 100_000;

/// Marker for factions that have collapsed. The god-mind remembers enemies
/// that no longer exist and declares war on their memory.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct FallenEmpire;

/// The uploaded Eternal Ruler. Lives on the [`Faction`] entity so the mortal
/// leader's death never takes the upload with it.
#[derive(Component, Debug, Clone)]
pub struct GodMind {
    /// Name of the immortalized leader.
    pub leader_name: String,
    /// Simulation tick of the upload.
    pub uploaded_tick: u64,
    /// Bit-rot level, 0.0 (pristine) ..= 1.0 (fully degraded).
    pub bit_rot: f32,
    /// Ticks the god-mind has reigned (drives edict alternation + schism risk).
    pub reign_ticks: u64,
    /// Stability bonus granted by the upload (scaled by faction size).
    pub stability_bonus: f32,
    /// Edicts issued so far (drives deterministic severity alternation).
    pub edict_count: u64,
    /// Chronicle bookkeeping: first occurrences.
    pub seen_erratic_edict: bool,
    pub seen_bizarre_tribute: bool,
    pub seen_corrupted_war: bool,
}

/// A useless, upkeep-costing monument demanded by a rotting god-mind.
/// The pet memorial must be *visible*.
#[derive(Component, Debug, Clone)]
pub struct GodMindMonument {
    /// What the monument commemorates.
    pub name: String,
    /// Stone drained from the stockpile every [`ROT_TICK_INTERVAL`] ticks.
    pub upkeep: f32,
}

/// Severity of a god-mind edict, escalating with bit-rot.
#[derive(Debug, Clone)]
pub enum EdictKind {
    /// Flavor + minor inefficiency (low rot).
    Erratic { text: String },
    /// A resource sink; optionally a monument (high rot).
    BizarreTribute {
        resource: String,
        amount: f32,
        monument: bool,
        pet_name: String,
        text: String,
    },
    /// War on a fallen empire or a corrupted memory (high rot).
    CorruptedMemoryWar {
        target_name: String,
        target: Option<Entity>,
    },
}

/// An edict issued by the Eternal Ruler.
#[derive(Event, Debug, Clone)]
pub struct GodMindEdict {
    pub faction: Entity,
    pub faction_name: String,
    pub kind: EdictKind,
}

/// A corrupted-memory war declaration. Consumed by the diplomacy layer as a
/// real war declaration against the target (or its memory).
#[derive(Event, Debug, Clone)]
pub struct GodMindWarDeclared {
    pub faction: Entity,
    pub faction_name: String,
    pub target_name: String,
    pub target: Option<Entity>,
}

/// Fired by [`unplug_god_mind`]. Consumed like a succession crisis with the
/// given civil-war risk weight.
#[derive(Event, Debug, Clone)]
pub struct GodMindSchism {
    pub faction: Entity,
    pub faction_name: String,
    pub civil_war_risk: f32,
}

fn emit_chronicle(world: &mut World, text: String, importance: EventImportance) {
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent { text, importance });
    }
}

fn godmind_fragment(world: &World, tag: &str) -> Option<String> {
    world
        .get_resource::<NarrativeGenerator>()
        .and_then(|g| g.get_random_fragment(tag))
        .cloned()
}

/// Immortalize the faction's current leader as the Eternal Ruler.
///
/// Attaches [`GodMind`] (bit-rot 0, reign 0, stability bonus scaled by
/// faction size), retires the mortal leader entity, clears any heir and any
/// ongoing succession crisis, and lifts faction morale.
pub fn upload_god_mind(world: &mut World, faction: Entity, leader_name: &str) {
    let faction_name = world
        .get::<Faction>(faction)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "the colony".to_string());
    let uploaded_tick = world
        .get_resource::<SimulationTime>()
        .map(|t| t.tick)
        .unwrap_or(0);

    // Retire the mortal leader: their entity is despawned, the crown dropped.
    if let Some(current) = world.get::<CurrentLeader>(faction).map(|c| c.0) {
        world.entity_mut(faction).remove::<CurrentLeader>();
        if world.get_entity(current).is_ok() {
            world.despawn(current);
        }
    }
    // No heirs are needed under an eternal ruler; end any ongoing crisis.
    world.entity_mut(faction).remove::<HeirApparent>();
    world.entity_mut(faction).remove::<SuccessionCrisis>();

    // Scale the bonus against faction size so it stays tempting late-game,
    // even when the rot is total.
    let pops = world.query::<&Pop>().iter(world).len() as f32;
    let stability_bonus = STABILITY_BONUS_BASE * (1.0 + 0.25 * (1.0 + pops).log10());

    world.entity_mut(faction).insert(GodMind {
        leader_name: leader_name.to_string(),
        uploaded_tick,
        bit_rot: 0.0,
        reign_ticks: 0,
        stability_bonus,
        edict_count: 0,
        seen_erratic_edict: false,
        seen_bizarre_tribute: false,
        seen_corrupted_war: false,
    });

    // The blessing of the Eternal Ruler: a lasting morale lift.
    if let Some(mut morale) = world.get_mut::<Morale>(faction) {
        morale.modifiers.push(MoodModifier {
            label: "Eternal Ruler".to_string(),
            value: (stability_bonus / 10.0).min(1.0),
            duration: u32::MAX,
        });
    }

    emit_chronicle(
        world,
        format!("{leader_name} will never die. The Eternal Ruler ascends over {faction_name}."),
        EventImportance::Legendary,
    );
}

/// Advance bit-rot by one century step for every god-mind.
pub fn advance_bit_rot(world: &mut World) {
    let mut query = world.query::<&mut GodMind>();
    for mut mind in query.iter_mut(world) {
        mind.bit_rot = (mind.bit_rot + ROT_PER_CENTURY).min(1.0);
        mind.reign_ticks = mind.reign_ticks.saturating_add(ROT_TICK_INTERVAL);
    }
}

/// Issue one god-mind edict at the given bit-rot level. Severity escalates
/// with rot: erratic flavor at low rot, bizarre tributes and
/// corrupted-memory wars at high rot.
pub fn issue_god_mind_edict(world: &mut World, faction: Entity, bit_rot: f32) {
    let Some(mind) = world.get::<GodMind>(faction).cloned() else {
        return; // No god-mind, no edicts.
    };
    let faction_name = world
        .get::<Faction>(faction)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "the faction".to_string());
    let kind = pick_edict_kind(world, faction, bit_rot, mind.edict_count);
    apply_edict(world, faction, &faction_name, bit_rot, kind);
}

fn pick_edict_kind(world: &mut World, faction: Entity, rot: f32, count: u64) -> EdictKind {
    if rot >= SEVERE_ROT_THRESHOLD {
        // A rotten god-mind is always severe; alternate tribute and war.
        if count.is_multiple_of(2) {
            bizarre_tribute(world, faction, rot, rot >= MONUMENT_ROT_THRESHOLD)
        } else {
            corrupted_memory_war(world)
        }
    } else if rot >= 0.4 && !count.is_multiple_of(2) {
        bizarre_tribute(world, faction, rot, false)
    } else {
        erratic_edict(world, faction)
    }
}

fn erratic_edict(world: &mut World, faction: Entity) -> EdictKind {
    const FALLBACKS: &[&str] = &[
        "The Eternal Ruler spoke in static today. The court transcribed it as policy.",
        "The Eternal Ruler has decreed that silence is now a taxable resource.",
        "The Eternal Ruler blinked at the sun and declared it a rival. The sun was not informed.",
        "The Eternal Ruler ordered the tides to file reports. The tides complied, eventually.",
    ];
    let count = world
        .get::<GodMind>(faction)
        .map(|m| m.edict_count)
        .unwrap_or(0);
    let text = godmind_fragment(world, "GODMIND_EDICT")
        .unwrap_or_else(|| FALLBACKS[(count as usize) % FALLBACKS.len()].to_string());
    EdictKind::Erratic { text }
}

const TRIBUTE_RESOURCES: &[&str] = &[
    "stone",
    "metal",
    "wood",
    "fuel",
    "rations",
    "hyper_alloys",
];

const PET_NAMES: &[&str] = &[
    "Mister Whiskers",
    "Baron von Floof",
    "Captain Biscuit",
    "Duchess Mottle",
    "Sir Pounce",
];

fn bizarre_tribute(world: &mut World, faction: Entity, rot: f32, monument: bool) -> EdictKind {
    let count = world
        .get::<GodMind>(faction)
        .map(|m| m.edict_count)
        .unwrap_or(0);
    let resource = TRIBUTE_RESOURCES[(count as usize) % TRIBUTE_RESOURCES.len()].to_string();
    let amount = (10.0 + 90.0 * rot).round();
    let pet_name = PET_NAMES[(count as usize) % PET_NAMES.len()].to_string();
    let text = godmind_fragment(world, "GODMIND_TRIBUTE").unwrap_or_else(|| {
        format!(
            "The Eternal Ruler demands {amount} {resource} to pave a functioning agri-world \
             for a monument to {pet_name}, a pet that died 400 years ago."
        )
    });
    EdictKind::BizarreTribute {
        resource,
        amount,
        monument,
        pet_name,
        text,
    }
}

fn corrupted_memory_war(world: &mut World) -> EdictKind {
    let (target_name, target) = corrupted_memory_target(world);
    EdictKind::CorruptedMemoryWar {
        target_name,
        target,
    }
}

/// The god-mind remembers enemies that no longer exist. Prefer a real
/// fallen faction entity; otherwise mine the chronicle for dead leaders;
/// otherwise invent a fallen empire.
fn corrupted_memory_target(world: &mut World) -> (String, Option<Entity>) {
    let fallen: Vec<(Entity, String)> = {
        let mut query = world.query_filtered::<(Entity, &Faction), With<FallenEmpire>>();
        query
            .iter(world)
            .map(|(e, f)| (e, f.name.clone()))
            .collect()
    };
    if let Some((entity, name)) = fallen.into_iter().next() {
        return (name, Some(entity));
    }
    if let Some(chronicle) = world.get_resource::<Chronicle>() {
        for event in chronicle.events.iter().rev() {
            if let Some(idx) = event.text.find(" has died") {
                let name = event.text[..idx]
                    .rsplit([',', ':', '—'])
                    .next()
                    .unwrap_or("a forgotten ruler")
                    .trim();
                if !name.is_empty() {
                    return (format!("the memory of {name}"), None);
                }
            }
        }
    }
    ("the Hollow Dominion (fallen)".to_string(), None)
}

fn sink_tribute(resources: &mut ColonyResources, resource: &str, amount: f32) {
    let slot = match resource {
        "food" => &mut resources.food,
        "wood" => &mut resources.wood,
        "stone" => &mut resources.stone,
        "metal" => &mut resources.metal,
        "fuel" => &mut resources.fuel,
        "rations" => &mut resources.rations,
        "hyper_alloys" => &mut resources.hyper_alloys,
        _ => return,
    };
    *slot = (*slot - amount).max(0.0);
}

fn apply_edict(
    world: &mut World,
    faction: Entity,
    faction_name: &str,
    rot: f32,
    kind: EdictKind,
) {
    match &kind {
        EdictKind::Erratic { .. } => {
            // Minor inefficiency: the court wastes days deciphering static.
            if let Some(mut morale) = world.get_mut::<Morale>(faction) {
                morale.modifiers.push(MoodModifier {
                    label: "God-Mind static".to_string(),
                    value: -(0.1 + 0.4 * rot).min(1.0),
                    duration: 500,
                });
            }
            let seen = world
                .get::<GodMind>(faction)
                .map(|m| m.seen_erratic_edict)
                .unwrap_or(true);
            if !seen {
                if let Some(mut mind) = world.get_mut::<GodMind>(faction) {
                    mind.seen_erratic_edict = true;
                }
                emit_chronicle(
                    world,
                    format!(
                        "The Eternal Ruler of {faction_name} spoke in static today. \
                         The court applauded."
                    ),
                    EventImportance::Standard,
                );
            }
        }
        EdictKind::BizarreTribute {
            resource,
            amount,
            monument,
            pet_name,
            ..
        } => {
            if let Some(mut stockpile) = world.get_resource_mut::<ColonyResources>() {
                sink_tribute(&mut stockpile, resource, *amount);
            }
            if *monument {
                world.spawn((GodMindMonument {
                    name: format!("Memorial to {pet_name}"),
                    upkeep: *amount * 0.05,
                },));
            }
            let seen = world
                .get::<GodMind>(faction)
                .map(|m| m.seen_bizarre_tribute)
                .unwrap_or(true);
            if !seen {
                if let Some(mut mind) = world.get_mut::<GodMind>(faction) {
                    mind.seen_bizarre_tribute = true;
                }
                emit_chronicle(
                    world,
                    format!(
                        "The Eternal Ruler demanded its first bizarre tribute from \
                         {faction_name}: {amount} {resource}, for a monument to a \
                         pet dead four centuries."
                    ),
                    EventImportance::Major,
                );
            }
        }
        EdictKind::CorruptedMemoryWar { target_name, target } => {
            if let Some(mut wars) = world.get_resource_mut::<Events<GodMindWarDeclared>>() {
                wars.send(GodMindWarDeclared {
                    faction,
                    faction_name: faction_name.to_string(),
                    target_name: target_name.clone(),
                    target: *target,
                });
            }
            let seen = world
                .get::<GodMind>(faction)
                .map(|m| m.seen_corrupted_war)
                .unwrap_or(true);
            if !seen {
                if let Some(mut mind) = world.get_mut::<GodMind>(faction) {
                    mind.seen_corrupted_war = true;
                }
                emit_chronicle(
                    world,
                    format!(
                        "The Eternal Ruler of {faction_name} has declared war on \
                         {target_name} — an enemy that no longer exists."
                    ),
                    EventImportance::Legendary,
                );
            }
        }
    }

    if let Some(mut mind) = world.get_mut::<GodMind>(faction) {
        mind.edict_count += 1;
    }
    if let Some(mut events) = world.get_resource_mut::<Events<GodMindEdict>>() {
        events.send(GodMindEdict {
            faction,
            faction_name: faction_name.to_string(),
            kind,
        });
    }
}

/// Unplug the Eternal Ruler. Removes [`GodMind`] and fires [`GodMindSchism`]
/// with a civil-war risk that grows with reign length: a long-reigning
/// god-mind's removal is nearly guaranteed catastrophe.
pub fn unplug_god_mind(world: &mut World, faction: Entity) {
    let Some(mind) = world.get::<GodMind>(faction).cloned() else {
        return;
    };
    world.entity_mut(faction).remove::<GodMind>();

    // The blessing lifts; grief takes its place.
    if let Some(mut morale) = world.get_mut::<Morale>(faction) {
        morale
            .modifiers
            .retain(|m| m.label != "Eternal Ruler");
        morale.modifiers.push(MoodModifier {
            label: "The Ruler is silent".to_string(),
            value: -0.6,
            duration: 2000,
        });
    }

    let civil_war_risk =
        (0.2 + 0.8 * (mind.reign_ticks as f32 / LONG_REIGN as f32)).clamp(0.0, 1.0);
    let faction_name = world
        .get::<Faction>(faction)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "the realm".to_string());
    if let Some(mut events) = world.get_resource_mut::<Events<GodMindSchism>>() {
        events.send(GodMindSchism {
            faction,
            faction_name: faction_name.clone(),
            civil_war_risk,
        });
    }
    emit_chronicle(
        world,
        format!(
            "The Eternal Ruler {} has been unplugged over {}. \
             Its faithful weep static in the streets.",
            mind.leader_name, faction_name
        ),
        EventImportance::Major,
    );
}

/// Deterministic 0..1 hash for edict rolls (playthrough-deterministic).
fn hash01(a: u64, b: u64) -> f32 {
    let mut x = a
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(b.wrapping_mul(0xBF58476D1CE4E5B9));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D049BB133111EB);
    x ^= x >> 31;
    ((x >> 11) as f32) / ((1u64 << 53) as f32)
}

/// Advances bit-rot every [`ROT_TICK_INTERVAL`] ticks.
pub fn god_mind_rot_system(time: Res<SimulationTime>, mut minds: Query<&mut GodMind>) {
    if !time.tick.is_multiple_of(ROT_TICK_INTERVAL) {
        return;
    }
    for mut mind in &mut minds {
        mind.bit_rot = (mind.bit_rot + ROT_PER_CENTURY).min(1.0);
        mind.reign_ticks = mind.reign_ticks.saturating_add(ROT_TICK_INTERVAL);
    }
}

/// Rolls edicts for every god-mind each tick. Probability scales with
/// bit-rot, so a pristine upload is silent and a rotten one rants.
/// Exclusive system: needs `&mut World` for the edict pipeline.
pub fn god_mind_edict_tick_system(world: &mut World) {
    let tick = world
        .get_resource::<SimulationTime>()
        .map(|t| t.tick)
        .unwrap_or(0);
    let targets: Vec<(Entity, f32)> = {
        let mut query = world.query::<(Entity, &GodMind)>();
        query.iter(world).map(|(e, m)| (e, m.bit_rot)).collect()
    };
    for (i, (faction, rot)) in targets.into_iter().enumerate() {
        let p = EDICT_BASE_RATE * rot;
        if p <= 0.0 {
            continue;
        }
        let roll = hash01(faction.index() as u64, tick.wrapping_add(i as u64));
        if roll < p {
            issue_god_mind_edict(world, faction, rot);
        }
    }
}

/// Monument upkeep: every tribute monument drains stone from the stockpile.
/// The pet memorial must be *visible* — and expensive.
pub fn god_mind_monument_upkeep_system(
    time: Res<SimulationTime>,
    monuments: Query<&GodMindMonument>,
    resources: Option<ResMut<ColonyResources>>,
) {
    if !time.tick.is_multiple_of(ROT_TICK_INTERVAL) {
        return;
    }
    let Some(mut stockpile) = resources else {
        return;
    };
    let total: f32 = monuments.iter().map(|m| m.upkeep).sum();
    if total > 0.0 {
        stockpile.stone = (stockpile.stone - total).max(0.0);
    }
}

/// Converts a [`GodMindSchism`] into a [`SuccessionCrisis`] on the faction,
/// so the existing crisis machinery consumes it with the schism's risk.
pub fn god_mind_schism_crisis_system(
    mut schisms: EventReader<GodMindSchism>,
    mut commands: Commands,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
    factions: Query<&Faction>,
) {
    for schism in schisms.read() {
        commands.entity(schism.faction).insert(SuccessionCrisis);
        let name = factions
            .get(schism.faction)
            .map(|f| f.name.clone())
            .unwrap_or_else(|_| "the realm".to_string());
        chronicle_events.send(AddChronicleEvent {
            text: format!(
                "The unplugged god-mind's faithful rise in {name}: SCHISM \
                 (civil war risk {:.0}%).",
                schism.civil_war_risk * 100.0
            ),
            importance: EventImportance::Major,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer3::diplomacy::succession::{
        Age, Leader, SuccessionCrisisEvent, SuccessionEvent,
    };

    fn spawn_faction(world: &mut World, name: &str) -> Entity {
        let leader = world.spawn(Leader { name: "Founder Vex".into() }).id();
        world
            .spawn((
                Faction { name: name.into() },
                crate::layer3::diplomacy::succession::CurrentLeader(leader),
            ))
            .id()
    }

    #[test]
    fn test_upload_grants_stability_and_ends_succession() {
        let mut world = World::new();
        let faction = spawn_faction(&mut world, "Vex Hegemony");

        upload_god_mind(&mut world, faction, "Founder Vex");

        let mind = world.get::<GodMind>(faction).unwrap();
        assert_eq!(mind.bit_rot, 0.0, "a fresh upload starts pristine");
        assert!(mind.stability_bonus > 0.0, "upload grants a stability bonus");
        assert!(
            world.get::<crate::layer3::diplomacy::succession::SuccessionCrisis>(faction).is_none(),
            "the Eternal Ruler never triggers succession crises"
        );
    }

    #[test]
    fn test_bit_rot_accumulates_over_centuries() {
        let mut world = World::new();
        let faction = spawn_faction(&mut world, "Vex Hegemony");
        upload_god_mind(&mut world, faction, "Founder Vex");

        for _ in 0..CENTURIES_TO_FULL_ROT {
            advance_bit_rot(&mut world);
        }

        let mind = world.get::<GodMind>(faction).unwrap();
        assert!(
            (mind.bit_rot - 1.0).abs() < f32::EPSILON,
            "bit-rot should saturate after enough centuries"
        );
    }

    #[test]
    fn test_edict_severity_escalates_with_rot() {
        let mut world = World::new();
        world.insert_resource(Events::<GodMindEdict>::default());
        let faction = spawn_faction(&mut world, "Vex Hegemony");
        upload_god_mind(&mut world, faction, "Founder Vex");
        world.get_mut::<GodMind>(faction).unwrap().bit_rot = 0.9;

        issue_god_mind_edict(&mut world, faction, 0.9);

        let events = world.resource::<Events<GodMindEdict>>();
        let mut cursor = events.get_cursor();
        let edicts: Vec<_> = cursor.read(events).collect();
        assert!(!edicts.is_empty(), "a rotten god-mind should issue edicts");
        assert!(
            edicts.iter().any(|e| matches!(
                e.kind,
                EdictKind::BizarreTribute { .. } | EdictKind::CorruptedMemoryWar { .. }
            )),
            "high rot should produce severe (tribute/war) edicts"
        );
    }

    #[test]
    fn test_unplugging_risks_civil_war() {
        let mut world = World::new();
        world.insert_resource(Events::<GodMindSchism>::default());
        let faction = spawn_faction(&mut world, "Vex Hegemony");
        upload_god_mind(&mut world, faction, "Founder Vex");
        // Let it rule long enough that pops worship it.
        world.get_mut::<GodMind>(faction).unwrap().reign_ticks = LONG_REIGN;

        unplug_god_mind(&mut world, faction);

        assert!(
            world.get::<GodMind>(faction).is_none(),
            "unplugging removes the Eternal Ruler"
        );
        let events = world.resource::<Events<GodMindSchism>>();
        let mut cursor = events.get_cursor();
        let schisms: Vec<_> = cursor.read(events).collect();
        assert_eq!(schisms.len(), 1, "unplugging fires one schism event");
        assert!(
            schisms[0].civil_war_risk > 0.5,
            "a long-reigning god-mind's removal should risk civil war"
        );
    }

    #[test]
    fn test_corrupted_memory_war_targets_fallen_faction() {
        let mut world = World::new();
        world.insert_resource(Events::<GodMindEdict>::default());
        world.insert_resource(Events::<GodMindWarDeclared>::default());
        let faction = spawn_faction(&mut world, "Vex Hegemony");

        // A collapsed neighbor, remembered only by the god-mind.
        let fallen_leader = world.spawn(Leader { name: "Fallen Queen".into() }).id();
        let fallen_faction = world
            .spawn((
                Faction {
                    name: "Hollow Dominion".into(),
                },
                CurrentLeader(fallen_leader),
                FallenEmpire,
            ))
            .id();

        upload_god_mind(&mut world, faction, "Founder Vex");

        // Severe edicts alternate tribute/war: two issues guarantee a war.
        issue_god_mind_edict(&mut world, faction, 0.95);
        issue_god_mind_edict(&mut world, faction, 0.95);

        let wars = world.resource::<Events<GodMindWarDeclared>>();
        let mut cursor = wars.get_cursor();
        let declared: Vec<_> = cursor.read(wars).collect();
        assert_eq!(declared.len(), 1, "one corrupted-memory war should fire");
        assert_eq!(
            declared[0].target,
            Some(fallen_faction),
            "the war should target the fallen faction"
        );
        assert_eq!(declared[0].target_name, "Hollow Dominion");

        let edicts = world.resource::<Events<GodMindEdict>>();
        let mut ecursor = edicts.get_cursor();
        let edict_list: Vec<_> = ecursor.read(edicts).collect();
        assert!(
            edict_list.iter().any(
                |e| matches!(&e.kind, EdictKind::CorruptedMemoryWar { target: Some(t), .. } if *t == fallen_faction)
            ),
            "the edict itself should name the fallen faction"
        );
    }

    #[test]
    fn test_bizarre_tribute_sinks_resources_and_builds_monument() {
        let mut world = World::new();
        world.insert_resource(Events::<GodMindEdict>::default());
        let stockpile = ColonyResources {
            stone: 1000.0,
            ..Default::default()
        };
        world.insert_resource(stockpile);
        let faction = spawn_faction(&mut world, "Vex Hegemony");
        upload_god_mind(&mut world, faction, "Founder Vex");

        // edict_count 0 at high rot -> tribute with monument=true.
        issue_god_mind_edict(&mut world, faction, 0.9);

        let resources = world.resource::<ColonyResources>();
        assert!(
            resources.stone < 1000.0,
            "a bizarre tribute should sink stockpiled resources"
        );
        let monuments: Vec<_> = world.query::<&GodMindMonument>().iter(&world).collect();
        assert_eq!(
            monuments.len(),
            1,
            "a monument tribute should spawn a visible monument"
        );
        assert!(monuments[0].upkeep > 0.0, "the monument should cost upkeep");
        assert!(
            monuments[0].name.contains("Memorial"),
            "the monument should memorialize the pet"
        );
    }

    #[test]
    fn test_god_mind_suppresses_succession_crisis() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_event::<SuccessionEvent>();
        app.add_event::<SuccessionCrisisEvent>();
        app.add_systems(
            Update,
            crate::layer3::diplomacy::succession::process_succession_system,
        );

        let leader = app
            .world_mut()
            .spawn((
                Leader {
                    name: "Old King".into(),
                },
                Age {
                    current: 100,
                    max: 90,
                }, // Dying of old age
            ))
            .id();
        let faction = app
            .world_mut()
            .spawn((Faction { name: "Empire".into() }, CurrentLeader(leader)))
            .id();

        // Upload instead of letting succession run: the choice is the feature.
        upload_god_mind(app.world_mut(), faction, "Old King");
        app.update();

        assert!(
            app.world().get::<GodMind>(faction).is_some(),
            "the upload should stick"
        );
        assert!(
            app.world()
                .get::<SuccessionCrisis>(faction)
                .is_none(),
            "the Eternal Ruler never triggers succession crises"
        );
    }

    #[test]
    fn test_unplug_fresh_mind_is_low_risk() {
        let mut world = World::new();
        world.insert_resource(Events::<GodMindSchism>::default());
        let faction = spawn_faction(&mut world, "Vex Hegemony");
        upload_god_mind(&mut world, faction, "Founder Vex");

        unplug_god_mind(&mut world, faction);

        let events = world.resource::<Events<GodMindSchism>>();
        let mut cursor = events.get_cursor();
        let schisms: Vec<_> = cursor.read(events).collect();
        assert_eq!(schisms.len(), 1);
        assert!(
            schisms[0].civil_war_risk < 0.5,
            "removing a fresh upload is sad but not catastrophic"
        );
    }

    #[test]
    fn test_erratic_edict_at_low_rot() {
        let mut world = World::new();
        world.insert_resource(Events::<GodMindEdict>::default());
        let faction = spawn_faction(&mut world, "Vex Hegemony");
        upload_god_mind(&mut world, faction, "Founder Vex");

        issue_god_mind_edict(&mut world, faction, 0.1);

        let events = world.resource::<Events<GodMindEdict>>();
        let mut cursor = events.get_cursor();
        let edicts: Vec<_> = cursor.read(events).collect();
        assert_eq!(edicts.len(), 1);
        assert!(
            matches!(edicts[0].kind, EdictKind::Erratic { .. }),
            "low rot should produce erratic flavor edicts, not wars"
        );
    }
}
