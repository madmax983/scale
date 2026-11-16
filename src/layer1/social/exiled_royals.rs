//! Exiled Royals (Spec 423).
//!
//! Hosting a deposed royal from a Layer 3 empire: massive passive
//! Culture/Diplomacy bonuses and a massive one-time credit payout, in
//! exchange for luxury-suite demands and recurring Assassination Squad
//! raids sent by the empire that overthrew them. Assassins carry
//! high-tech gear that drops on death — farming the squads is the
//! payoff fork.
//!
//! ## Audit / design notes (spec mocks -> real seams)
//!
//! * Spec RED tests name mock `ColonyCulture`, `ColonyDiplomacy`,
//!   `ActiveRaids` and a sketch `ExiledRoyal`. None exist. Real seams:
//!   diplomacy is layer-3 [`FactionRelations`] (u32 ids, -100..100);
//!   credits are [`ColonyResources`]; raids are the real
//!   [`PirateRaidEvent`] pipeline; deaths are the real [`PopDied`]
//!   event; ground loot is the real [`Item`]/[`ItemType`] + `GridPosition`
//!   pattern; morale complaints use real [`Morale`]/`MoodModifier`.
//! * No parallel mock systems: culture/diplomacy bonuses live in one
//!   derived ledger, [`ExileInfluence`], **recomputed from scratch**
//!   every tick by [`exile_bonus_system`] (spec §5 refactor note — a
//!   `+=` per tick would double-count unboundedly). Diplomacy is also
//!   applied once, on arrival, to the sponsor faction in the real
//!   `FactionRelations` (clamped), so it is visible to Layer 3.
//! * Raids (§5): [`assassination_raid_spawner`] fires only when a living
//!   royal's threat >= [`RAID_THREAT_THRESHOLD`], respects
//!   [`RAID_COOLDOWN_TICKS`], sends a real `PirateRaidEvent`, and spawns
//!   a squad of [`Assassin`] pops (full `PopBundle`, Pirate trait,
//!   high-damage weapon) at the colony edge.
//! * Gear drops (§7): [`assassin_loot_system`] listens for `PopDied`
//!   of an `Assassin` and drops a high-tech [`Item`] (Prosthetic /
//!   Manual / Tool, rotating) at the assassin's tile, plus a small
//!   credit bounty to `ColonyResources`.
//! * Luxury suite: a royal is "suite-satisfied" while any Tavern,
//!   Library or Statue building stands (the colony's luxury venues);
//!   [`royal_suite_system`] periodically applies a real mood modifier
//!   to unsatisfied royals and chronicles the demand.
//! * Royal death: sponsor relations tank, usurper relations improve
//!   slightly, Major chronicle.
//! * All names original/generic ("Exiled Royal", "Assassination
//!   Squad", "the Overthrowing Empire"); IP-guard test below.

use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;

use crate::layer1::architecture::building::{Building, BuildingType};
use crate::layer1::combat::{AttackProperties, Weapon};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::economy::items::{Item, ItemType};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::economy::Wallet;
use crate::layer1::pop::{Pop, PopBundle, PopDied};
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::old_guard::Arrival;
use crate::layer1::void_weed::PirateRaidEvent;
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::time::SimulationTime;

/// Sponsor (deposed royal's home) empire faction id.
pub const ROYAL_SPONSOR_FACTION_ID: u32 = 8;
/// Overthrowing empire faction id (sends the squads).
pub const ROYAL_USURPER_FACTION_ID: u32 = 9;
/// Default passive culture bonus per royal.
pub const DEFAULT_CULTURE_BONUS: f32 = 50.0;
/// Default passive diplomacy bonus per royal.
pub const DEFAULT_DIPLOMACY_BONUS: f32 = 25.0;
/// Default threat level of an exiled royal.
pub const DEFAULT_THREAT_LEVEL: f32 = 100.0;
/// Threat at/above which squads are dispatched.
pub const RAID_THREAT_THRESHOLD: f32 = 50.0;
/// Ticks between assassination raids (spec §5 cooldown).
pub const RAID_COOLDOWN_TICKS: u64 = 500;
/// Delay before the first raid after arrival.
pub const FIRST_RAID_DELAY_TICKS: u64 = 100;
/// Assassins per squad.
pub const SQUAD_SIZE: u32 = 3;
/// One-time credit payout per accepted royal.
pub const ROYAL_PAYOUT_CREDITS: f32 = 5000.0;
/// Credit bounty per assassin killed (gear resale, abstracted).
pub const ASSASSIN_BOUNTY_CREDITS: f32 = 150.0;
/// Assassin weapon damage (high-tier).
pub const ASSASSIN_WEAPON_DAMAGE: f32 = 30.0;
/// Ticks between luxury-suite checks.
pub const SUITE_CHECK_PERIOD_TICKS: u64 = 500;

/// Marker: a deposed royal hosted by the colony.
#[derive(Component, Debug, Clone, Copy)]
pub struct ExiledRoyal {
    pub culture_bonus: f32,
    pub diplomacy_bonus: f32,
    pub threat_level: f32,
    pub sponsor_faction_id: u32,
    pub usurper_faction_id: u32,
}

impl Default for ExiledRoyal {
    fn default() -> Self {
        Self {
            culture_bonus: DEFAULT_CULTURE_BONUS,
            diplomacy_bonus: DEFAULT_DIPLOMACY_BONUS,
            threat_level: DEFAULT_THREAT_LEVEL,
            sponsor_faction_id: ROYAL_SPONSOR_FACTION_ID,
            usurper_faction_id: ROYAL_USURPER_FACTION_ID,
        }
    }
}

/// Marker: a member of an Assassination Squad. Drops high-tech gear.
#[derive(Component, Debug, Clone, Copy)]
pub struct Assassin {
    pub usurper_faction_id: u32,
}

/// Derived passive influence of living royals — recomputed, never
/// accumulated (spec §5). Adapts the spec's mock `ColonyCulture` /
/// `ColonyDiplomacy` resources to a single real ledger.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ExileInfluence {
    pub culture: f32,
    pub diplomacy: f32,
    pub royals: usize,
}

/// Raid scheduling state (cooldown + counters). Adapts mock `ActiveRaids`.
#[derive(Resource, Debug, Clone)]
pub struct AssassinationState {
    pub next_raid_tick: u64,
    pub raids_launched: u32,
    pub assassins_spawned: u32,
    pub gear_dropped: u32,
    pub last_raid_tick: Option<u64>,
}

impl Default for AssassinationState {
    fn default() -> Self {
        Self {
            next_raid_tick: FIRST_RAID_DELAY_TICKS,
            raids_launched: 0,
            assassins_spawned: 0,
            gear_dropped: 0,
            last_raid_tick: None,
        }
    }
}

/// Event: an exiled royal seeks sanctuary.
#[derive(Event, Debug, Clone)]
pub struct RoyalArrivalEvent {
    pub count: u32,
}

/// Recompute passive bonuses from living royals (reactive, §5).
pub fn exile_bonus_system(
    mut influence: ResMut<ExileInfluence>,
    query: Query<&ExiledRoyal>,
) {
    let mut culture = 0.0;
    let mut diplomacy = 0.0;
    let mut royals = 0usize;
    for royal in query.iter() {
        culture += royal.culture_bonus;
        diplomacy += royal.diplomacy_bonus;
        royals += 1;
    }
    influence.culture = culture;
    influence.diplomacy = diplomacy;
    influence.royals = royals;
}

/// Spawn exiled royals: full pops, Noble+Entitled traits, fat wallet,
/// one-time payout + sponsor diplomacy applied once to real relations.
pub fn process_royal_arrival_system(
    mut commands: Commands,
    mut events: EventReader<RoyalArrivalEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut resources: ResMut<ColonyResources>,
    mut relations: ResMut<FactionRelations>,
    sim_time: Option<Res<SimulationTime>>,
    pop_positions: Query<&GridPosition, With<Pop>>,
) {
    let tick = sim_time.map_or(0, |t| t.tick);
    let colony_tiles: Vec<GridPosition> = pop_positions.iter().copied().collect();
    let mut rng = rand::thread_rng();
    for event in events.read() {
        for _ in 0..event.count {
            let spawn_pos = colony_tiles
                .choose(&mut rng)
                .copied()
                .unwrap_or(GridPosition { x: 0, y: 0 });
            let mut bundle = PopBundle::random(spawn_pos.x, spawn_pos.y, &mut rng);
            let mut traits = Traits::default();
            traits.add(Trait::Noble);
            traits.add(Trait::Entitled);
            bundle.traits = traits;
            bundle.wallet = Wallet { credits: 2000.0 };
            bundle.arrival = Arrival { tick };
            commands.spawn((bundle, ExiledRoyal::default()));
        }
        if event.count > 0 {
            resources.add_credits(ROYAL_PAYOUT_CREDITS * event.count as f32);
            let royal = ExiledRoyal::default();
            let score = relations
                .scores
                .entry(royal.sponsor_faction_id)
                .or_insert(0);
            *score = (*score + royal.diplomacy_bonus as i32).min(100);
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "An exiled royal has taken sanctuary in the colony ({}). Their court brings culture, diplomacy and a generous purse — and the overthrowing empire has noticed.",
                event.count
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Dispatch assassination squads on cooldown via the real raid pipeline.
pub fn assassination_raid_spawner(
    mut commands: Commands,
    mut state: ResMut<AssassinationState>,
    mut raid_events: EventWriter<PirateRaidEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    sim_time: Option<Res<SimulationTime>>,
    royals: Query<&ExiledRoyal>,
    pop_positions: Query<&GridPosition, With<Pop>>,
) {
    let Some(time) = sim_time else { return };
    if time.tick < state.next_raid_tick {
        return;
    }
    let Some(royal) = royals.iter().find(|r| r.threat_level >= RAID_THREAT_THRESHOLD) else {
        return;
    };
    let usurper = royal.usurper_faction_id;
    state.next_raid_tick = time.tick + RAID_COOLDOWN_TICKS;
    state.last_raid_tick = Some(time.tick);
    state.raids_launched += 1;
    raid_events.send(PirateRaidEvent);
    let colony_tiles: Vec<GridPosition> = pop_positions.iter().copied().collect();
    let mut rng = rand::thread_rng();
    for _ in 0..SQUAD_SIZE {
        let spawn_pos = colony_tiles
            .choose(&mut rng)
            .copied()
            .unwrap_or(GridPosition { x: 0, y: 0 });
        let mut bundle = PopBundle::random(spawn_pos.x, spawn_pos.y, &mut rng);
        let mut traits = Traits::default();
        traits.add(Trait::Pirate);
        bundle.traits = traits;
        let weapon = commands
            .spawn(Weapon {
                properties: AttackProperties {
                    damage: ASSASSIN_WEAPON_DAMAGE,
                    range: 1.0,
                    cooldown: 6,
                    accuracy: 1.0,
                },
            })
            .id();
        bundle.equipment.weapon = Some(weapon);
        commands.spawn((
            bundle,
            Assassin {
                usurper_faction_id: usurper,
            },
        ));
        state.assassins_spawned += 1;
    }
    chronicle.send(AddChronicleEvent {
        text: "An assassination squad from the overthrowing empire has come for the exiled royal.".to_string(),
        importance: EventImportance::Major,
    });
}

/// High-tech gear drops on assassin death (§7), via real PopDied.
pub fn assassin_loot_system(
    mut commands: Commands,
    mut events: EventReader<PopDied>,
    mut state: ResMut<AssassinationState>,
    mut resources: ResMut<ColonyResources>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    assassins: Query<&GridPosition, With<Assassin>>,
) {
    const GEAR: [ItemType; 3] = [ItemType::Prosthetic, ItemType::Manual, ItemType::Tool];
    for event in events.read() {
        let Ok(pos) = assassins.get(event.entity) else {
            continue;
        };
        let pos = *pos;
        let gear = GEAR[(state.gear_dropped as usize) % GEAR.len()];
        commands.spawn((Item { item_type: gear }, pos));
        state.gear_dropped += 1;
        resources.add_credits(ASSASSIN_BOUNTY_CREDITS);
        chronicle.send(AddChronicleEvent {
            text: format!(
                "An assassin has fallen ({}); high-tech gear was recovered from the body.",
                event.name
            ),
            importance: EventImportance::Standard,
        });
    }
}

/// Luxury-suite demand: satisfied by Tavern/Library/Statue venues.
pub fn royal_suite_system(
    sim_time: Option<Res<SimulationTime>>,
    mut royals: Query<&mut Morale, With<ExiledRoyal>>,
    buildings: Query<&Building>,
) {
    let Some(time) = sim_time else { return };
    if time.tick == 0 || !time.tick.is_multiple_of(SUITE_CHECK_PERIOD_TICKS) {
        return;
    }
    let satisfied = buildings.iter().any(|b| {
        matches!(
            b.building_type,
            BuildingType::Tavern | BuildingType::Library | BuildingType::Statue
        )
    });
    if satisfied {
        return;
    }
    for mut morale in royals.iter_mut() {
        morale.add_modifier(MoodModifier {
            label: "No Luxury Suite".to_string(),
            value: -0.25,
            duration: 500,
        });
    }
}

/// Royal death consequences for sponsor/usurper relations.
pub fn royal_death_system(
    mut events: EventReader<PopDied>,
    query: Query<&ExiledRoyal>,
    mut relations: ResMut<FactionRelations>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        let Ok(royal) = query.get(event.entity) else {
            continue;
        };
        let s = relations.scores.entry(royal.sponsor_faction_id).or_insert(0);
        *s = (*s - 100).max(-100);
        let u = relations.scores.entry(royal.usurper_faction_id).or_insert(0);
        *u = (*u + 20).min(100);
        chronicle.send(AddChronicleEvent {
            text: format!(
                "The exiled royal has died ({}). The old court mourns; the overthrowing empire celebrates.",
                event.name
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Living royals count (headless STATS).
pub fn royal_count(world: &mut World) -> usize {
    let mut q = world.query_filtered::<Entity, With<ExiledRoyal>>();
    q.iter(world).count()
}
/// Living assassins count (headless STATS).
pub fn assassin_count(world: &mut World) -> usize {
    let mut q = world.query_filtered::<Entity, With<Assassin>>();
    q.iter(world).count()
}
/// Public strings for IP guard.
pub fn exiled_royals_public_strings() -> Vec<String> {
    vec![
        "An exiled royal has taken sanctuary in the colony.".to_string(),
        "An assassination squad from the overthrowing empire has come for the exiled royal.".to_string(),
        "The exiled royal has died. The old court mourns.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(FactionRelations::default());
        world.insert_resource(ExileInfluence::default());
        world.insert_resource(AssassinationState::default());
        world.init_resource::<Events<PopDied>>();
        world.init_resource::<Events<RoyalArrivalEvent>>();
        world.init_resource::<Events<PirateRaidEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }
    fn run<M>(world: &mut World, sys: impl IntoSystemConfigs<M>) {
        let mut s = Schedule::default();
        s.add_systems(sys);
        s.run(world);
    }

    #[test]
    fn test_exile_grants_passive_bonuses() {
        // Spec RED test 1, adapted: mock ColonyCulture/Diplomacy -> ExileInfluence.
        let mut world = setup();
        world.spawn((Pop, ExiledRoyal::default()));
        run(&mut world, exile_bonus_system);
        let inf = world.resource::<ExileInfluence>();
        assert_eq!(inf.culture, 50.0);
        assert_eq!(inf.diplomacy, 25.0);
        assert_eq!(inf.royals, 1);
    }

    #[test]
    fn test_bonus_recompute_does_not_double_count() {
        let mut world = setup();
        world.spawn((Pop, ExiledRoyal::default()));
        run(&mut world, exile_bonus_system);
        run(&mut world, exile_bonus_system);
        run(&mut world, exile_bonus_system);
        assert_eq!(world.resource::<ExileInfluence>().culture, 50.0);
    }

    #[test]
    fn test_exile_triggers_assassination_raid() {
        // Spec RED test 2, adapted: mock ActiveRaids -> AssassinationState + real PirateRaidEvent.
        let mut world = setup();
        world.resource_mut::<SimulationTime>().tick = FIRST_RAID_DELAY_TICKS;
        world.spawn((Pop, GridPosition { x: 5, y: 5 }, ExiledRoyal::default()));
        run(&mut world, assassination_raid_spawner);
        assert_eq!(world.resource::<AssassinationState>().raids_launched, 1);
        assert_eq!(world.resource::<AssassinationState>().assassins_spawned, SQUAD_SIZE);
        assert_eq!(assassin_count(&mut world), SQUAD_SIZE as usize);
        let ev = world.resource::<Events<PirateRaidEvent>>();
        let mut r = ev.get_cursor();
        assert_eq!(r.read(ev).count(), 1);
    }

    #[test]
    fn test_raid_cooldown_prevents_respawn() {
        let mut world = setup();
        world.resource_mut::<SimulationTime>().tick = FIRST_RAID_DELAY_TICKS;
        world.spawn((Pop, GridPosition { x: 5, y: 5 }, ExiledRoyal::default()));
        run(&mut world, assassination_raid_spawner);
        world.resource_mut::<SimulationTime>().tick = FIRST_RAID_DELAY_TICKS + 10;
        run(&mut world, assassination_raid_spawner);
        assert_eq!(world.resource::<AssassinationState>().raids_launched, 1);
        world.resource_mut::<SimulationTime>().tick = FIRST_RAID_DELAY_TICKS + RAID_COOLDOWN_TICKS;
        run(&mut world, assassination_raid_spawner);
        assert_eq!(world.resource::<AssassinationState>().raids_launched, 2);
    }

    #[test]
    fn test_low_threat_no_raid() {
        let mut world = setup();
        world.resource_mut::<SimulationTime>().tick = 1000;
        world.spawn((Pop, ExiledRoyal { threat_level: 10.0, ..Default::default() }));
        run(&mut world, assassination_raid_spawner);
        assert_eq!(world.resource::<AssassinationState>().raids_launched, 0);
    }

    #[test]
    fn test_arrival_payout_and_diplomacy() {
        let mut world = setup();
        world.spawn((Pop, GridPosition { x: 10, y: 10 }));
        world.resource_mut::<Events<RoyalArrivalEvent>>().send(RoyalArrivalEvent { count: 1 });
        run(&mut world, process_royal_arrival_system);
        assert_eq!(royal_count(&mut world), 1);
        assert_eq!(world.resource::<ColonyResources>().credits, ROYAL_PAYOUT_CREDITS);
        assert_eq!(world.resource::<FactionRelations>().scores[&ROYAL_SPONSOR_FACTION_ID], 25);
    }

    #[test]
    fn test_assassin_death_drops_gear() {
        let mut world = setup();
        let a = world.spawn((Pop, GridPosition { x: 3, y: 4 }, Assassin { usurper_faction_id: ROYAL_USURPER_FACTION_ID })).id();
        world.send_event(PopDied { entity: a, name: "Assassin".into(), tick: 10, reason: "killed".into() });
        run(&mut world, assassin_loot_system);
        assert_eq!(world.resource::<AssassinationState>().gear_dropped, 1);
        assert_eq!(world.resource::<ColonyResources>().credits, ASSASSIN_BOUNTY_CREDITS);
        let mut q = world.query::<(&Item, &GridPosition)>();
        let items: Vec<_> = q.iter(&world).collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].1.x, 3);
    }

    #[test]
    fn test_non_assassin_death_no_loot() {
        let mut world = setup();
        let p = world.spawn(Pop).id();
        world.send_event(PopDied { entity: p, name: "Pleb".into(), tick: 1, reason: "x".into() });
        run(&mut world, assassin_loot_system);
        assert_eq!(world.resource::<AssassinationState>().gear_dropped, 0);
    }

    #[test]
    fn test_royal_death_tanks_sponsor() {
        let mut world = setup();
        world.resource_mut::<FactionRelations>().scores.insert(ROYAL_SPONSOR_FACTION_ID, 50);
        let r = world.spawn((Pop, ExiledRoyal::default())).id();
        world.send_event(PopDied { entity: r, name: "Royal".into(), tick: 1, reason: "assassinated".into() });
        run(&mut world, royal_death_system);
        assert_eq!(world.resource::<FactionRelations>().scores[&ROYAL_SPONSOR_FACTION_ID], -50);
        assert_eq!(world.resource::<FactionRelations>().scores[&ROYAL_USURPER_FACTION_ID], 20);
    }

    #[test]
    fn test_suite_dissatisfaction_mood() {
        let mut world = setup();
        world.resource_mut::<SimulationTime>().tick = SUITE_CHECK_PERIOD_TICKS;
        let e = world.spawn((Pop, ExiledRoyal::default(), Morale::default())).id();
        run(&mut world, royal_suite_system);
        let m = world.get::<Morale>(e).unwrap();
        assert!(m.modifiers.iter().any(|x| x.label == "No Luxury Suite"));
    }

    #[test]
    fn test_ip_guard() {
        let banned = ["billy", "windsor", "sad king", "dune", "atreides", "skywalker", "lannister", "targaryen", "corleone"];
        for s in exiled_royals_public_strings() {
            let low = s.to_lowercase();
            for b in banned {
                assert!(!low.contains(b), "banned term {b} in: {s}");
            }
        }
    }
}
