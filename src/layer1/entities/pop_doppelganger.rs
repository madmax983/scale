//! Doppelgangers and social mimics.
//!
//! This module introduces the terrifying reality of social mimics—entities that infiltrate
//! the colony by killing and replacing a [`Pop`]. Once hidden, they subtly sabotage operations
//! (like stalling mining progress) until they are detected.
//!
//! The `pop_doppelganger` module handles replacing legitimate pops with impostors,
//! executing their secretive sabotage tasks, and revealing their true nature to the colony.

use crate::layer1::execution::MovementTarget;
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::PopName;
use crate::layer1::resources::MiningProgress;
use bevy_ecs::prelude::*;

/// A component attached to an infiltrator entity masquerading as a legitimate pop.
///
/// A `Mimic` retains the name and basic attributes of the pop it replaced but works
/// against the colony's goals when unobserved.
///
/// # Examples
///
/// ```
/// use scale::layer1::entities::pop_doppelganger::{Mimic, MimicState};
///
/// let impostor = Mimic {
///     state: MimicState::Hidden,
///     original_identity: "Miner Bob".to_string(),
/// };
/// ```
#[derive(Component, Debug, PartialEq, Clone)]
pub struct Mimic {
    /// The current operational state of the mimic.
    pub state: MimicState,
    /// The name of the pop that was originally replaced by this mimic.
    pub original_identity: String,
}

/// The current operational state of a [`Mimic`].
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MimicState {
    /// The mimic is successfully masquerading as a normal pop.
    Hidden,
    /// The mimic has been discovered and is now recognized as a threat.
    Revealed,
}

/// Despawns a target pop and spawns a new identical entity with a [`Mimic`] component.
///
/// This function carefully copies the `PopName`, `GridPosition`, and `Health` components
/// from the victim to the new mimic entity to ensure the colony remains unaware of the substitution.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::entities::pop_doppelganger::replace_pop_with_mimic;
/// use scale::layer1::pop::{Pop, PopName};
///
/// let mut world = World::new();
/// let original_pop = world.spawn((Pop, PopName("Alice".to_string()))).id();
///
/// // The original entity is destroyed, replaced by a mimic with the same name.
/// let mimic_entity = replace_pop_with_mimic(&mut world, original_pop);
/// ```
pub fn replace_pop_with_mimic(world: &mut World, target: Entity) -> Entity {
    let mut name = None;
    let mut pos = None;
    let mut health = None;

    if let Ok(entity_ref) = world.get_entity(target) {
        if let Some(n) = entity_ref.get::<PopName>() {
            name = Some(n.clone());
        }
        if let Some(p) = entity_ref.get::<GridPosition>() {
            pos = Some(*p);
        }
        if let Some(h) = entity_ref.get::<Health>() {
            health = Some(h.clone());
        }
    }

    world.despawn(target);

    // GHOST-POP FIX (2026-10-04): the mimic used to spawn as a bare Pop
    // with only name/pos/health copied over — no Needs, no Wallet, no
    // Morale — invisible to parts of the sim. Build a full PopBundle so
    // the mimic is a believable colonist, then restore the victim's identity.
    let mut rng = rand::thread_rng();
    let (x, y) = pos.map(|p| (p.x, p.y)).unwrap_or((0, 0));
    let mut bundle = crate::layer1::PopBundle::random(x, y, &mut rng);
    if let Some(n) = name.clone() {
        bundle.name = n;
    }
    if let Some(h) = health {
        bundle.health = h;
    }

    let orig_name = name
        .map(|n| n.0.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let mut spawn = world.spawn(bundle);
    spawn.insert(Mimic {
        state: MimicState::Hidden,
        original_identity: orig_name,
    });

    spawn.id()
}

/// A Bevy system that causes hidden mimics to sabotage colony tasks.
///
/// If a [`Mimic`] is in the [`MimicState::Hidden`] state and targets a resource for mining,
/// they will stall the [`MiningProgress`] at 80%, preventing completion.
///
/// # Examples
///
/// ```
/// use scale::layer1::entities::pop_doppelganger::sabotage_system;
/// use bevy_ecs::prelude::*;
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(sabotage_system);
/// // The schedule will stall any mining progress targeted by a hidden mimic!
/// ```
pub fn sabotage_system(
    query: Query<(&Mimic, &MovementTarget)>,
    mut target_query: Query<&mut MiningProgress>,
) {
    for (mimic, target) in query.iter() {
        if mimic.state == MimicState::Hidden {
            if let Ok(mut progress) = target_query.get_mut(target.target_entity) {
                // Stall mining progress
                if progress.current > 80.0 {
                    progress.current = 80.0;
                }
            }
        }
    }
}

/// Exposes a mimic's true nature by changing its state to [`MimicState::Revealed`].
///
/// # Returns
///
/// Returns `true` if the entity was a hidden mimic and was successfully revealed,
/// or `false` if the entity was not a mimic or could not be found.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::entities::pop_doppelganger::{reveal_mimic, Mimic, MimicState};
///
/// let mut world = World::new();
/// let suspect = world.spawn(Mimic {
///     state: MimicState::Hidden,
///     original_identity: "Bob".to_string()
/// }).id();
///
/// let was_mimic = reveal_mimic(&mut world, suspect);
/// assert!(was_mimic);
/// ```
pub fn reveal_mimic(world: &mut World, target: Entity) -> bool {
    if let Ok(mut entity_mut) = world.get_entity_mut(target) {
        if let Some(mut mimic) = entity_mut.get_mut::<Mimic>() {
            mimic.state = MimicState::Revealed;
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::execution::AtTarget;
    use crate::layer1::pop::Pop;
    use crate::layer1::utility_types::ActionType;

    #[test]
    fn test_mimic_replacement() {
        let mut world = World::new();
        let original_pop = world
            .spawn((
                Pop,
                PopName("Miner Bob".to_string()),
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 10, y: 10 },
            ))
            .id();

        let mimic_entity = replace_pop_with_mimic(&mut world, original_pop);

        assert!(world.get_entity(original_pop).is_err());

        let mimic_name = world.get::<PopName>(mimic_entity).unwrap();
        assert_eq!(mimic_name.0, "Miner Bob");

        assert!(world.get::<Mimic>(mimic_entity).is_some());

        // GHOST-POP FIX (2026-10-04): the replacement must be a complete
        // colonist — Health, Wallet, Needs, Morale — not a bare Pop.
        assert!(world.get::<Health>(mimic_entity).is_some());
        assert!(world
            .get::<crate::layer1::economy::Wallet>(mimic_entity)
            .is_some());
        assert!(world
            .get::<crate::layer1::needs::Needs>(mimic_entity)
            .is_some());
        assert!(world
            .get::<crate::layer1::morale::Morale>(mimic_entity)
            .is_some());
        // Victim's tile is preserved.
        let pos = world.get::<GridPosition>(mimic_entity).unwrap();
        assert_eq!((pos.x, pos.y), (10, 10));
    }

    #[test]
    fn test_mimic_sabotage_during_work() {
        let mut world = World::new();

        let rock = world
            .spawn(MiningProgress {
                current: 90.0,
                max: 100.0,
            })
            .id();

        world.spawn((
            Pop,
            Mimic {
                state: MimicState::Hidden,
                original_identity: "Miner Bob".to_string(),
            },
            MovementTarget {
                target_entity: rock,
                target_position: GridPosition { x: 1, y: 1 },
                for_action: ActionType::Work,
            },
            AtTarget,
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(sabotage_system);
        schedule.run(&mut world);

        let progress = world.get::<MiningProgress>(rock).unwrap();
        assert!(progress.current <= 80.0);
    }

    #[test]
    fn test_mimic_detection_via_scan() {
        let mut world = World::new();
        let mimic = world
            .spawn((
                Pop,
                Mimic {
                    state: MimicState::Hidden,
                    original_identity: "Suspect".to_string(),
                },
                PopName("Suspect".to_string()),
            ))
            .id();

        let is_mimic = reveal_mimic(&mut world, mimic);
        assert!(is_mimic);

        let mimic_comp = world.get::<Mimic>(mimic).unwrap();
        assert_eq!(mimic_comp.state, MimicState::Revealed);
    }
}
