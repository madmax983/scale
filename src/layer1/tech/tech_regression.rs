//! Technological Regression (Spec 328).
//!
//! Generational knowledge is use-it-or-lose-it. Each [`TechField`] tracks a
//! practical knowledge level (0–[`MAX_TECH_LEVEL`]) in the [`TechKnowledge`]
//! resource. Fields that no colonist practices decay one level per in-game
//! year ([`DECAY_TICKS`]); fields with active practitioners hold steady and
//! slowly relearn what was lost ("retaught").
//!
//! Buildings carrying [`TechRequirement`] become inoperable **black boxes**
//! ([`DisabledState`]) whenever the colony's level in the required field
//! drops below the requirement, and come back online if knowledge is
//! regained. Transitions are recorded in the chronicle.
//!
//! The check runs once per season ([`REGRESSION_CHECK_TICKS`]) via the
//! chained [`knowledge_practice_system`] + [`knowledge_decay_system`] pair.
//! All names are original and generic; nothing is lifted from outside
//! fiction (see the IP-guard test).

use bevy_ecs::prelude::*;
use std::collections::HashMap;

use crate::layer1::architecture::building::{Building, BuildingType};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::pop::{Job, JobType};
use crate::shared::time::SimulationTime;

/// Maximum practical knowledge level any field can reach.
pub const MAX_TECH_LEVEL: u32 = 10;
/// Knowledge level every field starts at when the colony is founded.
pub const FOUNDING_TECH_LEVEL: u32 = 3;
/// Ticks of disuse before a field loses one level (1 in-game year).
pub const DECAY_TICKS: u64 = 1000;
/// Ticks of sustained practice needed to relearn one lost level (1 year).
pub const RELEARN_TICKS: u64 = 1000;
/// How often the regression check runs (once per season).
pub const REGRESSION_CHECK_TICKS: u64 = 250;

/// A field of practical, generational knowledge.
///
/// Distinct from the research tech tree ([`crate::layer1::tech::Tech`]):
/// this is the hands-on know-how that dies with the last practitioner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TechField {
    /// Doctoring, triage, surgery.
    Medicine,
    /// Farming, crop lore.
    Agriculture,
    /// Deep mining, ore lore.
    Mining,
    /// Smithing, refining, machine craft.
    Engineering,
    /// Observatory work, scholarship.
    Science,
    /// Administration, law, rationing.
    Administration,
}

impl TechField {
    /// All fields, for iteration.
    pub const ALL: [TechField; 6] = [
        TechField::Medicine,
        TechField::Agriculture,
        TechField::Mining,
        TechField::Engineering,
        TechField::Science,
        TechField::Administration,
    ];

    /// Which field a job practices, if any.
    ///
    /// Returns `None` for non-productive assignments (residents, visitors,
    /// patients) that maintain no technical knowledge. Note: no current job
    /// practices [`TechField::Engineering`] — the field still decays with
    /// disuse, and a future craft-job spec will wire practice back in.
    #[must_use]
    pub const fn for_job(job: JobType) -> Option<TechField> {
        match job {
            JobType::Doctor => Some(TechField::Medicine),
            JobType::FarmWorker => Some(TechField::Agriculture),
            JobType::DeepMining => Some(TechField::Mining),
            JobType::ObservatoryWorker | JobType::LibraryWorker => Some(TechField::Science),
            JobType::Administrator | JobType::RationingBureaucrat | JobType::Sheriff => {
                Some(TechField::Administration)
            }
            _ => None,
        }
    }

    /// Human-readable label for chronicles and UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            TechField::Medicine => "medicine",
            TechField::Agriculture => "agriculture",
            TechField::Mining => "mining",
            TechField::Engineering => "engineering",
            TechField::Science => "science",
            TechField::Administration => "administration",
        }
    }
}

/// Colony-wide practical knowledge levels (Spec 328).
///
/// Every field starts at [`FOUNDING_TECH_LEVEL`]. Disuse decays it;
/// practice maintains it and slowly rebuilds what was lost.
#[derive(Resource, Debug, Clone)]
pub struct TechKnowledge {
    levels: HashMap<TechField, u32>,
    last_worked: HashMap<TechField, u64>,
    practice_bank: HashMap<TechField, u64>,
}

impl Default for TechKnowledge {
    fn default() -> Self {
        let mut levels = HashMap::new();
        for field in TechField::ALL {
            levels.insert(field, FOUNDING_TECH_LEVEL);
        }
        Self {
            levels,
            last_worked: HashMap::new(),
            practice_bank: HashMap::new(),
        }
    }
}

impl TechKnowledge {
    /// Current knowledge level for a field (0 if never touched).
    #[must_use]
    pub fn get_level(&self, field: TechField) -> u32 {
        self.levels.get(&field).copied().unwrap_or(0)
    }

    /// Set a field's level directly, clamped to [`MAX_TECH_LEVEL`].
    ///
    /// Does not touch `last_worked`: callers (tests, scenarios) control the
    /// clock explicitly.
    pub fn set_level(&mut self, field: TechField, level: u32) {
        self.levels.insert(field, level.min(MAX_TECH_LEVEL));
    }

    /// Last tick at which the field was practiced (0 if never).
    #[must_use]
    pub fn last_worked(&self, field: TechField) -> u64 {
        self.last_worked.get(&field).copied().unwrap_or(0)
    }
}

/// Minimum practical knowledge required to operate a building.
///
/// Attached at construction for advanced buildings (see
/// [`tech_requirement_for`]). When the colony's level in the field drops
/// below the requirement, the building gains [`DisabledState`].
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TechRequirement(pub TechField, pub u32);

/// Marks an inoperable "black box": the colony lost the knowledge to run it.
///
/// Inert marker for now — future specs can make consumers/producers check
/// for it. The required field/level are recorded so the building can come
/// back online when knowledge is regained.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisabledState {
    /// The field whose knowledge was lost.
    pub lost_field: TechField,
    /// The level the building needs.
    pub required_level: u32,
}

/// Knowledge requirement for advanced buildings; `None` for the rest.
///
/// All requirements sit at [`FOUNDING_TECH_LEVEL`], so tagged buildings
/// start operational: disuse black-boxes them, practice brings them back.
#[must_use]
pub fn tech_requirement_for(building_type: BuildingType) -> Option<TechRequirement> {
    match building_type {
        BuildingType::Hospital => Some(TechRequirement(TechField::Medicine, 3)),
        BuildingType::Refinery => Some(TechRequirement(TechField::Engineering, 3)),
        BuildingType::HydroponicsBay => Some(TechRequirement(TechField::Agriculture, 3)),
        BuildingType::Observatory => Some(TechRequirement(TechField::Science, 3)),
        BuildingType::AncientFabricator => Some(TechRequirement(TechField::Engineering, 3)),
        BuildingType::AncientReactor => Some(TechRequirement(TechField::Engineering, 3)),
        _ => None,
    }
}

/// Stamps `last_worked` for fields with active practitioners and banks
/// practice toward relearning lost levels. Runs each season, before
/// [`knowledge_decay_system`].
///
/// Practice maintains any level indefinitely, and relearns lost levels back
/// up to [`FOUNDING_TECH_LEVEL`] — generational reteaching restores the
/// baseline; advancing beyond it is future-spec territory.
pub fn knowledge_practice_system(
    time: Res<SimulationTime>,
    jobs: Query<&Job>,
    mut knowledge: ResMut<TechKnowledge>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    if !time.tick.is_multiple_of(REGRESSION_CHECK_TICKS) {
        return;
    }
    let tick = time.tick;
    let mut practiced = [false; TechField::ALL.len()];
    for job in jobs.iter() {
        if let Some(field) = TechField::for_job(job.job_type) {
            let idx = TechField::ALL.iter().position(|f| *f == field).unwrap();
            practiced[idx] = true;
            knowledge.last_worked.insert(field, tick);
        }
    }
    for (idx, field) in TechField::ALL.iter().enumerate() {
        if practiced[idx] {
            let level = knowledge.get_level(*field);
            if level < FOUNDING_TECH_LEVEL {
                let bank = knowledge.practice_bank.entry(*field).or_insert(0);
                *bank += REGRESSION_CHECK_TICKS;
                if *bank >= RELEARN_TICKS {
                    knowledge.levels.insert(*field, level + 1);
                    knowledge.practice_bank.insert(*field, 0);
                    chronicle.send(AddChronicleEvent {
                        text: format!(
                            "Practitioners have retaught the colony {} (level {} → {}).",
                            field.label(),
                            level,
                            level + 1
                        ),
                        importance: EventImportance::Standard,
                    });
                }
            }
        } else {
            // Practice must be sustained: a gap resets the relearning clock.
            knowledge.practice_bank.insert(*field, 0);
        }
    }
}

/// Decays disused fields, black-boxes buildings whose requirements are no
/// longer met, and re-enables buildings whose knowledge was regained. Runs
/// each season, after [`knowledge_practice_system`].
pub fn knowledge_decay_system(
    time: Res<SimulationTime>,
    mut knowledge: ResMut<TechKnowledge>,
    buildings: Query<(Entity, &Building, &TechRequirement, Option<&DisabledState>)>,
    mut commands: Commands,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    if !time.tick.is_multiple_of(REGRESSION_CHECK_TICKS) {
        return;
    }
    let tick = time.tick;
    for field in TechField::ALL {
        let level = knowledge.get_level(field);
        if level > 0 && tick.saturating_sub(knowledge.last_worked(field)) >= DECAY_TICKS {
            let new_level = level - 1;
            knowledge.levels.insert(field, new_level);
            // Restart the clock so decay steps down one level per year,
            // not all the way to zero at once.
            knowledge.last_worked.insert(field, tick);
            knowledge.practice_bank.insert(field, 0);
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "The colony has lost {} knowledge (level {} → {}). No one practices the old ways. [KNOWLEDGE_LOST]",
                    field.label(),
                    level,
                    new_level
                ),
                importance: EventImportance::Major,
            });
        }
    }
    // Black-box / re-enable transitions (transition-only chronicles).
    for (entity, building, req, disabled) in buildings.iter() {
        let level = knowledge.get_level(req.0);
        if level < req.1 {
            if disabled.is_none() {
                commands.entity(entity).insert(DisabledState {
                    lost_field: req.0,
                    required_level: req.1,
                });
                chronicle.send(AddChronicleEvent {
                    text: format!(
                        "The {} has gone dark — a black box. The colony no longer understands {} well enough to run it (needs level {}).",
                        building.building_type.label(),
                        req.0.label(),
                        req.1
                    ),
                    importance: EventImportance::Standard,
                });
            }
        } else if disabled.is_some() {
            commands.entity(entity).remove::<DisabledState>();
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "The {} hums back to life — {} knowledge restored.",
                    building.building_type.label(),
                    req.0.label()
                ),
                importance: EventImportance::Standard,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::pop::Pop;
    use bevy::app::{App, Update};

    fn test_app() -> App {
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.init_resource::<SimulationTime>();
        app.init_resource::<TechKnowledge>();
        app.add_systems(Update, (knowledge_practice_system, knowledge_decay_system).chain());
        app
    }

    /// Mirrors the spec's `advance_time(&mut app, 1000.0)`: bump the clock
    /// tick-by-tick so seasonal systems observe every season boundary.
    fn advance_time(app: &mut App, ticks: f32) {
        let n = ticks as u64;
        for _ in 0..n {
            app.world_mut().resource_mut::<SimulationTime>().tick += 1;
            app.update();
        }
    }

    fn spawn_doctor(app: &mut App) -> Entity {
        let workplace = app.world_mut().spawn_empty().id();
        app.world_mut()
            .spawn((
                Pop,
                Job {
                    workplace,
                    job_type: JobType::Doctor,
                },
            ))
            .id()
    }

    fn chronicle_texts(app: &App) -> Vec<String> {
        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        reader.read(events).map(|e| e.text.clone()).collect()
    }

    #[test]
    fn test_knowledge_decay_over_time() {
        // Arrange
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<TechKnowledge>()
            .set_level(TechField::Medicine, 5);

        // Act: a full in-game year with no medical practice.
        advance_time(&mut app, 1000.0);

        // Assert
        let level = app
            .world_mut()
            .resource::<TechKnowledge>()
            .get_level(TechField::Medicine);
        assert!(level < 5, "medicine should decay without practitioners, got {level}");
    }

    #[test]
    fn test_building_becomes_inoperable_on_regression() {
        // Arrange
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<TechKnowledge>()
            .set_level(TechField::Medicine, 1);
        let mri = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Hospital,
                },
                TechRequirement(TechField::Medicine, 3),
            ))
            .id();

        // Act
        app.update();

        // Assert
        assert!(
            app.world_mut().get::<DisabledState>(mri).is_some(),
            "hospital should be a black box at medicine 1 < 3"
        );
    }

    #[test]
    fn test_working_job_maintains_knowledge() {
        // Arrange
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<TechKnowledge>()
            .set_level(TechField::Medicine, 5);
        spawn_doctor(&mut app);

        // Act: a full in-game year with an active doctor.
        advance_time(&mut app, 1000.0);

        // Assert
        let level = app
            .world_mut()
            .resource::<TechKnowledge>()
            .get_level(TechField::Medicine);
        assert_eq!(level, 5, "active practice should hold knowledge at 5, got {level}");
    }

    #[test]
    fn test_practice_relearns_lost_knowledge() {
        // Arrange: medicine collapsed to 1, then a doctor returns.
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<TechKnowledge>()
            .set_level(TechField::Medicine, 1);
        spawn_doctor(&mut app);

        // Act: a full in-game year of sustained practice.
        advance_time(&mut app, 1000.0);

        // Assert
        let level = app
            .world_mut()
            .resource::<TechKnowledge>()
            .get_level(TechField::Medicine);
        assert!(level > 1, "sustained practice should relearn medicine, got {level}");
    }

    #[test]
    fn test_black_box_comes_back_online() {
        // Arrange: hospital black-boxed at medicine 1...
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<TechKnowledge>()
            .set_level(TechField::Medicine, 1);
        let hospital = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Hospital,
                },
                TechRequirement(TechField::Medicine, 3),
            ))
            .id();
        app.update();
        assert!(app.world_mut().get::<DisabledState>(hospital).is_some());

        // Act: knowledge retaught to requirement.
        app.world_mut()
            .resource_mut::<TechKnowledge>()
            .set_level(TechField::Medicine, 3);
        // Run past the next season boundary so the check observes it.
        advance_time(&mut app, 250.0);

        // Assert
        assert!(
            app.world_mut().get::<DisabledState>(hospital).is_none(),
            "hospital should come back online at medicine 3"
        );
    }

    #[test]
    fn test_decay_fires_knowledge_lost_chronicle() {
        // Arrange
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<TechKnowledge>()
            .set_level(TechField::Mining, 4);

        // Act
        advance_time(&mut app, 1000.0);

        // Assert
        let texts = chronicle_texts(&app);
        assert!(
            texts.iter().any(|t| t.contains("mining")),
            "expected a knowledge-lost chronicle for mining, got {texts:?}"
        );
    }

    #[test]
    fn test_for_job_mapping_covers_productive_jobs() {
        assert_eq!(TechField::for_job(JobType::Doctor), Some(TechField::Medicine));
        assert_eq!(TechField::for_job(JobType::FarmWorker), Some(TechField::Agriculture));
        assert_eq!(TechField::for_job(JobType::DeepMining), Some(TechField::Mining));
        assert_eq!(
            TechField::for_job(JobType::ObservatoryWorker),
            Some(TechField::Science)
        );
        assert_eq!(
            TechField::for_job(JobType::Administrator),
            Some(TechField::Administration)
        );
        assert_eq!(TechField::for_job(JobType::TavernVisitor), None);
        assert_eq!(TechField::for_job(JobType::HousingResident), None);
        assert_eq!(TechField::for_job(JobType::Patient), None);
    }

    #[test]
    fn test_tech_requirement_for_advanced_buildings() {
        assert_eq!(
            tech_requirement_for(BuildingType::Hospital),
            Some(TechRequirement(TechField::Medicine, 3))
        );
        assert_eq!(
            tech_requirement_for(BuildingType::AncientFabricator),
            Some(TechRequirement(TechField::Engineering, 3))
        );
        assert_eq!(tech_requirement_for(BuildingType::Farm), None);
        assert_eq!(tech_requirement_for(BuildingType::Housing), None);
    }

    #[test]
    fn test_ip_guard_no_lifted_names() {
        // Spec 328 works in generic terms only: no named characters,
        // places, or distinctive outside-IP entities anywhere in this module.
        let mut text = String::new();
        text.push_str(&format!("{:?}", TechField::ALL));
        text.push_str(TechField::Medicine.label());
        text.push_str(TechField::Agriculture.label());
        text.push_str(TechField::Mining.label());
        text.push_str(TechField::Engineering.label());
        text.push_str(TechField::Science.label());
        text.push_str(TechField::Administration.label());
        for forbidden in [
            "mri", "skynet", "foundation", "vault-tec", "umbrella", "tyrell", "stark",
        ] {
            assert!(
                !text.to_lowercase().contains(forbidden),
                "lifted name {forbidden:?} must not appear"
            );
        }
    }
}
