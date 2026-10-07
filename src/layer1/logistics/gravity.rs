//! Gravity-Fed Logistics (Spec 1372).
//!
//! Liquids and items move automatically *down* Z-levels without power.
//! Pumping them *up* (or across a level) requires energy. Build your
//! reservoir at the top of the mountain — and pray nobody blows the dam,
//! because everything downhill gets washed away for free.
//!
//! Rules:
//! - Downhill transfer (source ZLevel > target ZLevel): always free.
//! - Level or uphill transfer: requires an active [`PowerConsumer`] on the
//!   source node (the pump). Missing [`PowerConsumer`] counts as unpowered.
//! - One item per source node per tick.

use crate::layer1::core::map::ZLevel;
use crate::layer1::energy::PowerConsumer;
use bevy_ecs::prelude::*;

/// A node in a gravity-fed chute network: holds items awaiting transport.
#[derive(Component, Debug, Clone, Default)]
pub struct GravityChute {
    /// Items currently held at this node.
    pub inventory: i32,
}

/// Directed link from one chute node to another.
#[derive(Component, Debug, Clone)]
pub struct ChuteLink {
    /// The node items flow toward.
    pub target: Entity,
}

/// Query item for a chute-network participant: the chute itself plus the
/// optional link, Z-level, and power state that govern its flow.
type ChuteQueryItem<'w> = (
    Entity,
    &'w mut GravityChute,
    Option<&'w ChuteLink>,
    Option<&'w ZLevel>,
    Option<&'w PowerConsumer>,
);

/// Moves one item per tick along each chute link, obeying gravity.
///
/// Downhill flow is free; level/uphill flow needs the source node's
/// [`PowerConsumer`] to be active. Two-pass within a single query (read-only
/// scan, then `get_many_mut` application) so source and target chutes can
/// both be mutated without conflicting borrows.
pub fn process_gravity_logistics_system(mut query: Query<ChuteQueryItem>) {
    // Phase 1: read-only scan — decide which links fire this tick.
    let mut transfers: Vec<(Entity, Entity)> = Vec::new();
    for (source_entity, source_chute, link, source_z, power) in query.iter() {
        let Some(link) = link else { continue };
        if source_chute.inventory <= 0 {
            continue;
        }
        if source_entity == link.target {
            continue; // self-loops would alias mutably below
        }
        let Ok((_, _, _, target_z, _)) = query.get(link.target) else {
            continue; // dangling link: no target, no transfer
        };

        let source_level = source_z.map_or(0, |z| z.0);
        let target_level = target_z.map_or(0, |z| z.0);
        let is_downhill = source_level > target_level;
        let has_power = power.is_some_and(|p| p.active);

        // Downhill is always free. Level/uphill requires power.
        if is_downhill || has_power {
            transfers.push((source_entity, link.target));
        }
    }

    // Phase 2: apply transfers.
    for (source_entity, target_entity) in transfers {
        let Ok([(_, mut source_chute, _, _, _), (_, mut target_chute, _, _, _)]) =
            query.get_many_mut([source_entity, target_entity])
        else {
            continue;
        };
        source_chute.inventory -= 1;
        target_chute.inventory += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_chute(world: &mut World, inventory: i32, z: Option<i32>) -> Entity {
        let mut e = world.spawn(GravityChute { inventory });
        if let Some(z) = z {
            e.insert(ZLevel(z));
        }
        e.id()
    }

    fn run_system(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(process_gravity_logistics_system);
        schedule.run(world);
    }

    #[test]
    fn test_items_move_downhill_without_power() {
        let mut world = World::new();
        let target = spawn_chute(&mut world, 0, Some(0));
        let source = world
            .spawn((GravityChute { inventory: 5 }, ZLevel(1), ChuteLink { target }))
            .id();

        run_system(&mut world);

        let a = world.get::<GravityChute>(source).unwrap();
        let b = world.get::<GravityChute>(target).unwrap();
        assert_eq!(a.inventory, 4, "source should lose one item downhill");
        assert_eq!(b.inventory, 1, "target should gain one item downhill");
    }

    #[test]
    fn test_items_cannot_move_uphill_without_power() {
        let mut world = World::new();
        let target = spawn_chute(&mut world, 0, Some(1));
        let source = world
            .spawn((
                GravityChute { inventory: 5 },
                ZLevel(0),
                ChuteLink { target },
                PowerConsumer {
                    demand: 1.0,
                    active: false,
                },
            ))
            .id();

        run_system(&mut world);

        let a = world.get::<GravityChute>(source).unwrap();
        let b = world.get::<GravityChute>(target).unwrap();
        assert_eq!(a.inventory, 5, "unpowered source keeps its items");
        assert_eq!(b.inventory, 0, "unpowered uphill target gains nothing");
    }

    #[test]
    fn test_items_move_uphill_with_power() {
        let mut world = World::new();
        let target = spawn_chute(&mut world, 0, Some(1));
        let source = world
            .spawn((
                GravityChute { inventory: 5 },
                ZLevel(0),
                ChuteLink { target },
                PowerConsumer {
                    demand: 1.0,
                    active: true,
                },
            ))
            .id();

        run_system(&mut world);

        let a = world.get::<GravityChute>(source).unwrap();
        let b = world.get::<GravityChute>(target).unwrap();
        assert_eq!(a.inventory, 4, "powered source pumps one item uphill");
        assert_eq!(b.inventory, 1, "uphill target gains one item");
    }

    #[test]
    fn test_items_move_level_with_power() {
        let mut world = World::new();
        let target = spawn_chute(&mut world, 0, Some(2));
        let source = world
            .spawn((
                GravityChute { inventory: 5 },
                ZLevel(2),
                ChuteLink { target },
                PowerConsumer {
                    demand: 1.0,
                    active: true,
                },
            ))
            .id();

        run_system(&mut world);

        let a = world.get::<GravityChute>(source).unwrap();
        let b = world.get::<GravityChute>(target).unwrap();
        assert_eq!(a.inventory, 4, "powered source moves one item level");
        assert_eq!(b.inventory, 1, "level target gains one item");
    }

    #[test]
    fn test_items_do_not_move_level_without_power() {
        let mut world = World::new();
        let target = spawn_chute(&mut world, 0, Some(2));
        // No PowerConsumer at all: missing counts as unpowered.
        let source = world
            .spawn((GravityChute { inventory: 5 }, ZLevel(2), ChuteLink { target }))
            .id();

        run_system(&mut world);

        let a = world.get::<GravityChute>(source).unwrap();
        let b = world.get::<GravityChute>(target).unwrap();
        assert_eq!(a.inventory, 5, "unpowered level source keeps items");
        assert_eq!(b.inventory, 0, "unpowered level target gains nothing");
    }

    #[test]
    fn test_empty_chute_transfers_nothing() {
        let mut world = World::new();
        let target = spawn_chute(&mut world, 0, Some(0));
        let source = world
            .spawn((GravityChute { inventory: 0 }, ZLevel(3), ChuteLink { target }))
            .id();

        run_system(&mut world);

        let a = world.get::<GravityChute>(source).unwrap();
        let b = world.get::<GravityChute>(target).unwrap();
        assert_eq!(a.inventory, 0, "empty source stays empty");
        assert_eq!(b.inventory, 0, "target gains nothing from empty source");
    }

    #[test]
    fn test_missing_zlevel_defaults_to_zero() {
        let mut world = World::new();
        // Neither node has ZLevel: both default to 0 -> level -> needs power.
        let target = world.spawn(GravityChute { inventory: 0 }).id();
        let source = world
            .spawn((GravityChute { inventory: 5 }, ChuteLink { target }))
            .id();

        run_system(&mut world);

        let a = world.get::<GravityChute>(source).unwrap();
        let b = world.get::<GravityChute>(target).unwrap();
        assert_eq!(a.inventory, 5, "z-less level source keeps items");
        assert_eq!(b.inventory, 0, "z-less level target gains nothing");
    }
}
