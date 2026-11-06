//! Nomadic Fleets (Spec 397).
//!
//! Layer 3 ark fleets that do not hold planets: they enter the
//! player's Layer 2 system, open a brief trading/patience window,
//! and — if ignored — strip-mine the system's asteroid belt,
//! permanently depleting it, before moving on.
//!
//! ## Adaptations from the mock-style spec (§8 grounding)
//!
//! * **Spec 373 already built a nomad fleet.** The Void Nomad Fleet
//!   ([`NomadFleet`][crate::layer2::entities::nomad_fleet::NomadFleet],
//!   spec 373) is real: random 1%/tick arrivals, goal-directed
//!   movement over its own system-map nodes, strip-mining of
//!   `NodeResources` and of real [`MiningTarget`] asteroids, an
//!   overwhelming combat composition, and a temporary
//!   [`NomadTradeMarket`] carried on the fleet. Spec 397 does NOT
//!   duplicate any of that. What 373 lacks — and this module adds —
//!   is the *lifecycle*: a [`FleetState`] machine
//!   (Arriving → Trading → Mining → Departing), a patience window,
//!   departure (despawn) when the target is depleted or patience
//!   expires, a scheduled late arrival, and the §5 diplomacy leg.
//!   Fleets spawned here carry the real 373 trade market and combat
//!   composition (so attacking one is still suicide through the
//!   real combat seam), but deliberately do NOT carry the 373
//!   `NomadFleet` mining component — this module's target-based
//!   mining is the only mining they do, so no stock is double-taken.
//! * **`AsteroidBelt` did not exist.** The spec sketch imports
//!   `layer2::system::AsteroidBelt { resources_remaining }`; the
//!   real Layer 2 system had only the [`SystemBody`] marker,
//!   [`OrbitalBody`], and per-asteroid [`MiningTarget`] amounts
//!   (spec 101 system mining). The belt component now exists at
//!   exactly the spec's path in `layer2::system`, as the belt-wide
//!   aggregate stock. [`nomad_mining_system`] depletes it — and,
//!   when the same entity also carries a [`MiningTarget`] (the
//!   headless belt spawner pairs them), depletes that real target
//!   by the same taken amount, so belt and per-asteroid stock stay
//!   in sync and the stock the game's own fleets mine is the stock
//!   the nomads take. A target with only a `MiningTarget` (no belt)
//!   is mined from the target alone. No parallel mining economy.
//! * **Amounts are `f32`, patience stays `u32`.** The sketch's
//!   `resources_remaining: u32` / `mining_rate: u32` are adapted to
//!   the real seams' native `f32` (MiningTarget amounts,
//!   NodeResources, ColonyResources are all f32 — the spec 396
//!   pattern), so depletion is exact with no cast rounding. The
//!   spec's 500 − 50 = 450 test runs as 500.0 − 50.0 = 450.0.
//! * **Arrival is scheduled, not `tick == 5000` forever.** The
//!   sketch spawns at `SimulationTime { tick: 5000 }`. This module
//!   keeps tick 5000 as [`NOMADIC_ARRIVAL_TICK`] but routes it
//!   through a [`NomadicDirector`] that fires exactly once (the
//!   sketch's bare equality would also fire only once in a
//!   monotonic clock, but the director makes the intent explicit
//!   and testable, and a tick skipped by a speed step cannot lose
//!   the arrival: the check is `tick >= NOMADIC_ARRIVAL_TICK`).
//!   The 373 random arrivals continue independently.
//! * **Diplomacy (§5) IS cheap on the real seams — so it is built.**
//!   Offering resources so the fleet leaves the belt alone needs
//!   only: the real [`ColonyResources`] ledger (`try_consume`,
//!   the spec 396 delivery seam), a [`FactionRelations`] score
//!   keyed by `u32` faction id (the spec 396/383 pattern; the
//!   nomads are faction [`NOMADIC_FACTION_ID`]), and the chronicle.
//!   [`offer_nomadic_bribe_system`] consumes the offered goods,
//!   sends the fleet `Departing`, and warms relations. An offer
//!   below [`BRIBE_MIN_AMOUNT`], or one the ledger cannot cover,
//!   is refused with no state change. The §5 trade leg rides the
//!   fleet's real 373 [`NomadTradeMarket`] (opened on spawn,
//!   dies with the fleet) — no new market is invented. §5 combat
//!   ("overwhelming ancient weaponry") and §5 physical movement
//!   are already real in 373 and are not rebuilt here; the
//!   Arriving/Departing states here are transit phases whose exit
//!   is despawn (documented, not silently skipped).
//! * **ZERO balance retunes.** No existing constant is changed;
//!   every constant below is new for spec 397.
//!
//! All names and strings are original and generic — no borrowed
//! IP (see the IP-guard test).

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::{ColonyResources, ResourceType};
use crate::layer2::entities::nomad_fleet::{nomad_fleet_composition, NomadTradeMarket};
use crate::layer2::fleet::{Fleet, FleetFaction};
use crate::layer2::mining::MiningTarget;
use crate::layer2::system::AsteroidBelt;
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::time::SimulationTime;

// ---------------------------------------------------------------------------
// Tuning constants (new for spec 397; ZERO existing constants retuned).
// Magnitudes grounded in the real economy / prior specs: the spec
// GREEN example mines at 10/tick with 1000 patience; a scion
// allowance is 50cr/1000 ticks and a contract delivery is ~10
// units — a 10-unit gift is a modest, real price for a belt.
// ---------------------------------------------------------------------------

/// Tick of the scheduled nomadic arrival (the spec's tick 5000).
pub const NOMADIC_ARRIVAL_TICK: u64 = 5000;
/// Default strip-mining rate (the spec GREEN example's 10).
pub const DEFAULT_NOMADIC_MINING_RATE: f32 = 10.0;
/// Default patience in ticks (the spec GREEN example's 1000):
/// how long the fleet stays before moving on.
pub const DEFAULT_NOMADIC_PATIENCE: u32 = 1000;
/// Layer-3 faction id of the nomadic fleets in [`FactionRelations`]
/// (1–3 are contract factions, 7 the Homeworld, 13 the pirate
/// raiders, 21 the marriage faction — 22 is unused).
pub const NOMADIC_FACTION_ID: u32 = 22;
/// Minimum offered amount for a bribe to be accepted (§5).
pub const BRIBE_MIN_AMOUNT: f32 = 10.0;
/// Faction-relations gain when a bribe is accepted.
pub const BRIBE_RELATIONS_BONUS: i32 = 15;

/// Lifecycle state of a [`NomadicFleet`] (spec type, name kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetState {
    /// Entering the system (one tick, then Trading).
    Arriving,
    /// Present and trading; the patience window is open. Moves to
    /// Mining as soon as a belt with resources exists.
    Trading,
    /// Strip-mining the targeted belt / asteroid entity.
    Mining {
        /// The belt (or bare mining target) being depleted.
        target: Entity,
    },
    /// Leaving the system; despawned by the departure system.
    Departing,
}

/// A Layer 3 nomadic ark fleet in the player's system (spec
/// component, name kept; `mining_rate` adapted to f32 — see the
/// module docs).
#[derive(Component, Debug, Clone, Copy)]
pub struct NomadicFleet {
    /// Current lifecycle state.
    pub state: FleetState,
    /// Stock strip-mined per tick while Mining.
    pub mining_rate: f32,
    /// Ticks until the fleet leaves on its own. Decremented in
    /// Trading and Mining; at 0 the fleet departs.
    pub patience: u32,
}

impl NomadicFleet {
    /// A freshly arrived fleet with the spec's default rate and
    /// patience, in the Arriving state.
    #[must_use]
    pub fn arriving() -> Self {
        Self {
            state: FleetState::Arriving,
            mining_rate: DEFAULT_NOMADIC_MINING_RATE,
            patience: DEFAULT_NOMADIC_PATIENCE,
        }
    }
}

/// Event: offer goods from the colony ledger to a nomadic fleet
/// so it leaves the belt alone (§5 diplomacy).
#[derive(Event, Debug, Clone, Copy)]
pub struct OfferNomadicBribeEvent {
    /// The fleet to buy off.
    pub fleet: Entity,
    /// Resource offered (a real colony resource type).
    pub resource: ResourceType,
    /// Amount offered; must be at least [`BRIBE_MIN_AMOUNT`] and
    /// covered by the ledger, or the offer is refused.
    pub amount: f32,
}

/// Event: a nomadic fleet has departed the system.
#[derive(Event, Debug, Clone, Copy)]
pub struct NomadicDepartedEvent {
    /// The departed fleet entity (already despawned).
    pub fleet: Entity,
    /// Total stock that fleet's visit strip-mined (ledger snapshot
    /// is authoritative; this is the departing fleet's own count
    /// is not tracked per-fleet, so this carries the visit total).
    pub total_mined: f32,
}

/// Scheduled-arrival director: fires the tick-5000 arrival once.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct NomadicDirector {
    /// Whether the scheduled arrival has already fired.
    pub scheduled_arrival_done: bool,
}

/// Ledger of nomadic visits (headless STATS / panel).
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct NomadicLedger {
    /// Fleets spawned (scheduled, debug, or direct).
    pub arrivals: u64,
    /// Total belt/target stock strip-mined by nomadic fleets.
    pub total_mined: f32,
    /// Fleets that have departed.
    pub departures: u64,
    /// Bribes accepted (§5).
    pub bribes_accepted: u64,
    /// Total goods paid out in accepted bribes.
    pub bribes_paid: f32,
}

/// Generic name for the nomadic fleets in chronicles.
#[must_use]
pub fn nomadic_fleet_name() -> &'static str {
    "the Nomadic Fleet"
}

/// Spawn the full nomadic-fleet bundle (used by the arrival system
/// and the world-level helper): the spec component plus the real
/// 373 seams — the `Fleet` combat identity with its overwhelming
/// composition and the temporary trade market.
fn nomadic_bundle() -> impl Bundle {
    (
        NomadicFleet::arriving(),
        Fleet,
        FleetFaction::Merchant,
        nomad_fleet_composition(),
        NomadTradeMarket::nomad_goods(),
    )
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Scheduled arrival (spec system, name kept — pluralised in the
/// spec sketch as `spawn_nomad_fleets_system`): at
/// [`NOMADIC_ARRIVAL_TICK`], exactly once, one ark fleet enters
/// the system in the Arriving state and is chronicled.
pub fn spawn_nomad_fleets_system(
    time: Res<SimulationTime>,
    mut director: ResMut<NomadicDirector>,
    mut ledger: ResMut<NomadicLedger>,
    mut commands: Commands,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    if director.scheduled_arrival_done || time.tick < NOMADIC_ARRIVAL_TICK {
        return;
    }
    director.scheduled_arrival_done = true;
    ledger.arrivals += 1;
    commands.spawn(nomadic_bundle());
    chronicle.send(AddChronicleEvent {
        text: "NOMADIC FLEET: Ark ships of the Nomadic Fleet have entered the system. Their market is open — for now. If they are ignored, they will strip-mine the asteroid belt and move on.".to_string(),
        importance: EventImportance::Major,
    });
}

/// Strip-mining (spec system, name kept): a fleet in
/// `Mining { target }` permanently depletes its target — the
/// belt's `resources_remaining` when the target carries an
/// [`AsteroidBelt`], keeping a co-located real [`MiningTarget`]
/// in sync by the same taken amount; a bare `MiningTarget`
/// target is depleted directly. Depletion (or a vanished target)
/// sends the fleet Departing. Nothing here ever refills a belt:
/// the loss is permanent.
pub fn nomad_mining_system(
    mut fleets: Query<&mut NomadicFleet>,
    mut belts: Query<(&mut AsteroidBelt, Option<&mut MiningTarget>)>,
    mut targets: Query<&mut MiningTarget, Without<AsteroidBelt>>,
    mut ledger: ResMut<NomadicLedger>,
) {
    for mut fleet in fleets.iter_mut() {
        let FleetState::Mining { target } = fleet.state else {
            continue;
        };
        let rate = fleet.mining_rate;
        if let Ok((mut belt, mining_target)) = belts.get_mut(target) {
            if belt.resources_remaining <= 0.0 {
                fleet.state = FleetState::Departing;
                continue;
            }
            let taken = rate.min(belt.resources_remaining);
            belt.resources_remaining -= taken;
            if let Some(mut mining_target) = mining_target {
                mining_target.amount = (mining_target.amount - taken).max(0.0);
            }
            ledger.total_mined += taken;
            if belt.resources_remaining <= 0.0 {
                fleet.state = FleetState::Departing;
            }
        } else if let Ok(mut mining_target) = targets.get_mut(target) {
            if mining_target.amount <= 0.0 {
                fleet.state = FleetState::Departing;
                continue;
            }
            let taken = rate.min(mining_target.amount);
            mining_target.amount -= taken;
            ledger.total_mined += taken;
            if mining_target.amount <= 0.0 {
                fleet.state = FleetState::Departing;
            }
        } else {
            // Target destroyed / despawned: nothing left to mine.
            fleet.state = FleetState::Departing;
        }
    }
}

/// Diplomacy (§5): an accepted bribe consumes the offered goods
/// from the real colony ledger, warms [`FactionRelations`], and
/// sends the fleet Departing — it leaves the belt alone. Offers
/// below [`BRIBE_MIN_AMOUNT`], offers the ledger cannot cover,
/// and offers to fleets already Departing are refused silently
/// (no state changes at all).
pub fn offer_nomadic_bribe_system(
    mut events: EventReader<OfferNomadicBribeEvent>,
    mut fleets: Query<&mut NomadicFleet>,
    mut resources: ResMut<ColonyResources>,
    mut relations: ResMut<FactionRelations>,
    mut ledger: ResMut<NomadicLedger>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        if event.amount < BRIBE_MIN_AMOUNT {
            continue;
        }
        let Ok(mut fleet) = fleets.get_mut(event.fleet) else {
            continue;
        };
        if fleet.state == FleetState::Departing {
            continue;
        }
        if !resources.try_consume(event.resource, event.amount) {
            continue;
        }
        fleet.state = FleetState::Departing;
        let score = relations.scores.entry(NOMADIC_FACTION_ID).or_insert(0);
        *score = (*score + BRIBE_RELATIONS_BONUS).clamp(-100, 100);
        ledger.bribes_accepted += 1;
        ledger.bribes_paid += event.amount;
        chronicle.send(AddChronicleEvent {
            text: format!(
                "NOMADIC FLEET: {} accepts {:.0} {:?} and departs, leaving the asteroid belt alone.",
                nomadic_fleet_name(),
                event.amount,
                event.resource,
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Departure & lifecycle (spec system, name kept): Arriving
/// fleets settle into Trading; Trading fleets with a live belt
/// to mine move to Mining (the richest belt first); patience
/// burns in Trading and Mining; a fleet that is Departing or
/// out of patience is despawned, counted, and chronicled.
pub fn evaluate_nomad_departure_system(
    mut commands: Commands,
    mut fleets: Query<(Entity, &mut NomadicFleet)>,
    belts: Query<(Entity, &AsteroidBelt)>,
    targets: Query<(Entity, &MiningTarget), Without<AsteroidBelt>>,
    mut ledger: ResMut<NomadicLedger>,
    mut departed: EventWriter<NomadicDepartedEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    // Richest live belt (a belt entity is also a valid bare
    // MiningTarget host, so belts are considered first).
    let richest_belt = belts
        .iter()
        .filter(|(_, belt)| belt.resources_remaining > 0.0)
        .max_by(|(_, a), (_, b)| {
            a.resources_remaining
                .partial_cmp(&b.resources_remaining)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(entity, _)| entity);
    let richest_target = targets
        .iter()
        .filter(|(_, target)| target.amount > 0.0)
        .max_by(|(_, a), (_, b)| {
            a.amount.partial_cmp(&b.amount).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(entity, _)| entity);
    let mining_target = richest_belt.or(richest_target);

    for (entity, mut fleet) in fleets.iter_mut() {
        match fleet.state {
            FleetState::Departing => {
                commands.entity(entity).despawn();
                ledger.departures += 1;
                departed.send(NomadicDepartedEvent {
                    fleet: entity,
                    total_mined: ledger.total_mined,
                });
                chronicle.send(AddChronicleEvent {
                    text: "NOMADIC FLEET: The ark ships have left the system.".to_string(),
                    importance: EventImportance::Standard,
                });
            }
            FleetState::Arriving => {
                fleet.state = FleetState::Trading;
            }
            FleetState::Trading | FleetState::Mining { .. } => {
                if fleet.patience == 0 {
                    fleet.state = FleetState::Departing;
                    continue;
                }
                fleet.patience = fleet.patience.saturating_sub(1);
                if fleet.state == FleetState::Trading {
                    if let Some(target) = mining_target {
                        fleet.state = FleetState::Mining { target };
                    } else if fleet.patience == 0 {
                        fleet.state = FleetState::Departing;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// World-level helpers (headless console + tests)
// ---------------------------------------------------------------------------

/// Spawn a nomadic fleet directly against the world (the
/// headless/debug path; the live path is
/// [`spawn_nomad_fleets_system`]). Also counts the arrival in
/// the ledger when one is present.
pub fn spawn_nomadic_fleet(world: &mut World) -> Entity {
    let entity = world.spawn(nomadic_bundle()).id();
    if let Some(mut ledger) = world.get_resource_mut::<NomadicLedger>() {
        ledger.arrivals += 1;
    }
    entity
}

/// Spawn an asteroid-belt entity against the world: a real
/// [`SystemBody`] carrying an [`AsteroidBelt`] aggregate paired
/// with a real [`MiningTarget`] of the same stock (Ore), so the
/// belt the nomads deplete is the stock the game's own fleets
/// mine. Returns the belt entity.
pub fn spawn_asteroid_belt(world: &mut World, amount: f32) -> Entity {
    use crate::layer2::system::SystemBody;
    let entity = world
        .spawn((
            SystemBody,
            AsteroidBelt {
                resources_remaining: amount,
            },
            MiningTarget {
                resource_type: ResourceType::Ore,
                amount,
                mining_difficulty: 1.0,
            },
        ))
        .id();
    entity
}

/// All nomadic fleets in entity order, cloned for display.
#[must_use]
pub fn nomadic_rows(world: &mut World) -> Vec<(Entity, NomadicFleet)> {
    let mut rows: Vec<(Entity, NomadicFleet)> = world
        .query::<(Entity, &NomadicFleet)>()
        .iter(world)
        .map(|(e, f)| (e, *f))
        .collect();
    rows.sort_by_key(|(e, _)| e.index());
    rows
}

/// Total belt stock remaining across all belt entities.
#[must_use]
pub fn belt_remaining(world: &mut World) -> f32 {
    world
        .query::<&AsteroidBelt>()
        .iter(world)
        .map(|belt| belt.resources_remaining)
        .sum()
}

/// Census for STATS: `fleets<live>/belt<remaining>/mined<total>`,
/// or `none` when no fleet is present, no belt exists, and
/// nothing was ever mined.
#[must_use]
pub fn nomadic_stats_label(world: &mut World) -> String {
    let fleets = nomadic_rows(world).len();
    let belt = belt_remaining(world);
    let mined = world
        .get_resource::<NomadicLedger>()
        .map_or(0.0, |ledger| ledger.total_mined);
    if fleets == 0 && belt <= 0.0 && mined <= 0.0 {
        "none".to_string()
    } else {
        format!("fleets{fleets}/belt{belt:.0}/mined{mined:.0}")
    }
}

/// Public-facing strings, for the IP-guard test.
#[must_use]
pub fn nomadic_public_strings() -> Vec<String> {
    vec![
        "Nomadic Fleets".to_string(),
        nomadic_fleet_name().to_string(),
        "NOMADIC FLEET: Ark ships of the Nomadic Fleet have entered the system. Their market is open — for now. If they are ignored, they will strip-mine the asteroid belt and move on.".to_string(),
        "NOMADIC FLEET: The ark ships have left the system.".to_string(),
    ]
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::layer2::system::SystemBody;

    fn setup_world(tick: u64) -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick,
            speed: Default::default(),
        });
        world.init_resource::<NomadicDirector>();
        world.init_resource::<NomadicLedger>();
        world.init_resource::<ColonyResources>();
        world.init_resource::<FactionRelations>();
        world.init_resource::<Events<OfferNomadicBribeEvent>>();
        world.init_resource::<Events<NomadicDepartedEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    // Spec RED test 1, adapted: the sketch's tick-5000 spawn with
    // its `|| true` placeholder becomes a deterministic director
    // check — no fleet before 5000, exactly one at 5000, and the
    // director never fires twice.
    #[test]
    fn test_nomad_fleet_arrival() {
        let mut world = setup_world(4999);
        let mut schedule = Schedule::default();
        schedule.add_systems(spawn_nomad_fleets_system);
        schedule.run(&mut world);
        assert_eq!(nomadic_rows(&mut world).len(), 0);

        world.resource_mut::<SimulationTime>().tick = 5000;
        schedule.run(&mut world);
        let rows = nomadic_rows(&mut world);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.state, FleetState::Arriving);
        assert_eq!(rows[0].1.patience, DEFAULT_NOMADIC_PATIENCE);

        world.resource_mut::<SimulationTime>().tick = 5001;
        schedule.run(&mut world);
        assert_eq!(nomadic_rows(&mut world).len(), 1, "director fires once");
    }

    // Spec RED test 2, near-verbatim (amounts adapted u32 -> f32,
    // belt at the spec's real path layer2::system::AsteroidBelt):
    // a Mining fleet at rate 50 takes a 500 belt to 450 in one tick.
    #[test]
    fn test_nomad_mining_depletes_asteroids() {
        let mut world = setup_world(0);
        let asteroid = world
            .spawn((
                SystemBody,
                AsteroidBelt {
                    resources_remaining: 500.0,
                },
            ))
            .id();
        world.spawn(NomadicFleet {
            state: FleetState::Mining { target: asteroid },
            mining_rate: 50.0,
            patience: 100,
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(nomad_mining_system);
        schedule.run(&mut world);

        let belt = world.get::<AsteroidBelt>(asteroid).unwrap();
        assert_eq!(belt.resources_remaining, 450.0);
        assert_eq!(world.resource::<NomadicLedger>().total_mined, 50.0);
    }

    // Real-seam proof: a belt paired with a MiningTarget (the
    // headless spawner shape) depletes both by the same amount —
    // the stock the game's own fleets mine is the stock taken.
    #[test]
    fn test_mining_keeps_paired_mining_target_in_sync() {
        let mut world = setup_world(0);
        let belt = spawn_asteroid_belt(&mut world, 500.0);
        world.spawn(NomadicFleet {
            state: FleetState::Mining { target: belt },
            mining_rate: 50.0,
            patience: 100,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(nomad_mining_system);
        schedule.run(&mut world);
        assert_eq!(world.get::<AsteroidBelt>(belt).unwrap().resources_remaining, 450.0);
        assert_eq!(world.get::<MiningTarget>(belt).unwrap().amount, 450.0);
    }

    // Bare MiningTarget targets (spec 101 asteroids with no belt
    // aggregate) are depleted directly.
    #[test]
    fn test_mining_depletes_bare_mining_target() {
        let mut world = setup_world(0);
        let asteroid = world
            .spawn(MiningTarget {
                resource_type: ResourceType::Ore,
                amount: 100.0,
                mining_difficulty: 1.0,
            })
            .id();
        let fleet = world
            .spawn(NomadicFleet {
                state: FleetState::Mining { target: asteroid },
                mining_rate: 50.0,
                patience: 100,
            })
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(nomad_mining_system);
        schedule.run(&mut world);
        assert_eq!(world.get::<MiningTarget>(asteroid).unwrap().amount, 50.0);
        // Still stocked: the fleet keeps mining.
        assert_eq!(
            world.get::<NomadicFleet>(fleet).unwrap().state,
            FleetState::Mining { target: asteroid }
        );
    }

    // Spec RED test 3, near-verbatim: a depleted target and zero
    // patience send the fleet Departing, and the departure system
    // despawns it.
    #[test]
    fn test_nomad_fleet_leaves_when_done() {
        let mut world = setup_world(0);
        let asteroid = world
            .spawn((
                SystemBody,
                AsteroidBelt {
                    resources_remaining: 0.0,
                },
            ))
            .id();
        let fleet = world
            .spawn(NomadicFleet {
                state: FleetState::Mining { target: asteroid },
                mining_rate: 50.0,
                patience: 0,
            })
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems((nomad_mining_system, evaluate_nomad_departure_system).chain());
        schedule.run(&mut world);

        assert!(
            world.get::<NomadicFleet>(fleet).is_none(),
            "fleet despawns once done"
        );
        assert_eq!(world.resource::<NomadicLedger>().departures, 1);
    }

    // Mining out the last of a belt flips the fleet to Departing
    // in the same tick (clamped, never negative).
    #[test]
    fn test_mining_last_of_belt_sends_fleet_departing() {
        let mut world = setup_world(0);
        let belt = spawn_asteroid_belt(&mut world, 40.0);
        let fleet = world
            .spawn(NomadicFleet {
                state: FleetState::Mining { target: belt },
                mining_rate: 50.0,
                patience: 100,
            })
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(nomad_mining_system);
        schedule.run(&mut world);
        assert_eq!(world.get::<AsteroidBelt>(belt).unwrap().resources_remaining, 0.0);
        assert_eq!(
            world.get::<NomadicFleet>(fleet).unwrap().state,
            FleetState::Departing
        );
    }

    // Lifecycle: Arriving -> Trading -> Mining when a live belt
    // exists; patience burns while waiting.
    #[test]
    fn test_lifecycle_arriving_trading_mining() {
        let mut world = setup_world(0);
        let belt = spawn_asteroid_belt(&mut world, 500.0);
        let fleet = spawn_nomadic_fleet(&mut world);
        let mut schedule = Schedule::default();
        schedule.add_systems(evaluate_nomad_departure_system);

        schedule.run(&mut world);
        assert_eq!(
            world.get::<NomadicFleet>(fleet).unwrap().state,
            FleetState::Trading
        );
        schedule.run(&mut world);
        assert_eq!(
            world.get::<NomadicFleet>(fleet).unwrap().state,
            FleetState::Mining { target: belt }
        );
    }

    // Patience expiry: a Trading fleet with no belt to mine
    // departs when its patience runs out.
    #[test]
    fn test_patience_expiry_departs() {
        let mut world = setup_world(0);
        let fleet = world
            .spawn(NomadicFleet {
                state: FleetState::Trading,
                mining_rate: 10.0,
                patience: 1,
            })
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(evaluate_nomad_departure_system);
        schedule.run(&mut world);
        // Patience burned to 0 with nowhere to mine: Departing.
        assert_eq!(
            world.get::<NomadicFleet>(fleet).unwrap().state,
            FleetState::Departing
        );
        schedule.run(&mut world);
        assert!(world.get::<NomadicFleet>(fleet).is_none());
    }

    // §5 diplomacy: an accepted bribe consumes real ledger goods,
    // warms relations, and sends the fleet Departing — the belt
    // is left alone.
    #[test]
    fn test_bribe_makes_fleet_leave_belt_alone() {
        let mut world = setup_world(0);
        let belt = spawn_asteroid_belt(&mut world, 500.0);
        let fleet = world
            .spawn(NomadicFleet {
                state: FleetState::Mining { target: belt },
                mining_rate: 50.0,
                patience: 100,
            })
            .id();
        world.resource_mut::<ColonyResources>().metal = 20.0;
        world.send_event(OfferNomadicBribeEvent {
            fleet,
            resource: ResourceType::Metal,
            amount: 10.0,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(offer_nomadic_bribe_system);
        schedule.run(&mut world);

        assert_eq!(world.resource::<ColonyResources>().metal, 10.0);
        assert_eq!(
            world.get::<NomadicFleet>(fleet).unwrap().state,
            FleetState::Departing
        );
        assert_eq!(
            world.resource::<FactionRelations>().scores[&NOMADIC_FACTION_ID],
            BRIBE_RELATIONS_BONUS
        );
        assert_eq!(world.resource::<NomadicLedger>().bribes_accepted, 1);
        assert_eq!(world.get::<AsteroidBelt>(belt).unwrap().resources_remaining, 500.0);
    }

    // §5 refusal paths: below the minimum, or beyond the ledger —
    // nothing is consumed and the fleet keeps mining.
    #[test]
    fn test_bribe_refused_when_small_or_unaffordable() {
        let mut world = setup_world(0);
        let belt = spawn_asteroid_belt(&mut world, 500.0);
        let fleet = world
            .spawn(NomadicFleet {
                state: FleetState::Mining { target: belt },
                mining_rate: 50.0,
                patience: 100,
            })
            .id();
        world.resource_mut::<ColonyResources>().metal = 5.0;
        let mut schedule = Schedule::default();
        schedule.add_systems(offer_nomadic_bribe_system);

        // Too small (below BRIBE_MIN_AMOUNT).
        world.send_event(OfferNomadicBribeEvent {
            fleet,
            resource: ResourceType::Metal,
            amount: 5.0,
        });
        schedule.run(&mut world);
        assert_eq!(
            world.get::<NomadicFleet>(fleet).unwrap().state,
            FleetState::Mining { target: belt }
        );

        // Big enough, but the ledger cannot cover it.
        world.send_event(OfferNomadicBribeEvent {
            fleet,
            resource: ResourceType::Metal,
            amount: 10.0,
        });
        schedule.run(&mut world);
        assert_eq!(world.resource::<ColonyResources>().metal, 5.0);
        assert_eq!(
            world.get::<NomadicFleet>(fleet).unwrap().state,
            FleetState::Mining { target: belt }
        );
        assert_eq!(world.resource::<NomadicLedger>().bribes_accepted, 0);
    }

    #[test]
    fn test_stats_label() {
        let mut world = setup_world(0);
        assert_eq!(nomadic_stats_label(&mut world), "none");
        let _belt = spawn_asteroid_belt(&mut world, 500.0);
        let _fleet = spawn_nomadic_fleet(&mut world);
        assert_eq!(nomadic_stats_label(&mut world), "fleets1/belt500/mined0");
    }

    #[test]
    fn test_ip_guard_no_borrowed_terms() {
        let banned = [
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "sandworm",
            "bene gesserit",
            "hitchhiker",
            "vogon",
            "fallen empire",
            "stellaris",
        ];
        for text in nomadic_public_strings() {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string contains banned term {term:?}: {text}"
                );
            }
        }
    }
}
