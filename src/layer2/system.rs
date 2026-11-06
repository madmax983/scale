//! Layer 2: System View.
//!
//! This module defines the core components and resources for the **Planetary System View** (Layer 2).
//! In this layer, the player manages fleets, orbital stations, and system-level resources.
//!
//! # Coordinate System
//!
//! The System View uses a **Polar Coordinate System** relative to the parent body.
//! *   **Star**: Fixed at (0, 0) in world space (conceptually).
//! *   **Planets**: Orbit the Star. Position is determined by `radius` and `angle`.
//! *   **Moons**: Orbit a Planet. Position is relative to the Planet.
//!
//! ## Rendering
//! The [`crate::layer2::system::OrbitalBody`] component provides visual data (char, color, size), while the [`crate::layer2::system::Orbit`] component
//! handles the mechanics of movement.

use bevy_ecs::prelude::*;
use ratatui::style::Color;

/// Marker component for a celestial body in the system.
///
/// This tag is used to query all "physical" objects in the system (Stars, Planets, Moons),
/// distinguishing them from abstract entities like Fleets or Stations.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SystemBody;

/// Aggregate stock of an asteroid belt (Spec 397: Nomadic Fleets).
///
/// The spec sketch's `AsteroidBelt { resources_remaining }` at
/// exactly this path: a belt-wide total that nomadic fleets
/// permanently deplete (see `layer3::nomadic_fleets`). Amount is
/// f32, the native unit of the real mining seams
/// (`MiningTarget::amount`, `NodeResources`) — when a belt entity
/// also carries a `MiningTarget`, the two are kept in sync by the
/// nomadic mining system, so this is an aggregate view of the
/// same stock, not a parallel one.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct AsteroidBelt {
    /// Total mineable stock remaining in the belt. Never refills:
    /// depletion is permanent.
    pub resources_remaining: f32,
}

impl AsteroidBelt {
    /// Whether the belt is fully depleted.
    #[must_use]
    pub fn is_depleted(&self) -> bool {
        self.resources_remaining <= 0.0
    }
}

/// Represents the visual properties of a celestial body.
///
/// Used by the rendering system to draw the entity on the system map.
#[derive(Component, Debug, Clone)]
pub struct OrbitalBody {
    /// The display name of the body (e.g., "Proxima Centauri", "Earth").
    pub name: String,
    /// The radius of the body (affects rendering size).
    /// *   Stars: ~5.0
    /// *   Gas Giants: ~3.0
    /// *   Rocky Planets: ~1.0
    /// *   Moons: ~0.3
    pub radius: f32,
    /// The color of the body.
    pub color: Color,
    /// The character used to represent the body (e.g., 'O', 'o', '☼').
    pub char: char,
}

impl Default for OrbitalBody {
    fn default() -> Self {
        Self {
            name: "Unknown Body".to_string(),
            radius: 1.0,
            color: Color::Gray,
            char: 'o',
        }
    }
}

/// Defines the orbit of a celestial body.
///
/// Entities with this component will move around their `parent` entity.
///
/// # Logic
///
/// *   **Position**: Calculated as `(radius * cos(angle), radius * sin(angle))` relative to parent.
/// *   **Movement**: Every tick, `angle` is incremented by `speed`.
///
/// # Examples
///
/// Creating a planet that orbits a star:
/// ```
/// use scale::layer2::system::Orbit;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// let star = world.spawn_empty().id();
///
/// let planet = world.spawn(Orbit {
///     parent: star,
///     radius: 10.0,
///     speed: 0.01,
///     angle: 0.0,
/// }).id();
/// ```
#[derive(Component, Debug, Clone)]
pub struct Orbit {
    /// The parent entity this body orbits around.
    pub parent: Entity,
    /// The radius of the orbit (distance from parent).
    pub radius: f32,
    /// The orbital speed (radians per tick).
    pub speed: f32,
    /// The current angle in the orbit (radians).
    pub angle: f32,
}

/// Gravity level of an orbital body or station.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GravityLevel {
    /// Normal planetary gravity
    #[default]
    Normal,
    /// Microgravity (e.g. moons, asteroids)
    MicroGravity,
    /// Zero-G environment (e.g. deep space stations)
    ZeroG,
}

/// Resource holding system-level map data.
///
/// Currently a placeholder for potential future features like fog of war,
/// discovered anomalies, or sector ownership maps.
#[derive(Resource, Default)]
pub struct SystemMap;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::keyboard::{GameKeyCode, GameKeyEvent};
    use crate::shared::state::GameState;
    use crate::shared::view_mode::ViewMode;
    use crate::ui::input::{InputContext, InputContextStack};
    // InputRouter might not be public or available directly. Let's check imports.
    // route_input is a function in crate::ui::input.
    use crate::ui::input::route_input;

    fn key_event(code: GameKeyCode) -> GameKeyEvent {
        GameKeyEvent::new(code)
    }

    #[test]
    fn test_view_mode_resource() {
        let mut world = World::new();
        world.insert_resource(ViewMode::default());

        assert_eq!(*world.resource::<ViewMode>(), ViewMode::Colony);
    }

    #[test]
    fn test_orbital_body_component() {
        let mut world = World::new();
        let entity = world
            .spawn(OrbitalBody {
                name: "Planet".to_string(),
                radius: 10.0,
                color: Color::Blue,
                char: 'O',
            })
            .id();

        let body = world.get::<OrbitalBody>(entity).unwrap();
        assert_eq!(body.name, "Planet");
        assert_eq!(body.char, 'O');
    }

    #[test]
    fn test_orbit_component() {
        let mut world = World::new();
        let sun = world.spawn_empty().id();
        let planet = world
            .spawn(Orbit {
                parent: sun,
                radius: 100.0,
                speed: 0.1,
                angle: 0.0,
            })
            .id();

        let orbit = world.get::<Orbit>(planet).unwrap();
        assert_eq!(orbit.parent, sun);
        assert!((orbit.radius - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_toggle_view_mode_input() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        world.insert_resource(ViewMode::Colony);
        world.insert_resource(crate::layer2::visibility::SystemVisibility::Full);

        // Setup Input Stack
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);

        // Press Tab to switch to System View
        // Note: route_input is the function to call.
        route_input(&mut world, key_event(GameKeyCode::Tab));

        assert_eq!(
            *world.resource::<ViewMode>(),
            ViewMode::System,
            "Tab should switch to System view"
        );

        // Press Tab to switch back to Colony View
        route_input(&mut world, key_event(GameKeyCode::Tab));
        assert_eq!(
            *world.resource::<ViewMode>(),
            ViewMode::Colony,
            "Tab should switch back to Colony view"
        );
    }
}
