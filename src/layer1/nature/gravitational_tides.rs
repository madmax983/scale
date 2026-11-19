//! Spec 430: Gravitational Tides.
//!
//! A massive neighbour hauls on the colony's world in a slow, predictable
//! cycle. High tide squeezes: pops move at half speed and automated
//! machines draw half again as much power. Low tide lets go: pops move
//! half again as fast, elevated pops risk a fall, and unsealed loose
//! items drift away. Pops born to low gravity thrive in it (spec §8);
//! everyone else is morale-neutral.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **No `MovementSpeed` / `Position` / `EnergyConsumer`.** The real
//!   movement seam is [`Speed`] (`pop::Speed { base, current,
//!   accumulator }`): `reset_speed_system` restores `current = base`
//!   every tick, then modifier systems multiply `current` — the
//!   cryo-shock pattern (Spec 413). [`apply_gravity_modifiers_system`]
//!   is this module's leg of that chain, scheduled
//!   `.after(reset_speed_system)`. The spec sketch's `Position.value.y`
//!   elevation is the real [`ZLevel`] component: a pop at
//!   [`ELEVATED_Z_LEVEL`] or above counts as elevated. The energy seam
//!   is the real [`PowerConsumer`] (`demand` / `active`): the live
//!   `power_grid_system` sums `demand` each tick, so scaling `demand`
//!   *is* scaling consumption. Scaling is anchored to a per-consumer
//!   [`TideDemandBase`] so it never compounds and restores exactly.
//! * **The cycle is the sim's own clock (spec §7).** Layer 2's orbital
//!   bodies do not expose a neighbour-phase resource Layer 1 can read,
//!   so the tide derives predictably from [`SimulationTime::tick`] over
//!   a [`TIDE_CYCLE_TICKS`] orbit (one year, the same 1000-tick period
//!   the atmospheric tide uses): first quarter High, middle half
//!   Normal, last quarter Low. [`tide_state_for_tick`] is pure and
//!   testable; the day-night cycle (250 ticks/day) nests inside it, so
//!   a tide spans four full days. Syzygy (`layer2::syzygy::TidalForce`)
//!   keeps its own crustal-tide lane and is deliberately untouched.
//! * **Fall damage is a risk *event* (spec §6), not instant damage.**
//!   [`FallDamageRiskEvent`] is the path: emitted once per tick for
//!   each elevated pop in Low tide, counted in the ledger, and left for
//!   hazard consumers to price — the same event-seam style as Spec
//!   367's fall pipeline, without re-implementing it.
//! * **Unsealed items (§5 REFACTOR).** Ground items are real [`Item`]
//!   entities at a [`GridPosition`]. An item carrying the new
//!   [`Unsealed`] marker can drift one tile in Low tide; the roll is a
//!   deterministic hash of entity + tick (no RNG resource, fully
//!   reproducible in tests and replays). Carried items have no ground
//!   position to drift from and are never touched.
//! * **Morale (§8).** No existing trait fits "Zero-G Native" —
//!   `VoidTouched` is void-stare resistance, `NativeBorn` is atmosphere
//!   hardiness — so [`Trait::ZeroGNative`] was added cleanly (label +
//!   random pool). In Low tide those pops carry one refreshed
//!   "Low Tide Drift" [`MoodModifier`]; all other pops get nothing.
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::energy::PowerConsumer;
use crate::layer1::items::Item;
use crate::layer1::map::{GridPosition, ZLevel};
use crate::layer1::pop::Speed;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::traits::{Trait, Traits};
use crate::shared::time::SimulationTime;

/// Full orbital period of the great neighbour, in ticks (one year —
/// the same period the atmospheric tide cycle uses).
pub const TIDE_CYCLE_TICKS: u64 = 1000;
/// Phase (tick within the cycle) where High tide ends.
pub const HIGH_PHASE_END: u64 = 250;
/// Phase where Low tide begins.
pub const LOW_PHASE_START: u64 = 750;
/// Movement multiplier in High tide (spec §6: halved).
pub const HIGH_SPEED_MULT: f32 = 0.5;
/// Movement multiplier in Low tide (spec §6: +50%).
pub const LOW_SPEED_MULT: f32 = 1.5;
/// Energy-demand multiplier for automated machines in High tide.
pub const HIGH_ENERGY_MULT: f32 = 1.5;
/// A pop at this Z level or above counts as elevated for fall risk
/// (the gravity-inversion ceiling default is 3, so 2+ is well off the
/// floor a pop normally stands on).
pub const ELEVATED_Z_LEVEL: i32 = 2;
/// Per-tick percentage chance an unsealed ground item drifts in Low
/// tide (deterministic hash roll — see [`should_float_away`]).
pub const FLOAT_CHANCE_PERCENT: u64 = 5;
/// Morale bonus for Zero-G Native pops in Low tide (spec §8: small).
pub const LOW_TIDE_MORALE_BONUS: f32 = 0.05;
/// Label of the Low-tide morale modifier (replace/refresh, never stacks).
pub const LOW_TIDE_MORALE_LABEL: &str = "Low Tide Drift";
/// Duration a Low-tide morale modifier is refreshed to each tick, so
/// it persists while Low lasts and decays shortly after the tide turns.
pub const MORALE_REFRESH_TICKS: u32 = 10;

/// The three tide states (spec §4, names kept from the spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TideState {
    /// The neighbour hauls close: slow movement, hungry machines.
    High,
    /// Ordinary pull.
    #[default]
    Normal,
    /// The neighbour lets go: fast movement, fall risk, drifting items.
    Low,
}

impl TideState {
    /// Short lowercase label (STATS / headless).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }

    /// Player-facing display name.
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::High => "High Tide",
            Self::Normal => "Normal Tide",
            Self::Low => "Low Tide",
        }
    }
}

/// The current gravitational tide (spec §4/§6: the resource tracks the
/// current state). `auto_cycle` is the predictable orbital drive;
/// headless/debug can pin a state by clearing it.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct GravitationalTide {
    /// Current tide state.
    pub state: TideState,
    /// When true, [`update_gravitational_tide_system`] derives the
    /// state from the simulation tick each tick. When false, the state
    /// is pinned (debug / scenario control).
    pub auto_cycle: bool,
}

impl Default for GravitationalTide {
    fn default() -> Self {
        Self {
            state: TideState::Normal,
            auto_cycle: true,
        }
    }
}

impl GravitationalTide {
    /// A tide pinned to `state` (tests, headless `tide <state>`).
    #[must_use]
    pub const fn pinned(state: TideState) -> Self {
        Self {
            state,
            auto_cycle: false,
        }
    }

    /// Movement-speed multiplier for the current state.
    #[must_use]
    pub fn speed_multiplier(&self) -> f32 {
        speed_multiplier_for(self.state)
    }

    /// Energy-demand multiplier for the current state.
    #[must_use]
    pub fn energy_multiplier(&self) -> f32 {
        energy_multiplier_for(self.state)
    }
}

/// Movement multiplier for a tide state: High halves, Low adds half,
/// Normal is neutral (spec §6).
#[must_use]
pub const fn speed_multiplier_for(state: TideState) -> f32 {
    match state {
        TideState::High => HIGH_SPEED_MULT,
        TideState::Normal => 1.0,
        TideState::Low => LOW_SPEED_MULT,
    }
}

/// Energy-demand multiplier for a tide state: only High tide costs
/// more (spec §5); Low tide's savings are spent on chaos, not watts.
#[must_use]
pub const fn energy_multiplier_for(state: TideState) -> f32 {
    match state {
        TideState::High => HIGH_ENERGY_MULT,
        TideState::Normal | TideState::Low => 1.0,
    }
}

/// The predictable tide for a simulation tick (spec §7): first quarter
/// of the 1000-tick orbit is High, the middle half Normal, the last
/// quarter Low, repeating forever.
#[must_use]
pub const fn tide_state_for_tick(tick: u64) -> TideState {
    let phase = tick % TIDE_CYCLE_TICKS;
    if phase < HIGH_PHASE_END {
        TideState::High
    } else if phase >= LOW_PHASE_START {
        TideState::Low
    } else {
        TideState::Normal
    }
}

/// True when a pop at this height counts as elevated for fall risk.
#[must_use]
pub fn is_elevated(z: Option<&ZLevel>) -> bool {
    z.is_some_and(|z| z.0 >= ELEVATED_Z_LEVEL)
}

/// Deterministic per-tick roll: does the unsealed item `entity_index`
/// drift on `tick`? A splitmix-style hash of the pair, so the outcome
/// is reproducible without an RNG resource (tests, replays, and the
/// headless playtest all agree).
#[must_use]
pub fn should_float_away(entity_index: u32, tick: u64) -> bool {
    let mut h = (u64::from(entity_index) + 0x9E37_79B9_7F4A_7C15)
        ^ tick.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    (h % 100) < FLOAT_CHANCE_PERCENT
}

/// Emitted for each elevated pop in Low tide (spec §4/§6): the fall-
/// damage risk path. Hazard consumers price the actual fall; this
/// module emits and counts the risk.
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FallDamageRiskEvent {
    /// The elevated pop at risk.
    pub entity: Entity,
}

/// Emitted when an unsealed item actually drifts in Low tide (§5).
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemFloatedAwayEvent {
    /// The item that drifted.
    pub entity: Entity,
    /// Tile it drifted from.
    pub from: GridPosition,
    /// Tile it drifted to.
    pub to: GridPosition,
}

/// Marker for a loose item that is not sealed down (§5 REFACTOR):
/// in Low tide it may drift. Items without this marker are secured
/// and never drift.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unsealed;

/// The nameplate power demand of a consumer, captured the first time
/// the tide system sees it, so tide scaling always multiplies the
/// *base* demand and restores exactly when the tide eases — never
/// compounding across ticks or shifts.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct TideDemandBase {
    /// Unscaled demand (power per tick).
    pub base: f32,
}

/// Colony ledger for the tides (STATS / headless panel surface).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GravitationalTideLedger {
    /// Tide-state shifts observed by the cycle system.
    pub shifts: u64,
    /// Fall-damage risk events emitted.
    pub fall_risks_emitted: u64,
    /// Unsealed items that drifted.
    pub items_floated: u64,
    /// Low-tide morale modifiers first applied.
    pub morale_buffs_applied: u64,
}

/// Chronicle line for a tide shift (original wording, IP-guarded).
#[must_use]
pub fn tide_shift_chronicle_text(state: TideState) -> String {
    match state {
        TideState::High => "The great neighbour hauls on the world: high tide — every step costs double, and the machines groan under the load.".to_string(),
        TideState::Normal => {
            "The tides ease: gravity settles back to its ordinary pull.".to_string()
        }
        TideState::Low => "The great neighbour lets go: low tide — the colony walks on springs, and loose things remember they can drift.".to_string(),
    }
}

/// Player-facing strings for this mechanic (IP-guard tested).
#[must_use]
pub fn gravitational_tides_public_strings() -> Vec<String> {
    vec![
        TideState::High.display_name().to_string(),
        TideState::Normal.display_name().to_string(),
        TideState::Low.display_name().to_string(),
        LOW_TIDE_MORALE_LABEL.to_string(),
        tide_shift_chronicle_text(TideState::High),
        tide_shift_chronicle_text(TideState::Normal),
        tide_shift_chronicle_text(TideState::Low),
        "Gravitational Tides".to_string(),
        "The great neighbour".to_string(),
        "Zero-G Native".to_string(),
    ]
}

/// STATS label: the live tide state, or `normal` when uninitialized.
#[must_use]
pub fn tide_stats_label(world: &mut World) -> &'static str {
    world
        .get_resource::<GravitationalTide>()
        .map_or("normal", |t| t.state.label())
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Derives the tide state from the simulation tick (spec §7) and
/// chronicles shifts. Pinned tides (`auto_cycle == false`) are left
/// alone. Runs in the Execution set after the day-night update.
pub fn update_gravitational_tide_system(
    time: Res<SimulationTime>,
    mut tide: ResMut<GravitationalTide>,
    mut ledger: Option<ResMut<GravitationalTideLedger>>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    if !tide.auto_cycle {
        return;
    }
    let next = tide_state_for_tick(time.tick);
    if next == tide.state {
        return;
    }
    tide.state = next;
    if let Some(ledger) = ledger.as_deref_mut() {
        ledger.shifts += 1;
    }
    if let Some(events) = chronicle.as_deref_mut() {
        events.send(AddChronicleEvent {
            text: tide_shift_chronicle_text(next),
            importance: EventImportance::Standard,
        });
    }
}

/// The movement leg (spec §3/§6, adapted to the real [`Speed`]
/// pipeline): multiplies `Speed.current` by the tide factor — High
/// halves, Low adds half — and, in Low tide, emits a
/// [`FallDamageRiskEvent`] for every elevated mover. Scheduled
/// `.after(reset_speed_system)`, so the multiply starts from `base`
/// every tick and never compounds.
pub fn apply_gravity_modifiers_system(
    tide: Res<GravitationalTide>,
    mut movers: Query<(Entity, &mut Speed, Option<&ZLevel>)>,
    mut fall_events: Option<ResMut<Events<FallDamageRiskEvent>>>,
    mut ledger: Option<ResMut<GravitationalTideLedger>>,
) {
    let mult = tide.speed_multiplier();
    let low = tide.state == TideState::Low;
    let mut emitted: u64 = 0;
    for (entity, mut speed, z) in movers.iter_mut() {
        // Multiply `current` (reset to `base` each tick upstream), so
        // the tide composes with the other speed legs instead of
        // overwriting them — the cryo-shock pattern.
        speed.current *= mult;
        if low && is_elevated(z) {
            if let Some(events) = fall_events.as_deref_mut() {
                events.send(FallDamageRiskEvent { entity });
            }
            emitted += 1;
        }
    }
    if emitted > 0 {
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.fall_risks_emitted += emitted;
        }
    }
}

/// The energy leg (spec §5/§7): automated machines — real
/// [`PowerConsumer`] entities — draw [`HIGH_ENERGY_MULT`]× their
/// nameplate demand in High tide. Demand is always recomputed from
/// the captured [`TideDemandBase`], so repeated ticks are idempotent
/// and easing the tide restores demand exactly.
pub fn apply_gravity_energy_modifiers_system(
    mut commands: Commands,
    tide: Res<GravitationalTide>,
    mut consumers: Query<(Entity, &mut PowerConsumer, Option<&TideDemandBase>)>,
) {
    let mult = tide.energy_multiplier();
    for (entity, mut consumer, base) in consumers.iter_mut() {
        let base_demand = match base {
            Some(b) => b.base,
            None => {
                commands.entity(entity).insert(TideDemandBase {
                    base: consumer.demand,
                });
                consumer.demand
            }
        };
        consumer.demand = base_demand * mult;
    }
}

/// The drifting-items leg (§5 REFACTOR): in Low tide, each unsealed
/// ground item rolls [`should_float_away`]; winners drift one tile
/// and are counted + evented. Sealed items and carried items (no
/// ground position) are never touched, and nothing drifts outside
/// Low tide.
#[allow(clippy::type_complexity)]
pub fn low_gravity_float_system(
    tide: Res<GravitationalTide>,
    time: Option<Res<SimulationTime>>,
    mut items: Query<(Entity, &mut GridPosition), (With<Item>, With<Unsealed>)>,
    mut ledger: Option<ResMut<GravitationalTideLedger>>,
    mut floated: Option<ResMut<Events<ItemFloatedAwayEvent>>>,
) {
    if tide.state != TideState::Low {
        return;
    }
    let tick = time.map_or(0, |t| t.tick);
    let mut drifted: u64 = 0;
    for (entity, mut pos) in items.iter_mut() {
        if should_float_away(entity.index(), tick) {
            let from = *pos;
            pos.x += 1;
            if let Some(events) = floated.as_deref_mut() {
                events.send(ItemFloatedAwayEvent {
                    entity,
                    from,
                    to: *pos,
                });
            }
            drifted += 1;
        }
    }
    if drifted > 0 {
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.items_floated += drifted;
        }
    }
}

/// The morale leg (spec §8): in Low tide, pops with
/// [`Trait::ZeroGNative`] carry a single refreshed "Low Tide Drift"
/// modifier (+0.05). Refresh replaces in place — it never stacks —
/// and no other pop is touched, in any tide.
pub fn tide_morale_system(
    tide: Res<GravitationalTide>,
    mut pops: Query<(&Traits, &mut Morale)>,
    mut ledger: Option<ResMut<GravitationalTideLedger>>,
) {
    if tide.state != TideState::Low {
        return;
    }
    let mut applied: u64 = 0;
    for (traits, mut morale) in pops.iter_mut() {
        if !traits.has(Trait::ZeroGNative) {
            continue;
        }
        if let Some(existing) = morale
            .modifiers
            .iter_mut()
            .find(|m| m.label == LOW_TIDE_MORALE_LABEL)
        {
            existing.value = LOW_TIDE_MORALE_BONUS;
            existing.duration = MORALE_REFRESH_TICKS;
        } else {
            morale.add_modifier(MoodModifier {
                label: LOW_TIDE_MORALE_LABEL.to_string(),
                value: LOW_TIDE_MORALE_BONUS,
                duration: MORALE_REFRESH_TICKS,
            });
            applied += 1;
        }
    }
    if applied > 0 {
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.morale_buffs_applied += applied;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::items::ItemType;
    use bevy_ecs::system::RunSystemOnce;

    fn world_with_tide(state: TideState) -> World {
        let mut world = World::new();
        world.insert_resource(GravitationalTide::pinned(state));
        world.insert_resource(GravitationalTideLedger::default());
        world.init_resource::<Events<FallDamageRiskEvent>>();
        world.init_resource::<Events<ItemFloatedAwayEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(SimulationTime::default());
        world
    }

    fn spawn_mover(world: &mut World, base: f32) -> Entity {
        world
            .spawn(Speed {
                base,
                current: base,
                accumulator: 0.0,
            })
            .id()
    }

    // --- Spec §3 RED tests, adapted to the real seams -----------------
    // Spec sketch: MovementSpeed { base, current } -> real pop::Speed;
    // Position { value: Vec3 } elevation -> real ZLevel component.

    #[test]
    fn test_high_gravity_slows_movement() {
        let mut world = world_with_tide(TideState::High);
        let entity = spawn_mover(&mut world, 10.0);
        world
            .run_system_once(apply_gravity_modifiers_system)
            .unwrap();
        let speed = world.get::<Speed>(entity).unwrap();
        assert_eq!(speed.current, 5.0, "High gravity halves speed");
    }

    #[test]
    fn test_low_gravity_increases_movement_and_risk() {
        let mut world = world_with_tide(TideState::Low);
        let entity = world
            .spawn((
                Speed {
                    base: 10.0,
                    current: 10.0,
                    accumulator: 0.0,
                },
                ZLevel(3), // Elevated (spec sketch: position y = 10)
            ))
            .id();
        world
            .run_system_once(apply_gravity_modifiers_system)
            .unwrap();
        let speed = world.get::<Speed>(entity).unwrap();
        assert_eq!(speed.current, 15.0, "Low gravity increases speed by 50%");
        let events: Vec<_> = world
            .resource_mut::<Events<FallDamageRiskEvent>>()
            .drain()
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].entity, entity);
        assert_eq!(
            world
                .resource::<GravitationalTideLedger>()
                .fall_risks_emitted,
            1
        );
    }

    #[test]
    fn test_low_gravity_ground_pop_no_fall_risk() {
        let mut world = world_with_tide(TideState::Low);
        let _ground = spawn_mover(&mut world, 10.0); // No ZLevel: on the floor
        let low = world
            .spawn((
                Speed {
                    base: 10.0,
                    current: 10.0,
                    accumulator: 0.0,
                },
                ZLevel(1), // One level up is not "elevated" (threshold 2)
            ))
            .id();
        let _ = low;
        world
            .run_system_once(apply_gravity_modifiers_system)
            .unwrap();
        let events: Vec<_> = world
            .resource_mut::<Events<FallDamageRiskEvent>>()
            .drain()
            .collect();
        assert!(events.is_empty(), "ground pops face no fall risk");
    }

    #[test]
    fn test_normal_gravity_leaves_speed_unchanged() {
        let mut world = world_with_tide(TideState::Normal);
        let entity = spawn_mover(&mut world, 10.0);
        world
            .run_system_once(apply_gravity_modifiers_system)
            .unwrap();
        assert_eq!(world.get::<Speed>(entity).unwrap().current, 10.0);
    }

    // --- The predictable cycle (spec §7) -------------------------------

    #[test]
    fn test_tide_state_for_tick_is_predictable_cycle() {
        assert_eq!(tide_state_for_tick(0), TideState::High);
        assert_eq!(tide_state_for_tick(249), TideState::High);
        assert_eq!(tide_state_for_tick(250), TideState::Normal);
        assert_eq!(tide_state_for_tick(500), TideState::Normal);
        assert_eq!(tide_state_for_tick(749), TideState::Normal);
        assert_eq!(tide_state_for_tick(750), TideState::Low);
        assert_eq!(tide_state_for_tick(999), TideState::Low);
        // The orbit repeats exactly.
        assert_eq!(tide_state_for_tick(1000), TideState::High);
        assert_eq!(tide_state_for_tick(1750), TideState::Low);
    }

    #[test]
    fn test_update_system_shifts_state_and_chronicles() {
        let mut world = World::new();
        world.insert_resource(GravitationalTide::default()); // Normal, auto
        world.insert_resource(GravitationalTideLedger::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(SimulationTime {
            tick: 800,
            ..Default::default()
        });
        world
            .run_system_once(update_gravitational_tide_system)
            .unwrap();
        assert_eq!(world.resource::<GravitationalTide>().state, TideState::Low);
        assert_eq!(world.resource::<GravitationalTideLedger>().shifts, 1);
        let chronicle: Vec<_> = world
            .resource_mut::<Events<AddChronicleEvent>>()
            .drain()
            .collect();
        assert_eq!(chronicle.len(), 1);
        assert!(chronicle[0].text.contains("low tide"));
        // Same tick again: no second shift, no second chronicle line.
        world
            .run_system_once(update_gravitational_tide_system)
            .unwrap();
        assert_eq!(world.resource::<GravitationalTideLedger>().shifts, 1);
    }

    #[test]
    fn test_pinned_tide_ignores_cycle() {
        let mut world = world_with_tide(TideState::High);
        world.resource_mut::<SimulationTime>().tick = 800; // Cycle says Low
        world
            .run_system_once(update_gravitational_tide_system)
            .unwrap();
        assert_eq!(world.resource::<GravitationalTide>().state, TideState::High);
    }

    // --- Energy leg (spec §5/§7) ----------------------------------------

    fn spawn_consumer(world: &mut World, demand: f32) -> Entity {
        world
            .spawn(PowerConsumer {
                demand,
                active: true,
            })
            .id()
    }

    #[test]
    fn test_high_gravity_increases_energy_consumption() {
        let mut world = world_with_tide(TideState::High);
        let machine = spawn_consumer(&mut world, 10.0);
        world
            .run_system_once(apply_gravity_energy_modifiers_system)
            .unwrap();
        assert_eq!(world.get::<PowerConsumer>(machine).unwrap().demand, 15.0);
        // Idempotent: a second tick must not compound the draw.
        world
            .run_system_once(apply_gravity_energy_modifiers_system)
            .unwrap();
        assert_eq!(world.get::<PowerConsumer>(machine).unwrap().demand, 15.0);
    }

    #[test]
    fn test_energy_restored_when_tide_eases() {
        let mut world = world_with_tide(TideState::High);
        let machine = spawn_consumer(&mut world, 10.0);
        world
            .run_system_once(apply_gravity_energy_modifiers_system)
            .unwrap();
        assert_eq!(world.get::<PowerConsumer>(machine).unwrap().demand, 15.0);
        // A machine built mid-High-tide joins at nameplate, then scales.
        let late = spawn_consumer(&mut world, 10.0);
        world
            .run_system_once(apply_gravity_energy_modifiers_system)
            .unwrap();
        assert_eq!(world.get::<PowerConsumer>(late).unwrap().demand, 15.0);
        // Tide eases: both restore exactly.
        world.resource_mut::<GravitationalTide>().state = TideState::Normal;
        world
            .run_system_once(apply_gravity_energy_modifiers_system)
            .unwrap();
        assert_eq!(world.get::<PowerConsumer>(machine).unwrap().demand, 10.0);
        assert_eq!(world.get::<PowerConsumer>(late).unwrap().demand, 10.0);
    }

    #[test]
    fn test_low_tide_energy_neutral() {
        let mut world = world_with_tide(TideState::Low);
        let machine = spawn_consumer(&mut world, 10.0);
        world
            .run_system_once(apply_gravity_energy_modifiers_system)
            .unwrap();
        assert_eq!(world.get::<PowerConsumer>(machine).unwrap().demand, 10.0);
    }

    // --- Morale leg (spec §8) -------------------------------------------

    fn spawn_pop_with_traits(world: &mut World, traits: &[Trait]) -> Entity {
        let mut t = Traits::default();
        for trait_ in traits {
            t.add(*trait_);
        }
        world.spawn((t, Morale::default())).id()
    }

    #[test]
    fn test_low_tide_morale_buff_for_zero_g_native() {
        let mut world = world_with_tide(TideState::Low);
        let native = spawn_pop_with_traits(&mut world, &[Trait::ZeroGNative]);
        world.run_system_once(tide_morale_system).unwrap();
        // Refresh on a second run must not stack a second modifier.
        world.run_system_once(tide_morale_system).unwrap();
        let morale = world.get::<Morale>(native).unwrap();
        let buffs: Vec<_> = morale
            .modifiers
            .iter()
            .filter(|m| m.label == LOW_TIDE_MORALE_LABEL)
            .collect();
        assert_eq!(buffs.len(), 1, "exactly one Low Tide Drift modifier");
        assert!((buffs[0].value - LOW_TIDE_MORALE_BONUS).abs() < f32::EPSILON);
        assert_eq!(
            world
                .resource::<GravitationalTideLedger>()
                .morale_buffs_applied,
            1
        );
    }

    #[test]
    fn test_low_tide_morale_neutral_for_other_pops() {
        let mut world = world_with_tide(TideState::Low);
        let other = spawn_pop_with_traits(&mut world, &[Trait::HardWorker]);
        let plain = spawn_pop_with_traits(&mut world, &[]);
        world.run_system_once(tide_morale_system).unwrap();
        assert!(world.get::<Morale>(other).unwrap().modifiers.is_empty());
        assert!(world.get::<Morale>(plain).unwrap().modifiers.is_empty());
    }

    #[test]
    fn test_high_tide_no_morale_buff() {
        let mut world = world_with_tide(TideState::High);
        let native = spawn_pop_with_traits(&mut world, &[Trait::ZeroGNative]);
        world.run_system_once(tide_morale_system).unwrap();
        assert!(world.get::<Morale>(native).unwrap().modifiers.is_empty());
    }

    // --- Drifting items (§5 REFACTOR) ------------------------------------

    fn spawn_ground_item(world: &mut World, unsealed: bool) -> Entity {
        let mut e = world.spawn((
            Item {
                item_type: ItemType::Potato,
            },
            GridPosition { x: 5, y: 5 },
        ));
        if unsealed {
            e.insert(Unsealed);
        }
        e.id()
    }

    #[test]
    fn test_unsealed_items_float_in_low_tide() {
        let mut world = world_with_tide(TideState::Low);
        let items: Vec<Entity> = (0..50)
            .map(|_| spawn_ground_item(&mut world, true))
            .collect();
        let start: Vec<i32> = items
            .iter()
            .map(|e| world.get::<GridPosition>(*e).unwrap().x)
            .collect();
        // Twenty ticks of Low tide: the deterministic roll must drift
        // at least one item, and every drift is exactly +1 per event.
        for tick in 0..20u64 {
            world.resource_mut::<SimulationTime>().tick = tick;
            world.run_system_once(low_gravity_float_system).unwrap();
        }
        let drifted = items
            .iter()
            .zip(start.iter())
            .filter(|(e, s)| world.get::<GridPosition>(**e).unwrap().x != **s)
            .count();
        assert!(drifted > 0, "some unsealed items should drift in Low tide");
        let ledger = world.resource::<GravitationalTideLedger>();
        assert!(ledger.items_floated > 0);
        let total_drift: i32 = items
            .iter()
            .zip(start.iter())
            .map(|(e, s)| world.get::<GridPosition>(*e).unwrap().x - *s)
            .sum();
        assert_eq!(total_drift as u64, ledger.items_floated);
    }

    #[test]
    fn test_sealed_items_never_float() {
        let mut world = world_with_tide(TideState::Low);
        let sealed: Vec<Entity> = (0..50)
            .map(|_| spawn_ground_item(&mut world, false))
            .collect();
        for tick in 0..20u64 {
            world.resource_mut::<SimulationTime>().tick = tick;
            world.run_system_once(low_gravity_float_system).unwrap();
        }
        for e in sealed {
            assert_eq!(world.get::<GridPosition>(e).unwrap().x, 5);
        }
        assert_eq!(world.resource::<GravitationalTideLedger>().items_floated, 0);
    }

    #[test]
    fn test_items_do_not_float_in_high_tide() {
        let mut world = world_with_tide(TideState::High);
        let item = spawn_ground_item(&mut world, true);
        for tick in 0..20u64 {
            world.resource_mut::<SimulationTime>().tick = tick;
            world.run_system_once(low_gravity_float_system).unwrap();
        }
        assert_eq!(world.get::<GridPosition>(item).unwrap().x, 5);
    }

    #[test]
    fn test_float_roll_is_deterministic() {
        for index in 0..200u32 {
            assert_eq!(
                should_float_away(index, 42),
                should_float_away(index, 42),
                "same entity + tick must always roll the same"
            );
        }
        // Roughly FLOAT_CHANCE_PERCENT of a large population rolls true.
        let hits = (0..10_000u32).filter(|i| should_float_away(*i, 7)).count();
        assert!(
            (200..=900).contains(&hits),
            "expected ~5% drift rate, got {hits}/10000"
        );
    }

    // --- Trait + IP guard -------------------------------------------------

    #[test]
    fn test_zero_g_native_trait_label() {
        assert_eq!(Trait::ZeroGNative.label(), "Zero-G Native");
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Gravitational tides are a generic orbital-mechanics trope:
        // public strings use original/generic wording only.
        let banned = [
            "sad king billy",
            "windsor",
            "interstellar",
            "gargantua",
            "murph",
            "mcconaughey",
            "hathaway",
            "nolan",
            "dune",
            "arrakis",
            "hyperion",
            "shrike",
            "endymion",
            "hegemony",
            "pandora",
            "avatar",
        ];
        for s in gravitational_tides_public_strings() {
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
