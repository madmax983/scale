//! Alien Domestication (Spec 353).
//!
//! The progressive-taming layer on top of Spec 075 husbandry and Spec 164
//! modular fauna. Where husbandry's [`crate::layer1::agriculture::husbandry::attempt_tame`]
//! is a single instant roll against a designated animal, domestication is a
//! *process*: wild fauna carry [`Tameable`] with a skill gate and a progress
//! meter, tamers file [`TameAction`]s that accumulate progress, and at 100
//! the animal converts to [`Tamed`]. Tamed animals have real upkeep (they
//! draw down their own [`Needs`] and graze from the colony stockpile),
//! revert to the wild when starving, yield resources on a timer via
//! [`ResourceProducer`], bond with their keeper, and grieve them when lost.
//!
//! ## §8 architectural adaptations (spec mocks vs. the real code)
//!
//! - The spec's `Needs { hunger: 100.0 }` uses a 0–100 sketch scale. The
//!   real [`Needs`] is 0.0–1.0 (default 0.8), so the feral threshold is
//!   0.2 and the metabolism decay is 0.01/tick.
//! - The spec's `ResourceType::Wool` does not exist in the real ledger
//!   (`src/layer1/economy/resources.rs` has no fiber variant). Following
//!   husbandry's precedent, milk-bearing fauna produce
//!   [`ResourceType::Food`]; wool-bearing fauna produce nothing until a
//!   fiber resource exists.
//! - The spec's `FaunaBundle` does not exist; fauna are spawned as loose
//!   component tuples (`Fauna` + `GridPosition` + `Health`), and this
//!   module follows that shape.
//! - Systems are written in the exclusive `fn(&mut World)` house style of
//!   `husbandry.rs` / `feral_outpost.rs` rather than the spec's
//!   Commands/Query sketches, so unit tests call them directly.
//! - `Tameable::required_skill` is compared against the action's
//!   `taming_power` (a float derived from the tamer's Husbandry skill),
//!   not against the raw skill level: real Husbandry levels are small
//!   (`floor(sqrt(xp/100))`), so a level gate of 10 could never open.
//! - Spec 353 does NOT replace Spec 075: the instant designation-based
//!   `Tame` path stays as-is. Domestication is the slow, bond-forming
//!   path; `fauna_behavior_system` now skips both kinds of tamed animal.

use super::{BodyPartType, Fauna, FaunaBody, FaunaState, FaunaType};
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::health::Dead;
use crate::layer1::map::GridPosition;
use crate::layer1::memory::{Memories, MemoryType};
use crate::layer1::morale::{MoodModifier, Morale};
use crate::layer1::needs::Needs;
use crate::layer1::pop::Pop;
use crate::layer1::resources::{ColonyResources, ResourceItem, ResourceType};
use crate::layer1::skills::{SkillType, Skills};
use bevy_ecs::prelude::*;

/// Progress at which a [`Tameable`] animal converts to [`Tamed`].
pub const TAMING_PROGRESS_MAX: f32 = 100.0;
/// Default skill gate for a freshly spawned wild animal.
pub const DEFAULT_REQUIRED_SKILL: u32 = 5;
/// Per-tick hunger decay for a tamed animal (real Needs scale, 0.0–1.0).
pub const ANIMAL_HUNGER_DECAY: f32 = 0.01;
/// Default hunger level below which a tamed animal reverts to the wild.
pub const FERAL_HUNGER_THRESHOLD: f32 = 0.2;
/// Hunger level below which a tamed animal grazes from the stockpile.
pub const FEED_HUNGER_THRESHOLD: f32 = 0.6;
/// Colony food consumed per graze.
pub const FEED_FOOD_COST: f32 = 0.05;
/// Hunger restored per graze.
pub const FEED_HUNGER_GAIN: f32 = 0.15;
/// Default production period (ticks) for a milk-bearing animal.
pub const MILK_PRODUCTION_PERIOD: f32 = 100.0;
/// Default production period (ticks) for a brush-harvested animal.
pub const BRUSH_PRODUCTION_PERIOD: f32 = 150.0;
/// Chebyshev range within which a keeper bonds with their animal.
pub const BOND_RANGE: u32 = 6;
/// Mood label for the keeper's bond buff.
pub const BOND_MODIFIER_LABEL: &str = "Bonded Companion";
/// Mood value of the bond buff.
pub const BOND_MODIFIER_VALUE: f32 = 0.05;
/// Duration the bond buff lingers after the keeper steps away (ticks).
pub const BOND_MODIFIER_DURATION: u32 = 3;
/// Mood label for the keeper's grief when a bonded animal dies.
pub const GRIEF_MODIFIER_LABEL: &str = "Lost Companion";
/// Mood value of the grief penalty.
pub const GRIEF_MODIFIER_VALUE: f32 = -0.30;
/// Duration of the grief penalty (ticks).
pub const GRIEF_MODIFIER_DURATION: u32 = 2000;
/// Chebyshev range within which a reverted predator turns on nearby pops.
pub const HOSTILE_REVERSION_RANGE: u32 = 8;

/// A wild animal that can be tamed (Spec 353).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Tameable {
    /// Minimum `taming_power` an action must carry to make progress.
    pub required_skill: u32,
    /// Accumulated taming progress; converts at [`TAMING_PROGRESS_MAX`].
    pub progress: f32,
}

impl Default for Tameable {
    fn default() -> Self {
        Self {
            required_skill: DEFAULT_REQUIRED_SKILL,
            progress: 0.0,
        }
    }
}

impl Tameable {
    /// Creates a tameable animal with the given skill gate.
    #[must_use]
    pub const fn new(required_skill: u32) -> Self {
        Self {
            required_skill,
            progress: 0.0,
        }
    }
}

/// A successfully domesticated animal (Spec 353).
///
/// Distinct from husbandry's instant-roll `Tame` component and from the
/// Layer-2 leviathan `Tamed` marker; this is the progressive path's bond.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tamed {
    /// The keeper who tamed the animal, if any.
    pub owner: Option<Entity>,
}

/// The wild streak that never quite leaves a tamed animal (Spec 353).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct FeralInstinct {
    /// Hunger below this level triggers feral reversion (0.0–1.0 scale).
    pub threshold: f32,
    /// Skill gate restored on the [`Tameable`] inserted by reversion.
    pub required_skill: u32,
}

impl Default for FeralInstinct {
    fn default() -> Self {
        Self {
            threshold: FERAL_HUNGER_THRESHOLD,
            required_skill: DEFAULT_REQUIRED_SKILL,
        }
    }
}

/// Periodic resource yield for a tamed animal (Spec 353).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct ResourceProducer {
    /// What the animal yields when the timer completes.
    pub resource_type: ResourceType,
    /// Ticks of production accumulated so far.
    pub progress: f32,
    /// Ticks required per yield.
    pub max: f32,
}

/// A single taming attempt, filed as a free-standing action entity and
/// consumed by [`execute_tame_action_system`] (Spec 353).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct TameAction {
    /// The animal being tamed.
    pub target: Entity,
    /// The pop doing the taming (becomes the animal's keeper).
    pub actor: Entity,
    /// Effective taming power, derived from the actor's Husbandry skill.
    pub taming_power: f32,
}

/// Marker: this fauna eats meat and remembers how (Spec 353 §5).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Carnivorous;

/// Marker: this fauna defends its ground (Spec 353 §5).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Territorial;

/// Marker: grief for this animal's death has already been applied, so the
/// `Added<Dead>`-style scan in [`bonded_grief_system`] fires exactly once.
#[derive(Component, Debug, Clone, Copy, Default)]
struct BondGriefProcessed;

/// Derives a taming action's power from a tamer's Husbandry skill.
///
/// Base 10.0 for an unskilled (or skill-less) tamer, +5.0 per Husbandry
/// level — enough that a novice can work a `required_skill` 5–10 animal
/// slowly, while a practised keeper (level 3+, 25.0 power) converts one
/// in a handful of sessions.
#[must_use]
pub fn taming_power_from_skills(skills: Option<&Skills>) -> f32 {
    let level = skills.map_or(0, |s| s.get_level(SkillType::Husbandry));
    #[allow(clippy::cast_precision_loss)]
    let level_f32 = level as f32;
    level_f32.mul_add(5.0, 10.0)
}

/// The producer a newly tamed animal starts with, if any.
///
/// Milk-bearing modular bodies (Spec 164 `can_produce("Milk")`) yield
/// [`ResourceType::Food`], as do brush-harvested SpaceRats (husbandry
/// precedent). Wolves and mascots yield nothing — their value is the
/// bond (and, for wolves, the teeth, while they stay fed).
#[must_use]
pub fn default_producer_for(
    fauna_type: Option<FaunaType>,
    body: Option<&FaunaBody>,
) -> Option<ResourceProducer> {
    if let Some(body) = body {
        if body.can_produce("Milk") {
            return Some(ResourceProducer {
                resource_type: ResourceType::Food,
                progress: 0.0,
                max: MILK_PRODUCTION_PERIOD,
            });
        }
    }
    match fauna_type {
        Some(FaunaType::SpaceRat) => Some(ResourceProducer {
            resource_type: ResourceType::Food,
            progress: 0.0,
            max: BRUSH_PRODUCTION_PERIOD,
        }),
        _ => None,
    }
}

fn send_chronicle(world: &mut World, text: String, importance: EventImportance) {
    if world.get_resource::<Events<AddChronicleEvent>>().is_some() {
        world
            .resource_mut::<Events<AddChronicleEvent>>()
            .send(AddChronicleEvent { text, importance });
    }
}

/// Whether this animal is a meat-eater by tag, kind, or body plan (§5).
fn is_carnivorous(world: &World, entity: Entity) -> bool {
    if world.get::<Carnivorous>(entity).is_some() {
        return true;
    }
    if let Some(fauna) = world.get::<Fauna>(entity) {
        if fauna.fauna_type == FaunaType::Wolf {
            return true;
        }
    }
    if let Some(body) = world.get::<FaunaBody>(entity) {
        if let Some(head) = body.parts.get(&BodyPartType::Head) {
            return head.stats.attack >= 5.0;
        }
    }
    false
}

/// Consumes every pending [`TameAction`], accumulating progress on its
/// target and converting it to [`Tamed`] at [`TAMING_PROGRESS_MAX`].
///
/// Actions whose power falls below the target's `required_skill` are
/// spent without effect — the animal simply does not respect them.
pub fn execute_tame_action_system(world: &mut World) {
    let actions: Vec<(Entity, TameAction)> = {
        let mut query = world.query::<(Entity, &TameAction)>();
        query.iter(world).map(|(e, a)| (e, *a)).collect()
    };

    for (action_entity, action) in actions {
        world.despawn(action_entity);

        let converted = {
            let Some(mut tameable) = world.get_mut::<Tameable>(action.target) else {
                continue;
            };
            #[allow(clippy::cast_precision_loss)]
            let required = tameable.required_skill as f32;
            if action.taming_power < required {
                continue;
            }
            tameable.progress += action.taming_power;
            tameable.progress >= TAMING_PROGRESS_MAX
        };
        if !converted {
            continue;
        }

        // Conversion (Spec 353 GREEN + §5 seams).
        let owner = if world.get::<Pop>(action.actor).is_some() {
            Some(action.actor)
        } else {
            None
        };
        let fauna_type = world.get::<Fauna>(action.target).map(|f| f.fauna_type);
        let producer = {
            let body = world.get::<FaunaBody>(action.target);
            default_producer_for(fauna_type, body)
        };

        {
            let mut entity_mut = world.entity_mut(action.target);
            entity_mut.remove::<Tameable>();
            entity_mut.insert(Tamed { owner });
            if entity_mut.get::<FeralInstinct>().is_none() {
                entity_mut.insert(FeralInstinct::default());
            }
            if entity_mut.get::<Needs>().is_none() {
                entity_mut.insert(Needs::default());
            }
            if let Some(producer) = producer {
                if entity_mut.get::<ResourceProducer>().is_none() {
                    entity_mut.insert(producer);
                }
            }
        }
        if let Some(mut fauna) = world.get_mut::<Fauna>(action.target) {
            fauna.state = FaunaState::Wander;
            fauna.target = None;
        }
        if let Some(mut skills) = world.get_mut::<Skills>(action.actor) {
            skills.add_xp(SkillType::Husbandry, 50.0);
        }
        let kind_label = fauna_type.map_or("creature".to_string(), |t| format!("{t:?}"));
        send_chronicle(
            world,
            format!("DOMESTICATION: A wild {kind_label} has been tamed and joins the colony."),
            EventImportance::Standard,
        );
    }
}

/// Tamed animals get hungry, and fed tamed animals eat (Spec 353).
///
/// Every tick a tamed animal's hunger decays. A hungry animal
/// (below [`FEED_HUNGER_THRESHOLD`]) grazes from the colony stockpile
/// when food is available — the colony's side of the upkeep bargain.
pub fn tamed_animal_metabolism_system(world: &mut World) {
    let animals: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<Tamed>>();
        query.iter(world).collect()
    };

    let mut grazers: Vec<Entity> = Vec::new();
    for entity in &animals {
        if let Some(mut needs) = world.get_mut::<Needs>(*entity) {
            needs.hunger = (needs.hunger - ANIMAL_HUNGER_DECAY).max(0.0);
            if needs.hunger < FEED_HUNGER_THRESHOLD {
                grazers.push(*entity);
            }
        }
    }

    if grazers.is_empty() || world.get_resource::<ColonyResources>().is_none() {
        return;
    }
    for entity in grazers {
        let stock = world.resource::<ColonyResources>().food;
        if stock < FEED_FOOD_COST {
            break;
        }
        world.resource_mut::<ColonyResources>().food -= FEED_FOOD_COST;
        if let Some(mut needs) = world.get_mut::<Needs>(entity) {
            needs.hunger = (needs.hunger + FEED_HUNGER_GAIN).min(1.0);
        }
    }
}

/// Starving tamed animals revert to the wild (Spec 353).
///
/// Below its [`FeralInstinct`] threshold the animal sheds [`Tamed`] (and
/// its production), regaining a fresh [`Tameable`] so it can be re-tamed
/// once someone feeds the colony again. §5: a carnivorous or territorial
/// animal reverts *hostile* — it turns on the nearest pop in range,
/// which is how a food shortage becomes a predator inside the fence.
pub fn feral_reversion_system(world: &mut World) {
    let reverting: Vec<(Entity, u32, bool)> = {
        let mut query = world
            .query_filtered::<(Entity, &Needs, &FeralInstinct, Has<Territorial>), With<Tamed>>();
        let mut out = Vec::new();
        for (entity, needs, instinct, territorial) in query.iter(world) {
            if needs.hunger < instinct.threshold {
                let hostile_kind = is_carnivorous(world, entity) || territorial;
                out.push((entity, instinct.required_skill, hostile_kind));
            }
        }
        out
    };
    if reverting.is_empty() {
        return;
    }

    // Snapshot pops once for hostile targeting.
    let pops: Vec<(Entity, GridPosition)> = {
        let mut query = world.query_filtered::<(Entity, &GridPosition), With<Pop>>();
        query.iter(world).map(|(e, p)| (e, *p)).collect()
    };

    for (entity, required_skill, hostile_kind) in reverting {
        let animal_pos = world.get::<GridPosition>(entity).copied();
        {
            let mut entity_mut = world.entity_mut(entity);
            entity_mut.remove::<Tamed>();
            entity_mut.remove::<ResourceProducer>();
            entity_mut.insert(Tameable {
                required_skill,
                progress: 0.0,
            });
        }

        let mut turned_hostile = false;
        if hostile_kind {
            if let (Some(pos), Some(mut fauna)) = (animal_pos, world.get_mut::<Fauna>(entity)) {
                let nearest = pops
                    .iter()
                    .map(|(e, p)| (*e, pos.distance_chebyshev(*p)))
                    .filter(|(_, d)| *d <= HOSTILE_REVERSION_RANGE)
                    .min_by_key(|(_, d)| *d);
                if let Some((target, _)) = nearest {
                    fauna.state = FaunaState::Attack;
                    fauna.target = Some(target);
                    turned_hostile = true;
                }
            }
        }

        if turned_hostile {
            send_chronicle(
                world,
                "FERAL REVERSION: A starved tamed beast has gone feral and turned on the colony!"
                    .to_string(),
                EventImportance::Major,
            );
        } else {
            send_chronicle(
                world,
                "FERAL REVERSION: A starved tamed animal has slipped back into the wild."
                    .to_string(),
                EventImportance::Standard,
            );
        }
    }
}

/// Tamed animals with a [`ResourceProducer`] yield on a timer (Spec 353).
///
/// §5: the yield drops at the animal's own [`GridPosition`] as a real
/// [`ResourceItem`], ready to be hauled — never into the void.
pub fn animal_production_system(world: &mut World) {
    let mut drops: Vec<(Option<GridPosition>, ResourceType)> = Vec::new();
    {
        let mut query = world.query::<(&mut ResourceProducer, Option<&GridPosition>, Has<Tamed>)>();
        for (mut producer, pos, tamed) in query.iter_mut(world) {
            if !tamed {
                continue;
            }
            producer.progress += 1.0;
            if producer.progress >= producer.max {
                producer.progress = 0.0;
                drops.push((pos.copied(), producer.resource_type));
            }
        }
    }
    for (pos, resource_type) in drops {
        let item = ResourceItem {
            resource_type,
            amount: 1.0,
        };
        if let Some(pos) = pos {
            world.spawn((item, pos));
        } else {
            world.spawn(item);
        }
    }
}

/// §5 bonding: a keeper near their animal carries a small, refreshed
/// mood buff. Stepping away lets it expire on its own.
pub fn bonding_mood_system(world: &mut World) {
    let bonds: Vec<(Entity, GridPosition)> = {
        let mut query = world.query::<(&Tamed, &GridPosition)>();
        query
            .iter(world)
            .filter_map(|(tamed, pos)| tamed.owner.map(|owner| (owner, *pos)))
            .collect()
    };
    let mut keepers: Vec<Entity> = Vec::new();
    for (owner, animal_pos) in bonds {
        if let Some(owner_pos) = world.get::<GridPosition>(owner).copied() {
            if owner_pos.distance_chebyshev(animal_pos) <= BOND_RANGE && !keepers.contains(&owner) {
                keepers.push(owner);
            }
        }
    }
    for owner in keepers {
        if let Some(mut morale) = world.get_mut::<Morale>(owner) {
            if let Some(existing) = morale
                .modifiers
                .iter_mut()
                .find(|m| m.label == BOND_MODIFIER_LABEL)
            {
                existing.duration = existing.duration.max(BOND_MODIFIER_DURATION);
            } else {
                morale.add_modifier(MoodModifier {
                    label: BOND_MODIFIER_LABEL.to_string(),
                    value: BOND_MODIFIER_VALUE,
                    duration: BOND_MODIFIER_DURATION,
                });
            }
        }
    }
}

/// §5 grief: when a bonded animal dies, its keeper takes a heavy,
/// long-lived morale penalty and a lasting memory of the loss.
pub fn bonded_grief_system(world: &mut World) {
    let dead_bonded: Vec<(Entity, Option<Entity>)> = {
        let mut query =
            world.query_filtered::<(Entity, &Tamed), (With<Dead>, Without<BondGriefProcessed>)>();
        query.iter(world).map(|(e, t)| (e, t.owner)).collect()
    };
    if dead_bonded.is_empty() {
        return;
    }
    let tick = world
        .get_resource::<crate::shared::time::SimulationTime>()
        .map_or(0, |t| t.tick);
    for (entity, _) in &dead_bonded {
        world.entity_mut(*entity).insert(BondGriefProcessed);
    }
    let mut grieved = false;
    for (_, owner) in dead_bonded {
        let Some(owner) = owner else { continue };
        if let Some(mut morale) = world.get_mut::<Morale>(owner) {
            if !morale
                .modifiers
                .iter()
                .any(|m| m.label == GRIEF_MODIFIER_LABEL)
            {
                morale.add_modifier(MoodModifier {
                    label: GRIEF_MODIFIER_LABEL.to_string(),
                    value: GRIEF_MODIFIER_VALUE,
                    duration: GRIEF_MODIFIER_DURATION,
                });
                grieved = true;
            }
        }
        if let Some(mut memories) = world.get_mut::<Memories>(owner) {
            memories.add(MemoryType::MascotDeath, tick);
            grieved = true;
        }
    }
    if grieved {
        send_chronicle(
            world,
            "A tamed companion has died; its keeper grieves.".to_string(),
            EventImportance::Major,
        );
    }
}

/// Number of progressively-tamed animals alive in the world (STATS).
#[must_use]
pub fn tamed_count(world: &mut World) -> usize {
    world.query::<&Tamed>().iter(world).count()
}

/// Number of wild-but-tameable animals in the world (STATS).
#[must_use]
pub fn tameable_count(world: &mut World) -> usize {
    world.query::<&Tameable>().iter(world).count()
}

/// Spawns a wild, tameable beast of the given kind at `pos` (headless
/// debug seam). `kind` is `wolf`, `rat`, or `mascot` (anything else
/// falls back to a wolf). Wolves arrive carnivorous, with the teeth to
/// prove it; rats arrive brush-harvestable; all arrive hungry-neutral
/// with a fresh [`Needs`] and a kind-appropriate skill gate.
pub fn spawn_tameable_beast(world: &mut World, kind: &str, pos: GridPosition) -> Entity {
    use crate::layer1::health::Health;
    let (fauna_type, required_skill) = match kind.to_ascii_lowercase().as_str() {
        "rat" | "spacerat" | "space_rat" => (FaunaType::SpaceRat, 5),
        "mascot" => (FaunaType::Mascot, 3),
        _ => (FaunaType::Wolf, 10),
    };
    let mut body = FaunaBody {
        name: format!("Wild {fauna_type:?}"),
        ..Default::default()
    };
    if fauna_type == FaunaType::Wolf {
        body.add_part(crate::layer1::fauna::FaunaPart {
            part_type: BodyPartType::Head,
            name: "Snapping Maw".to_string(),
            stats: crate::layer1::fauna::FaunaStats {
                attack: 6.0,
                ..Default::default()
            },
            resource_drop: None,
        });
    }
    let entity = world
        .spawn((
            Fauna {
                fauna_type,
                ..Default::default()
            },
            body,
            pos,
            Health::default(),
            Needs::default(),
            Tameable::new(required_skill),
        ))
        .id();
    if fauna_type == FaunaType::Wolf {
        world.entity_mut(entity).insert(Carnivorous);
    }
    entity
}

/// Files one taming action from `actor` against `target` and resolves it
/// immediately (headless debug seam over [`execute_tame_action_system`]).
/// Returns the target's progress afterwards, or `None` if the target is
/// not tameable.
pub fn apply_tame_attempt(world: &mut World, actor: Entity, target: Entity) -> Option<f32> {
    world.get::<Tameable>(target)?;
    let power = taming_power_from_skills(world.get::<Skills>(actor));
    world.spawn(TameAction {
        target,
        actor,
        taming_power: power,
    });
    execute_tame_action_system(world);
    if world.get::<Tamed>(target).is_some() {
        Some(TAMING_PROGRESS_MAX)
    } else {
        world.get::<Tameable>(target).map(|t| t.progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::skills::Skills;

    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(crate::shared::time::SimulationTime::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world
    }

    // --- Spec 353 RED test 1, adapted: progressive conversion. The
    // spec's single 15-power action could not cross its own 100
    // threshold, so the adapted animal starts at 90 progress. ---
    #[test]
    fn test_taming_action_converts_wild_fauna() {
        let mut world = setup_world();
        let animal = world
            .spawn((
                Fauna::default(),
                GridPosition { x: 0, y: 0 },
                Tameable {
                    required_skill: 5,
                    progress: 90.0,
                },
            ))
            .id();
        let pop = world.spawn(Pop).id();
        world.spawn(TameAction {
            target: animal,
            actor: pop,
            taming_power: 15.0,
        });

        execute_tame_action_system(&mut world);

        assert!(
            world.get::<Tamed>(animal).is_some(),
            "animal should be Tamed"
        );
        assert!(
            world.get::<Tameable>(animal).is_none(),
            "Tameable should be removed on conversion"
        );
        assert_eq!(world.get::<Tamed>(animal).unwrap().owner, Some(pop));
        assert!(
            world.get::<Needs>(animal).is_some(),
            "conversion should give the animal real Needs for upkeep"
        );
        assert!(
            world.get::<FeralInstinct>(animal).is_some(),
            "conversion should leave a FeralInstinct behind"
        );
    }

    #[test]
    fn test_taming_action_accumulates_progress() {
        let mut world = setup_world();
        let animal = world.spawn((Fauna::default(), Tameable::new(5))).id();
        let pop = world.spawn(Pop).id();
        for _ in 0..2 {
            world.spawn(TameAction {
                target: animal,
                actor: pop,
                taming_power: 15.0,
            });
            execute_tame_action_system(&mut world);
        }
        let tameable = world.get::<Tameable>(animal).unwrap();
        assert!((tameable.progress - 30.0).abs() < f32::EPSILON);
        assert!(world.get::<Tamed>(animal).is_none());
        // The action entities are consumed, not accumulated.
        assert_eq!(world.query::<&TameAction>().iter(&world).count(), 0);
    }

    #[test]
    fn test_taming_action_below_required_skill_makes_no_progress() {
        let mut world = setup_world();
        let animal = world.spawn((Fauna::default(), Tameable::new(10))).id();
        let pop = world.spawn(Pop).id();
        world.spawn(TameAction {
            target: animal,
            actor: pop,
            taming_power: 5.0,
        });
        execute_tame_action_system(&mut world);
        assert_eq!(world.get::<Tameable>(animal).unwrap().progress, 0.0);
        assert!(world.get::<Tamed>(animal).is_none());
    }

    #[test]
    fn test_taming_power_scales_with_husbandry_skill() {
        assert!((taming_power_from_skills(None) - 10.0).abs() < f32::EPSILON);
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Husbandry, 1000.0); // Level 3
        assert_eq!(skills.get_level(SkillType::Husbandry), 3);
        assert!((taming_power_from_skills(Some(&skills)) - 25.0).abs() < f32::EPSILON);
    }

    // --- Spec 353 RED test 2, adapted to the real 0.0–1.0 Needs. ---
    #[test]
    fn test_tamed_animals_consume_food() {
        let mut world = setup_world();
        let animal = world
            .spawn((
                Tamed { owner: None },
                Needs {
                    hunger: 0.8,
                    ..Default::default()
                },
            ))
            .id();
        tamed_animal_metabolism_system(&mut world);
        let needs = world.get::<Needs>(animal).unwrap();
        assert!(needs.hunger < 0.8, "hunger should decay (gets hungrier)");
        assert!((needs.hunger - 0.79).abs() < 0.001);
    }

    #[test]
    fn test_tamed_animal_grazes_from_colony_stockpile() {
        let mut world = setup_world();
        world.insert_resource(ColonyResources {
            food: 10.0,
            ..Default::default()
        });
        let animal = world
            .spawn((
                Tamed { owner: None },
                Needs {
                    hunger: 0.5,
                    ..Default::default()
                },
            ))
            .id();
        tamed_animal_metabolism_system(&mut world);
        // 0.5 - 0.01 decay = 0.49 < 0.6 threshold, so it grazes:
        // stockpile pays 0.05, hunger gains 0.15.
        assert!((world.resource::<ColonyResources>().food - 9.95).abs() < 0.001);
        let needs = world.get::<Needs>(animal).unwrap();
        assert!((needs.hunger - 0.64).abs() < 0.001);
    }

    // --- Spec 353 RED test 3, adapted to the real 0.0–1.0 Needs. ---
    #[test]
    fn test_starving_tamed_animals_revert_to_feral() {
        let mut world = setup_world();
        let animal = world
            .spawn((
                Tamed { owner: None },
                FeralInstinct {
                    threshold: 0.2,
                    ..Default::default()
                },
                Needs {
                    hunger: 0.1,
                    ..Default::default()
                },
                ResourceProducer {
                    resource_type: ResourceType::Food,
                    progress: 10.0,
                    max: 100.0,
                },
            ))
            .id();
        feral_reversion_system(&mut world);
        assert!(world.get::<Tamed>(animal).is_none());
        let tameable = world.get::<Tameable>(animal).expect("re-tameable");
        assert_eq!(tameable.progress, 0.0);
        assert!(
            world.get::<ResourceProducer>(animal).is_none(),
            "a reverted animal stops producing for the colony"
        );
    }

    #[test]
    fn test_fed_tamed_animals_do_not_revert() {
        let mut world = setup_world();
        let animal = world
            .spawn((
                Tamed { owner: None },
                FeralInstinct::default(),
                Needs {
                    hunger: 0.9,
                    ..Default::default()
                },
            ))
            .id();
        feral_reversion_system(&mut world);
        assert!(world.get::<Tamed>(animal).is_some());
    }

    // --- §5: hostile reversion for carnivorous fauna. ---
    #[test]
    fn test_carnivorous_reversion_turns_hostile() {
        let mut world = setup_world();
        let pop = world.spawn((Pop, GridPosition { x: 3, y: 0 })).id();
        let animal = world
            .spawn((
                Fauna {
                    fauna_type: FaunaType::Wolf,
                    ..Default::default()
                },
                GridPosition { x: 0, y: 0 },
                Tamed { owner: None },
                FeralInstinct::default(),
                Needs {
                    hunger: 0.05,
                    ..Default::default()
                },
            ))
            .id();
        feral_reversion_system(&mut world);
        let fauna = world.get::<Fauna>(animal).unwrap();
        assert_eq!(fauna.state, FaunaState::Attack);
        assert_eq!(fauna.target, Some(pop));
    }

    #[test]
    fn test_docile_reversion_stays_calm() {
        let mut world = setup_world();
        world.spawn((Pop, GridPosition { x: 1, y: 0 }));
        let animal = world
            .spawn((
                Fauna {
                    fauna_type: FaunaType::SpaceRat,
                    ..Default::default()
                },
                GridPosition { x: 0, y: 0 },
                Tamed { owner: None },
                FeralInstinct::default(),
                Needs {
                    hunger: 0.05,
                    ..Default::default()
                },
            ))
            .id();
        feral_reversion_system(&mut world);
        let fauna = world.get::<Fauna>(animal).unwrap();
        assert_eq!(fauna.state, FaunaState::Wander);
        assert_eq!(fauna.target, None);
    }

    // --- Spec 353 RED test 4, adapted: yield drops at the animal. ---
    #[test]
    fn test_tamed_animals_produce_resources() {
        let mut world = setup_world();
        let animal = world
            .spawn((
                Tamed { owner: None },
                GridPosition { x: 7, y: 9 },
                ResourceProducer {
                    resource_type: ResourceType::Food,
                    progress: 99.0,
                    max: 100.0,
                },
            ))
            .id();
        animal_production_system(&mut world);
        let producer = world.get::<ResourceProducer>(animal).unwrap();
        assert_eq!(producer.progress, 0.0, "producer should reset");
        let items: Vec<(ResourceItem, GridPosition)> = {
            let mut query = world.query::<(&ResourceItem, &GridPosition)>();
            query.iter(&world).map(|(i, p)| (*i, *p)).collect()
        };
        assert_eq!(items.len(), 1, "exactly one yield should drop");
        assert_eq!(items[0].0.resource_type, ResourceType::Food);
        assert_eq!(
            items[0].1,
            GridPosition { x: 7, y: 9 },
            "drops at the animal"
        );
    }

    #[test]
    fn test_untamed_animals_do_not_produce() {
        let mut world = setup_world();
        world.spawn((
            Tameable::default(),
            GridPosition { x: 0, y: 0 },
            ResourceProducer {
                resource_type: ResourceType::Food,
                progress: 99.0,
                max: 100.0,
            },
        ));
        animal_production_system(&mut world);
        assert_eq!(world.query::<&ResourceItem>().iter(&world).count(), 0);
    }

    // --- §5: bonding and grief. ---
    #[test]
    fn test_bonding_grants_owner_mood_buff_without_stacking() {
        let mut world = setup_world();
        let owner = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Morale::default()))
            .id();
        world.spawn((Tamed { owner: Some(owner) }, GridPosition { x: 2, y: 0 }));
        bonding_mood_system(&mut world);
        bonding_mood_system(&mut world);
        let morale = world.get::<Morale>(owner).unwrap();
        let bonds: Vec<&MoodModifier> = morale
            .modifiers
            .iter()
            .filter(|m| m.label == BOND_MODIFIER_LABEL)
            .collect();
        assert_eq!(bonds.len(), 1, "refresh, never stack");
        assert!((bonds[0].value - 0.05).abs() < f32::EPSILON);
    }

    #[test]
    fn test_no_bond_when_owner_is_far_away() {
        let mut world = setup_world();
        let owner = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Morale::default()))
            .id();
        world.spawn((Tamed { owner: Some(owner) }, GridPosition { x: 40, y: 40 }));
        bonding_mood_system(&mut world);
        assert!(world.get::<Morale>(owner).unwrap().modifiers.is_empty());
    }

    #[test]
    fn test_bonded_death_grieves_owner_exactly_once() {
        let mut world = setup_world();
        let owner = world
            .spawn((Pop, Morale::default(), Memories::default()))
            .id();
        world.spawn((Tamed { owner: Some(owner) }, Dead));
        bonded_grief_system(&mut world);
        bonded_grief_system(&mut world);
        let morale = world.get::<Morale>(owner).unwrap();
        let griefs: Vec<&MoodModifier> = morale
            .modifiers
            .iter()
            .filter(|m| m.label == GRIEF_MODIFIER_LABEL)
            .collect();
        assert_eq!(griefs.len(), 1);
        assert!((griefs[0].value - -0.30).abs() < f32::EPSILON);
        let memories = world.get::<Memories>(owner).unwrap();
        assert_eq!(memories.items.len(), 1, "one lasting memory of the loss");
        assert_eq!(memories.items[0].memory_type, MemoryType::MascotDeath);
    }

    #[test]
    fn test_counts_and_spawn_seam() {
        let mut world = setup_world();
        let wolf = spawn_tameable_beast(&mut world, "wolf", GridPosition { x: 5, y: 5 });
        spawn_tameable_beast(&mut world, "rat", GridPosition { x: 6, y: 5 });
        world.spawn(Tamed { owner: None });
        assert_eq!(tameable_count(&mut world), 2);
        assert_eq!(tamed_count(&mut world), 1);
        assert!(
            world.get::<Carnivorous>(wolf).is_some(),
            "wolves are carnivorous"
        );
        assert_eq!(
            world.get::<Tameable>(wolf).unwrap().required_skill,
            10,
            "wolves are the hard tame"
        );
    }

    #[test]
    fn test_ip_guard_original_names_only() {
        // Spec 353 is concept-only: no lifted creature-franchise names
        // may appear in any player-facing domestication string.
        let public_strings = [
            BOND_MODIFIER_LABEL,
            GRIEF_MODIFIER_LABEL,
            "DOMESTICATION: A wild creature has been tamed and joins the colony.",
            "FERAL REVERSION: A starved tamed beast has gone feral and turned on the colony!",
            "FERAL REVERSION: A starved tamed animal has slipped back into the wild.",
            "A tamed companion has died; its keeper grieves.",
        ];
        let banned = [
            "pokemon",
            "pokémon",
            "digimon",
            "tamagotchi",
            "jurassic",
            "monster hunter",
            "palworld",
            "how to train your dragon",
        ];
        for text in public_strings {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "player-facing string '{text}' contains banned IP term '{term}'"
                );
            }
        }
    }
}
