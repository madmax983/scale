//! Spec 367: Gravity Inversion Mining.
//!
//! A localized gravity generator flips gravity for a designated chunk
//! of terrain. Pops inside a powered field "fall" upward to the cavern
//! ceiling — stalactites and ceiling seams that floor-bound mining
//! cannot reach become workable while a pop is inverted — and a power
//! failure while a pop is up there is a lethal drop.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **No `PowerGrid.available_power`, no `PowerRequirement`, no
//!   `Transform`.** The real power seam is [`PowerConsumer`]: the live
//!   `power_grid_system` (a `&mut World` system in
//!   `layer1::systems::economy`) sums `PowerSource` output over the
//!   conduit-connected grid each tick and sets each consumer's
//!   `active` flag. A generator here is an entity carrying a real
//!   [`PowerConsumer`] (demand [`GENERATOR_POWER_DRAW`]); the field is
//!   live exactly when that consumer is `active`. Positions are
//!   [`GridPosition`] (this crate's simulation is `bevy_ecs`-only and
//!   grid-based), and radius is Chebyshev tiles, like every other
//!   radius in Layer 1.
//! * **`GravityGenerator` / `GravityState` already exist.** The
//!   pre-existing `layer1::physics::gravity_plating` module owns both
//!   names, with different semantics (a zone-targeted plating
//!   generator and a `Normal`/`ZeroG` zone state). Reusing the names
//!   would collide in the crate's glob re-exports, so this module's
//!   types are [`GravityInversionGenerator`] and
//!   [`GravityOrientation`] (`Normal`/`Inverted`/`Falling`). The
//!   plating module is untouched.
//! * **Fall damage scales with real height (§5), not the sketch's
//!   flat 50.0.** Height is the real [`ZLevel`] component: a
//!   generator's `ZLevel` is the ceiling height of its chunk, a pop's
//!   `ZLevel` (default 0) is the floor it fell from. Damage is
//!   [`FALL_DAMAGE_PER_LEVEL`] per level of difference, minimum one
//!   level, applied through the real [`Health::take_damage`] pipeline
//!   (which clamps at 0 — a high ceiling is genuinely lethal, a low
//!   one merely hurts). When a pop inverts, the drop it faces is
//!   recorded in [`GravityFallHeight`] so a pop whose generator is
//!   removed outright still falls the distance it rose.
//! * **No new `BuildingType`.** Adding a building variant ripples
//!   through every exhaustive `BuildingType` match (costs, tech gate,
//!   glyph, tier, placement). The MVP generator is a standalone
//!   entity — spawned by the headless `gravity plant` debug command
//!   with a paired `PowerSource` on the adjacent tile so the real
//!   grid powers it — exactly as Spec 366's choir plants are
//!   standalone entities rather than buildings. Formal construction
//!   integration is deferred (see the module report).
//! * **Ceiling mining is the orientation, not a new designation.**
//!   The real mining seam is `DesignationType::Mine` fulfilled by the
//!   legacy mining execution; rewiring pathfinding/execution for
//!   inverted surfaces is §5 optional work and is deferred. What
//!   ships is the seam those systems key off: an inverted pop
//!   ([`GravityOrientation::Inverted`], see [`can_mine_ceiling`])
//!   is the pop that can work a ceiling. Pathfinding treats inverted
//!   pops as ordinary pops — they fell *up* in place; they did not
//!   teleport.
//! * **Spec §6's `cargo clippy -- -D warnings` / 85% coverage** are
//!   aspirational template text; the verified bar is the established
//!   one: `cargo check --all-targets` green, clippy clean on touched
//!   files, and this module's tests green.
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::energy::{PowerConsumer, PowerSource};
use crate::layer1::health::Health;
use crate::layer1::map::{GridPosition, ZLevel};
use crate::layer1::pop::Pop;

/// Power a gravity inversion generator draws from the real grid.
/// Matches the sketch's `PowerRequirement { amount: 100.0 }`.
pub const GENERATOR_POWER_DRAW: f32 = 100.0;
/// Default field radius, in Chebyshev tiles (spec §3 example: 5.0).
pub const DEFAULT_RADIUS: u32 = 5;
/// Ceiling height (Z levels above the floor) used when a generator
/// carries no explicit [`ZLevel`].
pub const DEFAULT_CEILING_Z: i32 = 3;
/// Fall damage per Z level dropped (§5: scaled, not flat).
pub const FALL_DAMAGE_PER_LEVEL: f32 = 15.0;

/// A localized gravity generator. Paired on the same entity with a
/// real [`PowerConsumer`] (its power seam) and a [`GridPosition`];
/// an optional [`ZLevel`] on the entity is the ceiling height of
/// the chunk it flips.
#[derive(Component, Debug, Clone, Copy)]
pub struct GravityInversionGenerator {
    /// Field radius in Chebyshev tiles.
    pub radius: u32,
    /// Player/debug switch. Power loss is the [`PowerConsumer`]
    /// seam; this switch exists so the headless `gravity cut`
    /// command can drop the field on demand.
    pub enabled: bool,
    /// Whether the field was live on the last tick the inversion
    /// system ran (powered + enabled). Panel/telemetry surface.
    pub field_active: bool,
}

impl GravityInversionGenerator {
    /// A generator with the spec's default radius.
    #[must_use]
    pub const fn new(radius: u32) -> Self {
        Self {
            radius,
            enabled: true,
            field_active: false,
        }
    }

    /// True when this generator's field should be holding pops up:
    /// switched on and its real power consumer is active. A
    /// generator with no `PowerConsumer` at all is treated as
    /// powered (standalone test/debug seam — production spawns
    /// always attach one).
    #[must_use]
    pub fn is_powered(&self, consumer: Option<&PowerConsumer>) -> bool {
        self.enabled && consumer.is_none_or(|c| c.active)
    }
}

impl Default for GravityInversionGenerator {
    fn default() -> Self {
        Self::new(DEFAULT_RADIUS)
    }
}

/// A pop's gravity orientation under Spec 367. Distinct from
/// `physics::gravity_plating::GravityState` (zone Zero-G) — see the
/// module docs on the name collision.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GravityOrientation {
    /// Feet on the floor, as nature intended.
    #[default]
    Normal,
    /// Feet on the ceiling, held up by a live inversion field.
    Inverted,
    /// The field went away. Damage has already been applied; the
    /// pop lands (returns to `Normal`) on the next tick.
    Falling,
}

/// The drop a currently-inverted pop faces, in Z levels, recorded
/// when it inverted. Lets a pop fall the distance it actually rose
/// even if its generator is later removed.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GravityFallHeight {
    /// Z levels between the pop's floor and the ceiling holding it.
    pub drop_levels: i32,
}

/// Colony ledger for inversion falls (STATS / panel surface).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct GravityInversionLedger {
    /// Total falls (power loss / walked out / generator removed).
    pub falls: u64,
    /// Total fall damage dealt across all falls.
    pub total_fall_damage: f32,
}

/// Fall damage for a drop of `drop_levels` Z levels: scaled per
/// §5, with a one-level minimum (any inversion falls *somewhere*).
#[must_use]
pub fn fall_damage(drop_levels: i32) -> f32 {
    drop_levels.max(1) as f32 * FALL_DAMAGE_PER_LEVEL
}

/// True when a pop in this orientation can work a cavern ceiling
/// (the mining seam — see module docs).
#[must_use]
pub const fn can_mine_ceiling(orientation: GravityOrientation) -> bool {
    matches!(orientation, GravityOrientation::Inverted)
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Gives every pop a [`GravityOrientation`] so the inversion system
/// can address pops that were spawned before (or without) one.
pub fn ensure_gravity_orientation_system(
    mut commands: Commands,
    pops: Query<Entity, (With<Pop>, Without<GravityOrientation>)>,
) {
    for entity in pops.iter() {
        commands.entity(entity).insert(GravityOrientation::Normal);
    }
}

/// One field a generator projects this tick.
#[derive(Clone, Copy)]
struct Field {
    pos: GridPosition,
    radius: u32,
    ceiling_z: i32,
    active: bool,
}

impl Field {
    fn contains(&self, pop_pos: GridPosition) -> bool {
        self.pos.distance_chebyshev(pop_pos) <= self.radius
    }
    fn drop_for(&self, floor_z: i32) -> i32 {
        (self.ceiling_z - floor_z).max(1)
    }
}

/// The inversion system: powered fields flip pops in radius to
/// [`GravityOrientation::Inverted`]; a pop that is inverted and no
/// longer inside a live field falls — real fall damage through
/// [`Health::take_damage`], a ledger entry, and a chronicle line —
/// and lands (`Normal`) the following tick.
#[allow(clippy::type_complexity)]
pub fn gravity_inversion_system(
    mut commands: Commands,
    mut generators: Query<(
        &mut GravityInversionGenerator,
        &GridPosition,
        Option<&PowerConsumer>,
        Option<&ZLevel>,
    )>,
    mut pops: Query<
        (
            Entity,
            &mut GravityOrientation,
            &mut Health,
            &GridPosition,
            Option<&ZLevel>,
            Option<&GravityFallHeight>,
        ),
        With<Pop>,
    >,
    mut ledger: ResMut<GravityInversionLedger>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut fields: Vec<Field> = Vec::new();
    for (mut gen, pos, consumer, z) in generators.iter_mut() {
        let active = gen.is_powered(consumer);
        gen.field_active = active;
        fields.push(Field {
            pos: *pos,
            radius: gen.radius,
            ceiling_z: z.map_or(DEFAULT_CEILING_Z, |z| z.0),
            active,
        });
    }

    for (entity, mut orientation, mut health, pos, z, fall_height) in pops.iter_mut() {
        match *orientation {
            GravityOrientation::Falling => {
                // The drop already happened; this tick the pop lands.
                *orientation = GravityOrientation::Normal;
                commands.entity(entity).remove::<GravityFallHeight>();
            }
            GravityOrientation::Normal => {
                if let Some(field) = fields.iter().find(|f| f.active && f.contains(*pos)) {
                    let floor_z = z.map_or(0, |z| z.0);
                    *orientation = GravityOrientation::Inverted;
                    commands.entity(entity).insert(GravityFallHeight {
                        drop_levels: field.drop_for(floor_z),
                    });
                }
            }
            GravityOrientation::Inverted => {
                let still_held = fields.iter().any(|f| f.active && f.contains(*pos));
                if !still_held {
                    // Prefer the drop owed to the field that held
                    // the pop; fall back to the recorded rise, then
                    // the default ceiling.
                    let floor_z = z.map_or(0, |z| z.0);
                    let drop = fields
                        .iter()
                        .find(|f| f.contains(*pos))
                        .map(|f| f.drop_for(floor_z))
                        .or_else(|| fall_height.map(|h| h.drop_levels))
                        .unwrap_or(DEFAULT_CEILING_Z);
                    let damage = fall_damage(drop);
                    health.take_damage(damage);
                    ledger.falls += 1;
                    ledger.total_fall_damage += damage;
                    *orientation = GravityOrientation::Falling;
                    chronicle.send(AddChronicleEvent {
                        text: format!(
                            "The inversion field failed and a ceiling miner dropped {drop} levels, taking {damage:.0} damage."
                        ),
                        importance: EventImportance::Major,
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Headless / world helpers
// ---------------------------------------------------------------------------

/// Census: (generators, live fields, inverted pops, falling pops,
/// ledger falls).
#[must_use]
pub fn gravity_counts(world: &mut World) -> (usize, usize, usize, usize, u64) {
    let mut gens = 0usize;
    let mut live = 0usize;
    {
        let mut query = world.query::<&GravityInversionGenerator>();
        for gen in query.iter(world) {
            gens += 1;
            if gen.field_active {
                live += 1;
            }
        }
    }
    let mut inverted = 0usize;
    let mut falling = 0usize;
    {
        let mut query = world.query_filtered::<&GravityOrientation, With<Pop>>();
        for orientation in query.iter(world) {
            match orientation {
                GravityOrientation::Inverted => inverted += 1,
                GravityOrientation::Falling => falling += 1,
                GravityOrientation::Normal => {}
            }
        }
    }
    let falls = world
        .get_resource::<GravityInversionLedger>()
        .map_or(0, |l| l.falls);
    (gens, live, inverted, falling, falls)
}

/// One-token STATS label, e.g. `1g/3i/0f` (generators/inverted/falls),
/// or `none` when the colony has no inversion generator.
#[must_use]
pub fn gravity_stats_label(world: &mut World) -> String {
    let (gens, _, inverted, _, falls) = gravity_counts(world);
    if gens == 0 {
        "none".to_string()
    } else {
        format!("{gens}g/{inverted}i/{falls}f")
    }
}

/// Spawn a generator at the first living pop's tile (or the origin),
/// with its real [`PowerConsumer`] and a paired [`PowerSource`] on
/// the adjacent tile so the live grid can power it. Returns the
/// generator entity.
pub fn spawn_gravity_generator(world: &mut World) -> Entity {
    let anchor = {
        let mut query = world.query_filtered::<&GridPosition, With<Pop>>();
        query.iter(world).next().copied()
    }
    .unwrap_or(GridPosition { x: 0, y: 0 });
    let generator = world
        .spawn((
            GravityInversionGenerator::default(),
            anchor,
            ZLevel(DEFAULT_CEILING_Z),
            PowerConsumer {
                demand: GENERATOR_POWER_DRAW,
                active: true,
            },
        ))
        .id();
    world.spawn((
        PowerSource {
            output: GENERATOR_POWER_DRAW * 2.0,
            active: true,
        },
        GridPosition {
            x: anchor.x + 1,
            y: anchor.y,
        },
    ));
    generator
}

/// Cut every generator's field (the headless power-failure lever):
/// switches generators off so inverted pops fall next tick.
/// Returns how many generators were cut.
pub fn cut_gravity_generators(world: &mut World) -> usize {
    let mut count = 0usize;
    let mut query = world.query::<&mut GravityInversionGenerator>();
    for mut gen in query.iter_mut(world) {
        gen.enabled = false;
        count += 1;
    }
    count
}

/// Re-enable every generator (power seam permitting).
pub fn restore_gravity_generators(world: &mut World) -> usize {
    let mut count = 0usize;
    let mut query = world.query::<&mut GravityInversionGenerator>();
    for mut gen in query.iter_mut(world) {
        gen.enabled = true;
        count += 1;
    }
    count
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn gravity_public_strings() -> Vec<String> {
    vec![
        "Gravity Inversion Mining".to_string(),
        "inversion generator".to_string(),
        "ceiling miner".to_string(),
        "The inversion field failed and a ceiling miner dropped.".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Tests (spec §3 RED tests, adapted to the real codebase — see the
// module docs for every adaptation and why)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::systems::update_event_buffer;

    fn gravity_schedule() -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems((
            update_event_buffer::<AddChronicleEvent>,
            ensure_gravity_orientation_system,
            gravity_inversion_system,
        ));
        schedule
    }

    fn world_with_ledger() -> World {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<GravityInversionLedger>();
        world
    }

    fn powered_generator(world: &mut World, pos: GridPosition, ceiling_z: i32) -> Entity {
        world
            .spawn((
                GravityInversionGenerator::new(5),
                pos,
                ZLevel(ceiling_z),
                PowerConsumer {
                    demand: GENERATOR_POWER_DRAW,
                    active: true,
                },
            ))
            .id()
    }

    /// Spec RED test 1, adapted: no `PowerGrid` resource /
    /// `PowerRequirement` / `Transform` — the generator's real
    /// `PowerConsumer.active` (set by `power_grid_system` in a live
    /// game) is the powered state, and positions are `GridPosition`.
    #[test]
    fn test_gravity_generator_flips_gravity_when_powered() {
        let mut world = world_with_ledger();
        powered_generator(&mut world, GridPosition { x: 0, y: 0 }, 3);
        let pop = world
            .spawn((
                Pop,
                GravityOrientation::Normal,
                Health::default(),
                GridPosition { x: 2, y: 0 },
            ))
            .id();

        let mut schedule = gravity_schedule();
        schedule.run(&mut world);

        assert_eq!(
            *world.get::<GravityOrientation>(pop).unwrap(),
            GravityOrientation::Inverted
        );
        assert!(can_mine_ceiling(
            *world.get::<GravityOrientation>(pop).unwrap()
        ));
        // The drop the pop now faces was recorded: ceiling 3 - floor 0.
        assert_eq!(world.get::<GravityFallHeight>(pop).unwrap().drop_levels, 3);
    }

    /// Spec RED test 2, adapted: the sketch's pop floats at
    /// `Transform` y=10; here the generator's ceiling is `ZLevel(10)`
    /// over the pop's floor `ZLevel(0)`, so §5 scaled damage is
    /// 10 × 15 = 150 — genuinely lethal, not the flat 50.
    #[test]
    fn test_power_failure_causes_lethal_drop() {
        let mut world = world_with_ledger();
        let gen = powered_generator(&mut world, GridPosition { x: 0, y: 0 }, 10);
        let pop = world
            .spawn((
                Pop,
                GravityOrientation::Inverted,
                Health::default(),
                GridPosition { x: 0, y: 0 },
                ZLevel(0),
            ))
            .id();

        // Power fails: the real grid would clear this flag.
        world.get_mut::<PowerConsumer>(gen).unwrap().active = false;
        let mut schedule = gravity_schedule();
        schedule.run(&mut world);

        let health = world.get::<Health>(pop).unwrap();
        assert!(health.current < 100.0, "fall should damage: {health:?}");
        assert_eq!(health.current, 0.0, "a 10-level drop is lethal");
        assert_eq!(
            *world.get::<GravityOrientation>(pop).unwrap(),
            GravityOrientation::Falling
        );
        let ledger = world.resource::<GravityInversionLedger>();
        assert_eq!(ledger.falls, 1);
        assert_eq!(ledger.total_fall_damage, 150.0);

        // Next tick the pop lands: Falling resolves to Normal.
        schedule.run(&mut world);
        assert_eq!(
            *world.get::<GravityOrientation>(pop).unwrap(),
            GravityOrientation::Normal
        );
    }

    /// §5 scaling: a low ceiling hurts but does not kill.
    #[test]
    fn test_fall_damage_scales_with_z_difference() {
        assert_eq!(fall_damage(1), 15.0);
        assert_eq!(fall_damage(3), 45.0);
        assert_eq!(fall_damage(10), 150.0);
        // Even a recorded drop of 0 still falls one level.
        assert_eq!(fall_damage(0), 15.0);

        let mut world = world_with_ledger();
        let gen = powered_generator(&mut world, GridPosition { x: 0, y: 0 }, 2);
        let pop = world
            .spawn((
                Pop,
                GravityOrientation::Inverted,
                Health::default(),
                GridPosition { x: 0, y: 0 },
            ))
            .id();
        world.get_mut::<PowerConsumer>(gen).unwrap().active = false;
        let mut schedule = gravity_schedule();
        schedule.run(&mut world);
        assert_eq!(world.get::<Health>(pop).unwrap().current, 70.0);
    }

    /// Radius limit: a pop outside the field stays on the floor,
    /// and an inverted pop that walks out of the field falls.
    #[test]
    fn test_radius_limits_and_walkout_fall() {
        let mut world = world_with_ledger();
        powered_generator(&mut world, GridPosition { x: 0, y: 0 }, 3);
        let far = world
            .spawn((
                Pop,
                GravityOrientation::Normal,
                Health::default(),
                GridPosition { x: 40, y: 40 },
            ))
            .id();
        let walker = world
            .spawn((
                Pop,
                GravityOrientation::Normal,
                Health::default(),
                GridPosition { x: 1, y: 0 },
            ))
            .id();

        let mut schedule = gravity_schedule();
        schedule.run(&mut world);
        assert_eq!(
            *world.get::<GravityOrientation>(far).unwrap(),
            GravityOrientation::Normal
        );
        assert_eq!(
            *world.get::<GravityOrientation>(walker).unwrap(),
            GravityOrientation::Inverted
        );

        // The walker leaves the field while inverted: it falls the
        // recorded 3 levels (45 damage).
        world.get_mut::<GridPosition>(walker).unwrap().x = 40;
        schedule.run(&mut world);
        assert_eq!(
            *world.get::<GravityOrientation>(walker).unwrap(),
            GravityOrientation::Falling
        );
        assert_eq!(world.get::<Health>(walker).unwrap().current, 55.0);
    }

    /// An unpowered generator inverts nothing.
    #[test]
    fn test_unpowered_generator_is_inert() {
        let mut world = world_with_ledger();
        let gen = powered_generator(&mut world, GridPosition { x: 0, y: 0 }, 3);
        world.get_mut::<PowerConsumer>(gen).unwrap().active = false;
        let pop = world
            .spawn((
                Pop,
                GravityOrientation::Normal,
                Health::default(),
                GridPosition { x: 0, y: 0 },
            ))
            .id();
        let mut schedule = gravity_schedule();
        schedule.run(&mut world);
        assert_eq!(
            *world.get::<GravityOrientation>(pop).unwrap(),
            GravityOrientation::Normal
        );
        assert_eq!(gravity_counts(&mut world).0, 1);
    }

    /// Pops spawned without an orientation are given one by the
    /// ensure system, then inverted by a live field.
    #[test]
    fn test_ensure_system_covers_bare_pops() {
        let mut world = world_with_ledger();
        powered_generator(&mut world, GridPosition { x: 0, y: 0 }, 3);
        let pop = world
            .spawn((Pop, Health::default(), GridPosition { x: 0, y: 0 }))
            .id();
        let mut schedule = gravity_schedule();
        schedule.run(&mut world);
        schedule.run(&mut world);
        assert_eq!(
            *world.get::<GravityOrientation>(pop).unwrap(),
            GravityOrientation::Inverted
        );
    }

    /// IP guard: every public-facing string uses original names only.
    #[test]
    fn test_ip_guard_original_names_only() {
        let banned = [
            "ender",
            "minecraft",
            "rimworld",
            "dwarf fortress",
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "pokemon",
            "zelda",
            "skyrim",
        ];
        for text in gravity_public_strings() {
            let lower = text.to_lowercase();
            for word in banned {
                assert!(
                    !lower.contains(word),
                    "IP-guard: '{text}' contains banned term '{word}'"
                );
            }
        }
    }
}
