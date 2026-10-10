use crate::layer1::resources::ResourceType;
use crate::layer1::social::placebo::PlaceboProtocol;
use bevy_ecs::prelude::*;
use std::collections::HashSet;

/// Resource tracking active colony policies/edicts.

#[derive(Debug, Clone, PartialEq, Eq)]
/// Stores the current state of a policy, including its active status and duration.
///
/// # Examples
///
/// ```
/// use scale::layer1::administration::edicts::PolicyState;
///
/// let state = PolicyState { active: true, duration: 10, is_tradition: false };
/// ```
/// Stores the current state of a policy, including its active status and duration.
///
/// # Examples
///
/// ```
/// use scale::layer1::administration::edicts::PolicyState;
///
/// let state = PolicyState { active: true, duration: 10, is_tradition: false };
/// ```
/// Stores the current state of a policy, including its active status and duration.
///
/// # Examples
///
/// ```
/// use scale::layer1::administration::edicts::PolicyState;
///
/// let state = PolicyState { active: true, duration: 10, is_tradition: false };
/// ```
/// Stores the current state of a policy, including its active status and duration.
///
/// # Examples
///
/// ```
/// use scale::layer1::administration::edicts::PolicyState;
///
/// let state = PolicyState { active: true, duration: 10, is_tradition: false };
/// ```
pub struct PolicyState {
    /// Whether the policy is currently active.
    /// Whether the policy is currently active.
    /// Whether the policy is currently active.
    /// Whether the policy is currently active.
    pub active: bool,
    /// The remaining duration of the policy in ticks.
    /// The remaining duration of the policy in ticks.
    /// The remaining duration of the policy in ticks.
    /// The remaining duration of the policy in ticks.
    pub duration: u32,
    /// Whether this policy has been entrenched as a cultural tradition.
    /// Whether this policy has been entrenched as a cultural tradition.
    /// Whether this policy has been entrenched as a cultural tradition.
    /// Whether this policy has been entrenched as a cultural tradition.
    pub is_tradition: bool,
}

#[derive(Resource, Default, Debug, Clone)]
/// Resource tracking active colony policies/edicts and their states.
///
/// # Examples
///
/// ```
/// use scale::layer1::administration::edicts::{ColonyPolicies, Policy};
///
/// let mut policies = ColonyPolicies::default();
/// policies.toggle(Policy::Rationing);
/// assert!(policies.is_active(Policy::Rationing));
/// ```
/// Resource tracking active colony policies/edicts and their states.
///
/// # Examples
///
/// ```
/// use scale::layer1::administration::edicts::{ColonyPolicies, Policy};
///
/// let mut policies = ColonyPolicies::default();
/// policies.toggle(Policy::Rationing);
/// assert!(policies.is_active(Policy::Rationing));
/// ```
/// Resource tracking active colony policies/edicts and their states.
///
/// # Examples
///
/// ```
/// use scale::layer1::administration::edicts::{ColonyPolicies, Policy};
///
/// let mut policies = ColonyPolicies::default();
/// policies.toggle(Policy::Rationing);
/// assert!(policies.is_active(Policy::Rationing));
/// ```
/// Resource tracking active colony policies/edicts and their states.
///
/// # Examples
///
/// ```
/// use scale::layer1::administration::edicts::{ColonyPolicies, Policy};
///
/// let mut policies = ColonyPolicies::default();
/// policies.toggle(Policy::Rationing);
/// assert!(policies.is_active(Policy::Rationing));
/// ```
pub struct ColonyPolicies {
    /// Set of currently active policies.
    pub active_policies: HashSet<Policy>,
    /// A map of all policies and their current state information.
    /// A map of all policies and their current state information.
    /// A map of all policies and their current state information.
    /// A map of all policies and their current state information.
    pub policy_states: std::collections::HashMap<Policy, PolicyState>,
    /// Set of policies that have become orphaned and cannot be normally removed.
    pub orphaned_policies: HashSet<Policy>,
}

/// Available policies that can be enacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Policy {
    /// Reduces hunger decay but lowers morale.
    Rationing,
    /// Increases work speed but lowers morale.
    DoubleShifts,
    /// Reduces vermin growth but lowers work speed.
    PestControl,
    /// Bans a specific resource, making it contraband.
    Prohibition(ResourceType),
    /// Blocks Remote Bonds and Intel gain from Subspace Pen Pals.
    FirewallComms,
    /// Spec 1026: Sacrifices pops to reduce resource strain
    TheLottery,
    /// Issues a Placebo Protocol to temporarily reduce stress.
    Placebo(PlaceboProtocol),
    /// Censors delayed broadcasts, preventing large morale swings but increasing distrust.
    CensorBroadcasts,
    /// Automated systems target and destroy any infected individuals.
    ShootInfected,
    /// Halts heavy industry when pollution gets too high, enacted by orbital elites.
    Aesthetic,
    /// Offers amnesty visas to pirate fleets.
    AmnestyVisa,
    /// Declares martial law, restricting movement and increasing security.
    /// Declares martial law, restricting movement and increasing security.
    /// Declares martial law, restricting movement and increasing security.
    /// Declares martial law, restricting movement and increasing security.
    MartialLaw,
    /// Processes dead pops into VitalOrgans.
    MandatoryOrganHarvesting,
    /// Spec 323 The Informant's Dilemma: pays pops a credit bounty for
    /// reporting dissent — crushes unrest, breeds colony-wide paranoia.
    CitizenInformant,
}

#[derive(Event, Debug)]
/// Event triggered to toggle the active status of a policy.
/// Event triggered to toggle the active status of a policy.
/// Event triggered to toggle the active status of a policy.
/// Event triggered to toggle the active status of a policy.
pub struct TogglePolicyEvent(pub Policy);

#[derive(Event, Debug)]
/// Event triggered when a policy action is denied due to insufficient access.
/// Event triggered when a policy action is denied due to insufficient access.
/// Event triggered when a policy action is denied due to insufficient access.
/// Event triggered when a policy action is denied due to insufficient access.
pub struct AccessDeniedEvent {
    /// The reason why access was denied.
    /// The reason why access was denied.
    /// The reason why access was denied.
    /// The reason why access was denied.
    pub reason: String,
}

#[derive(Event, Debug, Clone)]
/// Event triggered to forcefully revoke an active policy.
/// Event triggered to forcefully revoke an active policy.
/// Event triggered to forcefully revoke an active policy.
/// Event triggered to forcefully revoke an active policy.
pub struct RevokePolicyEvent {
    /// The policy to revoke.
    /// The policy to revoke.
    /// The policy to revoke.
    /// The policy to revoke.
    pub policy: Policy,
}

#[derive(Event, Debug)]
/// Event triggered when attempting to hack the central hub to force a policy change.
/// Event triggered when attempting to hack the central hub to force a policy change.
/// Event triggered when attempting to hack the central hub to force a policy change.
/// Event triggered when attempting to hack the central hub to force a policy change.
pub struct HackCentralHubEvent {
    /// The policy targeted by the hack.
    /// The policy targeted by the hack.
    /// The policy targeted by the hack.
    /// The policy targeted by the hack.
    pub target_policy: Policy,
}

impl ColonyPolicies {
    /// Toggles the state of a policy.
    pub fn toggle(&mut self, policy: Policy) {
        if self.active_policies.contains(&policy) {
            self.active_policies.remove(&policy);
            self.policy_states.remove(&policy);
        } else {
            self.active_policies.insert(policy);
            self.policy_states.insert(
                policy,
                PolicyState {
                    active: true,
                    duration: 0,
                    is_tradition: false,
                },
            );
        }
    }

    /// Checks if a policy is currently active.
    #[must_use]
    pub fn is_active(&self, policy: Policy) -> bool {
        self.active_policies.contains(&policy)
    }
}

/// Periodically updates policy durations and entrenches long-running ones as traditions.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{update_policy_tradition_system, ColonyPolicies, Policy, PolicyState};
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.active_policies.insert(Policy::Rationing);
/// policies.policy_states.insert(Policy::Rationing, PolicyState { active: true, duration: 9999, is_tradition: false });
/// world.insert_resource(policies);
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(update_policy_tradition_system);
/// schedule.run(&mut world);
///
/// let p = world.resource::<ColonyPolicies>();
/// assert!(p.policy_states.get(&Policy::Rationing).unwrap().is_tradition);
/// ```
/// Periodically updates policy durations and entrenches long-running ones as traditions.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{update_policy_tradition_system, ColonyPolicies, Policy, PolicyState};
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.active_policies.insert(Policy::Rationing);
/// policies.policy_states.insert(Policy::Rationing, PolicyState { active: true, duration: 9999, is_tradition: false });
/// world.insert_resource(policies);
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(update_policy_tradition_system);
/// schedule.run(&mut world);
///
/// let p = world.resource::<ColonyPolicies>();
/// assert!(p.policy_states.get(&Policy::Rationing).unwrap().is_tradition);
/// ```
/// Periodically updates policy durations and entrenches long-running ones as traditions.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{update_policy_tradition_system, ColonyPolicies, Policy, PolicyState};
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.active_policies.insert(Policy::Rationing);
/// policies.policy_states.insert(Policy::Rationing, PolicyState { active: true, duration: 9999, is_tradition: false });
/// world.insert_resource(policies);
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(update_policy_tradition_system);
/// schedule.run(&mut world);
///
/// let p = world.resource::<ColonyPolicies>();
/// assert!(p.policy_states.get(&Policy::Rationing).unwrap().is_tradition);
/// ```
/// Periodically updates policy durations and entrenches long-running ones as traditions.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{update_policy_tradition_system, ColonyPolicies, Policy, PolicyState};
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.active_policies.insert(Policy::Rationing);
/// policies.policy_states.insert(Policy::Rationing, PolicyState { active: true, duration: 9999, is_tradition: false });
/// world.insert_resource(policies);
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(update_policy_tradition_system);
/// schedule.run(&mut world);
///
/// let p = world.resource::<ColonyPolicies>();
/// assert!(p.policy_states.get(&Policy::Rationing).unwrap().is_tradition);
/// ```
pub fn update_policy_tradition_system(mut policies: ResMut<ColonyPolicies>) {
    for state in policies.policy_states.values_mut() {
        if state.active {
            state.duration += 1;
            if state.duration >= 100 {
                state.is_tradition = true;
            }
        }
    }
}

/// Handles events to forcefully revoke an active policy.
///
/// If the policy is a tradition, revoking it will cause unrest.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_revoke_policy_system, RevokePolicyEvent, ColonyPolicies, Policy, PolicyState};
/// use scale::layer1::social::unrest::Unrest;
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.active_policies.insert(Policy::Rationing);
/// policies.policy_states.insert(Policy::Rationing, PolicyState { active: true, duration: 0, is_tradition: false });
/// world.insert_resource(policies);
/// world.init_resource::<Unrest>();
/// world.init_resource::<Events<RevokePolicyEvent>>();
/// world.send_event(RevokePolicyEvent { policy: Policy::Rationing });
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_revoke_policy_system);
/// schedule.run(&mut world);
/// ```
/// Handles events to forcefully revoke an active policy.
///
/// If the policy is a tradition, revoking it will cause unrest.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_revoke_policy_system, RevokePolicyEvent, ColonyPolicies, Policy, PolicyState};
/// use scale::layer1::social::unrest::Unrest;
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.active_policies.insert(Policy::Rationing);
/// policies.policy_states.insert(Policy::Rationing, PolicyState { active: true, duration: 0, is_tradition: false });
/// world.insert_resource(policies);
/// world.init_resource::<Unrest>();
/// world.init_resource::<Events<RevokePolicyEvent>>();
/// world.send_event(RevokePolicyEvent { policy: Policy::Rationing });
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_revoke_policy_system);
/// schedule.run(&mut world);
/// ```
/// Handles events to forcefully revoke an active policy.
///
/// If the policy is a tradition, revoking it will cause unrest.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_revoke_policy_system, RevokePolicyEvent, ColonyPolicies, Policy, PolicyState};
/// use scale::layer1::social::unrest::Unrest;
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.active_policies.insert(Policy::Rationing);
/// policies.policy_states.insert(Policy::Rationing, PolicyState { active: true, duration: 0, is_tradition: false });
/// world.insert_resource(policies);
/// world.init_resource::<Unrest>();
/// world.init_resource::<Events<RevokePolicyEvent>>();
/// world.send_event(RevokePolicyEvent { policy: Policy::Rationing });
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_revoke_policy_system);
/// schedule.run(&mut world);
/// ```
/// Handles events to forcefully revoke an active policy.
///
/// If the policy is a tradition, revoking it will cause unrest.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_revoke_policy_system, RevokePolicyEvent, ColonyPolicies, Policy, PolicyState};
/// use scale::layer1::social::unrest::Unrest;
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.active_policies.insert(Policy::Rationing);
/// policies.policy_states.insert(Policy::Rationing, PolicyState { active: true, duration: 0, is_tradition: false });
/// world.insert_resource(policies);
/// world.init_resource::<Unrest>();
/// world.init_resource::<Events<RevokePolicyEvent>>();
/// world.send_event(RevokePolicyEvent { policy: Policy::Rationing });
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_revoke_policy_system);
/// schedule.run(&mut world);
/// ```
pub fn handle_revoke_policy_system(
    mut events: EventReader<RevokePolicyEvent>,
    mut policies: ResMut<ColonyPolicies>,
    mut unrest: ResMut<crate::layer1::social::unrest::Unrest>,
) {
    for ev in events.read() {
        if let Some(state) = policies.policy_states.get(&ev.policy) {
            if state.is_tradition {
                unrest
                    .modifiers
                    .push(crate::layer1::social::unrest::UnrestModifier {
                        value: 0.5,
                        duration: 500,
                        label: "Revoked Tradition".to_string(),
                    });
            }
        }
        policies.active_policies.remove(&ev.policy);
        policies.policy_states.remove(&ev.policy);
    }
}

/// Handles events to toggle the active status of a policy.
///
/// If a policy is orphaned, it cannot be toggled normally and will emit an `AccessDeniedEvent`.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_policy_toggle_system, TogglePolicyEvent, AccessDeniedEvent, ColonyPolicies, Policy};
///
/// let mut world = World::new();
/// world.init_resource::<ColonyPolicies>();
/// world.init_resource::<Events<TogglePolicyEvent>>();
/// world.init_resource::<Events<AccessDeniedEvent>>();
/// world.send_event(TogglePolicyEvent(Policy::DoubleShifts));
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_policy_toggle_system);
/// schedule.run(&mut world);
/// ```
/// Handles events to toggle the active status of a policy.
///
/// If a policy is orphaned, it cannot be toggled normally and will emit an `AccessDeniedEvent`.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_policy_toggle_system, TogglePolicyEvent, AccessDeniedEvent, ColonyPolicies, Policy};
///
/// let mut world = World::new();
/// world.init_resource::<ColonyPolicies>();
/// world.init_resource::<Events<TogglePolicyEvent>>();
/// world.init_resource::<Events<AccessDeniedEvent>>();
/// world.send_event(TogglePolicyEvent(Policy::DoubleShifts));
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_policy_toggle_system);
/// schedule.run(&mut world);
/// ```
/// Handles events to toggle the active status of a policy.
///
/// If a policy is orphaned, it cannot be toggled normally and will emit an `AccessDeniedEvent`.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_policy_toggle_system, TogglePolicyEvent, AccessDeniedEvent, ColonyPolicies, Policy};
///
/// let mut world = World::new();
/// world.init_resource::<ColonyPolicies>();
/// world.init_resource::<Events<TogglePolicyEvent>>();
/// world.init_resource::<Events<AccessDeniedEvent>>();
/// world.send_event(TogglePolicyEvent(Policy::DoubleShifts));
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_policy_toggle_system);
/// schedule.run(&mut world);
/// ```
/// Handles events to toggle the active status of a policy.
///
/// If a policy is orphaned, it cannot be toggled normally and will emit an `AccessDeniedEvent`.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_policy_toggle_system, TogglePolicyEvent, AccessDeniedEvent, ColonyPolicies, Policy};
///
/// let mut world = World::new();
/// world.init_resource::<ColonyPolicies>();
/// world.init_resource::<Events<TogglePolicyEvent>>();
/// world.init_resource::<Events<AccessDeniedEvent>>();
/// world.send_event(TogglePolicyEvent(Policy::DoubleShifts));
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_policy_toggle_system);
/// schedule.run(&mut world);
/// ```
pub fn handle_policy_toggle_system(
    mut events: EventReader<TogglePolicyEvent>,
    mut policies: ResMut<ColonyPolicies>,
    mut access_denied: EventWriter<AccessDeniedEvent>,
) {
    for ev in events.read() {
        let policy = ev.0;
        if policies.orphaned_policies.contains(&policy) {
            access_denied.send(AccessDeniedEvent {
                reason: format!("Edict {:?} is orphaned and cannot be toggled", policy),
            });
        } else {
            policies.toggle(policy);
        }
    }
}

/// Handles events to hack the central hub and clear orphaned policies.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_hack_hub_system, HackCentralHubEvent, ColonyPolicies, Policy};
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.orphaned_policies.insert(Policy::Rationing);
/// world.insert_resource(policies);
/// world.init_resource::<Events<HackCentralHubEvent>>();
/// world.send_event(HackCentralHubEvent { target_policy: Policy::Rationing });
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_hack_hub_system);
/// schedule.run(&mut world);
/// ```
/// Handles events to hack the central hub and clear orphaned policies.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_hack_hub_system, HackCentralHubEvent, ColonyPolicies, Policy};
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.orphaned_policies.insert(Policy::Rationing);
/// world.insert_resource(policies);
/// world.init_resource::<Events<HackCentralHubEvent>>();
/// world.send_event(HackCentralHubEvent { target_policy: Policy::Rationing });
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_hack_hub_system);
/// schedule.run(&mut world);
/// ```
/// Handles events to hack the central hub and clear orphaned policies.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_hack_hub_system, HackCentralHubEvent, ColonyPolicies, Policy};
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.orphaned_policies.insert(Policy::Rationing);
/// world.insert_resource(policies);
/// world.init_resource::<Events<HackCentralHubEvent>>();
/// world.send_event(HackCentralHubEvent { target_policy: Policy::Rationing });
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_hack_hub_system);
/// schedule.run(&mut world);
/// ```
/// Handles events to hack the central hub and clear orphaned policies.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::administration::edicts::{handle_hack_hub_system, HackCentralHubEvent, ColonyPolicies, Policy};
///
/// let mut world = World::new();
/// let mut policies = ColonyPolicies::default();
/// policies.orphaned_policies.insert(Policy::Rationing);
/// world.insert_resource(policies);
/// world.init_resource::<Events<HackCentralHubEvent>>();
/// world.send_event(HackCentralHubEvent { target_policy: Policy::Rationing });
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(handle_hack_hub_system);
/// schedule.run(&mut world);
/// ```
pub fn handle_hack_hub_system(
    mut events: EventReader<HackCentralHubEvent>,
    mut policies: ResMut<ColonyPolicies>,
) {
    for ev in events.read() {
        let policy = ev.target_policy;
        policies.orphaned_policies.remove(&policy);
        policies.active_policies.remove(&policy);
    }
}

pub fn hack_hub_chronicle_bridge(
    mut events: bevy_ecs::prelude::EventReader<HackCentralHubEvent>,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Standard,
            text: format!(
                "A successful hack into the central hub has removed the orphaned {:?} edict.",
                event.target_policy
            ),
        });
    }
}

/// Calculates the global hunger decay modifier based on active policies.
///
///
/// # Examples
/// ```
/// use scale::layer1::administration::edicts::{ColonyPolicies, Policy, get_hunger_decay_modifier};
/// let mut policies = ColonyPolicies::default();
/// policies.toggle(Policy::Rationing);
/// assert_eq!(get_hunger_decay_modifier(&policies), 0.5);
/// ```
///
#[must_use]
pub fn get_hunger_decay_modifier(policies: &ColonyPolicies) -> f32 {
    if policies.is_active(Policy::Rationing) {
        0.5
    } else {
        1.0
    }
}

/// Calculates the global work speed modifier based on active policies.
///
///
/// # Examples
/// ```
/// use scale::layer1::administration::edicts::{ColonyPolicies, Policy, get_work_speed_modifier};
/// let mut policies = ColonyPolicies::default();
/// policies.toggle(Policy::DoubleShifts);
/// assert_eq!(get_work_speed_modifier(&policies), 1.2);
/// ```
///
#[must_use]
pub fn get_work_speed_modifier(policies: &ColonyPolicies) -> f32 {
    let mut modifier = 1.0;
    if policies.is_active(Policy::DoubleShifts) {
        modifier += 0.2;
    }
    if policies.is_active(Policy::PestControl) {
        modifier -= 0.05;
    }
    modifier
}

/// Returns the flat morale modifier.
///
/// * `Rationing`: -0.1 Morale.
/// * `DoubleShifts`: -0.15 Morale.
#[must_use]
pub fn get_morale_modifier(policies: &ColonyPolicies) -> f32 {
    let mut modifier = 0.0;
    if policies.is_active(Policy::Rationing) {
        modifier -= 0.1;
    }
    if policies.is_active(Policy::DoubleShifts) {
        modifier -= 0.15;
    }
    modifier
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::needs::Needs;
    use crate::layer1::GridPosition;

    #[test]
    fn test_policy_gains_tradition_xp_over_time() {
        let mut app = bevy_app::App::new();

        let mut policies = ColonyPolicies::default();
        policies.active_policies.insert(Policy::Rationing);
        policies.policy_states.insert(
            Policy::Rationing,
            PolicyState {
                active: true,
                duration: 0,
                is_tradition: false,
            },
        );
        app.insert_resource(policies);

        app.add_systems(bevy_app::Update, update_policy_tradition_system);
        app.update();

        let policies = app.world().resource::<ColonyPolicies>();
        let state = policies.policy_states.get(&Policy::Rationing).unwrap();
        assert_eq!(
            state.duration, 1,
            "Policy duration should increase per tick"
        );
    }

    #[test]
    fn test_policy_becomes_tradition_after_duration() {
        let mut app = bevy_app::App::new();

        let mut policies = ColonyPolicies::default();
        policies.active_policies.insert(Policy::Rationing);
        policies.policy_states.insert(
            Policy::Rationing,
            PolicyState {
                active: true,
                duration: 99,
                is_tradition: false,
            },
        );
        app.insert_resource(policies);

        app.add_systems(bevy_app::Update, update_policy_tradition_system);
        app.update(); // Tick 100

        let policies = app.world().resource::<ColonyPolicies>();
        let state = policies.policy_states.get(&Policy::Rationing).unwrap();
        assert!(
            state.is_tradition,
            "Policy should become a tradition after hitting duration threshold"
        );
    }

    #[test]
    fn test_revoking_tradition_causes_unrest() {
        let mut app = bevy_app::App::new();

        let mut policies = ColonyPolicies::default();
        policies.active_policies.insert(Policy::MartialLaw);
        policies.policy_states.insert(
            Policy::MartialLaw,
            PolicyState {
                active: true,
                duration: 150,
                is_tradition: true,
            },
        );
        app.insert_resource(policies);

        app.insert_resource(crate::layer1::social::unrest::Unrest::default());
        app.init_resource::<bevy_ecs::event::Events<RevokePolicyEvent>>();

        app.add_systems(bevy_app::Update, handle_revoke_policy_system);

        let _initial_unrest = app
            .world()
            .resource::<crate::layer1::social::unrest::Unrest>()
            .level;

        // Revoke the tradition
        app.world_mut()
            .resource_mut::<bevy_ecs::event::Events<RevokePolicyEvent>>()
            .send(RevokePolicyEvent {
                policy: Policy::MartialLaw,
            });

        app.update();

        let unrest = app
            .world()
            .resource::<crate::layer1::social::unrest::Unrest>();
        assert!(
            !unrest.modifiers.is_empty(),
            "Revoking a tradition should heavily increase unrest"
        );

        let policies = app.world().resource::<ColonyPolicies>();
        assert!(
            !policies.active_policies.contains(&Policy::MartialLaw),
            "Policy should be revoked"
        );
    }

    #[test]
    fn test_policies_resource_defaults() {
        let policies = ColonyPolicies::default();
        assert!(policies.active_policies.is_empty());
    }

    #[test]
    fn test_toggle_policy() {
        let mut policies = ColonyPolicies::default();

        // Enable Rationing
        policies.toggle(Policy::Rationing);
        assert!(
            policies.is_active(Policy::Rationing),
            "Rationing should be active after toggle"
        );

        // Disable Rationing
        policies.toggle(Policy::Rationing);
        assert!(
            !policies.is_active(Policy::Rationing),
            "Rationing should be inactive after toggle"
        );
    }

    #[test]
    fn test_rationing_reduces_hunger_decay() {
        let mut world = World::new();
        let mut policies = ColonyPolicies::default();
        policies.toggle(Policy::Rationing);
        world.insert_resource(policies);

        // Spawn a pop with standard needs
        let _pop = world
            .spawn((
                Needs {
                    hunger: 1.0,
                    ..Default::default()
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        let modifier = crate::layer1::administration::edicts::get_hunger_decay_modifier(
            world.resource::<ColonyPolicies>(),
        );
        assert!(
            modifier < 1.0,
            "Hunger decay modifier should be < 1.0 with Rationing"
        );
        assert!(
            (modifier - 0.5).abs() < f32::EPSILON,
            "Hunger decay should be 0.5"
        );
    }

    #[test]
    fn test_double_shifts_increases_speed_and_reduces_morale() {
        let mut world = World::new();
        let mut policies = ColonyPolicies::default();
        policies.toggle(Policy::DoubleShifts);
        world.insert_resource(policies);

        // Check modifiers
        let speed_mod = crate::layer1::administration::edicts::get_work_speed_modifier(
            world.resource::<ColonyPolicies>(),
        );
        assert!(
            speed_mod > 1.0,
            "Work speed modifier should be > 1.0 with DoubleShifts"
        );

        let morale_mod = crate::layer1::administration::edicts::get_morale_modifier(
            world.resource::<ColonyPolicies>(),
        );
        assert!(
            morale_mod < 0.0,
            "Morale modifier should be negative with DoubleShifts"
        );
    }

    #[test]
    fn test_pest_control_reduces_work_speed() {
        let mut policies = ColonyPolicies::default();
        policies.toggle(Policy::PestControl);
        // Should reduce speed by 0.05
        let modifier = get_work_speed_modifier(&policies);
        assert!((modifier - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn test_player_cannot_rescind_orphaned_edict() {
        // Arrange: Player attempts to toggle the edict off
        let mut app = bevy_app::App::new();
        app.add_event::<TogglePolicyEvent>();
        app.add_event::<AccessDeniedEvent>();

        let mut policies = ColonyPolicies::default();
        policies.active_policies.insert(Policy::ShootInfected);
        policies.orphaned_policies.insert(Policy::ShootInfected);
        app.insert_resource(policies);

        app.add_systems(bevy_app::Update, handle_policy_toggle_system);

        // Act: Send UI/Input event to disable the edict
        app.world_mut()
            .send_event(TogglePolicyEvent(Policy::ShootInfected));
        app.update();

        // Assert: The edict remains active, and an 'AccessDenied' event is logged.
        let policies = app.world().resource::<ColonyPolicies>();
        assert!(policies.is_active(Policy::ShootInfected));

        let events = app.world().resource::<Events<AccessDeniedEvent>>();
        let mut cursor = events.get_cursor();
        assert!(cursor.read(events).next().is_some());
    }

    #[test]
    fn test_resolving_orphaned_edict_via_bureaucratic_hack() {
        // Arrange: App with orphaned edict
        let mut app = bevy_app::App::new();
        app.add_event::<HackCentralHubEvent>();

        let mut policies = ColonyPolicies::default();
        policies.active_policies.insert(Policy::ShootInfected);
        policies.orphaned_policies.insert(Policy::ShootInfected);
        app.insert_resource(policies);

        app.add_systems(bevy_app::Update, handle_hack_hub_system);

        // Act: Perform a 'HackCentralHub' action
        app.world_mut().send_event(HackCentralHubEvent {
            target_policy: Policy::ShootInfected,
        });
        app.update();

        // Assert: The edict is finally removed from the active edicts list.
        let policies = app.world().resource::<ColonyPolicies>();
        assert!(!policies.is_active(Policy::ShootInfected));
        assert!(!policies.orphaned_policies.contains(&Policy::ShootInfected));
    }
}
