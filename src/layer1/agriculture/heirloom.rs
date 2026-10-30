//! Spec 377: Heirloom Seeds.
//!
//! The Earth crops the colony brought with it — wheat and potatoes — are
//! the last of their kind. [`SeedStock`] is a finite, non-replenishing
//! reserve: planting an [`EarthCrop`] consumes one seed, harvesting one
//! yields food plus a *probabilistic* seed return rolled on a seeded,
//! deterministic RNG ([`SeedReturnRng`], splitmix64 — never thread-rand,
//! so playthroughs and tests are reproducible and never flaky).
//!
//! A [`SeedDisasterEvent`] (crop disease or seed-vault fire) can wipe
//! planted Earth crops or the stored stock. If the stock hits zero with
//! no Earth crop left standing, [`native_adoption_system`] fires once:
//! the colony permanently adopts the native Grub-Root — farms growing
//! wheat/potatoes are converted, every pop takes a lasting gut/morale
//! hit, and a Major chronicle entry records the end of Earth agriculture.
//!
//! This is the *discrete* seed leg the spec asks for. The continuous
//! `produce_food_system` in `farm.rs` is deliberately NOT seed-gated:
//! its yield is an abstract per-tick trickle (0.004–0.006 food), and
//! charging a seed per tick would drain any finite stock in minutes.
//! Seeds govern planting/harvesting decisions, disasters, and diet —
//! not the per-tick trickle. The pre-existing `layer1::heirloom`
//! (ancient structures / heirloom tools) is a different concept and is
//! untouched; Crop Diversity (spec 120) crop stats in `farm.rs` are
//! reused via [`ItemType`] rather than duplicated.

use crate::layer1::agriculture::farm::Farm;
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::items::ItemType;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::entities::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
use bevy_ecs::prelude::*;

/// Wheat seeds in the colony's initial, non-replenishing stock.
pub const INITIAL_WHEAT_SEEDS: u32 = 20;
/// Potato seeds in the colony's initial, non-replenishing stock.
pub const INITIAL_POTATO_SEEDS: u32 = 20;
/// Default seed-return probability for wheat (tuned < 1: a slow bleed).
pub const WHEAT_SEED_CHANCE: f32 = 0.6;
/// Default seed-return probability for potatoes (hardier, returns more).
pub const POTATO_SEED_CHANCE: f32 = 0.75;
/// Default food yield of one harvested Earth crop planting.
pub const EARTH_CROP_YIELD: u32 = 10;
/// Morale value of the Grub-Root gut modifier applied on adoption.
pub const GRUB_ROOT_MORALE_HIT: f32 = -0.15;
/// Duration (ticks) of the Grub-Root gut modifier.
pub const GRUB_ROOT_MODIFIER_TICKS: u32 = 2000;

/// Which Earth seed lineage a planting belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SeedKind {
    /// Wheat (maps to [`ItemType::Wheat`]).
    Wheat,
    /// Potatoes (maps to [`ItemType::Potato`]).
    Potato,
}

impl SeedKind {
    /// The food item this seed grows into.
    #[must_use]
    pub const fn item_type(self) -> ItemType {
        match self {
            Self::Wheat => ItemType::Wheat,
            Self::Potato => ItemType::Potato,
        }
    }

    /// Default seed-return chance for this lineage.
    #[must_use]
    pub const fn default_seed_chance(self) -> f32 {
        match self {
            Self::Wheat => WHEAT_SEED_CHANCE,
            Self::Potato => POTATO_SEED_CHANCE,
        }
    }

    /// Parse a headless/console seed name (`wheat` / `potato`/`potatoes`).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "wheat" => Some(Self::Wheat),
            "potato" | "potatoes" | "potatoe" => Some(Self::Potato),
            _ => None,
        }
    }
}

/// Finite, non-replenishing stock of Earth seeds.
///
/// Nothing in the sim refills this except harvest seed returns — that
/// scarcity is the whole mechanic.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedStock {
    /// Stored wheat seeds.
    pub wheat: u32,
    /// Stored potato seeds.
    pub potatoes: u32,
}

impl Default for SeedStock {
    fn default() -> Self {
        Self {
            wheat: INITIAL_WHEAT_SEEDS,
            potatoes: INITIAL_POTATO_SEEDS,
        }
    }
}

impl SeedStock {
    /// An empty stock (tests / post-disaster).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            wheat: 0,
            potatoes: 0,
        }
    }

    /// Total seeds across both lineages.
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.wheat + self.potatoes
    }

    /// Seeds stored for one lineage.
    #[must_use]
    pub const fn amount(&self, kind: SeedKind) -> u32 {
        match kind {
            SeedKind::Wheat => self.wheat,
            SeedKind::Potato => self.potatoes,
        }
    }

    /// Take one seed of `kind`; returns false (taking nothing) at zero.
    pub fn take(&mut self, kind: SeedKind) -> bool {
        let slot = match kind {
            SeedKind::Wheat => &mut self.wheat,
            SeedKind::Potato => &mut self.potatoes,
        };
        if *slot == 0 {
            return false;
        }
        *slot -= 1;
        true
    }

    /// Return one seed of `kind` to the stock (harvest recovery).
    pub fn give(&mut self, kind: SeedKind) {
        match kind {
            SeedKind::Wheat => self.wheat += 1,
            SeedKind::Potato => self.potatoes += 1,
        }
    }
}

/// Seeded, deterministic RNG for seed-return rolls (splitmix64).
///
/// A fixed default seed means the same sequence of harvests always
/// returns the same sequence of seeds — reproducible playthroughs and
/// zero flaky tests, unlike `thread_rng`.
#[derive(Resource, Debug, Clone, Copy)]
pub struct SeedReturnRng {
    state: u64,
}

impl Default for SeedReturnRng {
    fn default() -> Self {
        Self::new(0x3775_EED5)
    }
}

impl SeedReturnRng {
    /// Create with an explicit seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next value in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // Top 24 bits -> [0, 1).
        #[allow(clippy::cast_precision_loss)]
        let v = (z >> 40) as f32 / (1u64 << 24) as f32;
        v
    }

    /// Roll against a probability in `[0, 1]` (1.0 always, 0.0 never).
    pub fn roll(&mut self, chance: f32) -> bool {
        if chance >= 1.0 {
            return true;
        }
        if chance <= 0.0 {
            return false;
        }
        self.next_f32() < chance
    }
}

/// A planted Earth crop (one discrete planting, not a farm building).
#[derive(Component, Debug, Clone, Copy)]
pub struct EarthCrop {
    /// Which seed lineage this planting is.
    pub kind: SeedKind,
    /// Food added to [`ColonyResources`] on harvest.
    pub yield_amount: u32,
    /// Probability (seeded roll) that harvest returns one seed.
    pub seed_chance: f32,
}

impl EarthCrop {
    /// A standard planting of `kind` with its default yield/chance.
    #[must_use]
    pub const fn new(kind: SeedKind) -> Self {
        Self {
            kind,
            yield_amount: EARTH_CROP_YIELD,
            seed_chance: kind.default_seed_chance(),
        }
    }
}

/// Marker: this planting grows the native Grub-Root, not an Earth crop.
/// Grub-Root needs no seed, yields less, and carries dietary side effects.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct GrubRootCrop;

/// Colony-level heirloom-seed state: counters and the one-way native flag.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct HeirloomSeedState {
    /// Earth plantings made.
    pub plantings: u32,
    /// Earth crops harvested.
    pub harvests: u32,
    /// Seeds recovered from harvests.
    pub seeds_returned: u32,
    /// Disasters suffered.
    pub disasters: u32,
    /// True once the last Earth seed is gone and Grub-Root is the diet.
    /// One-way: there is no path back to Earth agriculture.
    pub native_adopted: bool,
}

/// Request to plant the [`EarthCrop`] on `crop` (consumes one seed).
#[derive(Event, Debug, Clone, Copy)]
pub struct PlantCropEvent {
    /// Entity carrying the [`EarthCrop`] to plant.
    pub crop: Entity,
}

/// Request to harvest the [`EarthCrop`] on `crop` (food + seed roll).
#[derive(Event, Debug, Clone, Copy)]
pub struct HarvestCropEvent {
    /// Entity carrying the [`EarthCrop`] to harvest.
    pub crop: Entity,
}

/// What a seed disaster destroys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisasterTarget {
    /// Seeds sitting in storage (the doomsday-vault nightmare).
    StoredSeeds,
    /// Earth crops currently planted.
    PlantedCrops,
}

/// How a seed disaster arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedDisasterKind {
    /// Crop disease / seed blight.
    Disease,
    /// Fire in the fields or the seed vault.
    Fire,
}

/// A crop disease or fire strikes the seed supply.
#[derive(Event, Debug, Clone, Copy)]
pub struct SeedDisasterEvent {
    /// Disease or fire.
    pub kind: SeedDisasterKind,
    /// Stored seeds or planted crops.
    pub target: DisasterTarget,
    /// Fraction destroyed, `0.0..=1.0` (1.0 = total loss).
    pub severity: f32,
}

/// How many of `amount` a `severity` fraction destroys (rounded down).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn fraction_lost(amount: u32, severity: f32) -> u32 {
    (amount as f32 * severity) as u32
}

/// Planting consumes one seed of the crop's lineage.
///
/// If the stock for that lineage is empty the planting fails: the
/// unplanted [`EarthCrop`] is removed (the would-be crop never takes).
pub fn process_planting_system(
    mut events: EventReader<PlantCropEvent>,
    mut commands: Commands,
    mut stock: ResMut<SeedStock>,
    mut state: ResMut<HeirloomSeedState>,
    crops: Query<&EarthCrop>,
) {
    for event in events.read() {
        let Ok(crop) = crops.get(event.crop) else {
            continue;
        };
        if stock.take(crop.kind) {
            state.plantings += 1;
        } else {
            commands.entity(event.crop).remove::<EarthCrop>();
        }
    }
}

/// Harvesting yields food and rolls (seeded) for a seed return.
///
/// The harvested crop entity is despawned either way.
pub fn process_harvest_yield_system(
    mut events: EventReader<HarvestCropEvent>,
    mut commands: Commands,
    mut stock: ResMut<SeedStock>,
    mut state: ResMut<HeirloomSeedState>,
    mut rng: ResMut<SeedReturnRng>,
    mut resources: ResMut<ColonyResources>,
    crops: Query<&EarthCrop>,
) {
    for event in events.read() {
        let Ok(crop) = crops.get(event.crop) else {
            continue;
        };
        #[allow(clippy::cast_precision_loss)]
        resources.add_food(crop.yield_amount as f32);
        state.harvests += 1;
        if rng.roll(crop.seed_chance) {
            stock.give(crop.kind);
            state.seeds_returned += 1;
        }
        commands.entity(event.crop).despawn();
    }
}

/// A disease/fire disaster destroys a fraction of stored seeds or of
/// planted Earth crops, and chronicles the loss.
pub fn seed_disaster_system(
    mut events: EventReader<SeedDisasterEvent>,
    mut commands: Commands,
    mut stock: ResMut<SeedStock>,
    mut state: ResMut<HeirloomSeedState>,
    crops: Query<Entity, With<EarthCrop>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        let severity = event.severity.clamp(0.0, 1.0);
        state.disasters += 1;
        let (lost, what) = match event.target {
            DisasterTarget::StoredSeeds => {
                let wheat_lost = fraction_lost(stock.wheat, severity);
                let potato_lost = fraction_lost(stock.potatoes, severity);
                // A total-severity event always takes everything (no
                // rounding survivor mocking the vault keeper).
                let (wheat_lost, potato_lost) = if severity >= 1.0 {
                    (stock.wheat, stock.potatoes)
                } else {
                    (wheat_lost.min(stock.wheat), potato_lost.min(stock.potatoes))
                };
                stock.wheat -= wheat_lost;
                stock.potatoes -= potato_lost;
                (wheat_lost + potato_lost, "stored Earth seeds")
            }
            DisasterTarget::PlantedCrops => {
                let planted: Vec<Entity> = crops.iter().collect();
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let mut to_destroy = (planted.len() as f32 * severity) as usize;
                if severity >= 1.0 {
                    to_destroy = planted.len();
                }
                let mut destroyed = 0u32;
                for entity in planted.into_iter().take(to_destroy) {
                    commands.entity(entity).despawn();
                    destroyed += 1;
                }
                (destroyed, "planted Earth crops")
            }
        };
        let cause = match event.kind {
            SeedDisasterKind::Disease => "Crop disease",
            SeedDisasterKind::Fire => "Fire",
        };
        chronicle.send(AddChronicleEvent {
            text: format!("{cause} destroys {lost} {what}. The Earth seed heritage shrinks."),
            importance: EventImportance::Major,
        });
    }
}

/// The one-way door: when no Earth seeds remain stored or planted, the
/// colony adopts native Grub-Root permanently.
///
/// Farms growing wheat/potatoes are converted to Grub-Root, every pop
/// takes the Grub-Root gut modifier (the native root is edible, filling,
/// and quietly wrong), and a Major chronicle entry marks the end of
/// Earth agriculture.
pub fn native_adoption_system(
    mut commands: Commands,
    stock: Res<SeedStock>,
    mut state: ResMut<HeirloomSeedState>,
    crops: Query<(), With<EarthCrop>>,
    mut farms: Query<&mut Farm>,
    mut pops: Query<(Entity, Option<&mut Morale>), With<Pop>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    if state.native_adopted || stock.total() > 0 || !crops.is_empty() {
        return;
    }
    state.native_adopted = true;
    for mut farm in &mut farms {
        if matches!(farm.selected_crop, ItemType::Wheat | ItemType::Potato) {
            farm.selected_crop = ItemType::GrubRoot;
        }
    }
    for (entity, morale) in &mut pops {
        if let Some(mut morale) = morale {
            morale.modifiers.push(MoodModifier {
                label: "Grub-Root Diet".to_string(),
                value: GRUB_ROOT_MORALE_HIT,
                duration: GRUB_ROOT_MODIFIER_TICKS,
            });
        } else {
            commands.entity(entity).insert(Morale {
                value: 0.8,
                modifiers: vec![MoodModifier {
                    label: "Grub-Root Diet".to_string(),
                    value: GRUB_ROOT_MORALE_HIT,
                    duration: GRUB_ROOT_MODIFIER_TICKS,
                }],
            });
        }
    }
    chronicle.send(AddChronicleEvent {
        text: "The last Earth seed is gone. The colony adopts native Grub-Root — edible, filling, and quietly wrong. Earth agriculture is over.".to_string(),
        importance: EventImportance::Major,
    });
}
