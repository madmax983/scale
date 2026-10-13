//! # Planetary Traits (Quirks)
//!
//! This module defines the planetary-scale physical and atmospheric modifiers that
//! alter the fundamental constants of a simulation scenario. Rather than hardcoding
//! environment variables like gravity or orbit duration, traits like [`PlanetaryTrait::HighGravity`]
//! and [`PlanetaryTrait::RapidOrbit`] dynamically shift these rules.
//!
//! This is the mechanical foundation of a planet's "Personality", affecting base entity stats like
//! pop movement, solar power output, and the total tick duration of a solar day.
//!
//! ## Examples
//!
//! Simulating a world with a thin atmosphere and a slow orbit:
//!
//! ```rust
//! use bevy_ecs::prelude::*;
//! use scale::layer1::quirks::{PlanetaryTrait, PlanetaryTraits, apply_quirk_modifiers_system};
//! use scale::layer1::day_night::DayNightCycle;
//!
//! let mut world = World::new();
//!
//! // Define the specific traits for the scenario
//! world.insert_resource(PlanetaryTraits(vec![
//!     PlanetaryTrait::ThinAtmosphere,
//!     PlanetaryTrait::SlowOrbit,
//! ]));
//!
//! // Provide the necessary default component for the cycle
//! world.insert_resource(DayNightCycle::default());
//!
//! // Run the application of modifiers
//! let mut schedule = Schedule::default();
//! schedule.add_systems(apply_quirk_modifiers_system);
//! schedule.run(&mut world);
//!
//! // The slow orbit trait lengthens the day by 100%.
//! // Standard base tick duration is 250 ticks.
//! let day_night = world.resource::<DayNightCycle>();
//! assert_eq!(day_night.ticks_per_day, 500);
//! ```

use crate::layer1::atmosphere::AtmosphereGrid;
use crate::layer1::building::{Building, BuildingType};
use crate::layer1::day_night::DayNightCycle;
use crate::layer1::energy::PowerSource;
use crate::layer1::flora::{Flora, FloraType};
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, Speed};
use bevy_ecs::prelude::*;

/// Traits that modify planetary conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanetaryTrait {
    /// High gravity slows movement (-20%).
    HighGravity,
    /// Low gravity speeds movement (+20%).
    LowGravity,
    /// Rapid orbit shortens day length (-50%).
    RapidOrbit,
    /// Slow orbit lengthens day length (+100%).
    SlowOrbit,
    /// Dense atmosphere reduces solar power output (-20%).
    DenseAtmosphere,
    /// Thin atmosphere increases solar power output (+20%).
    ThinAtmosphere,
    /// Nullifies sound and suppresses fauna.
    SilentWorld,
    /// Planet is tidally locked (permanent day/night side).
    TidallyLocked,
    /// The local flora bites: strangling vines damage nearby pops (Spec 345).
    CarnivorousFlora,
    /// Volcanic world: geothermal power x2, ambient temperature +5C (Spec 345).
    Volcanic,
}

impl PlanetaryTrait {
    /// Retrieves the multiplier applied to the base movement speed of pops.
    ///
    /// Extreme gravitational forces directly alter traversal times across the map.
    /// High gravity worlds create slower logistical networks, making clustered base
    /// designs far more efficient than sprawling ones. Low gravity worlds allow
    /// rapid expansion but often come paired with thin atmospheres.
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use scale::layer1::quirks::PlanetaryTrait;
    ///
    /// let quirk = PlanetaryTrait::HighGravity;
    /// assert_eq!(quirk.speed_modifier(), 0.8);
    /// ```
    #[must_use]
    pub const fn speed_modifier(&self) -> f32 {
        match self {
            Self::HighGravity => 0.8,
            Self::LowGravity => 1.2,
            _ => 1.0,
        }
    }

    /// Retrieves the multiplier applied to the base tick length of a solar day.
    ///
    /// Modifying the day length alters the frequency of the [`DayNightCycle`],
    /// which dictates pop sleep schedules, solar power generation windows, and
    /// nocturnal threat spawning. A rapid orbit forces frequent context switching
    /// for pops, potentially increasing stress.
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use scale::layer1::quirks::PlanetaryTrait;
    ///
    /// let quirk = PlanetaryTrait::RapidOrbit;
    /// assert_eq!(quirk.day_length_modifier(), 0.5);
    /// ```
    #[must_use]
    pub const fn day_length_modifier(&self) -> f32 {
        match self {
            Self::RapidOrbit => 0.5,
            Self::SlowOrbit => 2.0,
            _ => 1.0,
        }
    }

    /// Retrieves the multiplier applied to the output of `Generator` building types.
    ///
    /// Atmospheric density directly affects light refraction and wind resistance.
    /// A dense atmosphere chokes out solar arrays, forcing colonies to rely
    /// heavily on subterranean geothermal or nuclear power sources early on.
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use scale::layer1::quirks::PlanetaryTrait;
    ///
    /// let quirk = PlanetaryTrait::DenseAtmosphere;
    /// assert_eq!(quirk.power_output_modifier(), 0.8);
    /// ```
    #[must_use]
    pub const fn power_output_modifier(&self) -> f32 {
        match self {
            Self::DenseAtmosphere => 0.8,
            Self::ThinAtmosphere => 1.2,
            _ => 1.0,
        }
    }

    /// Retrieves the atmospheric diffusion modifier affecting the `AtmosphereGrid`.
    ///
    /// High diffusion rates mean pollution and toxic gases dissipate quickly.
    /// A dense atmosphere traps gases, meaning heavy industry will rapidly render
    /// a local region uninhabitable without extensive precursor scrubber technology.
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use scale::layer1::quirks::PlanetaryTrait;
    ///
    /// let quirk = PlanetaryTrait::ThinAtmosphere;
    /// assert_eq!(quirk.diffusion_modifier(), 0.91);
    /// ```
    #[must_use]
    pub const fn diffusion_modifier(&self) -> f32 {
        match self {
            Self::DenseAtmosphere => 1.009, // ~0.999 retention
            Self::ThinAtmosphere => 0.91,   // ~0.90 retention
            _ => 1.0,
        }
    }

    /// Retrieves the UI-facing localization key or plain-text label for the trait.
    ///
    /// Used by the inspector UI to display the active planetary conditions.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::HighGravity => "High Gravity",
            Self::LowGravity => "Low Gravity",
            Self::RapidOrbit => "Rapid Orbit",
            Self::SlowOrbit => "Slow Orbit",
            Self::DenseAtmosphere => "Dense Atmosphere",
            Self::ThinAtmosphere => "Thin Atmosphere",
            Self::SilentWorld => "Silent World",
            Self::TidallyLocked => "Tidally Locked",
            Self::CarnivorousFlora => "Carnivorous Flora",
            Self::Volcanic => "Volcanic",
        }
    }

    /// Canonical machine name (no spaces), used in STATS and the headless
    /// `quirk <name>` command (Spec 345).
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::HighGravity => "HighGravity",
            Self::LowGravity => "LowGravity",
            Self::RapidOrbit => "RapidOrbit",
            Self::SlowOrbit => "SlowOrbit",
            Self::DenseAtmosphere => "DenseAtmosphere",
            Self::ThinAtmosphere => "ThinAtmosphere",
            Self::SilentWorld => "SilentWorld",
            Self::TidallyLocked => "TidallyLocked",
            Self::CarnivorousFlora => "CarnivorousFlora",
            Self::Volcanic => "Volcanic",
        }
    }

    /// Every quirk, in stable registration order (Spec 345).
    pub const ALL: &[Self] = &[
        Self::HighGravity,
        Self::LowGravity,
        Self::RapidOrbit,
        Self::SlowOrbit,
        Self::DenseAtmosphere,
        Self::ThinAtmosphere,
        Self::SilentWorld,
        Self::TidallyLocked,
        Self::CarnivorousFlora,
        Self::Volcanic,
    ];

    /// Parses a quirk from a player-typed name: case-insensitive, and
    /// spaces / underscores / hyphens are all equivalent separators.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let normalized: String = raw
            .chars()
            .filter(|c| !matches!(c, ' ' | '_' | '-'))
            .collect::<String>()
            .to_lowercase();
        Self::ALL
            .iter()
            .copied()
            .find(|q| q.name().to_lowercase() == normalized)
    }

    /// Multiplier on the per-tile movement *cost* (the threshold the speed
    /// accumulator must reach in `execution::movement`). This is the real
    /// form of Spec 345's RED test — the sketch's mock `PopMovementCost`
    /// maps onto this cost, distinct from the speed multiplier above:
    /// high gravity both slows pops (0.8x speed) and makes each tile
    /// cost half again as much energy (1.5x cost).
    #[must_use]
    pub const fn movement_cost_modifier(&self) -> f32 {
        match self {
            Self::HighGravity => 1.5,
            Self::LowGravity => 0.8,
            _ => 1.0,
        }
    }

    /// Multiplier on solar power only (real path: `nature::solar`).
    /// Spec 345 §7: a dense atmosphere halves sunlight; thin air helps.
    #[must_use]
    pub const fn solar_modifier(&self) -> f32 {
        match self {
            Self::DenseAtmosphere => 0.5,
            Self::ThinAtmosphere => 1.2,
            _ => 1.0,
        }
    }

    /// Multiplier on wind power. Spec 345 §7: dense air drives turbines
    /// twice as hard; thin air barely turns them.
    #[must_use]
    pub const fn wind_modifier(&self) -> f32 {
        match self {
            Self::DenseAtmosphere => 2.0,
            Self::ThinAtmosphere => 0.7,
            _ => 1.0,
        }
    }

    /// Multiplier on geothermal power. Spec 345 §7: volcanic worlds
    /// vent twice the usable heat.
    #[must_use]
    pub const fn geothermal_modifier(&self) -> f32 {
        match self {
            Self::Volcanic => 2.0,
            _ => 1.0,
        }
    }

    /// Additive ambient-temperature offset in Celsius (Spec 345 §7:
    /// volcanic worlds run hot). Additive, unlike the multipliers —
    /// stacking two volcanic-class effects should keep adding heat.
    #[must_use]
    pub const fn temperature_offset(&self) -> f32 {
        match self {
            Self::Volcanic => 5.0,
            _ => 0.0,
        }
    }
}

/// Resource containing the active planetary traits for the current game.
#[derive(Resource, Default, Debug)]
pub struct PlanetaryTraits(pub Vec<PlanetaryTrait>);

impl PlanetaryTraits {
    /// Whether a specific quirk is active on this world.
    #[must_use]
    pub fn has(&self, quirk: PlanetaryTrait) -> bool {
        self.0.contains(&quirk)
    }
}

/// Centralized global modifiers derived from the active quirks
/// (Spec 345 REFACTOR phase). Instead of every system re-checking the
/// quirk list, systems read the aggregated multipliers from here —
/// movement cost in `execution::movement`, solar in `nature::solar`,
/// ambient temperature in `nature::temperature`. Refreshed every tick
/// by [`update_global_modifiers_system`] and recomputed on demand by
/// [`apply_quirk_modifiers_system`].
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GlobalModifiers {
    /// Product of `speed_modifier` (High Gravity 0.8, Low Gravity 1.2).
    pub movement_speed: f32,
    /// Product of `movement_cost_modifier` (High Gravity 1.5).
    pub movement_cost: f32,
    /// Product of `solar_modifier` (Dense Atmosphere 0.5).
    pub solar: f32,
    /// Product of `wind_modifier` (Dense Atmosphere 2.0).
    pub wind: f32,
    /// Product of `geothermal_modifier` (Volcanic 2.0).
    pub geothermal: f32,
    /// Product of `day_length_modifier` (Rapid Orbit 0.5, Slow Orbit 2.0).
    pub day_length: f32,
    /// Product of `power_output_modifier` for generic Generators.
    pub power_output: f32,
    /// Product of `diffusion_modifier` for the atmosphere grid.
    pub diffusion: f32,
    /// Sum of `temperature_offset` (Volcanic +5C).
    pub temperature_offset: f32,
    /// Whether the TidallyLocked quirk is active.
    pub tidally_locked: bool,
    /// Whether the CarnivorousFlora quirk is active.
    pub carnivorous_flora: bool,
}

impl Default for GlobalModifiers {
    fn default() -> Self {
        Self {
            movement_speed: 1.0,
            movement_cost: 1.0,
            solar: 1.0,
            wind: 1.0,
            geothermal: 1.0,
            day_length: 1.0,
            power_output: 1.0,
            diffusion: 1.0,
            temperature_offset: 0.0,
            tidally_locked: false,
            carnivorous_flora: false,
        }
    }
}

impl GlobalModifiers {
    /// Aggregates a quirk list into the centralized modifiers. Pure and
    /// deterministic — the single source of truth every consumer reads.
    #[must_use]
    pub fn from_traits(traits: &[PlanetaryTrait]) -> Self {
        let mut mods = Self::default();
        for quirk in traits {
            mods.movement_speed *= quirk.speed_modifier();
            mods.movement_cost *= quirk.movement_cost_modifier();
            mods.solar *= quirk.solar_modifier();
            mods.wind *= quirk.wind_modifier();
            mods.geothermal *= quirk.geothermal_modifier();
            mods.day_length *= quirk.day_length_modifier();
            mods.power_output *= quirk.power_output_modifier();
            mods.diffusion *= quirk.diffusion_modifier();
            mods.temperature_offset += quirk.temperature_offset();
            mods.tidally_locked |= *quirk == PlanetaryTrait::TidallyLocked;
            mods.carnivorous_flora |= *quirk == PlanetaryTrait::CarnivorousFlora;
        }
        mods
    }
}

/// Keeps the [`GlobalModifiers`] resource in sync with [`PlanetaryTraits`].
pub fn update_global_modifiers_system(
    traits: Res<PlanetaryTraits>,
    mut mods: ResMut<GlobalModifiers>,
) {
    let fresh = GlobalModifiers::from_traits(&traits.0);
    if *mods != fresh {
        *mods = fresh;
    }
}

/// Which third of a tidally locked world a column sits in
/// (Spec 345 REFACTOR: permanent day side, terminator, night side —
/// the west third basks, the east third never sees the sun).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TidalZone {
    /// Permanent daylight.
    DaySide,
    /// The twilight band between day and night.
    Terminator,
    /// Permanent night.
    NightSide,
}

/// The tidal zone for a grid column. Columns split into equal thirds;
/// a degenerate (zero/negative) width is treated as all day side.
#[must_use]
pub fn tidal_zone(x: i32, width: i32) -> TidalZone {
    if width <= 0 || x < width / 3 {
        TidalZone::DaySide
    } else if x < 2 * width / 3 {
        TidalZone::Terminator
    } else {
        TidalZone::NightSide
    }
}

/// Solar factor for a column on a tidally locked world:
/// full sun on the day side, half at the terminator, none at night.
#[must_use]
pub fn tidal_solar_factor(x: i32, width: i32) -> f32 {
    match tidal_zone(x, width) {
        TidalZone::DaySide => 1.0,
        TidalZone::Terminator => 0.5,
        TidalZone::NightSide => 0.0,
    }
}

/// Health a pop loses per tick spent next to strangling vines while
/// CarnivorousFlora is active (Spec 345).
pub const CARNIVOROUS_FLORA_DAMAGE: f32 = 1.5;

/// Carnivorous flora (Spec 345): on worlds with the quirk, strangling
/// vines are predators — any pop on or beside a vine tile takes damage
/// every tick. Without the quirk the same vines are merely scenery
/// (their pre-existing flora behaviour is untouched).
pub fn carnivorous_flora_system(
    traits: Res<PlanetaryTraits>,
    flora: Query<(&Flora, &GridPosition)>,
    mut pops: Query<(&GridPosition, &mut Health), With<Pop>>,
) {
    if !traits.has(PlanetaryTrait::CarnivorousFlora) {
        return;
    }
    let vine_tiles: Vec<GridPosition> = flora
        .iter()
        .filter(|(f, _)| f.flora_type == FloraType::StrangleVines)
        .map(|(_, pos)| *pos)
        .collect();
    if vine_tiles.is_empty() {
        return;
    }
    for (pop_pos, mut health) in &mut pops {
        if vine_tiles
            .iter()
            .any(|vine| pop_pos.distance_chebyshev(*vine) <= 1)
        {
            health.take_damage(CARNIVOROUS_FLORA_DAMAGE);
        }
    }
}

/// Comma-joined canonical quirk names for STATS (`quirks=`), or `none`.
#[must_use]
pub fn quirks_summary(world: &World) -> String {
    world
        .get_resource::<PlanetaryTraits>()
        .filter(|traits| !traits.0.is_empty())
        .map(|traits| {
            traits
                .0
                .iter()
                .map(|q| q.name())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_else(|| "none".to_string())
}

/// One-line-per-quirk human summary for the headless `quirks` panel,
/// including the aggregated global modifiers.
#[must_use]
pub fn quirks_status_label(world: &World) -> String {
    let Some(traits) = world.get_resource::<PlanetaryTraits>() else {
        return "No planetary quirks recorded for this world.".to_string();
    };
    if traits.0.is_empty() {
        return "This world is unremarkable — no planetary quirks.\n\
                Use `quirk <name>` to curse (or bless) it after the fact."
            .to_string();
    }
    let mods = GlobalModifiers::from_traits(&traits.0);
    let mut lines: Vec<String> = traits
        .0
        .iter()
        .map(|q| format!("- {} ({})", q.label(), q.name()))
        .collect();
    lines.push(format!(
        "Modifiers: movement speed x{:.2}, movement cost x{:.2}, solar x{:.2}, wind x{:.2}, \
         geothermal x{:.2}, day length x{:.2}, ambient {:+.0}C",
        mods.movement_speed,
        mods.movement_cost,
        mods.solar,
        mods.wind,
        mods.geothermal,
        mods.day_length,
        mods.temperature_offset,
    ));
    if mods.tidally_locked {
        lines.push(
            "Tidally locked: the west third of the grid is permanent day, the middle \
             third is terminator twilight, the east third never sees the sun."
                .to_string(),
        );
    }
    if mods.carnivorous_flora {
        lines.push(format!(
            "Carnivorous flora: strangling vines deal {CARNIVOROUS_FLORA_DAMAGE:.1} damage/tick \
             to pops on or beside them."
        ));
    }
    lines.join("\n")
}

/// Chronicle text announcing a world's quirks at game start, when it has
/// any. `None` for unremarkable worlds (no chronicle spam).
#[must_use]
pub fn quirks_chronicle_text(traits: &[PlanetaryTrait]) -> Option<String> {
    if traits.is_empty() {
        return None;
    }
    let labels: Vec<&str> = traits.iter().map(|q| q.label()).collect();
    Some(format!(
        "Surveyors file the landing report: this world is marked by {}. \
         The colony will have to adapt.",
        labels.join(", ")
    ))
}

/// Adds a quirk by player-typed name to the world's `PlanetaryTraits`
/// (creating the resource if needed), refreshes `GlobalModifiers` when
/// present, and returns the quirk added. Adding an active quirk again
/// is a no-op success. Backs the headless `quirk <name>` command.
///
/// # Errors
/// Returns the list of valid names when `raw` does not parse.
pub fn add_quirk_by_name(world: &mut World, raw: &str) -> Result<PlanetaryTrait, String> {
    let Some(quirk) = PlanetaryTrait::parse(raw) else {
        let valid: Vec<&str> = PlanetaryTrait::ALL.iter().map(|q| q.name()).collect();
        return Err(format!(
            "unknown quirk '{raw}' — valid: {}",
            valid.join(", ")
        ));
    };
    if !world.contains_resource::<PlanetaryTraits>() {
        world.insert_resource(PlanetaryTraits::default());
    }
    {
        let mut traits = world.resource_mut::<PlanetaryTraits>();
        if !traits.0.contains(&quirk) {
            traits.0.push(quirk);
        }
    }
    if world.contains_resource::<GlobalModifiers>() {
        let fresh = {
            let traits = world.resource::<PlanetaryTraits>();
            GlobalModifiers::from_traits(&traits.0)
        };
        *world.resource_mut::<GlobalModifiers>() = fresh;
    }
    Ok(quirk)
}

/// Removes every quirk from the world (headless `quirk clear`) and
/// refreshes `GlobalModifiers` when present.
pub fn clear_quirks(world: &mut World) {
    if world.contains_resource::<PlanetaryTraits>() {
        world.resource_mut::<PlanetaryTraits>().0.clear();
    }
    if world.contains_resource::<GlobalModifiers>() {
        *world.resource_mut::<GlobalModifiers>() = GlobalModifiers::default();
    }
}

/// System that applies modifiers from planetary traits to simulation components.
///
/// This affects:
/// - `Speed`: Multiplies `current` speed.
/// - `DayNightCycle`: Sets `ticks_per_day` based on base value (250).
/// - `PowerSource`: Sets `output` for Generators based on base value (10.0).
/// - `AtmosphereGrid`: Sets `diffusion_rate` based on base value (0.99).
pub fn apply_quirk_modifiers_system(
    traits: Res<PlanetaryTraits>,
    mut pops: Query<&mut Speed>,
    mut day_night: ResMut<DayNightCycle>,
    mut power_sources: Query<(&mut PowerSource, &Building)>,
    atmosphere: Option<ResMut<AtmosphereGrid>>,
    global: Option<ResMut<GlobalModifiers>>,
) {
    // Constants defined at top of scope to appease clippy
    const BASE_TICKS_PER_DAY: f32 = 250.0;
    const BASE_GENERATOR_OUTPUT: f32 = 10.0;
    const BASE_DIFFUSION_RATE: f32 = 0.99;

    // Spec 345 REFACTOR: aggregation lives in one place — GlobalModifiers —
    // and this system also keeps that resource fresh for other consumers.
    let mods = GlobalModifiers::from_traits(&traits.0);
    if let Some(mut global) = global {
        if *global != mods {
            *global = mods.clone();
        }
    }

    if traits.0.is_empty() {
        return;
    }

    let speed_mod = mods.movement_speed;
    let day_mod = mods.day_length;
    let power_mod = mods.power_output;
    let diffusion_mod = mods.diffusion;

    // 2. Apply to Pops (Speed)
    // We multiply `current` instead of setting from `base` to respect other modifiers (like Lighting).
    for mut speed in &mut pops {
        speed.current *= speed_mod;
    }

    // 3. Apply to DayNightCycle
    // We use a hardcoded base value because DayNightCycle does not store one.
    // Default is 250 ticks per day.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        day_night.ticks_per_day = (BASE_TICKS_PER_DAY * day_mod) as u64;
    }

    // 4. Apply to Power Sources
    // Only affect Generators.
    // We use a hardcoded base value (10.0) because PowerSource does not store one.
    for (mut source, building) in &mut power_sources {
        if matches!(building.building_type, BuildingType::Generator) {
            source.output = BASE_GENERATOR_OUTPUT * power_mod;
        }
    }

    // 5. Apply to AtmosphereGrid
    if let Some(mut grid) = atmosphere {
        grid.diffusion_rate = (BASE_DIFFUSION_RATE * diffusion_mod).clamp(0.0, 1.0);
    }
}
