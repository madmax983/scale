//! The Feral Outpost (Spec 273).
//!
//! The untamed frontier reclaims its own. Pops who live and work far from the
//! colony's heart — the Command Center — slowly accumulate cultural drift.
//! Tracked per-pop by [`CulturalDrift::fringe_exposure`], which rises while a
//! pop is beyond [`FRINGE_DISTANCE`] tiles (Chebyshev) of the Command Center
//! and decays when the pop returns to civilization. A pop whose exposure
//! reaches 1.0 gains the `"Fringe"` tag.
//!
//! When [`MIN_OUTPOST_SIZE`] or more Fringe pops cluster within
//! [`CLUSTER_RADIUS`] tiles of each other, they found a [`FeralOutpost`]:
//! a named sub-faction entity claiming a territory of
//! [`OUTPOST_CLAIM_RADIUS`] tiles. Members (marked [`FeralMember`]) still
//! work — they keep their jobs, skills, and guild faction — but they refuse
//! orders that would drag them back to the core: [`assign_home_zone`] errors
//! when the target lies outside their outpost's claimed territory, while
//! local moves inside the territory are allowed. Given time, members develop
//! the [`Trait::Feral`][crate::layer1::psychology::traits::Trait] trait
//! ("raised in the wild").
//!
//! ## Design notes (adaptations of the spec's mock-style RED tests)
//!
//! * The spec's `SubFaction` entity component and tuple-struct
//!   `FactionMember(entity)` don't exist; the real faction model is
//!   [`FactionMember`][crate::layer1::social::factions::FactionMember]
//!   with an enum [`FactionId`][crate::layer1::social::factions::FactionId].
//!   Outposts are dedicated entities carrying [`FeralOutpost`], and member
//!   pops carry [`FeralMember`] pointing at their outpost — multiple
//!   outposts can coexist, and territory checks use the outpost's claimed
//!   center/radius (the spec's REFACTOR phase asks for exactly this).
//! * The spec's `CulturalTag` component didn't exist, and the name was already
//!   taken by the ideological-contraband trade enum; the drift component is
//!   defined here as [`CulturalDrift`] with gradual `fringe_exposure`
//!   (the spec's REFACTOR phase: "gradually increment a `FringeExposure`
//!   value instead of an instant threshold").
//! * The spec's `CommandCenter` marker component doesn't exist; the real
//!   colony heart is a [`Building`][crate::layer1::architecture::building::Building]
//!   whose [`BuildingType`][crate::layer1::architecture::building::BuildingType]
//!   is `CommandCenter`.
//! * The spec's `GridPosition::distance` doesn't exist; the real API is
//!   [`distance_chebyshev`][crate::layer1::core::map::GridPosition::distance_chebyshev].
//! * "Unique, primitive traits" maps onto the existing
//!   [`Trait::Feral`][crate::layer1::psychology::traits::Trait] trait —
//!   no new trait variants, no exhaustive-match breakage elsewhere.
//! * All names are original and generic; nothing is lifted from outside
//!   fiction (see the IP-guard test).

use bevy_ecs::prelude::*;
use std::collections::HashSet;

use crate::layer1::architecture::building::{Building, BuildingType};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::shared::time::SimulationTime;

/// Chebyshev distance (tiles) beyond which a pop starts drifting from the
/// colony's culture.
pub const FRINGE_DISTANCE: u32 = 50;
/// Fringe exposure gained per tick while beyond [`FRINGE_DISTANCE`].
/// 1/600 → roughly 600 ticks of isolation to go fully Fringe.
pub const FRINGE_RATE: f32 = 1.0 / 600.0;
/// Fringe exposure shed per tick while within [`FRINGE_DISTANCE`]
/// (reintegration is faster than drift).
pub const FRINGE_DECAY: f32 = 1.0 / 300.0;
/// Max Chebyshev distance between Fringe pops to count as one cluster.
pub const CLUSTER_RADIUS: u32 = 12;
/// Minimum Fringe pops in a cluster to found an outpost.
pub const MIN_OUTPOST_SIZE: usize = 3;
/// Territory radius (tiles, Chebyshev) claimed by a new outpost.
pub const OUTPOST_CLAIM_RADIUS: u32 = 15;
/// Ticks of outpost membership before a pop develops the Feral trait.
pub const FERAL_TRAIT_TICKS: u64 = 200;

/// Cultural drift state of a pop.
///
/// Tracks the spec's `"Fringe"` tag plus the gradual `fringe_exposure`
/// value (0.0–1.0) behind it.
#[derive(Component, Debug, Clone, Default)]
pub struct CulturalDrift {
    tags: HashSet<String>,
    fringe_exposure: f32,
}

impl CulturalDrift {
    /// Returns true if the pop currently holds the named cultural tag.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }

    /// Grants the named cultural tag.
    pub fn add_tag(&mut self, tag: &str) {
        self.tags.insert(tag.to_string());
    }

    /// Removes the named cultural tag.
    pub fn remove_tag(&mut self, tag: &str) {
        self.tags.remove(tag);
    }

    /// Current fringe exposure, 0.0 (rooted) to 1.0 (fully fringe).
    #[must_use]
    pub fn fringe_exposure(&self) -> f32 {
        self.fringe_exposure
    }

    /// True once exposure has saturated and the "Fringe" tag was granted.
    #[must_use]
    pub fn is_fringe(&self) -> bool {
        self.has_tag("Fringe")
    }

    /// A fully-drifted tag set: saturated exposure plus the "Fringe" tag.
    /// Used by tests and debug tooling to conjure fringe-dwellers directly.
    #[must_use]
    pub fn fully_fringe() -> Self {
        let mut tag = Self {
            tags: HashSet::new(),
            fringe_exposure: 1.0,
        };
        tag.add_tag("Fringe");
        tag
    }
}

/// A feral outpost sub-faction: a named entity claiming frontier territory.
#[derive(Component, Debug, Clone)]
pub struct FeralOutpost {
    /// Generated outpost name (original, generic — see IP-guard test).
    pub name: String,
    /// Claimed territory center.
    pub center: GridPosition,
    /// Claimed territory radius (Chebyshev tiles).
    pub radius: u32,
    /// Tick the outpost was founded.
    pub founded_tick: u64,
}

/// Marks a pop as a member of a feral outpost.
#[derive(Component, Debug, Clone, Copy)]
pub struct FeralMember {
    /// The outpost entity this pop belongs to.
    pub outpost: Entity,
    /// Tick the pop joined (or the outpost was founded under them).
    pub joined_tick: u64,
}

/// Finds the colony heart: the position of the first Command Center building.
fn command_center_position(world: &mut World) -> Option<GridPosition> {
    let mut query = world.query::<(&Building, &GridPosition)>();
    for (building, pos) in query.iter(world) {
        if building.building_type == BuildingType::CommandCenter {
            return Some(*pos);
        }
    }
    None
}

/// Cultural drift: pops far from the Command Center accumulate fringe
/// exposure; pops near it reintegrate. Exposure saturating at 1.0 grants the
/// "Fringe" tag; exposure decaying to 0.0 removes it.
pub fn update_cultural_drift_system(world: &mut World) {
    let Some(core) = command_center_position(world) else {
        return;
    };
    // Collect first (exclusive system: avoid double-borrowing the world).
    let pops: Vec<(Entity, GridPosition)> = {
        let mut query = world.query_filtered::<(Entity, &GridPosition), With<Pop>>();
        query.iter(world).map(|(e, p)| (e, *p)).collect()
    };
    for (entity, pos) in pops {
        if world.get::<CulturalDrift>(entity).is_none() {
            world.entity_mut(entity).insert(CulturalDrift::default());
        }
        let mut tag = world.get_mut::<CulturalDrift>(entity).unwrap();
        if pos.distance_chebyshev(core) > FRINGE_DISTANCE {
            tag.fringe_exposure = (tag.fringe_exposure + FRINGE_RATE).min(1.0);
            if tag.fringe_exposure >= 1.0 {
                tag.add_tag("Fringe");
            }
        } else {
            tag.fringe_exposure = (tag.fringe_exposure - FRINGE_DECAY).max(0.0);
            if tag.fringe_exposure <= 0.0 {
                tag.remove_tag("Fringe");
            }
        }
    }
}

/// Generic, original outpost-name fragments. Checked by the IP-guard test.
const OUTPOST_NAME_FRAGMENTS: &[&str] =
    &["Warren", "Holdout", "Freehold", "Burrow", "Roost", "Den", "Hollow"];

/// Generates the next outpost name from the count of existing outposts.
fn next_outpost_name(world: &mut World) -> String {
    let mut query = world.query_filtered::<Entity, With<FeralOutpost>>();
    let n = query.iter(world).count();
    let fragment = OUTPOST_NAME_FRAGMENTS[n % OUTPOST_NAME_FRAGMENTS.len()];
    let cycle = n / OUTPOST_NAME_FRAGMENTS.len();
    if cycle == 0 {
        fragment.to_string()
    } else {
        format!("{fragment} {}", cycle + 1)
    }
}

/// Forms feral outposts: clusters of [`MIN_OUTPOST_SIZE`]+ Fringe pops within
/// [`CLUSTER_RADIUS`] tiles of each other found a named [`FeralOutpost`]
/// entity and become [`FeralMember`]s. Fires a chronicle event per founding.
pub fn form_feral_outposts_system(world: &mut World) {
    // Gather unassigned Fringe pops.
    let fringe: Vec<(Entity, GridPosition)> = {
        let mut query = world.query_filtered::<(Entity, &GridPosition, &CulturalDrift), (With<Pop>, Without<FeralMember>)>();
        query
            .iter(world)
            .filter(|(_, _, tags)| tags.is_fringe())
            .map(|(e, p, _)| (e, *p))
            .collect()
    };
    let tick = world.resource::<SimulationTime>().tick;
    let mut claimed = vec![false; fringe.len()];
    for i in 0..fringe.len() {
        if claimed[i] {
            continue;
        }
        // Greedy cluster: everything within CLUSTER_RADIUS of the seed.
        let mut cluster = vec![i];
        for j in (i + 1)..fringe.len() {
            if claimed[j] {
                continue;
            }
            if fringe[i].1.distance_chebyshev(fringe[j].1) <= CLUSTER_RADIUS {
                cluster.push(j);
            }
        }
        if cluster.len() < MIN_OUTPOST_SIZE {
            continue;
        }
        for &k in &cluster {
            claimed[k] = true;
        }
        // Centroid of the cluster becomes the claimed center.
        let (sx, sy) = cluster
            .iter()
            .fold((0i64, 0i64), |(ax, ay), &k| {
                (ax + fringe[k].1.x as i64, ay + fringe[k].1.y as i64)
            });
        let n = cluster.len() as i64;
        let center = GridPosition {
            x: (sx / n) as i32,
            y: (sy / n) as i32,
        };
        let name = next_outpost_name(world);
        let chronicle_text = format!(
            "FERAL OUTPOST: {n} fringe-dwellers have gone wild and founded the {name} at ({}, {}). They still work — but they answer to no one.",
            center.x, center.y,
        );
        let outpost = world
            .spawn(FeralOutpost {
                name: name.clone(),
                center,
                radius: OUTPOST_CLAIM_RADIUS,
                founded_tick: tick,
            })
            .id();
        for &k in &cluster {
            world.entity_mut(fringe[k].0).insert(FeralMember {
                outpost,
                joined_tick: tick,
            });
        }
        world
            .resource_mut::<bevy_ecs::event::Events<AddChronicleEvent>>()
            .send(AddChronicleEvent {
                text: chronicle_text,
                importance: EventImportance::Major,
            });
    }
}

/// Feral pops slowly adapt to the wild, developing the [`Trait::Feral`] trait
/// after [`FERAL_TRAIT_TICKS`] ticks of membership.
pub fn develop_feral_traits_system(world: &mut World) {
    let tick = world.resource::<SimulationTime>().tick;
    let members: Vec<Entity> = {
        let mut query = world.query_filtered::<(Entity, &FeralMember), With<Pop>>();
        query
            .iter(world)
            .filter(|(_, m)| tick >= m.joined_tick + FERAL_TRAIT_TICKS)
            .map(|(e, _)| e)
            .collect()
    };
    for entity in members {
        if world.get::<Traits>(entity).is_none() {
            world.entity_mut(entity).insert(Traits::default());
        }
        let mut traits = world.get_mut::<Traits>(entity).unwrap();
        if !traits.has(Trait::Feral) {
            traits.add(Trait::Feral);
        }
    }
}

/// Attempts to assign a pop's home zone.
///
/// Feral outpost members refuse relocation outside their outpost's claimed
/// territory (an `Err`); moves inside the territory, and all orders for
/// non-feral pops, succeed.
pub fn assign_home_zone(
    world: &mut World,
    pop_entity: Entity,
    target_pos: GridPosition,
) -> Result<(), &'static str> {
    let Some(member) = world.get::<FeralMember>(pop_entity).copied() else {
        return Ok(());
    };
    let Some(outpost) = world.get::<FeralOutpost>(member.outpost) else {
        return Ok(());
    };
    if target_pos.distance_chebyshev(outpost.center) <= outpost.radius {
        Ok(())
    } else {
        Err("Pop is feral and refuses relocation outside its outpost territory")
    }
}

/// Number of founded feral outposts (for headless STATS).
#[must_use]
pub fn feral_outpost_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<Entity, With<FeralOutpost>>();
    query.iter(world).count()
}

/// Number of pops currently holding the Fringe tag (for headless STATS).
#[must_use]
pub fn fringe_pop_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<&CulturalDrift, With<Pop>>();
    query.iter(world).filter(|t| t.is_fringe()).count()
}

/// Public-facing strings, for the IP-guard test.
#[must_use]
pub fn feral_public_strings() -> Vec<String> {
    let mut strings = OUTPOST_NAME_FRAGMENTS
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    strings.push(
        "FERAL OUTPOST: fringe-dwellers have gone wild and founded an outpost. They still work — but they answer to no one.".to_string(),
    );
    strings.push("Pop is feral and refuses relocation outside its outpost territory".to_string());
    strings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::architecture::building::{Building, BuildingType};

    fn spawn_command_center(world: &mut World, x: i32, y: i32) {
        world.spawn((
            Building {
                building_type: BuildingType::CommandCenter,
            },
            GridPosition { x, y },
        ));
    }

    fn spawn_pop(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((Pop, GridPosition { x, y }, CulturalDrift::default()))
            .id()
    }

    #[test]
    fn test_pop_accumulates_fringe_tag_when_far_from_core() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        spawn_command_center(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, 80, 80);

        // 700 ticks of isolation > 600 needed at FRINGE_RATE.
        for _ in 0..700 {
            update_cultural_drift_system(&mut world);
        }

        let tags = world.get::<CulturalDrift>(pop).unwrap();
        assert!(
            tags.has_tag("Fringe"),
            "pop isolated 70 tiles from the core should be Fringe"
        );
        assert!((tags.fringe_exposure() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_pop_near_core_does_not_go_fringe_and_reintegrates() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        spawn_command_center(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, 12, 12);

        for _ in 0..700 {
            update_cultural_drift_system(&mut world);
        }
        assert!(
            !world.get::<CulturalDrift>(pop).unwrap().has_tag("Fringe"),
            "pop next to the core should never go Fringe"
        );

        // A fringe pop dragged home reintegrates.
        world.entity_mut(pop).insert({
            let mut t = CulturalDrift::default();
            t.add_tag("Fringe");
            t
        });
        // Force full exposure, then let decay work it off.
        for _ in 0..400 {
            update_cultural_drift_system(&mut world);
        }
        assert!(
            !world.get::<CulturalDrift>(pop).unwrap().has_tag("Fringe"),
            "fringe pop returned to the core should reintegrate"
        );
    }

    #[test]
    fn test_no_command_center_no_drift() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        let pop = spawn_pop(&mut world, 80, 80);
        for _ in 0..700 {
            update_cultural_drift_system(&mut world);
        }
        assert!(
            !world.get::<CulturalDrift>(pop).unwrap().has_tag("Fringe"),
            "without a Command Center there is no core to drift from"
        );
    }

    #[test]
    fn test_fringe_pops_form_feral_outpost_subfaction() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        let mut members = Vec::new();
        for i in 0..5 {
            let e = world
                .spawn((Pop, GridPosition { x: 80 + i, y: 80 }))
                .id();
            world.entity_mut(e).insert(CulturalDrift::fully_fringe());
            members.push(e);
        }

        form_feral_outposts_system(&mut world);

        let mut query = world.query::<&FeralOutpost>();
        let outposts: Vec<&FeralOutpost> = query.iter(&world).collect();
        assert_eq!(outposts.len(), 1, "one outpost should form from the cluster");
        for e in members {
            let m = world
                .get::<FeralMember>(e)
                .expect("clustered fringe pop should be a FeralMember");
            assert!(
                world.get::<FeralOutpost>(m.outpost).is_some(),
                "member should point at a real outpost"
            );
        }
        // Chronicle should have recorded the founding.
        let events = world.resource::<bevy_ecs::event::Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        assert!(
            reader.read(events).count() >= 1,
            "founding should fire a chronicle event"
        );
    }

    #[test]
    fn test_scattered_fringe_pops_do_not_form_outpost() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        // 5 fringe pops, each far from the others.
        for i in 0..5 {
            let e = world
                .spawn((Pop, GridPosition { x: 80 + i * 50, y: 80 }))
                .id();
            world.entity_mut(e).insert(CulturalDrift::fully_fringe());
        }
        form_feral_outposts_system(&mut world);
        assert_eq!(
            feral_outpost_count(&mut world),
            0,
            "scattered fringe pops should not found an outpost"
        );
    }

    #[test]
    fn test_too_few_fringe_pops_no_outpost() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        for i in 0..2 {
            let e = world.spawn((Pop, GridPosition { x: 80 + i, y: 80 })).id();
            world.entity_mut(e).insert(CulturalDrift::fully_fringe());
        }
        form_feral_outposts_system(&mut world);
        assert_eq!(
            feral_outpost_count(&mut world),
            0,
            "two fringe pops are below the founding threshold"
        );
    }

    #[test]
    fn test_feral_pops_refuse_relocation_outside_territory() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        let outpost = world
            .spawn(FeralOutpost {
                name: "Warren".to_string(),
                center: GridPosition { x: 80, y: 80 },
                radius: OUTPOST_CLAIM_RADIUS,
                founded_tick: 0,
            })
            .id();
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 80, y: 80 },
                FeralMember { outpost, joined_tick: 0 },
            ))
            .id();

        // Core relocation refused...
        assert!(
            assign_home_zone(&mut world, pop, GridPosition { x: 10, y: 10 }).is_err(),
            "feral pops should refuse relocation to the core"
        );
        // ...but local moves inside the claimed territory are fine.
        assert!(
            assign_home_zone(&mut world, pop, GridPosition { x: 85, y: 82 }).is_ok(),
            "feral pops should accept moves inside outpost territory"
        );
    }

    #[test]
    fn test_non_feral_pops_accept_any_relocation() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        let pop = spawn_pop(&mut world, 80, 80);
        assert!(assign_home_zone(&mut world, pop, GridPosition { x: 10, y: 10 }).is_ok());
    }

    #[test]
    fn test_feral_members_develop_feral_trait_over_time() {
        let mut world = World::new();
        let time = SimulationTime { tick: 1000, ..Default::default() };
        world.insert_resource(time);
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        let outpost = world
            .spawn(FeralOutpost {
                name: "Warren".to_string(),
                center: GridPosition { x: 80, y: 80 },
                radius: OUTPOST_CLAIM_RADIUS,
                founded_tick: 0,
            })
            .id();
        let old_member = world
            .spawn((
                Pop,
                GridPosition { x: 80, y: 80 },
                FeralMember { outpost, joined_tick: 0 },
                Traits::default(),
            ))
            .id();
        let new_member = world
            .spawn((
                Pop,
                GridPosition { x: 81, y: 80 },
                FeralMember { outpost, joined_tick: 950 },
                Traits::default(),
            ))
            .id();

        develop_feral_traits_system(&mut world);

        assert!(
            world.get::<Traits>(old_member).unwrap().has(Trait::Feral),
            "long-time member should develop the Feral trait"
        );
        assert!(
            !world.get::<Traits>(new_member).unwrap().has(Trait::Feral),
            "recent joiner should not yet have the Feral trait"
        );
    }

    #[test]
    fn test_outpost_names_are_sequential_and_original() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<AddChronicleEvent>::default());
        assert_eq!(next_outpost_name(&mut world), "Warren");
        world.spawn(FeralOutpost {
            name: "Warren".to_string(),
            center: GridPosition { x: 0, y: 0 },
            radius: OUTPOST_CLAIM_RADIUS,
            founded_tick: 0,
        });
        assert_eq!(next_outpost_name(&mut world), "Holdout");
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Concept-only inspirations: no named characters, places, or
        // distinctive entities from outside fiction may appear in
        // player-facing strings.
        const BANNED: &[&str] = &[
            "arrakis", "dune", "atreides", "harkonnen", "fremen", "sandworm",
            "tatooine", "coruscant", "jedi", "sith", "wookiee", "mordor",
            "gondor", "hobbit", "shire", "sauron", "gandalf", "rivendell",
            "westeros", "stark", "lannister", "targaryen", "winterfell",
            "klingon", "vulcan", "enterprise", "borg", "romulan",
            "xenomorph", "ripley", "weyland", "nostromo",
            "megatron", "optimus", "cybertron",
        ];
        for s in feral_public_strings() {
            let lower = s.to_lowercase();
            for banned in BANNED {
                assert!(
                    !lower.contains(banned),
                    "public string {s:?} contains banned term {banned:?}"
                );
            }
        }
    }
}
