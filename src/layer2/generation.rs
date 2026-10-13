use crate::layer1::quirks::{PlanetaryTrait, PlanetaryTraits};
use crate::layer2::binary_star::{BinaryPhase, BinaryStarSystem};
use crate::layer2::system::{Orbit, OrbitalBody};
use crate::shared::random::WorldSeed;
use bevy_ecs::prelude::*;
use rand::prelude::*;
use rand::rngs::StdRng;
use ratatui::style::Color;

/// Represents a Star in the system view.
#[derive(Component)]
pub struct Star;

/// Represents a Planet in the system view.
#[derive(Component)]
pub struct Planet;

/// Represents a Moon in the system view.
#[derive(Component)]
pub struct Moon;

/// Marks the planet where the colony is located.
#[derive(Component)]
pub struct ColonyLocation;

/// Stores the traits of a planet.
#[derive(Component)]
pub struct PlanetaryTraitsComponent {
    /// The list of traits this planet possesses.
    pub traits: Vec<PlanetaryTrait>,
}

/// The type of a planet, affecting its visual appearance and traits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanetType {
    Rocky,
    GasGiant,
    IceWorld,
}

impl PlanetType {
    const fn color(self) -> Color {
        match self {
            Self::Rocky => Color::Red,       // Mars-like
            Self::GasGiant => Color::Yellow, // Jupiter-like
            Self::IceWorld => Color::Cyan,   // Neptune-like
        }
    }

    const fn radius_range(self) -> std::ops::Range<f32> {
        match self {
            Self::Rocky => 0.8..1.5,
            Self::GasGiant => 2.0..4.0,
            Self::IceWorld => 1.2..2.0,
        }
    }
}

/// Generates the solar system based on the `WorldSeed`.
///
/// This system spawns:
/// - A central Star
/// - 3-8 Planets
/// - Moons orbiting planets
/// - Assigns one Rocky/Ice planet as the Colony Location
/// - Propagates traits from the colony planet to the global `PlanetaryTraits` resource.
#[allow(clippy::cast_precision_loss)]
fn generate_planet_traits(planet_type: PlanetType, rng: &mut StdRng) -> Vec<PlanetaryTrait> {
    let mut planet_traits = Vec::new();
    match planet_type {
        PlanetType::Rocky => {
            if rng.gen_bool(0.3) {
                planet_traits.push(PlanetaryTrait::HighGravity);
            }
            if rng.gen_bool(0.3) {
                planet_traits.push(PlanetaryTrait::ThinAtmosphere);
            }
            // Spec 345: rocky worlds can be volcanic or overgrown with
            // flora that bites back.
            if rng.gen_bool(0.15) {
                planet_traits.push(PlanetaryTrait::Volcanic);
            }
            if rng.gen_bool(0.2) {
                planet_traits.push(PlanetaryTrait::CarnivorousFlora);
            }
        }
        PlanetType::GasGiant => {
            planet_traits.push(PlanetaryTrait::HighGravity);
            planet_traits.push(PlanetaryTrait::DenseAtmosphere);
        }
        PlanetType::IceWorld => {
            if rng.gen_bool(0.3) {
                planet_traits.push(PlanetaryTrait::LowGravity);
            }
            planet_traits.push(PlanetaryTrait::SlowOrbit);
        }
    }

    if rng.gen_bool(0.1) {
        planet_traits.push(PlanetaryTrait::SilentWorld);
    }

    // Spec 345: any world can turn out tidally locked to its star.
    if rng.gen_bool(0.1) {
        planet_traits.push(PlanetaryTrait::TidallyLocked);
    }

    planet_traits
}

pub fn generate_system(
    mut commands: Commands,
    mut global_traits: ResMut<PlanetaryTraits>,
    seed: Res<WorldSeed>,
) {
    let mut rng = StdRng::seed_from_u64(seed.0);

    // 1. Create Star
    let star_entity = commands
        .spawn((
            Star,
            OrbitalBody {
                name: "Sun".to_string(),
                radius: 5.0,
                color: Color::Yellow,
                char: '☼',
            },
        ))
        .id();

    if rng.gen_bool(0.15) {
        commands.insert_resource(BinaryStarSystem {
            star1_intensity: 1.0,
            star2_intensity: rng.gen_range(0.5..1.0),
            current_phase: BinaryPhase::Normal,
            heat_wave_chance: 0.1,
        });
    }

    // 2. Create Planets
    let num_planets = rng.gen_range(3..=8);
    let mut planets = Vec::new();

    for i in 0..num_planets {
        #[allow(clippy::suboptimal_flops)] // Clippy false positive or irrelevant here
        let distance = (i as f32).mul_add(15.0, 20.0) + rng.gen_range(-5.0..5.0);

        // Determine Planet Type
        let planet_type = match rng.gen_range(0..100) {
            0..=50 => PlanetType::Rocky,
            51..=80 => PlanetType::GasGiant,
            _ => PlanetType::IceWorld,
        };

        let planet_traits = generate_planet_traits(planet_type, &mut rng);

        let planet = commands
            .spawn((
                Planet,
                OrbitalBody {
                    name: format!("Planet {}", i + 1),
                    radius: rng.gen_range(planet_type.radius_range()),
                    color: planet_type.color(),
                    char: 'O',
                },
                Orbit {
                    parent: star_entity,
                    radius: distance,
                    speed: 1.0 / distance.sqrt(), // Kepler-ish
                    angle: rng.gen_range(0.0..std::f32::consts::TAU),
                },
                PlanetaryTraitsComponent {
                    traits: planet_traits.clone(),
                },
            ))
            .id();

        planets.push((planet, planet_type, planet_traits));

        // 3. Create Moons
        if rng.gen_bool(0.4) {
            let num_moons = rng.gen_range(1..=3);
            for m in 0..num_moons {
                commands.spawn((
                    Moon,
                    OrbitalBody {
                        name: format!("Moon {}-{}", i + 1, m + 1),
                        radius: 0.3,
                        color: Color::Gray,
                        char: 'o',
                    },
                    Orbit {
                        parent: planet,
                        #[allow(clippy::suboptimal_flops)]
                        radius: (m as f32).mul_add(1.0, 2.5),
                        speed: 2.0, // Fast orbit
                        angle: rng.gen_range(0.0..std::f32::consts::TAU),
                    },
                ));
            }
        }
    }

    // 4. Select Colony Location
    // Filter out Gas Giants
    let candidates: Vec<_> = planets
        .iter()
        .filter(|(_, p_type, _)| *p_type != PlanetType::GasGiant)
        .collect();

    // Fallback to any planet if no candidates (unlikely but safe)
    let selection: Option<&(Entity, PlanetType, Vec<PlanetaryTrait>)> = if candidates.is_empty() {
        planets.choose(&mut rng)
    } else {
        candidates.choose(&mut rng).copied()
    };

    if let Some((colony_entity, _, traits)) = selection {
        commands.entity(*colony_entity).insert(ColonyLocation);

        // 5. Propagate Traits
        global_traits.0.clone_from(traits);
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::quirks::PlanetaryTraits;
    use crate::layer2::generation::{
        generate_system, ColonyLocation, Planet, PlanetaryTraitsComponent, Star,
    };
    use crate::layer2::system::{Orbit, OrbitalBody};
    use crate::shared::random::WorldSeed;
    use bevy_ecs::prelude::*;

    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(PlanetaryTraits::default());
        world.insert_resource(WorldSeed(42)); // Deterministic seed
                                              // Register required components if using reflection (optional for tests)
        world
    }

    #[test]
    fn test_generate_system_creates_star() {
        let mut world = setup_world();

        // Run generation system
        let mut schedule = Schedule::default();
        schedule.add_systems(generate_system);
        schedule.run(&mut world);

        // Assert exactly one Star exists
        let stars = world.query::<&Star>().iter(&world).count();
        assert_eq!(stars, 1, "Should generate exactly one star");

        // Assert Star has OrbitalBody (visuals) but NO Orbit (it's the center)
        let (star_entity, _) = world.query::<(Entity, &Star)>().single(&world);
        assert!(world.get::<OrbitalBody>(star_entity).is_some());
        assert!(world.get::<Orbit>(star_entity).is_none());
    }

    #[test]
    fn test_generate_system_creates_planets() {
        let mut world = setup_world();
        let mut schedule = Schedule::default();
        schedule.add_systems(generate_system);
        schedule.run(&mut world);

        // Assert at least one Planet exists
        let planet_count = world.query::<&Planet>().iter(&world).count();
        assert!(planet_count > 0, "Should generate at least one planet");

        // Assert Planets orbit the Star
        let (star_entity, _) = world.query::<(Entity, &Star)>().single(&world);
        for (planet_entity, orbit) in world.query::<(Entity, &Orbit)>().iter(&world) {
            // Check if this entity is a planet
            if world.get::<Planet>(planet_entity).is_some() {
                assert_eq!(orbit.parent, star_entity, "Planet should orbit the star");
            }
        }
    }

    #[test]
    fn test_colony_location_assignment() {
        let mut world = setup_world();
        let mut schedule = Schedule::default();
        schedule.add_systems(generate_system);
        schedule.run(&mut world);

        // Assert exactly one ColonyLocation exists
        let colony_locations = world.query::<&ColonyLocation>().iter(&world).count();
        assert_eq!(
            colony_locations, 1,
            "Should assign exactly one colony location"
        );

        // Assert ColonyLocation is on a Planet
        let (colony_entity, _) = world.query::<(Entity, &ColonyLocation)>().single(&world);
        assert!(
            world.get::<Planet>(colony_entity).is_some(),
            "Colony should be on a planet"
        );
    }

    #[test]
    fn test_planetary_traits_propagation() {
        let mut world = setup_world();

        // Mock: Ensure generation creates a planet with specific traits
        // This might require seeding or inspecting internal logic, but for black-box testing:
        // We verify that the Global Resource matches the Component on the Colony Planet.

        let mut schedule = Schedule::default();
        schedule.add_systems(generate_system);
        schedule.run(&mut world);

        let (colony_entity, _) = world.query::<(Entity, &ColonyLocation)>().single(&world);
        let component_traits = world
            .get::<PlanetaryTraitsComponent>(colony_entity)
            .expect("Colony planet should have traits component");

        let global_traits = world.resource::<PlanetaryTraits>();

        // Assert they match
        // Note: PlanetaryTraits struct in Spec 080 wraps a Vec.
        // We need to ensure PartialEq is derived or check contents manually.
        assert_eq!(
            component_traits.traits, global_traits.0,
            "Global traits should match starting planet traits"
        );
    }

    #[test]
    fn test_determinism() {
        let mut world1 = setup_world(); // Seed 42
        let mut schedule1 = Schedule::default();
        schedule1.add_systems(generate_system);
        schedule1.run(&mut world1);

        let mut world2 = setup_world(); // Seed 42
        let mut schedule2 = Schedule::default();
        schedule2.add_systems(generate_system);
        schedule2.run(&mut world2);

        // Compare generated planets count
        let planets1 = world1.query::<&Planet>().iter(&world1).count();
        let planets2 = world2.query::<&Planet>().iter(&world2).count();
        assert_eq!(
            planets1, planets2,
            "Determinism failed: Planet count mismatch"
        );

        // Compare first planet name
        let (p1, _) = world1
            .query::<(Entity, &OrbitalBody)>()
            .iter(&world1)
            .find(|(_, b)| b.name == "Planet 1")
            .unwrap();
        let (p2, _) = world2
            .query::<(Entity, &OrbitalBody)>()
            .iter(&world2)
            .find(|(_, b)| b.name == "Planet 1")
            .unwrap();

        // Check if traits match for Planet 1
        let t1 = world1.get::<PlanetaryTraitsComponent>(p1).unwrap();
        let t2 = world2.get::<PlanetaryTraitsComponent>(p2).unwrap();
        assert_eq!(
            t1.traits, t2.traits,
            "Determinism failed: Planet 1 traits mismatch"
        );
    }
}
