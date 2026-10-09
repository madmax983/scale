//! # Factions & Guilds
//!
//! Factions represent the organized interests of the colony's workforce.
//! Unlike individual Pops who have personal Needs, Factions care about
//! colony-wide policy and working conditions.
//!
//! ## Core Concepts
//!
//! *   **Guilds:** Pops automatically join a Faction based on their highest [Skill](crate::layer1::skills::SkillType).
//!     *   Miners -> Miners' Guild
//!     *   Farmers -> Growers' Circle
//!     *   Masons -> Masons' Lodge
//!     *   etc.
//!
//! *   **Satisfaction:** A value from 0.0 (Furious) to 1.0 (Content).
//!     *   It is recalculated every tick based on active [Policies](crate::layer1::edicts::Policy).
//!     *   Penalties (like `DoubleShifts`) lower satisfaction.
//!
//! *   **Demands & Strikes:**
//!     *   **Loyal** (> 0.4): The faction is cooperative.
//!     *   **Unhappy** (<= 0.4): The faction issues a [`FactionDemand`] (e.g., "End Double Shifts").
//!     *   **Striking**: If the demand is ignored for too long, the faction enters the [`FactionState::Striking`] state.
//!         Striking pops will refuse to work (checked by the execution system).

use crate::layer1::edicts::Policy;
use crate::layer1::skills::SkillType;
use bevy_ecs::prelude::*;
use std::collections::HashMap;

/// Current political state of a faction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactionState {
    /// Faction is content (Satisfaction > 0.4).
    /// Pops work normally.
    Loyal,
    /// Faction is unhappy (Satisfaction <= 0.4) and has issued a demand.
    /// Pops continue to work, but the clock is ticking.
    Unhappy,
    /// Faction is on strike (Demand ignored/timed out).
    /// Pops in this faction will refuse `Work` actions.
    Striking,
}

/// A specific demand issued by a faction.
///
/// Demands usually require the player to change a specific [`Policy`].
#[derive(Debug, Clone, PartialEq)]
pub struct FactionDemand {
    /// The policy they want changed (usually toggled off if it's a penalty).
    pub policy: Option<Policy>,
    /// Ticks remaining until the faction goes on strike.
    pub remaining_time: f32,
    /// Human-readable description of the demand.
    pub description: String,
}

impl Default for FactionDemand {
    fn default() -> Self {
        Self {
            policy: None,
            remaining_time: 1000.0, // Default duration
            description: "None".into(),
        }
    }
}

/// Unique identifier for each faction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FactionId {
    /// Faction for miners (`SkillType::Mining`).
    MinersGuild,
    /// Faction for farmers (`SkillType::Farming`, `SkillType::Husbandry`).
    FarmersGuild,
    /// Faction for construction workers (`SkillType::Construction`).
    MasonsGuild,
    /// Faction for foresters (`SkillType::Forestry`).
    LoggersGuild,
    /// Faction for crafters (`SkillType::Crafting`).
    ArtisansGuild,
    /// Faction for researchers (`SkillType::Science`).
    ScholarsGuild,
    /// Faction for those with no specific skill focus or balanced skills.
    Unaligned,
    /// Faction for criminals from neighboring empire bailing out debt.
    Cartel,
    /// Cult that worships the deep underground and fears the open sky.
    SubLithic,
    /// Society obsessed with achieving the Culinary Singularity.
    Gastronomers,
    /// Pops living in redlined/dezoned areas who stop paying taxes.
    Stateless,
    /// Hive mind collective that split from the unintegrated pops.
    Penitent,
    HiveMind,
}

/// Data associated with a faction.
///
/// Tracks the state, membership count, and current mood of a guild.
#[derive(Debug, Clone)]
pub struct FactionData {
    /// Display name of the faction (e.g., "Miners' Union").
    pub name: String,
    /// Current satisfaction level (0.0 to 1.0).
    /// *   **1.0**: Perfect.
    /// *   **< 0.4**: Unhappy (Demand triggered).
    pub satisfaction: f32,
    /// Number of members in the faction (updated every tick).
    pub members_count: usize,
    /// Current state of the faction (Loyal, Unhappy, Striking).
    pub state: FactionState,
    /// The active demand, if any.
    pub active_demand: Option<FactionDemand>,
}

impl Default for FactionData {
    fn default() -> Self {
        Self {
            name: "Unknown".to_string(),
            satisfaction: 1.0,
            members_count: 0,
            state: FactionState::Loyal,
            active_demand: None,
        }
    }
}

/// Resource containing all factions in the colony.
///
/// This is the primary interface for querying faction status.
///
/// # Note on Balance
///
/// Currently, faction satisfaction resets to `1.0` every tick, and penalties from policies
/// (like `DoubleShifts` -0.2) are not cumulative enough to drop satisfaction below the
/// "Unhappy" threshold (0.4). This means factions rarely, if ever, go on strike in the
/// current implementation.
///
/// # Examples
///
/// ```
/// use scale::layer1::factions::{Factions, FactionId};
///
/// let mut factions = Factions::default();
/// factions.initialize();
///
/// if let Some(miners) = factions.get(FactionId::MinersGuild) {
///     println!("Miners: {} members", miners.members_count);
/// }
/// ```
#[derive(Resource, Default)]
pub struct Factions {
    /// Map of `FactionId` to `FactionData`.
    pub map: HashMap<FactionId, FactionData>,
}

impl Factions {
    /// Retrieves data for a specific faction.
    #[must_use]
    pub fn get(&self, id: FactionId) -> Option<&FactionData> {
        self.map.get(&id)
    }

    /// Initializes default factions if the map is empty.
    ///
    /// Called automatically by `update_faction_satisfaction_system`, but useful for tests.
    pub fn initialize(&mut self) {
        if self.map.is_empty() {
            self.map.insert(
                FactionId::MinersGuild,
                FactionData {
                    name: "Miners' Union".into(),
                    ..Default::default()
                },
            );
            self.map.insert(
                FactionId::FarmersGuild,
                FactionData {
                    name: "Growers' Circle".into(),
                    ..Default::default()
                },
            );
            self.map.insert(
                FactionId::MasonsGuild,
                FactionData {
                    name: "Masons' Lodge".into(),
                    ..Default::default()
                },
            );
            self.map.insert(
                FactionId::LoggersGuild,
                FactionData {
                    name: "Foresters' Pact".into(),
                    ..Default::default()
                },
            );
            self.map.insert(
                FactionId::ArtisansGuild,
                FactionData {
                    name: "Artisans' Guild".into(),
                    ..Default::default()
                },
            );
            self.map.insert(
                FactionId::Unaligned,
                FactionData {
                    name: "Unaligned".into(),
                    ..Default::default()
                },
            );
            self.map.insert(
                FactionId::Cartel,
                FactionData {
                    name: "The Cartel".into(),
                    ..Default::default()
                },
            );
            self.map.insert(
                FactionId::SubLithic,
                FactionData {
                    name: "The Sub-Lithic Cult".into(),
                    ..Default::default()
                },
            );
            self.map.insert(
                FactionId::Penitent,
                FactionData {
                    name: "The Penitent".into(),
                    ..Default::default()
                },
            );
        }
    }
}

/// Component indicating faction membership.
///
/// Every Pop has this component. It is updated periodically based on their highest skill.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct FactionMember {
    /// The ID of the faction this entity belongs to.
    pub faction_id: Option<FactionId>,
}

/// System to update faction membership based on highest skill.
///
/// *   **Mining** -> `MinersGuild`
/// *   **Farming/Husbandry** -> `FarmersGuild`
/// *   **Construction** -> `MasonsGuild`
/// *   **Forestry** -> `LoggersGuild`
/// *   **Crafting** -> `ArtisansGuild`
/// *   **None/Tie** -> Unaligned (default tie-breaking order exists)
pub fn update_faction_membership_system(
    mut query: Query<
        (&crate::layer1::skills::Skills, &mut FactionMember),
        Changed<crate::layer1::skills::Skills>,
    >,
) {
    for (skills, mut member) in &mut query {
        let mut max_xp = 0.0;
        let mut best_skill = None;

        // Iterate in fixed order for determinism
        for &skill in &[
            SkillType::Mining,
            SkillType::Forestry,
            SkillType::Farming,
            SkillType::Husbandry,
            SkillType::Construction,
            SkillType::Crafting,
        ] {
            let xp = skills.get_xp(skill);
            if xp > max_xp {
                max_xp = xp;
                best_skill = Some(skill);
            }
        }

        let new_faction = match best_skill {
            Some(SkillType::Mining) => FactionId::MinersGuild,
            Some(SkillType::Farming | SkillType::Husbandry) => FactionId::FarmersGuild,
            Some(SkillType::Construction | SkillType::Engineering) => FactionId::MasonsGuild,
            Some(SkillType::Forestry) => FactionId::LoggersGuild,
            Some(SkillType::Crafting) => FactionId::ArtisansGuild,
            Some(SkillType::Science) => FactionId::ScholarsGuild,
            None => FactionId::Unaligned,
        };

        if member.faction_id != Some(new_faction) {
            member.faction_id = Some(new_faction);
        }
    }
}

/// System to update faction satisfaction and member counts.
///
/// This system runs every tick and recalculates satisfaction from scratch.
///
/// # Warning: Non-Cumulative Logic
///
/// Satisfaction is **reset to 1.0** at the start of every update. This means past events
/// do not affect current mood. Only *currently active* policies apply penalties.
///
/// # Logic
///
/// 1.  **Reset**: All factions set to 1.0 satisfaction.
/// 2.  **Census**: Member counts are recalculated.
/// 3.  **Penalties**:
///     *   `DoubleShifts`: -0.2
///     *   `Rationing`: -0.1
pub fn update_faction_satisfaction_system(
    policies: Res<crate::layer1::edicts::ColonyPolicies>,
    mut factions: ResMut<Factions>,
    member_query: Query<&FactionMember>,
) {
    // 1. Reset counts/Initialize
    factions.initialize();
    for data in factions.map.values_mut() {
        data.members_count = 0;
        data.satisfaction = 1.0;
    }

    // 2. Count members
    for member in &member_query {
        if member.faction_id == Some(FactionId::Stateless) {
            continue; // Stateless pops are not counted in normal faction satisfaction mechanics
        }
        if let Some(data) = member.faction_id.and_then(|id| factions.map.get_mut(&id)) {
            data.members_count += 1;
        }
    }

    // 3. Apply Policy Effects
    let double_shifts_penalty = if policies.is_active(crate::layer1::edicts::Policy::DoubleShifts) {
        -0.2
    } else {
        0.0
    };
    let rationing_penalty = if policies.is_active(crate::layer1::edicts::Policy::Rationing) {
        -0.1
    } else {
        0.0
    };

    let total_penalty = double_shifts_penalty + rationing_penalty;

    for data in factions.map.values_mut() {
        data.satisfaction = (data.satisfaction + total_penalty).clamp(0.0, 1.0);
    }
}

/// System to manage faction demands based on satisfaction.
///
/// # Demand Cycle
/// 1.  **Generate**: If satisfaction < 0.4 and no demand exists, generate a demand to stop a penalty policy.
/// 2.  **Resolve**: If satisfaction recovers (> 0.5) OR the demanded policy is toggled off, the demand is cleared.
/// 3.  **Strike**: Handled by `update_faction_strikes_system` if demand times out.
pub fn update_faction_demands_system(
    mut factions: ResMut<Factions>,
    policies: Res<crate::layer1::edicts::ColonyPolicies>,
) {
    for data in factions.map.values_mut() {
        // 1. Resolve existing demands if met
        if let Some(demand) = &data.active_demand {
            // Check if demand is met:
            // A. Satisfaction has recovered (> 0.5).
            // B. The specific policy demanded (e.g. Stop DoubleShifts) has been enacted (set to inactive).

            let satisfaction_improved = data.satisfaction > 0.5;

            let policy_met = demand.policy.is_some_and(|p| {
                if p == Policy::DoubleShifts || p == Policy::Rationing {
                    // They want these penalties OFF.
                    !policies.is_active(p)
                } else {
                    // Default for other policies: assume they want them ON.
                    policies.is_active(p)
                }
            });

            if satisfaction_improved || policy_met {
                data.active_demand = None;
                data.state = FactionState::Loyal;
                continue;
            }
        }

        // 2. Generate new demand if Unhappy and None
        if data.satisfaction < 0.4 && data.active_demand.is_none() {
            // Pick a demand.
            // Prioritize removing active penalty policies.
            let demand_policy = if policies.is_active(Policy::DoubleShifts) {
                Some(Policy::DoubleShifts)
            } else if policies.is_active(Policy::Rationing) {
                Some(Policy::Rationing)
            } else {
                // Default fallback if angry for other reasons
                Some(Policy::DoubleShifts)
            };

            data.active_demand = Some(FactionDemand {
                policy: demand_policy,
                remaining_time: 2000.0, // ~1 day
                description: "Change Policy".into(),
            });
            data.state = FactionState::Unhappy;
        }
    }
}

/// System to handle strike timeouts.
///
/// Decrements the `remaining_time` on active demands.
/// If time runs out while still Unhappy, the faction goes on Strike.
pub fn update_faction_strikes_system(mut factions: ResMut<Factions>) {
    for data in factions.map.values_mut() {
        if let Some(demand) = &mut data.active_demand {
            if demand.remaining_time > 0.0 {
                demand.remaining_time -= 1.0;
            }
            // Check expiry after decrement
            if demand.remaining_time <= 0.0 && data.state == FactionState::Unhappy {
                data.state = FactionState::Striking;
            }
        }
    }
}

/// Helper to check if a pop is in a striking faction.
///
/// Used by the Execution system to prevent work.
#[must_use]
pub fn is_pop_striking(world: &World, entity: Entity) -> bool {
    world
        .get::<FactionMember>(entity)
        .and_then(|m| m.faction_id)
        .and_then(|fid| world.get_resource::<Factions>().and_then(|f| f.get(fid)))
        .is_some_and(|data| data.state == FactionState::Striking)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::edicts::{ColonyPolicies, Policy};
    use crate::layer1::pop::Pop;
    use crate::layer1::skills::{SkillType, Skills};

    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(Factions::default());
        world.insert_resource(ColonyPolicies::default());
        world
    }

    #[test]
    fn test_pop_joins_highest_skill_faction() {
        let mut world = setup_world();

        // Spawn a Pop with high Mining skill
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Mining, 500.0); // High
        skills.add_xp(SkillType::Farming, 100.0); // Low

        let pop = world.spawn((Pop, skills, FactionMember::default())).id();

        // Run system
        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            update_faction_membership_system,
        );

        let member = world.get::<FactionMember>(pop).unwrap();
        assert_eq!(member.faction_id, Some(FactionId::MinersGuild));
    }

    #[test]
    fn test_pop_changes_faction_on_skill_change() {
        let mut world = setup_world();

        // Start as Miner
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Mining, 200.0);

        let pop = world
            .spawn((
                Pop,
                skills,
                FactionMember {
                    faction_id: Some(FactionId::MinersGuild),
                },
            ))
            .id();

        // Update skills to be a Farmer
        let mut skills = world.get_mut::<Skills>(pop).unwrap();
        skills.add_xp(SkillType::Farming, 500.0); // Now higher than Mining

        // Run system
        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            update_faction_membership_system,
        );

        let member = world.get::<FactionMember>(pop).unwrap();
        assert_eq!(member.faction_id, Some(FactionId::FarmersGuild));
    }

    #[test]
    fn test_faction_member_counts() {
        let mut world = setup_world();

        // Spawn 2 Miners, 1 Farmer
        for _ in 0..2 {
            let mut s = Skills::default();
            s.add_xp(SkillType::Mining, 100.0);
            world.spawn((Pop, s, FactionMember::default()));
        }
        let mut s = Skills::default();
        s.add_xp(SkillType::Farming, 100.0);
        world.spawn((Pop, s, FactionMember::default()));

        // Run membership update
        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            update_faction_membership_system,
        );
        // Run satisfaction update (which should also update counts or have a separate system)
        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            update_faction_satisfaction_system,
        );

        let factions = world.resource::<Factions>();

        let miners = factions
            .get(FactionId::MinersGuild)
            .expect("MinersGuild should exist");
        assert_eq!(miners.members_count, 2);

        let farmers = factions
            .get(FactionId::FarmersGuild)
            .expect("FarmersGuild should exist");
        assert_eq!(farmers.members_count, 1);
    }

    #[test]
    fn test_policy_affects_satisfaction() {
        let mut world = setup_world();

        // Enable DoubleShifts
        let mut policies = world.resource_mut::<ColonyPolicies>();
        policies.toggle(Policy::DoubleShifts);

        // Run update
        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            update_faction_satisfaction_system,
        );

        let factions = world.resource::<Factions>();
        let miners = factions.get(FactionId::MinersGuild).unwrap();

        // Assume base 1.0, penalty 0.2
        assert!(miners.satisfaction <= 0.8 + f32::EPSILON);
    }

    #[test]
    fn test_unaligned_faction() {
        let mut world = setup_world();
        let pop = world
            .spawn((
                Pop,
                Skills::default(), // No XP
                FactionMember::default(),
            ))
            .id();

        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            update_faction_membership_system,
        );

        let member = world.get::<FactionMember>(pop).unwrap();
        assert_eq!(member.faction_id, Some(FactionId::Unaligned));
    }

    #[test]
    fn test_tie_breaking_deterministic() {
        let mut world = setup_world();
        let mut skills = Skills::default();
        // Mining and Farming same XP
        skills.add_xp(SkillType::Mining, 100.0);
        skills.add_xp(SkillType::Farming, 100.0);

        let pop = world.spawn((Pop, skills, FactionMember::default())).id();

        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            update_faction_membership_system,
        );

        let member = world.get::<FactionMember>(pop).unwrap();
        // Mining is checked before Farming in the loop, so it wins.
        assert_eq!(member.faction_id, Some(FactionId::MinersGuild));
    }
}

pub mod subcontractor_factions {
    //! # 271: Subcontractor Factions
    //!
    //! Selling off pieces of sovereignty. The colony can lease designated
    //! zones to external megacorporations. The megacorp instantly builds
    //! extraction infrastructure ([`CorporateRig`]) for free and begins
    //! extracting resources; the colony receives rent in weekly chunks.
    //! Their laws apply in the zone: brutal security, no safety regulations.
    //! Striking workers (low morale) in a leased zone face corporate
    //! security sweeps. When the lease expires, original laws are restored
    //! and the rigs are dismantled.
    use bevy::math::Vec2;
    use bevy_ecs::prelude::*;

    use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
    use crate::layer1::economy::resources::ColonyResources;

    /// Ticks between rent payouts (weekly chunks, not per-tick micro-transactions).
    pub const RENT_PAYOUT_PERIOD_TICKS: u64 = 50;
    /// Ticks between automatic corporate security sweeps of a leased zone.
    pub const SWEEP_PERIOD_TICKS: u64 = 50;
    /// Morale below this marks a pop as "striking" for sweep purposes.
    pub const STRIKE_MORALE_THRESHOLD: f32 = 0.2;
    /// Damage dealt by a corporate security sweep to striking pops.
    pub const SWEEP_DAMAGE: f32 = 25.0;

    /// A zone leased to a megacorporation.
    #[derive(Component, Debug)]
    pub struct Leased {
        /// The megacorp entity holding the lease.
        pub lessee: Entity,
        /// Rent accrued per tick (paid out in chunks).
        pub rent_per_tick: f32,
        /// Ticks until the lease expires and laws are restored.
        pub ticks_remaining: u32,
        /// Original security level, restored on expiry.
        pub prior_security: SecurityLevel,
        /// Original hazard policy, restored on expiry.
        pub prior_hazards: bool,
        /// Rent accrued but not yet paid out.
        pub accrued: f32,
    }

    #[derive(Event, Debug, Clone)]
    pub struct LeaseZoneEvent {
        pub zone: Entity,
        pub lessee: Entity,
        pub rent_per_tick: f32,
        pub duration: u32,
    }

    #[derive(Event, Debug, Clone)]
    pub struct MegacorpSecuritySweepEvent {
        pub zone: Entity,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum ZoneType {
        Mining,
        Other,
    }

    #[derive(Component, Debug)]
    pub struct DesignatedZone {
        pub zone_type: ZoneType,
        pub tiles: Vec<Vec2>,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum SecurityLevel {
        Normal,
        Brutal,
    }

    #[derive(Component, Debug)]
    pub struct LawSet {
        pub security: SecurityLevel,
        pub hazards_allowed: bool,
    }

    #[derive(Component, Debug)]
    pub struct Megacorp {
        pub name: String,
    }

    /// Megacorp extraction infrastructure, built instantly on lease.
    /// Dismantled when the lease expires.
    #[derive(Component, Debug)]
    pub struct CorporateRig {
        /// The leased zone this rig extracts from.
        pub zone: Entity,
        /// The megacorp that owns it.
        pub lessee: Entity,
    }

    /// Handles new leases: overrides zone law, records prior law, spawns
    /// the corporate rig, and announces the deal in the chronicle.
    pub fn handle_leased_zones_system(
        mut commands: Commands,
        mut lease_events: EventReader<LeaseZoneEvent>,
        mut zones: Query<(Entity, &DesignatedZone, &mut LawSet)>,
        megacorps: Query<&Megacorp>,
        mut chronicle: EventWriter<AddChronicleEvent>,
    ) {
        for event in lease_events.read() {
            if let Ok((zone_entity, zone, mut law)) = zones.get_mut(event.zone) {
                let prior_security = law.security;
                let prior_hazards = law.hazards_allowed;
                law.security = SecurityLevel::Brutal;
                law.hazards_allowed = true;

                commands.entity(zone_entity).insert(Leased {
                    lessee: event.lessee,
                    rent_per_tick: event.rent_per_tick,
                    ticks_remaining: event.duration,
                    prior_security,
                    prior_hazards,
                    accrued: 0.0,
                });

                // Instant infrastructure: the rig goes on the first zone tile.
                if let Some(first) = zone.tiles.first() {
                    commands.spawn((
                        CorporateRig {
                            zone: zone_entity,
                            lessee: event.lessee,
                        },
                        crate::layer1::map::GridPosition {
                            x: first.x as i32,
                            y: first.y as i32,
                        },
                    ));
                }

                let corp_name = megacorps
                    .get(event.lessee)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|_| "an unknown combine".to_string());
                chronicle.send(AddChronicleEvent {
                    text: format!(
                        "The {corp_name} has leased a {:?} zone for {} ticks at {:.1} credits/tick. Their laws apply now.",
                        zone.zone_type, event.duration, event.rent_per_tick,
                    ),
                    importance: EventImportance::Major,
                });
            }
        }
    }

    /// Accrues rent, pays it out in chunks, and handles lease expiry
    /// (restores prior law, dismantles rigs, pays remaining balance).
    pub fn rent_collection_system(
        mut commands: Commands,
        mut tick: Local<u64>,
        mut leases: Query<(Entity, &mut Leased, &mut LawSet)>,
        rigs: Query<(Entity, &CorporateRig)>,
        mut treasury: Option<ResMut<ColonyResources>>,
        mut chronicle: EventWriter<AddChronicleEvent>,
    ) {
        *tick += 1;
        for (zone_entity, mut lease, mut law) in leases.iter_mut() {
            lease.accrued += lease.rent_per_tick;
            lease.ticks_remaining = lease.ticks_remaining.saturating_sub(1);

            let payout_due = tick.is_multiple_of(RENT_PAYOUT_PERIOD_TICKS);
            let expired = lease.ticks_remaining == 0;

            if (payout_due || expired) && lease.accrued > 0.0 {
                if let Some(ref mut res) = treasury {
                    res.credits += lease.accrued;
                }
                lease.accrued = 0.0;
            }

            if expired {
                law.security = lease.prior_security;
                law.hazards_allowed = lease.prior_hazards;
                for (rig_entity, rig) in rigs.iter() {
                    if rig.zone == zone_entity {
                        commands.entity(rig_entity).despawn();
                    }
                }
                commands.entity(zone_entity).remove::<Leased>();
                chronicle.send(AddChronicleEvent {
                    text: "A corporate lease has expired. Colonial law is restored and the rigs go quiet.".to_string(),
                    importance: EventImportance::Standard,
                });
            }
        }
    }

    /// Corporate security fires on striking (low-morale) pops inside a
    /// brutal-law leased zone.
    pub fn megacorp_security_sweep_system(
        mut events: EventReader<MegacorpSecuritySweepEvent>,
        zones: Query<(&DesignatedZone, &LawSet), With<Leased>>,
        mut pops: Query<
            (
                &crate::layer1::map::GridPosition,
                &mut crate::layer1::biology::health::Health,
                &crate::layer1::social::morale::Morale,
            ),
            With<crate::layer1::pop::Pop>,
        >,
    ) {
        for event in events.read() {
            if let Ok((zone, law)) = zones.get(event.zone) {
                if law.security == SecurityLevel::Brutal {
                    for (pos, mut health, morale) in pops.iter_mut() {
                        let vec_pos = Vec2::new(pos.x as f32, pos.y as f32);
                        if zone.tiles.contains(&vec_pos)
                            && morale.value < STRIKE_MORALE_THRESHOLD
                        {
                            health.current -= SWEEP_DAMAGE;
                        }
                    }
                }
            }
        }
    }

    /// The megacorp watches its territory: if striking workers are detected
    /// in a leased zone, corporate security is dispatched automatically.
    pub fn auto_sweep_system(
        mut tick: Local<u64>,
        leases: Query<(Entity, &DesignatedZone, &LawSet), With<Leased>>,
        pops: Query<
            (
                &crate::layer1::map::GridPosition,
                &crate::layer1::social::morale::Morale,
            ),
            With<crate::layer1::pop::Pop>,
        >,
        mut sweep_events: EventWriter<MegacorpSecuritySweepEvent>,
    ) {
        *tick += 1;
        if !tick.is_multiple_of(SWEEP_PERIOD_TICKS) {
            return;
        }
        for (zone_entity, zone, law) in leases.iter() {
            if law.security != SecurityLevel::Brutal {
                continue;
            }
            let unrest_detected = pops.iter().any(|(pos, morale)| {
                let vec_pos = Vec2::new(pos.x as f32, pos.y as f32);
                zone.tiles.contains(&vec_pos) && morale.value < STRIKE_MORALE_THRESHOLD
            });
            if unrest_detected {
                sweep_events.send(MegacorpSecuritySweepEvent { zone: zone_entity });
            }
        }
    }

    #[cfg(test)]
    mod subcontractor_tests {
        use super::*;
        use crate::layer1::biology::health::Health;
        use crate::layer1::map::GridPosition;
        use crate::layer1::pop::Pop;
        use crate::layer1::core::chronicle::AddChronicleEvent;
        use crate::layer1::economy::resources::ColonyResources;
        use crate::layer1::social::morale::Morale;
        use bevy_app::{App, Update};

        fn setup_app() -> App {
            let mut app = App::new();
            app.add_systems(
                Update,
                (
                    handle_leased_zones_system,
                    rent_collection_system,
                    megacorp_security_sweep_system,
                    auto_sweep_system,
                )
                    .chain(),
            );

            app.add_event::<LeaseZoneEvent>();
            app.add_event::<MegacorpSecuritySweepEvent>();
            app.add_event::<AddChronicleEvent>();

            app
        }

        #[test]
        fn test_leasing_zone_changes_law_and_provides_income() {
            let mut app = setup_app();

            let faction_id = app
                .world_mut()
                .spawn((Megacorp {
                    name: "OmniCorp".to_string(),
                },))
                .id();

            let zone_id = app
                .world_mut()
                .spawn((
                    DesignatedZone {
                        zone_type: ZoneType::Mining,
                        tiles: vec![Vec2::new(0.0, 0.0)],
                    },
                    LawSet {
                        security: SecurityLevel::Normal,
                        hazards_allowed: false,
                    },
                ))
                .id();

            app.world_mut().send_event(LeaseZoneEvent {
                zone: zone_id,
                lessee: faction_id,
                rent_per_tick: 5.0,
                duration: 100,
            });

            app.update(); // Tick 1

            let zone_law = app.world().get::<LawSet>(zone_id).unwrap();
            // The Megacorp's laws overwrite the colony's
            assert_eq!(zone_law.security, SecurityLevel::Brutal);
            assert!(zone_law.hazards_allowed);
        }

        #[test]
        fn test_megacorp_security_attacks_striking_workers() {
            let mut app = setup_app();

            // Setup a leased zone
            let zone_id = app
                .world_mut()
                .spawn((
                    DesignatedZone {
                        zone_type: ZoneType::Mining,
                        tiles: vec![Vec2::new(0.0, 0.0)],
                    },
                    Leased {
                        lessee: Entity::PLACEHOLDER,
                        rent_per_tick: 5.0,
                        ticks_remaining: 100,
                        prior_security: SecurityLevel::Normal,
                        prior_hazards: false,
                        accrued: 0.0,
                    },
                    LawSet {
                        security: SecurityLevel::Brutal,
                        hazards_allowed: true,
                    },
                ))
                .id();

            // Spawn a pop in the zone who is "Striking" (Unrest > threshold -> Morale < 0.2)
            let pop_id = app
                .world_mut()
                .spawn((
                    GridPosition { x: 0, y: 0 },
                    Pop,
                    Morale {
                        value: 0.1,
                        modifiers: vec![],
                    }, // Low Morale implies Striking/High Unrest
                    Health {
                        current: 100.0,
                        max: 100.0,
                        has_rust_lung: false,
                    },
                ))
                .id();

            app.world_mut()
                .send_event(MegacorpSecuritySweepEvent { zone: zone_id });
            app.update();

            // Pop should take damage because they are striking in a Brutal security zone
            let health = app.world().get::<Health>(pop_id).unwrap();
            assert!(health.current < 100.0);
        }

        #[test]
        fn test_corporate_rig_spawned_on_lease() {
            let mut app = setup_app();
            app.add_event::<AddChronicleEvent>();

            let faction_id = app
                .world_mut()
                .spawn((Megacorp {
                    name: "Helion Combine".to_string(),
                },))
                .id();

            let zone_id = app
                .world_mut()
                .spawn((
                    DesignatedZone {
                        zone_type: ZoneType::Mining,
                        tiles: vec![Vec2::new(3.0, 4.0), Vec2::new(3.0, 5.0)],
                    },
                    LawSet {
                        security: SecurityLevel::Normal,
                        hazards_allowed: false,
                    },
                ))
                .id();

            app.world_mut().send_event(LeaseZoneEvent {
                zone: zone_id,
                lessee: faction_id,
                rent_per_tick: 5.0,
                duration: 100,
            });
            app.update();

            // The megacorp instantly builds extraction infrastructure.
            let mut rigs = app
                .world_mut()
                .query::<(&CorporateRig, &GridPosition)>();
            let rigs: Vec<_> = rigs.iter(app.world()).collect();
            assert_eq!(rigs.len(), 1);
            assert_eq!(rigs[0].0.zone, zone_id);
            assert_eq!(rigs[0].0.lessee, faction_id);
            assert_eq!(rigs[0].1, &GridPosition { x: 3, y: 4 });
        }

        #[test]
        fn test_rent_accrues_and_pays_out_in_chunks() {
            let mut app = setup_app();
            app.add_event::<AddChronicleEvent>();
            app.world_mut().insert_resource(ColonyResources {
                credits: 0.0,
                ..Default::default()
            });

            let faction_id = app
                .world_mut()
                .spawn((Megacorp {
                    name: "Helion Combine".to_string(),
                },))
                .id();

            let zone_id = app
                .world_mut()
                .spawn((
                    DesignatedZone {
                        zone_type: ZoneType::Mining,
                        tiles: vec![Vec2::new(0.0, 0.0)],
                    },
                    LawSet {
                        security: SecurityLevel::Normal,
                        hazards_allowed: false,
                    },
                ))
                .id();

            app.world_mut().send_event(LeaseZoneEvent {
                zone: zone_id,
                lessee: faction_id,
                rent_per_tick: 5.0,
                duration: 200,
            });
            app.update();

            // Run 60 ticks: payout happens every 50 ticks, so 250 credits paid.
            for _ in 0..60 {
                app.update();
            }
            let credits = app.world().resource::<ColonyResources>().credits;
            assert_eq!(credits, 250.0);
        }

        #[test]
        fn test_lease_expiry_restores_law_and_removes_rig() {
            let mut app = setup_app();
            app.add_event::<AddChronicleEvent>();
            app.world_mut().insert_resource(ColonyResources {
                credits: 0.0,
                ..Default::default()
            });

            let faction_id = app
                .world_mut()
                .spawn((Megacorp {
                    name: "Helion Combine".to_string(),
                },))
                .id();

            let zone_id = app
                .world_mut()
                .spawn((
                    DesignatedZone {
                        zone_type: ZoneType::Mining,
                        tiles: vec![Vec2::new(0.0, 0.0)],
                    },
                    LawSet {
                        security: SecurityLevel::Normal,
                        hazards_allowed: false,
                    },
                ))
                .id();

            app.world_mut().send_event(LeaseZoneEvent {
                zone: zone_id,
                lessee: faction_id,
                rent_per_tick: 5.0,
                duration: 5,
            });
            for _ in 0..10 {
                app.update();
            }

            // Lease expired: component removed, original laws restored.
            assert!(app.world().get::<Leased>(zone_id).is_none());
            let law = app.world().get::<LawSet>(zone_id).unwrap();
            assert_eq!(law.security, SecurityLevel::Normal);
            assert!(!law.hazards_allowed);
            // Rig dismantled.
            let mut rigs = app.world_mut().query::<&CorporateRig>();
            assert_eq!(rigs.iter(app.world()).count(), 0);
            // Remaining accrued rent (5 ticks * 5.0) paid out on expiry.
            let credits = app.world().resource::<ColonyResources>().credits;
            assert_eq!(credits, 25.0);
        }

        #[test]
        fn test_auto_sweep_triggers_on_striking_workers() {
            let mut app = setup_app();
            app.add_event::<AddChronicleEvent>();

            let faction_id = app
                .world_mut()
                .spawn((Megacorp {
                    name: "Helion Combine".to_string(),
                },))
                .id();

            let zone_id = app
                .world_mut()
                .spawn((
                    DesignatedZone {
                        zone_type: ZoneType::Mining,
                        tiles: vec![Vec2::new(0.0, 0.0)],
                    },
                    LawSet {
                        security: SecurityLevel::Normal,
                        hazards_allowed: false,
                    },
                ))
                .id();

            app.world_mut().send_event(LeaseZoneEvent {
                zone: zone_id,
                lessee: faction_id,
                rent_per_tick: 5.0,
                duration: 500,
            });
            app.update();

            let pop_id = app
                .world_mut()
                .spawn((
                    GridPosition { x: 0, y: 0 },
                    Pop,
                    Morale {
                        value: 0.1,
                        modifiers: vec![],
                    },
                    Health {
                        current: 100.0,
                        max: 100.0,
                        has_rust_lung: false,
                    },
                ))
                .id();

            // Auto-sweep fires periodically without a manual event.
            for _ in 0..60 {
                app.update();
            }
            let health = app.world().get::<Health>(pop_id).unwrap();
            assert!(health.current < 100.0);
        }
    }
}
