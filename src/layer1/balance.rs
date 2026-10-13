//! Game balance constants.

/// Number of ticks per in-game year.
pub const TICKS_PER_YEAR: u64 = 1000;

/// Food produced per worker per tick in a Farm.
pub const FOOD_PER_WORKER_PER_TICK: f32 = 0.005;

/// Hunger level below which a Pop will seek food.
pub const FOOD_HUNGER_THRESHOLD: f32 = 0.4;

/// Amount of food consumed per meal.
pub const FOOD_PER_MEAL: f32 = 0.1;

/// Amount of hunger restored per meal.
pub const HUNGER_PER_MEAL: f32 = 0.3;

/// Food production modifier for Spring.
pub const SEASON_MODIFIER_SPRING: f32 = 1.0;

/// Food production modifier for Summer.
pub const SEASON_MODIFIER_SUMMER: f32 = 1.2;

/// Food production modifier for Autumn.
pub const SEASON_MODIFIER_AUTUMN: f32 = 1.5;

/// Food production modifier for Winter.
pub const SEASON_MODIFIER_WINTER: f32 = 0.5;

/// Movement speed modifier for Spring (mud) — Spec 344.
pub const SEASON_MOVE_MODIFIER_SPRING: f32 = 0.85;

/// Movement speed modifier for Summer (dry ground) — Spec 344.
pub const SEASON_MOVE_MODIFIER_SUMMER: f32 = 1.0;

/// Movement speed modifier for Autumn (firm ground) — Spec 344.
pub const SEASON_MOVE_MODIFIER_AUTUMN: f32 = 0.95;

/// Movement speed modifier for Winter (snow) — Spec 344.
pub const SEASON_MOVE_MODIFIER_WINTER: f32 = 0.6;

/// Crop growth-rate modifier for Spring — Spec 344.
pub const SEASON_GROWTH_MODIFIER_SPRING: f32 = 1.25;

/// Crop growth-rate modifier for Summer — Spec 344.
pub const SEASON_GROWTH_MODIFIER_SUMMER: f32 = 1.5;

/// Crop growth-rate modifier for Autumn — Spec 344.
pub const SEASON_GROWTH_MODIFIER_AUTUMN: f32 = 1.0;

/// Crop growth-rate modifier for Winter (near-dormant) — Spec 344.
pub const SEASON_GROWTH_MODIFIER_WINTER: f32 = 0.1;

/// Age in ticks when a Pop becomes an Adult.
pub const AGE_ADULT: u64 = 18 * TICKS_PER_YEAR;

/// Age in ticks when a Pop becomes an Elder.
pub const AGE_ELDER: u64 = 60 * TICKS_PER_YEAR;
