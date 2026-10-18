use crate::layer1::entities::blob::{Blob, BlobNode};
use crate::layer1::entities::drone::Drone;
use crate::layer1::entities::mascot::Mascot;
use crate::layer1::entities::the_visitor::TheVisitor;
use crate::layer1::entities::vermin::Vermin;
use crate::layer1::entities::visitor::Visitor;
use crate::layer1::funeral::{Corpse, Grave};
use crate::layer1::nature::symbiotic_pests::Vermin as PestVermin;
use crate::layer1::{
    building::Building, fertility::FertilityGrid, health::Health, needs::Needs, pop::Pop,
    GridPosition, TerrainGrid, Viewport,
};
use bevy_ecs::prelude::*;
use std::fmt::Write;

use crate::shared::keyboard::{GameKeyCode, GameKeyEvent, GameMouseEvent};

/// What is currently selected by the player.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelectionTarget {
    /// Nothing selected.
    #[default]
    None,
    /// A tile at grid position (x, y).
    Tile(i32, i32),
    /// An entity (pop, building, etc.).
    Entity(Entity),
}

/// Tracks the current player selection.
#[derive(Resource, Default, Debug)]
pub struct Selection {
    target: SelectionTarget,
}

impl Selection {
    /// Create a new selection resource.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            target: SelectionTarget::None,
        }
    }

    /// Get the current selection target.
    #[must_use]
    pub const fn target(&self) -> SelectionTarget {
        self.target
    }

    /// Select a tile at grid coordinates.
    pub const fn select_tile(&mut self, x: i32, y: i32) {
        self.target = SelectionTarget::Tile(x, y);
    }

    /// Select an entity.
    pub const fn select_entity(&mut self, entity: Entity) {
        self.target = SelectionTarget::Entity(entity);
    }

    /// Clear the selection.
    pub const fn clear(&mut self) {
        self.target = SelectionTarget::None;
    }

    /// Check if anything is selected.
    #[must_use]
    pub const fn is_selected(&self) -> bool {
        !matches!(self.target, SelectionTarget::None)
    }
}

/// Generate info panel text for a selected tile.
#[must_use]
#[allow(clippy::cast_sign_loss)] // Coordinate checking handles negative values
pub fn inspect_tile(world: &World, x: i32, y: i32) -> String {
    let terrain = world.resource::<TerrainGrid>();

    if x < 0 || y < 0 {
        return String::from("Empty space\n(outside map)");
    }

    terrain.get(x as usize, y as usize).map_or_else(
        || String::from("Empty space\n(outside map)"),
        |tile| {
            let mut info = format!(
                "Tile ({x}, {y})\n\nTerrain: {}\n\n(Click entity for details)",
                tile.name()
            );

            if let Some(fertility) = world.get_resource::<FertilityGrid>() {
                let val = fertility.get(x as usize, y as usize);
                if val > 0.0 {
                    let _ = write!(info, "\nFertility: {:.0}%", val * 100.0);
                }
            }

            info
        },
    )
}

/// Generate info panel text for a selected entity.
#[must_use]
pub fn inspect_entity(world: &World, entity: Entity) -> String {
    // Check if entity exists
    if !world.entities().contains(entity) {
        return String::from("Entity not found");
    }

    let entity_type = if world.get::<Pop>(entity).is_some() {
        "Pop"
    } else if let Some(b) = world.get::<Building>(entity) {
        b.building_type.label()
    } else if world.get::<Drone>(entity).is_some() {
        "Drone"
    } else if world.get::<Corpse>(entity).is_some() {
        "Corpse"
    } else {
        "Entity"
    };

    // Try to get Position
    if let Some(pos) = world.get::<GridPosition>(entity) {
        let mut info = format!("{entity_type}\nPosition: ({}, {})\n\n", pos.x, pos.y);

        if let Some(health) = world.get::<Health>(entity) {
            let _ = writeln!(info, "Health: {:.0}%", health.current);
        }

        if let Some(needs) = world.get::<Needs>(entity) {
            let _ = write!(
                info,
                "Hunger: {:.0}%\nRest: {:.0}%\n",
                needs.hunger * 100.0,
                needs.rest * 100.0
            );
        }

        return info;
    }

    String::from("Unknown entity")
}

/// Convert screen coordinates to world coordinates.
#[must_use]
pub const fn screen_to_world(screen_x: u16, screen_y: u16, viewport: &Viewport) -> (i32, i32) {
    let world_x = viewport.x.wrapping_add(screen_x as i32);
    let world_y = viewport.y.wrapping_add(screen_y as i32);
    (world_x, world_y)
}

/// Handle selection input in normal mode.
pub fn handle_selection_input(world: &mut World, key: GameKeyEvent) {
    if key.code == GameKeyCode::Esc {
        // Clear selection
        world.resource_mut::<Selection>().clear();
    }
}

/// Whether an entity is worth inspecting in its own right when its
/// tile is clicked: drones, corpses, mascots, visitors, and other
/// notable presences. Anything positioned that is neither this, a
/// Pop, nor a Building is a pure tile overlay (e.g. a `GeomeHazard`
/// spawned on the tile it endangers) and must never hijack the
/// click from the tile beneath it — clicking magma should inspect
/// the magma tile, not a nameless hazard entity.
fn is_click_notable(world: &World, entity: Entity) -> bool {
    world.get::<Drone>(entity).is_some()
        || world.get::<Corpse>(entity).is_some()
        || world.get::<Grave>(entity).is_some()
        || world.get::<Mascot>(entity).is_some()
        || world.get::<Visitor>(entity).is_some()
        || world.get::<TheVisitor>(entity).is_some()
        || world.get::<Blob>(entity).is_some()
        || world.get::<BlobNode>(entity).is_some()
        || world.get::<Vermin>(entity).is_some()
        || world.get::<PestVermin>(entity).is_some()
}

/// Handle mouse click for selection.
///
/// A left click selects (Pop > Building > notable > tile). A right
/// click is the secondary verb: it clears the current selection and
/// never selects the thing under the cursor — before the button was
/// threaded through, a right click silently selected like a left one.
pub fn handle_selection_click(world: &mut World, mouse: GameMouseEvent, viewport: &Viewport) {
    if mouse.is_right_click() {
        world.resource_mut::<Selection>().clear();
        return;
    }

    let (world_x, world_y) = screen_to_world(mouse.x, mouse.y, viewport);

    // Find all entities at position
    // ⚡ Bolt Optimization: Removed intermediate `.collect::<Vec<_>>()` allocation
    // and multiple loops, doing a single pass to track the best candidate in each category.
    let mut selected_pop = None;
    let mut selected_building = None;
    let mut selected_notable = None;

    let mut query = world.query::<(Entity, &GridPosition)>();
    for (entity, pos) in query.iter(world) {
        if pos.x == world_x && pos.y == world_y {
            if selected_pop.is_none() && world.get::<Pop>(entity).is_some() {
                selected_pop = Some(entity);
            } else if selected_building.is_none() && world.get::<Building>(entity).is_some() {
                selected_building = Some(entity);
            } else if selected_notable.is_none() && is_click_notable(world, entity) {
                selected_notable = Some(entity);
            }
        }
    }

    // Prioritize: Pop > Building > notable entity > the tile itself.
    // Pure overlays (hazards and friends) fall through to the tile.
    let selected_entity = selected_pop.or(selected_building).or(selected_notable);

    // Update selection
    let mut selection = world.resource_mut::<Selection>();
    if let Some(entity) = selected_entity {
        selection.select_entity(entity);
    } else {
        selection.select_tile(world_x, world_y);
    }
}

#[cfg(feature = "nova")]
use crate::layer1::observer::Observed;
#[cfg(feature = "nova")]
#[cfg(feature = "nova")]
/// System that adds/removes the `Observed` component based on player selection.
pub fn observer_awareness_system(
    mut commands: Commands,
    selection: Res<Selection>,
    observed_query: Query<Entity, With<Observed>>,
    pop_query: Query<&Pop>,
) {
    let target_entity = match selection.target() {
        SelectionTarget::Entity(e) => Some(e),
        _ => None,
    };

    // 1. Remove Observed from entities that are no longer selected
    for entity in &observed_query {
        if Some(entity) != target_entity {
            commands.entity(entity).remove::<Observed>();
        }
    }

    // 2. Add Observed to the selected entity if it's a Pop and doesn't have it
    if let Some(e) = target_entity {
        if pop_query.get(e).is_ok() && !observed_query.contains(e) {
            commands.entity(e).insert(Observed::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::{GridPosition, Needs, Pop, TerrainGrid, TerrainType};

    #[test]
    fn test_selection_default_none() {
        let selection = Selection::default();
        assert_eq!(selection.target(), SelectionTarget::None);
    }

    #[test]
    fn test_selection_tile() {
        let mut selection = Selection::default();
        selection.select_tile(10, 5);

        assert_eq!(selection.target(), SelectionTarget::Tile(10, 5));
    }

    #[test]
    fn test_selection_entity() {
        let mut world = World::new();
        let entity = world.spawn_empty().id();

        let mut selection = Selection::default();
        selection.select_entity(entity);

        assert_eq!(selection.target(), SelectionTarget::Entity(entity));
    }

    #[test]
    fn test_selection_clear() {
        let mut world = World::new();
        let entity = world.spawn_empty().id();

        let mut selection = Selection::default();
        selection.select_entity(entity);
        assert!(selection.is_selected());

        selection.clear();
        assert!(!selection.is_selected());
        assert_eq!(selection.target(), SelectionTarget::None);
    }

    #[test]
    fn test_selection_is_selected() {
        let mut selection = Selection::default();
        assert!(!selection.is_selected());

        selection.select_tile(0, 0);
        assert!(selection.is_selected());

        selection.clear();
        assert!(!selection.is_selected());
    }

    #[test]
    fn test_selection_get_tile() {
        let mut selection = Selection::default();
        selection.select_tile(10, 20);

        if let SelectionTarget::Tile(x, y) = selection.target() {
            assert_eq!(x, 10);
            assert_eq!(y, 20);
        } else {
            panic!("Expected tile selection");
        }
    }

    #[test]
    fn test_selection_get_entity() {
        let mut world = World::new();
        let entity = world.spawn_empty().id();

        let mut selection = Selection::default();
        selection.select_entity(entity);

        if let SelectionTarget::Entity(e) = selection.target() {
            assert_eq!(e, entity);
        } else {
            panic!("Expected entity selection");
        }
    }

    #[test]
    fn test_selection_overwrite() {
        let mut selection = Selection::default();
        selection.select_tile(5, 5);
        assert!(selection.is_selected());

        // Selecting new target should overwrite
        selection.select_tile(10, 10);
        assert_eq!(selection.target(), SelectionTarget::Tile(10, 10));
    }

    #[test]
    fn test_inspect_tile_with_terrain() {
        let mut world = World::new();
        let terrain = TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        };
        world.insert_resource(terrain);

        let info = inspect_tile(&world, 5, 5);

        assert!(info.contains("Grass"));
        assert!(info.contains('5'));
    }

    #[test]
    fn test_inspect_tile_out_of_bounds() {
        let mut world = World::new();
        let terrain = TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        };
        world.insert_resource(terrain);

        let info = inspect_tile(&world, 100, 100);

        assert!(info.contains("Empty") || info.contains("Nothing"));
    }

    #[test]
    fn test_inspect_entity_pop() {
        let mut world = World::new();
        let entity = world
            .spawn((
                Pop,
                GridPosition { x: 10, y: 5 },
                Needs {
                    hunger: 0.75,
                    rest: 0.50,
                    ..Default::default()
                },
            ))
            .id();

        let info = inspect_entity(&world, entity);

        assert!(info.contains("Pop"));
        assert!(info.contains("10"));
        assert!(info.contains("hunger") || info.contains("Hunger"));
    }

    #[test]
    fn test_inspect_entity_nonexistent() {
        let world = World::new();
        let fake_entity = Entity::from_raw(999_999);

        let info = inspect_entity(&world, fake_entity);

        assert!(info.contains("not found") || info.contains("None"));
    }

    #[test]
    fn test_selection_target_is_copy() {
        let target1 = SelectionTarget::Tile(5, 5);
        let target2 = target1; // Should copy
        assert_eq!(target1, target2);
    }

    #[test]
    fn test_click_to_screen_pos() {
        let viewport = Viewport { x: 10, y: 5 };
        let screen_x = 3;
        let screen_y = 2;

        let (world_x, world_y) = screen_to_world(screen_x, screen_y, &viewport);

        assert_eq!(world_x, 13); // 10 + 3
        assert_eq!(world_y, 7); // 5 + 2
    }

    #[test]
    fn test_screen_to_world_overflow() {
        // Test potential overflow safety
        let viewport = Viewport {
            x: i32::MAX,
            y: i32::MAX,
        };
        let screen_x = 10;
        let screen_y = 10;

        // Should not panic
        let (world_x, world_y) = screen_to_world(screen_x, screen_y, &viewport);

        // Should wrap around
        assert_eq!(world_x, i32::MAX.wrapping_add(10));
        assert_eq!(world_y, i32::MAX.wrapping_add(10));
    }

    #[test]
    #[cfg(feature = "nova")]
    fn test_observer_awareness_adds_component() {
        use crate::layer1::observer::Observed;
        use crate::ui::selection::observer_awareness_system;
        let mut world = World::new();
        world.insert_resource(Selection::default());

        let pop = world.spawn(Pop).id();

        // Select the pop
        world.resource_mut::<Selection>().select_entity(pop);

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(observer_awareness_system);
        schedule.run(&mut world);

        assert!(world.get::<Observed>(pop).is_some());
    }

    #[test]
    #[cfg(feature = "nova")]
    fn test_observer_awareness_removes_component() {
        use crate::layer1::observer::Observed;
        use crate::ui::selection::observer_awareness_system;
        let mut world = World::new();
        let pop = world.spawn((Pop, Observed::default())).id();

        // Selection is None (default)
        world.insert_resource(Selection::default());

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(observer_awareness_system);
        schedule.run(&mut world);

        assert!(world.get::<Observed>(pop).is_none());
    }

    #[test]
    fn test_click_on_hazard_tile_selects_tile_not_hazard_entity() {
        use crate::layer1::environment::geomes::GeomeHazard;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        // A magma-style hazard entity shares the tile it endangers; it
        // is an overlay on the tile, not a thing to inspect.
        world.spawn((
            GeomeHazard {
                damage_per_tick: 10.0,
            },
            GridPosition { x: 5, y: 5 },
        ));

        let viewport = Viewport::default();
        handle_selection_click(&mut world, GameMouseEvent::new(5, 5), &viewport);

        assert_eq!(
            world.resource::<Selection>().target(),
            SelectionTarget::Tile(5, 5),
            "clicking a hazard's tile should select the tile, not the hazard entity"
        );
    }

    #[test]
    fn test_click_on_drone_selects_the_drone() {
        use crate::layer1::entities::drone::Drone;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let drone = world
            .spawn((Drone::default(), GridPosition { x: 2, y: 2 }))
            .id();

        let viewport = Viewport::default();
        handle_selection_click(&mut world, GameMouseEvent::new(2, 2), &viewport);

        assert_eq!(
            world.resource::<Selection>().target(),
            SelectionTarget::Entity(drone),
            "drones are notable entities and stay click-selectable"
        );
    }

    #[test]
    fn test_click_on_corpse_selects_the_corpse() {
        use crate::layer1::funeral::Corpse;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let corpse = world
            .spawn((
                Corpse {
                    name: "Ada".to_string(),
                    decay: 0.1,
                },
                GridPosition { x: 4, y: 1 },
            ))
            .id();

        let viewport = Viewport::default();
        handle_selection_click(&mut world, GameMouseEvent::new(4, 1), &viewport);

        assert_eq!(
            world.resource::<Selection>().target(),
            SelectionTarget::Entity(corpse),
            "corpses are notable entities and stay click-selectable"
        );
    }

    #[test]
    fn test_click_on_standalone_grave_selects_the_grave() {
        let mut world = World::new();
        world.insert_resource(Selection::default());
        // Death-site graves are standalone entities (no Building):
        // the grave component rides on the tile where the pop died.
        let grave = world
            .spawn((
                Grave {
                    occupied: true,
                    corpse_name: Some("Ada".to_string()),
                },
                GridPosition { x: 6, y: 2 },
            ))
            .id();

        let viewport = Viewport::default();
        handle_selection_click(&mut world, GameMouseEvent::new(6, 2), &viewport);

        assert_eq!(
            world.resource::<Selection>().target(),
            SelectionTarget::Entity(grave),
            "standalone graves are notable entities and stay click-selectable"
        );
    }

    #[test]
    fn test_click_on_pest_vermin_selects_the_vermin() {
        let mut world = World::new();
        world.insert_resource(Selection::default());
        let rat = world
            .spawn((PestVermin::grain_rat(), GridPosition { x: 7, y: 7 }))
            .id();

        let viewport = Viewport::default();
        handle_selection_click(&mut world, GameMouseEvent::new(7, 7), &viewport);

        assert_eq!(
            world.resource::<Selection>().target(),
            SelectionTarget::Entity(rat),
            "pest vermin are notable entities and stay click-selectable"
        );
    }

    #[test]
    fn test_right_click_clears_selection_and_does_not_select() {
        use crate::shared::keyboard::GameMouseButton;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let pop = world.spawn((Pop, GridPosition { x: 5, y: 5 })).id();
        world.resource_mut::<Selection>().select_tile(1, 1);

        let viewport = Viewport::default();
        // Right-click directly on a pop: it must clear, never select it.
        handle_selection_click(
            &mut world,
            GameMouseEvent::new(5, 5).with_button(GameMouseButton::Right),
            &viewport,
        );

        assert_eq!(
            world.resource::<Selection>().target(),
            SelectionTarget::None,
            "right-click is the clear-selection verb; it must not select the pop under it"
        );
        assert_ne!(
            world.resource::<Selection>().target(),
            SelectionTarget::Entity(pop)
        );
    }

    #[test]
    fn test_right_click_with_nothing_selected_stays_unselected() {
        use crate::shared::keyboard::GameMouseButton;

        let mut world = World::new();
        world.insert_resource(Selection::default());

        let viewport = Viewport::default();
        handle_selection_click(
            &mut world,
            GameMouseEvent::new(5, 5).with_button(GameMouseButton::Right),
            &viewport,
        );

        assert_eq!(
            world.resource::<Selection>().target(),
            SelectionTarget::None,
            "right-click on empty ground must not fall through to tile selection"
        );
    }

    #[test]
    fn test_left_click_still_selects_after_right_click_support() {
        let mut world = World::new();
        world.insert_resource(Selection::default());

        let viewport = Viewport::default();
        handle_selection_click(&mut world, GameMouseEvent::new(5, 5), &viewport);

        assert_eq!(
            world.resource::<Selection>().target(),
            SelectionTarget::Tile(5, 5),
            "left-click behaviour is unchanged: it still selects the tile"
        );
    }

    #[test]
    fn test_click_prefers_actor_over_notable_over_tile() {
        use crate::layer1::entities::drone::Drone;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let drone = world
            .spawn((Drone::default(), GridPosition { x: 3, y: 3 }))
            .id();
        let pop = world.spawn((Pop, GridPosition { x: 3, y: 3 })).id();

        let viewport = Viewport::default();
        handle_selection_click(&mut world, GameMouseEvent::new(3, 3), &viewport);

        assert_eq!(
            world.resource::<Selection>().target(),
            SelectionTarget::Entity(pop),
            "a pop sharing a tile with a drone should win the click"
        );
        assert_ne!(
            world.resource::<Selection>().target(),
            SelectionTarget::Entity(drone)
        );
    }
}
