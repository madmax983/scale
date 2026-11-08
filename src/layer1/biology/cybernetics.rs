use crate::layer1::actions::{AssignedTo, AssignmentType};
use crate::layer1::map::GridPosition;
use bevy_ecs::prelude::*;

/// Types of prosthetic augmentations available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Component)]
pub enum ProstheticType {
    /// Replaces an arm.
    BionicArm,
    /// Replaces a leg.
    BionicLeg,
    /// Replaces an eye.
    BionicEye,
    /// Replaces skin.
    SyntheticSkin,
    /// Connects brain to machines.
    NeuralInterface,
}

/// Component representing a prosthetic item.
#[derive(Component, Debug, Clone)]
pub struct Prosthetic {
    /// The type of prosthetic.
    pub prosthetic_type: ProstheticType,
    /// Multiplier for work speed (e.g. 0.2 = +20%).
    pub efficiency_bonus: f32,
    /// Malus for social interactions (e.g. 0.1 = -10% relationship gain).
    pub social_penalty: f32,
    /// Power consumed per tick (future use).
    pub power_consumption: f32,
}

/// Component tracking installed augmentations on a Pop.
#[derive(Component, Debug, Default, Clone)]
pub struct Augmentations {
    /// List of installed prosthetic entities.
    pub installed: Vec<Entity>,
}

/// Component representing a missing limb (e.g. from accident).
#[derive(Component, Debug, Clone)]
pub struct MissingLimb {
    /// Severity of the loss (0.0 to 1.0).
    /// Typically 0.5 for a single limb.
    pub severity: f32,
}

/// Component indicating a surgery is in progress.
#[derive(Component, Debug, Clone)]
pub struct PendingSurgery {
    /// The prosthetic item being installed.
    pub item: Entity,
    /// Current progress of the surgery (ticks).
    pub progress: f32,
    /// Total duration required for the surgery (ticks).
    pub duration: f32,
}

/// System to advance surgery progress and install prosthetics upon completion.
pub fn surgery_system(world: &mut World) {
    let mut completed_surgeries = Vec::new();

    // Query for pops undergoing surgery
    let mut query = world.query::<(Entity, &mut PendingSurgery, &AssignedTo)>();

    // Check assignments
    for (entity, mut surgery, assignment) in query.iter_mut(world) {
        if assignment.assignment_type == AssignmentType::Surgery {
            // Advance progress (1.0 per tick)
            surgery.progress += 1.0;

            if surgery.progress >= surgery.duration {
                completed_surgeries.push((entity, surgery.item));
            }
        }
    }

    // Apply results
    for (pop_entity, item_entity) in completed_surgeries {
        // Move item to installed list
        if let Some(mut augs) = world.get_mut::<Augmentations>(pop_entity) {
            augs.installed.push(item_entity);
        }

        // Check if we fixed a missing limb
        if let Some(prosthetic) = world.get::<Prosthetic>(item_entity) {
            if matches!(
                prosthetic.prosthetic_type,
                ProstheticType::BionicArm | ProstheticType::BionicLeg
            ) {
                world.entity_mut(pop_entity).remove::<MissingLimb>();
            }
        }

        // Remove PendingSurgery component
        world.entity_mut(pop_entity).remove::<PendingSurgery>();

        // Remove AssignedTo so pop becomes idle (or handles post-surgery logic)
        world.entity_mut(pop_entity).remove::<AssignedTo>();

        // Cleanup item entity (remove from map if present)
        // We keep the entity alive as it is now "owned" by Augmentations,
        // but it shouldn't be on the grid.
        world.entity_mut(item_entity).remove::<GridPosition>();
    }
}

/// Calculates the total work efficiency bonus from all installed augmentations.
pub fn get_efficiency_bonus(world: &World, pop: Entity) -> f32 {
    let mut total = 0.0;
    if let Some(augs) = world.get::<Augmentations>(pop) {
        for &item in &augs.installed {
            if let Some(prosthetic) = world.get::<Prosthetic>(item) {
                total += prosthetic.efficiency_bonus;
            }
        }
    }

    if let Some(missing) = world.get::<MissingLimb>(pop) {
        total -= missing.severity;
    }

    total
}

/// Calculates the total social penalty from all installed augmentations.
pub fn get_social_penalty(world: &World, pop: Entity) -> f32 {
    let mut total = 0.0;
    if let Some(augs) = world.get::<Augmentations>(pop) {
        for &item in &augs.installed {
            if let Some(prosthetic) = world.get::<Prosthetic>(item) {
                total += prosthetic.social_penalty;
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::actions::{AssignedTo, AssignmentType};
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::items::Item;
    use crate::layer1::map::GridPosition;
    use crate::layer1::medical::Hospital;
    use crate::layer1::pop::{Pop, Speed};

    #[test]
    fn test_prosthetic_component_defaults() {
        let prosthetic = Prosthetic {
            prosthetic_type: ProstheticType::BionicArm,
            efficiency_bonus: 0.2,
            social_penalty: 0.1,
            power_consumption: 1.0,
        };
        assert_eq!(prosthetic.prosthetic_type, ProstheticType::BionicArm);
    }

    #[test]
    fn test_augmentations_component_exists() {
        let mut world = World::new();
        let entity = world.spawn((Pop, Augmentations::default())).id();
        let augs = world
            .get::<Augmentations>(entity)
            .expect("Missing resource or component");
        assert!(augs.installed.is_empty());
    }

    #[test]
    fn test_surgery_installs_prosthetic() {
        let mut world = World::new();

        // Spawn Hospital
        let hospital = world
            .spawn((
                Building {
                    building_type: BuildingType::Hospital,
                },
                Hospital::default(),
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        // Spawn Prosthetic Item
        let bionic_arm = world
            .spawn((
                Item::default(),
                Prosthetic {
                    prosthetic_type: ProstheticType::BionicArm,
                    efficiency_bonus: 0.5,
                    social_penalty: 0.1,
                    power_consumption: 0.0,
                },
            ))
            .id();

        let pop = world
            .spawn((
                Pop,
                Augmentations::default(),
                AssignedTo {
                    assignment_type: AssignmentType::Surgery,
                    entity: hospital,
                },
                PendingSurgery {
                    item: bionic_arm,
                    progress: 0.0,
                    duration: 10.0,
                },
            ))
            .id();

        // Run surgery system multiple times to complete progress
        for _ in 0..11 {
            surgery_system(&mut world);
        }

        // Verify installation
        let augs = world
            .get::<Augmentations>(pop)
            .expect("Missing resource or component");
        assert!(
            augs.installed.contains(&bionic_arm),
            "Prosthetic should be installed"
        );

        // Verify PendingSurgery component is removed
        assert!(
            world.get::<PendingSurgery>(pop).is_none(),
            "PendingSurgery should be removed"
        );
    }

    #[test]
    fn test_augmentation_affects_stats() {
        let mut world = World::new();

        let bionic_arm = world
            .spawn(Prosthetic {
                prosthetic_type: ProstheticType::BionicArm,
                efficiency_bonus: 0.5,
                social_penalty: 0.1,
                power_consumption: 0.0,
            })
            .id();

        let pop = world
            .spawn((
                Pop,
                Augmentations {
                    installed: vec![bionic_arm],
                },
                Speed::default(),
            ))
            .id();

        // Helper function to get total bonus
        let bonus = get_efficiency_bonus(&world, pop);
        assert!(
            (bonus - 0.5).abs() < f32::EPSILON,
            "Efficiency bonus should be 0.5"
        );
    }

    #[test]
    fn test_augmentation_social_penalty() {
        let mut world = World::new();
        let bionic_face = world
            .spawn(Prosthetic {
                prosthetic_type: ProstheticType::SyntheticSkin,
                efficiency_bonus: 0.0,
                social_penalty: 0.2,
                power_consumption: 0.0,
            })
            .id();

        let pop = world
            .spawn((
                Pop,
                Augmentations {
                    installed: vec![bionic_face],
                },
            ))
            .id();

        let penalty = get_social_penalty(&world, pop);
        assert!(
            (penalty - 0.2).abs() < f32::EPSILON,
            "Social penalty should be 0.2"
        );
    }
}

// ============================================================================
// Spec 405: Cybernetic Augmentation (implementation)
// ============================================================================
/// Work bonus per MotorServo (spec 405 §3: base 1.0 -> 1.5).
pub const MOTOR_SERVO_WORK_BONUS: f32 = 0.5;
/// Social penalty per NeuralLink (spec 405 §3: base 1.0 -> 0.7).
pub const NEURAL_LINK_SOCIAL_PENALTY: f32 = 0.3;
/// Physical damage resistance per SubdermalArmor plate.
pub const SUBDERMAL_ARMOR_RATING: f32 = 0.25;
/// Charge drained per implant per tick (spec 405 §4).
pub const MAINTENANCE_DRAIN_PER_IMPLANT: f32 = 0.1;
/// Default battery size for a fresh implant loadout.
pub const DEFAULT_MAX_CHARGE: f32 = 100.0;
/// Implant count at which the Ship of Theseus alienation begins.
pub const THESEUS_IMPLANT_THRESHOLD: usize = 3;
/// Morale value of the Ship of Theseus alienation modifier.
pub const THESEUS_MORALE_VALUE: f32 = -0.15;
/// Mood label for Ship of Theseus alienation (original name, spec 405).
pub const THESEUS_MORALE_LABEL: &str = "Ship of Theseus";
/// Mood label applied while an augmented pop's battery is dead.
pub const DEAD_IMPLANTS_MORALE_LABEL: &str = "Dead Implants";
/// Morale value of the dead-battery modifier.
pub const DEAD_IMPLANTS_MORALE_VALUE: f32 = -0.10;
/// Ticks a synced cybernetic mood modifier lasts before this system
/// refreshes it (the penal-labor sync pattern: refresh every tick,
/// expire almost immediately once the condition ends).
pub const CYBER_MOOD_REFRESH_TICKS: u32 = 2;

/// A craftable mechanical prosthetic from spec 405, mapped onto the
/// pre-existing [`ProstheticType`] seam rather than duplicating it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImplantType {
    /// Servo motors in the limbs: +0.5 work bonus.
    MotorServo,
    /// Cranial link: +social penalty 0.3 (affinity gains scaled 0.7x
    /// through the real cybernetic-prejudice seam).
    NeuralLink,
    /// Plating under the skin: +0.25 physical damage resistance.
    SubdermalArmor,
}

impl ImplantType {
    /// The pre-existing prosthetic kind this implant installs as.
    #[must_use]
    pub fn prosthetic_type(self) -> ProstheticType {
        match self {
            Self::MotorServo => ProstheticType::BionicArm,
            Self::NeuralLink => ProstheticType::NeuralInterface,
            Self::SubdermalArmor => ProstheticType::SyntheticSkin,
        }
    }

    /// Work-efficiency bonus while powered (feeds `get_efficiency_bonus`
    /// through the registered `Prosthetic` entity, and the derived
    /// [`CyberneticModifiers`] ledger).
    #[must_use]
    pub fn work_bonus(self) -> f32 {
        match self {
            Self::MotorServo => MOTOR_SERVO_WORK_BONUS,
            Self::NeuralLink | Self::SubdermalArmor => 0.0,
        }
    }

    /// Social penalty (affinity-gain scaling) — applies powered or not:
    /// alienation is how the pop looks, not whether the gear runs.
    #[must_use]
    pub fn social_penalty(self) -> f32 {
        match self {
            Self::NeuralLink => NEURAL_LINK_SOCIAL_PENALTY,
            Self::MotorServo | Self::SubdermalArmor => 0.0,
        }
    }

    /// Physical damage resistance while powered.
    #[must_use]
    pub fn armor_rating(self) -> f32 {
        match self {
            Self::SubdermalArmor => SUBDERMAL_ARMOR_RATING,
            Self::MotorServo | Self::NeuralLink => 0.0,
        }
    }

    /// Display / debug-command name (original terminology only).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::MotorServo => "Motor Servo",
            Self::NeuralLink => "Neural Link",
            Self::SubdermalArmor => "Subdermal Armor",
        }
    }

    /// Parse a headless/debug token (`motor`, `neural`, `armor`, or the
    /// full type name, case-insensitive).
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.to_lowercase().as_str() {
            "motor" | "servo" | "motorservo" | "motor_servo" => Some(Self::MotorServo),
            "neural" | "link" | "neurallink" | "neural_link" => Some(Self::NeuralLink),
            "armor" | "armour" | "plate" | "subdermalarmor" | "subdermal_armor" => {
                Some(Self::SubdermalArmor)
            }
            _ => None,
        }
    }
}

/// Spec 405: the installed implant loadout on a pop. The authoritative
/// per-implant stat rows live on the registered [`Prosthetic`] entities
/// in [`Augmentations`] (installed by [`install_implant`]); this list is
/// the loadout ledger the maintenance and modifier systems iterate.
#[derive(Component, Debug, Default, Clone)]
pub struct CyberneticImplants {
    /// Installed implants, in installation order (duplicates stack).
    pub implants: Vec<ImplantType>,
}

/// Spec 405: battery charge backing a pop's implants. Drains
/// [`MAINTENANCE_DRAIN_PER_IMPLANT`] per implant per tick; at zero the
/// powered bonuses (work, armor) switch off until recharged.
#[derive(Component, Debug, Clone, Copy)]
pub struct CyberneticMaintenance {
    /// Remaining charge.
    pub current_charge: f32,
    /// Battery capacity.
    pub max_charge: f32,
}

impl Default for CyberneticMaintenance {
    fn default() -> Self {
        Self {
            current_charge: DEFAULT_MAX_CHARGE,
            max_charge: DEFAULT_MAX_CHARGE,
        }
    }
}

impl CyberneticMaintenance {
    /// True when the battery is empty and powered bonuses are off.
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.current_charge <= 0.0
    }

    /// Charge fraction 0.0..=1.0 (0.0 for a zero-capacity battery).
    #[must_use]
    pub fn fraction(&self) -> f32 {
        if self.max_charge <= 0.0 {
            0.0
        } else {
            (self.current_charge / self.max_charge).clamp(0.0, 1.0)
        }
    }

    /// Add charge, clamped to capacity. Returns the amount added.
    pub fn recharge(&mut self, amount: f32) -> f32 {
        if amount <= 0.0 || amount.is_nan() {
            return 0.0;
        }
        let before = self.current_charge;
        self.current_charge = (self.current_charge + amount).min(self.max_charge);
        self.current_charge - before
    }
}

/// Derived spec 405 modifier ledger, recomputed idempotently every tick
/// by [`apply_cybernetic_modifiers_system`]. This is the spec §5
/// REFACTOR answer to its own §4 sketch: base stats are never mutated
/// (and never reset to a hardcoded 1.0) — modifiers live here and in
/// the real consumed seams, tracked separately from base skills.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct CyberneticModifiers {
    /// Additive work-efficiency bonus (powered only).
    pub work_bonus: f32,
    /// Social penalty 0.0.. (affinity gains scale by `1.0 - penalty`).
    pub social_penalty: f32,
    /// Physical armor rating currently granted (powered only).
    pub armor: f32,
    /// Whether the battery has charge.
    pub powered: bool,
    /// Whether the Ship of Theseus threshold is met.
    pub theseus_active: bool,
    /// Armor this system has added to `DamageResistance` so far, so
    /// syncing adjusts by delta and never clobbers other systems.
    pub armor_applied: f32,
}

/// Pure derivation of (work_bonus, social_penalty, armor) for a loadout.
/// Work and armor require power; the social penalty does not.
#[must_use]
pub fn implant_modifiers(implants: &[ImplantType], powered: bool) -> (f32, f32, f32) {
    let mut work = 0.0;
    let mut social = 0.0;
    let mut armor = 0.0;
    for implant in implants {
        social += implant.social_penalty();
        if powered {
            work += implant.work_bonus();
            armor += implant.armor_rating();
        }
    }
    (work, social, armor)
}

/// One tick of maintenance drain for a loadout (pure, testable).
pub fn cybernetic_maintenance_tick(
    implants: &CyberneticImplants,
    maint: &mut CyberneticMaintenance,
) {
    #[allow(clippy::cast_precision_loss)]
    let drain = implants.implants.len() as f32 * MAINTENANCE_DRAIN_PER_IMPLANT;
    maint.current_charge = (maint.current_charge - drain).max(0.0);
}

/// Spec 405: implants drain maintenance charge over time.
pub fn cybernetic_maintenance_system(
    mut query: Query<(&CyberneticImplants, &mut CyberneticMaintenance)>,
) {
    for (implants, mut maint) in &mut query {
        cybernetic_maintenance_tick(implants, &mut maint);
    }
}

/// Spec 405: derive and sync a pop's cybernetic modifiers onto the real
/// seams. Runs every tick (not `Changed<>`-gated) because the battery
/// crossing zero must switch bonuses off the same tick, and every sync
/// is idempotent: armor adjusts `DamageResistance` by delta only, and
/// the Theseus / dead-battery mood modifiers are refreshed in place
/// under their labels instead of stacking.
#[allow(clippy::type_complexity)]
pub fn apply_cybernetic_modifiers_system(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &CyberneticImplants,
        Option<&CyberneticMaintenance>,
        Option<&mut CyberneticModifiers>,
        Option<&mut crate::layer1::morale::Morale>,
        Option<&mut crate::layer1::biology::health::DamageResistance>,
    )>,
    mut chronicle: Option<ResMut<Events<crate::layer1::core::chronicle::AddChronicleEvent>>>,
) {
    for (entity, implants, maint, modifiers, morale, mut resistance) in &mut query {
        let powered = maint.is_none_or(|m| !m.is_dead());
        let (work, social, armor) = implant_modifiers(&implants.implants, powered);
        let theseus = implants.implants.len() >= THESEUS_IMPLANT_THRESHOLD;
        let dead = maint.is_some_and(|m| m.is_dead()) && !implants.implants.is_empty();

        let (prev_armor_applied, prev_theseus) = modifiers
            .as_deref()
            .map_or((0.0, false), |m| (m.armor_applied, m.theseus_active));

        // Armor seam: adjust the real DamageResistance by delta only.
        let has_resistance = resistance.is_some();
        let armor_delta = armor - prev_armor_applied;
        if armor_delta != 0.0 {
            if let Some(dr) = &mut resistance {
                dr.physical = (dr.physical + armor_delta).max(0.0);
            } else if armor > 0.0 {
                commands
                    .entity(entity)
                    .insert(crate::layer1::biology::health::DamageResistance {
                        physical: armor,
                    });
            }
        }
        let armor_applied = if !has_resistance && prev_armor_applied == 0.0 && armor == 0.0 {
            0.0
        } else {
            armor
        };

        // Morale seam: refresh exactly one labelled modifier per state.
        if let Some(mut morale) = morale {
            sync_cyber_mood(&mut morale, THESEUS_MORALE_LABEL, THESEUS_MORALE_VALUE, theseus);
            sync_cyber_mood(
                &mut morale,
                DEAD_IMPLANTS_MORALE_LABEL,
                DEAD_IMPLANTS_MORALE_VALUE,
                dead,
            );
        }

        // Chronicle the Theseus crossing once, on the rising edge.
        if theseus && !prev_theseus {
            if let Some(ref mut events) = chronicle {
                events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                    text: "A colonist crosses the Ship of Theseus threshold — more machine than memory, and the others have started to notice.".to_string(),
                    importance: crate::layer1::core::chronicle::EventImportance::Major,
                });
            }
        }

        let derived = CyberneticModifiers {
            work_bonus: work,
            social_penalty: social,
            armor,
            powered,
            theseus_active: theseus,
            armor_applied,
        };
        if let Some(mut existing) = modifiers {
            *existing = derived;
        } else {
            commands.entity(entity).insert(derived);
        }
    }
}

/// Refresh (or, when inactive, stop refreshing) one labelled mood
/// modifier. Active entries are rewritten with a short duration every
/// tick, so they read as permanent while the condition holds and decay
/// within a couple of ticks after it ends — never stacking duplicates.
fn sync_cyber_mood(
    morale: &mut crate::layer1::morale::Morale,
    label: &str,
    value: f32,
    active: bool,
) {
    morale.modifiers.retain(|m| m.label != label);
    if active {
        morale.modifiers.push(crate::layer1::morale::MoodModifier {
            label: label.to_string(),
            value,
            duration: CYBER_MOOD_REFRESH_TICKS,
        });
    }
}

/// Spec 405 install path (the surgical seam): register one implant on a
/// pop through the REAL prosthetic machinery — a [`Prosthetic`] entity
/// is spawned and added to the pop's [`Augmentations`], so the existing
/// work pipeline (`get_efficiency_bonus` in general work) and the
/// cybernetic-prejudice affinity system consume it with no parallel
/// stat system. Also records the loadout in [`CyberneticImplants`] and
/// ensures a full [`CyberneticMaintenance`] battery exists.
/// Returns false when the pop entity does not exist.
pub fn install_implant(world: &mut World, pop: Entity, implant: ImplantType) -> bool {
    if world.get_entity(pop).is_err() {
        return false;
    }
    let prosthetic = world
        .spawn(Prosthetic {
            prosthetic_type: implant.prosthetic_type(),
            efficiency_bonus: implant.work_bonus(),
            social_penalty: implant.social_penalty(),
            power_consumption: 1.0,
        })
        .id();
    if world.get::<Augmentations>(pop).is_none() {
        world.entity_mut(pop).insert(Augmentations::default());
    }
    if let Some(mut augs) = world.get_mut::<Augmentations>(pop) {
        augs.installed.push(prosthetic);
    }
    if world.get::<CyberneticImplants>(pop).is_none() {
        world.entity_mut(pop).insert(CyberneticImplants::default());
    }
    if let Some(mut loadout) = world.get_mut::<CyberneticImplants>(pop) {
        loadout.implants.push(implant);
    }
    if world.get::<CyberneticMaintenance>(pop).is_none() {
        world
            .entity_mut(pop)
            .insert(CyberneticMaintenance::default());
    }
    true
}

/// Recharge a pop's implant battery by `amount` (the maintenance seam
/// a recharge station / energy-cell job would call). Returns false when
/// the pop has no battery.
pub fn recharge_implants(world: &mut World, pop: Entity, amount: f32) -> bool {
    if let Some(mut maint) = world.get_mut::<CyberneticMaintenance>(pop) {
        maint.recharge(amount);
        true
    } else {
        false
    }
}

/// Census of spec 405 augmentation for headless observability.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct CyberneticCensus {
    /// Pops carrying at least one implant.
    pub augmented: usize,
    /// Total installed implants across the colony.
    pub implants: usize,
    /// Augmented pops whose battery is dead.
    pub dead_batteries: usize,
    /// Mean charge fraction across augmented pops (0.0 when none).
    pub avg_charge_fraction: f32,
}

/// Count augmentation state across all pops in a raw `World`.
#[must_use]
pub fn cybernetic_census(world: &mut World) -> CyberneticCensus {
    let mut census = CyberneticCensus::default();
    let mut fraction_sum = 0.0f32;
    let mut query = world.query::<(&CyberneticImplants, Option<&CyberneticMaintenance>)>();
    for (loadout, maint) in query.iter(world) {
        if loadout.implants.is_empty() {
            continue;
        }
        census.augmented += 1;
        census.implants += loadout.implants.len();
        match maint {
            Some(m) => {
                fraction_sum += m.fraction();
                if m.is_dead() {
                    census.dead_batteries += 1;
                }
            }
            None => fraction_sum += 1.0,
        }
    }
    if census.augmented > 0 {
        #[allow(clippy::cast_precision_loss)]
        let n = census.augmented as f32;
        census.avg_charge_fraction = fraction_sum / n;
    }
    census
}

/// Compact STATS label for spec 405 (full panel: `cyber`):
/// `<augmented>p/<implants>i/<dead>dead`.
#[must_use]
pub fn cyber_stats_label(world: &mut World) -> String {
    let census = cybernetic_census(world);
    format!(
        "{}p/{}i/{}dead",
        census.augmented, census.implants, census.dead_batteries
    )
}


// ============================================================================
// Spec 405: Cybernetic Augmentation — installed implant loadout, maintenance
// charge, and derived modifiers on the REAL seams (see module section below).
// ============================================================================

#[cfg(test)]
mod spec405_tests {
    use super::*;
    use crate::layer1::biology::health::DamageResistance;
    use crate::layer1::morale::Morale;
    use crate::layer1::pop::Pop;
    use bevy_app::{App, Update};

    fn run_systems(app: &mut App) {
        app.add_systems(
            Update,
            (
                cybernetic_maintenance_system,
                apply_cybernetic_modifiers_system,
            )
                .chain(),
        );
    }

    /// Spec RED test 1, adapted: the spec asserted on a `PopSkills`
    /// component with `work_speed`/`social_affinity` fields. That type
    /// does not exist — the real skill type is `Skills` (an XP map whose
    /// efficiency is derived from level). The adapted assertion targets
    /// the derived `CyberneticModifiers` ledger plus the real consumed
    /// seams (`get_efficiency_bonus` / `get_social_penalty` via the
    /// `Augmentations`/`Prosthetic` registration `install_implant`
    /// performs, and `DamageResistance` for armor).
    #[test]
    fn test_augmentation_applies_stat_boosts() {
        let mut app = App::new();
        run_systems(&mut app);
        let pop = app
            .world_mut()
            .spawn((
                Pop,
                CyberneticImplants {
                    implants: vec![ImplantType::MotorServo, ImplantType::NeuralLink],
                },
                CyberneticMaintenance::default(),
            ))
            .id();
        app.update();
        let mods = app.world().get::<CyberneticModifiers>(pop).unwrap();
        // Base work factor 1.0 + MotorServo 0.5 (spec: work_speed 1.5).
        assert!(
            (1.0 + mods.work_bonus - 1.5).abs() < f32::EPSILON,
            "MotorServo should boost work speed to 1.5x, got bonus {}",
            mods.work_bonus
        );
        // Base social factor 1.0 - NeuralLink 0.3 (spec: social 0.7).
        assert!(
            (1.0 - mods.social_penalty - 0.7).abs() < f32::EPSILON,
            "NeuralLink should reduce social affinity to 0.7x, got penalty {}",
            mods.social_penalty
        );
    }

    /// Spec RED test 2, verbatim in spirit: charge drains over time.
    #[test]
    fn test_augmentation_requires_maintenance() {
        let mut app = App::new();
        app.add_systems(Update, cybernetic_maintenance_system);
        let pop = app
            .world_mut()
            .spawn((
                Pop,
                CyberneticImplants {
                    implants: vec![ImplantType::MotorServo],
                },
                CyberneticMaintenance {
                    current_charge: 100.0,
                    max_charge: 100.0,
                },
            ))
            .id();
        app.update();
        let maint = app.world().get::<CyberneticMaintenance>(pop).unwrap();
        assert!(
            maint.current_charge < 100.0,
            "Implants should drain maintenance charge over time"
        );
        assert!(
            (maint.current_charge - 99.9).abs() < 0.001,
            "one implant drains 0.1/tick, got {}",
            maint.current_charge
        );
    }

    #[test]
    fn test_maintenance_drain_scales_with_implant_count_and_clamps() {
        let mut maint = CyberneticMaintenance {
            current_charge: 0.15,
            max_charge: 100.0,
        };
        let implants = CyberneticImplants {
            implants: vec![ImplantType::MotorServo, ImplantType::NeuralLink],
        };
        cybernetic_maintenance_tick(&implants, &mut maint);
        assert_eq!(maint.current_charge, 0.0, "charge clamps at zero");
        assert!(maint.is_dead());
    }

    #[test]
    fn test_dead_battery_disables_work_and_armor_but_not_alienation() {
        let mut app = App::new();
        run_systems(&mut app);
        let pop = app
            .world_mut()
            .spawn((
                Pop,
                CyberneticImplants {
                    implants: vec![
                        ImplantType::MotorServo,
                        ImplantType::NeuralLink,
                        ImplantType::SubdermalArmor,
                    ],
                },
                CyberneticMaintenance {
                    current_charge: 0.0,
                    max_charge: 100.0,
                },
                Morale::default(),
            ))
            .id();
        app.update();
        let mods = app.world().get::<CyberneticModifiers>(pop).unwrap();
        assert_eq!(mods.work_bonus, 0.0, "dead implants give no work bonus");
        assert_eq!(mods.armor, 0.0, "dead plating gives no armor");
        assert!(
            mods.social_penalty > 0.0,
            "alienation is how you look, not whether it runs"
        );
        assert!(mods.theseus_active, "3 implants trigger Ship of Theseus");
        let morale = app.world().get::<Morale>(pop).unwrap();
        assert!(
            morale
                .modifiers
                .iter()
                .any(|m| m.label == THESEUS_MORALE_LABEL),
            "Theseus alienation mood modifier should be present"
        );
        assert!(
            morale
                .modifiers
                .iter()
                .any(|m| m.label == DEAD_IMPLANTS_MORALE_LABEL),
            "dead-battery mood modifier should be present"
        );
    }

    #[test]
    fn test_subdermal_armor_feeds_real_damage_resistance() {
        let mut app = App::new();
        run_systems(&mut app);
        let pop = app
            .world_mut()
            .spawn((
                Pop,
                CyberneticImplants {
                    implants: vec![ImplantType::SubdermalArmor],
                },
                CyberneticMaintenance::default(),
            ))
            .id();
        app.update();
        let dr = app.world().get::<DamageResistance>(pop).unwrap();
        assert!(
            (dr.physical - SUBDERMAL_ARMOR_RATING).abs() < f32::EPSILON,
            "armor should add {SUBDERMAL_ARMOR_RATING} physical resistance, got {}",
            dr.physical
        );
        // Idempotent: a second tick must not stack the armor again.
        app.update();
        let dr = app.world().get::<DamageResistance>(pop).unwrap();
        assert!(
            (dr.physical - SUBDERMAL_ARMOR_RATING).abs() < f32::EPSILON,
            "armor sync must be idempotent, got {}",
            dr.physical
        );
    }

    #[test]
    fn test_install_implant_registers_on_real_prosthetic_seams() {
        let mut world = World::new();
        let pop = world.spawn(Pop).id();
        assert!(install_implant(&mut world, pop, ImplantType::MotorServo));
        assert!(install_implant(&mut world, pop, ImplantType::NeuralLink));
        // Real work seam: general_work's augmentation bonus.
        assert!(
            (get_efficiency_bonus(&world, pop) - 0.5).abs() < f32::EPSILON,
            "MotorServo should feed get_efficiency_bonus"
        );
        // Real social seam: modify_affinity_system's prejudice penalty.
        assert!(
            (get_social_penalty(&world, pop) - 0.3).abs() < f32::EPSILON,
            "NeuralLink should feed get_social_penalty"
        );
        assert!(world.get::<CyberneticMaintenance>(pop).is_some());
        assert_eq!(
            world.get::<CyberneticImplants>(pop).unwrap().implants.len(),
            2
        );
        // Recharge restores a drained battery.
        world.get_mut::<CyberneticMaintenance>(pop).unwrap().current_charge = 10.0;
        assert!(recharge_implants(&mut world, pop, 50.0));
        assert!(
            (world.get::<CyberneticMaintenance>(pop).unwrap().current_charge - 60.0).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn test_census_counts_augmented_pops() {
        let mut world = World::new();
        let a = world.spawn(Pop).id();
        let _b = world.spawn(Pop).id();
        install_implant(&mut world, a, ImplantType::MotorServo);
        install_implant(&mut world, a, ImplantType::SubdermalArmor);
        let census = cybernetic_census(&mut world);
        assert_eq!(census.augmented, 1);
        assert_eq!(census.implants, 2);
        assert_eq!(census.dead_batteries, 0);
    }

    #[test]
    fn test_ip_guard_no_lifted_names() {
        // Original names only (concept-only IP rule): the public surface
        // of this spec must not carry distinctive lifted terminology.
        let surface = format!(
            "{} {} {} {} {}",
            ImplantType::MotorServo.name(),
            ImplantType::NeuralLink.name(),
            ImplantType::SubdermalArmor.name(),
            THESEUS_MORALE_LABEL,
            DEAD_IMPLANTS_MORALE_LABEL
        )
        .to_lowercase();
        for banned in ["cyberpunk", "ripperdoc", "warhammer", "borg", "deus ex"] {
            assert!(!surface.contains(banned), "banned term present: {banned}");
        }
    }
}
