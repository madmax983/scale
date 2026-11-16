//! Spec 422: Reverse Terraforming — poisoning the well to get to the gold.
//!
//! Deploy Atmo-Strippers from orbit to intentionally degrade the
//! planet's habitability. Stripping removes flora (and disperses
//! fauna) pressure and, once the air is thin enough, exposes deep
//! crustal resources — Core-Gem deposits on the real terrain grid —
//! at the cost of a toxic wasteland: unsealed pops take toxic damage
//! through the real health pipeline, while pops in pressurized
//! (sealed) habitat or wearing a live EVA suit are protected.
//!
//! ## Grounding audit (spec §3/§4 mocks vs the real seams)
//!
//! * **`Habitability { value }` / `EnvironmentToxicity { level }` are
//!   MOCKS.** The real planetary environment is
//!   [`PlanetaryAtmosphere`][crate::layer1::environment::terraforming::PlanetaryAtmosphere]
//!   (`toxicity` 0.0 clean .. 1.0 toxic wasteland, already live in
//!   `setup.rs` and the Layer 1 environment schedule). Habitability
//!   is a pure derivation of it — [`habitability_for`] =
//!   `(1.0 - toxicity) * 100.0` — so the spec's 100.0 start is
//!   toxicity 0.0 and its 80.0 toxicity test is toxicity 0.8. No
//!   parallel habitability/toxicity state is created.
//! * **`CrustalResources { exposed }` is a MOCK as a component.**
//!   Exposure is real and grid-based (spec §7's preference): when
//!   toxicity first reaches [`EXPOSURE_TOXICITY_THRESHOLD`], the
//!   degradation system scans the real
//!   [`TerrainGrid`][crate::layer1::terrain::TerrainGrid] for
//!   Rock/DeepRock tiles and spawns real [`CoreGemDeposit`] entities
//!   on them (the `MotherLode` / `ExoticResourceDeposit` entity
//!   pattern), and [`ReverseTerraformState::exposed`] records that
//!   the crust is open. Harvesting a deposit pays the real
//!   [`ResourceType::CoreGem`] into the real
//!   [`ColonyResources`][crate::layer1::resources::ColonyResources]
//!   ledger (new variant + fields, following the Organs / DarkMote /
//!   AncientTech pattern) and sends the real `ResourceMinedEvent`.
//! * **`Pop { health }` is a MOCK.** Real pops carry
//!   [`Health`][crate::layer1::health::Health]; toxic damage goes
//!   through `Health::take_damage`, the same pipeline every other
//!   hazard uses.
//! * **`SealedSuit { is_sealed }` is a MOCK.** The real sealed seams
//!   are (a) the real
//!   [`PressureGrid`][crate::layer1::pressure::PressureGrid]: a pop
//!   standing at or above
//!   [`SUFFOCATION_PRESSURE_THRESHOLD`][crate::layer1::pressure::SUFFOCATION_PRESSURE_THRESHOLD]
//!   is inside pressurized habitat, and (b) a worn
//!   [`EvaSuit`][crate::layer1::eva_suit::EvaSuit] with air, reached
//!   through the real `Equipment.body` slot (the exact seam
//!   `pressure_damage_system` uses for vacuum). Either protects.
//!   The pre-existing `apply_planetary_effects_system` ambient leg
//!   (max ~0.025 HP/tick, no sealed check) is untouched — this
//!   module's [`toxicity_damage_system`] is the acute exposure leg.
//! * **Orbital deployment is real.** An Atmo-Stripper is an entity
//!   in orbit (`InOrbit` at the first orbital body when one exists,
//!   the privateer emissary pattern) carrying [`AtmoStripper`].
//! * **Tick-based, deterministic (spec §5).** Degradation is
//!   per-tick, not instantaneous; flora clearing scans the grid in
//!   row-major order and fauna dispersal takes the first fauna
//!   entity — no randomness anywhere, so no pure-rand flake.
//! * **Flora/fauna pressure removal.** While stripping, vegetation
//!   tiles (Tree/Shrub/Sapling) are scorched to Ash and Grass is
//!   parched to Dirt, a few tiles per stripper per tick; every
//!   [`FAUNA_DISPERSAL_INTERVAL`] stripping ticks the first live
//!   fauna entity disperses (despawns), counted in the ledger.

use bevy_ecs::prelude::*;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::environment::terraforming::PlanetaryAtmosphere;
use crate::layer1::eva_suit::EvaSuit;
use crate::layer1::fauna::Fauna;
use crate::layer1::health::Health;
use crate::layer1::items::Equipment;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::pressure::{PressureGrid, SUFFOCATION_PRESSURE_THRESHOLD};
use crate::layer1::resources::{ColonyResources, ResourceMinedEvent, ResourceType};
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer2::fleet::InOrbit;
use crate::layer2::system::OrbitalBody;
use crate::shared::log::MessageLog;
use crate::shared::time::SimulationTime;

/// Toxicity added per active stripper per tick. One stripper takes
/// a clean world (0.0) to the exposure threshold (0.30) in 60 ticks
/// and to full wasteland (1.0) in 200 ticks.
pub const TOXICITY_PER_STRIPPER_PER_TICK: f32 = 0.005;
/// Toxicity at or above which the crust is exposed: habitability
/// has fallen to 70 or below and Core-Gem deposits surface.
pub const EXPOSURE_TOXICITY_THRESHOLD: f32 = 0.30;
/// Toxicity above which unsealed pops take acute toxic damage
/// (matches the pre-existing ambient leg's 0.5 threshold).
pub const TOXIC_DAMAGE_THRESHOLD: f32 = 0.5;
/// Acute toxic damage scale: damage per tick is
/// `(toxicity - 0.5) * 10.0` — 3.0/tick at toxicity 0.8 (the spec
/// sketch's high-toxicity world), 5.0/tick at 1.0 (its sketch rate).
pub const TOXIC_DAMAGE_SCALE: f32 = 10.0;
/// Vegetation/ground tiles stripped per stripper per tick.
pub const STRIP_TILES_PER_STRIPPER_PER_TICK: usize = 2;
/// Maximum Core-Gem deposits exposed in one exposure event.
pub const MAX_EXPOSED_DEPOSITS: usize = 12;
/// Core-Gems held by one exposed deposit.
pub const CORE_GEMS_PER_DEPOSIT: f32 = 3.0;
/// Stripping ticks between fauna dispersals while strippers run.
pub const FAUNA_DISPERSAL_INTERVAL: u64 = 25;

/// Habitability (0..100) for a planetary toxicity (0..1): the pure
/// derivation the spec's `Habitability` mock collapses into.
#[must_use]
pub fn habitability_for(toxicity: f32) -> f32 {
    (1.0 - toxicity.clamp(0.0, 1.0)) * 100.0
}

/// Acute toxic damage per tick at `toxicity` for an unsealed pop.
/// Zero at or below [`TOXIC_DAMAGE_THRESHOLD`]. Pure.
#[must_use]
pub fn toxic_damage_for(toxicity: f32) -> f32 {
    let excess = toxicity.clamp(0.0, 1.0) - TOXIC_DAMAGE_THRESHOLD;
    if excess > 0.0 {
        excess * TOXIC_DAMAGE_SCALE
    } else {
        0.0
    }
}

/// Whether `terrain` is flora/ground cover stripping removes.
#[must_use]
pub const fn is_strippable(terrain: TerrainType) -> bool {
    matches!(
        terrain,
        TerrainType::Tree | TerrainType::Shrub | TerrainType::Sapling | TerrainType::Grass
    )
}

/// What a stripped tile becomes: vegetation scorches to Ash,
/// grassland parches to Dirt.
#[must_use]
pub const fn stripped_form(terrain: TerrainType) -> TerrainType {
    match terrain {
        TerrainType::Tree | TerrainType::Shrub | TerrainType::Sapling => TerrainType::Ash,
        _ => TerrainType::Dirt,
    }
}

/// An Atmo-Stripper in orbit: while present, the degradation
/// system runs. Spawned by [`deploy_atmo_stripper`], removed by
/// [`recall_atmo_strippers`].
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct AtmoStripper {
    /// Total ticks this stripper has run (diagnostics).
    pub ticks_active: u64,
}

/// A deep crustal Core-Gem deposit exposed by stripping, sitting
/// on a real Rock/DeepRock tile of the terrain grid.
#[derive(Component, Debug, Clone, Copy)]
pub struct CoreGemDeposit {
    /// Core-Gems remaining in the deposit.
    pub amount: f32,
}

/// Colony-wide reverse-terraforming ledger (Spec 422).
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct ReverseTerraformState {
    /// Strippers ever deployed.
    pub deployed: u64,
    /// Strippers recalled.
    pub recalled: u64,
    /// Ticks with at least one stripper active.
    pub stripping_ticks: u64,
    /// Flora/ground tiles stripped.
    pub tiles_stripped: u64,
    /// Fauna dispersed by the stripping.
    pub fauna_dispersed: u64,
    /// True once toxicity first reached the exposure threshold.
    pub exposed: bool,
    /// Core-Gem deposits exposed (all time).
    pub deposits_exposed: u64,
    /// Core-Gems harvested into the colony ledger (all time).
    pub gems_harvested: f32,
    /// The toxic-threshold chronicle has fired (hysteresis latch).
    pub toxic_warned: bool,
}

fn ensure_state(world: &mut World) {
    if world.get_resource::<ReverseTerraformState>().is_none() {
        world.insert_resource(ReverseTerraformState::default());
    }
    if world.get_resource::<PlanetaryAtmosphere>().is_none() {
        world.insert_resource(PlanetaryAtmosphere {
            toxicity: 0.0,
            temperature: 0.0,
        });
    }
}

fn current_tick(world: &World) -> u64 {
    world.get_resource::<SimulationTime>().map_or(0, |t| t.tick)
}

fn chronicle(world: &mut World, text: String, importance: EventImportance) {
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(text.clone());
    }
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent { text, importance });
    }
}

/// Deploy one Atmo-Stripper into orbit. Returns its entity.
pub fn deploy_atmo_stripper(world: &mut World) -> Entity {
    ensure_state(world);
    let parent = {
        let mut query = world.query_filtered::<Entity, With<OrbitalBody>>();
        query.iter(world).next()
    };
    let mut entity = world.spawn(AtmoStripper::default());
    if let Some(parent) = parent {
        entity.insert(InOrbit { parent });
    }
    let id = entity.id();
    if let Some(mut state) = world.get_resource_mut::<ReverseTerraformState>() {
        state.deployed += 1;
    }
    chronicle(
        world,
        "An Atmo-Stripper takes station in orbit and begins to peel the sky away. The air will thin, the green will die back, and the deep crust will show what it was hiding.".to_string(),
        EventImportance::Major,
    );
    let _ = current_tick(world);
    id
}

/// Recall every Atmo-Stripper. Degradation stops; the toxicity
/// already done stays (the existing AtmosphericProcessor pipeline
/// is the slow way back). Returns how many were recalled.
pub fn recall_atmo_strippers(world: &mut World) -> usize {
    ensure_state(world);
    let strippers: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<AtmoStripper>>();
        query.iter(world).collect()
    };
    let count = strippers.len();
    for entity in strippers {
        world.despawn(entity);
    }
    if count > 0 {
        if let Some(mut state) = world.get_resource_mut::<ReverseTerraformState>() {
            state.recalled += u64::try_from(count).unwrap_or(0);
        }
        chronicle(
            world,
            "The Atmo-Strippers withdraw from orbit. The sky stops thinning — what was stripped stays stripped.".to_string(),
            EventImportance::Major,
        );
    }
    count
}

/// Expose Core-Gem deposits on the real terrain grid: deterministic
/// row-major scan for Rock/DeepRock tiles without a deposit yet,
/// up to [`MAX_EXPOSED_DEPOSITS`]. Returns deposits spawned.
fn expose_core_gem_deposits(world: &mut World) -> usize {
    let positions: Vec<GridPosition> = {
        // Snapshot rock positions first (drops the grid borrow),
        // then filter out tiles that already carry a deposit.
        let rock_positions: Vec<GridPosition> = {
            let Some(grid) = world.get_resource::<TerrainGrid>() else {
                return 0;
            };
            let mut rocks = Vec::new();
            for y in 0..grid.height {
                for x in 0..grid.width {
                    let Some(tile) = grid.get(x, y) else { continue };
                    if matches!(tile, TerrainType::Rock | TerrainType::DeepRock) {
                        if let (Ok(ix), Ok(iy)) = (i32::try_from(x), i32::try_from(y)) {
                            rocks.push(GridPosition { x: ix, y: iy });
                        }
                    }
                }
            }
            rocks
        };
        let taken: std::collections::HashSet<(i32, i32)> = {
            let mut query = world.query::<(&CoreGemDeposit, &GridPosition)>();
            query.iter(world).map(|(_, p)| (p.x, p.y)).collect()
        };
        rock_positions
            .into_iter()
            .filter(|p| !taken.contains(&(p.x, p.y)))
            .take(MAX_EXPOSED_DEPOSITS)
            .collect()
    };
    let count = positions.len();
    for pos in positions {
        world.spawn((
            CoreGemDeposit {
                amount: CORE_GEMS_PER_DEPOSIT,
            },
            pos,
        ));
    }
    count
}

/// Harvest one unit from a deposit into the real colony ledger.
/// Returns false when the deposit is gone/empty or the ledger is
/// full (the deposit keeps the gem — no silent loss).
pub fn harvest_core_gem_deposit(world: &mut World, deposit: Entity) -> bool {
    let Some(amount) = world.get::<CoreGemDeposit>(deposit).map(|d| d.amount) else {
        return false;
    };
    if amount <= 0.0 {
        return false;
    }
    let room = world.get_resource::<ColonyResources>().is_some_and(|r| {
        r.get_amount(ResourceType::CoreGem) < r.get_max_amount(ResourceType::CoreGem)
    });
    if !room {
        return false;
    }
    if let Some(mut resources) = world.get_resource_mut::<ColonyResources>() {
        resources.add_resource(&ResourceType::CoreGem, 1.0);
    }
    let remaining = {
        let mut dep = world.get_mut::<CoreGemDeposit>(deposit).unwrap();
        dep.amount -= 1.0;
        dep.amount
    };
    if let Some(mut state) = world.get_resource_mut::<ReverseTerraformState>() {
        state.gems_harvested += 1.0;
    }
    if let Some(mut events) = world.get_resource_mut::<Events<ResourceMinedEvent>>() {
        events.send(ResourceMinedEvent {
            resource_type: ResourceType::CoreGem,
            amount: 1,
        });
    }
    if remaining <= 0.0 {
        world.despawn(deposit);
    }
    true
}

/// Harvest one unit from the first live deposit (headless fast path).
pub fn harvest_next_core_gem(world: &mut World) -> bool {
    ensure_state(world);
    let first = {
        let mut query = world.query_filtered::<Entity, With<CoreGemDeposit>>();
        query.iter(world).next()
    };
    first.is_some_and(|entity| harvest_core_gem_deposit(world, entity))
}

/// Spec 422 degradation (exclusive system): while any stripper is
/// active, toxicity rises per tick, flora/ground tiles are stripped
/// in deterministic row-major order, fauna disperses on an interval,
/// and crossing the exposure threshold surfaces Core-Gem deposits.
pub fn reverse_terraforming_system(world: &mut World) {
    if world.get_resource::<ReverseTerraformState>().is_none()
        || world.get_resource::<PlanetaryAtmosphere>().is_none()
    {
        return;
    }
    let stripper_count = {
        let mut query = world.query_filtered::<Entity, With<AtmoStripper>>();
        query.iter(world).count()
    };
    if stripper_count == 0 {
        return;
    }
    // 1. Degrade the real planetary atmosphere, tick by tick.
    let (toxicity, crossed_exposure, crossed_toxic) = {
        let (already_exposed, already_warned) = {
            let state = world.resource::<ReverseTerraformState>();
            (state.exposed, state.toxic_warned)
        };
        let mut atmo = world.resource_mut::<PlanetaryAtmosphere>();
        let before = atmo.toxicity;
        atmo.toxicity = (atmo.toxicity + TOXICITY_PER_STRIPPER_PER_TICK * stripper_count as f32)
            .clamp(0.0, 1.0);
        // Exposure/toxic latches are state-based, not crossing-based:
        // the default world starts at toxicity 0.8 (PlanetaryAtmosphere
        // default), already above both thresholds, so a crossing-only
        // check would never fire in a real game (live-playtest find).
        let _ = before;
        (
            atmo.toxicity,
            atmo.toxicity >= EXPOSURE_TOXICITY_THRESHOLD && !already_exposed,
            atmo.toxicity > TOXIC_DAMAGE_THRESHOLD && !already_warned,
        )
    };
    let stripping_tick = {
        let mut state = world.resource_mut::<ReverseTerraformState>();
        state.stripping_ticks += 1;
        state.stripping_ticks
    };
    {
        let mut query = world.query::<&mut AtmoStripper>();
        for mut stripper in query.iter_mut(world) {
            stripper.ticks_active += 1;
        }
    }
    // 2. Strip flora/ground tiles, row-major, a few per stripper.
    let budget = STRIP_TILES_PER_STRIPPER_PER_TICK * stripper_count;
    let mut stripped = 0usize;
    if let Some(mut grid) = world.get_resource_mut::<TerrainGrid>() {
        'outer: for y in 0..grid.height {
            for x in 0..grid.width {
                if stripped >= budget {
                    break 'outer;
                }
                if let Some(tile) = grid.get(x, y) {
                    if is_strippable(tile) {
                        grid.set(x, y, stripped_form(tile));
                        stripped += 1;
                    }
                }
            }
        }
    }
    if stripped > 0 {
        if let Some(mut state) = world.get_resource_mut::<ReverseTerraformState>() {
            state.tiles_stripped += u64::try_from(stripped).unwrap_or(0);
        }
    }
    // 3. Fauna dispersal on a deterministic interval.
    if stripping_tick % FAUNA_DISPERSAL_INTERVAL == 0 {
        let first_fauna = {
            let mut query = world.query_filtered::<Entity, With<Fauna>>();
            query.iter(world).next()
        };
        if let Some(fauna) = first_fauna {
            world.despawn(fauna);
            if let Some(mut state) = world.get_resource_mut::<ReverseTerraformState>() {
                state.fauna_dispersed += 1;
            }
        }
    }
    // 4. Exposure: the crust opens, Core-Gems surface.
    if crossed_exposure {
        let spawned = expose_core_gem_deposits(world);
        if let Some(mut state) = world.get_resource_mut::<ReverseTerraformState>() {
            state.exposed = true;
            state.deposits_exposed += u64::try_from(spawned).unwrap_or(0);
        }
        chronicle(
            world,
            format!(
                "The thinned sky can no longer hide the deep crust: {spawned} Core-Gem deposits stand exposed on the bare rock. Habitability has fallen to {:.0}.",
                habitability_for(toxicity)
            ),
            EventImportance::Major,
        );
    }
    if crossed_toxic {
        if let Some(mut state) = world.get_resource_mut::<ReverseTerraformState>() {
            state.toxic_warned = true;
        }
        chronicle(
            world,
            "The air has turned toxic. Unsealed colonists are breathing poison — only pressurized habitat and live suits keep the stripping from killing the miners it was meant to enrich.".to_string(),
            EventImportance::Major,
        );
    }
}

/// Spec 422 acute toxicity (regular system): unsealed pops take
/// damage through the real [`Health`] pipeline while toxicity is
/// above [`TOXIC_DAMAGE_THRESHOLD`]. A pop is sealed when it stands
/// in pressurized habitat (real [`PressureGrid`] at/above the
/// suffocation threshold) or wears an [`EvaSuit`] with air through
/// the real `Equipment.body` slot.
pub fn toxicity_damage_system(
    atmosphere: Option<Res<PlanetaryAtmosphere>>,
    grid_opt: Option<Res<PressureGrid>>,
    mut pops: Query<(&GridPosition, &mut Health, Option<&Equipment>), With<Pop>>,
    suits: Query<&EvaSuit>,
) {
    let Some(atmosphere) = atmosphere else { return };
    let damage = toxic_damage_for(atmosphere.toxicity);
    if damage <= 0.0 {
        return;
    }
    for (pos, mut health, equipment) in pops.iter_mut() {
        if health.current <= 0.0 {
            continue;
        }
        let in_sealed_habitat = grid_opt
            .as_deref()
            .is_some_and(|grid| grid.get(pos.x, pos.y) >= SUFFOCATION_PRESSURE_THRESHOLD);
        if in_sealed_habitat {
            continue;
        }
        let suited = equipment
            .and_then(|eq| eq.body)
            .and_then(|body| suits.get(body).ok())
            .is_some_and(|suit| suit.air > 0.0);
        if suited {
            continue;
        }
        health.take_damage(damage);
    }
}

/// Census of strippers and deposits for STATS / the headless panel.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ReverseTerraformCensus {
    /// Live Atmo-Strippers in orbit.
    pub strippers: usize,
    /// Live Core-Gem deposits on the grid.
    pub deposits: usize,
    /// Core-Gems remaining across live deposits.
    pub gems_in_ground: f32,
}

/// Count live strippers and deposits.
#[must_use]
pub fn reverse_terraform_census(world: &mut World) -> ReverseTerraformCensus {
    let strippers = {
        let mut query = world.query_filtered::<Entity, With<AtmoStripper>>();
        query.iter(world).count()
    };
    let (deposits, gems_in_ground) = {
        let mut query = world.query::<&CoreGemDeposit>();
        let mut count = 0usize;
        let mut gems = 0.0f32;
        for deposit in query.iter(world) {
            count += 1;
            gems += deposit.amount;
        }
        (count, gems)
    };
    ReverseTerraformCensus {
        strippers,
        deposits,
        gems_in_ground,
    }
}

/// Compact STATS label, e.g. `stripping:tox0.65/dep3/gems2`.
/// No spaces.
#[must_use]
pub fn reverse_terraform_stats_label(world: &mut World) -> String {
    let census = reverse_terraform_census(world);
    let toxicity = world
        .get_resource::<PlanetaryAtmosphere>()
        .map_or(0.0, |a| a.toxicity);
    let (exposed, harvested) = world
        .get_resource::<ReverseTerraformState>()
        .map_or((false, 0.0), |s| (s.exposed, s.gems_harvested));
    if census.strippers > 0 {
        format!(
            "stripping:tox{toxicity:.2}/dep{}/gems{harvested:.0}",
            census.deposits
        )
    } else if exposed {
        format!(
            "exposed:tox{toxicity:.2}/dep{}/gems{harvested:.0}",
            census.deposits
        )
    } else if census.deposits > 0 {
        format!("deposits:{}", census.deposits)
    } else {
        "none".to_string()
    }
}

/// Player-facing strings introduced by Spec 422 (IP-guard input).
#[must_use]
pub fn reverse_terraform_public_strings() -> Vec<String> {
    vec![
        "Atmo-Stripper".to_string(),
        "Core-Gem".to_string(),
        "Reverse Terraforming".to_string(),
        "An Atmo-Stripper takes station in orbit and begins to peel the sky away. The air will thin, the green will die back, and the deep crust will show what it was hiding.".to_string(),
        "The thinned sky can no longer hide the deep crust".to_string(),
        "The air has turned toxic. Unsealed colonists are breathing poison".to_string(),
        "The Atmo-Strippers withdraw from orbit. The sky stops thinning".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::terrain::generate_terrain;

    fn base_world(toxicity: f32) -> World {
        let mut world = World::new();
        world.insert_resource(PlanetaryAtmosphere {
            toxicity,
            temperature: 0.0,
        });
        world.insert_resource(ReverseTerraformState::default());
        world.insert_resource(SimulationTime::default());
        world.insert_resource(MessageLog::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<ResourceMinedEvent>>();
        world
    }

    fn grid_with_rock() -> TerrainGrid {
        let mut grid = TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        };
        grid.set(2, 2, TerrainType::Rock);
        grid.set(3, 3, TerrainType::DeepRock);
        grid.set(4, 4, TerrainType::Rock);
        grid.set(0, 0, TerrainType::Tree);
        grid.set(1, 0, TerrainType::Tree);
        grid
    }

    fn run_degradation(world: &mut World, ticks: usize) {
        let mut schedule = Schedule::default();
        schedule.add_systems(reverse_terraforming_system);
        for _ in 0..ticks {
            schedule.run(world);
        }
    }

    fn run_toxicity(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(toxicity_damage_system);
        schedule.run(world);
    }

    #[test]
    fn test_habitability_math() {
        assert_eq!(habitability_for(0.0), 100.0);
        assert_eq!(habitability_for(1.0), 0.0);
        assert!((habitability_for(0.8) - 20.0).abs() < 0.01);
        assert_eq!(toxic_damage_for(0.5), 0.0);
        assert!((toxic_damage_for(0.8) - 3.0).abs() < 0.01);
        assert!((toxic_damage_for(1.0) - 5.0).abs() < 0.01);
    }

    /// Spec RED test 1, adapted to the real seams: the sketch's
    /// `Habitability` component is the derived
    /// [`habitability_for`] over the real `PlanetaryAtmosphere`,
    /// its `CrustalResources.exposed` is deposits on the real
    /// `TerrainGrid` plus the ledger flag, and per spec §5 the
    /// degradation is tick-based — one tick must NOT expose.
    #[test]
    fn test_atmo_stripper_degrades_habitability_and_exposes_resources() {
        let mut world = base_world(0.0);
        world.insert_resource(grid_with_rock());
        assert_eq!(
            habitability_for(world.resource::<PlanetaryAtmosphere>().toxicity),
            100.0
        );
        deploy_atmo_stripper(&mut world);
        run_degradation(&mut world, 1);
        let after_one = world.resource::<PlanetaryAtmosphere>().toxicity;
        assert!(habitability_for(after_one) < 100.0, "habitability falls");
        assert!(
            habitability_for(after_one) > 99.0,
            "tick-based, not instant"
        );
        assert!(
            !world.resource::<ReverseTerraformState>().exposed,
            "one tick must not expose the crust"
        );
        run_degradation(&mut world, 100);
        let toxicity = world.resource::<PlanetaryAtmosphere>().toxicity;
        assert!(habitability_for(toxicity) < 100.0);
        assert!(toxicity >= EXPOSURE_TOXICITY_THRESHOLD);
        let state = world.resource::<ReverseTerraformState>();
        assert!(state.exposed, "deep crustal resources become exposed");
        assert!(state.deposits_exposed >= 1);
        let census = reverse_terraform_census(&mut world);
        assert!(census.deposits >= 1, "real deposits on the real grid");
        assert!(census.gems_in_ground >= CORE_GEMS_PER_DEPOSIT);
        // Deposits sit on rock tiles.
        let mut query = world.query::<(&CoreGemDeposit, &GridPosition)>();
        let grid = world.resource::<TerrainGrid>();
        for (_, pos) in query.iter(&world) {
            let tile = grid
                .get(
                    usize::try_from(pos.x).unwrap(),
                    usize::try_from(pos.y).unwrap(),
                )
                .unwrap();
            assert!(matches!(tile, TerrainType::Rock | TerrainType::DeepRock));
        }
    }

    /// Spec RED test 2, adapted: the sketch's `Pop { health }` is
    /// the real `Health`, and `SealedSuit` is the real sealed seam —
    /// pressurized habitat (PressureGrid) or a live EVA suit.
    #[test]
    fn test_toxic_environment_damages_unsealed_pops() {
        let mut world = base_world(0.8);
        let mut grid = PressureGrid::new(10, 10);
        grid.set(1, 1, 1.0);
        world.insert_resource(grid);
        let unsealed = world
            .spawn((Pop, Health::default(), GridPosition { x: 0, y: 0 }))
            .id();
        let sealed_habitat = world
            .spawn((Pop, Health::default(), GridPosition { x: 1, y: 1 }))
            .id();
        let suit = world
            .spawn(EvaSuit {
                class: crate::layer1::eva_suit::SuitClass::Base,
                air: 50.0,
                durability: 100.0,
                max_durability: 100.0,
                colony_issued: true,
            })
            .id();
        let suited = world
            .spawn((
                Pop,
                Health::default(),
                GridPosition { x: 2, y: 2 },
                Equipment {
                    body: Some(suit),
                    ..Equipment::default()
                },
            ))
            .id();
        run_toxicity(&mut world);
        assert!(
            world.get::<Health>(unsealed).unwrap().current < 100.0,
            "unsealed pop takes toxic damage"
        );
        assert!(
            (world.get::<Health>(unsealed).unwrap().current - 97.0).abs() < 0.01,
            "damage is (0.8 - 0.5) * 10 = 3.0"
        );
        assert_eq!(world.get::<Health>(sealed_habitat).unwrap().current, 100.0);
        assert_eq!(world.get::<Health>(suited).unwrap().current, 100.0);
    }

    /// Regression (live playtest): the default world starts at
    /// toxicity 0.8, already above the exposure threshold — the
    /// first stripping tick must expose, not wait for a crossing
    /// that already happened before deployment.
    #[test]
    fn test_exposure_fires_from_default_hostile_start() {
        let mut world = base_world(0.8);
        world.insert_resource(grid_with_rock());
        deploy_atmo_stripper(&mut world);
        run_degradation(&mut world, 1);
        assert!(world.resource::<ReverseTerraformState>().exposed);
        assert!(reverse_terraform_census(&mut world).deposits >= 1);
        assert!(world.resource::<ReverseTerraformState>().toxic_warned);
    }

    #[test]
    fn test_no_stripper_no_degradation() {
        let mut world = base_world(0.1);
        world.insert_resource(grid_with_rock());
        run_degradation(&mut world, 50);
        assert_eq!(world.resource::<PlanetaryAtmosphere>().toxicity, 0.1);
        assert_eq!(world.resource::<ReverseTerraformState>().tiles_stripped, 0);
    }

    #[test]
    fn test_stripping_clears_flora_deterministically() {
        let mut world = base_world(0.0);
        world.insert_resource(grid_with_rock());
        deploy_atmo_stripper(&mut world);
        run_degradation(&mut world, 1);
        // Row-major: (0,0) Tree -> Ash is stripped first.
        let grid = world.resource::<TerrainGrid>();
        assert_eq!(grid.get(0, 0), Some(TerrainType::Ash));
        assert!(world.resource::<ReverseTerraformState>().tiles_stripped >= 1);
        let _ = generate_terrain(4, 4); // grid helper stays exercised
    }

    #[test]
    fn test_recall_stops_degradation() {
        let mut world = base_world(0.0);
        world.insert_resource(grid_with_rock());
        deploy_atmo_stripper(&mut world);
        run_degradation(&mut world, 10);
        let mid = world.resource::<PlanetaryAtmosphere>().toxicity;
        assert!(mid > 0.0);
        assert_eq!(recall_atmo_strippers(&mut world), 1);
        run_degradation(&mut world, 10);
        assert_eq!(world.resource::<PlanetaryAtmosphere>().toxicity, mid);
    }

    #[test]
    fn test_core_gem_harvest_pays_into_real_ledger() {
        let mut world = base_world(0.6);
        world.insert_resource(ColonyResources {
            max_core_gems: 50.0,
            ..ColonyResources::default()
        });
        let deposit = world
            .spawn((CoreGemDeposit { amount: 2.0 }, GridPosition { x: 2, y: 2 }))
            .id();
        assert!(harvest_core_gem_deposit(&mut world, deposit));
        assert!(harvest_core_gem_deposit(&mut world, deposit));
        assert_eq!(
            world
                .resource::<ColonyResources>()
                .get_amount(ResourceType::CoreGem),
            2.0
        );
        assert!(
            world.get::<CoreGemDeposit>(deposit).is_none(),
            "spent deposit despawns"
        );
        assert!(!harvest_core_gem_deposit(&mut world, deposit));
        assert_eq!(
            world.resource::<ReverseTerraformState>().gems_harvested,
            2.0
        );
    }

    #[test]
    fn test_ip_guard_no_lifted_names() {
        let banned = [
            "arrakis",
            "sandworm",
            "pandora",
            "weyland",
            "red mars",
            "gaia project",
            "dune",
            "spice melange",
            "xenomorph",
        ];
        for text in reverse_terraform_public_strings() {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(!lower.contains(term), "banned term {term} in: {text}");
            }
        }
        let joined = reverse_terraform_public_strings().join(" ");
        assert!(joined.contains("Atmo-Stripper"));
        assert!(joined.contains("Core-Gem"));
    }
}
