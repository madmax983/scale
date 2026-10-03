//! Spec 1379: The Sentient Commute — Desire Dust roads.
//!
//! Heavily-used path tiles accumulate "Desire Dust," which slightly speeds pop
//! movement along them. Past a threshold of use, an overused road coagulates
//! into a rudimentary immobile intelligence — a [`RoadMind`] — that emits
//! pheromones compelling pops to route through it even when better paths
//! exist. Deleting the road doesn't help: pops keep walking the ghost of the
//! route until the player excavates the dust tile by tile (see
//! [`DesignationType::ExcavateDust`](crate::layer1::administration::designation::DesignationType)).
//!
//! Organic efficiency vs. stubborn permanence.

use bevy_ecs::prelude::*;
use rand::Rng;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::execution::components::{HabituatedRoute, MovementTarget};
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, Speed};

/// Dust deposited per tick by a travelling pop (one with a `MovementTarget`).
pub const DUST_PER_TICK: f32 = 0.05;
/// Dust deposited per tick by a pop standing still (foot traffic, not travel).
pub const DUST_PER_TICK_IDLE: f32 = 0.01;
/// Speed bonus per unit of dust: `speed.current = base * (1 + bonus * dust)`.
pub const DUST_SPEED_BONUS: f32 = 0.15;
/// Hard cap on the dust speed bonus so highways can't break balance.
pub const DUST_SPEED_BONUS_MAX: f32 = 0.5;
/// Total dust in a connected region needed to coagulate a [`RoadMind`].
pub const SENTIENCE_THRESHOLD: f32 = 10.0;
/// Very slow per-tick decay — roads are remembered for a long time, but not forever.
pub const DUST_DECAY_PER_TICK: f32 = 0.0005;
/// Pops whose movement target lies within this many tiles of a road feel the pull.
pub const PHEROMONE_RADIUS: f32 = 8.0;
/// Per-tick probability that an in-range pop gets compelled onto the road.
pub const COMPEL_CHANCE_PER_TICK: f32 = 0.06;

/// Grid of accumulated Desire Dust, keyed by tile. Resource, not entities.
#[derive(Resource, Default, Debug)]
pub struct DesireDust {
    tiles: HashMap<(i32, i32), f32>,
}

impl DesireDust {
    /// Add `amount` of dust to a tile.
    pub fn deposit(&mut self, x: i32, y: i32, amount: f32) {
        *self.tiles.entry((x, y)).or_insert(0.0) += amount;
    }

    /// Dust currently on a tile (0.0 if none).
    #[must_use]
    pub fn amount_at(&self, x: i32, y: i32) -> f32 {
        self.tiles.get(&(x, y)).copied().unwrap_or(0.0)
    }

    /// Remove all dust from a tile (excavation).
    pub fn clear(&mut self, x: i32, y: i32) {
        self.tiles.remove(&(x, y));
    }

    /// Total dust across the whole map.
    #[must_use]
    pub fn total(&self) -> f32 {
        self.tiles.values().sum()
    }

    /// Number of tiles holding any dust.
    #[must_use]
    pub fn dusty_tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Iterate over all dusty tiles.
    pub fn iter(&self) -> impl Iterator<Item = (&(i32, i32), &f32)> {
        self.tiles.iter()
    }

    /// Apply the slow per-tick decay; abandoned ghost roads eventually fade.
    fn decay(&mut self) {
        for amount in self.tiles.values_mut() {
            *amount = (*amount - DUST_DECAY_PER_TICK).max(0.0);
        }
        self.tiles.retain(|_, amount| *amount > 0.0);
    }
}

/// A rudimentary immobile intelligence coagulated from an overused road.
///
/// Emits pheromones that compel nearby pops to route through its tiles.
/// Dissolves when all of its tiles are excavated.
#[derive(Component, Debug, Clone)]
pub struct RoadMind {
    /// The dusty tiles this mind is made of, in no particular order.
    pub tiles: Vec<(i32, i32)>,
}

/// Send a chronicle event if the resource exists (unit-test worlds may not have it).
fn chronicle(world: &mut World, text: &str, importance: EventImportance) {
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent {
            text: text.to_string(),
            importance,
        });
    }
}

/// Deposit Desire Dust under every pop's feet each tick.
///
/// Travelling pops (with a [`MovementTarget`]) leave a real trail;
/// stationary pops leave a faint scuff. Exclusive system: registered
/// directly as `fn(world: &mut World)`.
pub fn accumulate_desire_dust(world: &mut World) {
    let deposits: Vec<((i32, i32), f32)> = {
        let mut query = world.query_filtered::<(&GridPosition, Option<&MovementTarget>), With<Pop>>();
        query
            .iter(world)
            .map(|(pos, target)| {
                let amount = if target.is_some() {
                    DUST_PER_TICK
                } else {
                    DUST_PER_TICK_IDLE
                };
                ((pos.x, pos.y), amount)
            })
            .collect()
    };
    let mut dust = world.resource_mut::<DesireDust>();
    for ((x, y), amount) in deposits {
        dust.deposit(x, y, amount);
    }
}

/// Grant a capped speed bonus to pops standing on dusty tiles.
///
/// Runs after `reset_speed_system` so the bonus is not wiped each tick.
pub fn apply_desire_dust_speed(world: &mut World) {
    let targets: Vec<(Entity, f32)> = {
        // Snapshot first: query creation needs &mut world.
        let amounts: HashMap<(i32, i32), f32> = {
            let dust = world.resource::<DesireDust>();
            dust.iter().map(|(tile, amount)| (*tile, *amount)).collect()
        };
        let mut query = world.query_filtered::<(Entity, &GridPosition), With<Pop>>();
        query
            .iter(world)
            .map(|(entity, pos)| {
                (
                    entity,
                    amounts.get(&(pos.x, pos.y)).copied().unwrap_or(0.0),
                )
            })
            .filter(|(_, amount)| *amount > 0.0)
            .collect()
    };
    for (entity, amount) in targets {
        if let Some(mut speed) = world.get_mut::<Speed>(entity) {
            let bonus = (DUST_SPEED_BONUS * amount).min(DUST_SPEED_BONUS_MAX);
            speed.current = speed.base * (1.0 + bonus);
        }
    }
}

/// Slow per-tick decay of all dust.
pub fn decay_desire_dust(world: &mut World) {
    world.resource_mut::<DesireDust>().decay();
}

/// Flood-fill connected dusty regions; any region whose total dust reaches
/// [`SENTIENCE_THRESHOLD`] coagulates one [`RoadMind`] (unless already claimed).
pub fn check_road_sentience(world: &mut World) {
    // Tiles already claimed by existing minds are not re-scanned.
    let claimed: HashSet<(i32, i32)> = {
        let mut query = world.query::<&RoadMind>();
        query
            .iter(world)
            .flat_map(|mind| mind.tiles.iter().copied())
            .collect()
    };

    let dusty: HashMap<(i32, i32), f32> = {
        let dust = world.resource::<DesireDust>();
        dust.iter()
            .filter(|(tile, amount)| **amount > 0.0 && !claimed.contains(*tile))
            .map(|(tile, amount)| (*tile, *amount))
            .collect()
    };

    let mut visited: HashSet<(i32, i32)> = HashSet::new();
    let mut new_minds: Vec<Vec<(i32, i32)>> = Vec::new();

    for &start in dusty.keys() {
        if visited.contains(&start) {
            continue;
        }
        // Flood-fill this connected region (4-neighborhood).
        let mut region: Vec<(i32, i32)> = Vec::new();
        let mut queue: VecDeque<(i32, i32)> = VecDeque::from([start]);
        visited.insert(start);
        while let Some(tile) = queue.pop_front() {
            region.push(tile);
            for neighbor in [
                (tile.0 + 1, tile.1),
                (tile.0 - 1, tile.1),
                (tile.0, tile.1 + 1),
                (tile.0, tile.1 - 1),
            ] {
                if dusty.contains_key(&neighbor) && !visited.contains(&neighbor) {
                    visited.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        let total: f32 = region.iter().map(|tile| dusty[tile]).sum();
        if total >= SENTIENCE_THRESHOLD {
            new_minds.push(region);
        }
    }

    let first_ever = world.query::<&RoadMind>().iter(world).count() == 0;
    for (i, tiles) in new_minds.into_iter().enumerate() {
        world.spawn(RoadMind { tiles });
        if first_ever && i == 0 {
            chronicle(
                world,
                "The old north road has begun to dream.",
                EventImportance::Major,
            );
        }
    }
}

/// Excavate the dust from a single tile (the counterplay).
///
/// RoadMinds whose tiles are all excavated dissolve into silence.
pub fn excavate_dust_tile(world: &mut World, x: i32, y: i32) {
    world.resource_mut::<DesireDust>().clear(x, y);

    // Collect shrink/dissolve decisions first; mutation happens after the borrow ends.
    let mut dissolved: Vec<Entity> = Vec::new();
    let mut shrunk: Vec<(Entity, Vec<(i32, i32)>)> = Vec::new();
    {
        // Snapshot the amounts first: query creation needs &mut world.
        let amounts: HashMap<(i32, i32), f32> = {
            let dust = world.resource::<DesireDust>();
            dust.iter().map(|(tile, amount)| (*tile, *amount)).collect()
        };
        let mut query = world.query::<(Entity, &RoadMind)>();
        for (entity, mind) in query.iter(world) {
            let remaining: Vec<(i32, i32)> = mind
                .tiles
                .iter()
                .copied()
                .filter(|(tx, ty)| amounts.get(&(*tx, *ty)).copied().unwrap_or(0.0) > 0.0)
                .collect();
            if remaining.is_empty() {
                dissolved.push(entity);
            } else if remaining.len() != mind.tiles.len() {
                // Partial excavation shrinks the mind without killing it.
                shrunk.push((entity, remaining));
            }
        }
    }
    for (entity, tiles) in shrunk {
        world.entity_mut(entity).insert(RoadMind { tiles });
    }
    for entity in dissolved {
        world.despawn(entity);
        chronicle(
            world,
            "A road-mind has been unmade, tile by patient tile.",
            EventImportance::Minor,
        );
    }
}

/// A RoadMind's pheromones compel nearby pops to route through its tiles.
///
/// Implemented as a small per-tick probabilistic bias on top of the existing
/// movement-target machinery: compelled pops get a [`HabituatedRoute`] that
/// leads through the nearest road tile to their original destination, and the
/// existing phantom-commute systems steer them along it.
pub fn road_mind_pheromone_system(world: &mut World) {
    let minds: Vec<Vec<(i32, i32)>> = {
        let mut query = world.query::<&RoadMind>();
        query.iter(world).map(|mind| mind.tiles.clone()).collect()
    };
    if minds.is_empty() {
        return;
    }

    let mut rng = rand::thread_rng();
    let mut compelled: Vec<(Entity, HabituatedRoute)> = Vec::new();
    {
        let mut query = world.query_filtered::<(
            Entity,
            &GridPosition,
            &MovementTarget,
            Option<&HabituatedRoute>,
        ), With<Pop>>();
        for (entity, pos, target, route) in query.iter(world) {
            if route.is_some() {
                continue; // Already walking a route (ghost or compelled); don't stack.
            }
            let dest = (target.target_position.x, target.target_position.y);
            let near_road = minds.iter().flatten().any(|&(rx, ry)| {
                let dx = (target.target_position.x - rx) as f32;
                let dy = (target.target_position.y - ry) as f32;
                dx.hypot(dy) <= PHEROMONE_RADIUS
            });
            if !near_road || !rng.gen_bool(f64::from(COMPEL_CHANCE_PER_TICK)) {
                continue;
            }
            // Nearest road tile to the pop becomes the compelled waypoint.
            let nearest = minds
                .iter()
                .flatten()
                .min_by(|&&(ax, ay), &&(bx, by)| {
                    let da = ((pos.x - ax).pow(2) + (pos.y - ay).pow(2)) as f32;
                    let db = ((pos.x - bx).pow(2) + (pos.y - by).pow(2)) as f32;
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();
            if let Some((rx, ry)) = nearest {
                if (rx, ry) == (pos.x, pos.y) {
                    continue; // Already on the road; no need to reroute.
                }
                compelled.push((
                    entity,
                    HabituatedRoute {
                        path: vec![
                            GridPosition { x: rx, y: ry },
                            GridPosition {
                                x: dest.0,
                                y: dest.1,
                            },
                        ],
                        urgency: 1.0,
                        frustration: 0,
                        last_pos: None,
                    },
                ));
            }
        }
    }
    for (entity, route) in compelled {
        world.entity_mut(entity).insert(route);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::entities::pop::{Pop, Speed};
    use crate::layer1::map::GridPosition;

    #[test]
    fn test_foot_traffic_accumulates_desire_dust() {
        let mut world = World::new();
        world.insert_resource(DesireDust::default());
        let pop = world.spawn((Pop, GridPosition { x: 3, y: 3 })).id();

        // Simulate one tick of a moving pop standing on (3, 3).
        accumulate_desire_dust(&mut world);

        let dust = world.resource::<DesireDust>();
        assert!(
            dust.amount_at(3, 3) > 0.0,
            "foot traffic should deposit Desire Dust"
        );
        let _ = pop;
    }

    #[test]
    fn test_dusty_tiles_grant_speed_bonus() {
        let mut world = World::new();
        let mut dust = DesireDust::default();
        dust.deposit(5, 5, SENTIENCE_THRESHOLD); // well-worn road
        world.insert_resource(dust);
        let pop = world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            Speed {
                base: 1.0,
                current: 1.0,
                accumulator: 0.0,
            },
        ))
        .id();

        apply_desire_dust_speed(&mut world);

        let speed = world.get::<Speed>(pop).unwrap();
        assert!(
            speed.current > speed.base,
            "pops on dusty tiles should move faster"
        );
    }

    #[test]
    fn test_road_mind_coagulates_past_threshold() {
        let mut world = World::new();
        let mut dust = DesireDust::default();
        // Paint a connected stretch of heavy-use tiles.
        for x in 0..6 {
            dust.deposit(x, 0, SENTIENCE_THRESHOLD / 4.0);
        }
        world.insert_resource(dust);

        check_road_sentience(&mut world);

        let minds = world.query::<&RoadMind>().iter(&world).count();
        assert_eq!(minds, 1, "a saturated road should coagulate one RoadMind");
    }

    #[test]
    fn test_dream_chronicle_event_sent_on_first_mind() {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();
        let mut dust = DesireDust::default();
        for x in 0..6 {
            dust.deposit(x, 0, SENTIENCE_THRESHOLD / 4.0);
        }
        world.insert_resource(dust);

        check_road_sentience(&mut world);

        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut cursor = events.get_cursor();
        let texts: Vec<String> = cursor.read(events).map(|e| e.text.clone()).collect();
        assert!(
            texts.iter().any(|t| t.contains("dream")),
            "expected a dream chronicle event, got {texts:?}"
        );
    }

    #[test]
    fn test_excavation_clears_dust_and_kills_road_mind() {
        let mut world = World::new();
        let mut dust = DesireDust::default();
        dust.deposit(2, 2, SENTIENCE_THRESHOLD);
        world.insert_resource(dust);
        let mind = world.spawn((RoadMind { tiles: vec![(2, 2)] },)).id();

        excavate_dust_tile(&mut world, 2, 2);

        assert_eq!(world.resource::<DesireDust>().amount_at(2, 2), 0.0);
        assert!(
            world.get_entity(mind).is_err(),
            "a RoadMind with no dusty tiles should dissolve"
        );
    }
}
