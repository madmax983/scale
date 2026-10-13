//! Spec 348: The Black Market.
//!
//! Where the state fails, the underworld provides. The pre-existing
//! scaffolding in this module was a stub: `ColonyStats.unmet_luxury`
//! was fed by `update_unmet_luxury_system` (fixed in the same pass —
//! it compared the 0.0..=1.0 leisure need against 30.0, so it counted
//! every pop), the spawn system demanded `unmet_luxury > 50` (a
//! 51-pop colony) and then spawned an unbounded bare [`Smuggler`]
//! every tick, and the trade system added +1.0 corruption per tick
//! forever while delivering nothing and siphoning nothing.
//!
//! This pass wires the real loop in place, reusing the existing
//! types (no parallel `ColonyStats` / `Smuggler`):
//!
//! * [`update_unmet_luxury_system`](crate::layer1::integration::update_unmet_luxury_system)
//!   counts pops whose leisure need is genuinely low (< 0.3).
//! * [`black_market_spawn_system`] spawns a docked smuggler when at
//!   least [`UNMET_LUXURY_SPAWN_THRESHOLD`] pops are unmet, capped at
//!   [`MAX_DOCKED_SMUGGLERS`] with a [`SMUGGLER_SPAWN_COOLDOWN_TICKS`]
//!   cooldown. A smuggler is an orbital visitor entity carrying a
//!   [`SmugglerVisit`] docking timer — the Layer-1 world has no
//!   Layer-2 `Fleet`/`InOrbit` parent to attach to, and the existing
//!   `smuggler_arrival_event_bridge` already announces `Added<Smuggler>`
//!   to the void-weed trade network.
//! * [`smuggler_trade_system`] executes exactly one trade per visit
//!   when the docking timer expires: it delivers real luxury stock
//!   into [`ColonyResources`], relieves the leisure need of unmet
//!   pops, siphons credits at black-market markup, raises
//!   `ColonyStats.corruption` (clamped to 100), feeds the existing
//!   `SmugglingHeat` (which the void-weed systems turn into pirate
//!   raids), writes the first-docking chronicle entry, and the
//!   smuggler departs (despawns).
//! * Corruption does real work (spec REFACTOR): [`pay_wage`] skims
//!   every wage by [`corruption_efficiency_factor`] — at corruption
//!   100, half of every wage vanishes into the underworld before it
//!   reaches a wallet. It is also visible in headless STATS
//!   (`smugglers=` / `corruption=`).

use crate::layer1::chronicle::{Chronicle, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::law::justice::Wanted;
use crate::layer1::map::GridPosition;
use crate::layer1::needs::Needs;
use crate::layer1::pop::Pop;
use crate::layer1::void_weed::SmugglingHeat;
use crate::layer1::zone::{ZoneGrid, ZoneType};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Pops with unmet luxury needs required before a smuggler answers.
pub const UNMET_LUXURY_SPAWN_THRESHOLD: u32 = 3;
/// Maximum smugglers docked at the colony at once.
pub const MAX_DOCKED_SMUGGLERS: usize = 3;
/// Minimum ticks between smuggler spawns.
pub const SMUGGLER_SPAWN_COOLDOWN_TICKS: u64 = 100;
/// Ticks a smuggler spends docked before the trade executes.
pub const SMUGGLER_DOCK_TICKS: u32 = 25;
/// Luxury stock delivered per trade (into `ColonyResources.luxury`).
pub const SMUGGLER_LUXURY_DELIVERY: f32 = 5.0;
/// Credits siphoned per trade (black-market markup; capped at balance).
pub const SMUGGLER_CREDIT_SIPHON: f32 = 25.0;
/// Corruption added per completed smuggler trade.
pub const SMUGGLER_CORRUPTION_PER_TRADE: f32 = 2.0;
/// Smuggling heat added per completed trade (void-weed raid fuel).
pub const SMUGGLER_HEAT_PER_TRADE: f32 = 5.0;
/// Leisure relief granted to each unmet pop per trade.
pub const SMUGGLER_LEISURE_RELIEF: f32 = 0.3;
/// Leisure level below which a pop counts as luxury-unmet.
pub const UNMET_LEISURE_THRESHOLD: f32 = 0.3;
/// Chronicle text written the first time a smuggler completes a trade.
pub const FIRST_DOCKING_CHRONICLE_TEXT: &str =
    "A smuggler has docked in the shadows and sold luxury goods the colony could not provide. Credits are missing, and everyone knows why.";

#[derive(Resource, Default)]
/// Colony-wide statistics tracking economy and law.
pub struct ColonyStats {
    /// Measure of how many luxury needs are currently unmet.
    pub unmet_luxury: u32,
    /// Total corruption level in the colony (0.0..=100.0 in practice;
    /// pardons and sanctuary crime also feed this ledger).
    pub corruption: f32,
}

/// Ledger for the black-market loop (spec 348): lifetime counters the
/// headless `blackmarket` panel and STATS helpers read.
#[derive(Resource, Default, Debug, Clone)]
pub struct BlackMarketState {
    /// Smugglers spawned over the whole run.
    pub total_spawned: u32,
    /// Trades completed over the whole run.
    pub total_trades: u32,
    /// Credits siphoned by smugglers over the whole run.
    pub total_credits_siphoned: f32,
    /// Luxury stock delivered over the whole run.
    pub total_luxury_delivered: f32,
    /// Tick of the most recent spawn (cooldown bookkeeping).
    pub last_spawn_tick: Option<u64>,
    /// Whether the first-docking chronicle entry has been written.
    pub chronicled_first_docking: bool,
}

#[derive(Component)]
/// Marker component for a pop acting as a smuggler.
pub struct Smuggler;

/// Docking state for one smuggler visit: counts down to the trade,
/// then the smuggler departs. Smugglers spawned without a visit
/// (legacy/debug paths) trade immediately.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SmugglerVisit {
    /// Ticks remaining before the trade executes.
    pub ticks_until_trade: u32,
    /// Whether this visit's trade has already executed.
    pub traded: bool,
}

/// Fraction of a wage (or other payout) that survives corruption:
/// 1.0 at corruption 0, 0.5 at corruption 100+, linear between.
/// This is the spec's tax/production-efficiency hook — `pay_wage`
/// multiplies every wage by this factor.
#[must_use]
pub fn corruption_efficiency_factor(corruption: f32) -> f32 {
    1.0 - (corruption.clamp(0.0, 100.0) / 100.0) * 0.5
}

/// Number of smugglers currently docked (headless STATS helper).
pub fn smuggler_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<(), With<Smuggler>>();
    query.iter(world).count()
}

/// One-word black-market status for STATS: `none`, `docked`, or
/// `trading` once at least one trade has completed.
pub fn black_market_status_label(world: &mut World) -> String {
    let trades = world
        .get_resource::<BlackMarketState>()
        .map_or(0, |s| s.total_trades);
    if trades > 0 {
        "trading".to_string()
    } else if smuggler_count(world) > 0 {
        "docked".to_string()
    } else {
        "none".to_string()
    }
}

/// Spawns a docked smuggler when unmet luxury needs are high.
///
/// Gated by [`UNMET_LUXURY_SPAWN_THRESHOLD`], the
/// [`MAX_DOCKED_SMUGGLERS`] cap, and — when the ledger and clock are
/// present — the spawn cooldown. The ledger/clock are `Option` so
/// the spec's bare-world RED test (only `ColonyStats` inserted)
/// still spawns exactly one smuggler.
pub fn black_market_spawn_system(
    mut commands: Commands,
    stats: Res<ColonyStats>,
    existing: Query<(), With<Smuggler>>,
    time: Option<Res<SimulationTime>>,
    mut state: Option<ResMut<BlackMarketState>>,
) {
    if stats.unmet_luxury < UNMET_LUXURY_SPAWN_THRESHOLD {
        return;
    }
    if existing.iter().count() >= MAX_DOCKED_SMUGGLERS {
        return;
    }
    if let (Some(state), Some(time)) = (state.as_deref(), time.as_deref()) {
        if let Some(last) = state.last_spawn_tick {
            if time.tick.saturating_sub(last) < SMUGGLER_SPAWN_COOLDOWN_TICKS {
                return;
            }
        }
    }
    commands.spawn((
        Smuggler,
        SmugglerVisit {
            ticks_until_trade: SMUGGLER_DOCK_TICKS,
            traded: false,
        },
    ));
    if let Some(state) = state.as_deref_mut() {
        state.total_spawned += 1;
        if let Some(time) = time.as_deref() {
            state.last_spawn_tick = Some(time.tick);
        }
    }
}

/// Executes one trade per docked smuggler whose docking timer has
/// expired, then the smuggler departs. See the module docs for the
/// full delivery / siphon / corruption accounting.
#[allow(clippy::too_many_arguments)]
pub fn smuggler_trade_system(
    mut commands: Commands,
    mut stats: ResMut<ColonyStats>,
    mut smugglers: Query<(Entity, Option<&mut SmugglerVisit>), With<Smuggler>>,
    mut resources: Option<ResMut<ColonyResources>>,
    mut state: Option<ResMut<BlackMarketState>>,
    mut chronicle: Option<ResMut<Chronicle>>,
    time: Option<Res<SimulationTime>>,
    mut pops: Query<&mut Needs, With<Pop>>,
    mut heat: Option<ResMut<SmugglingHeat>>,
) {
    let mut ready: Vec<Entity> = Vec::new();
    for (entity, visit) in &mut smugglers {
        let is_ready = match visit {
            None => true,
            Some(mut visit) => {
                if visit.traded {
                    false
                } else if visit.ticks_until_trade > 0 {
                    visit.ticks_until_trade -= 1;
                    false
                } else {
                    visit.traded = true;
                    true
                }
            }
        };
        if is_ready {
            ready.push(entity);
        }
    }
    if ready.is_empty() {
        return;
    }

    for entity in ready {
        // Deliver real luxury stock; siphon credits at markup.
        let mut siphoned = 0.0f32;
        if let Some(resources) = resources.as_deref_mut() {
            resources.luxury =
                (resources.luxury + SMUGGLER_LUXURY_DELIVERY).min(resources.max_luxury);
            siphoned = resources.credits.min(SMUGGLER_CREDIT_SIPHON);
            resources.credits -= siphoned;
        }
        // The delivery is the point: unmet pops feel it immediately.
        for mut needs in &mut pops {
            if needs.leisure < UNMET_LEISURE_THRESHOLD {
                needs.leisure = (needs.leisure + SMUGGLER_LEISURE_RELIEF).min(1.0);
            }
        }
        stats.corruption = (stats.corruption + SMUGGLER_CORRUPTION_PER_TRADE).min(100.0);
        if let Some(heat) = heat.as_deref_mut() {
            heat.level += SMUGGLER_HEAT_PER_TRADE;
        }
        if let Some(state) = state.as_deref_mut() {
            state.total_trades += 1;
            state.total_credits_siphoned += siphoned;
            state.total_luxury_delivered += SMUGGLER_LUXURY_DELIVERY;
            if !state.chronicled_first_docking {
                state.chronicled_first_docking = true;
                if let Some(chronicle) = chronicle.as_deref_mut() {
                    let tick = time.as_deref().map_or(0, |t| t.tick);
                    chronicle.add_event(
                        tick,
                        FIRST_DOCKING_CHRONICLE_TEXT.to_string(),
                        EventImportance::Major,
                    );
                }
            }
        }
        commands.entity(entity).despawn();
    }
}

#[derive(Event)]
pub struct SmugglerArrivalEvent {
    pub intensity: u32,
}

#[derive(Component)]
pub struct DropNode {
    pub stored_metal: f32,
}

#[derive(Component)]
pub struct Desperate;

#[derive(Component)]
pub struct ContrabandUser;

#[derive(Event)]
pub struct ShutdownDropNodeEvent {
    pub node_entity: Entity,
}

pub fn handle_smuggler_arrival(
    mut commands: Commands,
    mut events: EventReader<SmugglerArrivalEvent>,
) {
    for _ in events.read() {
        commands.spawn(DropNode { stored_metal: 0.0 });
    }
}

pub fn pop_smuggling_system(
    mut colony_resources: ResMut<crate::layer1::economy::resources::ColonyResources>,
    mut drop_nodes: Query<&mut DropNode>,
    mut desperate_pops: Query<
        (Entity, &mut crate::layer1::social::morale::Morale),
        With<Desperate>,
    >,
    mut commands: Commands,
) {
    if let Ok(mut node) = drop_nodes.get_single_mut() {
        for (pop_entity, mut morale) in desperate_pops.iter_mut() {
            if colony_resources.metal >= 5.0 && node.stored_metal < 500.0 {
                colony_resources.metal -= 5.0;
                node.stored_metal += 5.0;

                // Morale boost from contraband
                morale.add_modifier(crate::layer1::social::morale::MoodModifier {
                    label: "Contraband High".to_string(),
                    value: 0.2,
                    duration: 100,
                });

                commands.entity(pop_entity).insert(ContrabandUser);
            }
        }
    }
}

pub fn shutdown_drop_node_system(
    mut commands: Commands,
    mut events: EventReader<ShutdownDropNodeEvent>,
    nodes: Query<&DropNode>,
    mut colony_resources: ResMut<crate::layer1::economy::resources::ColonyResources>,
    mut contraband_users: Query<&mut crate::layer1::social::morale::Morale, With<ContrabandUser>>,
) {
    for event in events.read() {
        if let Ok(node) = nodes.get(event.node_entity) {
            // Refund stolen resources up to max capacity
            colony_resources.metal =
                (colony_resources.metal + node.stored_metal).min(colony_resources.max_metal);
            commands.entity(event.node_entity).despawn();

            for mut morale in contraband_users.iter_mut() {
                morale.add_modifier(crate::layer1::social::morale::MoodModifier {
                    label: "Contraband Withdrawal".to_string(),
                    value: -0.5,
                    duration: 500,
                });
            }
        }
    }
}

pub fn black_market_economy_system(
    wanted_query: Query<(&GridPosition, &Wanted), With<Pop>>,
    zone_grid: Res<ZoneGrid>,
    mut stats: ResMut<ColonyStats>,
) {
    let mut illegal_goods_generated = 0.0;

    for (pos, wanted) in wanted_query.iter() {
        if zone_grid.get(pos.x, pos.y) == ZoneType::Sanctuary {
            illegal_goods_generated += wanted.severity * 0.5; // Example calculation
        }
    }

    if illegal_goods_generated > 0.0 {
        stats.corruption = (stats.corruption + illegal_goods_generated).min(100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_unmet_needs_spawn_smugglers() {
        // Arrange
        let mut world = World::new();
        // Setup colony with massive unmet luxury needs
        world.insert_resource(ColonyStats {
            unmet_luxury: 100,
            ..Default::default()
        });

        // Act
        world.run_system_once(black_market_spawn_system).unwrap();

        // Assert
        let smugglers = world.query::<&Smuggler>().iter(&world).count();
        assert_eq!(smugglers, 1, "A smuggler should spawn due to unmet needs");
    }

    #[test]
    fn test_smuggler_increases_corruption() {
        // Arrange
        let mut world = World::new();
        world.insert_resource(ColonyStats {
            corruption: 0.0,
            ..Default::default()
        });
        let _smuggler = world.spawn(Smuggler).id();

        // Act
        world.run_system_once(smuggler_trade_system).unwrap(); // Mock trade

        // Assert
        let stats = world.get_resource::<ColonyStats>().unwrap();
        assert!(
            stats.corruption > 0.0,
            "Corruption should increase after smuggler trade"
        );
    }

    use crate::layer1::economy::resources::ColonyResources;
    use crate::layer1::social::morale::Morale;
    use bevy::prelude::{App, Update};

    #[test]
    fn test_smugglers_establish_drop_node() {
        // Arrange
        let mut app = App::new();
        app.add_event::<SmugglerArrivalEvent>();
        app.add_systems(Update, handle_smuggler_arrival);

        // Act
        app.world_mut()
            .send_event(SmugglerArrivalEvent { intensity: 1 });
        app.update();

        // Assert
        let mut query = app.world_mut().query::<&DropNode>();
        assert_eq!(
            query.iter(app.world()).count(),
            1,
            "A Drop Node should be established upon smuggler arrival"
        );
    }

    #[test]
    fn test_desperate_pop_exchanges_resources_for_contraband() {
        // Arrange
        let mut app = App::new();
        let res = ColonyResources {
            metal: 50.0,
            ..Default::default()
        };
        app.insert_resource(res);

        app.add_systems(Update, pop_smuggling_system);

        let drop_node_entity = app.world_mut().spawn(DropNode { stored_metal: 0.0 }).id();
        let pop_entity = app
            .world_mut()
            .spawn((
                Pop,
                Morale::default(),
                Desperate, // Tag indicating they are likely to smuggle
            ))
            .id();

        // Act
        app.update();

        // Assert
        let colony_resources = app.world().resource::<ColonyResources>();
        assert!(
            colony_resources.metal < 50.0,
            "Colony alloys should have been stolen"
        );

        let drop_node = app.world().get::<DropNode>(drop_node_entity).unwrap();
        assert!(
            drop_node.stored_metal > 0.0,
            "Drop Node should have accumulated stolen alloys"
        );

        let pop_morale = app.world().get::<Morale>(pop_entity).unwrap();
        assert!(
            pop_morale
                .modifiers
                .iter()
                .any(|m| m.label == "Contraband High"),
            "Pop should have received a morale boost from contraband"
        );
    }

    #[test]
    fn test_shutting_down_drop_node_returns_resources_and_crashes_morale() {
        // Arrange
        let mut app = App::new();
        let res = ColonyResources {
            metal: 50.0,
            max_metal: 1000.0,
            ..Default::default()
        };
        app.insert_resource(res);
        app.add_event::<ShutdownDropNodeEvent>();
        app.add_systems(Update, shutdown_drop_node_system);

        let drop_node_entity = app
            .world_mut()
            .spawn(DropNode {
                stored_metal: 100.0,
            })
            .id();
        let pop_entity = app
            .world_mut()
            .spawn((Pop, Morale::default(), ContrabandUser))
            .id();

        // Act
        app.world_mut().send_event(ShutdownDropNodeEvent {
            node_entity: drop_node_entity,
        });
        app.update();

        // Assert
        assert!(
            app.world().get::<DropNode>(drop_node_entity).is_none(),
            "Drop Node should be destroyed"
        );

        let colony_resources = app.world().resource::<ColonyResources>();
        assert_eq!(
            colony_resources.metal, 150.0,
            "Stored alloys should be returned to the colony"
        );

        let pop_morale = app.world().get::<Morale>(pop_entity).unwrap();
        assert!(
            pop_morale
                .modifiers
                .iter()
                .any(|m| m.label == "Contraband Withdrawal"),
            "Pop morale should crash when their contraband supply is cut off"
        );
    }

    #[test]
    fn test_black_market_economy_system() {
        let mut world = World::new();

        let mut zone_grid = ZoneGrid::new(10, 10);
        zone_grid.set(5, 5, ZoneType::Sanctuary); // Sanctuary at 5,5
        world.insert_resource(zone_grid);

        world.insert_resource(ColonyStats {
            unmet_luxury: 0,
            corruption: 0.0,
        });

        // Wanted pop in Sanctuary
        world.spawn((Pop, GridPosition { x: 5, y: 5 }, Wanted { severity: 2.0 }));

        // Wanted pop outside Sanctuary
        world.spawn((Pop, GridPosition { x: 0, y: 0 }, Wanted { severity: 2.0 }));

        world.run_system_once(black_market_economy_system).unwrap();

        let stats = world.get_resource::<ColonyStats>().unwrap();
        assert_eq!(
            stats.corruption, 1.0,
            "Wanted Pops in Sanctuary should generate corruption."
        );
    }

    // ---- Spec 348: wired black-market loop ----

    #[test]
    fn test_spawn_requires_real_unmet_threshold() {
        let mut world = World::new();
        world.insert_resource(ColonyStats {
            unmet_luxury: UNMET_LUXURY_SPAWN_THRESHOLD - 1,
            ..Default::default()
        });
        world.run_system_once(black_market_spawn_system).unwrap();
        assert_eq!(
            world.query::<&Smuggler>().iter(&world).count(),
            0,
            "Below the unmet threshold, no smuggler should answer"
        );
    }

    #[test]
    fn test_spawn_respects_docked_cap() {
        let mut world = World::new();
        world.insert_resource(ColonyStats {
            unmet_luxury: 100,
            ..Default::default()
        });
        for _ in 0..MAX_DOCKED_SMUGGLERS {
            world.spawn(Smuggler);
        }
        world.run_system_once(black_market_spawn_system).unwrap();
        assert_eq!(
            world.query::<&Smuggler>().iter(&world).count(),
            MAX_DOCKED_SMUGGLERS,
            "Docked smugglers are capped"
        );
    }

    #[test]
    fn test_trade_waits_for_docking_timer() {
        let mut world = World::new();
        world.insert_resource(ColonyStats::default());
        world.insert_resource(ColonyResources::default());
        world.spawn((
            Smuggler,
            SmugglerVisit {
                ticks_until_trade: 5,
                traded: false,
            },
        ));
        world.run_system_once(smuggler_trade_system).unwrap();
        let stats = world.get_resource::<ColonyStats>().unwrap();
        assert_eq!(stats.corruption, 0.0, "No trade before docking completes");
        assert_eq!(
            world.query::<&Smuggler>().iter(&world).count(),
            1,
            "Smuggler still docked"
        );
    }

    #[test]
    fn test_trade_delivers_luxury_siphons_credits_and_departs() {
        let mut world = World::new();
        world.insert_resource(ColonyStats::default());
        world.insert_resource(ColonyResources {
            credits: 100.0,
            luxury: 0.0,
            ..Default::default()
        });
        world.insert_resource(BlackMarketState::default());
        let smuggler = world
            .spawn((
                Smuggler,
                SmugglerVisit {
                    ticks_until_trade: 0,
                    traded: false,
                },
            ))
            .id();
        // One unmet pop to receive leisure relief.
        let pop = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.0,
                    ..Default::default()
                },
            ))
            .id();

        world.run_system_once(smuggler_trade_system).unwrap();

        let resources = world.get_resource::<ColonyResources>().unwrap();
        assert_eq!(resources.luxury, SMUGGLER_LUXURY_DELIVERY);
        assert_eq!(resources.credits, 100.0 - SMUGGLER_CREDIT_SIPHON);
        let stats = world.get_resource::<ColonyStats>().unwrap();
        assert_eq!(stats.corruption, SMUGGLER_CORRUPTION_PER_TRADE);
        let state = world.get_resource::<BlackMarketState>().unwrap();
        assert_eq!(state.total_trades, 1);
        assert_eq!(state.total_credits_siphoned, SMUGGLER_CREDIT_SIPHON);
        let needs = world.get::<Needs>(pop).unwrap();
        assert!(
            needs.leisure >= SMUGGLER_LEISURE_RELIEF,
            "Delivery relieves the unmet leisure need"
        );
        assert!(
            world.get::<Smuggler>(smuggler).is_none(),
            "Smuggler departs after trading"
        );
    }

    #[test]
    fn test_first_trade_writes_chronicle_once() {
        let mut world = World::new();
        world.insert_resource(ColonyStats::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(BlackMarketState::default());
        world.insert_resource(Chronicle::default());
        world.insert_resource(SimulationTime::default());
        world.spawn(Smuggler);
        world.run_system_once(smuggler_trade_system).unwrap();
        world.spawn(Smuggler);
        world.run_system_once(smuggler_trade_system).unwrap();
        let chronicle = world.get_resource::<Chronicle>().unwrap();
        assert_eq!(
            chronicle.events.len(),
            1,
            "Only the first docking is chronicled"
        );
        assert_eq!(chronicle.events[0].importance, EventImportance::Major);
        assert!(chronicle.events[0].text.contains("smuggler"));
    }

    #[test]
    fn test_corruption_efficiency_factor_bounds() {
        assert_eq!(corruption_efficiency_factor(0.0), 1.0);
        assert_eq!(corruption_efficiency_factor(100.0), 0.5);
        assert_eq!(corruption_efficiency_factor(1_000.0), 0.5);
        assert_eq!(corruption_efficiency_factor(-5.0), 1.0);
    }

    #[test]
    fn test_black_market_public_strings_are_original() {
        // IP guard (concept-only): public strings use generic
        // underworld language, no lifted names or franchises.
        let banned = ["coruscant", "nar shaddaa", "kessel", "hutt", "cartel boss"];
        assert!(!FIRST_DOCKING_CHRONICLE_TEXT
            .to_lowercase()
            .contains(banned[0]));
        for term in banned {
            assert!(
                !FIRST_DOCKING_CHRONICLE_TEXT.to_lowercase().contains(term),
                "banned term in chronicle text: {term}"
            );
        }
    }
}
