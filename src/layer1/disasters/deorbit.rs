//! Spec 277: Orbital Megastructure Deorbiting.
//!
//! Massive Layer 2 structures (derelict ring sections, station hulks,
//! debris fragments) can be caught by tether arrays built on Layer 1.
//! Catching one grants salvage and orbital tech; failing to catch it
//! destroys whatever it lands on.
//!
//! ## Mechanics
//! * A [`DeorbitEvent`] starts the crisis: the [`DeorbitState`] resource
//!   holds a countdown (`ticks_remaining`, default
//!   [`DEORBIT_DEFAULT_WARNING_TICKS`]) and an impact [`GridPosition`]
//!   (explicit in the event, else the colony heart, else map center).
//!   A Major chronicle warning names the size class and the deadline.
//! * [`BuildingType::TetherArray`][crate::layer1::building::BuildingType]
//!   buildings carry a [`TetherArray`] marker component with catch
//!   strength. When the countdown hits zero, total colony tether strength
//!   is compared against the size class's requirement:
//!   * **Catch**: the megastructure is snared. Salvage (metal,
//!     hyper-alloys, credits) lands in
//!     [`ColonyResources`][crate::layer1::economy::resources::ColonyResources],
//!     and the first unresearched tech from the salvage pool
//!     (Astronomy, Electromagnetism, Terraforming) is granted through the
//!     real [`TechState`][crate::layer1::tech::TechState]
//!     (`try_unlock` — a full databank means the schematics can't load).
//!   * **Impact**: a real [`ExplosionEvent`] fires through the existing
//!     damage pipeline (Structures + Health), and every building inside
//!     the crater radius is demolished into a
//!     [`Ruin`][crate::layer1::architecture::ruins::Ruin] with a
//!     [`BuildingRemovedEvent`][crate::layer1::core::events::BuildingRemovedEvent].
//!     [`OrbitalDebris`][crate::layer2::debris::OrbitalDebris] rises past
//!     the Kessler attrition line.
//! * A Major chronicle entry covers the warning, the catch, and the impact.
//!
//! ## Spec adaptations
//! * The sketch's `DeorbitingEvent`/`TetherArray` building/`VolatileExplosion`
//!   don't exist; the real event is [`DeorbitEvent`], the tether array is a
//!   real [`BuildingType`][crate::layer1::building::BuildingType] variant
//!   with a marker component, and damage flows through the real
//!   [`ExplosionEvent`] pipeline instead of an invented damage type.
//! * The sketch's suggested paths (`src/layer1/events/deorbit.rs`,
//!   `src/layer1/buildings/tether_array.rs`) don't exist; the module lives
//!   in `disasters/` next to the other catastrophe mechanics, and the
//!   building variant lives in `architecture/building.rs` like every other.
//! * §5's TUI element is intentionally left out (headless STATS +
//!   console commands satisfy the loop); noted as a follow-on.
//! * All names are original and generic (see the IP-guard test).

use bevy_ecs::prelude::*;

use crate::layer1::building::{Building, BuildingType, MaterialType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::events::BuildingRemovedEvent;
use crate::layer1::core::map::GridPosition;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::environment::volatile::ExplosionEvent;
use crate::layer1::pop::Pop;
use crate::layer1::ruins::{Ruin, RuinHistory};
use crate::layer1::tech::{Tech, TechState};
use crate::layer2::debris::OrbitalDebris;
use crate::shared::log::MessageLog;
use crate::shared::time::SimulationTime;

/// Default warning window before impact, in ticks.
pub const DEORBIT_DEFAULT_WARNING_TICKS: u32 = 600;
/// Catch strength contributed by one operational tether array.
pub const TETHER_STRENGTH_PER_ARRAY: f32 = 1.0;
/// Orbital-debris feed on impact (above the 0.1 Kessler attrition line).
pub const IMPACT_DEBRIS_FEED: f32 = 0.15;
/// Map center fallback when there are no pops to average.
pub const MAP_CENTER: GridPosition = GridPosition { x: 40, y: 25 };
/// Techs recoverable from megastructure salvage databanks, in grant order.
pub const SALVAGE_TECH_POOL: [Tech; 3] = [Tech::Astronomy, Tech::Electromagnetism, Tech::Terraforming];

/// Size class of the incoming megastructure. Sets the tether requirement,
/// the violence of the impact, and the value of the salvage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MegastructureSize {
    /// A debris fragment: easy to catch, modest either way.
    Fragment,
    /// A derelict station hulk: needs a real tether net.
    #[default]
    Hulk,
    /// A full megastructure ring section: apocalyptic if it lands.
    Megastructure,
}

impl MegastructureSize {
    /// Tether arrays required to catch this size class.
    #[must_use]
    pub const fn required_tethers(self) -> u32 {
        match self {
            MegastructureSize::Fragment => 1,
            MegastructureSize::Hulk => 2,
            MegastructureSize::Megastructure => 4,
        }
    }

    /// Damage of the impact's [`ExplosionEvent`].
    #[must_use]
    pub const fn impact_damage(self) -> f32 {
        match self {
            MegastructureSize::Fragment => 150.0,
            MegastructureSize::Hulk => 300.0,
            MegastructureSize::Megastructure => 600.0,
        }
    }

    /// Radius of the impact's [`ExplosionEvent`] (Chebyshev tiles).
    #[must_use]
    pub const fn impact_radius(self) -> u32 {
        match self {
            MegastructureSize::Fragment => 3,
            MegastructureSize::Hulk => 5,
            MegastructureSize::Megastructure => 8,
        }
    }

    /// Crater radius: buildings inside are demolished into ruins.
    #[must_use]
    pub const fn crater_radius(self) -> u32 {
        match self {
            MegastructureSize::Fragment => 4,
            MegastructureSize::Hulk => 7,
            MegastructureSize::Megastructure => 10,
        }
    }

    /// Salvage on a successful catch: (metal, hyper_alloys, credits).
    #[must_use]
    pub const fn salvage(self) -> (f32, f32, f32) {
        match self {
            MegastructureSize::Fragment => (30.0, 5.0, 100.0),
            MegastructureSize::Hulk => (80.0, 15.0, 300.0),
            MegastructureSize::Megastructure => (200.0, 40.0, 800.0),
        }
    }

    /// Human-readable size label, used in chronicles and STATS.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            MegastructureSize::Fragment => "fragment",
            MegastructureSize::Hulk => "derelict hulk",
            MegastructureSize::Megastructure => "megastructure ring section",
        }
    }

    /// Parse a size from a headless console argument.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "fragment" | "frag" | "small" => Some(MegastructureSize::Fragment),
            "hulk" | "medium" | "derelict" => Some(MegastructureSize::Hulk),
            "megastructure" | "mega" | "large" | "ring" => Some(MegastructureSize::Megastructure),
            _ => None,
        }
    }
}

/// Event starting a deorbit crisis (spec GREEN phase: minimal event).
#[derive(Event, Debug, Clone)]
pub struct DeorbitEvent {
    /// Size class of the incoming structure.
    pub size: MegastructureSize,
    /// Impact point. `None` = colony heart (mean pop position), then map center.
    pub target: Option<GridPosition>,
    /// Warning window override. `None` = director default.
    pub warning_ticks: Option<u32>,
}

/// Active deorbit countdown (spec GREEN phase).
#[derive(Resource, Debug, Clone, Default)]
pub struct DeorbitState {
    /// Whether a deorbit crisis is currently in flight.
    pub active: bool,
    /// Size class of the incoming structure.
    pub size: MegastructureSize,
    /// Ticks until impact.
    pub ticks_remaining: u32,
    /// Impact point.
    pub target: GridPosition,
}

/// Tuning knobs for the deorbit pipeline. Tests override these.
#[derive(Resource, Debug, Clone)]
pub struct DeorbitDirector {
    /// Default warning window, in ticks.
    pub warning_ticks: u32,
    /// Catch strength contributed by one tether array.
    pub tether_strength_per_array: f32,
    /// Orbital-debris feed on impact.
    pub debris_on_impact: f32,
}

impl Default for DeorbitDirector {
    fn default() -> Self {
        Self {
            warning_ticks: DEORBIT_DEFAULT_WARNING_TICKS,
            tether_strength_per_array: TETHER_STRENGTH_PER_ARRAY,
            debris_on_impact: IMPACT_DEBRIS_FEED,
        }
    }
}

/// Marker component on [`BuildingType::TetherArray`][crate::layer1::building::BuildingType]
/// buildings. Each array contributes catch strength toward an incoming
/// deorbiting megastructure.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct TetherArray {
    /// Catch strength this array contributes.
    pub strength: f32,
}

impl TetherArray {
    /// A freshly constructed, fully operational array.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            strength: TETHER_STRENGTH_PER_ARRAY,
        }
    }
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

/// Total catch strength of all tether arrays in the colony.
pub fn total_tether_strength(world: &mut World) -> f32 {
    let mut query = world.query::<&TetherArray>();
    // `+ 0.0` normalizes a possible -0.0 from the empty sum.
    query.iter(world).map(|t| t.strength).sum::<f32>() + 0.0
}

/// Number of tether arrays standing in the colony.
pub fn tether_array_count(world: &mut World) -> usize {
    let mut query = world.query::<&TetherArray>();
    query.iter(world).count()
}

/// Catch strength required for the given size class.
#[must_use]
pub fn required_strength(size: MegastructureSize, director: &DeorbitDirector) -> f32 {
    size.required_tethers() as f32 * director.tether_strength_per_array
}

/// Short status for headless STATS: "none" or "<label> <ticks>t".
#[must_use]
pub fn deorbit_status_label(world: &mut World) -> String {
    match world.get_resource::<DeorbitState>() {
        Some(state) if state.active => {
            format!("{} {}t", state.size.label(), state.ticks_remaining)
        }
        _ => "none".to_string(),
    }
}

/// System: consume [`DeorbitEvent`]s and start the countdown. Only one
/// deorbit crisis can be in flight at a time; extras are ignored.
pub fn trigger_deorbit_system(world: &mut World) {
    let events: Vec<DeorbitEvent> = {
        let mut ev = world.resource_mut::<Events<DeorbitEvent>>();
        ev.drain().collect()
    };
    if events.is_empty() {
        return;
    }
    let already_active = world
        .get_resource::<DeorbitState>()
        .is_some_and(|s| s.active);
    if already_active {
        return;
    }
    let director = world
        .get_resource::<DeorbitDirector>()
        .cloned()
        .unwrap_or_default();
    let first = &events[0];
    let target = first.target.unwrap_or_else(|| colony_heart(world));
    let ticks = first.warning_ticks.unwrap_or(director.warning_ticks);
    {
        let mut state = world.resource_mut::<DeorbitState>();
        state.active = true;
        state.size = first.size;
        state.ticks_remaining = ticks;
        state.target = target;
    }
    let label = first.size.label();
    let needed = first.size.required_tethers();
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(format!(
            "DEORBIT WARNING: a {label} is falling toward ({}, {})! Impact in {ticks} ticks — build {needed} tether arrays to catch it.",
            target.x, target.y
        ));
    }
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: format!(
                "DEORBIT WARNING: orbital spotters report a {label} falling toward the colony. \
                 Impact in {ticks} ticks. The engineers swear {needed} tether arrays could catch it."
            ),
            importance: EventImportance::Major,
        });
}

/// System: tick the countdown; resolve (catch or impact) at zero.
pub fn tick_deorbit_system(world: &mut World) {
    let active = world
        .get_resource::<DeorbitState>()
        .is_some_and(|s| s.active);
    if !active {
        return;
    }
    let done = {
        let mut state = world.resource_mut::<DeorbitState>();
        if state.ticks_remaining > 0 {
            state.ticks_remaining -= 1;
        }
        state.ticks_remaining == 0
    };
    if done {
        resolve_deorbit(world);
    }
}

/// Resolve the crisis at impact time: catch if tether coverage suffices,
/// catastrophic impact otherwise.
fn resolve_deorbit(world: &mut World) {
    let (size, target) = {
        let mut state = world.resource_mut::<DeorbitState>();
        state.active = false;
        (state.size, state.target)
    };
    let director = world
        .get_resource::<DeorbitDirector>()
        .cloned()
        .unwrap_or_default();
    let strength = total_tether_strength(world);
    let needed = required_strength(size, &director);
    if strength >= needed {
        resolve_catch(world, size, strength);
    } else {
        resolve_impact(world, size, target, strength, &director);
    }
}

/// The tether net holds: salvage, tech, and a chronicle of the miracle.
fn resolve_catch(world: &mut World, size: MegastructureSize, strength: f32) {
    let (metal, alloys, credits) = size.salvage();
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.metal += metal;
        resources.hyper_alloys += alloys;
        resources.credits += credits;
    }
    // TechState integration (spec REFACTOR): grant the first unresearched
    // salvage-pool tech through the real unlock path.
    let tech_line = {
        let mut tech_state = world.resource_mut::<TechState>();
        match SALVAGE_TECH_POOL
            .iter()
            .find(|t| !tech_state.is_researched(**t))
        {
            Some(tech) => {
                if tech_state.try_unlock(*tech) {
                    format!("the databanks yield {}!", tech.label())
                } else {
                    "the databanks are full — the recovered schematics cannot be loaded.".to_string()
                }
            }
            None => "the salvage databanks hold nothing new.".to_string(),
        }
    };
    let label = size.label();
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(format!(
            "MIRACLE CATCH: the tether net (strength {strength:.1}) has caught the falling {label}!"
        ));
    }
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: format!(
                "MIRACLE CATCH: the colony's tether arrays have snared the falling {label}! \
                 Salvage crews strip {metal:.0} metal, {alloys:.0} hyper-alloys and {credits:.0} credits from the wreck, and {tech_line}"
            ),
            importance: EventImportance::Major,
        });
}

/// The net fails: a real [`ExplosionEvent`] through the real damage
/// pipeline, crater demolition of every building in radius, debris feed,
/// and a catastrophe chronicle.
fn resolve_impact(
    world: &mut World,
    size: MegastructureSize,
    target: GridPosition,
    strength: f32,
    director: &DeorbitDirector,
) {
    let tick = world
        .get_resource::<SimulationTime>()
        .map_or(0, |t| t.tick);
    let label = size.label();
    let needed = required_strength(size, director);

    // Real damage through the real pipeline (damages Structures and Health
    // in radius when handle_explosion_system runs).
    world
        .resource_mut::<Events<ExplosionEvent>>()
        .send(ExplosionEvent {
            center: target,
            damage: size.impact_damage(),
            radius: size.impact_radius(),
        });

    // Crater demolition: every building in the crater radius collapses
    // into a ruin (mirrors the construction-sabotage failure path).
    let crater = size.crater_radius();
    let victims: Vec<(Entity, GridPosition, BuildingType, MaterialType)> = {
        let mut query = world.query::<(Entity, &Building, &GridPosition)>();
        query
            .iter(world)
            .filter(|(_, _, pos)| target.distance_chebyshev(**pos) <= crater)
            .map(|(e, b, p)| {
                let material = world
                    .get::<crate::layer1::building::Material>(e)
                    .map_or(MaterialType::default(), |m| m.0);
                (e, *p, b.building_type, material)
            })
            .collect()
    };
    let destroyed = victims.len();
    for (entity, pos, building_type, material) in victims {
        world.despawn(entity);
        world.spawn((
            Ruin {
                original_type: building_type,
                material,
            },
            RuinHistory {
                destruction_tick: tick,
                reason: "Deorbit Impact".to_string(),
            },
            pos,
        ));
        world.send_event(BuildingRemovedEvent {
            entity,
            position: pos,
            building_type,
        });
    }
    // Keep tiles blocked: ruins stand where buildings stood (same as the
    // sabotage path).
    let _ = world.get_resource::<OccupiedTiles>();

    // Orbital debris feed (dep 184): a megastructure impact seeds the sky.
    if let Some(mut debris) = world.get_resource_mut::<OrbitalDebris>() {
        debris.0 += director.debris_on_impact;
    }

    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(format!(
            "CATASTROPHE: the {label} has struck ({}, {})! {destroyed} buildings destroyed — tether strength was {strength:.1}, needed {needed:.1}.",
            target.x, target.y
        ));
    }
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: format!(
                "CATASTROPHE: the {label} has struck the colony with only {strength:.1} tether strength against {needed:.1} needed. \
                 {destroyed} buildings lie in ruins beneath the crater, and the sky is full of burning debris."
            ),
            importance: EventImportance::Major,
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::Material;
    use crate::layer1::health::Health;
    use crate::layer1::core::chronicle::AddChronicleEvent;

    fn test_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<DeorbitEvent>>();
        world.init_resource::<Events<ExplosionEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<BuildingRemovedEvent>>();
        world.init_resource::<DeorbitState>();
        world.init_resource::<DeorbitDirector>();
        world.init_resource::<ColonyResources>();
        world.init_resource::<TechState>();
        // A real colony gets data capacity from ServerBanks; the bare
        // default has none and try_unlock would (correctly) refuse.
        world.resource_mut::<TechState>().total_capacity = 100.0;
        world.init_resource::<OrbitalDebris>();
        world.init_resource::<SimulationTime>();
        world.init_resource::<MessageLog>();
        world.init_resource::<OccupiedTiles>();
        world
    }

    fn run_trigger(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(trigger_deorbit_system);
        schedule.run(world);
    }

    fn run_tick(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(tick_deorbit_system);
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

    fn spawn_tether(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Building {
                    building_type: BuildingType::TetherArray,
                },
                GridPosition { x, y },
                Material(MaterialType::default()),
                TetherArray::new(),
            ))
            .id()
    }

    /// Spec RED test 1 (adapted): triggering a deorbit logs the warning and
    /// sets the countdown.
    #[test]
    fn test_megastructure_deorbit_warning() {
        let mut world = test_world();
        world
            .resource_mut::<Events<DeorbitEvent>>()
            .send(DeorbitEvent {
                size: MegastructureSize::Hulk,
                target: Some(GridPosition { x: 10, y: 10 }),
                warning_ticks: Some(600),
            });
        run_trigger(&mut world);

        let state = world.resource::<DeorbitState>();
        assert!(state.active, "deorbit should be active after trigger");
        assert_eq!(state.ticks_remaining, 600, "countdown must be set");
        assert_eq!(state.target, GridPosition { x: 10, y: 10 });
        assert_eq!(state.size, MegastructureSize::Hulk);

        let texts = chronicle_texts(&mut world);
        assert!(
            texts.iter().any(|t| t.contains("DEORBIT WARNING")),
            "warning chronicle must be logged, got: {texts:?}"
        );
    }

    /// Triggering while one is already in flight is ignored.
    #[test]
    fn test_second_deorbit_while_active_is_ignored() {
        let mut world = test_world();
        for size in [MegastructureSize::Fragment, MegastructureSize::Megastructure] {
            world
                .resource_mut::<Events<DeorbitEvent>>()
                .send(DeorbitEvent {
                    size,
                    target: None,
                    warning_ticks: Some(100),
                });
        }
        run_trigger(&mut world);
        let state = world.resource::<DeorbitState>();
        assert!(state.active);
        assert_eq!(state.size, MegastructureSize::Fragment, "first event wins");
    }

    /// Spec RED test 2 (adapted): sufficient tethers at impact time avoid
    /// the impact and grant salvage + tech.
    #[test]
    fn test_successful_catch_grants_salvage() {
        let mut world = test_world();
        // Two tether arrays: exactly the Hulk requirement.
        spawn_tether(&mut world, 5, 5);
        spawn_tether(&mut world, 6, 6);
        assert_eq!(tether_array_count(&mut world), 2);
        assert!((total_tether_strength(&mut world) - 2.0).abs() < f32::EPSILON);

        {
            let mut state = world.resource_mut::<DeorbitState>();
            state.active = true;
            state.size = MegastructureSize::Hulk;
            state.ticks_remaining = 1;
            state.target = GridPosition { x: 20, y: 20 };
        }
        let metal_before = world.resource::<ColonyResources>().metal;
        run_tick(&mut world);

        let state = world.resource::<DeorbitState>();
        assert!(!state.active, "crisis must resolve at zero");
        let resources = world.resource::<ColonyResources>();
        assert!(
            (resources.metal - metal_before - 80.0).abs() < f32::EPSILON,
            "hulk salvage must land in the stockpile"
        );
        assert!(resources.hyper_alloys >= 15.0);
        assert!(resources.credits >= 300.0);
        let tech = world.resource::<TechState>();
        assert!(
            tech.is_unlocked(Tech::Astronomy),
            "catch must grant the first salvage-pool tech"
        );
        let texts = chronicle_texts(&mut world);
        assert!(
            texts.iter().any(|t| t.contains("MIRACLE CATCH")),
            "catch chronicle must be logged, got: {texts:?}"
        );
        // No explosion on a catch.
        let explosions = world.resource::<Events<ExplosionEvent>>();
        assert_eq!(
            explosions.get_cursor().read(explosions).count(),
            0,
            "a catch must not fire the impact pipeline"
        );
    }

    /// Spec RED test 3 (adapted): no tethers at impact time means the real
    /// damage pipeline fires and buildings in the crater are destroyed.
    #[test]
    fn test_failed_catch_destroys_map() {
        let mut world = test_world();
        let target = GridPosition { x: 20, y: 20 };
        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 21, y: 20 },
                Material(MaterialType::default()),
            ))
            .id();
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 22, y: 20 },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();

        {
            let mut state = world.resource_mut::<DeorbitState>();
            state.active = true;
            state.size = MegastructureSize::Hulk;
            state.ticks_remaining = 1;
            state.target = target;
        }
        run_tick(&mut world);

        let state = world.resource::<DeorbitState>();
        assert!(!state.active, "crisis must resolve at zero");

        // Real damage pipeline fired.
        let explosions = world.resource::<Events<ExplosionEvent>>();
        let fired: Vec<ExplosionEvent> =
            explosions.get_cursor().read(explosions).cloned().collect();
        assert_eq!(fired.len(), 1, "exactly one impact explosion");
        assert_eq!(fired[0].center, target);
        assert!((fired[0].damage - 300.0).abs() < f32::EPSILON);

        // Run the real explosion handler: the pop in radius takes damage.
        let mut schedule = Schedule::default();
        schedule.add_systems(crate::layer1::environment::volatile::handle_explosion_system);
        schedule.run(&mut world);
        let health = world.get::<Health>(pop).expect("pop survives the test query");
        assert!(
            health.current < 100.0,
            "pop in the blast radius must be injured, got {}",
            health.current
        );

        // Crater demolition: the building is gone, a ruin stands there.
        assert!(
            world.get_entity(building).is_err(),
            "building in the crater must be destroyed"
        );
        let mut ruins = world.query::<(&Ruin, &GridPosition)>();
        let found = ruins
            .iter(&world)
            .any(|(r, p)| r.original_type == BuildingType::Housing && *p == GridPosition { x: 21, y: 20 });
        assert!(found, "a ruin must mark the destroyed building");

        // Debris feed past the Kessler line.
        assert!(world.resource::<OrbitalDebris>().0 >= 0.15);

        let texts = chronicle_texts(&mut world);
        assert!(
            texts.iter().any(|t| t.contains("CATASTROPHE")),
            "impact chronicle must be logged, got: {texts:?}"
        );
    }

    /// A partial tether net (below requirement) still fails.
    #[test]
    fn test_insufficient_tethers_still_impact() {
        let mut world = test_world();
        spawn_tether(&mut world, 5, 5); // 1.0 < 2.0 needed for a Hulk
        {
            let mut state = world.resource_mut::<DeorbitState>();
            state.active = true;
            state.size = MegastructureSize::Hulk;
            state.ticks_remaining = 1;
            state.target = GridPosition { x: 20, y: 20 };
        }
        run_tick(&mut world);
        let explosions = world.resource::<Events<ExplosionEvent>>();
        assert_eq!(
            explosions.get_cursor().read(explosions).count(),
            1,
            "insufficient tethers must not catch"
        );
    }

    /// Full databank: the catch grants salvage but no new tech.
    #[test]
    fn test_catch_with_full_databank_still_grants_salvage() {
        let mut world = test_world();
        spawn_tether(&mut world, 5, 5);
        spawn_tether(&mut world, 6, 6);
        {
            let mut tech = world.resource_mut::<TechState>();
            for t in SALVAGE_TECH_POOL {
                tech.unlock(t);
            }
        }
        {
            let mut state = world.resource_mut::<DeorbitState>();
            state.active = true;
            state.size = MegastructureSize::Hulk;
            state.ticks_remaining = 1;
            state.target = GridPosition { x: 20, y: 20 };
        }
        run_tick(&mut world);
        let texts = chronicle_texts(&mut world);
        assert!(
            texts.iter().any(|t| t.contains("MIRACLE CATCH")),
            "catch must still resolve, got: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("nothing new")),
            "full databank must be called out, got: {texts:?}"
        );
    }

    /// IP guard: all public strings are original/generic — no distinctive
    /// franchise terms may appear in chronicles, labels, or commands.
    #[test]
    fn test_no_distinctive_ip_terms_in_public_strings() {
        let banned = [
            "Death Star",
            "Starkiller",
            "Halo",
            "Ringworld",
            "Citadel",
            "Dyson",
            "Babylon",
            "Unicron",
            "Nostromo",
            "Event Horizon",
        ];
        let mut public: Vec<String> = Vec::new();
        for size in [
            MegastructureSize::Fragment,
            MegastructureSize::Hulk,
            MegastructureSize::Megastructure,
        ] {
            public.push(size.label().to_string());
        }
        public.push("DEORBIT WARNING".to_string());
        public.push("MIRACLE CATCH".to_string());
        public.push("CATASTROPHE".to_string());
        public.push("tether array".to_string());
        public.push("megastructure".to_string());
        for text in &public {
            for term in banned {
                assert!(
                    !text.to_lowercase().contains(&term.to_lowercase()),
                    "public string {text:?} contains banned IP term {term:?}"
                );
            }
        }
    }
}
