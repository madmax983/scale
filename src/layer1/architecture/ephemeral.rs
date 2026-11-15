//! Spec 420: Ephemeral Architecture — cheap, abundant materials that the
//! environment takes back.
//!
//! # Fantasy
//! Building castles of ice and salt. Temporary solutions for permanent
//! problems: an ice fortress raised in winter wins the raid, then summer
//! melts it and the prisoners walk out. Speed and cost versus permanence
//! and environmental vulnerability.
//!
//! # Grounding (spec §3/§4 mocks vs the real seams)
//! The spec sketch invents `Structure { health }`, a `Temperature`
//! resource, and a `Weather { is_raining }` resource. None of those exist
//! on trunk; the real seams are:
//! * Structure integrity is [`Structure`](super::structure::Structure)
//!   (`current_hp` / `max_hp`) — the same pool fire, entropy, and fragile
//!   decay already drain. Pop [`Health`](crate::layer1::health::Health)
//!   is biological only, so the spec §7 advice ("route through
//!   `Health::take_damage`") does not apply to buildings; building damage
//!   on trunk is direct `current_hp` arithmetic, and this module follows
//!   the fire/entropy pattern exactly.
//! * Temperature is the per-tile
//!   [`TemperatureGrid`](crate::layer1::temperature::TemperatureGrid)
//!   (`get_safe`), falling back to the season's base temperature
//!   ([`Season::base_temperature`](crate::layer1::seasons::Season)) when
//!   no grid exists (bare test worlds).
//! * Weather is [`WeatherState`](crate::layer1::weather::WeatherState) /
//!   [`WeatherType`](crate::layer1::weather::WeatherType): "raining" is
//!   `Rain`, `Storm`, or `MutagenicRain` (it is, in fact, raining).
//! * The carrier is any building entity with a
//!   [`Building`](super::building::Building) + `Structure` +
//!   `GridPosition`; [`EphemeralMaterial`] is the tag. No new
//!   `BuildingType` variants: the build-menu enum is exhaustively matched
//!   across the codebase, and the material is an orthogonal property —
//!   an ice *Wall*, a salt *Wall*, a packed-mud *Wall*.
//!
//! # Destruction vs recovery (spec §5 REFACTOR choice)
//! Fire and sabotage leave a `Ruin` + `ScrapHeap` because something
//! solid remains. Ephemeral decay is *dissolution*: ice becomes water,
//! salt becomes brine, mud becomes soil. There is nothing to scavenge
//! and no rubble to clear, so destruction here despawns the building,
//! frees its tile (`OccupiedTiles` + `BuildingMap`), emits the real
//! [`BuildingRemovedEvent`], counts the loss in [`EphemeralLedger`],
//! and writes a Standard chronicle entry — but spawns **no Ruin and no
//! ScrapHeap**. The tile is immediately reusable: that instant reuse is
//! the flip side of the cheap/fast bargain, and the zero salvage is its
//! price.
//!
//! # Decay constants (new, Spec 420 only — no unrelated retunes)
//! * Ice melts above 0°C: [`ICE_MELT_BASE`] + [`ICE_MELT_PER_DEGREE`]
//!   per degree (0.35/tick at 30°C summer heat, 0.05/tick just above
//!   freezing). At or below 0°C ice is permanent.
//! * Salt dissolves in wet weather: [`SALT_RAIN_RATE`] in rain,
//!   [`SALT_STORM_RATE`] in a storm. Dry salt is permanent.
//! * Packed Mud washes out slower than salt ([`MUD_RAIN_RATE`] /
//!   [`MUD_STORM_RATE`]) but also crumbles at [`MUD_EROSION_RATE`] in
//!   *all* weather — packed mud is never truly permanent.

use super::building::{Building, BuildingType, MaterialType, OccupiedTiles};
use super::structure::Structure;
use crate::layer1::core::events::BuildingRemovedEvent;
use crate::layer1::map::GridPosition;
use crate::layer1::seasons::SeasonState;
use crate::layer1::temperature::TemperatureGrid;
use crate::layer1::weather::{WeatherState, WeatherType};
use crate::shared::log::MessageLog;
use bevy_ecs::prelude::*;

/// Ice melt at 1°C above freezing, per tick (Spec 420).
pub const ICE_MELT_BASE: f32 = 0.05;
/// Additional ice melt per degree Celsius above freezing, per tick.
pub const ICE_MELT_PER_DEGREE: f32 = 0.01;
/// Salt dissolution per tick in rain (Spec 420).
pub const SALT_RAIN_RATE: f32 = 0.4;
/// Salt dissolution per tick in a storm (Spec 420).
pub const SALT_STORM_RATE: f32 = 0.8;
/// Packed-mud washout per tick in rain (Spec 420).
pub const MUD_RAIN_RATE: f32 = 0.2;
/// Packed-mud washout per tick in a storm (Spec 420).
pub const MUD_STORM_RATE: f32 = 0.4;
/// Packed-mud dry crumbling per tick in any weather (Spec 420).
pub const MUD_EROSION_RATE: f32 = 0.01;

/// A building material the environment reclaims (Spec 420).
///
/// Attached to a building entity alongside its
/// [`Structure`](super::structure::Structure); the decay system drains
/// that real HP pool and dissolves the building at zero.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EphemeralMaterial {
    /// Frozen water. Melts when its tile is above 0°C.
    Ice,
    /// Crystallised salt. Dissolves in rain and storms.
    Salt,
    /// Packed mud. Washes out in wet weather, crumbles slowly always.
    PackedMud,
}

impl EphemeralMaterial {
    /// Player-facing label.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Ice => "Ice",
            Self::Salt => "Salt",
            Self::PackedMud => "Packed Mud",
        }
    }

    /// Parse a headless/debug material name (case-insensitive).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "ice" => Some(Self::Ice),
            "salt" => Some(Self::Salt),
            "mud" | "packedmud" | "packed_mud" | "packed-mud" => Some(Self::PackedMud),
            _ => None,
        }
    }

    /// Chronicle verb for this material's dissolution.
    #[must_use]
    pub const fn dissolution_verb(&self) -> &'static str {
        match self {
            Self::Ice => "melted away",
            Self::Salt => "dissolved in the wet",
            Self::PackedMud => "washed away",
        }
    }
}

/// True when the weather is wet (rain, storm, or mutagenic rain).
#[must_use]
pub const fn is_wet(weather: WeatherType) -> bool {
    matches!(
        weather,
        WeatherType::Rain | WeatherType::Storm | WeatherType::MutagenicRain
    )
}

/// True when the weather is a full storm.
#[must_use]
pub const fn is_storm(weather: WeatherType) -> bool {
    matches!(weather, WeatherType::Storm)
}

/// Per-tick HP decay for `material` at tile temperature `temp_c` under
/// `weather` (Spec 420). Pure, so the environmental response of every
/// material is unit-testable without a world.
#[must_use]
pub fn decay_rate_for(material: EphemeralMaterial, temp_c: f32, weather: WeatherType) -> f32 {
    match material {
        EphemeralMaterial::Ice => {
            if temp_c > 0.0 {
                ICE_MELT_BASE + ICE_MELT_PER_DEGREE * temp_c
            } else {
                0.0
            }
        }
        EphemeralMaterial::Salt => {
            if is_storm(weather) {
                SALT_STORM_RATE
            } else if is_wet(weather) {
                SALT_RAIN_RATE
            } else {
                0.0
            }
        }
        EphemeralMaterial::PackedMud => {
            let wet = if is_storm(weather) {
                MUD_STORM_RATE
            } else if is_wet(weather) {
                MUD_RAIN_RATE
            } else {
                0.0
            };
            wet + MUD_EROSION_RATE
        }
    }
}

/// Colony ledger for ephemeral architecture (Spec 420): how many
/// ephemeral structures the environment has taken back. Live counts come
/// from [`ephemeral_census`]; this ledger is the memory of the losses.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EphemeralLedger {
    /// Ephemeral structures dissolved by decay since colony start.
    pub dissolved: u64,
}

/// Temperature at a tile: the real [`TemperatureGrid`] when present,
/// otherwise the season's base temperature, otherwise a mild 15°C.
fn tile_temperature(world: &World, pos: GridPosition) -> f32 {
    if let Some(grid) = world.get_resource::<TemperatureGrid>() {
        return grid.get_safe(pos.x, pos.y);
    }
    world
        .get_resource::<SeasonState>()
        .map_or(15.0, |s| s.current_season.base_temperature())
}

/// Current weather, defaulting to Clear in bare worlds.
fn current_weather(world: &World) -> WeatherType {
    world
        .get_resource::<WeatherState>()
        .map_or(WeatherType::Clear, |w| w.current_weather)
}

/// Spawn a Wall built from an ephemeral material through the real
/// construction seam ([`spawn_building`](super::building::spawn_building)),
/// tag it with its material, and mark its tile occupied (mirroring
/// `try_place_building`'s post-placement step). Used by the headless
/// debug command; the material tag is the whole integration — decay,
/// repair, fire, and malfunction all see a normal Wall.
pub fn spawn_ephemeral_wall(
    world: &mut World,
    material: EphemeralMaterial,
    x: i32,
    y: i32,
) -> Entity {
    let entity =
        super::building::spawn_building(world, x, y, BuildingType::Wall, MaterialType::Wood);
    world.entity_mut(entity).insert(material);
    if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
        occupied.0.insert((x, y));
    }
    entity
}

/// Spec 420 decay: drain ephemeral structures by their material's
/// environmental rate, and dissolve them at zero HP.
///
/// Exclusive system in the style of `entropy_system` /
/// `fire_damage_structure_system`. Damage is direct `Structure`
/// arithmetic (the building pattern on trunk); destruction follows the
/// documented dissolution choice in the module header — despawn, free
/// the tile, [`BuildingRemovedEvent`], ledger + chronicle, no ruin and
/// no scrap heap.
pub fn ephemeral_decay_system(world: &mut World) {
    let weather = current_weather(world);
    // (entity, pos, building_type, material kind)
    let mut dissolved: Vec<(
        Entity,
        GridPosition,
        Option<BuildingType>,
        EphemeralMaterial,
    )> = Vec::new();
    {
        let mut query = world.query::<(
            Entity,
            &GridPosition,
            &Structure,
            &EphemeralMaterial,
            Option<&Building>,
        )>();
        // Temperatures need a shared world borrow; collect positions
        // first, then mutate. The grid/season fallback is read per tile.
        let rows: Vec<(Entity, GridPosition, EphemeralMaterial)> = query
            .iter(world)
            .map(|(e, pos, _, mat, _)| (e, *pos, *mat))
            .collect();
        let temps: Vec<f32> = rows
            .iter()
            .map(|(_, pos, _)| tile_temperature(world, *pos))
            .collect();
        let rates: Vec<f32> = rows
            .iter()
            .zip(temps.iter())
            .map(|((_, _, mat), temp)| decay_rate_for(*mat, *temp, weather))
            .collect();
        for ((entity, pos, mat), rate) in rows.iter().zip(rates.iter()) {
            if *rate <= 0.0 {
                continue;
            }
            let Some(mut structure) = world.get_mut::<Structure>(*entity) else {
                continue;
            };
            structure.current_hp = (structure.current_hp - rate).max(0.0);
            if structure.current_hp <= 0.0 {
                let b_type = world.get::<Building>(*entity).map(|b| b.building_type);
                dissolved.push((*entity, *pos, b_type, *mat));
            }
        }
    }

    for (entity, pos, b_type, mat) in dissolved {
        if world.get_entity(entity).is_err() {
            continue;
        }
        world.despawn(entity);

        // Dissolution frees the tile (unlike fire/sabotage ruins, which
        // keep it blocked): remove both occupancy records that point at
        // the gone entity.
        if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
            occupied.0.remove(&(pos.x, pos.y));
        }
        if let Some(mut map) = world.get_resource_mut::<crate::layer1::building::BuildingMap>() {
            if map.0.get(&(pos.x, pos.y)) == Some(&entity) {
                map.0.remove(&(pos.x, pos.y));
            }
        }

        if let Some(mut ledger) = world.get_resource_mut::<EphemeralLedger>() {
            ledger.dissolved += 1;
        }

        if let Some(building_type) = b_type {
            if let Some(mut events) = world.get_resource_mut::<Events<BuildingRemovedEvent>>() {
                events.send(BuildingRemovedEvent {
                    entity,
                    position: pos,
                    building_type,
                });
            }
        }

        let label = b_type.map_or("structure", |b| b.label());
        let text = format!(
            "The {} {} at ({}, {}) has {}. Nothing remains to salvage.",
            mat.label(),
            label,
            pos.x,
            pos.y,
            mat.dissolution_verb()
        );
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add(text.clone());
        }
        if let Some(mut events) =
            world.get_resource_mut::<Events<crate::layer1::chronicle::AddChronicleEvent>>()
        {
            events.send(crate::layer1::chronicle::AddChronicleEvent {
                text,
                importance: crate::layer1::chronicle::EventImportance::Standard,
            });
        }
    }
}

/// Census of live ephemeral structures for STATS / the headless panel.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct EphemeralCensus {
    /// All live ephemeral structures.
    pub total: usize,
    /// Live ice structures.
    pub ice: usize,
    /// Live salt structures.
    pub salt: usize,
    /// Live packed-mud structures.
    pub mud: usize,
    /// Structures dissolved since colony start (from the ledger).
    pub dissolved: u64,
    /// Lowest current HP among live ephemeral structures, if any.
    pub weakest_hp: Option<f32>,
}

/// Count live ephemeral structures by material, plus ledger losses.
#[must_use]
pub fn ephemeral_census(world: &mut World) -> EphemeralCensus {
    let mut census = EphemeralCensus {
        dissolved: world
            .get_resource::<EphemeralLedger>()
            .map_or(0, |l| l.dissolved),
        ..EphemeralCensus::default()
    };
    let mut query = world.query::<(&Structure, &EphemeralMaterial)>();
    for (structure, mat) in query.iter(world) {
        census.total += 1;
        match mat {
            EphemeralMaterial::Ice => census.ice += 1,
            EphemeralMaterial::Salt => census.salt += 1,
            EphemeralMaterial::PackedMud => census.mud += 1,
        }
        census.weakest_hp = Some(
            census
                .weakest_hp
                .map_or(structure.current_hp, |w: f32| w.min(structure.current_hp)),
        );
    }
    census
}

/// Player-facing strings introduced by Spec 420 (IP-guard test input).
#[must_use]
pub fn ephemeral_public_strings() -> Vec<String> {
    vec![
        "Ice".to_string(),
        "Salt".to_string(),
        "Packed Mud".to_string(),
        "melted away".to_string(),
        "dissolved in the wet".to_string(),
        "washed away".to_string(),
        "Nothing remains to salvage.".to_string(),
        "ephemeral".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::architecture::ruins::Ruin;
    use crate::layer1::weather::WeatherState;

    fn world_with_env(temp: f32, weather: WeatherType) -> World {
        let mut world = World::new();
        world.insert_resource(TemperatureGrid::new(20, 20, temp));
        world.insert_resource(WeatherState {
            current_weather: weather,
            duration_remaining: 1000,
        });
        world.insert_resource(EphemeralLedger::default());
        world.insert_resource(OccupiedTiles::default());
        world.insert_resource(MessageLog::default());
        world.init_resource::<Events<BuildingRemovedEvent>>();
        world.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
        world
    }

    fn spawn_ephemeral(
        world: &mut World,
        mat: EphemeralMaterial,
        hp: f32,
        x: i32,
        y: i32,
    ) -> Entity {
        world
            .spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                Structure {
                    current_hp: hp,
                    max_hp: 100.0,
                },
                mat,
                GridPosition { x, y },
            ))
            .id()
    }

    /// Spec §3 RED test 1, adapted to the real seams (Structure HP pool,
    /// per-tile TemperatureGrid instead of the mock Temperature resource).
    #[test]
    fn test_ice_wall_decays_in_high_temperature() {
        let mut world = world_with_env(30.0, WeatherType::Clear);
        let wall = spawn_ephemeral(&mut world, EphemeralMaterial::Ice, 100.0, 0, 0);
        ephemeral_decay_system(&mut world);
        let hp = world.get::<Structure>(wall).unwrap().current_hp;
        assert!(
            hp < 100.0,
            "Ice Wall health should decrease in high temperatures, got {hp}"
        );
        let expected = 100.0 - (ICE_MELT_BASE + ICE_MELT_PER_DEGREE * 30.0);
        assert!(
            (hp - expected).abs() < 0.001,
            "expected {expected}, got {hp}"
        );
    }

    /// Spec §3 RED test 2, adapted (WeatherState/WeatherType::Rain
    /// instead of the mock Weather { is_raining }).
    #[test]
    fn test_salt_wall_decays_in_rain() {
        let mut world = world_with_env(15.0, WeatherType::Rain);
        let wall = spawn_ephemeral(&mut world, EphemeralMaterial::Salt, 100.0, 0, 0);
        ephemeral_decay_system(&mut world);
        let hp = world.get::<Structure>(wall).unwrap().current_hp;
        assert!(
            hp < 100.0,
            "Salt Wall health should decrease when it rains, got {hp}"
        );
        assert!((hp - (100.0 - SALT_RAIN_RATE)).abs() < 0.001);
    }

    #[test]
    fn test_ice_is_permanent_at_or_below_freezing() {
        for temp in [-5.0, 0.0] {
            let mut world = world_with_env(temp, WeatherType::Clear);
            let wall = spawn_ephemeral(&mut world, EphemeralMaterial::Ice, 100.0, 0, 0);
            for _ in 0..10 {
                ephemeral_decay_system(&mut world);
            }
            assert_eq!(world.get::<Structure>(wall).unwrap().current_hp, 100.0);
        }
    }

    #[test]
    fn test_salt_is_permanent_when_dry() {
        let mut world = world_with_env(30.0, WeatherType::Clear);
        let wall = spawn_ephemeral(&mut world, EphemeralMaterial::Salt, 100.0, 0, 0);
        for _ in 0..10 {
            ephemeral_decay_system(&mut world);
        }
        assert_eq!(world.get::<Structure>(wall).unwrap().current_hp, 100.0);
    }

    #[test]
    fn test_hotter_melts_faster() {
        assert!(
            decay_rate_for(EphemeralMaterial::Ice, 30.0, WeatherType::Clear)
                > decay_rate_for(EphemeralMaterial::Ice, 5.0, WeatherType::Clear)
        );
        assert_eq!(
            decay_rate_for(EphemeralMaterial::Ice, 0.0, WeatherType::Clear),
            0.0
        );
    }

    #[test]
    fn test_storm_decays_salt_and_mud_faster_than_rain() {
        assert!(
            decay_rate_for(EphemeralMaterial::Salt, 15.0, WeatherType::Storm)
                > decay_rate_for(EphemeralMaterial::Salt, 15.0, WeatherType::Rain)
        );
        assert!(
            decay_rate_for(EphemeralMaterial::PackedMud, 15.0, WeatherType::Storm)
                > decay_rate_for(EphemeralMaterial::PackedMud, 15.0, WeatherType::Rain)
        );
    }

    #[test]
    fn test_mud_washes_slower_than_salt_but_always_erodes() {
        assert!(
            decay_rate_for(EphemeralMaterial::PackedMud, 15.0, WeatherType::Rain)
                < decay_rate_for(EphemeralMaterial::Salt, 15.0, WeatherType::Rain)
        );
        // Dry mud still crumbles; dry salt does not decay at all.
        assert_eq!(
            decay_rate_for(EphemeralMaterial::PackedMud, 15.0, WeatherType::Clear),
            MUD_EROSION_RATE
        );
        let mut world = world_with_env(15.0, WeatherType::Clear);
        let wall = spawn_ephemeral(&mut world, EphemeralMaterial::PackedMud, 100.0, 0, 0);
        ephemeral_decay_system(&mut world);
        let hp = world.get::<Structure>(wall).unwrap().current_hp;
        assert!((hp - (100.0 - MUD_EROSION_RATE)).abs() < 0.001, "got {hp}");
    }

    #[test]
    fn test_non_ephemeral_buildings_are_untouched() {
        let mut world = world_with_env(30.0, WeatherType::Storm);
        let plain = world
            .spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
                GridPosition { x: 1, y: 1 },
            ))
            .id();
        ephemeral_decay_system(&mut world);
        assert_eq!(world.get::<Structure>(plain).unwrap().current_hp, 100.0);
    }

    #[test]
    fn test_season_fallback_temperature_when_no_grid() {
        // No TemperatureGrid: Summer base temperature (30C) melts ice.
        let mut world = World::new();
        world.insert_resource(SeasonState {
            current_season: crate::layer1::seasons::Season::Summer,
        });
        world.insert_resource(WeatherState::default());
        let wall = spawn_ephemeral(&mut world, EphemeralMaterial::Ice, 100.0, 0, 0);
        ephemeral_decay_system(&mut world);
        assert!(world.get::<Structure>(wall).unwrap().current_hp < 100.0);
        // Winter base temperature (-5C) does not.
        let mut world = World::new();
        world.insert_resource(SeasonState {
            current_season: crate::layer1::seasons::Season::Winter,
        });
        world.insert_resource(WeatherState::default());
        let wall = spawn_ephemeral(&mut world, EphemeralMaterial::Ice, 100.0, 0, 0);
        ephemeral_decay_system(&mut world);
        assert_eq!(world.get::<Structure>(wall).unwrap().current_hp, 100.0);
    }

    #[test]
    fn test_dissolution_despawns_frees_tile_and_leaves_no_ruin_or_scrap() {
        let mut world = world_with_env(30.0, WeatherType::Clear);
        world.resource_mut::<OccupiedTiles>().0.insert((3, 3));
        let wall = spawn_ephemeral(&mut world, EphemeralMaterial::Ice, 0.2, 3, 3);
        ephemeral_decay_system(&mut world);

        assert!(
            world.get_entity(wall).is_err(),
            "dissolved wall must despawn"
        );
        assert!(
            !world.resource::<OccupiedTiles>().0.contains(&(3, 3)),
            "dissolution frees the tile for immediate reuse"
        );
        assert_eq!(
            world.query::<&Ruin>().iter(&world).count(),
            0,
            "dissolved material leaves no ruin (documented §5 choice)"
        );
        assert_eq!(
            world
                .query::<&crate::layer1::agriculture::compost::ScrapHeap>()
                .iter(&world)
                .count(),
            0,
            "dissolved material leaves nothing to salvage"
        );
        assert_eq!(world.resource::<EphemeralLedger>().dissolved, 1);
        let events = world.resource::<Events<BuildingRemovedEvent>>();
        let mut reader = events.get_cursor();
        let sent: Vec<_> = reader.read(events).collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].building_type, BuildingType::Wall);
    }

    #[test]
    fn test_census_counts_by_material_and_weakest() {
        let mut world = world_with_env(15.0, WeatherType::Clear);
        spawn_ephemeral(&mut world, EphemeralMaterial::Ice, 80.0, 0, 0);
        spawn_ephemeral(&mut world, EphemeralMaterial::Ice, 40.0, 1, 0);
        spawn_ephemeral(&mut world, EphemeralMaterial::Salt, 90.0, 2, 0);
        spawn_ephemeral(&mut world, EphemeralMaterial::PackedMud, 70.0, 3, 0);
        let census = ephemeral_census(&mut world);
        assert_eq!(census.total, 4);
        assert_eq!(census.ice, 2);
        assert_eq!(census.salt, 1);
        assert_eq!(census.mud, 1);
        assert_eq!(census.weakest_hp, Some(40.0));
        assert_eq!(census.dissolved, 0);
    }

    #[test]
    fn test_spawn_ephemeral_wall_uses_real_building_seam() {
        let mut world = world_with_env(15.0, WeatherType::Clear);
        let entity = spawn_ephemeral_wall(&mut world, EphemeralMaterial::Salt, 5, 5);
        assert_eq!(
            world.get::<Building>(entity).unwrap().building_type,
            BuildingType::Wall
        );
        assert_eq!(
            world.get::<EphemeralMaterial>(entity).copied(),
            Some(EphemeralMaterial::Salt)
        );
        assert!(world.get::<Structure>(entity).is_some());
        assert!(world.resource::<OccupiedTiles>().0.contains(&(5, 5)));
    }

    #[test]
    fn test_material_parse_and_labels() {
        assert_eq!(
            EphemeralMaterial::parse("ice"),
            Some(EphemeralMaterial::Ice)
        );
        assert_eq!(
            EphemeralMaterial::parse("SALT"),
            Some(EphemeralMaterial::Salt)
        );
        assert_eq!(
            EphemeralMaterial::parse("mud"),
            Some(EphemeralMaterial::PackedMud)
        );
        assert_eq!(EphemeralMaterial::parse("stone"), None);
        assert_eq!(EphemeralMaterial::PackedMud.label(), "Packed Mud");
    }

    #[test]
    fn test_ip_guard_no_distinctive_ip_in_public_strings() {
        let banned = [
            "winterfell",
            "hogwarts",
            "frodo",
            "gandalf",
            "skyrim",
            "minecraft",
            "rimworld",
            "dwarf fortress",
        ];
        for s in ephemeral_public_strings() {
            let lower = s.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string {s:?} contains banned term {term}"
                );
            }
        }
    }
}
