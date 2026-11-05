//! Spec 392 — Tool Economy: durability, breakage, and replacement.
//!
//! Grounding (prior art, do not duplicate):
//! - Spec 030 built the global pool: `ColonyResources.tools`, the Smithy
//!   building, and its refining recipe (metal + wood -> tools).
//! - Spec 058 built personal tools: `Equipment.tool` points at a `Tool`
//!   entity with `durability`/`max_durability`; `FetchTool` utility AI
//!   fetches a replacement from the stockpile pool; the World-based
//!   `work_execution_system` already drains durability while a pop works
//!   and despawns the tool at zero.
//! - Spec 218 / 1125 built the improvised fallback: global
//!   `calculate_work_efficiency` yields 1.0 / 0.75 / 0.5.
//! - Spec 1086 heirloom tools ride the same `Tool` entity.
//!
//! What was missing for Spec 392, and what this module adds:
//! 1. A severe work penalty — exactly 0.1x — when a pop does tool-requiring
//!    work with a broken or missing required tool (the old bare-hands
//!    fallback of 0.5x was far too gentle to create a tool economy).
//! 2. A dedicated [`ToolBrokenEvent`] on breakage (previously only a log
//!    line and a generic `UnequipEvent`).
//! 3. Replacement urgency scoring: replacing a broken/missing required
//!    tool scores 0.95 so utility AI strongly prioritizes it.
//!
//! Adaptation notes: the spec sketch invented `Job { task: String }`, a
//! `HashMap`-based `Equipment`, and `WorkAction`. The real codebase has
//! `Job { workplace, job_type }`, `Equipment { tool: Option<Entity> }`,
//! and active work is signalled by `PopAction::Work` + `AtTarget` (the
//! same gate `work_execution_system` uses). Tests below use the real
//! types throughout.

use bevy_ecs::prelude::*;

use crate::layer1::designation::DesignationType;
use crate::layer1::execution::AtTarget;
use crate::layer1::items::{Equipment, Tool, ToolType};
use crate::layer1::pop::Job;
use crate::layer1::utility_types::{ActionType, AssignmentType, PopAction};

/// Durability lost per tick of active work. Mirrors the drain the
/// World-based execution path applies (`TOOL_DURABILITY_LOSS` there is
/// defined from this constant so the two paths cannot drift).
pub const TOOL_WEAR_PER_WORK_TICK: f32 = 0.1;

/// Work-speed factor when a required tool is broken or missing: a 90%
/// penalty (Spec 392 acceptance criterion).
pub const BROKEN_TOOL_WORK_FACTOR: f32 = 0.1;

/// Utility score for fetching a replacement when a required tool is
/// broken or missing. Deliberately above the 0.8 "strongly prioritized"
/// line in the spec, and above ordinary logistics actions.
pub const REPLACEMENT_TOOL_UTILITY: f32 = 0.95;

/// Utility score for replacing a badly worn (but unbroken) tool.
pub const WORN_TOOL_UTILITY: f32 = 0.85;

/// A tool at or below this fraction of max durability counts as worn.
pub const WORN_DURABILITY_FRACTION: f32 = 0.25;

/// User-facing line emitted when a tool breaks.
pub const TOOL_BROKEN_MESSAGE: &str = "CRACK! A tool has broken.";

/// Event fired exactly when an equipped tool's durability reaches zero.
///
/// The World-based execution path emits this alongside the pre-existing
/// `UnequipEvent`; [`apply_tool_wear_system`] emits it for Bevy-schedule
/// consumers and tests.
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolBrokenEvent {
    /// The pop who was holding the tool.
    pub pop: Entity,
    /// The tool entity that broke (despawned by the wear path).
    pub tool: Entity,
    /// What kind of tool broke.
    pub tool_type: ToolType,
}

/// Per-pop work-speed factor derived from tool state (Spec 392's
/// `WorkSpeed`, adapted to a dedicated component so it composes with —
/// rather than overwrites — the existing `WorkSpeedBuff` /
/// `WorkEfficiency` multipliers).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct ToolWorkSpeed(pub f32);

impl Default for ToolWorkSpeed {
    fn default() -> Self {
        Self(1.0)
    }
}

/// The tool a designation logically calls for, if any.
///
/// Only work designations that the World-based execution path actually
/// performs are mapped; social/zone designations need no tool.
#[must_use]
pub const fn required_tool_for_designation(designation: DesignationType) -> Option<ToolType> {
    match designation {
        DesignationType::Mine | DesignationType::ExcavateDust => Some(ToolType::Pickaxe),
        DesignationType::Chop | DesignationType::ClearFlora => Some(ToolType::Axe),
        DesignationType::Repair
        | DesignationType::JuryRig
        | DesignationType::Demolish
        | DesignationType::Cannibalize
        | DesignationType::Destroy => Some(ToolType::Hammer),
        _ => None,
    }
}

/// Whether a designation requires a working tool at all.
#[must_use]
pub const fn designation_requires_tool(designation: DesignationType) -> bool {
    required_tool_for_designation(designation).is_some()
}

/// The tool a formal job assignment calls for, if any.
///
/// Only deep mining has an unambiguous tool in the current assignment
/// set; other assignments work through designations instead.
#[must_use]
pub const fn required_tool_for_assignment(assignment: AssignmentType) -> Option<ToolType> {
    match assignment {
        AssignmentType::DeepMining => Some(ToolType::Pickaxe),
        _ => None,
    }
}

/// Whether an equipped tool satisfies a requirement.
///
/// Production reality: the Smithy issues one generic tool (the
/// `handle_fetch_tool` arrival handler spawns a Pickaxe "generic for
/// now"), so a Pickaxe satisfies any requirement until typed tool
/// production lands. Other types satisfy only their own requirement.
#[must_use]
pub const fn tool_satisfies(equipped: ToolType, required: ToolType) -> bool {
    equipped as u8 == required as u8 || matches!(equipped, ToolType::Pickaxe)
}

/// Work-speed factor for a piece of work.
///
/// - No tool required: 1.0 (tool state is irrelevant).
/// - Required tool present, satisfying, and unbroken: 1.0.
/// - Required tool missing, wrong for the job, or broken: 0.1.
#[must_use]
pub fn tool_speed_factor(required: Option<ToolType>, equipped: Option<(ToolType, f32)>) -> f32 {
    let Some(required) = required else {
        return 1.0;
    };
    match equipped {
        Some((tool_type, durability))
            if durability > 0.0 && tool_satisfies(tool_type, required) =>
        {
            1.0
        }
        _ => BROKEN_TOOL_WORK_FACTOR,
    }
}

/// Apply one work tick of wear. Returns `(new_durability, broke_now)`.
#[must_use]
pub fn wear_tool(current: f32, amount: f32) -> (f32, bool) {
    let new = (current - amount).max(0.0);
    (new, new <= 0.0)
}

/// Utility score for fetching a replacement tool, or `None` when the pop
/// should not fetch one (healthy tool, or the stockpile pool is empty —
/// in which case the Smithy, not the pop, is the bottleneck).
#[must_use]
pub fn equip_tool_score(
    required: Option<ToolType>,
    equipped: Option<(ToolType, f32, f32)>,
    stockpile_tools: f32,
) -> Option<f32> {
    if stockpile_tools < 1.0 {
        return None;
    }
    match equipped {
        None => Some(REPLACEMENT_TOOL_UTILITY),
        Some((tool_type, durability, max_durability)) => {
            let broken_or_wrong =
                durability <= 0.0 || required.is_some_and(|req| !tool_satisfies(tool_type, req));
            if broken_or_wrong {
                return Some(REPLACEMENT_TOOL_UTILITY);
            }
            let fraction = if max_durability > 0.0 {
                durability / max_durability
            } else {
                0.0
            };
            if fraction <= WORN_DURABILITY_FRACTION {
                return Some(WORN_TOOL_UTILITY);
            }
            None
        }
    }
}

/// Bevy-system form of tool wear (Spec 392 RED test shape): pops actively
/// working (`PopAction::Work` + `AtTarget`, the same gate the World-based
/// execution path uses) drain their equipped tool; at zero the tool
/// breaks — [`ToolBrokenEvent`] fires, the tool despawns, and the pop's
/// equipment slot clears so `FetchTool` can replace it.
///
/// The live simulation schedule does not run this system: the World-based
/// `work_execution_system` already applies wear at the moment work is
/// performed (and now emits the same event). Running both would double
/// the drain. This system is the schedule-native seam for consumers and
/// tests that build an `App` rather than driving a raw `World`.
pub fn apply_tool_wear_system(
    mut commands: Commands,
    mut broken_events: EventWriter<ToolBrokenEvent>,
    pops: Query<(Entity, &Equipment, &PopAction, Has<AtTarget>)>,
    mut tools: Query<&mut Tool>,
) {
    for (pop, equipment, action, at_target) in &pops {
        if action.current != ActionType::Work || !at_target {
            continue;
        }
        let Some(tool_entity) = equipment.tool else {
            continue;
        };
        let Ok(mut tool) = tools.get_mut(tool_entity) else {
            continue;
        };
        let tool_type = tool.tool_type;
        let (new_durability, broke) = wear_tool(tool.durability, TOOL_WEAR_PER_WORK_TICK);
        tool.durability = new_durability;
        if broke {
            broken_events.send(ToolBrokenEvent {
                pop,
                tool: tool_entity,
                tool_type,
            });
            commands.entity(tool_entity).despawn();
            commands.entity(pop).insert(Equipment {
                tool: None,
                ..*equipment
            });
        }
    }
}

/// Maintains [`ToolWorkSpeed`] from each pop's equipped tool and job.
pub fn calculate_work_speed_system(
    mut pops: Query<(&Equipment, Option<&Job>, &mut ToolWorkSpeed)>,
    tools: Query<&Tool>,
) {
    for (equipment, job, mut speed) in &mut pops {
        let required = job.and_then(|j| required_tool_for_assignment(j.job_type));
        let equipped = equipment
            .tool
            .and_then(|e| tools.get(e).ok())
            .map(|t| (t.tool_type, t.durability));
        // A broken tool still in hand penalizes any work, even work with
        // no formal tool requirement; otherwise the factor keys off the
        // job's required tool.
        speed.0 = match equipped {
            Some((_, durability)) if durability <= 0.0 => BROKEN_TOOL_WORK_FACTOR,
            _ => tool_speed_factor(required, equipped),
        };
    }
}

/// Census of colony tool state for headless observability.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ToolCensus {
    /// Pops carrying an `Equipment` component.
    pub pops: usize,
    /// Pops with a tool equipped.
    pub equipped: usize,
    /// Pops with no tool equipped.
    pub toolless: usize,
    /// Equipped tools at or below the worn threshold.
    pub worn: usize,
    /// Mean durability fraction of equipped tools (0.0 when none).
    pub avg_durability: f32,
}

/// Count tool state across all pops in a raw `World`.
#[must_use]
pub fn tool_census(world: &mut World) -> ToolCensus {
    let mut census = ToolCensus::default();
    let mut durability_sum = 0.0f32;
    let tool_entities: Vec<Entity> = {
        let mut q = world.query::<(Entity, &Equipment)>();
        let mut v = Vec::new();
        for (_, equipment) in q.iter(world) {
            census.pops += 1;
            match equipment.tool {
                Some(tool) => {
                    census.equipped += 1;
                    v.push(tool);
                }
                None => census.toolless += 1,
            }
        }
        v
    };
    for tool_entity in tool_entities {
        if let Some(tool) = world.get::<Tool>(tool_entity) {
            let fraction = if tool.max_durability > 0.0 {
                tool.durability / tool.max_durability
            } else {
                0.0
            };
            durability_sum += fraction;
            if fraction <= WORN_DURABILITY_FRACTION {
                census.worn += 1;
            }
        }
    }
    if census.equipped > 0 {
        census.avg_durability = durability_sum / census.equipped as f32;
    }
    census
}

/// User-facing strings introduced by this module (IP-guard surface).
#[must_use]
pub fn tool_economy_public_strings() -> Vec<&'static str> {
    vec![
        TOOL_BROKEN_MESSAGE,
        "Tool Economy",
        "equipped",
        "toolless",
        "worn",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::execution::AtTarget;
    use crate::layer1::items::Item;
    use bevy::app::{App, Update};

    fn spawn_worker(
        app: &mut App,
        tool: Option<(ToolType, f32)>,
        active: bool,
    ) -> (Entity, Option<Entity>) {
        let tool_entity = tool.map(|(tool_type, durability)| {
            app.world_mut()
                .spawn((
                    Item::default(),
                    Tool {
                        tool_type,
                        durability,
                        max_durability: 100.0,
                    },
                ))
                .id()
        });
        let mut pop = app.world_mut().spawn((
            Equipment {
                tool: tool_entity,
                ..Default::default()
            },
            PopAction {
                current: if active {
                    ActionType::Work
                } else {
                    ActionType::Idle
                },
                ..Default::default()
            },
            ToolWorkSpeed(1.0),
        ));
        if active {
            pop.insert(AtTarget);
        }
        (pop.id(), tool_entity)
    }

    // Spec RED test 1, adapted: Job{task:String} -> active-work markers,
    // HashMap Equipment -> real Equipment + Tool entity.
    #[test]
    fn test_tool_durability_drain() {
        let mut app = App::new();
        app.add_event::<ToolBrokenEvent>();
        app.add_systems(Update, apply_tool_wear_system);
        let (_, tool) = spawn_worker(&mut app, Some((ToolType::Pickaxe, 100.0)), true);
        app.update();
        let t = app.world().get::<Tool>(tool.unwrap()).unwrap();
        assert!(
            t.durability < 100.0,
            "durability should decrease on active work"
        );
    }

    #[test]
    fn test_tool_wear_only_when_active() {
        let mut app = App::new();
        app.add_event::<ToolBrokenEvent>();
        app.add_systems(Update, apply_tool_wear_system);
        let (_, tool) = spawn_worker(&mut app, Some((ToolType::Pickaxe, 100.0)), false);
        app.update();
        let t = app.world().get::<Tool>(tool.unwrap()).unwrap();
        assert_eq!(t.durability, 100.0, "idle pops do not wear tools");
    }

    #[test]
    fn test_tool_breakage_emits_event_and_clears_equipment() {
        let mut app = App::new();
        app.add_event::<ToolBrokenEvent>();
        app.add_systems(Update, apply_tool_wear_system);
        let (pop, tool) = spawn_worker(&mut app, Some((ToolType::Pickaxe, 0.05)), true);
        app.update();
        assert!(
            app.world().get_entity(tool.unwrap()).is_err(),
            "broken tool entity should be despawned"
        );
        let eq = app.world().get::<Equipment>(pop).unwrap();
        assert!(eq.tool.is_none(), "equipment slot should clear on breakage");
        let events = app.world().resource::<Events<ToolBrokenEvent>>();
        let mut reader = events.get_cursor();
        let fired: Vec<_> = reader.read(events).collect();
        assert_eq!(fired.len(), 1, "exactly one ToolBrokenEvent should fire");
        assert_eq!(fired[0].tool_type, ToolType::Pickaxe);
        assert_eq!(fired[0].pop, pop);
    }

    // Spec RED test 2, adapted: WorkSpeed -> ToolWorkSpeed component.
    #[test]
    fn test_tool_breakage_slows_work() {
        let mut app = App::new();
        app.add_systems(Update, calculate_work_speed_system);
        let (pop, _) = spawn_worker(&mut app, Some((ToolType::Pickaxe, 0.0)), false);
        // Give the pop a job that requires the (broken) pickaxe.
        let workplace = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(pop).insert(Job {
            workplace,
            job_type: AssignmentType::DeepMining,
        });
        app.update();
        let speed = app.world().get::<ToolWorkSpeed>(pop).unwrap();
        assert!(speed.0 < 1.0, "broken tool should apply severe penalty");
        assert_eq!(speed.0, 0.1);
    }

    #[test]
    fn test_missing_required_tool_slows_work() {
        assert_eq!(
            tool_speed_factor(Some(ToolType::Pickaxe), None),
            BROKEN_TOOL_WORK_FACTOR
        );
        assert_eq!(
            tool_speed_factor(Some(ToolType::Pickaxe), Some((ToolType::Pickaxe, 50.0))),
            1.0
        );
        assert_eq!(tool_speed_factor(None, None), 1.0);
    }

    // Spec RED test 3, adapted: UtilityScores HashMap -> pure scoring seam
    // used by evaluate_fetch_tool (the real utility-AI entry point).
    #[test]
    fn test_utility_ai_seeks_replacement_tool() {
        let score = equip_tool_score(
            Some(ToolType::Pickaxe),
            Some((ToolType::Pickaxe, 0.0, 100.0)),
            3.0,
        )
        .expect("broken required tool with stock available should score");
        assert!(
            score > 0.8,
            "replacement should be heavily prioritized, got {score}"
        );
        // Missing tool scores the same.
        let missing = equip_tool_score(Some(ToolType::Pickaxe), None, 3.0).unwrap();
        assert!(missing > 0.8);
        // Healthy tool: no fetch.
        assert_eq!(
            equip_tool_score(
                Some(ToolType::Pickaxe),
                Some((ToolType::Pickaxe, 90.0, 100.0)),
                3.0
            ),
            None
        );
        // Empty stockpile: no fetch (Smithy is the bottleneck).
        assert_eq!(equip_tool_score(Some(ToolType::Pickaxe), None, 0.0), None);
    }

    #[test]
    fn test_required_tool_mapping() {
        assert_eq!(
            required_tool_for_designation(DesignationType::Mine),
            Some(ToolType::Pickaxe)
        );
        assert_eq!(
            required_tool_for_designation(DesignationType::Chop),
            Some(ToolType::Axe)
        );
        assert_eq!(required_tool_for_designation(DesignationType::Tame), None);
        // The generic issued tool satisfies any requirement (see fn docs).
        assert!(tool_satisfies(ToolType::Pickaxe, ToolType::Axe));
        assert!(!tool_satisfies(ToolType::Axe, ToolType::Pickaxe));
    }

    #[test]
    fn test_smithy_replacement_integration() {
        // Real seam: the Smithy refining recipe turns metal + wood into
        // the ColonyResources.tools pool that FetchTool draws from.
        let resources = crate::layer1::resources::ColonyResources {
            metal: 1.0,
            wood: 1.0,
            tools: 0.0,
            ..Default::default()
        };
        let (can_refine, _input, output, _) = crate::layer1::refining::get_refining_recipe(
            crate::layer1::building::BuildingType::Smithy,
            &resources,
        );
        assert!(can_refine);
        assert_eq!(output.tools, 1.0);
    }

    #[test]
    fn test_work_amount_penalty_in_production_path() {
        // The real seam: calculate_work_amount (used by
        // work_execution_system) applies the 0.1 factor for Mine work
        // with no pickaxe, overriding the improvised fallback.
        use crate::layer1::execution::calculate_work_amount;
        let mut world = World::new();
        let pop = world.spawn_empty().id();
        let with_tool_entity = world
            .spawn(Tool {
                tool_type: ToolType::Pickaxe,
                durability: 100.0,
                max_durability: 100.0,
            })
            .id();
        let with_tool = calculate_work_amount(
            &world,
            pop,
            DesignationType::Mine,
            Some(with_tool_entity),
            50.0,
            1.0,
            0.5,
        );
        let without_tool =
            calculate_work_amount(&world, pop, DesignationType::Mine, None, 50.0, 1.0, 0.5);
        assert!(with_tool > 0.0);
        assert!(
            without_tool < with_tool * 0.2,
            "missing required tool should crater work amount: {without_tool} vs {with_tool}"
        );
        // Non-tool work is untouched by the penalty.
        let tame = calculate_work_amount(&world, pop, DesignationType::Tame, None, 50.0, 1.0, 0.5);
        assert!(tame > without_tool);
    }

    #[test]
    fn test_ip_guard_original_names_only() {
        const BANNED: &[&str] = &[
            "Minecraft",
            "Dwarf Fortress",
            "RimWorld",
            "Terraria",
            "Factorio",
        ];
        for text in tool_economy_public_strings() {
            for banned in BANNED {
                assert!(
                    !text.contains(banned),
                    "public string {text:?} contains banned IP term {banned:?}"
                );
            }
        }
    }
}
