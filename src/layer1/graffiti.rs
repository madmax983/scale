//! Graffiti and Signage system (Spec 144).
//!
//! Pops can leave permanent "Markings" on walls and buildings based on their Morale.
//! These markings persist and influence other Pops who see them.

use crate::layer1::building::OccupiedTiles;
use crate::layer1::map::GridPosition;
use crate::layer1::morale::{MoodModifier, Morale};
use bevy::utils::HashMap;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Types of graffiti that can be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraffitiType {
    /// Placed by pops with low morale. Lowers morale of observers.
    Vandalism,
    /// Placed by pops with high morale. Increases morale of observers.
    Inspiration,
    /// Artistic expression (requires Artist trait).
    Mural,
    /// Faction propaganda (requires Rebel trait or Faction Leader).
    Propaganda,
    /// Memetic hazard that infects observers.
    MemeticSigil,
}

/// A graffiti marking on a tile.
#[derive(Debug, Clone)]
pub struct Graffiti {
    /// The type of graffiti.
    pub graffiti_type: GraffitiType,
    /// Remaining duration in ticks.
    pub decay: f32,
    /// Morale impact when observed.
    pub modifier: f32,
}

impl GraffitiType {
    /// Short display name for panels and logs.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            GraffitiType::Vandalism => "Vandalism",
            GraffitiType::Inspiration => "Inspiration",
            GraffitiType::Mural => "Mural",
            GraffitiType::Propaganda => "Propaganda",
            GraffitiType::MemeticSigil => "MemeticSigil",
        }
    }

    /// Parse a type name (case-insensitive) for debug seeding.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "vandalism" | "vandal" => Some(GraffitiType::Vandalism),
            "inspiration" | "inspiring" => Some(GraffitiType::Inspiration),
            "mural" | "art" => Some(GraffitiType::Mural),
            "propaganda" | "poster" => Some(GraffitiType::Propaganda),
            "memetic" | "sigil" | "memeticsigil" => Some(GraffitiType::MemeticSigil),
            _ => None,
        }
    }

    /// Canonical (decay, morale modifier) a fresh marking of this type gets.
    #[must_use]
    pub fn fresh_decay_modifier(&self) -> (f32, f32) {
        match self {
            GraffitiType::Vandalism => (1000.0, -0.05),
            GraffitiType::Inspiration | GraffitiType::Propaganda | GraffitiType::Mural => {
                (1000.0, 0.05)
            }
            GraffitiType::MemeticSigil => (1000.0, 0.0),
        }
    }
}

/// Spec 357: place (or replace) a marking at a tile with canonical tuning.
/// Used by the headless `tag` debug seed; returns the inserted marking.
pub fn tag_graffiti(map: &mut GraffitiMap, x: i32, y: i32, graffiti_type: GraffitiType) -> Graffiti {
    let (decay, modifier) = graffiti_type.fresh_decay_modifier();
    let graffiti = Graffiti {
        graffiti_type,
        decay,
        modifier,
    };
    map.markings.insert((x, y), graffiti.clone());
    graffiti
}

/// Spec 357 CleanGraffiti counterplay (adapted to the GraffitiMap resource
/// model, §8): scrub the marking at a tile. Returns true if one was removed.
pub fn scrub_graffiti(map: &mut GraffitiMap, x: i32, y: i32) -> bool {
    map.markings.remove(&(x, y)).is_some()
}

/// Number of active markings in a live world (headless STATS `graffiti=`).
#[must_use]
pub fn graffiti_count(world: &mut World) -> usize {
    world
        .get_resource::<GraffitiMap>()
        .map_or(0, |m| m.markings.len())
}

/// Resource storing all active graffiti on the map.
/// ⚡ Bolt Optimization: Switched to bevy::utils::HashMap (AHash) to eliminate SipHash overhead for integer coordinate keys.
#[derive(Resource, Default)]
pub struct GraffitiMap {
    /// Map from grid coordinates to graffiti data.
    pub markings: HashMap<(i32, i32), Graffiti>,
}

/// System to place graffiti based on pop morale.
type GraffitiPlacementQuery<'a> = (
    &'a GridPosition,
    &'a Morale,
    Option<&'a crate::layer1::psychology::stress::StressTracker>,
    Option<&'a crate::layer1::traits::Traits>,
);

pub fn graffiti_placement_system(
    mut graffiti_map: ResMut<GraffitiMap>,
    pops: Query<GraffitiPlacementQuery, With<crate::layer1::pop::Pop>>,
    occupied_tiles: Option<Res<OccupiedTiles>>,
) {
    let mut rng = rand::thread_rng();
    // 1% chance per tick to place graffiti if conditions are met
    // This prevents map from being covered instantly
    let placement_chance = 0.01;

    // We use OccupiedTiles for fast lookup if available.
    // If not (e.g. in tests), we might need another way or just skip.
    // Tests might mock OccupiedTiles or we can use the slow query method if needed,
    // but using OccupiedTiles is better for performance.
    // For tests that don't add OccupiedTiles, we might fail to place graffiti if we strictly rely on it.
    // The RED phase tests added `Building` entities but didn't populate `OccupiedTiles`.
    // I should probably update the tests to populate `OccupiedTiles` or use a fallback.
    // Given the codebase uses `OccupiedTiles` for validity, let's rely on it.
    // But I must update tests! (Or I can check if I can query buildings directly as a fallback, which is slow).

    // Actually, let's look at the tests I wrote.
    // `world.spawn((Building...))`
    // This does NOT automatically update `OccupiedTiles` unless `try_place_building` was used or a system runs.
    // My tests manually spawn.
    // So I should stick to the "slow" way (iterating buildings) OR update tests.
    // Updating tests is cleaner but "REFACTOR" step.
    // OR I can just iterate buildings in the system? No, that's O(P*B).

    // Compromise: In the system, I will assume `OccupiedTiles` is the source of truth for "is there a building".
    // I will update the tests to manually insert into `OccupiedTiles`.
    // Wait, I can't update tests in this `write_file` call easily without rewriting the whole file.
    // I'll rewrite the whole file including tests.

    let Some(occupied) = occupied_tiles else {
        return;
    };

    for (pos, morale, stress, traits) in &pops {
        // Optimization: Quick RNG check first
        if rng.gen_range(0.0..1.0) > placement_chance {
            continue;
        }

        let graffiti_type = if stress.map_or(0.0, |s| s.accumulated_stress) > 80.0
            && traits.is_some_and(|t| t.has(crate::layer1::traits::Trait::Creative))
        {
            GraffitiType::Propaganda
        } else if morale.value < 0.2 {
            GraffitiType::Vandalism
        } else if morale.value > 0.8 {
            GraffitiType::Inspiration
        } else {
            continue;
        };

        // Find adjacent building
        let neighbors = [
            (pos.x, pos.y - 1),
            (pos.x + 1, pos.y),
            (pos.x, pos.y + 1),
            (pos.x - 1, pos.y),
        ];

        for target in neighbors {
            if occupied.0.contains(&target) {
                // Found a wall/building!
                let (decay, modifier) = match graffiti_type {
                    GraffitiType::Vandalism => (1000.0, -0.05),
                    GraffitiType::Inspiration => (1000.0, 0.05),
                    GraffitiType::Propaganda => (1000.0, 0.05),
                    _ => (1000.0, 0.0),
                };

                graffiti_map.markings.insert(
                    target,
                    Graffiti {
                        graffiti_type,
                        decay,
                        modifier,
                    },
                );
                break; // Only place one
            }
        }
    }
}

/// System to apply mood modifiers when observing graffiti.
pub fn graffiti_observation_system(
    mut commands: Commands,
    graffiti_map: Res<GraffitiMap>,
    mut pops: Query<(
        Entity,
        &GridPosition,
        &mut Morale,
        Option<&crate::layer1::memetics::MemeticCarrier>,
    )>,
    config: Option<Res<crate::layer1::memetics::MemeticConfig>>,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let infection_chance = config.map_or(0.1, |c| c.infection_chance);

    for (entity, pos, mut morale, carrier) in &mut pops {
        // Check adjacent tiles for graffiti
        let neighbors = [
            (pos.x, pos.y - 1),
            (pos.x + 1, pos.y),
            (pos.x, pos.y + 1),
            (pos.x - 1, pos.y),
        ];

        for target in neighbors {
            if let Some(graffiti) = graffiti_map.markings.get(&target) {
                // Handle Memetic Infection
                if graffiti.graffiti_type == GraffitiType::MemeticSigil
                    && carrier.is_none()
                    && rng.gen_bool(infection_chance)
                {
                    commands
                        .entity(entity)
                        .insert(crate::layer1::memetics::MemeticCarrier);
                }

                let label = match graffiti.graffiti_type {
                    GraffitiType::Vandalism => "Saw Vandalism",
                    GraffitiType::Inspiration => "Saw Inspiration",
                    GraffitiType::Mural => "Saw Mural",
                    GraffitiType::Propaganda => "Saw Propaganda",
                    GraffitiType::MemeticSigil => "Saw Strange Sigil",
                };

                // Check if already affected
                let already_affected = morale.modifiers.iter().any(|m| m.label == label);
                if !already_affected {
                    morale.add_modifier(MoodModifier {
                        label: label.to_string(),
                        value: graffiti.modifier,
                        duration: 50, // Short duration
                    });
                }
            }
        }
    }
}

/// System to decay graffiti over time.
pub fn graffiti_decay_system(world: &mut World) {
    // We need to mutate GraffitiMap, so we extract it.
    // Using `world` directly allows this system to be registered as `fn(world: &mut World)` if needed,
    // but usually systems take `ResMut`.
    // The spec defined it as `fn(&mut World)`. I'll stick to that signature for compatibility with spec,
    // but normally in Bevy it's `ResMut<GraffitiMap>`.
    // However, looking at the code, it's cleaner to use `ResMut`.
    // But I must match the signature used in the Test if I export it.
    // The test calls `super::graffiti_decay_system(&mut world)`.
    // So I must keep the signature.

    world.resource_scope(|_world, mut map: Mut<GraffitiMap>| {
        let mut to_remove = Vec::new();
        for (pos, graffiti) in &mut map.markings {
            graffiti.decay -= 1.0;
            if graffiti.decay <= 0.0 {
                to_remove.push(*pos);
            }
        }
        for pos in to_remove {
            map.markings.remove(&pos);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
    use crate::layer1::map::GridPosition;
    use crate::layer1::morale::Morale;
    use crate::layer1::pop::Pop;

    #[test]
    fn test_graffiti_map_starts_empty() {
        let map = GraffitiMap::default();
        assert!(map.markings.is_empty());
    }

    #[test]
    fn test_low_mood_pop_places_vandalism() {
        let mut world = World::new();
        world.insert_resource(GraffitiMap::default());

        // Setup OccupiedTiles manually for test
        let mut occupied = OccupiedTiles::default();
        occupied.0.insert((5, 5));
        world.insert_resource(occupied);

        // Spawn Wall at (5,5)
        world.spawn((
            Building {
                building_type: BuildingType::Wall,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Spawn Sad Pop at (5,4) (adjacent)
        // Mood 10.0 in spec -> Morale 0.1
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 4 },
            Morale {
                value: 0.1,
                ..Default::default()
            },
        ));

        // Run placement system repeatedly to overcome 1% chance
        // Or mock RNG? We can't easily mock `rand::thread_rng` inside the system without dependency injection.
        // For test purposes, we can try running it many times, or modify the system to take a config.
        // Or we can just trust the logic if we could force probability.
        // Given I implemented `let placement_chance = 0.01;`, this test will flake or fail if run once.
        // I should probably make `placement_chance` configurable or 1.0 in tests.
        // But `graffiti_placement_system` doesn't take config.

        // HACK: Run it 1000 times.
        let mut schedule = Schedule::default();
        schedule.add_systems(graffiti_placement_system);

        let mut placed = false;
        for _ in 0..1000 {
            schedule.run(&mut world);
            if world
                .resource::<GraffitiMap>()
                .markings
                .contains_key(&(5, 5))
            {
                placed = true;
                break;
            }
        }

        assert!(placed, "Graffiti should be placed eventually");

        let map = world.resource::<GraffitiMap>();
        let graffiti = map.markings.get(&(5, 5));
        assert!(graffiti.is_some());
        assert_eq!(graffiti.unwrap().graffiti_type, GraffitiType::Vandalism);
    }

    #[test]
    fn test_high_mood_pop_places_inspiration() {
        let mut world = World::new();
        world.insert_resource(GraffitiMap::default());

        let mut occupied = OccupiedTiles::default();
        occupied.0.insert((5, 5));
        world.insert_resource(occupied);

        world.spawn((
            Building {
                building_type: BuildingType::Wall,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Mood 95.0 in spec -> Morale 0.95
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 4 },
            Morale {
                value: 0.95,
                ..Default::default()
            },
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(graffiti_placement_system);

        let mut placed = false;
        for _ in 0..1000 {
            schedule.run(&mut world);
            if world
                .resource::<GraffitiMap>()
                .markings
                .contains_key(&(5, 5))
            {
                placed = true;
                break;
            }
        }

        assert!(placed, "Inspiration should be placed eventually");

        let map = world.resource::<GraffitiMap>();
        let graffiti = map.markings.get(&(5, 5));
        assert!(graffiti.is_some());
        assert_eq!(graffiti.unwrap().graffiti_type, GraffitiType::Inspiration);
    }

    #[test]
    fn test_graffiti_observation_affects_mood() {
        let mut world = World::new();

        // Pre-place Vandalism
        let mut map = GraffitiMap::default();
        map.markings.insert(
            (5, 5),
            Graffiti {
                graffiti_type: GraffitiType::Vandalism,
                decay: 100.0,
                modifier: -0.05,
            },
        );
        world.insert_resource(map);

        // Spawn Pop at (5,4) observing (5,5)
        let pop_id = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 4 },
                Morale {
                    value: 0.5,
                    ..Default::default()
                },
            ))
            .id();

        // Run observation system
        let mut schedule = Schedule::default();
        schedule.add_systems(graffiti_observation_system);
        schedule.run(&mut world);

        // Check Mood reduced
        let morale = world.get::<Morale>(pop_id).unwrap();
        assert!(
            !morale.modifiers.is_empty(),
            "Should have added a mood modifier"
        );
        let modifier = &morale.modifiers[0];
        // Value should be -0.05
        assert!(
            (modifier.value - -0.05).abs() < f32::EPSILON,
            "Modifier should be -0.05"
        );
        assert_eq!(modifier.label, "Saw Vandalism");
    }

    #[test]
    fn test_graffiti_map_uses_fast_hash() {
        let map = GraffitiMap::default();
        let type_name = std::any::type_name_of_val(&map.markings);
        assert!(
            type_name.contains("hashbrown") || type_name.contains("bevy_utils::hash"),
            "GraffitiMap should use a fast hash implementation (AHash) instead of std::collections::HashMap, got: {}",
            type_name
        );
    }

    #[test]
    fn test_graffiti_decay() {
        let mut world = World::new();
        let mut map = GraffitiMap::default();
        map.markings.insert(
            (0, 0),
            Graffiti {
                graffiti_type: GraffitiType::Vandalism,
                decay: 1.0, // Almost gone
                modifier: -0.05,
            },
        );
        world.insert_resource(map);

        // Run decay system
        super::graffiti_decay_system(&mut world);

        let map = world.resource::<GraffitiMap>();
        assert!(
            map.markings.is_empty(),
            "Graffiti should be removed after decay reaches 0"
        );
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
    use crate::layer1::map::GridPosition;
    use crate::layer1::morale::Morale;
    use crate::layer1::pop::Pop;
    use crate::layer1::psychology::stress::StressTracker;
    use crate::layer1::traits::{Trait, Traits};

    #[test]
    fn test_creative_pop_creates_graffiti_under_high_stress() {
        let mut world = World::new();
        world.insert_resource(GraffitiMap::default());

        let mut occupied = OccupiedTiles::default();
        occupied.0.insert((5, 5));
        world.insert_resource(occupied);

        world.spawn((
            Building {
                building_type: BuildingType::Wall,
            },
            GridPosition { x: 5, y: 5 },
        ));

        let mut traits = Traits::default();
        traits.add(Trait::Creative);

        world.spawn((
            Pop,
            GridPosition { x: 5, y: 4 },
            Morale {
                value: 0.5,
                ..Default::default()
            },
            StressTracker {
                accumulated_stress: 90.0,
            },
            traits,
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(graffiti_placement_system);

        let mut placed = false;
        for _ in 0..1000 {
            schedule.run(&mut world);
            if world
                .resource::<GraffitiMap>()
                .markings
                .contains_key(&(5, 5))
            {
                placed = true;
                break;
            }
        }

        assert!(placed, "Propaganda should be placed eventually");

        let map = world.resource::<GraffitiMap>();
        let graffiti = map.markings.get(&(5, 5));
        assert!(graffiti.is_some());
        assert_eq!(graffiti.unwrap().graffiti_type, GraffitiType::Propaganda);
    }
}

#[cfg(test)]
mod spec357_tests {
    //! Spec 357 acceptance, adapted to the existing Spec-144 GraffitiMap
    //! resource model (§8 pattern): scrub counterplay + IP guard. The
    //! live-schedule proofs live in `live_wiring_tests` below.
    use super::*;

    #[test]
    fn test_scrub_removes_marking() {
        let mut map = GraffitiMap::default();
        tag_graffiti(&mut map, 5, 5, GraffitiType::Vandalism);
        assert_eq!(map.markings.len(), 1);
        assert!(scrub_graffiti(&mut map, 5, 5), "scrub must report removal");
        assert!(map.markings.is_empty());
        assert!(!scrub_graffiti(&mut map, 5, 5), "empty tile scrubs false");
    }

    #[test]
    fn test_tag_seeds_canonical_marking() {
        let mut map = GraffitiMap::default();
        let g = tag_graffiti(&mut map, 1, 2, GraffitiType::Inspiration);
        assert_eq!(g.modifier, 0.05);
        assert_eq!(map.markings.get(&(1, 2)).unwrap().decay, 1000.0);
        assert_eq!(GraffitiType::from_name("sigil"), Some(GraffitiType::MemeticSigil));
        assert_eq!(GraffitiType::from_name("nope"), None);
    }

    #[test]
    fn test_no_banned_ip_strings() {
        // Original names only: no distinctive third-party IP terms in the
        // public strings this spec adds.
        let banned = ["banksy", "kilroy", "obey giant", "shepard fairey"];
        let mut strings: Vec<String> = (0..5)
            .map(|_| String::new())
            .collect();
        let types = [
            GraffitiType::Vandalism,
            GraffitiType::Inspiration,
            GraffitiType::Mural,
            GraffitiType::Propaganda,
            GraffitiType::MemeticSigil,
        ];
        for (i, t) in types.iter().enumerate() {
            strings[i] = t.name().to_ascii_lowercase();
        }
        for s in &strings {
            for b in banned {
                assert!(!s.contains(b), "banned IP string {b} in {s}");
            }
        }
    }
}

#[cfg(test)]
mod live_wiring_tests {
    //! Spec 357 live-wiring audit: these run the REAL production schedule
    //! (`run_simulation_tick`) on a REAL world from `setup_world_with_config`
    //! — real OccupiedTiles, real GraffitiMap insertion, real set ordering
    //! (Economy observation -> Consumption -> Observation placement).
    //! Test scaffolding pins the starter pops' needs/morale/positions each
    //! tick so the 1%-per-tick placement roll gets enough trials; the
    //! systems under audit are unmodified production code.
    use super::*;
    use crate::layer1::building::OccupiedTiles;
    use crate::layer1::map::GridPosition;
    use crate::layer1::morale::Morale;
    use crate::layer1::needs::Needs;
    use crate::layer1::pop::Pop;

    fn pop_entities(world: &mut World) -> Vec<Entity> {
        let mut q = world.query_filtered::<Entity, With<Pop>>();
        q.iter(world).collect()
    }

    #[test]
    fn live_stressed_pops_place_graffiti_in_full_sim() {
        let mut world =
            crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        let occupied: Vec<(i32, i32)> = world
            .resource::<OccupiedTiles>()
            .0
            .iter()
            .copied()
            .collect();
        assert!(!occupied.is_empty(), "setup world must have buildings");
        let pops = pop_entities(&mut world);
        assert!(!pops.is_empty(), "setup world must have pops");

        let mut placed = false;
        for tick in 0..400 {
            // Pin: every pop miserable + parked beside an occupied tile.
            for (i, e) in pops.iter().enumerate() {
                let (ox, oy) = occupied[i % occupied.len()];
                if let Some(mut pos) = world.get_mut::<GridPosition>(*e) {
                    pos.x = ox;
                    pos.y = oy - 1;
                }
                if let Some(mut needs) = world.get_mut::<Needs>(*e) {
                    needs.hunger = 0.1;
                    needs.rest = 0.1;
                    needs.leisure = 0.1;
                    needs.hygiene = 0.1;
                }
                if let Some(mut morale) = world.get_mut::<Morale>(*e) {
                    morale.value = 0.1;
                    // Needs recover during Execution (pops eat/rest), so
                    // pin misery through the game's own mood-modifier
                    // channel — the morale cache sums modifiers, holding
                    // value at ~0 for the Observation-phase placement roll.
                    morale.modifiers.push(
                        crate::layer1::morale::MoodModifier {
                            label: "Audit Misery".to_string(),
                            value: -1.0,
                            duration: 3,
                        },
                    );
                }
            }
            crate::simulation::run_simulation_tick(&mut world);
            // Other live systems can also write markings (memetic arrival
            // sigils, protest/binge lanes); the audit target is the
            // morale-driven Vandalism the pinned pops must produce.
            if world
                .resource::<GraffitiMap>()
                .markings
                .values()
                .any(|g| g.graffiti_type == GraffitiType::Vandalism)
            {
                placed = true;
                println!("graffiti placed live at tick {tick}");
                break;
            }
        }
        assert!(
            placed,
            "a stressed pop adjacent to a building must place graffiti in the live schedule"
        );
    }

    #[test]
    fn live_observation_applies_mood_modifier_in_full_sim() {
        let mut world =
            crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        let pops = pop_entities(&mut world);
        let anchor = world.get::<GridPosition>(pops[0]).copied().unwrap();
        // Blanket the anchor's neighbourhood so wherever the pop steps
        // during Execution it stays cardinally adjacent to a marking.
        {
            let mut map = world.resource_mut::<GraffitiMap>();
            for dx in -2..=2 {
                for dy in -2..=2 {
                    tag_graffiti(
                        &mut map,
                        anchor.x + dx,
                        anchor.y + dy,
                        GraffitiType::Vandalism,
                    );
                }
            }
        }
        let mut observed = false;
        for _ in 0..10 {
            crate::simulation::run_simulation_tick(&mut world);
            let mut q = world.query::<&Morale>();
            observed = q
                .iter(&world)
                .any(|m| m.modifiers.iter().any(|md| md.label == "Saw Vandalism"));
            if observed {
                break;
            }
        }
        assert!(
            observed,
            "a pop near vandalism must gain the 'Saw Vandalism' mood modifier in the live schedule"
        );
    }

    #[test]
    fn live_scrub_empties_map_in_full_sim() {
        let mut world =
            crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        {
            let mut map = world.resource_mut::<GraffitiMap>();
            tag_graffiti(&mut map, 3, 3, GraffitiType::Propaganda);
        }
        assert_eq!(graffiti_count(&mut world), 1);
        let removed = {
            let mut map = world.resource_mut::<GraffitiMap>();
            scrub_graffiti(&mut map, 3, 3)
        };
        assert!(removed);
        assert_eq!(graffiti_count(&mut world), 0);
    }
}

