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
    /// Spec 393: Double Rations — boosts morale but doubles food
    /// consumption (hunger decay x2). The generous mirror of Rationing.
    DoubleRations,
    /// Increases work speed but lowers morale.
    DoubleShifts,
    /// Spec 393: Night Shifts — work speed up, rest burns faster,
    /// morale slightly down. The colony keeps the lights on all night.
    NightShifts,
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
    /// Spec 387: Safety Protocols — global edict: hazard risk x0.5 colony-wide, work speed -0.10.
    SafetyProtocols,
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

    /// Spec 393: idempotently enact a policy (no-op if already active).
    /// The spec's sketch API (`ActiveEdicts::enable`) on the real
    /// 054 resource — see the `ActiveEdicts`/`EdictType` aliases below.
    pub fn enable(&mut self, policy: Policy) {
        if !self.is_active(policy) {
            self.toggle(policy);
        }
    }

    /// Spec 393: idempotently repeal a policy (no-op if not active).
    pub fn disable(&mut self, policy: Policy) {
        if self.is_active(policy) {
            self.toggle(policy);
        }
    }
}

/// Spec 393 name for the policy enum. The spec sketch introduced
/// `EdictType` as a new enum; in this codebase the 054 `Policy` enum
/// already plays that role, so this is an alias, not a parallel type.
pub type EdictType = Policy;

/// Spec 393 name for the active-edicts resource. Aliases the real
/// 054 `ColonyPolicies` resource — there is exactly one edict
/// resource in the simulation.
pub type ActiveEdicts = ColonyPolicies;

/// The three edicts spec 393 names, in menu order.
pub const SPEC_393_EDICTS: [Policy; 3] = [
    Policy::DoubleRations,
    Policy::MartialLaw,
    Policy::NightShifts,
];

/// Morale-modifier label for Double Rations (refresh-never-stack lane).
pub const DOUBLE_RATIONS_LABEL: &str = "Edict: Double Rations";
/// Morale-modifier label for Night Shifts (refresh-never-stack lane).
pub const NIGHT_SHIFTS_LABEL: &str = "Edict: Night Shifts";
/// Double Rations morale bonus (0.0..=1.0 morale scale; the spec
/// sketch's +10 on its mock 0..100 Mood scale).
pub const DOUBLE_RATIONS_MORALE_BONUS: f32 = 0.10;
/// Double Rations hunger-decay multiplier: pops eat twice as fast.
pub const DOUBLE_RATIONS_HUNGER_MULTIPLIER: f32 = 2.0;
/// Martial Law liberty penalty, subtracted from a pop's liberty base
/// (0..100 scale) for as long as the edict is active.
pub const MARTIAL_LAW_LIBERTY_PENALTY: f32 = 20.0;
/// Night Shifts work-speed bonus (additive, like DoubleShifts' +0.2).
pub const NIGHT_SHIFTS_WORK_SPEED_BONUS: f32 = 0.15;
/// Night Shifts rest-decay multiplier: night work burns rest faster.
pub const NIGHT_SHIFTS_REST_MULTIPLIER: f32 = 1.5;
/// Night Shifts flat morale modifier (0.0..=1.0 scale).
pub const NIGHT_SHIFTS_MORALE_MODIFIER: f32 = -0.05;
/// Duration of refreshed edict mood modifiers (matches the Spec 359
/// night-shift bridge's refresh pattern).
pub const EDICT_MODIFIER_DURATION: u32 = 100;

/// Per-pop liberty (Spec 393). The spec sketch had a bare
/// `Liberty(pub f32)` that its GREEN code decremented every tick —
/// permanent accumulation. This component instead stores the pop's
/// `base` alongside the derived `current`: `apply_edict_modifiers`
/// recomputes `current` from `base` every tick, so repealing Martial
/// Law restores liberty in full on the next tick.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Liberty {
    /// The pop's liberty with no edicts in force (0..100).
    pub base: f32,
    /// Effective liberty after active edicts (derived, 0..=base).
    pub current: f32,
}

impl Default for Liberty {
    fn default() -> Self {
        Self {
            base: 100.0,
            current: 100.0,
        }
    }
}

/// Parses a console/menu edict name into a spec-393 edict, if it is
/// one. Accepts snake_case and concatenated spellings.
#[must_use]
pub fn parse_edict(name: &str) -> Option<Policy> {
    match name
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "double_rations" | "doublerations" => Some(Policy::DoubleRations),
        "martial_law" | "martiallaw" | "martial" => Some(Policy::MartialLaw),
        "night_shifts" | "nightshifts" | "night_shift" | "nightshift" => {
            Some(Policy::NightShifts)
        }
        _ => None,
    }
}

/// Short snake_case name for a spec-393 edict (`"other"` for the
/// older 054-era policies, which predate this naming lane).
#[must_use]
pub fn edict_short_name(policy: Policy) -> &'static str {
    match policy {
        Policy::DoubleRations => "double_rations",
        Policy::MartialLaw => "martial_law",
        Policy::NightShifts => "night_shifts",
        _ => "other",
    }
}

/// Whether active edicts suppress colony unrest (Spec 393: Martial Law).
#[must_use]
pub fn is_unrest_suppressed(policies: &ColonyPolicies) -> bool {
    policies.is_active(Policy::MartialLaw)
}

/// Rest-decay multiplier from active policies (Spec 393: Night Shifts
/// burn rest at 1.5x). Pull-based: `decay_needs_system` recomputes
/// from the base rate every tick.
#[must_use]
pub fn get_rest_decay_modifier(policies: &ColonyPolicies) -> f32 {
    if policies.is_active(Policy::NightShifts) {
        NIGHT_SHIFTS_REST_MULTIPLIER
    } else {
        1.0
    }
}

fn refresh_edict_modifier(
    morale: &mut crate::layer1::social::morale::Morale,
    label: &str,
    value: f32,
    active: bool,
) {
    if !active {
        morale.modifiers.retain(|m| m.label != label);
        return;
    }
    if let Some(existing) = morale.modifiers.iter_mut().find(|m| m.label == label) {
        existing.value = value;
        existing.duration = EDICT_MODIFIER_DURATION;
    } else {
        morale.add_modifier(crate::layer1::social::morale::MoodModifier {
            label: label.to_string(),
            value,
            duration: EDICT_MODIFIER_DURATION,
        });
    }
}

/// Spec 393's `apply_edict_modifiers`, adapted to real types.
///
/// Non-accumulation contract (the spec's §7 guidance): every effect
/// here is *derived* — Morale effects ride the refresh-never-stack
/// MoodModifier lane (exactly one modifier per edict, removed the
/// tick the edict repeals), Liberty is recomputed from its stored
/// base, and the global Unrest level is forced to 0 only for as long
/// as Martial Law is active (the authoritative suppression also
/// lives in `calculate_unrest_system`, which recomputes the level
/// from scratch each tick). Hunger/rest/work-speed effects are
/// pull-based modifier functions consumed by their own systems and
/// never touch per-pop state at all.
pub fn apply_edict_modifiers(
    policies: Res<ColonyPolicies>,
    unrest: Option<ResMut<crate::layer1::social::unrest::Unrest>>,
    mut query: Query<(
        &mut crate::layer1::social::morale::Morale,
        Option<&mut Liberty>,
    )>,
) {
    if let Some(mut unrest) = unrest {
        if is_unrest_suppressed(&policies) {
            unrest.level = 0.0;
        }
    }
    let double_rations = policies.is_active(Policy::DoubleRations);
    let night_shifts = policies.is_active(Policy::NightShifts);
    let martial_law = policies.is_active(Policy::MartialLaw);
    for (mut morale, liberty) in query.iter_mut() {
        refresh_edict_modifier(
            &mut morale,
            DOUBLE_RATIONS_LABEL,
            DOUBLE_RATIONS_MORALE_BONUS,
            double_rations,
        );
        refresh_edict_modifier(
            &mut morale,
            NIGHT_SHIFTS_LABEL,
            NIGHT_SHIFTS_MORALE_MODIFIER,
            night_shifts,
        );
        if let Some(mut liberty) = liberty {
            let penalty = if martial_law {
                MARTIAL_LAW_LIBERTY_PENALTY
            } else {
                0.0
            };
            liberty.current = (liberty.base - penalty).clamp(0.0, liberty.base);
        }
    }
}

/// Inserts a [`Liberty`] tracker on any pop missing one (the
/// total-coverage ensure pattern used by Spec 359's night-shift
/// adaptation), so the derived-liberty lane covers pops spawned by
/// any path, not just `PopBundle`.
pub fn ensure_liberty_system(
    mut commands: Commands,
    query: Query<Entity, (With<crate::layer1::pop::Pop>, Without<Liberty>)>,
) {
    for entity in query.iter() {
        commands.entity(entity).insert(Liberty::default());
    }
}

/// Mean effective liberty across pops carrying a [`Liberty`]
/// tracker (100.0 when no pop carries one yet). Headless/STATS lane.
#[must_use]
pub fn average_liberty(world: &mut World) -> f32 {
    let mut query = world.query::<&Liberty>();
    let (mut sum, mut count) = (0.0f32, 0u32);
    for liberty in query.iter(world) {
        sum += liberty.current;
        count += 1;
    }
    if count == 0 {
        100.0
    } else {
        sum / count as f32
    }
}

/// STATS label for the spec-393 edicts: active short names joined
/// with `+`, or `"none"`. Headless/STATS lane.
#[must_use]
pub fn edicts_stats_label(world: &mut World) -> String {
    let Some(policies) = world.get_resource::<ColonyPolicies>() else {
        return "none".to_string();
    };
    let active: Vec<&str> = SPEC_393_EDICTS
        .iter()
        .filter(|p| policies.is_active(**p))
        .map(|p| edict_short_name(*p))
        .collect();
    if active.is_empty() {
        "none".to_string()
    } else {
        active.join("+")
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
    // Spec 393: multiplicative composition — Rationing (x0.5) and
    // Double Rations (x2.0) cancel out when both are somehow active.
    let mut modifier = 1.0;
    if policies.is_active(Policy::Rationing) {
        modifier *= 0.5;
    }
    if policies.is_active(Policy::DoubleRations) {
        modifier *= DOUBLE_RATIONS_HUNGER_MULTIPLIER;
    }
    modifier
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
/// Hazard-risk multiplier from active policies.
/// Spec 387: Safety Protocols multiplies all hazard risk by 0.5 —
/// consumed by the per-action hazard lane in `general_work.rs`
/// (passed as `hazard_modifier` to `handle_workplace_hazards`) and
/// mirrored for the building-level lane in
/// `workplace_hazards::safety_risk_multiplier`.
#[must_use]
pub fn get_hazard_modifier(policies: &ColonyPolicies) -> f64 {
    if policies.is_active(Policy::SafetyProtocols) {
        0.5
    } else {
        1.0
    }
}

#[must_use]
pub fn get_work_speed_modifier(policies: &ColonyPolicies) -> f32 {
    let mut modifier = 1.0;
    if policies.is_active(Policy::DoubleShifts) {
        modifier += 0.2;
    }
    if policies.is_active(Policy::NightShifts) {
        modifier += NIGHT_SHIFTS_WORK_SPEED_BONUS;
    }
    if policies.is_active(Policy::PestControl) {
        modifier -= 0.05;
    }
    if policies.is_active(Policy::SafetyProtocols) {
        modifier -= 0.10;
    }
    modifier
}

/// Returns the flat morale modifier.
///
/// * `Rationing`: -0.1 Morale.
/// * `DoubleShifts`: -0.15 Morale.
/// * `DoubleRations` (Spec 393): +0.1 Morale (well-fed pops).
/// * `NightShifts` (Spec 393): -0.05 Morale.
#[must_use]
pub fn get_morale_modifier(policies: &ColonyPolicies) -> f32 {
    let mut modifier = 0.0;
    if policies.is_active(Policy::Rationing) {
        modifier -= 0.1;
    }
    if policies.is_active(Policy::DoubleShifts) {
        modifier -= 0.15;
    }
    if policies.is_active(Policy::DoubleRations) {
        modifier += DOUBLE_RATIONS_MORALE_BONUS;
    }
    if policies.is_active(Policy::NightShifts) {
        modifier += NIGHT_SHIFTS_MORALE_MODIFIER;
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

    // ---- Spec 393: Colony Edicts (full implementation) ----
    // Adapted from the spec's mock-style RED sketch to real types:
    // Mood (0..100) -> Morale component (0.0..=1.0 + MoodModifiers),
    // Metabolism -> pull-based get_hunger_decay_modifier consumed by
    // decay_needs_system, per-pop Unrest component -> global Unrest
    // resource recomputed by calculate_unrest_system, Liberty -> new
    // derived Liberty component (base/current, recomputed per tick).

    #[test]
    fn test_393_double_rations_active() {
        let mut app = bevy_app::App::new();

        let mut edicts = ActiveEdicts::default();
        edicts.enable(EdictType::DoubleRations);
        app.insert_resource(edicts);
        app.insert_resource(crate::layer1::social::unrest::Unrest::default());
        app.add_systems(bevy_app::Update, apply_edict_modifiers);

        let pop = app
            .world_mut()
            .spawn((
                crate::layer1::needs::Needs::default(),
                crate::layer1::social::morale::Morale::default(),
                Liberty::default(),
            ))
            .id();

        app.update();

        // Double rations should increase mood (via a refreshed Morale
        // modifier, the codebase's non-accumulating mood lane)...
        let morale = app
            .world()
            .get::<crate::layer1::social::morale::Morale>(pop)
            .unwrap();
        let bonus: f32 = morale
            .modifiers
            .iter()
            .filter(|m| m.label == DOUBLE_RATIONS_LABEL)
            .map(|m| m.value)
            .sum();
        assert!(
            bonus > 0.0,
            "Double Rations should boost mood via a Morale modifier"
        );

        // ...and increase hunger drain (food consumption rate x2).
        let policies = app.world().resource::<ColonyPolicies>();
        assert!(
            get_hunger_decay_modifier(policies) > 1.0,
            "Double Rations should double food consumption rate"
        );
        assert!((get_hunger_decay_modifier(policies) - 2.0).abs() < f32::EPSILON);
        assert!(get_morale_modifier(policies) > 0.0);
    }

    #[test]
    fn test_393_martial_law_suppresses_unrest() {
        let mut app = bevy_app::App::new();

        let mut edicts = ActiveEdicts::default();
        edicts.enable(EdictType::MartialLaw);
        app.insert_resource(edicts);
        app.insert_resource(crate::layer1::social::unrest::Unrest {
            level: 0.5,
            modifiers: Vec::new(),
        });
        app.add_systems(
            bevy_app::Update,
            (
                apply_edict_modifiers,
                crate::layer1::social::unrest::calculate_unrest_system
                    .after(apply_edict_modifiers),
            ),
        );

        let pop = app
            .world_mut()
            .spawn((
                crate::layer1::pop::Pop,
                crate::layer1::social::morale::Morale {
                    value: 0.1, // Very low morale, should cause unrest
                    modifiers: Vec::new(),
                },
                Liberty {
                    base: 100.0,
                    current: 100.0,
                },
            ))
            .id();

        app.update();
        app.update();

        // Martial Law suppresses unrest to 0 (even after the unrest
        // system recomputes it from the low average morale)...
        let unrest = app.world().resource::<crate::layer1::social::unrest::Unrest>();
        assert_eq!(unrest.level, 0.0, "Martial Law should reduce Unrest to 0");
        assert!(is_unrest_suppressed(
            app.world().resource::<ColonyPolicies>()
        ));

        // ...but lowers liberty — derived from base, so two ticks of
        // application still read exactly 80, never 60/40/... (the
        // spec sketch's per-tick `-20` would accumulate permanently).
        let liberty = app.world().get::<Liberty>(pop).unwrap();
        assert!(liberty.current < 100.0, "Martial Law should decrease Liberty");
        assert!(
            (liberty.current - 80.0).abs() < f32::EPSILON,
            "Liberty must be recomputed from base, not accumulated: {}",
            liberty.current
        );
    }

    #[test]
    fn test_393_modifiers_do_not_accumulate() {
        let mut app = bevy_app::App::new();
        let mut edicts = ActiveEdicts::default();
        edicts.enable(EdictType::DoubleRations);
        edicts.enable(EdictType::MartialLaw);
        app.insert_resource(edicts);
        app.insert_resource(crate::layer1::social::unrest::Unrest::default());
        app.add_systems(bevy_app::Update, apply_edict_modifiers);

        let pop = app
            .world_mut()
            .spawn((
                crate::layer1::social::morale::Morale::default(),
                Liberty::default(),
            ))
            .id();

        for _ in 0..5 {
            app.update();
        }

        let morale = app
            .world()
            .get::<crate::layer1::social::morale::Morale>(pop)
            .unwrap();
        let ration_mods: Vec<_> = morale
            .modifiers
            .iter()
            .filter(|m| m.label == DOUBLE_RATIONS_LABEL)
            .collect();
        assert_eq!(ration_mods.len(), 1, "modifier must refresh, never stack");
        assert!((ration_mods[0].value - DOUBLE_RATIONS_MORALE_BONUS).abs() < f32::EPSILON);

        let liberty = app.world().get::<Liberty>(pop).unwrap();
        assert!((liberty.current - 80.0).abs() < f32::EPSILON);

        // Turning the edicts off fully restores base values next tick.
        app.world_mut()
            .resource_mut::<ColonyPolicies>()
            .disable(EdictType::DoubleRations);
        app.world_mut()
            .resource_mut::<ColonyPolicies>()
            .disable(EdictType::MartialLaw);
        app.update();

        let morale = app
            .world()
            .get::<crate::layer1::social::morale::Morale>(pop)
            .unwrap();
        assert!(morale
            .modifiers
            .iter()
            .all(|m| m.label != DOUBLE_RATIONS_LABEL));
        let liberty = app.world().get::<Liberty>(pop).unwrap();
        assert!((liberty.current - liberty.base).abs() < f32::EPSILON);
    }

    #[test]
    fn test_393_night_shifts_modifiers() {
        let mut policies = ColonyPolicies::default();
        policies.enable(Policy::NightShifts);
        assert!(get_work_speed_modifier(&policies) > 1.0);
        assert!(get_rest_decay_modifier(&policies) > 1.0);
        assert!(get_morale_modifier(&policies) < 0.0);
    }

    #[test]
    fn test_393_rationing_and_double_rations_cancel_on_hunger() {
        let mut policies = ColonyPolicies::default();
        policies.enable(Policy::Rationing);
        policies.enable(Policy::DoubleRations);
        // 0.5 (rationing) x 2.0 (double rations) = 1.0 baseline drain.
        assert!((get_hunger_decay_modifier(&policies) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_393_enable_disable_idempotent() {
        let mut policies = ActiveEdicts::default();
        policies.enable(EdictType::NightShifts);
        policies.enable(EdictType::NightShifts);
        assert!(policies.is_active(EdictType::NightShifts));
        policies.disable(EdictType::NightShifts);
        policies.disable(EdictType::NightShifts);
        assert!(!policies.is_active(EdictType::NightShifts));
    }

    #[test]
    fn test_393_ensure_liberty_covers_bare_pops() {
        let mut app = bevy_app::App::new();
        app.add_systems(bevy_app::Update, ensure_liberty_system);
        let pop = app.world_mut().spawn(crate::layer1::pop::Pop).id();
        app.update();
        assert!(app.world().get::<Liberty>(pop).is_some());
    }

    #[test]
    fn test_393_parse_edict_names() {
        assert_eq!(parse_edict("double_rations"), Some(Policy::DoubleRations));
        assert_eq!(parse_edict("martial_law"), Some(Policy::MartialLaw));
        assert_eq!(parse_edict("night_shifts"), Some(Policy::NightShifts));
        assert_eq!(parse_edict("bogus"), None);
        for policy in [Policy::DoubleRations, Policy::MartialLaw, Policy::NightShifts] {
            assert_eq!(parse_edict(edict_short_name(policy)), Some(policy));
        }
    }
}
