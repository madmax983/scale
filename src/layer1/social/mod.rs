//! Social Simulation and Relationships.
//!
//! This module handles the "Layer 1" social interactions between pops.
//! Unlike grand strategy games where factions interact abstractly, here individual pops
//! form relationships, hold grudges, and socialize in physical spaces.
//!
//! # Key Systems
//!
//! *   **Taverns**: Dedicated spaces for leisure recovery and socialization.
//! *   **Relationships**: Pairwise affinity scores (-100 to +100) between pops.
//! *   **Social Debt**: Informal favors tracked via [`debt::SocialDebt`].
//! *   **Proximity**: Pops gain morale buffs/debuffs simply by being near friends/enemies.

use crate::layer1::actions::{AssignedTo, AssignmentType};
use crate::layer1::cybernetics::{Augmentations, Prosthetic};
use crate::layer1::map::GridPosition;
use crate::layer1::needs::Needs;
use bevy_ecs::prelude::*;
use std::collections::HashMap;

/// Social gathering place component.
///
/// Attached to buildings (like Taverns) where pops go to relax.
/// Stores a list of current visitors to enable social interactions.
#[derive(Component)]
pub struct Tavern {
    /// Maximum number of visitors allowed simultaneously.
    pub capacity: usize,
    /// List of visitors currently socializing inside.
    pub visitors: Vec<Entity>,
}

impl Default for Tavern {
    fn default() -> Self {
        Self {
            capacity: 5,
            visitors: Vec::new(),
        }
    }
}

/// Executes the socialize action (pop entering tavern).
///
/// Checks capacity and assigns the pop to the tavern as a `TavernVisitor`.
pub fn handle_socialize(
    commands: &mut Commands,
    taverns: &mut Query<&mut Tavern>,
    target_entity: Entity,
    pop_entity: Entity,
    is_ostracized: bool,
) {
    if is_ostracized {
        return; // Ostracized pops are not welcome in taverns
    }

    if let Ok(mut tavern) = taverns.get_mut(target_entity) {
        if tavern.visitors.len() < tavern.capacity {
            tavern.visitors.push(pop_entity);
            commands.entity(pop_entity).insert(AssignedTo {
                entity: target_entity,
                assignment_type: AssignmentType::TavernVisitor,
            });
        }
    }
}

/// Restores leisure for pops visiting taverns.
///
/// Calculates leisure recovery based on the environment and crowd.
///
/// # Formula
///
/// `Recovery = Base * (1.0 + ZoneBonus + SocialBonus)`
///
/// *   **Base**: 0.05 per tick.
/// *   **`ZoneBonus`**: From [`crate::layer1::zone::ZoneType`] (e.g. Dining zone).
/// *   **`SocialBonus`**: `(VisitorCount - 1) * 0.1`. More people = more fun.
pub fn restore_leisure_system(
    mut needs_query: Query<&mut Needs>,
    tavern_query: Query<(&Tavern, &crate::layer1::building::Building, &GridPosition)>,
    zone_grid: Option<Res<crate::layer1::zone::ZoneGrid>>,
) {
    for (tavern, building, pos) in &tavern_query {
        let zone_bonus = zone_grid.as_ref().map_or(0.0, |grid| {
            let zone = grid.get(pos.x, pos.y);
            crate::layer1::zone::calculate_zone_bonus(zone, building.building_type)
        });

        let visitor_count = tavern.visitors.len();
        let social_bonus = if visitor_count > 1 {
            #[allow(clippy::cast_precision_loss)]
            {
                (visitor_count as f32 - 1.0) * 0.1
            }
        } else {
            0.0
        };

        for &visitor in &tavern.visitors {
            if let Ok(mut needs) = needs_query.get_mut(visitor) {
                let amount = 0.05 * (1.0 + zone_bonus + social_bonus);
                needs.leisure = (needs.leisure + amount).min(1.0);
            }
        }
    }
}

/// Relationships component storing affinity values for other pops.
///
/// Every pop has this component to track how they feel about others.
/// *   **Positive**: Friend (>20), Best Friend (>80).
/// *   **Negative**: Rival (<-20), Nemesis (<-80).
#[derive(Component, Default)]
pub struct Relationships {
    /// Map of target entity to affinity value (-100.0 to 100.0).
    pub affinities: HashMap<Entity, f32>,
}

impl Relationships {
    /// Gets the affinity towards a target entity. Defaults to 0.0 (Neutral).
    #[must_use]
    pub fn get_affinity(&self, target: Entity) -> f32 {
        *self.affinities.get(&target).unwrap_or(&0.0)
    }

    /// Sets the affinity towards a target entity, clamping between -100.0 and 100.0.
    pub fn set_affinity(&mut self, target: Entity, value: f32) {
        self.affinities.insert(target, value.clamp(-100.0, 100.0));
    }

    /// Helper to create a Relationships component with an initial affinity.
    #[must_use]
    pub fn with_affinity(target: Entity, value: f32) -> Self {
        let mut r = Self::default();
        r.set_affinity(target, value);
        r
    }
}

/// Event triggered when affinity between pops changes.
///
/// Use this to modify relationships. The system handles clamping and
/// modifiers (like cybernetic prejudice).
#[derive(Event)]
pub struct AffinityChange {
    /// The pop whose opinion is changing.
    pub source: Entity,
    /// The target of the opinion.
    pub target: Entity,
    /// The amount to change affinity by (positive or negative).
    pub amount: f32,
}

/// System to apply affinity changes from events.
///
/// Processes [`AffinityChange`] events and updates [`Relationships`].
/// Also applies "Cybernetic Prejudice": pops gain less affinity with cyborgs.
pub fn modify_affinity_system(
    mut events: EventReader<AffinityChange>,
    mut query: Query<&mut Relationships>,
    augmentations: Query<&Augmentations>,
    prosthetics: Query<&Prosthetic>,
) {
    for event in events.read() {
        if let Ok(mut rel) = query.get_mut(event.source) {
            let mut amount = event.amount;

            // Apply Cybernetic Penalty if Target has augmentations (Pops dislike cyborgs)
            if let Ok(augs) = augmentations.get(event.target) {
                let mut penalty = 0.0;
                for &item in &augs.installed {
                    if let Ok(prosthetic) = prosthetics.get(item) {
                        penalty += prosthetic.social_penalty;
                    }
                }

                // If gain is positive, reduce it by penalty
                if amount > 0.0 {
                    amount *= (1.0 - penalty).max(0.0);
                }
            }

            let current = rel.get_affinity(event.target);
            rel.set_affinity(event.target, current + amount);
        }
    }
}

/// Buff component applied when near friends (positive) or enemies (negative).
#[derive(Component)]
pub struct SocialBuff {
    /// The morale modifier value.
    pub value: f32,
}

/// System to calculate social proximity buffs/debuffs.
///
/// Checks for other pops within 5 tiles.
/// *   **Friends (>20 affinity)**: +0.1 morale per friend.
/// *   **Enemies (<-20 affinity)**: -0.1 morale per enemy.
///
/// Note: This is currently O(N^2) and should be optimized with spatial partitioning
/// if pop count grows large.
pub fn proximity_social_system(
    mut commands: Commands,
    pops: Query<(
        Entity,
        &GridPosition,
        &Relationships,
        Option<&crate::layer1::social::grievances::Ostracized>,
        Option<&crate::layer1::social::informants::SocialDesire>,
    )>,
    other_pops: Query<(
        Entity,
        &GridPosition,
        Option<&crate::layer1::social::grievances::Ostracized>,
        Option<&crate::layer1::social::informants::SocialDesire>,
    )>,
) {
    // O(N^2) naive implementation for Green phase
    for (entity, pos, rel, ostracized, desire) in pops.iter() {
        let mut total_buff: f32 = 0.0;

        // Ostracized pops get no proximity buff
        if ostracized.is_some() {
            commands.entity(entity).remove::<SocialBuff>();
            continue;
        }

        // Spec 323: pops too paranoid to socialize (SocialDesire 0) neither
        // mingle nor count as company — trust has fully broken down.
        if desire.map_or(false, |d| d.score <= 0.0) {
            commands.entity(entity).remove::<SocialBuff>();
            continue;
        }

        for (other_entity, other_pos, other_ostracized, other_desire) in other_pops.iter() {
            if entity == other_entity {
                continue;
            }

            // You don't get buffs from ostracized pops, you pretend they aren't there
            if other_ostracized.is_some() {
                continue;
            }

            // Spec 323: withdrawn pops are socially invisible.
            if other_desire.map_or(false, |d| d.score <= 0.0) {
                continue;
            }

            // Naive distance check
            let dx = pos.x.abs_diff(other_pos.x).min(i32::MAX as u32) as i32;
            let dy = pos.y.abs_diff(other_pos.y).min(i32::MAX as u32) as i32;
            let distance = dx.max(dy); // Chebyshev

            if distance <= 5 {
                // 5 tile radius
                let affinity = rel.get_affinity(other_entity);
                if affinity > 20.0 {
                    total_buff += 0.1; // Small boost per friend
                } else if affinity < -20.0 {
                    total_buff -= 0.1; // Small penalty per enemy
                }
            }
        }

        if total_buff.abs() > f32::EPSILON {
            commands
                .entity(entity)
                .insert(SocialBuff { value: total_buff });
        } else {
            commands.entity(entity).remove::<SocialBuff>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::map::GridPosition;
    use crate::layer1::needs::{decay_needs_system, Needs};
    use crate::layer1::pop::Pop;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_needs_has_leisure() {
        let needs = Needs::default();
        // Leisure starts high like others
        assert!((needs.leisure - 0.8).abs() < f32::EPSILON);
    }

    fn setup() -> World {
        crate::setup::init_task_pools();
        World::new()
    }

    #[test]
    fn test_leisure_decays() {
        let mut world = setup();
        world.spawn((Pop, Needs::default()));

        world.run_system_once(decay_needs_system).unwrap();

        let needs = world.query::<&Needs>().single(&world);
        assert!(needs.leisure < 0.8, "Leisure should decay");
    }

    #[test]
    fn test_tavern_component_defaults() {
        let tavern = Tavern::default();
        assert_eq!(tavern.capacity, 5); // Taverns hold more people than houses
        assert!(tavern.visitors.is_empty());
    }

    #[test]
    fn test_restore_leisure_system() {
        let mut world = World::new();

        let pop = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.2,
                    ..Default::default()
                },
            ))
            .id();

        let mut tavern = Tavern::default();
        tavern.visitors.push(pop);
        world.spawn((
            tavern,
            Building {
                building_type: BuildingType::Tavern,
            },
            GridPosition { x: 0, y: 0 },
        ));

        world.run_system_once(restore_leisure_system).unwrap();

        let needs = world.get::<Needs>(pop).unwrap();
        assert!(needs.leisure > 0.2, "Leisure should be restored");
        assert!(needs.leisure <= 1.0);
    }

    // NEW RELATIONSHIP TESTS

    #[test]
    fn test_relationships_default() {
        let rel = Relationships::default();
        assert!(rel.affinities.is_empty());
    }

    #[test]
    fn test_affinity_change_event() {
        let mut world = World::new();
        world.init_resource::<Events<AffinityChange>>(); // FIX: Init event resource

        let pop1 = world.spawn((Pop, Relationships::default())).id();
        let pop2 = world.spawn(Pop).id();

        // Trigger event to boost affinity
        world.send_event(AffinityChange {
            source: pop1,
            target: pop2,
            amount: 10.0,
        });

        // Register and run system
        let mut schedule = Schedule::default();
        schedule.add_systems(modify_affinity_system);
        schedule.run(&mut world);

        let rel = world.get::<Relationships>(pop1).unwrap();
        assert_eq!(rel.get_affinity(pop2), 10.0);
    }

    #[test]
    fn test_affinity_clamping() {
        let mut world = World::new();
        world.init_resource::<Events<AffinityChange>>(); // FIX: Init event resource

        let pop1 = world.spawn((Pop, Relationships::default())).id();
        let pop2 = world.spawn(Pop).id();

        world.send_event(AffinityChange {
            source: pop1,
            target: pop2,
            amount: 150.0,
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(modify_affinity_system);
        schedule.run(&mut world);

        let rel = world.get::<Relationships>(pop1).unwrap();
        assert_eq!(rel.get_affinity(pop2), 100.0); // Clamped at 100
    }

    #[test]
    fn test_proximity_morale_buff() {
        let mut world = World::new();

        // Pop 1 and Pop 2 are friends (affinity 50) and nearby
        let pop1 = world
            .spawn((
                Pop,
                GridPosition { x: 10, y: 10 },
                Relationships::with_affinity(Entity::PLACEHOLDER, 50.0), // Placeholder until updated
            ))
            .id();

        let pop2 = world
            .spawn((
                Pop,
                GridPosition { x: 10, y: 11 }, // Adjacent
            ))
            .id();

        // Update relationship with real ID
        world
            .get_mut::<Relationships>(pop1)
            .unwrap()
            .set_affinity(pop2, 50.0);

        // Run proximity system
        let mut schedule = Schedule::default();
        schedule.add_systems(proximity_social_system);
        schedule.run(&mut world);

        // Check for SocialBuff component
        let buff = world.get::<SocialBuff>(pop1);
        assert!(buff.is_some());
        assert!(buff.unwrap().value > 0.0);
    }

    #[test]
    fn test_proximity_morale_debuff() {
        let mut world = World::new();

        // Pop 1 and Pop 2 are enemies (affinity -50)
        let pop1 = world
            .spawn((Pop, GridPosition { x: 10, y: 10 }, Relationships::default()))
            .id();

        let pop2 = world.spawn((Pop, GridPosition { x: 10, y: 11 })).id();

        world
            .get_mut::<Relationships>(pop1)
            .unwrap()
            .set_affinity(pop2, -50.0);

        let mut schedule = Schedule::default();
        schedule.add_systems(proximity_social_system);
        schedule.run(&mut world);

        let buff = world.get::<SocialBuff>(pop1);
        assert!(buff.is_some());
        assert!(buff.unwrap().value < 0.0);
    }

    #[test]
    fn test_restore_leisure_with_zone_bonus() {
        let mut world = World::new();
        let mut zone_grid = crate::layer1::zone::ZoneGrid::new(10, 10);
        zone_grid.set(0, 0, crate::layer1::zone::ZoneType::Dining);
        world.insert_resource(zone_grid);

        let pop = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
            ))
            .id();

        let mut tavern = Tavern::default();
        tavern.visitors.push(pop);
        world.spawn((
            tavern,
            Building {
                building_type: BuildingType::Tavern,
            },
            GridPosition { x: 0, y: 0 },
        ));

        // Base: 0.05. Bonus (Dining): 0.1. Total: 0.05 * 1.1 = 0.055.
        world.run_system_once(restore_leisure_system).unwrap();
        let needs = world.get::<Needs>(pop).unwrap();
        assert!(
            (needs.leisure - 0.555).abs() < f32::EPSILON,
            "Expected 0.555, got {}",
            needs.leisure
        );
    }

    #[test]
    fn test_restore_leisure_social_bonus() {
        let mut world = World::new();

        let pop1 = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
            ))
            .id();

        let pop2 = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
            ))
            .id();

        let mut tavern = Tavern::default();
        tavern.visitors.push(pop1);
        tavern.visitors.push(pop2);

        world.spawn((
            tavern,
            Building {
                building_type: BuildingType::Tavern,
            },
            GridPosition { x: 0, y: 0 },
        ));

        // Base: 0.05
        // Social Bonus: (2 - 1) * 0.1 = 0.1
        // Total Amount: 0.05 * (1.0 + 0.1) = 0.055

        world.run_system_once(restore_leisure_system).unwrap();

        let needs1 = world.get::<Needs>(pop1).unwrap();
        assert!(
            (needs1.leisure - 0.555).abs() < f32::EPSILON,
            "Expected 0.555 with social bonus, got {}",
            needs1.leisure
        );
    }
}
/// Social debt system (Spec 130).
pub mod debt;
/// Public Grievances system (Spec 233).
pub mod grievances;
pub mod protest_crowds;
pub mod secret_societies;
pub use protest_crowds::*;
/// Old Guard logic: Generational friction between Founders and Immigrants (Spec 078).
pub mod old_guard;

mod old_guard_tests;

pub use debt::*;
pub use grievances::*;

/// Empty Room logic (Spec 251).
pub mod empty_room;

mod empty_room_tests;

pub use empty_room::*;

/// Cultural Vandalism (Spec 253).
pub mod cultural_vandalism;
pub use cultural_vandalism::*;

/// Subspace Pen Pals (Spec 257).
pub mod pen_pals;
pub use pen_pals::*;

/// The Cadet Branch (Spec 263).
pub mod cadet;
pub use cadet::*;
pub mod indoctrination;
pub use indoctrination::*;

pub mod placebo;

/// Secret Societies system (Spec 147).
pub mod society;
pub use society::*;

pub mod subversion;
#[allow(unused_imports)]
#[allow(ambiguous_glob_reexports)]
pub use subversion::*;

/// The Sentient Standard logic (Spec 295).
pub mod sentient_standard;
pub use sentient_standard::*;
pub mod zero_g_sports;
pub use zero_g_sports::*;

/// Rumor web system (Spec 055).
pub mod rumor;
pub use rumor::*;

/// Civil unrest and mental break system.
pub mod unrest;
pub use unrest::*;

/// Pop factions system (Spec 068).
pub mod factions;
pub use factions::*;

#[cfg(test)]
/// Tests for faction demands logic (Spec 085).
pub mod faction_demands_tests;

/// Mentorship system (Spec 069).
pub mod mentorship;
pub use mentorship::*;

/// Omens & Taboos system (Spec 088).
pub mod taboo;
pub use taboo::*;

/// Morale system (Spec 031/090).
pub mod morale;
pub use morale::*;

/// Social stratification system (Spec 113).
pub mod social_stratification;
pub use social_stratification::*;

/// Technological rituals and machine spirits.
pub mod rituals;
pub use rituals::*;

pub mod echoes;
/// Social mimicry system (Spec 128).
pub mod social_mimicry;
pub use social_mimicry::*;

/// Civic Ideology system (Spec 197).
pub mod civic_ideology;
pub use civic_ideology::*;

/// Customs checkpoint system (Spec 214).
pub mod customs;
pub use customs::*;

/// Politics and Elections (Spec 241).
pub mod politics;
pub mod propaganda;
pub use politics::*;
pub use propaganda::*;
pub mod scrap_code_prophets;
pub use scrap_code_prophets::*;
pub mod ghost_shift_strike;
pub use ghost_shift_strike::*;
pub mod exile;
pub use exile::*;
pub mod gossip_economy;
pub use gossip_economy::*;
pub mod generational_dissonance;
pub use generational_dissonance::*;
pub mod pirates;

pub mod hoarder;
pub mod sub_lithic;
pub use hoarder::*;
pub mod xenoflora_pet;
pub use xenoflora_pet::*;
pub mod cargo_cult;
pub use cargo_cult::*;

pub mod pop_relationships;
pub use pop_relationships::*;
pub mod golden_age;
pub use golden_age::*;
pub mod inherited_grudges;
pub use inherited_grudges::*;
pub mod emotional_contagion;
pub mod hedonic_treadmill;
pub use hedonic_treadmill::*;
pub mod hedonic_treadmill_integration;
pub mod the_empty_room;
pub use the_empty_room::*;
pub mod whisper_trade;
pub use whisper_trade::*;

pub mod biometric_drift;
pub mod sartorial_rebellion;
pub mod xeno_pet;
pub use sartorial_rebellion::*;

pub mod generational_knowledge;
pub mod memory_tithe;
pub mod propaganda_graffitists;
pub mod slippery_slope;
pub use propaganda_graffitists::*;
pub mod faction_diet;
pub use faction_diet::*;
pub mod flesh_famine;
pub use flesh_famine::*;
pub mod architecture_of_paranoia;
pub mod blacksite;
pub mod bureau_of_regrets;
#[allow(unused_imports)]
#[allow(ambiguous_glob_reexports)]
pub use architecture_of_paranoia::*;

pub mod pets;
pub use pets::*;
pub mod bureaucratic_strike;

pub mod scapegoat;

/// The Ship of Fools (Spec 1375): useless luxury-demanding passengers.
pub mod ship_of_fools;
pub use ship_of_fools::*;

/// The Martyrdom Effect (Spec 272): enemy-slain leaders ignite colony-wide
/// fervor — unrest silenced, work redoubled, ideological casus belli.
pub mod martyrdom;
pub use martyrdom::*;

/// The Feral Outpost (Spec 273): frontier pops drift from the colony's
/// culture, found wild outposts, and refuse orders to return.
pub mod feral_outpost;
pub use feral_outpost::*;

/// The Empathy Broadcast (Spec 305): a neighbouring empire's psychic
/// distress wave saturates every colony pop with unavoidable stress and
/// posts a DemandIntervention grievance.
pub mod empathy_broadcast;
pub use empathy_broadcast::*;

/// The Informant's Dilemma (Spec 323): the Citizen Informant edict pays pops
/// to report dissent — crushing unrest while breeding colony-wide paranoia,
/// social withdrawal, and false treason accusations.
pub mod informants;
pub use informants::*;
