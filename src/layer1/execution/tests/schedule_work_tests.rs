//! Schedule-level regression tests for designation work.
//!
//! The direct-call tests in `work_tests.rs` invoke `work_execution_system`
//! by hand, so they cannot detect the system missing from the live
//! schedule. These tests instead build the world exactly the way the
//! game does (`setup_world_with_config`) and advance it with
//! `run_simulation_tick` — the same entry point used by the headless,
//! desktop, and WASM binaries — then watch Chop/Mine/Repair work travel
//! the whole pipeline:
//!
//! designation placed -> utility AI selects Work/Repair -> pop travels ->
//! pop arrives (`AtTarget`) -> work executes -> designation consumed /
//! structure restored -> yield exists as items / stockpile growth.
//!
//! Each stage has its own assertion message so a failure names the exact
//! link that broke (selection, movement/arrival, or execution).
//!
//! Arrangement notes (all disclosed, all about isolating the pipeline):
//! * The ambient starter colony is replaced by a FRESH workforce
//!   (`fresh_workforce`): the random colony's pops arrive with hundreds
//!   of ticks of entangled state (hauling contracts, panic history,
//!   job assignments, bureaucracy locks) that can capture or
//!   incapacitate them for reasons owned by other specs' systems.
//!   Fresh pops spawned from the game's own `PopBundle::random` carry
//!   none of that, so what remains is the pipeline itself. Organic
//!   colony-wide gathering is exercised end-to-end by headless
//!   playtests instead.
//! * Work sites are chosen inside the colony's pressure envelope:
//!   map edges are vacuum, and a pop working an unpressurized tile
//!   suffocates (1 HP/tick below 0.2 pressure) before finishing —
//!   measured directly during this fix's development. Players
//!   designate near their base for the same reason.
//! * Food is deepened (starvation is not the subject), fauna and
//!   `Terrifying` entities are removed, and a janitor sweeps corpses
//!   and clears panic/coma each tick: a random panic-lock can
//!   incapacitate a workforce for hundreds of ticks (the corpse that
//!   causes it can only be buried from inside its own terror radius).
//!   Those systems have their own suites.
//! * Tools are zeroed so the founders' fetch-tools errand does not
//!   dominate early ticks; work proceeds at improvised efficiency.
//! * EVA suits are removed (pool zeroed, loose suits swept by the
//!   janitor): Spec 1384's FetchSuit scores 0.97 — above Work's 0.5
//!   plus the decider's 0.15 switch threshold — whenever ANY
//!   designation or anomaly anywhere sits in vacuum (a global gate),
//!   so in roughly half of generated worlds it preempts the inserted
//!   Work plan on the first evaluation tick. The pop then walks to a
//!   stockpile to dress instead of working, and if the single greedy
//!   movement step toward the stockpile is blocked it can wait there
//!   forever (item-92 forensics, 2026-10-16: 5/12 failures, every one
//!   with zero ForestryProgress ever created — the executor was
//!   running; the plan never survived to reach it). Suits have their
//!   own suites (spec 1384); here they are an ambient system owned by
//!   another spec, isolated like fauna and panic above. The same
//!   forensics found a rarer second preemptor (~1/15 runs): Repair
//!   (0.6 base, scaled by damage depth) beats the threshold when a
//!   weathered structure stands near the work site, so
//!   `run_until_consumed` also keeps structures topped up and sweeps
//!   Repair designations — repair is the sibling test's subject, not
//!   this one's. Colony clothing is zeroed for the same reason
//!   (FetchClothing's 0.95 nakedness band). And because the decider
//!   re-evaluates every tick, no inserted plan is self-sustaining in
//!   a live world: `run_until_consumed` re-pins the plan whenever a
//!   preemptor has stripped it (worker left with no MovementTarget).
//!   That restores the arrangement only — arrival, execution, and
//!   yield still happen entirely through the production schedule,
//!   and if the executor were missing the re-pinned pop would stand
//!   at the designation forever and the assertion would still fire.

use crate::layer1::administration::designation::{try_designate, Designation, DesignationType};
use crate::layer1::biology::health::Health;
use crate::layer1::culture::funeral::Corpse;
use crate::layer1::economy::ResourceItem;
use crate::layer1::entities::pop::PopBundle;
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::fauna::Fauna;
use crate::layer1::map::GridPosition;
use crate::layer1::mind::sleep_debt::ForcedComa;
use crate::layer1::physics::pressure::PressureGrid;
use crate::layer1::pop::Pop;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::psychology::panic_spirals::{Panic, Terrifying};
use crate::layer1::resources::ColonyResources;
use crate::layer1::structure::Structure;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::utility_types::{ActionType, PopAction};
use crate::setup::{setup_world_with_config, SetupConfig};
use crate::simulation::run_simulation_tick;
use bevy_ecs::prelude::*;

/// A live-schedule world with the environmental controls described in
/// the module docs.
fn test_world() -> World {
    let mut world = setup_world_with_config(SetupConfig {
        headless: true,
        ..Default::default()
    });
    {
        let mut res = world.resource_mut::<ColonyResources>();
        res.food = 500.0;
        res.tools = 0.0;
        res.clothing = 0.0;
        res.eva_suits = 0.0;
    }
    remove_ambient_threats(&mut world);
    world
}

/// Despawns loose (unworn) EVA suit entities: the FetchSuit fetch
/// targets. Worn suits are left alone — no pop in these tests starts
/// suited, and the workforce is replaced before the phases begin.
fn remove_loose_suits(world: &mut World) {
    let worn: Vec<Entity> = {
        let mut q = world.query::<&crate::layer1::items::Equipment>();
        q.iter(world).filter_map(|eq| eq.body).collect()
    };
    let loose: Vec<Entity> = {
        let mut q = world.query::<(Entity, &crate::layer1::eva_suit::EvaSuit)>();
        q.iter(world)
            .map(|(e, _)| e)
            .filter(|e| !worn.contains(e))
            .collect()
    };
    for e in loose {
        world.despawn(e);
    }
}

/// Despawns fauna and `Terrifying` entities (ambient terror sources
/// owned by other specs).
fn remove_ambient_threats(world: &mut World) {
    let fauna: Vec<Entity> = {
        let mut q = world.query::<(Entity, &Fauna)>();
        q.iter(world).map(|(e, _)| e).collect()
    };
    for e in fauna {
        world.despawn(e);
    }
    let terrifying: Vec<Entity> = {
        let mut q = world.query::<(Entity, &Terrifying)>();
        q.iter(world).map(|(e, _)| e).collect()
    };
    for e in terrifying {
        world.despawn(e);
    }
    remove_loose_suits(world);
}

/// Test-harness janitor: sweeps corpses and clears Panic / ForcedComa
/// so terror/coma dynamics (other specs' systems, with their own
/// suites) cannot incapacitate the workforce mid-test.
fn keep_workforce_functional(world: &mut World) {
    let corpses: Vec<Entity> = {
        let mut q = world.query::<(Entity, &Corpse)>();
        q.iter(world).map(|(e, _)| e).collect()
    };
    for e in corpses {
        world.despawn(e);
    }
    let pops: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, With<Pop>>();
        q.iter(world).collect()
    };
    for pop in pops {
        let mut e = world.entity_mut(pop);
        e.remove::<Panic>();
        e.remove::<ForcedComa>();
    }
    // Keep the EVA isolation (see module docs): no suit pool to draw
    // from and no loose suits to fetch, so FetchSuit can never score.
    world.resource_mut::<ColonyResources>().eva_suits = 0.0;
    remove_loose_suits(world);
    // Top up needs: with hunger/rest/hygiene full, the needs actions
    // sit at their utility floors, so the decider's choice is governed
    // by the work candidates' distance — which the arrangement (pops
    // spawned at the work site) controls. Without this, a fresh pop's
    // randomized starting rest makes SatisfyRest (base urgency
    // (1 - rest) + 0.5, never None) capture it for hundreds of ticks.
    let mut q = world.query::<&mut Needs>();
    for mut needs in q.iter_mut(world) {
        needs.hunger = 1.0;
        needs.rest = 1.0;
        needs.hygiene = 1.0;
        needs.leisure = 1.0;
    }
    // Keep workers alive: some worlds have no rock inside the
    // pressure envelope at all, so a miner may have to work a
    // low-pressure tile; without this the phase is decided by
    // suffocation geography instead of the work pipeline. (The
    // atmosphere system itself is covered by its own suites and by
    // the item-77 forensics work.)
    let mut q = world.query::<&mut Health>();
    for mut health in q.iter_mut(world) {
        health.current = health.max;
    }
}

/// Replaces the ambient colony with `count` fresh pops spawned from
/// the game's own founder bundle on walkable tiles nearest to
/// `site`. Spawning is deliberately not restricted to the pressure
/// envelope: in some worlds the only rock is at the map edge, tens of
/// tiles from the envelope, and a worker spawned at the envelope
/// never reaches it (long-range movement stalls). The janitor's
/// health top-up covers the low-pressure exposure instead; see the
/// module docs.
fn fresh_workforce(world: &mut World, count: usize, site: (i32, i32)) {
    let pops: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, With<Pop>>();
        q.iter(world).collect()
    };
    for e in pops {
        world.despawn(e);
    }
    let tiles = {
        let grid = world.resource::<TerrainGrid>();
        let mut tiles: Vec<(i32, i32, i32)> = Vec::new();
        for x in 0..grid.width as i32 {
            for y in 0..grid.height as i32 {
                let terrain = grid.get(x as usize, y as usize);
                if terrain.is_some_and(TerrainType::is_walkable) {
                    let d = (site.0 - x).abs() + (site.1 - y).abs();
                    tiles.push((d, x, y));
                }
            }
        }
        tiles.sort();
        tiles
            .into_iter()
            .take(count)
            .map(|(_, x, y)| (x, y))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        tiles.len(),
        count,
        "colony should have {count} walkable spawn tiles"
    );
    let mut rng = rand::thread_rng();
    for (x, y) in tiles {
        world.spawn(PopBundle::random(x, y, &mut rng));
    }
}

/// The `count` tiles of `terrain` nearest (Manhattan) to any pop,
/// nearest first, restricted to tiles inside the colony's pressure
/// envelope (pressure >= 0.3), with a highest-pressure fallback if no
/// tile qualifies. See the module docs for why the filter matters.
fn nearest_tiles_to_pops(world: &mut World, terrain: TerrainType, count: usize) -> Vec<(i32, i32)> {
    let positions: Vec<GridPosition> = {
        let mut q = world.query_filtered::<&GridPosition, With<Pop>>();
        q.iter(world).copied().collect()
    };
    let pressure = world.resource::<PressureGrid>();
    let grid = world.resource::<TerrainGrid>();
    let mut tiles: Vec<(i32, i32, i32)> = Vec::new();
    for x in 0..grid.width as i32 {
        for y in 0..grid.height as i32 {
            if grid.get(x as usize, y as usize) == Some(terrain) && pressure.get(x, y) >= 0.3 {
                let d = positions
                    .iter()
                    .map(|p| (p.x - x).abs() + (p.y - y).abs())
                    .min()
                    .unwrap_or(i32::MAX);
                tiles.push((d, x, y));
            }
        }
    }
    tiles.sort();
    let mut picked: Vec<(i32, i32)> = tiles
        .into_iter()
        .take(count)
        .map(|(_, x, y)| (x, y))
        .collect();
    if picked.is_empty() {
        let mut best: Option<(i32, i32)> = None;
        let mut best_p = f32::MIN;
        for x in 0..grid.width as i32 {
            for y in 0..grid.height as i32 {
                if grid.get(x as usize, y as usize) == Some(terrain) {
                    let p = pressure.get(x, y);
                    if p > best_p {
                        best_p = p;
                        best = Some((x, y));
                    }
                }
            }
        }
        if let Some(t) = best {
            picked.push(t);
        }
    }
    picked
}

/// The designation entity at a tile position, if one exists.
fn designation_at(world: &mut World, pos: GridPosition) -> Option<Entity> {
    let mut q = world.query::<(Entity, &GridPosition, &Designation)>();
    q.iter(world)
        .find(|(_, p, _)| **p == pos)
        .map(|(e, _, _)| e)
}

/// Runs the live schedule (janitor each tick) until `des` is consumed
/// or the tick budget runs out; consumption is asserted by the caller.
fn run_until_consumed(
    world: &mut World,
    worker: Entity,
    des: Entity,
    target: (i32, i32),
    max_ticks: usize,
) {
    for _ in 0..max_ticks {
        if world.get_entity(des).is_err() {
            return;
        }
        keep_workforce_functional(world);
        suppress_repair_candidates(world);
        // Re-pin the arranged plan if a preemptor stripped it (see
        // module docs): a worker with no MovementTarget at all has
        // lost the plan; a worker still travelling or working keeps
        // its current target untouched.
        if world.get_entity(worker).is_ok() && world.get::<MovementTarget>(worker).is_none() {
            assign_work_plan(world, worker, des, target);
        }
        run_simulation_tick(world);
    }
}

/// Keeps the Repair band empty during the designation phases: tops
/// every structure to full HP (so no repair candidates are offered)
/// and sweeps any Repair designations. Used only by
/// `run_until_consumed` — the sibling repair test drives repair
/// organically and must keep its damaged structures, so this is NOT
/// part of the shared janitor.
fn suppress_repair_candidates(world: &mut World) {
    let mut q = world.query::<&mut Structure>();
    for mut s in q.iter_mut(world) {
        s.current_hp = s.max_hp;
    }
    let repairs: Vec<Entity> = {
        let mut q = world.query::<(Entity, &Designation)>();
        q.iter(world)
            .filter(|(_, d)| d.designation_type == DesignationType::Repair)
            .map(|(e, _)| e)
            .collect()
    };
    for e in repairs {
        world.despawn(e);
    }
}

/// The player-visible bug, at the link that was actually broken.
///
/// The utility AI's half of the pipeline (buffer -> decider -> plan)
/// is exercised organically by the repair test below. This test pins
/// the EXECUTION link through the live schedule: a pop standing at a
/// Chop designation with the plan the decider produces (a Work
/// `MovementTarget`) must be marked arrived by the movement system,
/// processed by the arrival handler, and EXECUTED — progress accrues,
/// the designation is consumed, and yield appears. Then the same for
/// a Mine designation.
///
/// On unpatched trunk this fails deterministically at execution: the
/// pop arrives and stands at the designation forever, because
/// `work_execution_system` was never added to the schedule (only
/// referenced in `.before()`/`.after()` ordering constraints, which
/// Bevy silently drops for absent systems). Its direct-call unit
/// tests could not see the omission; this schedule-level test can.
///
/// Why the plan is inserted rather than left to the decider: in the
/// ambient colony, Work's base utility (0.5) sits inside a dense band
/// of competitors (rest at 0.5+, farming, hauling, leisure), so
/// organic engagement timing is a lottery even with a healthy
/// pipeline — measured over dozens of worlds during this fix's
/// development. What this regression must pin is the wiring, so the
/// decider's output is supplied as the arrangement and everything
/// downstream of it is the production schedule.
#[test]
fn test_designation_work_executes_in_live_schedule() {
    let mut world = test_world();
    for _ in 0..40 {
        run_simulation_tick(&mut world);
    }
    let start = {
        let res = world.resource::<ColonyResources>();
        (res.wood, res.stone)
    };

    // Phase 1: Chop.
    let tree = nearest_tiles_to_pops(&mut world, TerrainType::Tree, 1)[0];
    fresh_workforce(&mut world, 1, tree);
    let worker = {
        let mut q = world.query_filtered::<Entity, With<Pop>>();
        q.iter(&world).next().expect("one fresh worker")
    };
    assert!(try_designate(
        &mut world,
        tree.0,
        tree.1,
        DesignationType::Chop
    ));
    let chop_des = designation_at(
        &mut world,
        GridPosition {
            x: tree.0,
            y: tree.1,
        },
    )
    .expect("chop designation entity");
    assign_work_plan(&mut world, worker, chop_des, tree);
    run_until_consumed(&mut world, worker, chop_des, tree, 200);
    assert!(
        world.get_entity(chop_des).is_err(),
        "EXECUTION BREAK: a pop arrived at a designated tree with a Work \
         plan, but the designation was never consumed — the work executor \
         is not running in the live schedule"
    );

    // Phase 2: Mine.
    let rock = nearest_tiles_to_pops(&mut world, TerrainType::Rock, 1)[0];
    fresh_workforce(&mut world, 1, rock);
    let worker = {
        let mut q = world.query_filtered::<Entity, With<Pop>>();
        q.iter(&world).next().expect("one fresh worker")
    };
    assert!(try_designate(
        &mut world,
        rock.0,
        rock.1,
        DesignationType::Mine
    ));
    let mine_des = designation_at(
        &mut world,
        GridPosition {
            x: rock.0,
            y: rock.1,
        },
    )
    .expect("mine designation entity");
    assign_work_plan(&mut world, worker, mine_des, rock);
    run_until_consumed(&mut world, worker, mine_des, rock, 200);
    assert!(
        world.get_entity(mine_des).is_err(),
        "EXECUTION BREAK: a pop arrived at a designated rock with a Work \
         plan, but the designation was never consumed — the work executor \
         is not running in the live schedule"
    );

    // The work must yield resources: physical items on the ground
    // and/or stockpiled gains.
    let items = world.query::<&ResourceItem>().iter(&world).count();
    let res = world.resource::<ColonyResources>();
    assert!(
        items > 0 || res.wood > start.0 || res.stone > start.1,
        "YIELD BREAK: designations were consumed but no resource items or \
         stockpile increase resulted (items={items})"
    );
}

/// Gives `pop` the plan the utility AI produces for a Work verdict:
/// a `PopAction` recording the choice and a `MovementTarget` naming
/// the designation. Everything after this — movement, arrival,
/// execution — is the production schedule's own work.
fn assign_work_plan(world: &mut World, pop: Entity, des: Entity, target: (i32, i32)) {
    let mut e = world.entity_mut(pop);
    e.insert(PopAction {
        current: ActionType::Work,
        current_utility: 0.5,
        ticks_committed: 10,
    });
    e.insert(MovementTarget {
        target_entity: des,
        target_position: GridPosition {
            x: target.0,
            y: target.1,
        },
        for_action: ActionType::Work,
    });
    e.remove::<AtTarget>();
}

/// Repair rides the same executor, exercised fully organically:
/// damage every structure in a live colony and the utility AI must
/// select Repair, a pop must travel, arrive, and the executor must
/// restore HP through the real schedule. Nothing here calls the
/// repair pipeline directly, and no plan is inserted — repair's
/// utility (0.6 base, more for life support and deeper damage) wins
/// its band organically in a way designation Work's 0.5 does not,
/// which is why this test can stay organic while the designation
/// test above pins the executor with an inserted plan.
#[test]
fn test_structure_repair_completes_in_live_schedule() {
    let mut world = test_world();
    for _ in 0..40 {
        run_simulation_tick(&mut world);
    }

    // Damage ALL structures to 25% HP: a colony-wide repair
    // emergency. (Damaging a single structure proved unreliable —
    // a small structure can weather away before a repairer is
    // assigned, and a farm building's crew has competing Farm work.)
    let damaged: Vec<(Entity, f32)> = {
        let mut q = world.query::<(Entity, &Structure)>();
        q.iter(&world).map(|(e, s)| (e, s.max_hp)).collect()
    };
    assert!(
        !damaged.is_empty(),
        "starter colony should have structures to damage"
    );
    // Spawn the workforce beside the first damaged structure.
    let site = world
        .get::<GridPosition>(damaged[0].0)
        .map_or((0, 0), |p| (p.x, p.y));
    fresh_workforce(&mut world, 6, site);
    for _ in 0..5 {
        run_simulation_tick(&mut world);
    }
    for (entity, max_hp) in &damaged {
        if let Some(mut s) = world.get_mut::<Structure>(*entity) {
            s.current_hp = max_hp * 0.25;
        }
    }

    // A pop must pick up a repair, arrive, and the executor must
    // restore a structure. Weathering chips structures continuously,
    // so "restored" means some structure's HP was driven back to
    // ~full at some point, not that it ends exactly at max.
    let mut repairer_arrived = false;
    let mut any_restored = false;
    for _ in 0..400 {
        keep_workforce_functional(&mut world);
        run_simulation_tick(&mut world);
        if !repairer_arrived {
            let mut q = world.query_filtered::<&MovementTarget, With<AtTarget>>();
            repairer_arrived = q.iter(&world).any(|mt| mt.for_action == ActionType::Repair);
        }
        for (entity, max_hp) in &damaged {
            if let Some(s) = world.get::<Structure>(*entity) {
                if s.current_hp >= max_hp * 0.99 {
                    any_restored = true;
                }
            }
        }
        if repairer_arrived && any_restored {
            break;
        }
    }
    println!(
        "schedule repair pipeline: arrival={repairer_arrived}, any structure restored={any_restored}"
    );
    assert!(
        repairer_arrived,
        "SELECTION/MOVEMENT BREAK: in 400 live-schedule ticks no pop ever \
         arrived at a damaged structure with a Repair plan"
    );
    assert!(
        any_restored,
        "EXECUTION BREAK: a pop arrived to repair but no structure's HP \
         recovered to ~full — the work executor is not running in the \
         live schedule"
    );
}
