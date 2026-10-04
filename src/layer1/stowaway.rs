//! Stowaway system implementation.
//!
//! Handles stowaways hiding in buildings, stealing resources, and eventually being discovered.

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::map::GridPosition;
use crate::layer1::resources::ColonyResources;
use crate::layer1::social::old_guard::Arrival;
use crate::layer1::visitor::Visitor;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Component for a stowaway hiding in a building.
#[derive(Component)]
pub struct Stowaway {
    /// Stealth level (0.0 to 1.0). 1.0 = Fully Hidden, 0.0 = Revealed.
    pub stealth: f32,
    /// Hunger level. Increases over time.
    pub hunger: f32,
    /// Tick when the last theft occurred.
    pub last_theft_tick: u64,
}

impl Default for Stowaway {
    fn default() -> Self {
        Self {
            stealth: 1.0,
            hunger: 0.0,
            last_theft_tick: 0,
        }
    }
}

/// Component indicating a visitor might become a stowaway.
#[derive(Component)]
pub struct InfiltrationRisk {
    /// Probability per tick to infiltrate if near a suitable building.
    pub chance: f32,
}

/// System to handle visitors infiltrating buildings.
pub fn infiltration_system(
    mut commands: Commands,
    visitors: Query<(Entity, &GridPosition, &InfiltrationRisk), With<Visitor>>,
    buildings: Query<(Entity, &GridPosition, &Building), Without<Stowaway>>,
) {
    let mut rng = rand::thread_rng();

    for (visitor_entity, visitor_pos, risk) in &visitors {
        if rng.r#gen::<f32>() < risk.chance {
            // Find nearby suitable building (Stockpile)
            for (building_entity, building_pos, building) in &buildings {
                // Check if building is Stockpile and visitor is at same position
                if building.building_type == BuildingType::Stockpile && visitor_pos == building_pos
                {
                    // Infiltrate!
                    commands.entity(visitor_entity).despawn(); // Visitor "disappears"

                    // Add Stowaway component to building
                    commands.entity(building_entity).insert(Stowaway::default());

                    // Log event could go here
                    break;
                }
            }
        }
    }
}

/// System to handle stowaways stealing resources.
pub fn theft_system(
    mut resources: ResMut<ColonyResources>,
    time: Res<SimulationTime>,
    mut query: Query<&mut Stowaway>,
) {
    for mut stowaway in &mut query {
        // Steal if enough time passed and hungry
        // 100 ticks cooldown (approx 10% of a day)
        if time.tick > stowaway.last_theft_tick + 100 && stowaway.hunger > 10.0 {
            // Steal food
            // Cap at what's available or 5.0 units
            let stolen = 5.0f32.min(resources.food);
            resources.food -= stolen;

            // Reduce hunger
            stowaway.hunger -= stolen;
            stowaway.last_theft_tick = time.tick;

            // In a real implementation, we would add a notification here
        }

        // Passive hunger increase
        // 0.1 per tick is very fast if tick is every frame, but matches spec.
        // Needs::hunger decay is 0.005 per tick usually.
        // Spec said "stowaway.hunger += 0.1; // Passive hunger".
        // This means they get hungry very fast (100 ticks = 10 hunger).
        stowaway.hunger += 0.1;
    }
}

/// System to handle discovery of stowaways.
pub fn discovery_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Stowaway, &GridPosition)>,
    time: Res<SimulationTime>,
    // Optional: Log resource
    // mut log: Option<ResMut<MessageLog>>,
) {
    let mut rng = rand::thread_rng();

    for (entity, mut stowaway, pos) in &mut query {
        // Decrease stealth
        // 0.005 per tick -> revealed in 200 ticks (approx).
        // This gives enough time for hunger (0.1/tick) to reach > 10.0 (100 ticks) and steal.
        stowaway.stealth -= 0.005;

        if stowaway.stealth <= 0.0 {
            // Reveal!
            commands.entity(entity).remove::<Stowaway>();

            // GHOST-POP FIX (2026-10-04): the revealed stowaway used to
            // spawn as a hand-rolled partial bundle — no Wallet, no Morale,
            // no Traits, no ContagionCooldown, ... — half-invisible to the
            // sim. Spawn a full PopBundle with a real arrival tick instead.
            let mut bundle =
                crate::layer1::PopBundle::random(pos.x, pos.y, &mut rng);
            bundle.arrival = Arrival { tick: time.tick };
            commands.spawn(bundle);

            // Log event: "A stowaway was found hiding in the stockpile!"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::map::GridPosition;
    use crate::layer1::resources::ColonyResources;
    use crate::layer1::visitor::{Visitor, VisitorState};
    use crate::shared::time::SimulationTime;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_stowaway_component_defaults() {
        let stowaway = Stowaway::default();
        assert!((stowaway.stealth - 1.0).abs() < f32::EPSILON); // 100% hidden
        assert!(stowaway.hunger.abs() < f32::EPSILON);
    }

    #[test]
    fn test_infiltration_adds_stowaway_to_building() {
        let mut world = World::new();

        // Spawn Visitor near Building
        let visitor = world
            .spawn((
                Visitor {
                    state: VisitorState::Loitering,
                    ..Default::default()
                },
                GridPosition { x: 10, y: 10 },
                InfiltrationRisk { chance: 1.0 }, // Force infiltration
            ))
            .id();

        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Stockpile,
                },
                GridPosition { x: 10, y: 10 },
            ))
            .id();

        // Run system
        world.run_system_once(infiltration_system).unwrap();

        // Visitor should be despawned (or removed from world/transformed)
        assert!(world.get::<Visitor>(visitor).is_none());

        // Building should have Stowaway component
        assert!(world.get::<Stowaway>(building).is_some());
    }

    #[test]
    fn test_stowaway_steals_food() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            food: 100.0,
            ..Default::default()
        });
        world.insert_resource(SimulationTime {
            tick: 101,
            ..Default::default()
        });

        // Spawn Building with Stowaway
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stowaway {
                hunger: 50.0,
                ..Default::default()
            }, // Hungry
        ));

        world.run_system_once(theft_system).unwrap();

        let resources = world.resource::<ColonyResources>();
        assert!(resources.food < 100.0, "Food should be stolen");
    }

    #[test]
    fn test_discovery_removes_component_spawns_pop() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 500,
            ..Default::default()
        });

        // Spawn Building with Stowaway (low stealth)
        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Stockpile,
                },
                Stowaway {
                    stealth: 0.0,
                    ..Default::default()
                }, // Revealed
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        // Run system
        world.run_system_once(discovery_system).unwrap();

        // Stowaway component removed
        assert!(world.get::<Stowaway>(building).is_none());

        // New Pop spawned at location
        let pop_count = world
            .query::<&crate::layer1::pop::Pop>()
            .iter(&world)
            .count();
        assert_eq!(pop_count, 1);

        // GHOST-POP FIX (2026-10-04): the revealed stowaway must be a
        // complete colonist, not a partial-bundle ghost.
        assert_eq!(world.query::<&crate::layer1::pop::PopName>().iter(&world).count(), 1);
        assert_eq!(world.query::<&crate::layer1::health::Health>().iter(&world).count(), 1);
        assert_eq!(world.query::<&crate::layer1::economy::Wallet>().iter(&world).count(), 1);
        assert_eq!(world.query::<&crate::layer1::needs::Needs>().iter(&world).count(), 1);

        // Verify Arrival tick
        let arrival = world.query::<&Arrival>().single(&world);
        assert_eq!(arrival.tick, 500);
    }
}
