//! Spec 426: The Archive of Regret.
//!
//! A massive, indestructible building that records every moral
//! transgression the colony commits. Starving a pop, executing a
//! prisoner, cannibalism — each [`AtrocityEvent`] is inscribed into the
//! Archive forever. The colony learns from its sins: the Archive
//! generates Knowledge (research) from what it holds. But the record
//! weighs on everyone: a permanent "Guilt" aura depresses colony morale
//! for as long as the Archive stands — and it always stands.
//!
//! Grounding (audited against the real seams, per the 360–425 pattern):
//! - The spec's `src/layer1/building.rs` / `chronicle.rs` / `morale.rs` /
//!   `research.rs` paths are sketches. The real homes are
//!   `layer1::architecture::building`, `layer1::core::chronicle`,
//!   `layer1::social::morale`, and research-as-currency in
//!   `layer1::economy::resources::ColonyResources::knowledge` (there is
//!   no `ResearchPoints` resource; Knowledge is spent through
//!   `TechState`). Research generation therefore flows into
//!   `ColonyResources` via `add_knowledge` (which clamps to capacity).
//! - The spec's `AtrocityEvent { description, severity }` in a chronicle
//!   module does not exist. The real event is
//!   `layer1::social::slippery_slope::AtrocityEvent { severity }`,
//!   already produced/consumed by the desensitization systems. The
//!   Archive listens to that same event with its own `EventReader`
//!   (spec §5: multiple systems may listen; never drain the buffer).
//!   Chronicle entries are written through the real
//!   `AddChronicleEvent` bus with generated text (the event carries no
//!   description string to copy).
//! - The spec's `Building { name: String, .. }` does not exist; the real
//!   component is `Building { building_type: BuildingType }`.
//!   `BuildingType::Archive` is already taken by Spec 414's Black Box,
//!   so this building is `BuildingType::ArchiveOfRegret`, carrying the
//!   [`ArchiveOfRegret`] component plus the [`Indestructible`] marker.
//! - Morale (spec §5/§7 refactor): the sketch's per-tick destructive
//!   subtraction from a mock `MoodModifier { current, label }` is NOT
//!   implemented. Instead, following the sovereign / governor /
//!   propaganda pattern, [`apply_guilt_aura_system`] refreshes a single
//!   persistent [`MoodModifier`] ("Guilty Conscience", duration 2) on
//!   every pop's real [`Morale`], and `update_morale_cache_system`
//!   folds it into `Morale.value`. The summed aura is published as the
//!   [`GlobalAura`] resource (spec §7's GlobalAura pattern) so other
//!   systems and STATS read the aura without touching pops.
//! - Scaling limits (spec §5 refactor): raw `aura_strength` accumulates
//!   linearly (the spec RED test's 1 severity = 1.0 aura holds) but is
//!   capped at [`MAX_AURA_STRENGTH`], and both effects derive from it
//!   through a logarithmic curve with hard caps — research tops out at
//!   [`RESEARCH_CAP_PER_TICK`] Knowledge/tick and guilt at
//!   [`GUILT_CAP`] morale, so a bloody colony neither maxes the tech
//!   tree instantly nor collapses on the first atrocity.
//! - Indestructible (spec §7): `BuildingType::is_indestructible()`
//!   flags the Archive; `can_designate` refuses Demolish/Destroy
//!   designations on it and the demolish/destroy executors refuse to
//!   despawn it. (Explosion *damage* to its Structure is out of scope —
//!   the spec only asks for the un-demolishable flag.)

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::entities::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::slippery_slope::AtrocityEvent;

/// Hard cap on stored aura: the Archive's record can only weigh so
/// much, however bloody the colony's history gets.
pub const MAX_AURA_STRENGTH: f32 = 500.0;
/// Logarithmic factor for the guilt curve (morale per ln(1 + aura)).
pub const GUILT_LOG_FACTOR: f32 = 0.08;
/// The guilt aura never depresses morale by more than this.
pub const GUILT_CAP: f32 = 0.40;
/// Logarithmic factor for the research curve (Knowledge per ln(1 + aura)).
pub const RESEARCH_LOG_FACTOR: f32 = 1.0;
/// The Archive never generates more Knowledge than this per tick.
pub const RESEARCH_CAP_PER_TICK: f32 = 5.0;
/// Label of the persistent morale modifier the guilt aura maintains.
pub const GUILT_MODIFIER_LABEL: &str = "Guilty Conscience";

/// State carried by an Archive of Regret building (Spec 426).
///
/// Attached by building configuration when a
/// `BuildingType::ArchiveOfRegret` is constructed.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq)]
pub struct ArchiveOfRegret {
    /// How many atrocities the Archive has recorded. Never decreases.
    pub stored_atrocities: u32,
    /// Summed severity of everything recorded, capped at
    /// [`MAX_AURA_STRENGTH`]. Drives research and the guilt aura.
    pub aura_strength: f32,
}

/// Marker: this building cannot be demolished or destroyed.
///
/// Inserted alongside [`ArchiveOfRegret`] by building configuration.
/// The authoritative check is `BuildingType::is_indestructible()` (the
/// designation and execution layers consult the building type); this
/// marker exists so queries can find indestructible structures
/// directly.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Indestructible;

/// The colony-wide guilt aura (spec §7's GlobalAura pattern).
///
/// Refreshed every tick by [`sync_guilt_aura_system`] from the standing
/// Archives. Other systems read the aura from here instead of scanning
/// buildings or mutating pops themselves.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct GlobalAura {
    /// Total raw aura strength summed over all standing Archives.
    pub guilt: f32,
    /// Total atrocities recorded across all standing Archives.
    pub atrocities: u32,
}

impl GlobalAura {
    /// The morale penalty this aura currently applies (0.0..=[`GUILT_CAP`]).
    #[must_use]
    pub fn guilt_penalty(&self) -> f32 {
        guilt_penalty(self.guilt)
    }
}

/// Effective aura after the storage cap.
#[must_use]
pub fn effective_aura(raw_aura: f32) -> f32 {
    raw_aura.clamp(0.0, MAX_AURA_STRENGTH)
}

/// Morale penalty for a given raw aura: logarithmic, hard-capped so the
/// colony is severely depressed by a full Archive but never instantly
/// collapses (spec §5 scaling limits).
#[must_use]
pub fn guilt_penalty(raw_aura: f32) -> f32 {
    (GUILT_LOG_FACTOR * (1.0 + effective_aura(raw_aura)).ln()).min(GUILT_CAP)
}

/// Knowledge generated per tick for a given raw aura: logarithmic,
/// hard-capped so the Archive is a powerful teacher but cannot max out
/// the tech tree on its own (spec §5 scaling limits).
#[must_use]
pub fn research_gain(raw_aura: f32) -> f32 {
    (RESEARCH_LOG_FACTOR * (1.0 + effective_aura(raw_aura)).ln()).min(RESEARCH_CAP_PER_TICK)
}

/// Records every [`AtrocityEvent`] into every standing Archive.
///
/// Uses an `EventReader` (spec §5): the desensitization systems read
/// the same event stream independently. Each recorded atrocity also
/// writes a chronicle entry through the real `AddChronicleEvent` bus —
/// the Archive remembers, and so does the colony's history.
pub fn update_archive_of_regret_system(
    mut events: EventReader<AtrocityEvent>,
    mut archives: Query<&mut ArchiveOfRegret>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        let mut recorded = false;
        for mut archive in archives.iter_mut() {
            archive.stored_atrocities = archive.stored_atrocities.saturating_add(1);
            archive.aura_strength =
                (archive.aura_strength + event.severity.max(0.0)).min(MAX_AURA_STRENGTH);
            recorded = true;
        }
        if recorded {
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "The Archive of Regret records an atrocity (severity {:.0}). The colony remembers.",
                    event.severity
                ),
                importance: if event.severity >= 10.0 {
                    EventImportance::Major
                } else {
                    EventImportance::Standard
                },
            });
        }
    }
}

/// Generates Knowledge from the Archive's contents: the colony learns
/// from its recorded mistakes. Gain scales with total aura through the
/// capped logarithmic curve ([`research_gain`]) and flows through
/// `ColonyResources::add_knowledge`, so storage capacity still gates
/// how much the colony can hold.
pub fn generate_archive_research_system(
    archives: Query<&ArchiveOfRegret>,
    mut resources: ResMut<ColonyResources>,
) {
    let total_aura: f32 = archives.iter().map(|a| a.aura_strength).sum();
    let gain = research_gain(total_aura);
    if gain > 0.0 {
        resources.add_knowledge(gain);
    }
}

/// Publishes the summed Archive aura into the [`GlobalAura`] resource.
pub fn sync_guilt_aura_system(
    archives: Query<&ArchiveOfRegret>,
    mut aura: ResMut<GlobalAura>,
) {
    aura.guilt = archives.iter().map(|a| a.aura_strength).sum();
    aura.atrocities = archives.iter().map(|a| a.stored_atrocities).sum();
}

/// Applies the guilt aura to colony morale — non-destructively.
///
/// Refreshes exactly one persistent "Guilty Conscience"
/// [`MoodModifier`] per pop (the sovereign/governor pattern): the old
/// guilt modifier is dropped and re-added at the current strength with
/// a 2-tick duration, so `update_morale_cache_system` keeps folding it
/// into `Morale.value` while the Archive stands, and it decays away
/// naturally if the aura ever reaches zero. `Morale.value` itself is
/// never written here (spec §5: no destructive per-tick subtraction).
pub fn apply_guilt_aura_system(aura: Res<GlobalAura>, mut pops: Query<&mut Morale, With<Pop>>) {
    let penalty = aura.guilt_penalty();
    for mut morale in pops.iter_mut() {
        morale.modifiers.retain(|m| m.label != GUILT_MODIFIER_LABEL);
        if penalty > 0.0 {
            morale.add_modifier(MoodModifier {
                label: GUILT_MODIFIER_LABEL.to_string(),
                value: -penalty,
                duration: 2,
            });
        }
    }
}

/// Live census for STATS / headless: (archives standing, atrocities
/// recorded, total raw aura).
#[must_use]
pub fn archive_of_regret_totals(world: &mut World) -> (usize, u32, f32) {
    let mut query = world.query::<&ArchiveOfRegret>();
    let mut count = 0usize;
    let mut atrocities = 0u32;
    let mut aura = 0.0f32;
    for archive in query.iter(world) {
        count += 1;
        atrocities += archive.stored_atrocities;
        aura += archive.aura_strength;
    }
    (count, atrocities, aura)
}

/// STATS label: `{archives}a/{atrocities}t/{aura}u` (no spaces —
/// STATS is space-parsed). Full panel: headless `regrets`.
#[must_use]
pub fn archive_of_regret_stats_label(world: &mut World) -> String {
    let (count, atrocities, aura) = archive_of_regret_totals(world);
    format!("{count}a/{atrocities}t/{aura:.0}u")
}

/// Player-facing strings for this mechanic (IP-guard tested): original,
/// generic wording only.
#[must_use]
pub fn archive_of_regret_public_strings() -> Vec<&'static str> {
    vec![
        "Archive of Regret",
        "Guilty Conscience",
        "The Archive of Regret records an atrocity. The colony remembers.",
        "The Archive of Regret cannot be demolished.",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::architecture::building::{Building, BuildingType};

    fn setup_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<AtrocityEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(ColonyResources::default());
        world.insert_resource(GlobalAura::default());
        world.spawn((
            Building {
                building_type: BuildingType::ArchiveOfRegret,
            },
            ArchiveOfRegret::default(),
            Indestructible,
        ));
        world
    }

    fn run(world: &mut World, systems: &mut Schedule) {
        systems.run(world);
    }

    // Spec RED test 1, adapted: AtrocityEvent is the real
    // slippery_slope event ({ severity } only, no description).
    #[test]
    fn test_archive_records_atrocity() {
        let mut world = setup_world();
        let mut schedule = Schedule::default();
        schedule.add_systems(update_archive_of_regret_system);

        world
            .resource_mut::<Events<AtrocityEvent>>()
            .send(AtrocityEvent { severity: 5.0 });
        run(&mut world, &mut schedule);

        let mut query = world.query::<&ArchiveOfRegret>();
        let archive = query.single(&world);
        assert_eq!(archive.stored_atrocities, 1);
        assert!((archive.aura_strength - 5.0).abs() < f32::EPSILON);

        // The recording is also written to the chronicle bus.
        let mut events = world.resource_mut::<Events<AddChronicleEvent>>();
        let drained: Vec<_> = events.drain().collect();
        assert_eq!(drained.len(), 1);
        assert!(drained[0].text.contains("Archive of Regret"));
    }

    #[test]
    fn test_archive_aura_is_capped() {
        let mut world = setup_world();
        let mut schedule = Schedule::default();
        schedule.add_systems(update_archive_of_regret_system);

        for _ in 0..2 {
            world
                .resource_mut::<Events<AtrocityEvent>>()
                .send(AtrocityEvent { severity: 300.0 });
        }
        run(&mut world, &mut schedule);

        let mut query = world.query::<&ArchiveOfRegret>();
        let archive = query.single(&world);
        assert_eq!(archive.stored_atrocities, 2);
        assert!((archive.aura_strength - MAX_AURA_STRENGTH).abs() < f32::EPSILON);
    }

    #[test]
    fn test_atrocity_without_archive_records_nothing() {
        let mut world = World::new();
        world.init_resource::<Events<AtrocityEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(GlobalAura::default());
        let mut schedule = Schedule::default();
        schedule.add_systems((update_archive_of_regret_system, sync_guilt_aura_system));

        world
            .resource_mut::<Events<AtrocityEvent>>()
            .send(AtrocityEvent { severity: 5.0 });
        run(&mut world, &mut schedule);

        assert_eq!(world.resource::<GlobalAura>().atrocities, 0);
        assert_eq!(world.resource::<GlobalAura>().guilt, 0.0);
        let mut events = world.resource_mut::<Events<AddChronicleEvent>>();
        assert_eq!(events.drain().count(), 0);
    }

    // Spec RED test 2, adapted: research is Knowledge in ColonyResources
    // (no ResearchPoints resource exists), and the refactor-phase curve
    // makes the gain sublinear — aura 50 must NOT pay out 50 at once.
    #[test]
    fn test_archive_generates_research() {
        let mut world = setup_world();
        {
            let mut query = world.query::<&mut ArchiveOfRegret>();
            for mut archive in query.iter_mut(&mut world) {
                archive.stored_atrocities = 10;
                archive.aura_strength = 50.0;
            }
        }
        let mut schedule = Schedule::default();
        schedule.add_systems(generate_archive_research_system);
        run(&mut world, &mut schedule);

        let knowledge = world.resource::<ColonyResources>().knowledge;
        assert!(
            (knowledge - research_gain(50.0)).abs() < 0.001,
            "knowledge {knowledge} should equal the curve gain for aura 50"
        );
        assert!(knowledge > 0.0 && knowledge < 50.0, "gain must be sublinear");
    }

    #[test]
    fn test_archive_research_is_capped() {
        let mut world = setup_world();
        {
            let mut query = world.query::<&mut ArchiveOfRegret>();
            for mut archive in query.iter_mut(&mut world) {
                archive.aura_strength = MAX_AURA_STRENGTH;
            }
        }
        let mut schedule = Schedule::default();
        schedule.add_systems(generate_archive_research_system);
        run(&mut world, &mut schedule);

        let knowledge = world.resource::<ColonyResources>().knowledge;
        assert!((knowledge - RESEARCH_CAP_PER_TICK).abs() < 0.001);
    }

    // Spec RED test 3, adapted: the real morale system is Morale +
    // MoodModifier list (no MoodModifier { current, label } component).
    // The guilt aura must appear as ONE persistent modifier, and the
    // cached Morale.value must NOT be destructively subtracted.
    #[test]
    fn test_archive_applies_guilt_aura() {
        let mut world = setup_world();
        let pop = world.spawn((Pop, Morale::default())).id();
        world.insert_resource(GlobalAura {
            guilt: 20.0,
            atrocities: 1,
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(apply_guilt_aura_system);
        run(&mut world, &mut schedule);

        let morale = world.get::<Morale>(pop).unwrap();
        let guilt: Vec<_> = morale
            .modifiers
            .iter()
            .filter(|m| m.label == GUILT_MODIFIER_LABEL)
            .collect();
        assert_eq!(guilt.len(), 1, "exactly one persistent guilt modifier");
        assert!(
            (guilt[0].value + guilt_penalty(20.0)).abs() < 0.001,
            "modifier should be -guilt_penalty(20)"
        );
        assert!(
            (morale.value - Morale::default().value).abs() < f32::EPSILON,
            "apply_guilt_aura_system must not destructively subtract Morale.value"
        );

        // Refreshing the aura never stacks a second modifier.
        run(&mut world, &mut schedule);
        let morale = world.get::<Morale>(pop).unwrap();
        assert_eq!(
            morale
                .modifiers
                .iter()
                .filter(|m| m.label == GUILT_MODIFIER_LABEL)
                .count(),
            1
        );
    }

    #[test]
    fn test_guilt_aura_syncs_from_archives() {
        let mut world = setup_world();
        {
            let mut query = world.query::<&mut ArchiveOfRegret>();
            for mut archive in query.iter_mut(&mut world) {
                archive.stored_atrocities = 3;
                archive.aura_strength = 42.0;
            }
        }
        let mut schedule = Schedule::default();
        schedule.add_systems(sync_guilt_aura_system);
        run(&mut world, &mut schedule);

        let aura = world.resource::<GlobalAura>();
        assert_eq!(aura.atrocities, 3);
        assert!((aura.guilt - 42.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_guilt_penalty_is_capped() {
        assert!((guilt_penalty(MAX_AURA_STRENGTH) - GUILT_CAP).abs() < f32::EPSILON);
        assert!((guilt_penalty(1_000_000.0) - GUILT_CAP).abs() < f32::EPSILON);
        assert_eq!(guilt_penalty(0.0), 0.0);
    }

    #[test]
    fn test_archive_building_is_indestructible() {
        assert!(BuildingType::ArchiveOfRegret.is_indestructible());
        assert!(!BuildingType::Housing.is_indestructible());
        assert_eq!(BuildingType::ArchiveOfRegret.label(), "Archive of Regret");
    }

    #[test]
    fn test_spawned_archive_carries_regret_component() {
        use crate::layer1::architecture::building::{spawn_building, MaterialType};
        let mut world = World::new();
        let entity = spawn_building(
            &mut world,
            5,
            5,
            BuildingType::ArchiveOfRegret,
            MaterialType::Stone,
        );
        assert!(world.get::<ArchiveOfRegret>(entity).is_some());
        assert!(world.get::<Indestructible>(entity).is_some());
    }

    #[test]
    fn test_stats_label() {
        let mut world = setup_world();
        {
            let mut query = world.query::<&mut ArchiveOfRegret>();
            for mut archive in query.iter_mut(&mut world) {
                archive.stored_atrocities = 2;
                archive.aura_strength = 30.0;
            }
        }
        assert_eq!(archive_of_regret_stats_label(&mut world), "1a/2t/30u");
    }

    #[test]
    fn test_ip_guard_no_distinctive_ip() {
        const BANNED: &[&str] = &[
            "Sad King Billy",
            "Windsor",
            "Atreides",
            "Kwisatz",
            "Hitchhiker",
            "Ender",
        ];
        for text in archive_of_regret_public_strings() {
            for banned in BANNED {
                assert!(
                    !text.contains(banned),
                    "public string {text:?} contains banned IP term {banned:?}"
                );
            }
        }
    }
}
