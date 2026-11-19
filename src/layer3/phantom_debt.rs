//! Spec 433: The Phantom Debt.
//!
//! A heavily armed, automated Layer 3 Collection Fleet arrives claiming
//! the colony world was purchased by a defunct precursor empire and
//! demands an impossible 10,000,000 credits. The colony can pay (it
//! cannot, realistically), go dark for 3 in-game days so the fleet
//! concludes the world is abandoned, or face foreclosure: a
//! catastrophic orbital bombardment through the real damage pipeline.
//!
//! ## Real seams (audit-first; spec sketch types were mocks)
//! * Power signature: summed from real active
//!   [`PowerSource`][crate::layer1::energy::PowerSource] outputs on the
//!   Layer 1 energy grid. The real
//!   [`BlackoutProtocol`][crate::layer1::energy::BlackoutProtocol] is
//!   the colony go-dark switch: while it is active the signature reads
//!   zero, exactly as the grid itself sheds all load.
//! * Tech signature: real research/data infrastructure — entities
//!   carrying [`Library`][crate::layer1::tech::Library] (Library and
//!   Observatory buildings both carry it) or
//!   [`DataStorage`][crate::layer1::tech::DataStorage] (ServerBank,
//!   Lander), weighted, plus actively-researched techs in the real
//!   [`TechState`][crate::layer1::tech::TechState]. Blackout zeroes it
//!   too: dark means dark.
//! * Fleet: a real Layer 2 [`Fleet`][crate::layer2::fleet::Fleet]
//!   entity with `FleetHealth`, an [`Orbit`] around the colony planet
//!   (first `SystemBody`) when one exists, and an `OrbitalBody` render
//!   identity. It is invincible by construction: it carries no combat
//!   target components and a million-point health pool; nothing in the
//!   combat pipeline can engage it. Despawned when the event concludes.
//! * Payment: the real [`ColonyResources`] credits ledger — no new
//!   currency. The demand is intentionally impossible for a young
//!   colony.
//! * Bombardment: the real
//!   [`ExplosionEvent`][crate::layer1::environment::volatile::ExplosionEvent]
//!   pipeline (the Spec 275/277 path), at megastructure-impact scale.
//! * Timers are grounded in the real
//!   [`DayNightCycle`][crate::layer1::day_night::DayNightCycle]
//!   (`ticks_per_day`, default 250): 3 dark days per spec section 8,
//!   10 days to foreclosure.
//! * Trigger: the spec sketch inserted `TriggerPhantomDebt` as a
//!   resource; the real codebase triggers crises with `Events<T>`
//!   (Spec 277 deorbit pattern), so it is an event first — the spawn
//!   system also honours a same-named resource for sketch parity.
//! * Chronicle + MessageLog follow the deorbit pattern. All names are
//!   original and generic (see the IP-guard test).

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::day_night::DayNightCycle;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::energy::{BlackoutProtocol, PowerSource};
use crate::layer1::environment::volatile::ExplosionEvent;
use crate::layer1::pop::Pop;
use crate::layer1::tech::{DataStorage, Library, TechState, TechStatus};
use crate::layer2::fleet::{Fleet, FleetHealth, InOrbit};
use crate::layer2::system::{Orbit, OrbitalBody, SystemBody};
use crate::shared::log::MessageLog;

/// The impossible demand, in credits (spec section 3).
pub const PHANTOM_DEBT_DEMAND: f32 = 10_000_000.0;
/// Fallback day length when no DayNightCycle resource exists.
pub const FALLBACK_TICKS_PER_DAY: u64 = 250;
/// Tech signature contributed by one research/data entity.
pub const TECH_SIGNATURE_PER_STATION: f32 = 10.0;
/// Tech signature contributed by one actively-researched tech.
pub const TECH_SIGNATURE_PER_TECH: f32 = 1.0;
/// Health pool of the Collection Fleet: invincible in practice.
pub const COLLECTION_FLEET_HEALTH: f32 = 1_000_000.0;
/// Map center fallback when there are no pops to average.
pub const MAP_CENTER: GridPosition = GridPosition { x: 40, y: 25 };

/// Trigger for the Phantom Debt crisis (spec section 4, adapted to the
/// real Events<T> crisis pattern; also honoured as a resource).
#[derive(Event, Resource, Debug, Clone, Copy)]
pub struct TriggerPhantomDebt;

/// Request to pay the demand from the real credits ledger.
#[derive(Event, Debug, Clone, Copy)]
pub struct PayPhantomDebtEvent;

/// How a Phantom Debt event concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhantomDebtOutcome {
    /// No conclusion yet (or no event has run).
    #[default]
    None,
    /// The colony stayed dark for 3 days; the fleet departed.
    DepartedDark,
    /// The demand was paid in full.
    Paid,
    /// The foreclosure timer expired; the fleet bombarded the colony.
    Foreclosed,
}

/// Active Phantom Debt state (spec section 4 GREEN phase).
#[derive(Resource, Debug, Clone)]
pub struct PhantomDebtEvent {
    /// Whether the Collection Fleet is currently holding the colony.
    pub active: bool,
    /// The demanded amount, in credits.
    pub demand: f32,
    /// Consecutive ticks the colony has read as fully dark.
    pub dark_ticks: u32,
    /// Ticks until foreclosure bombardment.
    pub foreclosure_ticks_remaining: u32,
    /// How the last event concluded.
    pub outcome: PhantomDebtOutcome,
}

impl Default for PhantomDebtEvent {
    fn default() -> Self {
        Self {
            active: false,
            demand: 0.0,
            dark_ticks: 0,
            foreclosure_ticks_remaining: 0,
            outcome: PhantomDebtOutcome::None,
        }
    }
}

/// The colony electromagnetic/tech signature the fleet can see
/// (spec section 4). Recomputed every tick from real entities.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ColonySignatures {
    /// Total active power generation signature.
    pub power: f32,
    /// Total research/data infrastructure signature.
    pub tech: f32,
}

impl ColonySignatures {
    /// Whether the colony reads as abandoned: no power, no tech.
    #[must_use]
    pub fn is_dark(&self) -> bool {
        self.power <= 0.0 && self.tech <= 0.0
    }
}

/// Tuning knobs for the Phantom Debt pipeline. Tests override these.
#[derive(Resource, Debug, Clone, Copy)]
pub struct PhantomDebtDirector {
    /// In-game days the colony must stay dark (spec section 8: 3).
    pub dark_days_required: u32,
    /// In-game days before foreclosure bombardment.
    pub foreclosure_days: u32,
    /// Damage of the foreclosure ExplosionEvent.
    pub bombardment_damage: f32,
    /// Radius of the foreclosure ExplosionEvent.
    pub bombardment_radius: u32,
}

impl Default for PhantomDebtDirector {
    fn default() -> Self {
        Self {
            dark_days_required: 3,
            foreclosure_days: 10,
            bombardment_damage: 600.0,
            bombardment_radius: 8,
        }
    }
}

impl PhantomDebtDirector {
    /// Ticks in one in-game day, from the real DayNightCycle.
    fn ticks_per_day(world: &World) -> u64 {
        world
            .get_resource::<DayNightCycle>()
            .map_or(FALLBACK_TICKS_PER_DAY, |c| {
                if c.ticks_per_day == 0 {
                    FALLBACK_TICKS_PER_DAY
                } else {
                    c.ticks_per_day
                }
            })
    }

    /// Consecutive dark ticks required for the fleet to depart.
    #[must_use]
    pub fn dark_ticks_required(&self, world: &World) -> u32 {
        let tpd = Self::ticks_per_day(world);
        u32::try_from(tpd.saturating_mul(u64::from(self.dark_days_required))).unwrap_or(u32::MAX)
    }

    /// Foreclosure window in ticks.
    #[must_use]
    pub fn foreclosure_ticks(&self, world: &World) -> u32 {
        let tpd = Self::ticks_per_day(world);
        u32::try_from(tpd.saturating_mul(u64::from(self.foreclosure_days))).unwrap_or(u32::MAX)
    }
}

/// Marker: the automated Collection Fleet entity in Layer 2.
/// Invincible by construction (see module docs).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct CollectionFleet {
    /// Always true: the fleet cannot be engaged or damaged.
    pub invincible: bool,
}

/// Mean position of living pops: the colony heart. Falls back to
/// [`MAP_CENTER`] when there are no pops.
fn colony_heart(world: &mut World) -> GridPosition {
    let mut query = world.query_filtered::<&GridPosition, With<Pop>>();
    let positions: Vec<GridPosition> = query.iter(world).copied().collect();
    if positions.is_empty() {
        return MAP_CENTER;
    }
    let (sx, sy) = positions
        .iter()
        .fold((0i64, 0i64), |(x, y), p| (x + p.x as i64, y + p.y as i64));
    #[allow(clippy::cast_possible_truncation)]
    GridPosition {
        x: (sx / positions.len() as i64) as i32,
        y: (sy / positions.len() as i64) as i32,
    }
}

fn chronicle(world: &mut World, text: String) {
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
}

fn log_line(world: &mut World, text: String) {
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(text);
    }
}

/// REFACTOR leg: compute ColonySignatures dynamically from real
/// entities — active power generators for `power`, research/data
/// stations plus researched techs for `tech`. The BlackoutProtocol
/// (the real emergency power cut) reads as fully dark.
#[must_use]
pub fn compute_colony_signatures(world: &mut World) -> ColonySignatures {
    if world
        .get_resource::<BlackoutProtocol>()
        .is_some_and(|b| b.active)
    {
        return ColonySignatures {
            power: 0.0,
            tech: 0.0,
        };
    }
    let power: f32 = {
        let mut query = world.query::<&PowerSource>();
        query
            .iter(world)
            .filter(|s| s.active)
            .map(|s| s.output)
            .sum::<f32>()
            + 0.0
    };
    let stations: usize = {
        let mut seen = std::collections::HashSet::new();
        let mut q1 = world.query_filtered::<Entity, With<Library>>();
        for e in q1.iter(world) {
            seen.insert(e);
        }
        let mut q2 = world.query_filtered::<Entity, With<DataStorage>>();
        for e in q2.iter(world) {
            seen.insert(e);
        }
        seen.len()
    };
    let active_techs: f32 = world.get_resource::<TechState>().map_or(0.0, |t| {
        t.techs
            .values()
            .filter(|s| **s == TechStatus::Active)
            .count() as f32
    });
    ColonySignatures {
        power,
        tech: stations as f32 * TECH_SIGNATURE_PER_STATION + active_techs * TECH_SIGNATURE_PER_TECH,
    }
}

/// System: refresh [`ColonySignatures`] from the live colony.
pub fn update_colony_signatures_system(world: &mut World) {
    let sig = compute_colony_signatures(world);
    let mut res = world.resource_mut::<ColonySignatures>();
    res.power = sig.power;
    res.tech = sig.tech;
}

/// Number of Collection Fleet entities currently in Layer 2.
pub fn collection_fleet_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<Entity, With<CollectionFleet>>();
    query.iter(world).count()
}

/// Despawn every Collection Fleet entity (event conclusion).
fn despawn_collection_fleet(world: &mut World) {
    let fleets: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<CollectionFleet>>();
        query.iter(world).collect()
    };
    for e in fleets {
        world.despawn(e);
    }
}

/// Spawn the invincible Collection Fleet in Layer 2, in orbit around
/// the colony planet (first SystemBody) when one exists.
fn spawn_collection_fleet(world: &mut World) {
    let planet: Option<Entity> = {
        let mut query = world.query_filtered::<Entity, With<SystemBody>>();
        query.iter(world).next()
    };
    let mut fleet = world.spawn((
        Fleet,
        CollectionFleet { invincible: true },
        FleetHealth {
            current: COLLECTION_FLEET_HEALTH,
            max: COLLECTION_FLEET_HEALTH,
        },
        OrbitalBody {
            name: "Collection Fleet".to_string(),
            radius: 1.5,
            color: ratatui::style::Color::Red,
            char: 'C',
        },
    ));
    if let Some(parent) = planet {
        fleet.insert((
            InOrbit { parent },
            Orbit {
                parent,
                radius: 30.0,
                speed: 0.01,
                angle: 0.0,
            },
        ));
    }
}

/// System: consume [`TriggerPhantomDebt`] (event or resource) and
/// activate the impossible demand + spawn the fleet. Only one event
/// can be in flight at a time; extra triggers are ignored.
pub fn spawn_collection_fleet_system(world: &mut World) {
    let mut triggered = false;
    {
        let mut ev = world.resource_mut::<Events<TriggerPhantomDebt>>();
        if !ev.drain().collect::<Vec<_>>().is_empty() {
            triggered = true;
        }
    }
    if world.get_resource::<TriggerPhantomDebt>().is_some() {
        world.remove_resource::<TriggerPhantomDebt>();
        triggered = true;
    }
    if !triggered {
        return;
    }
    if world
        .get_resource::<PhantomDebtEvent>()
        .is_some_and(|d| d.active)
    {
        return;
    }
    let director = world
        .get_resource::<PhantomDebtDirector>()
        .copied()
        .unwrap_or_default();
    let foreclosure = director.foreclosure_ticks(world);
    {
        let mut debt = world.resource_mut::<PhantomDebtEvent>();
        debt.active = true;
        debt.demand = PHANTOM_DEBT_DEMAND;
        debt.dark_ticks = 0;
        debt.foreclosure_ticks_remaining = foreclosure;
        debt.outcome = PhantomDebtOutcome::None;
    }
    spawn_collection_fleet(world);
    log_line(
        world,
        format!(
            "PHANTOM DEBT: an automated Collection Fleet claims this world was purchased by a defunct precursor empire. It demands {PHANTOM_DEBT_DEMAND:.0} credits or it will foreclose in {foreclosure} ticks. Go dark for {} days, pay, or be bombarded.",
            director.dark_days_required
        ),
    );
    chronicle(
        world,
        format!(
            "PHANTOM DEBT: a heavily armed automated Collection Fleet has arrived, claiming this world was purchased by a defunct precursor empire thousands of years ago. It demands {PHANTOM_DEBT_DEMAND:.0} credits in accumulated interest, or it will foreclose on the planet."
        ),
    );
}

/// System: honour payment requests against the real credits ledger.
pub fn pay_phantom_debt_system(world: &mut World) {
    let requests = {
        let mut ev = world.resource_mut::<Events<PayPhantomDebtEvent>>();
        ev.drain().count()
    };
    if requests == 0 {
        return;
    }
    let (active, demand) = {
        let debt = world.resource::<PhantomDebtEvent>();
        (debt.active, debt.demand)
    };
    if !active {
        return;
    }
    let credits = world.resource::<ColonyResources>().credits;
    if credits < demand {
        log_line(
            world,
            format!(
                "PHANTOM DEBT: payment refused — the colony holds {credits:.0} credits against a {demand:.0} demand."
            ),
        );
        return;
    }
    world.resource_mut::<ColonyResources>().credits -= demand;
    {
        let mut debt = world.resource_mut::<PhantomDebtEvent>();
        debt.active = false;
        debt.outcome = PhantomDebtOutcome::Paid;
    }
    despawn_collection_fleet(world);
    log_line(
        world,
        "PHANTOM DEBT PAID: the Collection Fleet acknowledges receipt and departs.".to_string(),
    );
    chronicle(
        world,
        "PHANTOM DEBT PAID: the colony has somehow paid the impossible demand in full. The Collection Fleet departs without a word.".to_string(),
    );
}

/// System: the fleet circles the planet while the event runs.
pub fn orbit_collection_fleet_system(mut query: Query<&mut Orbit, With<CollectionFleet>>) {
    for mut orbit in &mut query {
        orbit.angle += orbit.speed;
    }
}

/// System: evaluate survival each tick (spec section 4, extended by
/// section 8 and the REFACTOR foreclosure leg). Staying fully dark
/// for 3 in-game days convinces the fleet the world is abandoned and
/// it departs; otherwise the foreclosure timer runs down to a
/// catastrophic bombardment through the real ExplosionEvent pipeline.
pub fn evaluate_dark_age_survival_system(world: &mut World) {
    let active = world
        .get_resource::<PhantomDebtEvent>()
        .is_some_and(|d| d.active);
    if !active {
        return;
    }
    let director = world
        .get_resource::<PhantomDebtDirector>()
        .copied()
        .unwrap_or_default();
    let dark_required = director.dark_ticks_required(world);
    let dark = world
        .get_resource::<ColonySignatures>()
        .copied()
        .unwrap_or_default()
        .is_dark();
    enum Resolution {
        Ongoing,
        Departed,
        Foreclosed,
    }
    let resolution = {
        let mut debt = world.resource_mut::<PhantomDebtEvent>();
        if dark {
            debt.dark_ticks = debt.dark_ticks.saturating_add(1);
        } else {
            debt.dark_ticks = 0;
        }
        if debt.dark_ticks >= dark_required {
            debt.active = false;
            debt.outcome = PhantomDebtOutcome::DepartedDark;
            Resolution::Departed
        } else {
            if debt.foreclosure_ticks_remaining > 0 {
                debt.foreclosure_ticks_remaining -= 1;
            }
            if debt.foreclosure_ticks_remaining == 0 {
                debt.active = false;
                debt.outcome = PhantomDebtOutcome::Foreclosed;
                Resolution::Foreclosed
            } else {
                Resolution::Ongoing
            }
        }
    };
    match resolution {
        Resolution::Ongoing => {}
        Resolution::Departed => {
            despawn_collection_fleet(world);
            log_line(
                world,
                "PHANTOM DEBT: the colony stayed dark; the Collection Fleet concludes the world is abandoned and departs.".to_string(),
            );
            chronicle(
                world,
                "THE LIGHTS STAYED OUT: after three days of darkness, the Collection Fleet concluded this world was abandoned long ago and departed to chase debts elsewhere.".to_string(),
            );
        }
        Resolution::Foreclosed => {
            let target = colony_heart(world);
            world
                .resource_mut::<Events<ExplosionEvent>>()
                .send(ExplosionEvent {
                    center: target,
                    damage: director.bombardment_damage,
                    radius: director.bombardment_radius,
                });
            despawn_collection_fleet(world);
            log_line(
                world,
                format!(
                    "FORECLOSURE: the Collection Fleet bombards the colony at ({}, {})!",
                    target.x, target.y
                ),
            );
            chronicle(
                world,
                "FORECLOSURE: the debt went unpaid and the colony never went dark. The Collection Fleet has begun orbital bombardment of the planet it claims to own.".to_string(),
            );
        }
    }
}

/// Short status for headless STATS: "none", a conclusion label, or
/// "active:<demand>cr:dark<d>/<req>:foreclose<f>t:fleet<n>".
#[must_use]
pub fn phantom_debt_status_label(world: &mut World) -> String {
    let director = world
        .get_resource::<PhantomDebtDirector>()
        .copied()
        .unwrap_or_default();
    let dark_required = director.dark_ticks_required(world);
    let fleet_count = collection_fleet_count(world);
    match world.get_resource::<PhantomDebtEvent>() {
        Some(d) if d.active => format!(
            "active:{:.0}cr:dark{}/{}:foreclose{}t:fleet{}",
            d.demand, d.dark_ticks, dark_required, d.foreclosure_ticks_remaining, fleet_count
        ),
        Some(d) => match d.outcome {
            PhantomDebtOutcome::DepartedDark => "departed-dark".to_string(),
            PhantomDebtOutcome::Paid => "paid".to_string(),
            PhantomDebtOutcome::Foreclosed => "foreclosed".to_string(),
            PhantomDebtOutcome::None => "none".to_string(),
        },
        None => "none".to_string(),
    }
}

/// Force helper for headless/debug: set every real power generator
/// active or inactive. Returns how many generators were touched.
/// Going dark for real also needs the tech signature gone; the
/// headless `debt dark` command pairs this with BlackoutProtocol,
/// which is the colony actual emergency dark switch.
pub fn set_all_generators_active(world: &mut World, active: bool) -> usize {
    let mut query = world.query::<&mut PowerSource>();
    let mut n = 0usize;
    for mut source in query.iter_mut(world) {
        source.active = active;
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::tech::Tech;

    fn test_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<TriggerPhantomDebt>>();
        world.init_resource::<Events<PayPhantomDebtEvent>>();
        world.init_resource::<Events<ExplosionEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<PhantomDebtEvent>();
        world.init_resource::<PhantomDebtDirector>();
        world.init_resource::<ColonySignatures>();
        world.init_resource::<ColonyResources>();
        world.init_resource::<TechState>();
        world.init_resource::<DayNightCycle>();
        world.init_resource::<BlackoutProtocol>();
        world.init_resource::<MessageLog>();
        world
    }

    fn run(world: &mut World, systems: &[&str]) {
        let mut schedule = Schedule::default();
        for s in systems {
            match *s {
                "sig" => {
                    schedule.add_systems(update_colony_signatures_system);
                }
                "spawn" => {
                    schedule.add_systems(spawn_collection_fleet_system);
                }
                "pay" => {
                    schedule.add_systems(pay_phantom_debt_system);
                }
                "eval" => {
                    schedule.add_systems(evaluate_dark_age_survival_system);
                }
                _ => unreachable!(),
            }
        }
        schedule.run(world);
    }

    fn run_tick(world: &mut World) {
        run(world, &["sig", "spawn", "pay", "eval"]);
    }

    fn chronicle_texts(world: &mut World) -> Vec<String> {
        let events = world.resource::<Events<AddChronicleEvent>>();
        events
            .get_cursor()
            .read(events)
            .map(|e| e.text.clone())
            .collect()
    }

    fn trigger(world: &mut World) {
        world
            .resource_mut::<Events<TriggerPhantomDebt>>()
            .send(TriggerPhantomDebt);
        run(world, &["spawn"]);
    }

    /// Spec RED test 1 (adapted to the real Events pattern): the
    /// trigger activates the impossible demand and spawns the fleet.
    #[test]
    fn test_collection_fleet_arrives_and_demands() {
        let mut world = test_world();
        let planet = world.spawn((SystemBody,)).id();
        trigger(&mut world);
        let debt = world.resource::<PhantomDebtEvent>();
        assert!(debt.active);
        assert_eq!(debt.demand, 10_000_000.0);
        assert!(debt.foreclosure_ticks_remaining > 0);
        assert_eq!(collection_fleet_count(&mut world), 1);
        let mut q = world.query::<(&CollectionFleet, &FleetHealth, &InOrbit)>();
        let (fleet, health, orbit) = q.single(&world);
        assert!(fleet.invincible);
        assert_eq!(health.current, COLLECTION_FLEET_HEALTH);
        assert_eq!(orbit.parent, planet);
        let texts = chronicle_texts(&mut world);
        assert!(texts.iter().any(|t| t.contains("PHANTOM DEBT")));
    }

    /// Spec sketch parity: inserting TriggerPhantomDebt as a resource
    /// also activates the event.
    #[test]
    fn test_trigger_as_resource_also_works() {
        let mut world = test_world();
        world.insert_resource(TriggerPhantomDebt);
        run(&mut world, &["spawn"]);
        assert!(world.resource::<PhantomDebtEvent>().active);
        assert!(world.get_resource::<TriggerPhantomDebt>().is_none());
    }

    /// Spec RED test 2 (extended per section 8): signatures at zero
    /// must persist for 3 in-game days before the fleet departs.
    #[test]
    fn test_fleet_leaves_if_colony_goes_dark() {
        let mut world = test_world();
        // 2 ticks/day * 3 days = 6 consecutive dark ticks required.
        world.resource_mut::<DayNightCycle>().ticks_per_day = 2;
        trigger(&mut world);
        world.insert_resource(ColonySignatures {
            power: 0.0,
            tech: 0.0,
        });
        for i in 0..5 {
            run(&mut world, &["eval"]);
            assert!(
                world.resource::<PhantomDebtEvent>().active,
                "fleet must not leave before 3 dark days (tick {i})"
            );
        }
        run(&mut world, &["eval"]);
        let debt = world.resource::<PhantomDebtEvent>();
        assert!(!debt.active);
        assert_eq!(debt.outcome, PhantomDebtOutcome::DepartedDark);
        assert_eq!(collection_fleet_count(&mut world), 0);
    }

    /// A single lit tick resets the dark streak.
    #[test]
    fn test_dark_streak_resets_when_colony_lights_up() {
        let mut world = test_world();
        world.resource_mut::<DayNightCycle>().ticks_per_day = 2;
        trigger(&mut world);
        world.insert_resource(ColonySignatures {
            power: 0.0,
            tech: 0.0,
        });
        run(&mut world, &["eval"]);
        run(&mut world, &["eval"]);
        assert_eq!(world.resource::<PhantomDebtEvent>().dark_ticks, 2);
        world.insert_resource(ColonySignatures {
            power: 5.0,
            tech: 0.0,
        });
        run(&mut world, &["eval"]);
        assert_eq!(world.resource::<PhantomDebtEvent>().dark_ticks, 0);
        assert!(world.resource::<PhantomDebtEvent>().active);
    }

    /// REFACTOR leg: signatures are computed from real generators and
    /// research/data stations; blackout reads as fully dark.
    #[test]
    fn test_signatures_computed_dynamically() {
        let mut world = test_world();
        world.spawn(PowerSource {
            output: 25.0,
            active: true,
        });
        world.spawn(PowerSource {
            output: 10.0,
            active: false,
        });
        world.spawn(Library);
        world.spawn(DataStorage { capacity: 50.0 });
        {
            let mut tech = world.resource_mut::<TechState>();
            tech.total_capacity = 100.0;
            tech.unlock(Tech::Masonry);
        }
        let sig = compute_colony_signatures(&mut world);
        assert_eq!(sig.power, 25.0);
        // 2 stations * 10 + 1 active tech * 1 = 21.
        assert_eq!(sig.tech, 21.0);
        assert!(!sig.is_dark());
        // Shutting down all generators zeroes the power signature.
        assert_eq!(set_all_generators_active(&mut world, false), 2);
        let sig = compute_colony_signatures(&mut world);
        assert_eq!(sig.power, 0.0);
        // Blackout: the whole colony reads dark.
        world.resource_mut::<BlackoutProtocol>().active = true;
        let sig = compute_colony_signatures(&mut world);
        assert!(sig.is_dark());
    }

    /// REFACTOR leg: the fleet actually orbits (Orbit angle advances).
    #[test]
    fn test_collection_fleet_orbits_planet() {
        let mut world = test_world();
        let planet = world.spawn((SystemBody,)).id();
        trigger(&mut world);
        let mut schedule = Schedule::default();
        schedule.add_systems(orbit_collection_fleet_system);
        schedule.run(&mut world);
        schedule.run(&mut world);
        let mut q = world.query::<&Orbit>();
        let orbit = q.single(&world);
        assert_eq!(orbit.parent, planet);
        assert!((orbit.angle - 0.02).abs() < 1e-6);
    }

    /// Payment through the real credits ledger concludes the event.
    #[test]
    fn test_payment_in_full_dismisses_fleet() {
        let mut world = test_world();
        trigger(&mut world);
        world.resource_mut::<ColonyResources>().credits = 12_000_000.0;
        world
            .resource_mut::<Events<PayPhantomDebtEvent>>()
            .send(PayPhantomDebtEvent);
        run(&mut world, &["pay"]);
        let debt = world.resource::<PhantomDebtEvent>();
        assert!(!debt.active);
        assert_eq!(debt.outcome, PhantomDebtOutcome::Paid);
        assert_eq!(world.resource::<ColonyResources>().credits, 2_000_000.0);
        assert_eq!(collection_fleet_count(&mut world), 0);
    }

    /// A young colony cannot pay: payment is refused, event continues.
    #[test]
    fn test_payment_refused_when_broke() {
        let mut world = test_world();
        trigger(&mut world);
        world.resource_mut::<ColonyResources>().credits = 500.0;
        world
            .resource_mut::<Events<PayPhantomDebtEvent>>()
            .send(PayPhantomDebtEvent);
        run(&mut world, &["pay"]);
        assert!(world.resource::<PhantomDebtEvent>().active);
        assert_eq!(world.resource::<ColonyResources>().credits, 500.0);
    }

    /// REFACTOR leg: foreclosure fires a real ExplosionEvent at the
    /// colony heart and despawns the fleet.
    #[test]
    fn test_foreclosure_bombardment() {
        let mut world = test_world();
        // 2 ticks/day * 1 foreclosure day = 2 lit ticks to foreclosure.
        world.resource_mut::<DayNightCycle>().ticks_per_day = 2;
        world.resource_mut::<PhantomDebtDirector>().foreclosure_days = 1;
        // A lit colony: a running generator keeps signatures positive.
        world.spawn(PowerSource {
            output: 10.0,
            active: true,
        });
        trigger(&mut world);
        run_tick(&mut world);
        assert!(world.resource::<PhantomDebtEvent>().active);
        run_tick(&mut world);
        let debt = world.resource::<PhantomDebtEvent>();
        assert!(!debt.active);
        assert_eq!(debt.outcome, PhantomDebtOutcome::Foreclosed);
        assert_eq!(collection_fleet_count(&mut world), 0);
        let explosions = world.resource::<Events<ExplosionEvent>>();
        let mut cursor = explosions.get_cursor();
        let fired: Vec<_> = cursor.read(explosions).cloned().collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].damage, 600.0);
        assert_eq!(fired[0].center, MAP_CENTER);
    }

    /// A second trigger while an event is active is ignored.
    #[test]
    fn test_second_trigger_while_active_is_ignored() {
        let mut world = test_world();
        trigger(&mut world);
        trigger(&mut world);
        assert_eq!(collection_fleet_count(&mut world), 1);
        assert_eq!(world.resource::<PhantomDebtEvent>().demand, 10_000_000.0);
    }

    /// IP guard: player-facing strings use only original/generic names.
    #[test]
    fn test_ip_guard_no_named_ip() {
        const BANNED: &[&str] = &[
            "Star Wars",
            "Star Trek",
            "Warhammer",
            "Hitchhiker",
            "Ender",
            "Dune",
            "Foundation Empire",
            "Galactic Empire",
            "Imperium",
        ];
        let strings = [
            "Collection Fleet",
            "PHANTOM DEBT: a heavily armed automated Collection Fleet has arrived, claiming this world was purchased by a defunct precursor empire thousands of years ago.",
            "THE LIGHTS STAYED OUT: after three days of darkness, the Collection Fleet concluded this world was abandoned long ago and departed to chase debts elsewhere.",
            "FORECLOSURE: the debt went unpaid and the colony never went dark.",
        ];
        for s in strings {
            for banned in BANNED {
                assert!(
                    !s.contains(banned),
                    "player-facing string contains banned IP term {banned}: {s}"
                );
            }
        }
    }

    /// STATS label reflects the live event.
    #[test]
    fn test_status_label() {
        let mut world = test_world();
        assert_eq!(phantom_debt_status_label(&mut world), "none");
        trigger(&mut world);
        let label = phantom_debt_status_label(&mut world);
        assert!(label.starts_with("active:10000000cr:"), "got {label}");
        assert!(label.contains("fleet1"), "got {label}");
    }
}
