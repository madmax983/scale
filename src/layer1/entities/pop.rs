//! Population management and entity definitions.
//!
//! # The "Pop" (Population Agent)
//!
//! A "Pop" is the atomic unit of agency in the colony. They are not just resources;
//! they are semi-autonomous agents driven by a hierarchy of needs (see [`crate::layer1::needs`])
//! and decision-making logic (see [`crate::layer1::utility_ai`]).
//!
//! ## Lifecycle
//!
//! 1.  **Spawning**: Pops are created by [`spawn_initial_pops`] (or potential future immigration events).
//! 2.  **Simulation**: Every tick, systems in [`crate::layer1::needs`] update their physiological state.
//! 3.  **Decision**: The Utility AI ([`crate::layer1::utility_ai`]) evaluates options and assigns a [`PopAction`].
//! 4.  **Execution**: The chosen action is carried out, modifying the world or the pop's state.
//!
//! ## Components
//!
//! A fully initialized Pop entity typically has:
//! * [`Pop`]: The marker component.
//! * [`GridPosition`]: Physical location.
//! * [`Needs`]: Hunger, rest, etc.
//! * [`PopAction`]: Current task state.
//! * [`UtilityWeights`]: Personality/learning factors.

use super::wild_child::WildExposure;
use crate::layer1::admin::AdminConsumer;
use crate::layer1::artifacts::ActiveAuras;
use crate::layer1::biocompatibility::Biocompatibility;
use crate::layer1::cabin_fever::CabinFever;
use crate::layer1::contagion::ContagionCooldown;
use crate::layer1::economy::Wallet;
use crate::layer1::factions::FactionMember;
use crate::layer1::funeral::Corpse;
use crate::layer1::gut_biome::GutBiome;
use crate::layer1::health::{Dead, Health};
use crate::layer1::hygiene::Filth;
use crate::layer1::items::Equipment;
use crate::layer1::language::{Dialect, Linguistics};
use crate::layer1::lifecycle::Age;
use crate::layer1::map::{GridPosition, ScreenShake};
use crate::layer1::memory::{Memories, MemoryType};
use crate::layer1::morale::Morale;
use crate::layer1::needs::Needs;
use crate::layer1::palette_fatigue::DietaryHistory;
use crate::layer1::rumor::Knowledge;
use crate::layer1::skills::Skills;
use crate::layer1::social::debt::SocialDebt;
use crate::layer1::social::old_guard::Arrival;
use crate::layer1::social::sartorial_rebellion::Appearance;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::traits::Traits;
use crate::layer1::utility_types::AssignmentType;
/// Alias for `AssignmentType` for job-related contexts (Spec 113).
pub use crate::layer1::utility_types::AssignmentType as JobType;
use crate::layer1::utility_types::{PopAction, UtilityWeights};
use crate::layer1::GlobalHitStop;
use crate::shared::log::MessageLog;
use bevy_ecs::prelude::*;
use rand::Rng;
use ratatui::style::Color;

/// A pop's individual name.
///
/// Currently selected from a hardcoded list of short names.
///
/// # Examples
///
/// ```
/// use scale::layer1::pop::PopName;
/// use rand::thread_rng;
///
/// let mut rng = thread_rng();
/// let name = PopName::random(&mut rng);
/// assert!(!name.0.is_empty());
/// ```
#[derive(Component, Clone, Debug)]
pub struct PopName(pub String);

/// Resource tracking the total living population count.
#[derive(Resource, Default, Debug, Clone)]
pub struct PopulationCount {
    pub total: usize,
}

/// Event triggered when a pop dies.
#[derive(Event, Debug, Clone)]
pub struct PopDied {
    /// The entity that died.
    pub entity: Entity,
    /// The name of the pop.
    pub name: String,
    /// The tick when death occurred.
    pub tick: u64,
    /// The cause of death.
    pub reason: String,
}

const POP_DEATH_SCREEN_SHAKE: f32 = 0.25;
const POP_DEATH_HIT_STOP_TICKS: u32 = 2;

/// Event triggered when a pop is born/spawned (e.g. from Clone Vat).
#[derive(Event, Debug, Clone)]
pub struct PopBorn {
    /// The entity that was born.
    pub entity: Entity,
    /// The name of the pop.
    pub name: String,
    /// The tick when birth occurred.
    pub tick: u64,
    /// The source of the spawn (e.g., "Clone Vat", "Immigrant").
    pub source: String,
}

/// Event triggered when pops reproduce.
#[derive(Event, Debug, Clone)]
pub struct ReproductionEvent {
    pub parent_a: Entity,
    pub parent_b: Entity,
    pub child: Entity,
}

const POP_NAMES: &[&str] = &[
    "Ada", "Bryn", "Cole", "Dara", "Eli", "Fern", "Gale", "Hana", "Iris", "Joss", "Kael", "Luna",
    "Milo", "Neva", "Orin", "Pax", "Quinn", "Rhea", "Sable", "Tarn", "Uma", "Vale", "Wren", "Xia",
    "Yara", "Zev",
];

/// Pick a random name from the hardcoded list.
fn generate_name<R: Rng>(rng: &mut R) -> PopName {
    let idx = rng.gen_range(0..POP_NAMES.len());
    PopName(POP_NAMES[idx].to_string())
}

impl PopName {
    /// Generates a random `PopName`.
    ///
    /// ## Examples
    /// ```
    /// use rand::thread_rng;
    /// use scale::layer1::entities::pop::PopName;
    ///
    /// let mut rng = thread_rng();
    /// let name = PopName::random(&mut rng);
    /// assert!(!name.0.is_empty());
    /// ```
    pub fn random<R: Rng>(rng: &mut R) -> Self {
        generate_name(rng)
    }
}

/// Marker component for pop entities.
///
/// This component identifies an entity as a "Citizen" of the colony. It is the
/// primary query filter for most simulation systems (needs, AI, jobs).
///
/// # Examples
///
/// Querying all pops:
/// ```
/// use scale::layer1::pop::Pop;
/// use bevy_ecs::prelude::*;
///
/// fn count_pops(query: Query<&Pop>) -> usize {
///     query.iter().count()
/// }
/// ```
#[derive(Component, Default)]
pub struct Pop;

/// Tracks a pop's persistent employment, even when temporarily reassigned (e.g. to hospital).
///
/// # Examples
///
/// ```
/// use scale::layer1::pop::Job;
/// use scale::layer1::utility_types::AssignmentType;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// let workplace_entity = world.spawn_empty().id();
///
/// let pop_entity = world.spawn(Job {
///     workplace: workplace_entity,
///     job_type: AssignmentType::FarmWorker,
/// }).id();
///
/// let job = world.get::<Job>(pop_entity).unwrap();
/// assert_eq!(job.workplace, workplace_entity);
/// assert_eq!(job.job_type, AssignmentType::FarmWorker);
/// ```
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Job {
    /// The building entity where the pop works.
    pub workplace: Entity,
    /// The type of job (e.g. `FarmWorker`, `LibraryWorker`).
    pub job_type: AssignmentType,
}

/// Represents the formal education level of a Pop.
///
/// Used to determine eligibility for certain advanced tasks or traits.
///
/// # Examples
///
/// ```
/// use scale::layer1::pop::EducationLevel;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// let uneducated_pop = world.spawn(EducationLevel(0)).id();
/// let educated_pop = world.spawn(EducationLevel(3)).id();
///
/// assert_eq!(world.get::<EducationLevel>(educated_pop).unwrap().0, 3);
/// ```
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EducationLevel(pub u32);

/// Movement speed of a pop.
///
/// Speed is a multiplier for movement logic, which is discrete (tile-based).
/// Since we cannot move "0.5 tiles", we use an accumulator.
///
/// # How it works
///
/// 1. Every tick, `current` speed is added to `accumulator`.
/// 2. If `accumulator >= 1.0`, the pop moves 1 tile and `1.0` is subtracted.
/// 3. If `accumulator >= 2.0` (super speed), the pop moves multiple tiles.
///
/// # Examples
///
/// ```
/// use scale::layer1::pop::Speed;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// let pop = world.spawn(Speed {
///     base: 1.0,
///     current: 0.5, // Slow (encumbered or injured)
///     accumulator: 0.0,
/// }).id();
///
/// let mut speed = world.get_mut::<Speed>(pop).unwrap();
///
/// // Tick 1: Accumulate 0.5
/// speed.accumulator += speed.current;
/// assert!(speed.accumulator < 1.0); // No move
///
/// // Tick 2: Accumulate 0.5 -> 1.0
/// speed.accumulator += speed.current;
/// if speed.accumulator >= 1.0 {
///     // Move logic here
///     speed.accumulator -= 1.0;
/// }
/// assert_eq!(speed.accumulator, 0.0);
/// ```
#[derive(Component, Debug, Clone)]
pub struct Speed {
    /// Base speed multiplier (usually 1.0).
    pub base: f32,
    /// Current effective speed multiplier.
    /// Modified by terrain, health, and traits.
    pub current: f32,
    /// Accumulator for fractional movement.
    /// Tracks progress towards the next tile step.
    pub accumulator: f32,
}

impl Default for Speed {
    fn default() -> Self {
        Self {
            base: 1.0,
            current: 1.0,
            accumulator: 0.0,
        }
    }
}

/// Role of a pop (Spec 142).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Role {
    #[default]
    /// Standard citizen with no special access.
    Civilian,
    /// Professional military personnel.
    Soldier,
    /// Technical personnel for maintenance and construction.
    Engineer,
    /// Conscripted defense force.
    Militia,
}

/// Bundle defining a complete Pop entity.
#[derive(Bundle)]
pub struct PopBundle {
    /// Marker component.
    pub pop: Pop,
    /// Name of the pop.
    pub name: PopName,
    /// Position in the world.
    pub pos: GridPosition,
    /// Health status.
    pub health: Health,
    /// Physiological needs.
    pub needs: Needs,
    /// Memories and trauma.
    pub memories: Memories,
    /// Compliance with edicts and laws.
    pub edict_compliance: crate::layer1::social::generational_dissonance::EdictCompliance,
    /// Skill levels.
    pub skills: Skills,
    /// Appearance signifiers for Sartorial Rebellion.
    pub appearance: Appearance,
    /// Movement speed.
    pub speed: Speed,
    /// Current action.
    pub action: PopAction,
    /// Equipped items.
    pub equipment: Equipment,
    /// AI decision weights.
    pub weights: UtilityWeights,
    /// Known rumors and topics.
    pub knowledge: Knowledge,
    /// Biological age.
    pub age: Age,
    /// Faction membership.
    pub faction: FactionMember,
    /// Arrival time.
    pub arrival: Arrival,
    /// Ethics configuration.
    pub ethics: crate::layer1::social::indoctrination::PopEthics,
    /// Hygiene filth level.
    pub filth: Filth,
    /// Active status effects.
    pub auras: ActiveAuras,
    /// Social debts owed.
    pub social_debt: SocialDebt,
    /// Overall morale.
    pub morale: Morale,
    /// Disease cooldown.
    pub contagion: ContagionCooldown,
    /// Personality traits.
    pub traits: Traits,
    /// Cabin fever status.
    pub cabin_fever: CabinFever,
    /// Biocompatibility with environment.
    pub biocompatibility: Biocompatibility,
    /// Exposure to the wild.
    pub wild_exposure: WildExposure,
    /// Bureaucracy demand.
    pub admin_consumer: AdminConsumer,
    /// Food variety history.
    pub dietary_history: DietaryHistory,
    /// Personal currency.
    pub wallet: Wallet,
    /// Spoken dialect.
    pub dialect: Dialect,
    /// Linguistic knowledge.
    pub linguistics: Linguistics,
    /// Biological digestive adaptation.
    pub gut_biome: GutBiome,
    /// Formal education level.
    pub education: EducationLevel,
    /// Base stats.
    pub stats: crate::layer1::psychology::void_sickness::PopStats,
    /// Resonance architectural traits.
    pub improvised_tools: crate::layer1::improvised_tools::ImprovisedTools,
    pub resonance_traits: crate::layer1::architecture::resonant_architecture::PopResonanceTraits,
    pub base_resonance_traits:
        crate::layer1::architecture::resonant_architecture::PopBaseResonanceTraits,
}

impl PopBundle {
    /// Generates a completely randomized `PopBundle` at the specified position.
    pub fn random<R: Rng>(x: i32, y: i32, rng: &mut R) -> Self {
        Self {
            pop: Pop,
            name: generate_name(rng),
            pos: GridPosition { x, y },
            health: Health::default(),
            needs: Needs::default(),
            memories: Memories::default(),
            edict_compliance:
                crate::layer1::social::generational_dissonance::EdictCompliance::default(),
            skills: Skills::default(),
            appearance: Appearance::default(),
            speed: Speed::default(),
            action: PopAction::default(),
            equipment: Equipment::default(),
            weights: UtilityWeights::default(),
            knowledge: Knowledge::default(),
            age: Age::new(rng.gen_range(20..40)),
            faction: FactionMember::default(),
            improvised_tools: crate::layer1::improvised_tools::ImprovisedTools::default(),
            arrival: Arrival { tick: 0 },
            ethics: crate::layer1::social::indoctrination::PopEthics {
                ethic: match rng.gen_range(0..5) {
                    0 => crate::layer1::social::indoctrination::Ethic::Pacifist,
                    1 => crate::layer1::social::indoctrination::Ethic::Militarist,
                    2 => crate::layer1::social::indoctrination::Ethic::Collectivist,
                    3 => crate::layer1::social::indoctrination::Ethic::Individualist,
                    _ => crate::layer1::social::indoctrination::Ethic::FreeThinker,
                },
                stubbornness: rng.gen_range(0.2..1.0),
            },
            filth: Filth::default(),
            auras: ActiveAuras::default(),
            social_debt: SocialDebt::default(),
            morale: Morale::default(),
            contagion: ContagionCooldown::default(),
            traits: Traits::random(rng),
            cabin_fever: CabinFever::default(),
            biocompatibility: Biocompatibility::default(),
            wild_exposure: WildExposure::default(),
            admin_consumer: AdminConsumer { demand: 1.0 },
            dietary_history: DietaryHistory::default(),
            wallet: Wallet { credits: 50.0 },
            dialect: Dialect::default(),
            linguistics: Linguistics::default(),
            gut_biome: GutBiome::default(),
            education: EducationLevel(rng.gen_range(1..=3)),
            stats: crate::layer1::psychology::void_sickness::PopStats::default(),
            resonance_traits:
                crate::layer1::architecture::resonant_architecture::PopResonanceTraits::default(),
            base_resonance_traits:
                crate::layer1::architecture::resonant_architecture::PopBaseResonanceTraits::default(
                ),
        }
    }
}

/// Spawn 5 initial pops at random walkable positions.
///
/// This function attempts to find valid starting locations for the initial colony.
/// It uses a "Monte Carlo" approach:
///
/// 1.  Pick a random coordinate (x, y).
/// 2.  Check if it is walkable (not Water, not Rock).
/// 3.  If valid, spawn a Pop.
/// 4.  If invalid, retry up to `MAX_ATTEMPTS` (1000) times.
///
/// # Panics
///
/// This function does not panic, but it might fail to spawn all 5 pops if the map
/// is completely full of water/rock (though unlikely with 1000 attempts).
pub fn spawn_initial_pops(world: &mut World) {
    let mut rng = rand::thread_rng();
    spawn_initial_pops_internal(world, &mut rng);
}

fn spawn_initial_pops_internal<R: Rng>(world: &mut World, rng: &mut R) {
    const MAX_ATTEMPTS: usize = 1000;

    // Get dimensions first to release borrow
    let (width, height) = {
        let terrain = world.resource::<TerrainGrid>();
        (terrain.width, terrain.height)
    };

    // Safe bounds for GridPosition (i32)
    // If map is larger than i32::MAX, we just spawn within the i32 limit
    // because GridPosition cannot represent coordinates beyond that anyway.
    let max_x = i32::try_from(width).unwrap_or(i32::MAX);
    let max_y = i32::try_from(height).unwrap_or(i32::MAX);

    let mut spawned = 0;
    let mut attempts = 0;
    // Safety: we assume there is at least one walkable tile to avoid infinite loop.
    // In a real game we might want a timeout or more robust search.
    while spawned < 5 {
        attempts += 1;
        if attempts >= MAX_ATTEMPTS {
            break;
        }

        let x = rng.gen_range(0..max_x);
        let y = rng.gen_range(0..max_y);

        // Check terrain type in a separate scope to handle borrowing
        let is_walkable = {
            let terrain = world.resource::<TerrainGrid>();
            #[allow(clippy::cast_sign_loss)]
            terrain
                .get(x as usize, y as usize)
                .is_some_and(|t| t != TerrainType::Water && t != TerrainType::Rock)
        };

        if is_walkable {
            world.spawn(PopBundle::random(x, y, rng));
            if let Some(mut landing) =
                world.get_resource_mut::<crate::layer1::foundation_soil::LandingTiles>()
            {
                landing.coords.insert((x, y));
            }
            spawned += 1;
        }
    }
}

/// System to reset speed to base value before applying modifiers.
pub fn reset_speed_system(mut query: Query<&mut Speed>) {
    for mut speed in &mut query {
        speed.current = speed.base;
    }
}

/// Handles death events specific to Pops.
#[allow(clippy::type_complexity)]
pub fn handle_pop_death_system(
    mut pop_died_events: EventWriter<PopDied>,
    query: Query<
        (
            Entity,
            Option<&GridPosition>,
            Option<&PopName>,
            Option<&crate::layer1::health::DeathCause>,
        ),
        (With<Pop>, Added<Dead>),
    >,
    mut commands: Commands,
    mut log: Option<ResMut<MessageLog>>,
    mut shake: Option<ResMut<ScreenShake>>,
    mut hit_stop: Option<ResMut<GlobalHitStop>>,
    time: Option<Res<crate::shared::time::SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);

    for (entity, pos_opt, name_opt, cause_opt) in query.iter() {
        let name = name_opt.map_or_else(|| "Unknown".to_string(), |n| n.0.clone());
        // Item 84: in the full schedule, `attribute_death_causes_system`
        // (biology::death_attribution) has already resolved a specific
        // DeathCause for every Dead pop before this handler runs, so the
        // fallback below is only a canary for bare worlds that run this
        // handler alone — the DeathLedger counts it when it leaks.
        let reason = cause_opt
            .map(|cause| cause.0.clone())
            .unwrap_or_else(|| "Causes unknown".to_string());

        // 1. Spawn Corpse & Visuals
        if let Some(pos) = pos_opt {
            commands.spawn((
                Corpse {
                    name: name.clone(),
                    decay: 0.0,
                },
                *pos,
            ));

            // Soul Particle
            commands.spawn((
                crate::layer1::particles::Particle {
                    char: '@',
                    color: Color::Cyan,
                    lifetime: 20,
                },
                *pos,
            ));
        }

        // 2. Screen Shake & Hit Stop (Ludwig)
        if let Some(shake) = shake.as_mut() {
            shake.trigger(POP_DEATH_SCREEN_SHAKE);
        }
        if let Some(hs) = hit_stop.as_mut() {
            hs.trigger(POP_DEATH_HIT_STOP_TICKS);
        }

        // 3. Log
        if let Some(log) = log.as_mut() {
            log.add_colored(format!("DEATH: {name} has died!"), Color::Red);
        }

        // 4. Emit PopDied Event
        if let Some(pos) = pos_opt {
            // Spec 349: the death site becomes the pop's Grave tile. It is
            // occupied from birth — it is this pop's resting place; an
            // unoccupied grave would be reused by the burial system for a
            // second corpse, overwriting the name. The `AncestralGrave`
            // link records who lies here so kin (Relationships affinity
            // >= 80) can be recognised by the grave-visit system even
            // though the pop entity itself is about to be despawned.
            commands.spawn((
                crate::layer1::culture::funeral::Grave {
                    occupied: true,
                    corpse_name: Some(name.clone()),
                },
                crate::layer1::ancestral_graves::AncestralGrave {
                    pop_name: name.clone(),
                    original_entity: entity,
                },
                *pos,
            ));
        }
        pop_died_events.send(PopDied {
            entity,
            name,
            tick,
            reason,
        });

        // Note: We do NOT despawn here. Generic `despawn_dead_entities_system` handles it.
    }
}

/// System that adds `WitnessedDeath` memory to survivors when a Pop dies.
pub fn handle_witness_death_system(
    mut events: EventReader<PopDied>,
    mut query: Query<(Entity, &mut Memories), With<Pop>>,
) {
    for event in events.read() {
        // Parallel iterator could be used if we had Res<TaskPool>, but simplistic loop is fine for MVP
        for (entity, mut memories) in &mut query {
            if entity != event.entity {
                memories.add(MemoryType::WitnessedDeath, event.tick);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::terrain::{generate_terrain, TerrainGrid, TerrainType};

    #[test]
    fn test_pop_component_exists() {
        let mut world = World::new();
        let entity = world.spawn(Pop).id();

        assert!(world.get::<Pop>(entity).is_some());
    }

    #[test]
    fn test_spawn_initial_pops_count() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 5, "Should spawn exactly 5 pops");
    }

    #[test]
    fn test_spawn_initial_pops_have_positions() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let mut query = world.query::<(&Pop, &GridPosition)>();
        let all_have_positions = query.iter(&world).count() == 5;
        assert!(all_have_positions, "All pops should have GridPosition");
    }

    #[test]
    fn test_spawn_initial_pops_have_skills() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let mut query = world.query::<(&Pop, &Skills)>();
        let all_have_skills = query.iter(&world).count() == 5;
        assert!(all_have_skills, "All pops should have Skills component");
    }

    #[test]
    fn test_spawn_initial_pops_have_traits() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let mut query = world.query::<(&Pop, &Traits)>();
        let all_have_traits = query.iter(&world).count() == 5;
        assert!(all_have_traits, "All pops should have Traits component");
    }

    #[test]
    fn test_spawn_only_on_walkable_terrain() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        // Initialize query before borrowing resource to avoid conflict
        let mut query = world.query::<(&Pop, &GridPosition)>();
        let terrain = world.resource::<TerrainGrid>();

        for (_, pos) in query.iter(&world) {
            let x = usize::try_from(pos.x).expect("Pop x should be non-negative");
            let y = usize::try_from(pos.y).expect("Pop y should be non-negative");
            if let Some(tile_type) = terrain.get(x, y) {
                assert_ne!(tile_type, TerrainType::Water, "Pop spawned on water");
                assert_ne!(tile_type, TerrainType::Rock, "Pop spawned on rock");
            }
        }
    }

    #[test]
    fn test_spawn_within_terrain_bounds() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        for (_, pos) in world.query::<(&Pop, &GridPosition)>().iter(&world) {
            assert!(pos.x >= 0 && pos.x < 80, "Pop x out of bounds");
            assert!(pos.y >= 0 && pos.y < 50, "Pop y out of bounds");
        }
    }

    #[test]
    fn test_spawn_initial_pops_retries() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;

        let mut world = World::new();
        let width: usize = 10;
        let height = 10;
        let size = width.checked_mul(height).expect("Grid size overflow");
        assert!(size <= 10_000_000, "Grid size too large");
        let mut tiles = vec![TerrainType::Water; size];
        // Only one walkable tile
        tiles[0] = TerrainType::Grass;
        let terrain = TerrainGrid {
            width,
            height,
            tiles,
        };
        world.insert_resource(terrain);

        // Use internal spawning with seeded RNG to ensure we hit the 1/100 chance enough times
        // or effectively test the retry logic deterministically.
        // With Seed 42, we know it works.
        let mut rng = StdRng::seed_from_u64(42);
        spawn_initial_pops_internal(&mut world, &mut rng);

        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 5);

        // All pops should be at (0,0)
        for (_, pos) in world.query::<(&Pop, &GridPosition)>().iter(&world) {
            assert_eq!(pos.x, 0);
            assert_eq!(pos.y, 0);
        }
    }

    #[test]
    fn test_spawn_initial_pops_internal_with_custom_rng() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;

        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        // Use seeded RNG for determinism
        let mut rng = StdRng::seed_from_u64(42);
        spawn_initial_pops_internal(&mut world, &mut rng);

        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 5, "Should spawn exactly 5 pops");
    }

    #[test]
    fn test_spawn_with_mixed_terrain() {
        let mut world = World::new();
        let width: usize = 10;
        let height = 10;
        let size = width.checked_mul(height).expect("Grid size overflow");
        assert!(size <= 10_000_000, "Grid size too large");
        let mut tiles = vec![TerrainType::Grass; size];

        // Add some non-walkable tiles to force retries
        tiles[5] = TerrainType::Water;
        tiles[15] = TerrainType::Rock;
        tiles[25] = TerrainType::Water;
        tiles[35] = TerrainType::Rock;

        let terrain = TerrainGrid {
            width,
            height,
            tiles,
        };
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 5);

        // Verify no pops on water or rock
        let mut query = world.query::<(&Pop, &GridPosition)>();
        let terrain = world.resource::<TerrainGrid>();
        for (_, pos) in query.iter(&world) {
            let x = usize::try_from(pos.x).expect("x should be non-negative");
            let y = usize::try_from(pos.y).expect("y should be non-negative");
            if let Some(tile) = terrain.get(x, y) {
                assert_ne!(tile, TerrainType::Water);
                assert_ne!(tile, TerrainType::Rock);
            }
        }
    }

    #[test]
    fn test_spawn_internal_multiple_attempts() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;

        let mut world = World::new();
        let width: usize = 20;
        let height = 20;
        let size = width.checked_mul(height).expect("Grid size overflow");
        assert!(size <= 10_000_000, "Grid size too large");
        let mut tiles = vec![TerrainType::Water; size];

        // Create a sparse walkable area (only 20 out of 400 tiles)
        for i in 0..20 {
            tiles[i * 20] = TerrainType::Grass;
        }

        let terrain = TerrainGrid {
            width,
            height,
            tiles,
        };
        world.insert_resource(terrain);

        let mut rng = StdRng::seed_from_u64(123);
        spawn_initial_pops_internal(&mut world, &mut rng);

        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_spawn_on_grass() {
        let mut world = World::new();
        let tiles = vec![TerrainType::Grass; 100];
        let terrain = TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        };
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_spawn_on_dirt() {
        let mut world = World::new();
        let tiles = vec![TerrainType::Dirt; 100];
        let terrain = TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        };
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_pop_component_on_entity() {
        let mut world = World::new();
        let entity = world.spawn((Pop, GridPosition { x: 1, y: 2 })).id();

        assert!(world.get::<Pop>(entity).is_some());
        let pos = world.get::<GridPosition>(entity).unwrap();
        assert_eq!(pos.x, 1);
        assert_eq!(pos.y, 2);
    }

    use rand::RngCore;

    struct LimitedRng {
        count: usize,
        limit: usize,
    }

    impl RngCore for LimitedRng {
        fn next_u32(&mut self) -> u32 {
            #[allow(clippy::cast_possible_truncation)]
            {
                self.next_u64() as u32
            }
        }

        fn next_u64(&mut self) -> u64 {
            self.count += 1;
            assert!(
                self.count <= self.limit,
                "Too many RNG calls - infinite loop detected"
            );
            0
        }

        fn fill_bytes(&mut self, dest: &mut [u8]) {
            for b in dest {
                *b = 0;
            }
            self.next_u64();
        }

        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    #[test]
    fn test_spawn_initial_pops_terminates_on_full_map_repro() {
        let mut world = World::new();
        // 1x1 map
        let width: usize = 1;
        let height = 1;
        let size = width.checked_mul(height).expect("Grid size overflow");
        assert!(size <= 10_000_000, "Grid size too large");
        let tiles = vec![TerrainType::Water; size];
        let terrain = TerrainGrid {
            width,
            height,
            tiles,
        };
        world.insert_resource(terrain);

        let mut rng = LimitedRng {
            count: 0,
            limit: 2000,
        };
        spawn_initial_pops_internal(&mut world, &mut rng);

        // Verify that we didn't spawn anything (because map is full of water)
        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_pop_name_component() {
        let mut world = World::new();
        let entity = world.spawn(PopName("Ada".to_string())).id();
        let name = world.get::<PopName>(entity).unwrap();
        assert_eq!(name.0, "Ada");
    }

    #[test]
    fn test_spawn_initial_pops_have_names() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let mut query = world.query::<(&Pop, &PopName)>();
        let count = query.iter(&world).count();
        assert_eq!(count, 5, "All 5 pops should have names");
    }

    #[test]
    fn test_generated_names_not_empty() {
        let mut rng = rand::thread_rng();
        for _ in 0..50 {
            let name = generate_name(&mut rng);
            assert!(!name.0.is_empty(), "Generated name should not be empty");
        }
    }

    #[test]
    fn test_spawn_initial_pops_have_age() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let mut query = world.query::<(&Pop, &crate::layer1::Age)>();
        let count = query.iter(&world).count();
        assert_eq!(count, 5, "All 5 pops should have Age component");

        for (_, age) in query.iter(&world) {
            // Check age range (20-40 years)
            let years = age.ticks_alive / crate::layer1::balance::TICKS_PER_YEAR;
            assert!((20..40).contains(&years), "Age should be between 20 and 40");
        }
    }

    #[test]
    fn test_spawn_initial_pops_have_wild_exposure() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let mut query = world.query::<(&Pop, &WildExposure)>();
        let count = query.iter(&world).count();
        assert_eq!(count, 5, "All 5 pops should have WildExposure component");

        for (_, exposure) in query.iter(&world) {
            assert_eq!(exposure.current, 0.0);
        }
    }

    #[test]
    fn test_spawn_initial_pops_have_dietary_history() {
        let mut world = World::new();
        let terrain = generate_terrain(80, 50);
        world.insert_resource(terrain);

        spawn_initial_pops(&mut world);

        let mut query = world.query::<(&Pop, &crate::layer1::DietaryHistory)>();
        let count = query.iter(&world).count();
        assert_eq!(count, 5, "All 5 pops should have DietaryHistory component");
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use crate::layer1::chemical::{
        apply_chemical_speed_modifiers_system, ActiveEffect, ChemicalState, ChemicalType,
    };
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_speed_stable_with_reset() {
        let mut world = World::new();
        world.insert_resource(crate::shared::time::SimulationTime::default());

        let pop = world
            .spawn((
                Pop,
                Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
                ChemicalState {
                    active_effects: vec![ActiveEffect {
                        chemical: ChemicalType::Stim,
                        duration: 100,
                        magnitude: 1.5, // 1.5x multiplier
                    }],
                    addictions: vec![],
                },
            ))
            .id();

        // Run system cycle 1: Reset -> Modify
        world.run_system_once(reset_speed_system).unwrap();
        world
            .run_system_once(apply_chemical_speed_modifiers_system)
            .unwrap();
        let speed_1 = world.get::<Speed>(pop).unwrap().current;
        assert!(
            (speed_1 - 1.5).abs() < f32::EPSILON,
            "First run should be 1.5"
        );

        // Run system cycle 2: Reset -> Modify
        world.run_system_once(reset_speed_system).unwrap();
        world
            .run_system_once(apply_chemical_speed_modifiers_system)
            .unwrap();
        let speed_2 = world.get::<Speed>(pop).unwrap().current;

        // Should remain 1.5, NOT 2.25
        assert!(
            (speed_2 - 1.5).abs() < f32::EPSILON,
            "Speed should remain stable with reset"
        );
    }
}
