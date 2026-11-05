use crate::layer1::cabin_fever::CabinFever;
use crate::layer1::morale::Morale;
use crate::layer1::needs::Needs;
use crate::layer1::pop::Pop;
use crate::layer1::structure::{Fragile, Structure, FRAGILITY_DAMAGE_MULTIPLIER};
use crate::layer1::traits::{Trait, Traits};
use bevy_ecs::prelude::*;

/// Global unrest level in the colony.
///
/// *   **0.0**: Perfect harmony.
/// *   **1.0**: Total anarchy.
#[derive(Resource, Default, Debug)]
pub struct Unrest {
    /// Current level of unrest (0.0 to 1.0).
    pub level: f32,
    /// Active modifiers affecting unrest.
    pub modifiers: Vec<UnrestModifier>,
}

/// A temporary modifier affecting global unrest.
#[derive(Debug, Clone)]
pub struct UnrestModifier {
    /// The value of the modifier (negative reduces unrest).
    pub value: f32,
    /// Remaining duration in ticks.
    pub duration: u32,
    /// The reason for this unrest modification.
    pub label: String,
}

/// Marker component for a Pop identified as a Scapegoat target.
#[derive(Component, Debug)]
pub struct ScapegoatTarget;

/// Actions that can be taken against a Scapegoat.
#[derive(Debug, Clone, Copy)]
pub enum ScapegoatAction {
    /// Publicly shame the target (Trauma, Reduced Unrest).
    PublicShame,
    /// Exile the target (Despawn, Moderate Unrest Reduction).
    Exile,
    /// Execute the target (Despawn + Corpse, High Unrest Reduction).
    Execute,
}

/// Event triggered when a player chooses to denounce a scapegoat.
#[derive(Event, Debug, Clone)]
pub struct DenounceEvent {
    /// The target entity to denounce.
    pub target: Entity,
    /// The action to take.
    pub action: ScapegoatAction,
}

/// Represents the mental stability of a Pop.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MentalState {
    /// The Pop is functioning normally.
    #[default]
    Normal,
    /// The Pop has suffered a mental break and is acting out.
    Broken(MentalBreakType),
}

/// The specific type of mental break a Pop is experiencing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MentalBreakType {
    /// The Pop destroys nearby structures.
    Vandalize,
    /// The Pop consumes resources uncontrollably.
    Binge,
    /// The Pop wanders aimlessly, unresponsive to commands.
    Daze,
    /// The Pop sleepwalks while resting.
    Sleepwalking,
    /// The Pop forms a violent mob due to forged memories collapsing.
    RealityCollapse,
    /// The Pop becomes extremely violent due to withdrawal or other severe reasons.
    Violent,
}

/// Calculates global unrest based on average colony morale.
///
/// Spec 393: while the Martial Law edict is active, unrest is
/// suppressed to 0. The suppression lives here — in the system that
/// recomputes `level` from scratch every tick — so it can never
/// accumulate or leave a permanent residue once the edict is repealed.
pub fn calculate_unrest_system(
    mut unrest: ResMut<Unrest>,
    query: Query<&Morale, With<Pop>>,
    policies: Option<Res<crate::layer1::edicts::ColonyPolicies>>,
) {
    if policies.is_some_and(|p| crate::layer1::edicts::is_unrest_suppressed(&p)) {
        // Still tick down timed modifiers so they expire on schedule
        // underneath the suppression.
        unrest.modifiers.retain_mut(|m| {
            if m.duration > 0 {
                m.duration -= 1;
                true
            } else {
                false
            }
        });
        unrest.level = 0.0;
        return;
    }
    let mut total_morale = 0.0;
    let mut count = 0;

    for morale in query.iter() {
        total_morale += morale.value;
        count += 1;
    }

    let mut base_unrest = 0.0;
    if count > 0 {
        #[allow(clippy::cast_precision_loss)]
        let avg_morale = total_morale / count as f32;
        base_unrest = (1.0 - avg_morale).clamp(0.0, 1.0);
    }

    // Process modifiers
    let mut modifier_sum = 0.0;
    unrest.modifiers.retain_mut(|m| {
        if m.duration > 0 {
            m.duration -= 1;
            modifier_sum += m.value;
            true
        } else {
            false
        }
    });

    unrest.level = (base_unrest + modifier_sum).clamp(0.0, 1.0);
}

/// Identifies a suitable scapegoat when unrest is high.
#[allow(clippy::type_complexity)]
pub fn identify_scapegoat_system(
    mut commands: Commands,
    unrest: Res<Unrest>,
    query: Query<(Entity, Option<&Traits>), (With<Pop>, Without<ScapegoatTarget>)>,
    existing_scapegoats: Query<(), With<ScapegoatTarget>>,
) {
    if unrest.level < 0.6 || !existing_scapegoats.is_empty() {
        return; // No scapegoat needed if unrest is low or one already exists
    }

    // Simple priority: Outsider -> Mutant -> Random
    let mut best_candidate = None;
    let mut best_score = 0;

    for (entity, traits_opt) in query.iter() {
        let score = traits_opt.map_or(10, |traits| {
            if traits.has(Trait::Outsider) {
                100
            } else if traits.has(Trait::Mutant) {
                50
            } else {
                10
            }
        });

        if score > best_score {
            best_score = score;
            best_candidate = Some(entity);
        }
    }

    if let Some(entity) = best_candidate {
        commands.entity(entity).insert(ScapegoatTarget);
    }
}

/// System that processes Denounce events.
pub fn handle_denounce_event_system(
    mut commands: Commands,
    mut events: EventReader<DenounceEvent>,
    mut unrest: ResMut<Unrest>,
    mut query: Query<&mut MentalState>,
) {
    for event in events.read() {
        // Apply Unrest reduction modifier
        let (reduction, duration) = match event.action {
            ScapegoatAction::Exile => (0.5, 2000),
            ScapegoatAction::PublicShame => (0.2, 1000),
            ScapegoatAction::Execute => (0.8, 3000),
        };

        unrest.modifiers.push(UnrestModifier {
            value: -reduction,
            duration,
            label: "Scapegoat".to_string(),
        });

        // Apply Penalty
        match event.action {
            ScapegoatAction::Exile | ScapegoatAction::Execute => {
                // Despawn the entity
                commands.entity(event.target).despawn();
            }
            ScapegoatAction::PublicShame => {
                if let Ok(mut state) = query.get_mut(event.target) {
                    *state = MentalState::Broken(MentalBreakType::Daze); // Represent trauma/shame
                }
                commands.entity(event.target).remove::<ScapegoatTarget>();
            }
        }
    }
}

/// System to check if Pops should suffer a mental break based on morale.
pub fn check_mental_break_system(
    mut query: Query<(
        &Needs,
        Option<&Morale>,
        Option<&CabinFever>,
        &mut MentalState,
    )>,
) {
    for (needs, morale_comp, fever, mut state) in &mut query {
        if *state == MentalState::Normal {
            let morale = morale_comp.map_or_else(|| needs.morale(), |m| m.value);
            let stress_break = fever.is_some_and(|f| f.total_stress() >= 90.0);

            if morale < 0.15 || stress_break {
                // Simplified deterministic logic for Green phase
                // In real game, use RNG.
                *state = MentalState::Broken(MentalBreakType::Vandalize);
            }
        }
    }
}

/// Logic for executing a Vandalize action against a target.
#[allow(clippy::cast_precision_loss)]
pub fn perform_vandalize_logic(world: &mut World, _pop: Entity, target: Entity) {
    let mut damage = 10.0;

    if let Some(fragile) = world.get::<Fragile>(target) {
        damage *= (fragile.stacks as f32).mul_add(FRAGILITY_DAMAGE_MULTIPLIER, 1.0);
    }

    if let Some(mut structure) = world.get_mut::<Structure>(target) {
        structure.current_hp = (structure.current_hp - damage).max(0.0);
    }
}

/// System to check if Pops recover from a mental break.
pub fn recover_mental_break_system(mut query: Query<(&Needs, &mut MentalState)>) {
    for (needs, mut state) in &mut query {
        #[allow(clippy::collapsible_if)]
        if let MentalState::Broken(_) = *state {
            if needs.morale() > 0.3 {
                *state = MentalState::Normal;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::map::GridPosition;
    use crate::layer1::needs::Needs;
    use crate::layer1::pop::Pop;
    use crate::layer1::structure::Structure;
    use crate::layer1::utility_ai::{evaluate_actions_system, ActionType, PopAction};
    use bevy_ecs::system::RunSystemOnce;

    fn setup_world() -> World {
        let mut world = World::new();
        crate::setup::init_task_pools(); // Required for parallel queries in evaluate_actions_system
        world.insert_resource(crate::layer1::utility_types::UtilityConfig::default());
        world.insert_resource(crate::layer1::resources::ColonyResources::default());
        world.insert_resource(crate::shared::time::SimulationTime::default());
        world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world.insert_resource(crate::layer1::erosion::ErosionGrid::new(10, 10));
        world.insert_resource(crate::layer1::building::OccupiedTiles::default());
        world.insert_resource(crate::layer1::terrain::TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![crate::layer1::terrain::TerrainType::Grass; 100],
        });
        world
    }

    #[test]
    fn test_mental_state_default() {
        let state = MentalState::default();
        assert_eq!(state, MentalState::Normal);
    }

    #[test]
    fn test_check_break_risk_triggers_break() {
        let mut world = setup_world();
        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.1,
                    rest: 0.1,
                    leisure: 0.1,
                    hygiene: 0.1,
                }, // Very low morale (0.1)
                MentalState::Normal,
            ))
            .id();

        // Run check system
        world.run_system_once(check_mental_break_system).unwrap();

        let state = world.get::<MentalState>(pop).unwrap();
        // Should have transitioned to Broken
        // This fails initially as system does nothing
        assert!(matches!(state, MentalState::Broken(_)));
    }

    #[test]
    fn test_broken_state_overrides_utility_ai() {
        let mut world = setup_world();

        // Pop is Broken(Vandalize)
        let pop = world
            .spawn((
                Pop,
                Needs::default(),
                MentalState::Broken(MentalBreakType::Vandalize),
                PopAction {
                    ticks_committed: 10,
                    ..Default::default()
                },
                GridPosition { x: 0, y: 0 },
                crate::layer1::utility_types::UtilityWeights::default(),
            ))
            .id();

        // Add a building target
        world.spawn((
            Building {
                building_type: BuildingType::Housing,
            },
            GridPosition { x: 1, y: 0 },
            Structure {
                current_hp: 100.0,
                max_hp: 100.0,
            },
        ));

        // Run utility AI
        evaluate_actions_system(&mut world);

        let action = world.get::<PopAction>(pop).unwrap();

        assert_eq!(action.current, ActionType::Vandalize);
    }

    #[test]
    fn test_vandalize_damages_building() {
        let mut world = setup_world();

        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 1, y: 0 },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
            ))
            .id();

        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                MentalState::Broken(MentalBreakType::Vandalize),
            ))
            .id();

        // Manually trigger damage logic
        perform_vandalize_logic(&mut world, pop, building);

        let structure = world.get::<Structure>(building).unwrap();
        // Should fail as logic is empty
        assert!(structure.current_hp < 100.0);
    }

    #[test]
    fn test_recover_from_break_when_morale_improves() {
        let mut world = setup_world();
        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.1,
                    rest: 0.1,
                    leisure: 0.1,
                    hygiene: 0.8,
                },
                MentalState::Broken(MentalBreakType::Vandalize),
            ))
            .id();

        // Improve morale
        let mut needs = world.get_mut::<Needs>(pop).unwrap();
        needs.hunger = 0.8;
        needs.rest = 0.8;
        needs.leisure = 0.8;

        world.run_system_once(recover_mental_break_system).unwrap();

        let state = world.get::<MentalState>(pop).unwrap();
        assert_eq!(*state, MentalState::Normal);
    }

    #[test]
    fn test_vandalize_integration_damages_building() {
        use crate::layer1::execution::{
            movement_system, process_start_plan_system, vandalize_execution_system, AtTarget,
            MovementTarget,
        };
        use crate::layer1::utility_ai::{
            evaluate_actions_system, PopAction, StartPlan, UtilityWeights,
        };

        let mut world = setup_world();
        crate::setup::init_task_pools(); // Ensure task pools are initialized

        // Create a Building (Target)
        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 1, y: 0 },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
            ))
            .id();

        // Create a Pop with Vandalize mental break
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Needs::default(),
                MentalState::Broken(MentalBreakType::Vandalize),
                PopAction {
                    ticks_committed: 10, // Force evaluation
                    ..Default::default()
                },
                UtilityWeights::default(),
            ))
            .id();

        // 1. Evaluate Actions (should pick Vandalize AND a target)
        evaluate_actions_system(&mut world);

        // Verify action picked
        let action = world.get::<PopAction>(pop).unwrap();
        assert_eq!(
            action.current,
            ActionType::Vandalize,
            "Should pick Vandalize action"
        );

        // Verify StartPlan has target (THIS IS WHERE IT FAILS CURRENTLY)
        let plan = world.get::<StartPlan>(pop).unwrap();
        assert_eq!(plan.action, ActionType::Vandalize);
        assert!(plan.target.is_some(), "Vandalize requires a target!");
        assert_eq!(plan.target, Some(building), "Should target the building");

        // 2. Process StartPlan -> MovementTarget
        world.run_system_once(process_start_plan_system).unwrap();
        assert!(
            world.get::<MovementTarget>(pop).is_some(),
            "Should have MovementTarget"
        );

        // 3. Move (pop is adjacent, so should arrive immediately or in 1 tick)
        world.run_system_once(movement_system).unwrap();

        // If pop moved to (1,0), it should be AtTarget.
        let pos = world.get::<GridPosition>(pop).unwrap();
        assert_eq!(pos.x, 1);
        assert!(world.get::<AtTarget>(pop).is_some(), "Should be AtTarget");

        // 4. Execute Vandalize
        world.run_system_once(vandalize_execution_system).unwrap();

        // Verify Damage
        let structure = world.get::<Structure>(building).unwrap();
        assert!(structure.current_hp < 100.0, "Building should take damage");
    }

    #[test]
    fn test_unrest_calculation() {
        let mut world = setup_world();
        world.insert_resource(Unrest::default());

        // Spawn pops with varying morale
        // Pop 1: High morale (1.0)
        world.spawn((
            Pop,
            Morale {
                value: 1.0,
                ..Default::default()
            },
        ));
        // Pop 2: Low morale (0.0)
        world.spawn((
            Pop,
            Morale {
                value: 0.0,
                ..Default::default()
            },
        ));

        // Run system
        world.run_system_once(calculate_unrest_system).unwrap();

        let unrest = world.resource::<Unrest>();
        // Avg Morale = 0.5. Unrest = 1.0 - 0.5 = 0.5.
        // Fails if system is empty
        assert!(
            (unrest.level - 0.5).abs() < f32::EPSILON,
            "Unrest should be 0.5"
        );
    }

    #[test]
    fn test_scapegoat_identification_preference() {
        let mut world = setup_world();
        // Insert Unrest high enough to trigger search
        world.insert_resource(Unrest {
            level: 0.8,
            ..Default::default()
        });

        // Normal Pop
        let normal = world.spawn((Pop, Traits::default())).id();

        // Outsider Pop (Should be preferred)
        let mut traits = Traits::default();
        traits.add(Trait::Outsider);
        let outsider = world.spawn((Pop, traits)).id();

        world.run_system_once(identify_scapegoat_system).unwrap();

        // Check if Outsider was tagged
        assert!(
            world.get::<ScapegoatTarget>(outsider).is_some(),
            "Outsider should be targeted"
        );
        assert!(
            world.get::<ScapegoatTarget>(normal).is_none(),
            "Normal pop should not be targeted"
        );
    }

    #[test]
    fn test_denounce_lowers_unrest_and_punishes_target() {
        let mut world = setup_world();
        world.insert_resource(Unrest {
            level: 0.9,
            ..Default::default()
        });
        world.init_resource::<Events<DenounceEvent>>();

        let target = world
            .spawn((Pop, Traits::default(), ScapegoatTarget, MentalState::Normal))
            .id();

        // Send Denounce event (Exile)
        world.send_event(DenounceEvent {
            target,
            action: ScapegoatAction::Exile,
        });

        // Run system
        world.run_system_once(handle_denounce_event_system).unwrap();

        // Verify Modifier added
        let unrest = world.resource::<Unrest>();
        assert!(!unrest.modifiers.is_empty(), "Should add a modifier");
        assert!(
            unrest.modifiers[0].value < -0.4,
            "Modifier should be significant"
        );

        // Verify Target is gone (Exiled/Despawned)
        assert!(
            world.get_entity(target).is_err(),
            "Target should be despawned"
        );
    }

    #[test]
    fn test_denounce_punishment_trauma() {
        let mut world = setup_world();
        world.insert_resource(Unrest {
            level: 0.9,
            ..Default::default()
        });
        world.init_resource::<Events<DenounceEvent>>();

        let target = world
            .spawn((Pop, Traits::default(), ScapegoatTarget, MentalState::Normal))
            .id();

        // Send Denounce event (Shame)
        world.send_event(DenounceEvent {
            target,
            action: ScapegoatAction::PublicShame,
        });

        // Run system
        world.run_system_once(handle_denounce_event_system).unwrap();

        // Verify Modifier added
        let unrest = world.resource::<Unrest>();
        assert!(!unrest.modifiers.is_empty(), "Should add a modifier");
        assert!(
            unrest.modifiers[0].value < 0.0,
            "Modifier should be negative"
        );

        // Verify Target gained Traumatized trait or breakdown
        let state = world.get::<MentalState>(target).unwrap();
        // Assuming we use Breakdown::Daze as per my plan
        assert!(
            matches!(state, MentalState::Broken(MentalBreakType::Daze)),
            "Target should be broken"
        );
    }
}
