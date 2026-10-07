//! # Panic Spirals (Spec 1371)
//!
//! Fear is contagious. Pops witnessing a "Terrifying Event" (Monster, Fire,
//! Corpse) flee in a random direction; pops sharing a tile with (or adjacent
//! to) a panicking pop catch the Panic state themselves. Panicked pops drop
//! whatever they are carrying and cannot work.
//!
//! Design tension: crowd control (wide halls break the chains) vs. efficiency
//! (tight spaces propagate the spiral).
//!
//! Interaction with Spec 1370 (Civilizational Compost): an unburied corpse is
//! a terrifying sight while it rots (decay < 1.0). Burying it (despawn via
//! `bury_corpse`) removes the terror; full decomposition (despawn) ends it.
//!
//! Counterplay built in:
//! - [`Panic`] carries a timer: pops calm down on their own.
//! - The [`Trait::Courage`] trait (see [`crate::layer1::psychology::traits`])
//!   resists cascades: courageous pops only panic at very close range and only
//!   catch spreading panic when surrounded by two or more panickers.
//! - [`Trait::Psychopath`] ("unaffected by horror") pops never panic at all.
//! - Spatial lookup is a one-pass `HashMap` tile grid, not O(N^2) pairwise.

use std::collections::{HashMap, HashSet};

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::culture::funeral::Corpse;
use crate::layer1::defense::is_walkable;
use crate::layer1::economy::items::CarryingItem;
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::fauna::{Fauna, FaunaState};
use crate::layer1::map::GridPosition;
use crate::layer1::nature::fire::Fire;
use crate::layer1::pop::{Pop, PopName};
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::utility_types::{ActionType, PopAction};

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Witness radius (Chebyshev) for terrifying events: a pop this close to a
/// terrifying sighting panics.
pub const TERRIFY_RADIUS: u32 = 6;
/// Courageous pops only panic when the horror is very close.
pub const COURAGEOUS_TERRIFY_RADIUS: u32 = 2;
/// Ticks a panic lasts before the pop calms down.
pub const PANIC_DURATION_TICKS: u32 = 60;
/// A courageous pop catches spreading panic only when this many panicking
/// pops are adjacent (strength in numbers overwhelms courage).
pub const COURAGEOUS_SPREAD_OVERWHELM: usize = 2;
/// Maximum panic chronicle entries emitted per system run (spam guard — a
/// crowded colony discovering one corpse should not fill the chronicle).
pub const PANIC_CHRONICLE_CAP_PER_RUN: usize = 2;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Kind of a terrifying sighting, used for chronicle flavor text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrifyingKind {
    /// A monster: a hostile fauna (chasing/attacking) or a [`Terrifying`]
    /// marker spawned by an event.
    Monster,
    /// A burning fire.
    Fire,
    /// A rotting, unburied corpse.
    Corpse,
    /// A fellow colonist screaming and fleeing (panic spreading).
    PanickingPop,
}

impl TerrifyingKind {
    /// Short noun phrase for chronicle text.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Monster => "a monster",
            Self::Fire => "a fire",
            Self::Corpse => "a rotting corpse",
            Self::PanickingPop => "a screaming colonist",
        }
    }
}

/// Marker for an entity that is intrinsically terrifying to witness
/// (a monster sighting, a haunting apparition, a debug `scare` spawn).
#[derive(Component, Debug, Clone, Copy)]
pub struct Terrifying {
    /// What kind of horror this is (chronicle flavor).
    pub kind: TerrifyingKind,
}

/// A pop gripped by panic: flees, drops items, cannot work.
///
/// The timer ([`PANIC_DURATION_TICKS`]) counts down in
/// [`panic_timer_system`]; at zero the pop calms down and the component is
/// removed. `fled_from` remembers the sighting that caused the panic so the
/// flee system runs *away* from it.
#[derive(Component, Debug, Clone, Copy)]
pub struct Panic {
    /// Ticks remaining before the pop calms down.
    pub timer: u32,
    /// Tile of the terrifying sighting that caused this panic.
    pub fled_from: GridPosition,
}

impl Panic {
    /// Create a fresh panic with the full duration.
    #[must_use]
    pub const fn new(fled_from: GridPosition) -> Self {
        Self {
            timer: PANIC_DURATION_TICKS,
            fled_from,
        }
    }
}

// ---------------------------------------------------------------------------
// Terrifying sightings
// ---------------------------------------------------------------------------

/// One terrifying thing a pop can witness: its tile and what it is.
#[derive(Debug, Clone, Copy)]
pub struct TerrifyingSighting {
    /// Tile the horror occupies.
    pub pos: GridPosition,
    /// What kind of horror it is.
    pub kind: TerrifyingKind,
}

/// Collect every terrifying sighting in the world right now.
///
/// Sources (Spec 1371's three kinds, adapted to the real codebase):
/// - entities carrying [`Terrifying`] (monster sightings / event spawns),
/// - [`Fire`] entities (burning fires),
/// - [`Corpse`] entities with `decay < 1.0` (rotting, unburied corpses —
///   burial despawns the corpse via `bury_corpse`, full decomposition
///   despawns it via compost; either way the terror ends),
/// - [`Fauna`] in `Chase`/`Attack` state (hostile monsters in the flesh).
///
/// Uses one-pass queries plus a `HashSet` dedupe — no O(N^2) pairwise work.
#[must_use]
pub fn collect_terrifying_sightings(world: &mut World) -> Vec<TerrifyingSighting> {
    let mut out = Vec::new();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let push = |out: &mut Vec<TerrifyingSighting>,
                    seen: &mut HashSet<(i32, i32)>,
                    pos: GridPosition,
                    kind: TerrifyingKind| {
        if seen.insert((pos.x, pos.y)) {
            out.push(TerrifyingSighting { pos, kind });
        }
    };

    {
        let mut q = world.query::<(&GridPosition, &Terrifying)>();
        for (pos, terrifying) in q.iter(world) {
            push(&mut out, &mut seen, *pos, terrifying.kind);
        }
    }
    {
        let mut q = world.query_filtered::<&GridPosition, With<Fire>>();
        for pos in q.iter(world) {
            push(&mut out, &mut seen, *pos, TerrifyingKind::Fire);
        }
    }
    {
        let mut q = world.query::<(&GridPosition, &Corpse)>();
        for (pos, corpse) in q.iter(world) {
            // A fully-rotted corpse is compost (despawned by the compost
            // system); only the rotting one is a horror.
            if corpse.decay < 1.0 {
                push(&mut out, &mut seen, *pos, TerrifyingKind::Corpse);
            }
        }
    }
    {
        let mut q = world.query::<(&GridPosition, &Fauna)>();
        for (pos, fauna) in q.iter(world) {
            if matches!(fauna.state, FaunaState::Chase | FaunaState::Attack) {
                push(&mut out, &mut seen, *pos, TerrifyingKind::Monster);
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Panic application
// ---------------------------------------------------------------------------

/// Chronicle helper: init the resource if missing (tests), then send.
fn send_panic_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    world.init_resource::<Events<AddChronicleEvent>>();
    world.resource_mut::<Events<AddChronicleEvent>>().send(AddChronicleEvent {
        text: text.to_string(),
        importance,
    });
}

/// Drop whatever the pop is carrying at their tile (the hauling payload goes
/// everywhere — this is the emergent "dropped the explosive payload" hook).
///
/// Mirrors the drop half of `hauling::drop_item_on_ground`: the item entity is
/// re-grounded at the pop's tile, the pop's [`CarryingItem`] is removed, and
/// any in-flight movement targets are abandoned.
fn drop_carried_items(world: &mut World, pop: Entity, pos: GridPosition) {
    let carrying = world.get::<CarryingItem>(pop).copied();
    let Some(CarryingItem(item)) = carrying else {
        return;
    };
    if world.get_entity(item).is_ok() {
        world.entity_mut(item).insert(pos);
        // Photophobic items track a container parent; dropping to the ground
        // clears that.
        world
            .entity_mut(item)
            .remove::<crate::layer1::environment::photophobic::Parent>();
    }
    world.entity_mut(pop).remove::<CarryingItem>();
    world.entity_mut(pop).remove::<MovementTarget>();
    world.entity_mut(pop).remove::<AtTarget>();
}

/// Try to push a pop into the [`Panic`] state.
///
/// Returns `true` when the panic took hold. Resistance rules:
/// - [`Trait::Psychopath`] pops are immune ("unaffected by horror");
/// - [`Trait::Courage`] pops only panic from direct sightings within
///   [`COURAGEOUS_TERRIFY_RADIUS`], and only catch *spread* panic when
///   surrounded by at least [`COURAGEOUS_SPREAD_OVERWHELM`] panickers
///   (checked by the caller via `adjacent_panickers`).
///
/// On success the pop abandons work ([`ActionType::Idle`]), drops carried
/// items, and a chronicle entry fires (spam-capped by the caller).
pub fn try_panic_pop(
    world: &mut World,
    pop: Entity,
    sighting: TerrifyingSighting,
    distance: u32,
    is_spread: bool,
    adjacent_panickers: usize,
) -> bool {
    if world.get::<Panic>(pop).is_some() {
        return false;
    }
    let traits = world.get::<Traits>(pop).cloned().unwrap_or_default();
    if traits.has(Trait::Psychopath) {
        return false;
    }
    if traits.has(Trait::Courage) {
        if is_spread {
            if adjacent_panickers < COURAGEOUS_SPREAD_OVERWHELM {
                return false;
            }
        } else if distance > COURAGEOUS_TERRIFY_RADIUS {
            return false;
        }
    }

    world.entity_mut(pop).insert(Panic::new(sighting.pos));
    drop_carried_items(
        world,
        pop,
        world.get::<GridPosition>(pop).copied().unwrap_or_default(),
    );
    // Panicked pops abandon work: the utility AI never picks them up again
    // (see the `Without<Panic>` filter in `collect_pop_data`), and their
    // current action is reset so no in-flight work system keeps driving them.
    if let Some(mut action) = world.get_mut::<PopAction>(pop) {
        action.current = ActionType::Idle;
        action.current_utility = 0.0;
        action.ticks_committed = 0;
    }
    true
}

/// Chronicle the panic of a pop, honoring the per-run spam cap.
fn chronicle_panic(
    world: &mut World,
    pop: Entity,
    sighting: TerrifyingSighting,
    emitted: &mut usize,
) {
    if *emitted >= PANIC_CHRONICLE_CAP_PER_RUN {
        return;
    }
    *emitted += 1;
    let name = world
        .get::<PopName>(pop)
        .map_or_else(|| "Someone".to_string(), |n| n.0.clone());
    send_panic_chronicle(
        world,
        &format!("{name} panics at the sight of {}!", sighting.kind.label()),
        EventImportance::Minor,
    );
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Trigger: pops witnessing a terrifying event panic.
///
/// Each calm pop within [`TERRIFY_RADIUS`] (Chebyshev) of a sighting is
/// offered panic via [`try_panic_pop`] (courage/psychopath resistance
/// applies). Chronicle output is spam-capped.
pub fn trigger_panic_system(world: &mut World) {
    let sightings = collect_terrifying_sightings(world);
    if sightings.is_empty() {
        return;
    }
    let pops: Vec<(Entity, GridPosition)> = {
        let mut q = world.query_filtered::<(Entity, &GridPosition), (With<Pop>, Without<Panic>)>();
        q.iter(world).map(|(e, p)| (e, *p)).collect()
    };
    let mut emitted = 0;
    for (pop, pos) in pops {
        for sighting in &sightings {
            let distance = pos.distance_chebyshev(sighting.pos);
            if distance <= TERRIFY_RADIUS
                && try_panic_pop(world, pop, *sighting, distance, false, 0)
            {
                chronicle_panic(world, pop, *sighting, &mut emitted);
                break;
            }
        }
    }
}

/// Spread: pops sharing a tile with (or adjacent to) a panicking pop catch
/// the panic themselves.
///
/// Spatial lookup is a one-pass tile `HashMap`, not O(N^2) pairwise checks:
/// build calm-pop buckets per tile once, then probe the 3x3 neighborhood of
/// each panicker.
pub fn spread_panic_system(world: &mut World) {
    let panickers: Vec<(Entity, GridPosition, GridPosition)> = {
        let mut q = world.query_filtered::<(Entity, &GridPosition, &Panic), With<Pop>>();
        q.iter(world)
            .map(|(e, pos, panic)| (e, *pos, panic.fled_from))
            .collect()
    };
    if panickers.is_empty() {
        return;
    }
    // Calm pops, bucketed by tile.
    let mut calm_by_tile: HashMap<(i32, i32), Vec<Entity>> = HashMap::new();
    {
        let mut q = world.query_filtered::<(Entity, &GridPosition), (With<Pop>, Without<Panic>)>();
        for (e, pos) in q.iter(world) {
            calm_by_tile.entry((pos.x, pos.y)).or_default().push(e);
        }
    }
    let mut emitted = 0;
    // Accumulate adjacent-panicker counts across ALL panickers first: a
    // courageous pop is overwhelmed only when the crowd of panickers around
    // it (not the neighborhood of a single panicker) is big enough.
    let mut candidates: HashMap<Entity, usize> = HashMap::new();
    let mut origins: HashMap<Entity, GridPosition> = HashMap::new();
    for (_panicker, pos, fled_from) in panickers {
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(ents) = calm_by_tile.get(&(pos.x + dx, pos.y + dy)) {
                    for e in ents {
                        *candidates.entry(*e).or_default() += 1;
                        origins.entry(*e).or_insert(fled_from);
                    }
                }
            }
        }
    }
    for (candidate, adjacent) in candidates {
        let fled_from = origins.get(&candidate).copied().unwrap_or_default();
        let spread_sighting = TerrifyingSighting {
            // The new panicker flees *away from the horror that started the
            // chain*, not from the panicker that scared them.
            pos: fled_from,
            kind: TerrifyingKind::PanickingPop,
        };
        if try_panic_pop(world, candidate, spread_sighting, 1, true, adjacent) {
            chronicle_panic(
                world,
                candidate,
                TerrifyingSighting {
                    pos: fled_from,
                    kind: TerrifyingKind::PanickingPop,
                },
                &mut emitted,
            );
        }
    }
}

/// Panic timer: pops calm down after [`PANIC_DURATION_TICKS`].
pub fn panic_timer_system(world: &mut World) {
    let mut calmed: Vec<(Entity, String)> = Vec::new();
    {
        let mut q = world.query_filtered::<(Entity, &mut Panic, Option<&PopName>), With<Pop>>();
        for (e, mut panic, name) in q.iter_mut(world) {
            panic.timer = panic.timer.saturating_sub(1);
            if panic.timer == 0 {
                calmed.push((
                    e,
                    name.map_or_else(|| "Someone".to_string(), |n| n.0.clone()),
                ));
            }
        }
    }
    for (e, name) in calmed {
        world.entity_mut(e).remove::<Panic>();
        send_panic_chronicle(
            world,
            &format!("{name} catches their breath and calms down."),
            EventImportance::Minor,
        );
    }
}

/// Flee: panicked pops move one tile per tick away from the horror.
///
/// Picks a random walkable neighbor maximizing Chebyshev distance from the
/// `fled_from` tile; falls back to any random walkable neighbor; stands
/// still if boxed in.
pub fn panic_flee_system(world: &mut World) {
    const DIRS: [(i32, i32); 8] = [
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ];
    let panickers: Vec<(Entity, GridPosition, GridPosition)> = {
        let mut q = world.query_filtered::<(Entity, &GridPosition, &Panic), With<Pop>>();
        q.iter(world)
            .map(|(e, pos, panic)| (e, *pos, panic.fled_from))
            .collect()
    };
    for (e, pos, fled_from) in panickers {
        let old_distance = pos.distance_chebyshev(fled_from);
        let mut walkable: Vec<(i32, i32, u32)> = Vec::new();
        for (dx, dy) in DIRS {
            let (nx, ny) = (pos.x + dx, pos.y + dy);
            if is_walkable(world, nx, ny) {
                let d = GridPosition { x: nx, y: ny }.distance_chebyshev(fled_from);
                walkable.push((nx, ny, d));
            }
        }
        if walkable.is_empty() {
            continue;
        }
        // Prefer tiles strictly farther from the horror; fall back to any
        // walkable tile so the pop at least keeps moving.
        let best_distance = walkable
            .iter()
            .map(|(_, _, d)| *d)
            .max()
            .unwrap_or(old_distance);
        let away: Vec<(i32, i32)> = walkable
            .iter()
            .filter(|(_, _, d)| *d == best_distance && *d > old_distance)
            .map(|(x, y, _)| (*x, *y))
            .collect();
        let pool: Vec<(i32, i32)> = if away.is_empty() {
            walkable.iter().map(|(x, y, _)| (*x, *y)).collect()
        } else {
            away
        };
        let idx = rand::thread_rng().gen_range(0..pool.len());
        let (nx, ny) = pool[idx];
        if let Some(mut grid_pos) = world.get_mut::<GridPosition>(e) {
            grid_pos.x = nx;
            grid_pos.y = ny;
        }
    }
}

// ---------------------------------------------------------------------------
// Headless helpers
// ---------------------------------------------------------------------------

/// Spawn a [`Terrifying`] sighting at a tile (headless `scare` command,
/// playtests).
pub fn spawn_terrifying(world: &mut World, x: i32, y: i32, kind: TerrifyingKind) -> Entity {
    world
        .spawn((Terrifying { kind }, GridPosition { x, y }))
        .id()
}

/// Count currently panicking pops (headless STATS `panicking=`).
#[must_use]
pub fn panicking_count(world: &mut World) -> usize {
    let mut q = world.query_filtered::<Entity, (With<Pop>, With<Panic>)>();
    q.iter(world).count()
}

// ---------------------------------------------------------------------------
// Tests (atomic TDD: RED first, adapted to the real codebase)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::items::{Item, ItemType};
    use crate::layer1::nature::terrain::{
        generate_terrain, TerrainGrid, TerrainType,
    };
    use crate::shared::time::SimulationTime;

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        // Walkable everywhere, so the flee system has somewhere to run.
        world.insert_resource(generate_terrain(60, 60));
        world
    }

    /// A world with a fully open, deterministic field: every tile walkable
    /// Grass (no random rocks/water from `generate_terrain`).
    fn setup_open_field() -> World {
        let mut world = setup();
        world.insert_resource(TerrainGrid {
            width: 60,
            height: 60,
            tiles: vec![TerrainType::Grass; 60 * 60],
        });
        world
    }

    fn spawn_pop(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Pop,
                PopName("Testy".to_string()),
                GridPosition { x, y },
                PopAction::default(),
                Traits::default(),
            ))
            .id()
    }

    fn spawn_courageous_pop(world: &mut World, x: i32, y: i32) -> Entity {
        let pop = spawn_pop(world, x, y);
        world
            .get_mut::<Traits>(pop)
            .expect("traits")
            .add(Trait::Courage);
        pop
    }

    fn spawn_psychopath_pop(world: &mut World, x: i32, y: i32) -> Entity {
        let pop = spawn_pop(world, x, y);
        world
            .get_mut::<Traits>(pop)
            .expect("traits")
            .add(Trait::Psychopath);
        pop
    }

    fn spawn_monster(world: &mut World, x: i32, y: i32) -> Entity {
        spawn_terrifying(world, x, y, TerrifyingKind::Monster)
    }

    fn spawn_fire(world: &mut World, x: i32, y: i32) -> Entity {
        world.spawn((Fire::default(), GridPosition { x, y })).id()
    }

    fn spawn_corpse(world: &mut World, x: i32, y: i32, decay: f32) -> Entity {
        world
            .spawn((
                Corpse {
                    name: "Poor Yorick".to_string(),
                    decay,
                },
                GridPosition { x, y },
            ))
            .id()
    }

    fn spawn_hostile_fauna(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Fauna {
                    state: FaunaState::Attack,
                    ..Default::default()
                },
                GridPosition { x, y },
            ))
            .id()
    }

    fn drain_chronicle(world: &mut World) -> Vec<String> {
        let texts: Vec<String> = world
            .resource::<Events<AddChronicleEvent>>()
            .iter_current_update_events()
            .map(|e| e.text.clone())
            .collect();
        world.resource_mut::<Events<AddChronicleEvent>>().clear();
        texts
    }

    // --- trigger ----------------------------------------------------------

    #[test]
    fn test_pop_panics_near_terrifying_event() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 6); // adjacent

        trigger_panic_system(&mut world);

        let panic = world.get::<Panic>(pop).expect("pop should be panicking");
        assert_eq!(panic.timer, PANIC_DURATION_TICKS);
        assert_eq!(panic.fled_from, GridPosition { x: 5, y: 5 });
    }

    #[test]
    fn test_pop_does_not_panic_far_from_terrifying_event() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 5 + TERRIFY_RADIUS as i32 + 1);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none());
    }

    #[test]
    fn test_pop_panics_near_fire() {
        let mut world = setup();
        spawn_fire(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, 11, 10);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_some());
    }

    #[test]
    fn test_pop_panics_near_rotting_corpse() {
        let mut world = setup();
        spawn_corpse(&mut world, 20, 20, 0.5);
        let pop = spawn_pop(&mut world, 21, 20);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_some());
    }

    #[test]
    fn test_pop_panics_near_hostile_fauna() {
        let mut world = setup();
        spawn_hostile_fauna(&mut world, 30, 30);
        let pop = spawn_pop(&mut world, 31, 30);

        trigger_panic_system(&mut world);

        let panic = world.get::<Panic>(pop).expect("should panic");
        assert_eq!(panic.fled_from, GridPosition { x: 30, y: 30 });
    }

    #[test]
    fn test_pop_ignores_wandering_fauna() {
        let mut world = setup();
        let _ = world
            .spawn((
                Fauna {
                    state: FaunaState::Wander,
                    ..Default::default()
                },
                GridPosition { x: 40, y: 40 },
            ))
            .id();
        let pop = spawn_pop(&mut world, 41, 40);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none());
    }

    #[test]
    fn test_psychopath_immune_to_terror() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_psychopath_pop(&mut world, 5, 6);

        trigger_panic_system(&mut world);

        assert!(
            world.get::<Panic>(pop).is_none(),
            "psychopaths are unaffected by horror"
        );
    }

    #[test]
    fn test_courage_resists_distant_terror() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        // Distance 3: inside normal radius, outside courageous radius.
        let pop = spawn_courageous_pop(&mut world, 8, 5);

        trigger_panic_system(&mut world);

        assert!(
            world.get::<Panic>(pop).is_none(),
            "courageous pops shrug off distant horror"
        );
    }

    #[test]
    fn test_courage_panics_up_close() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_courageous_pop(&mut world, 6, 5);

        trigger_panic_system(&mut world);

        assert!(
            world.get::<Panic>(pop).is_some(),
            "even courage breaks at arm's length"
        );
    }

    #[test]
    fn test_panic_chronicle_fires() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        spawn_pop(&mut world, 5, 6);

        trigger_panic_system(&mut world);

        let log = drain_chronicle(&mut world);
        assert!(
            log.iter().any(|t| t.contains("panics at the sight of a monster")),
            "panic chronicle should fire, got: {log:?}"
        );
    }

    #[test]
    fn test_panic_chronicle_spam_capped() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        for i in 0..10 {
            spawn_pop(&mut world, 5 + i, 6);
        }

        trigger_panic_system(&mut world);

        let log = drain_chronicle(&mut world);
        assert!(
            log.len() <= PANIC_CHRONICLE_CAP_PER_RUN,
            "chronicle spam-capped, got {} entries",
            log.len()
        );
    }

    // --- spread -----------------------------------------------------------

    #[test]
    fn test_panic_spreads_on_collision() {
        let mut world = setup();
        // Panicked pop.
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        // Normal pop on the same tile.
        let normal = spawn_pop(&mut world, 10, 10);

        spread_panic_system(&mut world);

        assert!(
            world.get::<Panic>(normal).is_some(),
            "sharing a tile with a panicker spreads panic"
        );
    }

    #[test]
    fn test_panic_spreads_to_adjacent() {
        let mut world = setup();
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        let normal = spawn_pop(&mut world, 11, 11); // diagonal neighbor

        spread_panic_system(&mut world);

        assert!(
            world.get::<Panic>(normal).is_some(),
            "adjacent pops catch the panic"
        );
    }

    #[test]
    fn test_panic_does_not_spread_far() {
        let mut world = setup();
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        let far = spawn_pop(&mut world, 12, 10); // two tiles away

        spread_panic_system(&mut world);

        assert!(world.get::<Panic>(far).is_none());
    }

    #[test]
    fn test_courage_resists_single_panicker_spread() {
        let mut world = setup();
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        let brave = spawn_courageous_pop(&mut world, 10, 10);

        spread_panic_system(&mut world);

        assert!(
            world.get::<Panic>(brave).is_none(),
            "one panicker does not rattle the courageous"
        );
    }

    #[test]
    fn test_courage_overwhelmed_by_crowd_spread() {
        let mut world = setup();
        for (x, y) in [(10, 10), (10, 11)] {
            let p = spawn_pop(&mut world, x, y);
            world
                .entity_mut(p)
                .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        }
        let brave = spawn_courageous_pop(&mut world, 11, 10);

        spread_panic_system(&mut world);

        assert!(
            world.get::<Panic>(brave).is_some(),
            "two adjacent panickers overwhelm courage"
        );
    }

    #[test]
    fn test_psychopath_immune_to_spread() {
        let mut world = setup();
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        let psycho = spawn_psychopath_pop(&mut world, 10, 10);

        spread_panic_system(&mut world);

        assert!(world.get::<Panic>(psycho).is_none());
    }

    // --- timer ------------------------------------------------------------

    #[test]
    fn test_panic_timer_decrements() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(pop)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));

        panic_timer_system(&mut world);

        let panic = world.get::<Panic>(pop).expect("still panicking");
        assert_eq!(panic.timer, PANIC_DURATION_TICKS - 1);
    }

    #[test]
    fn test_panic_ends_and_chronicles() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);
        world.entity_mut(pop).insert(Panic {
            timer: 1,
            fled_from: GridPosition { x: 1, y: 1 },
        });

        panic_timer_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none(), "panic should end");
        let log = drain_chronicle(&mut world);
        assert!(
            log.iter().any(|t| t.contains("calms down")),
            "calm-down chronicle should fire, got: {log:?}"
        );
    }

    // --- flee -------------------------------------------------------------

    #[test]
    fn test_panicked_pop_flees_away_from_horror() {
        let mut world = setup_open_field();
        let pop = spawn_pop(&mut world, 30, 31);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 30, y: 30 },
        });
        let before = world.get::<GridPosition>(pop).copied().unwrap();

        panic_flee_system(&mut world);

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        assert_ne!(before, after, "a panicking pop must move");
        assert!(
            after.distance_chebyshev(GridPosition { x: 30, y: 30 })
                >= before.distance_chebyshev(GridPosition { x: 30, y: 30 }),
            "fleeing must not move toward the horror"
        );
    }

    #[test]
    fn test_flee_prefers_increasing_distance() {
        let mut world = setup_open_field();
        // Horror directly south; every neighbor north is walkable and
        // farther, so the pop must end up strictly farther away.
        let pop = spawn_pop(&mut world, 30, 31);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 30, y: 40 },
        });

        panic_flee_system(&mut world);

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        let horror = GridPosition { x: 30, y: 40 };
        assert!(
            after.distance_chebyshev(horror) > GridPosition { x: 30, y: 31 }.distance_chebyshev(horror),
            "flee should increase distance, ended at {after:?}"
        );
    }

    // --- items & work -----------------------------------------------------

    #[test]
    fn test_panicked_pop_drops_carried_item() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 6);
        let item = world
            .spawn((Item {
                item_type: ItemType::Potato,
            },))
            .id();
        world.entity_mut(pop).insert(CarryingItem(item));
        let pop_pos = world.get::<GridPosition>(pop).copied().unwrap();

        trigger_panic_system(&mut world);

        assert!(
            world.get::<CarryingItem>(pop).is_none(),
            "panicked pop drops the payload"
        );
        assert_eq!(
            world.get::<GridPosition>(item).copied(),
            Some(pop_pos),
            "dropped item lands on the pop's tile"
        );
    }

    #[test]
    fn test_panic_resets_action_to_idle() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 6);
        world.entity_mut(pop).insert(PopAction {
            current: ActionType::Work,
            current_utility: 0.9,
            ticks_committed: 40,
        });

        trigger_panic_system(&mut world);

        let action = world.get::<PopAction>(pop).expect("action");
        assert_eq!(
            action.current,
            ActionType::Idle,
            "panicked pops abandon work for flight"
        );
    }

    // --- sightings --------------------------------------------------------

    #[test]
    fn test_collect_terrifying_sightings_kinds() {
        let mut world = setup();
        spawn_monster(&mut world, 1, 1);
        spawn_fire(&mut world, 2, 2);
        spawn_corpse(&mut world, 3, 3, 0.2);
        spawn_hostile_fauna(&mut world, 4, 4);

        let sightings = collect_terrifying_sightings(&mut world);
        let kinds: HashSet<TerrifyingKind> =
            sightings.iter().map(|s| s.kind).collect();

        assert!(kinds.contains(&TerrifyingKind::Monster));
        assert!(kinds.contains(&TerrifyingKind::Fire));
        assert!(kinds.contains(&TerrifyingKind::Corpse));
        assert_eq!(sightings.len(), 4, "got {sightings:?}");
    }

    #[test]
    fn test_fully_rotted_corpse_not_terrifying() {
        let mut world = setup();
        // decay == 1.0: compost-ready, despawned by the compost system.
        spawn_corpse(&mut world, 3, 3, 1.0);
        let pop = spawn_pop(&mut world, 4, 3);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none());
    }

    #[test]
    fn test_terrifying_kind_labels() {
        assert_eq!(TerrifyingKind::Monster.label(), "a monster");
        assert_eq!(TerrifyingKind::Fire.label(), "a fire");
        assert_eq!(TerrifyingKind::Corpse.label(), "a rotting corpse");
        assert_eq!(TerrifyingKind::PanickingPop.label(), "a screaming colonist");
    }

    #[test]
    fn test_panicking_count() {
        let mut world = setup();
        let a = spawn_pop(&mut world, 1, 1);
        spawn_pop(&mut world, 2, 2);
        world
            .entity_mut(a)
            .insert(Panic::new(GridPosition { x: 0, y: 0 }));

        assert_eq!(panicking_count(&mut world), 1);
    }

    #[test]
    fn test_no_panic_without_sightings() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);

        trigger_panic_system(&mut world);
        spread_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none());
    }

    #[test]
    fn test_flee_boxed_in_stays_put() {
        let mut world = setup();
        // Wall the pop in with rock on all 8 neighbors.
        {
            let mut grid = world.resource_mut::<TerrainGrid>();
            for (dx, dy) in [
                (-1, -1),
                (-1, 0),
                (-1, 1),
                (0, -1),
                (0, 1),
                (1, -1),
                (1, 0),
                (1, 1),
            ] {
                let idx = (31 + dy) as usize * 60 + (30 + dx) as usize;
                grid.tiles[idx] = TerrainType::Rock;
            }
        }
        let pop = spawn_pop(&mut world, 30, 31);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 30, y: 30 },
        });

        panic_flee_system(&mut world);

        assert_eq!(
            world.get::<GridPosition>(pop).copied(),
            Some(GridPosition { x: 30, y: 31 }),
            "a boxed-in pop holds position instead of walking through rock"
        );
    }

    #[test]
    fn test_flee_falls_back_when_no_neighbor_farther() {
        let mut world = setup();
        // Horror five tiles south. Wall every neighbor except (30,34)
        // (d=4, closer) and (29,35)/(31,35) (d=5, same) — none strictly
        // farther than the current d=5, so the pop must still move (fallback
        // pool) rather than freeze.
        {
            let mut grid = world.resource_mut::<TerrainGrid>();
            // Keep only (30,34) d=4, (29,35)/(31,35) d=5 walkable — none
            // strictly farther than the pop's current d=5.
            let keep = [(0, -1), (-1, 0), (1, 0)];
            for (dx, dy) in [
                (-1, -1),
                (-1, 0),
                (-1, 1),
                (0, -1),
                (0, 1),
                (1, -1),
                (1, 0),
                (1, 1),
            ] {
                if !keep.contains(&(dx, dy)) {
                    let idx = (35 + dy) as usize * 60 + (30 + dx) as usize;
                    grid.tiles[idx] = TerrainType::Rock;
                }
            }
        }
        let pop = spawn_pop(&mut world, 30, 35);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 30, y: 30 },
        });
        let before = world.get::<GridPosition>(pop).copied().unwrap();

        panic_flee_system(&mut world);

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        assert_ne!(before, after, "a panicking pop keeps moving");
        assert!(
            after.distance_chebyshev(GridPosition { x: 30, y: 30 }) <= 5,
            "fallback tiles are no farther than the start, got {after:?}"
        );
    }

    #[test]
    fn test_try_panic_pop_rejects_already_panicked() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 5, 6);
        world
            .entity_mut(pop)
            .insert(Panic::new(GridPosition { x: 5, y: 5 }));
        let sighting = TerrifyingSighting {
            pos: GridPosition { x: 5, y: 5 },
            kind: TerrifyingKind::Monster,
        };

        assert!(!try_panic_pop(&mut world, pop, sighting, 1, false, 0));
        assert_eq!(
            world.get::<Panic>(pop).unwrap().timer,
            PANIC_DURATION_TICKS,
            "re-panicking must not refresh the timer"
        );
    }

    #[test]
    fn test_panic_drops_missing_item_gracefully() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 6);
        let item = world
            .spawn((Item {
                item_type: ItemType::Potato,
            },))
            .id();
        world.entity_mut(pop).insert(CarryingItem(item));
        // The item entity is gone (consumed elsewhere) — panic must not crash.
        world.despawn(item);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_some());
        assert!(world.get::<CarryingItem>(pop).is_none());
    }
}
