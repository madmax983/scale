//! # Bureaucratic Drag: The Cost of Growth
//!
//! This module implements the "Bureaucratic Drag" mechanic (Spec 175), which simulates
//! the increasing inefficiency of a colony as it grows larger.
//!
//! ## The Mechanic
//!
//! **Efficiency = Supply / Demand**
//!
//! *   **Supply**: Administrative Points provided by [`Office`] buildings and [`JobType::Administrator`](crate::layer1::pop::JobType).
//! *   **Demand**: Administrative Points consumed by every Building and Pop.
//!
//! ## Impact
//!
//! When Efficiency drops below 1.0, it acts as a global multiplier for:
//! *   Work Speed
//! *   Construction Speed
//! *   Research Speed
//!
//! If you build too fast without expanding your administration, your entire colony slows down.
//!
//! ## How to Fix Low Efficiency
//!
//! 1.  Build more [`Office`] buildings.
//! 2.  Assign Pops to work as Administrators.
//! 3.  Reduce the number of non-essential buildings (reduce demand).
//!
//! ## The Efficiency Floor
//!
//! Efficiency never drops below [`MIN_ADMIN_EFFICIENCY`]. Every Pop and
//! Building is an [`AdminConsumer`] from the moment it exists, while the
//! first source of Supply (an [`Office`]) costs more Wood and Stone
//! than a new colony starts with — and gathering more requires work,
//! whose speed this very multiplier scales. Without a floor, a fresh
//! colony's efficiency computes to exactly 0.0 and every work amount
//! with it: a permanent deadlock before the first Office can ever be
//! afforded. The floor models informal coordination: a colony with no
//! bureaucracy is slow, never fully stopped. Drag above the floor is
//! unchanged — outgrowing your administration still hurts.

use bevy_ecs::prelude::*;

/// Tracks global administrative efficiency for the colony.
///
/// This resource is updated every tick by [`calculate_admin_stats`].
#[derive(Resource, Default, Debug)]
pub struct AdminStats {
    /// Total Admin points provided by offices and administrators.
    pub supply: f32,
    /// Total Admin points consumed by buildings and pops.
    pub demand: f32,
    /// Global efficiency multiplier ([`MIN_ADMIN_EFFICIENCY`] to 1.0).
    ///
    /// *   **1.0**: Perfect efficiency.
    /// *   **0.5**: Everything takes twice as long.
    pub efficiency: f32,
}

/// Minimum value of [`AdminStats::efficiency`] (see "The Efficiency
/// Floor" in the module docs). Bureaucratic drag can slow a colony to
/// a crawl, but never to a complete standstill.
pub const MIN_ADMIN_EFFICIENCY: f32 = 0.25;

/// Component for entities that provide Administrative Points (Supply).
///
/// Usually attached to [`Office`] buildings or Pops with administrative jobs.
#[derive(Component, Default, Debug)]
pub struct AdminProvider {
    /// Amount of Admin points provided.
    pub amount: f32,
}

/// Component for entities that consume Administrative Points (Demand).
///
/// Attached to almost every entity in the colony (Buildings, Pops).
#[derive(Component, Default, Debug)]
pub struct AdminConsumer {
    /// Amount of Admin points consumed.
    pub demand: f32,
}

/// Component defining an Office building.
///
/// Offices are the primary source of Admin Supply. They provide a base amount
/// plus additional points per working Administrator.
#[derive(Component, Debug, Clone)]
pub struct Office {
    /// Max number of administrators that can work here.
    pub capacity: usize,
    /// List of Pops currently working here.
    pub workers: Vec<Entity>,
}

impl Default for Office {
    fn default() -> Self {
        Self {
            capacity: 2,
            workers: Vec::new(),
        }
    }
}

/// Recalculates the global [`AdminStats`] based on current world state.
///
/// Sums up all `AdminProvider` and `AdminConsumer` components to determine
/// the efficiency ratio.
///
/// # Logic
///
/// `Efficiency = clamp(Supply / Demand, MIN_ADMIN_EFFICIENCY, 1.0)`
///
/// If `Demand` is 0, Efficiency is 1.0.
///
/// # Examples
///
/// ```
/// use scale::layer1::admin::{AdminStats, calculate_admin_stats, AdminProvider, AdminConsumer};
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// world.insert_resource(AdminStats::default());
///
/// // Supply: 10
/// world.spawn(AdminProvider { amount: 10.0 });
///
/// // Demand: 20
/// world.spawn(AdminConsumer { demand: 20.0 });
///
/// calculate_admin_stats(&mut world);
///
/// let stats = world.resource::<AdminStats>();
/// assert_eq!(stats.efficiency, 0.5); // 10 / 20
/// ```
pub fn calculate_admin_stats(world: &mut World) {
    let mut supply = 0.0;
    let mut demand = 0.0;

    // Sum Providers
    let mut providers = world.query::<&AdminProvider>();
    for provider in providers.iter(world) {
        supply += provider.amount;
    }

    // Sum Consumers
    let mut consumers = world.query::<&AdminConsumer>();
    for consumer in consumers.iter(world) {
        demand += consumer.demand;
    }

    // Default to 1.0 efficiency if no demand. Otherwise the ratio is
    // floored at MIN_ADMIN_EFFICIENCY so a colony with no admin supply
    // yet is slowed, not stopped (see module docs: "The Efficiency
    // Floor").
    let efficiency = if demand <= f32::EPSILON {
        1.0
    } else {
        (supply / demand).clamp(MIN_ADMIN_EFFICIENCY, 1.0)
    };

    world.insert_resource(AdminStats {
        supply,
        demand,
        efficiency,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::pop::{Job, JobType, Pop};

    #[test]
    fn test_admin_stats_calculation() {
        let mut world = World::new();
        world.insert_resource(AdminStats::default());

        // 1. Spawn Provider (Office with Administrator)
        // We simulate Office by manually adding AdminProvider since we haven't updated BuildingType yet
        let office = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing, // Placeholder until Office exists
                },
                AdminProvider { amount: 10.0 }, // Base provided by building
            ))
            .id();

        // Spawn Administrator working there
        world.spawn((
            Pop,
            Job {
                workplace: office,
                job_type: JobType::Administrator,
            },
            AdminProvider { amount: 5.0 }, // Provided by the job
        ));

        // 2. Spawn Consumer (Housing)
        world.spawn((
            Building {
                building_type: BuildingType::Housing,
            },
            AdminConsumer { demand: 2.0 },
        ));

        // 3. Spawn Consumer (Pop)
        world.spawn((Pop, AdminConsumer { demand: 1.0 }));

        // Run calculation
        calculate_admin_stats(&mut world);

        let stats = world.resource::<AdminStats>();

        // Supply: 10 (Building) + 5 (Job) = 15
        assert_eq!(stats.supply, 15.0);

        // Demand: 2 (Building) + 1 (Pop) = 3
        assert_eq!(stats.demand, 3.0);

        // Efficiency: 15 / 3 = 5.0 -> Clamped to 1.0
        assert_eq!(stats.efficiency, 1.0);
    }

    #[test]
    fn test_admin_efficiency_penalty() {
        let mut world = World::new();
        world.insert_resource(AdminStats::default());

        // Supply: 5
        world.spawn(AdminProvider { amount: 5.0 });

        // Demand: 10
        world.spawn(AdminConsumer { demand: 10.0 });

        calculate_admin_stats(&mut world);

        let stats = world.resource::<AdminStats>();

        // Efficiency: 5 / 10 = 0.5
        assert_eq!(stats.efficiency, 0.5);
    }

    #[test]
    fn test_zero_supply_efficiency_is_floored_not_zero() {
        // Regression: a brand-new colony has AdminConsumers (every Pop
        // and Building) but no Office yet, so Supply is 0. Efficiency
        // used to compute to exactly 0.0, which zeroed every work
        // amount in `calculate_work_amount` — a permanent deadlock,
        // since the first Office costs more Wood/Stone than a new
        // colony starts with and gathering more requires work.
        let mut world = World::new();
        world.insert_resource(AdminStats::default());

        // Demand with no supply at all (the fresh-colony state).
        world.spawn(AdminConsumer { demand: 20.0 });

        calculate_admin_stats(&mut world);

        let stats = world.resource::<AdminStats>();
        assert_eq!(stats.supply, 0.0);
        assert_eq!(stats.efficiency, MIN_ADMIN_EFFICIENCY);
    }

    #[test]
    fn test_deep_drag_is_floored() {
        // A colony that vastly outgrew its administration is dragged
        // to the floor, not below it.
        let mut world = World::new();
        world.insert_resource(AdminStats::default());

        world.spawn(AdminProvider { amount: 1.0 });
        world.spawn(AdminConsumer { demand: 100.0 });

        calculate_admin_stats(&mut world);

        // Raw ratio would be 0.01; the floor applies.
        assert_eq!(
            world.resource::<AdminStats>().efficiency,
            MIN_ADMIN_EFFICIENCY
        );
    }

    #[test]
    fn test_zero_demand_handling() {
        let mut world = World::new();
        world.insert_resource(AdminStats::default());

        // Supply: 5, Demand: 0
        world.spawn(AdminProvider { amount: 5.0 });

        calculate_admin_stats(&mut world);

        let stats = world.resource::<AdminStats>();
        assert_eq!(stats.efficiency, 1.0); // Should not be Infinity or NaN
    }
}
