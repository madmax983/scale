//! Spec 376: The Schism.
//!
//! "A colony divided cannot stand."
//!
//! When a faction becomes extremely unhappy, it stops protesting and
//! secedes: it claims a cluster of buildings as its own territory,
//! bars the doors to outsiders, and goes on strike behind them. The
//! colony is left to negotiate or force the issue later — this spec
//! builds the secession itself.
//!
//! ## How happiness is measured here
//!
//! Effective happiness for a faction is the lower of its stored
//! [`Factions`] satisfaction and the average [`Morale`] of its living
//! members (when members carry morale). The stored satisfaction alone
//! can never fall far in the live sim: the satisfaction system resets
//! it to 1.0 every tick and only policy penalties apply, a limitation
//! the factions module itself documents. Member morale is the organic
//! signal that can actually collapse, so the schism listens to both
//! and fires on whichever gives way first.
//!
//! ## Adaptations (spec sketch -> real codebase)
//!
//! * The sketch `layer1::faction::{Faction, FactionHappiness}` types
//!   do not exist. Real factions are not entities: they are entries in
//!   the [`Factions`] resource, keyed by [`FactionId`], with a
//!   `satisfaction` field in the 0.0..=1.0 range. [`SchismEvent`]
//!   therefore carries a [`FactionId`], and the sketch threshold of
//!   10.0 on a 0..100 scale maps to [`DEFAULT_SCHISM_THRESHOLD`]
//!   (0.10) on the real scale.
//! * The sketch `layer1::building::Building` path is real
//!   (`crate::layer1::building::Building`), as is `GridPosition`.
//! * The module lives in `layer1::social::schism`, beside the real
//!   factions module, not in a new `layer1::faction` tree.
//! * Both systems are exclusive `&mut World` systems (the Spec 375
//!   pattern): they tolerate missing event resources, avoid the
//!   Bevy 0.15 tuple-arity limit, and never use `Option<Res<T>>`.
//! * Locked doors use the real access system, not a parallel one:
//!   claimed buildings gain an [`AccessControl`] in
//!   [`AccessMode::Restricted`] whose allow list is exactly the
//!   seceding members, so the existing `check_access` pathfinding
//!   hook bars outsiders and admits members. No separate LockedDoor
//!   component was invented.
//! * Secession latches: once a faction has seceded it is recorded in
//!   [`SchismState`] and never re-fires, and its [`FactionData`] state
//!   is set to [`FactionState::Striking`] (the existing hostile stance
//!   other systems already respect).
//!
//! ## Left alone (documented, not stubbed)
//!
//! * Hoarding: buildings do not own inventories in this codebase —
//!   stock is global (`ColonyResources`) or item entities — so there
//!   is no clean per-building hoarding hook. Not invented.
//! * Negotiate / force interactions: there is no clean existing hook
//!   for a player order against a specific building (no building UI
//!   command path or targeted combat order). The tension is recorded
//!   in the chronicle and STATS; resolution mechanics are future work.
//! * Reintegration: a seceded faction does not rejoin when happiness
//!   recovers. Secession is a scar, not a mood.
//!
//! All names and strings are original and generic (see the IP-guard
//! test in `schism_tests.rs`).

use bevy_ecs::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::layer1::access_control::{AccessControl, AccessMode};
use crate::layer1::building::{Building, BuildingType};
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::factions::{FactionId, FactionMember, FactionState, Factions};
use crate::layer1::map::GridPosition;
use crate::layer1::social::morale::Morale;
use crate::shared::log::MessageLog;

/// Default critical threshold: at or below this effective happiness
/// (0.0..=1.0 scale) a faction secedes. Maps to the spec value of
/// 10.0 on its 0..100 sketch scale.
pub const DEFAULT_SCHISM_THRESHOLD: f32 = 0.10;
/// Maximum buildings one secession claims.
pub const MAX_TERRITORY_BUILDINGS: usize = 4;
/// Maximum Chebyshev distance from the claimed cluster seed for a
/// building to count as part of the same contiguous cluster.
pub const CLUSTER_RADIUS: i32 = 12;

/// Configurable schism threshold (Spec 376 REFACTOR: the sketch
/// hardcoded its threshold; the real one is a resource).
#[derive(Resource, Debug, Clone, Copy)]
pub struct SchismConfig {
    /// Effective happiness at or below which a faction secedes.
    pub threshold: f32,
}

impl Default for SchismConfig {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_SCHISM_THRESHOLD,
        }
    }
}

/// Latched record of factions that have already seceded, so the
/// schism fires exactly once per faction.
#[derive(Resource, Default)]
pub struct SchismState {
    /// Factions that have seceded (and will never re-fire).
    pub seceded: HashSet<FactionId>,
}

/// Fired when a faction secedes (Spec 376). Carries the real
/// [`FactionId`] key, not an entity — real factions are resource
/// entries, not entities (see module docs).
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchismEvent {
    /// The seceding faction.
    pub faction: FactionId,
}

/// Marker on a building claimed by a seceded faction.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecededTerritory {
    /// The faction holding this building.
    pub faction: FactionId,
}

/// Territory preference: housing first, then workplaces, then nothing.
/// Returns None for buildings a secession does not prefer (they are
/// only claimed as a fallback when no preferred building exists).
#[must_use]
pub fn territory_priority(building_type: BuildingType) -> Option<u8> {
    match building_type {
        BuildingType::Housing => Some(0),
        BuildingType::Farm
        | BuildingType::Greenhouse
        | BuildingType::HydroponicsBay
        | BuildingType::Plantation
        | BuildingType::Smokehouse
        | BuildingType::Smelter
        | BuildingType::Smithy
        | BuildingType::LumberMill
        | BuildingType::StoneMason
        | BuildingType::Weaver
        | BuildingType::Tailor
        | BuildingType::Refinery
        | BuildingType::Library
        | BuildingType::Tavern
        | BuildingType::Hospital
        | BuildingType::Office
        | BuildingType::School
        | BuildingType::TradeDepot => Some(1),
        _ => None,
    }
}

/// Effective happiness of a faction: the lower of its stored
/// satisfaction and its members average morale (when known).
#[must_use]
pub fn effective_happiness(satisfaction: f32, member_morale: Option<f32>) -> f32 {
    match member_morale {
        Some(avg) => satisfaction.min(avg),
        None => satisfaction,
    }
}

/// Parses a faction from a loose name ("miners", "farmers",
/// "scholars", "unaligned", and friends) for headless commands.
#[must_use]
pub fn parse_faction_id(text: &str) -> Option<FactionId> {
    let lower = text.to_ascii_lowercase();
    match lower.as_str() {
        "miners" | "minersguild" | "miners union" => Some(FactionId::MinersGuild),
        "farmers" | "farmersguild" | "growers" => Some(FactionId::FarmersGuild),
        "masons" | "masonsguild" => Some(FactionId::MasonsGuild),
        "loggers" | "loggersguild" | "foresters" => Some(FactionId::LoggersGuild),
        "artisans" | "artisansguild" => Some(FactionId::ArtisansGuild),
        "scholars" | "scholarsguild" => Some(FactionId::ScholarsGuild),
        "unaligned" => Some(FactionId::Unaligned),
        "cartel" => Some(FactionId::Cartel),
        _ => None,
    }
}

/// Display name of a faction from the live [`Factions`] resource,
/// falling back to its debug label.
#[must_use]
pub fn faction_name(world: &World, faction: FactionId) -> String {
    world
        .get_resource::<Factions>()
        .and_then(|factions| factions.get(faction))
        .map_or_else(|| format!("{:?}", faction), |data| data.name.clone())
}

fn chebyshev(a: GridPosition, b: GridPosition) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

fn send_chronicle(world: &mut World, text: String, importance: EventImportance) {
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent { text, importance });
    }
}

/// System: identify factions whose effective happiness has dropped
/// to or below the configurable [`SchismConfig`] threshold and fire
/// a [`SchismEvent`] for each, exactly once per faction.
///
/// Exclusive system (Spec 375 pattern): tolerates missing resources
/// and reads member morale directly from the world.
pub fn process_schism_system(world: &mut World) {
    // Average morale per faction, from living members carrying Morale.
    let mut morale_sums: HashMap<FactionId, (f32, usize)> = HashMap::new();
    {
        let mut query = world.query::<(&FactionMember, Option<&Morale>)>();
        for (member, morale) in query.iter(world) {
            if let (Some(fid), Some(morale)) = (member.faction_id, morale) {
                let entry = morale_sums.entry(fid).or_insert((0.0, 0));
                entry.0 += morale.value;
                entry.1 += 1;
            }
        }
    }

    let threshold = world
        .get_resource::<SchismConfig>()
        .map_or(DEFAULT_SCHISM_THRESHOLD, |config| config.threshold);
    let already: HashSet<FactionId> = world
        .get_resource::<SchismState>()
        .map_or_else(HashSet::new, |state| state.seceded.clone());

    let mut to_fire: Vec<FactionId> = Vec::new();
    if let Some(factions) = world.get_resource::<Factions>() {
        for (id, data) in &factions.map {
            if already.contains(id) {
                continue;
            }
            let avg_morale = morale_sums.get(id).and_then(|(sum, count)| {
                if *count > 0 {
                    #[allow(clippy::cast_precision_loss)]
                    Some(sum / *count as f32)
                } else {
                    None
                }
            });
            if effective_happiness(data.satisfaction, avg_morale) <= threshold {
                to_fire.push(*id);
            }
        }
    }
    // Deterministic order despite HashMap iteration.
    to_fire.sort_by_key(|id| format!("{:?}", id));
    if to_fire.is_empty() {
        return;
    }

    // Latch first, so the schism can never re-fire for a faction,
    // even if no building is available to claim this tick.
    if world.get_resource::<SchismState>().is_none() {
        world.insert_resource(SchismState::default());
    }
    {
        let mut state = world.resource_mut::<SchismState>();
        for id in &to_fire {
            state.seceded.insert(*id);
        }
    }
    if let Some(mut events) = world.get_resource_mut::<Events<SchismEvent>>() {
        for id in to_fire {
            events.send(SchismEvent { faction: id });
        }
    }
}

/// One claimable building, pre-scored for the cluster pick.
struct Candidate {
    entity: Entity,
    priority: Option<u8>,
    pos: Option<GridPosition>,
}

impl Candidate {
    fn distance_to_members(&self, member_positions: &[GridPosition]) -> i32 {
        match self.pos {
            Some(pos) => member_positions
                .iter()
                .map(|member_pos| chebyshev(pos, *member_pos))
                .min()
                .unwrap_or(0),
            None => 0,
        }
    }

    fn distance_to_claimed(&self, claimed: &[Candidate]) -> i32 {
        match self.pos {
            Some(pos) => claimed
                .iter()
                .filter_map(|other| other.pos)
                .map(|other_pos| chebyshev(pos, other_pos))
                .min()
                .unwrap_or(0),
            None => 0,
        }
    }
}

/// Claims territory for one seceded faction: a contiguous cluster of
/// up to [`MAX_TERRITORY_BUILDINGS`] preferred buildings (housing,
/// then workplaces) nearest the faction members, falling back to a
/// single building of any type when nothing preferred exists.
fn claim_territory(world: &mut World, faction: FactionId) {
    // The Execution set can claim before the Economy-set census has
    // initialized the faction map on a fresh world; initialize first
    // (idempotent) so names and the Striking mark resolve correctly.
    if let Some(mut factions) = world.get_resource_mut::<Factions>() {
        factions.initialize();
    }
    if world.get_resource::<SchismState>().is_none() {
        world.insert_resource(SchismState::default());
    }
    world
        .resource_mut::<SchismState>()
        .seceded
        .insert(faction);

    // Faction members: entities (for the door allow list) and
    // positions (for the cluster anchor).
    let mut member_entities: Vec<Entity> = Vec::new();
    let mut member_positions: Vec<GridPosition> = Vec::new();
    {
        let mut query = world.query::<(Entity, &FactionMember, Option<&GridPosition>)>();
        for (entity, member, pos) in query.iter(world) {
            if member.faction_id == Some(faction) {
                member_entities.push(entity);
                if let Some(pos) = pos {
                    member_positions.push(*pos);
                }
            }
        }
    }

    // Unclaimed buildings.
    let mut candidates: Vec<Candidate> = Vec::new();
    {
        let mut query = world
            .query::<(Entity, &Building, Option<&GridPosition>, Has<SecededTerritory>)>();
        for (entity, building, pos, already) in query.iter(world) {
            if !already {
                candidates.push(Candidate {
                    entity,
                    priority: territory_priority(building.building_type),
                    pos: pos.copied(),
                });
            }
        }
    }
    if candidates.is_empty() {
        let name = faction_name(world, faction);
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add(format!(
                "THE SCHISM: {name} has seceded, but there was no building to claim."
            ));
        }
        return;
    }

    // Prefer housing and workplaces; fall back to anything standing.
    let has_preferred = candidates.iter().any(|c| c.priority.is_some());
    let mut pool: Vec<Candidate> = candidates
        .into_iter()
        .filter(|c| !has_preferred || c.priority.is_some())
        .collect();
    let claim_limit = if has_preferred {
        MAX_TERRITORY_BUILDINGS
    } else {
        1
    };

    // Seed: best priority, nearest the members, lowest entity index.
    pool.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| {
                a.distance_to_members(&member_positions)
                    .cmp(&b.distance_to_members(&member_positions))
            })
            .then_with(|| a.entity.index().cmp(&b.entity.index()))
    });
    let mut claimed: Vec<Candidate> = Vec::new();
    claimed.push(pool.remove(0));

    // Grow the cluster greedily: nearest to what is already claimed,
    // never reaching beyond CLUSTER_RADIUS of the cluster.
    while claimed.len() < claim_limit && !pool.is_empty() {
        let mut best: Option<usize> = None;
        let mut best_key: Option<(i32, Option<u8>, u32)> = None;
        for (idx, cand) in pool.iter().enumerate() {
            let dist = cand.distance_to_claimed(&claimed);
            let positioned =
                cand.pos.is_some() && claimed.iter().any(|other| other.pos.is_some());
            if positioned && dist > CLUSTER_RADIUS {
                continue;
            }
            let key = (dist, cand.priority, cand.entity.index());
            if best_key.is_none() || key < best_key.unwrap() {
                best_key = Some(key);
                best = Some(idx);
            }
        }
        match best {
            Some(idx) => claimed.push(pool.remove(idx)),
            None => break,
        }
    }

    // Claim: tag the territory and bar the doors to outsiders via the
    // real AccessControl system (restricted, members allow-listed).
    let member_set: HashSet<Entity> = member_entities.iter().copied().collect();
    let claimed_count = claimed.len();
    for cand in &claimed {
        world
            .entity_mut(cand.entity)
            .insert(SecededTerritory { faction });
        world.entity_mut(cand.entity).insert(AccessControl {
            mode: AccessMode::Restricted,
            allowed_pops: member_set.clone(),
            allowed_roles: HashSet::new(),
        });
    }

    // The seceded faction takes the existing hostile stance.
    if let Some(mut factions) = world.get_resource_mut::<Factions>() {
        if let Some(data) = factions.map.get_mut(&faction) {
            data.state = FactionState::Striking;
        }
    }

    let name = faction_name(world, faction);
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(format!(
            "THE SCHISM: {name} has seceded, claiming {claimed_count} building(s). The doors are barred to outsiders."
        ));
    }
    send_chronicle(
        world,
        format!(
            "THE SCHISM: {name} has seceded from the colony, claiming {claimed_count} building(s) as its own territory. Its members have barred the doors against outsiders and will not work for the colony that failed them."
        ),
        EventImportance::Major,
    );
}

/// System: process [`SchismEvent`]s — claim territory, lock doors,
/// mark the faction as striking, and chronicle the secession.
pub fn handle_schism_event_system(world: &mut World) {
    let pending: Vec<SchismEvent> = match world.get_resource_mut::<Events<SchismEvent>>() {
        Some(mut events) => events.drain().collect(),
        None => return,
    };
    for event in pending {
        claim_territory(world, event.faction);
    }
}

/// Whether a pop may enter a building: seceded territory admits only
/// its own faction members, and the real access control decides the
/// rest (claimed buildings are Restricted to the member allow list).
#[must_use]
pub fn outsider_can_enter(world: &World, building: Entity, pop: Entity) -> bool {
    if let Some(territory) = world.get::<SecededTerritory>(building) {
        let member_of = world
            .get::<FactionMember>(pop)
            .and_then(|member| member.faction_id);
        if member_of != Some(territory.faction) {
            return false;
        }
    }
    crate::layer1::access_control::check_access(world, building, pop)
}

/// Number of buildings currently held as seceded territory.
#[must_use]
pub fn seceded_territory_count(world: &mut World) -> usize {
    let mut query = world.query::<&SecededTerritory>();
    query.iter(world).count()
}

/// Number of factions recorded as seceded in [`SchismState`].
#[must_use]
pub fn seceded_faction_count(world: &World) -> usize {
    world
        .get_resource::<SchismState>()
        .map_or(0, |state| state.seceded.len())
}

/// Names of the seceded factions, or "none" (for STATS and panels).
#[must_use]
pub fn seceded_factions_label(world: &World) -> String {
    let Some(state) = world.get_resource::<SchismState>() else {
        return "none".to_string();
    };
    if state.seceded.is_empty() {
        return "none".to_string();
    }
    let mut ids: Vec<FactionId> = state.seceded.iter().copied().collect();
    ids.sort_by_key(|id| format!("{:?}", id));
    ids.iter()
        .map(|id| faction_name(world, *id))
        .collect::<Vec<String>>()
        .join(", ")
}

/// Headless seam: set one faction satisfaction directly and return
/// whether the faction exists. Note the live satisfaction system
/// recomputes satisfaction from policies each tick, so the debug
/// trigger command sends the event itself rather than relying on
/// this value surviving a tick.
pub fn set_faction_satisfaction(world: &mut World, faction: FactionId, level: f32) -> bool {
    match world
        .get_resource_mut::<Factions>()
        .and_then(|mut factions| factions.map.get_mut(&faction).map(|data| {
            data.satisfaction = level.clamp(0.0, 1.0);
        })) {
        Some(()) => true,
        None => false,
    }
}

/// Every public-facing string this module puts in front of a player
/// (IP-guard test scans these).
#[must_use]
pub fn schism_public_strings() -> Vec<String> {
    vec![
        "The Schism".to_string(),
        "THE SCHISM".to_string(),
        "has seceded from the colony".to_string(),
        "claiming its own territory".to_string(),
        "The doors are barred to outsiders.".to_string(),
        "Seceded Territory".to_string(),
    ]
}
