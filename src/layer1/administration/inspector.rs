//! The Inspector — colony evaluation visitors.
//!
//! Two generations live in this module:
//!
//! * **Legacy (pre-410):** a periodic inspector visitor
//!   ([`InspectorSource`] cadence) samples the [`BeautyGrid`] into
//!   `beauty_score` and files a notification grade on departure
//!   ([`inspector_report_system`] + [`Reported`]). Unchanged by
//!   spec 410; its tests below are the originals.
//! * **Spec 410 "The Inspector":** a high-ranking dignitary
//!   (Inspector / Ambassador / Auditor — generic titles only) arrives
//!   via shuttle on [`SpawnInspectorEvent`], tours high-value /
//!   high-traffic stops, their `mood` is shaped by what they see
//!   (decor/beauty, food quality, squalor, corpse piles), and their
//!   exit report buffs/debuffs Layer-3 faction relations and funding.
//!   The tension: restrict their movement (insulting) vs risk them
//!   seeing the ugly truth (risky).
//!
//! # Grounding audit (spec §8 adaptation pattern, specs 360–409)
//!
//! The spec's sketch types were audited against the real codebase:
//!
//! * `Inspector { base_mood }` + `Transform` — **PARTLY REAL.** An
//!   `Inspector` component already existed HERE (beauty sampler), so
//!   spec 410 EXTENDS it in place — no parallel component. There are
//!   no `Transform`-positioned pops in this codebase: the real spatial
//!   seam is [`GridPosition`]. The spec's `base_mood` arrives as the
//!   new `mood` field (0.0..=100.0, start [`START_MOOD`]).
//! * `Corpse` marker (spec sketch) — **MOCK.** The real corpse is
//!   [`crate::layer1::culture::funeral::Corpse`] (`name` + `decay`),
//!   spawned by `handle_pop_death_system` with a `GridPosition`.
//!   Mood observation counts each real corpse once (per-entity dedup).
//! * `FactionId::Empire` / `FactionRelation(FactionId, f32)` /
//!   `FactionRelations { empire: f32 }` (spec sketch) — **MOCK.** The
//!   real Layer-3 seam is
//!   [`crate::layer3::diplomacy::system_sovereignty::FactionRelations`]:
//!   `scores: HashMap<u32, i32>` on the documented −100 Hostile ..
//!   100 Allied scale (the seam specs 1208/396/402 already write from
//!   Layer 1). The dignitary answers to [`INSPECTOR_FACTION_ID`] (24 —
//!   the next free id after 21 marriage / 22 nomadic / 23
//!   conservationist). No named empire anywhere; titles are generic.
//! * Visitor lifecycle — **REAL** via [`Visitor`] + [`VisitorSource`]
//!   + [`VisitorState`] (this module's legacy spawn already rides it;
//!   spec 408 tourists ride it too). Dignitaries spawn WITHOUT
//!   `UtilityWeights`, so the utility AI never assigns them jobs.
//! * Beauty / squalor — **REAL** via the spatial [`BeautyGrid`]:
//!   positive tiles are decor/gardens/statues, negative tiles are
//!   squalor (dirt, paths, waste, clutter all feed the grid). The
//!   legacy system accumulates it as a grade; spec 410 also samples
//!   it into mood. No separate `Squalor` component was invented.
//! * Food quality — **NO dedicated seam exists.** The only
//!   meal-quality signal in the codebase is the `AteFineMeal` memory.
//!   The honest bridge is the shared stockpile itself (the spec 408
//!   tourism dilemma seam): mid-tour the dignitary samples the colony
//!   table — a stocked stockpile reads as a fine meal, an empty one
//!   as a missed meal. No parallel food-quality economy.
//! * Zone designation (High Value / High Traffic) — **PARTLY MOCK.**
//!   `ZoneGrid`/`ZoneType` are real but carry no HighValue/HighTraffic
//!   variant, and adding one would churn persistence + designation
//!   for a derived concept: high-value IS the highest-[`BeautyGrid`]
//!   tile, high-traffic is proxied by the colony heart (map centre,
//!   where the lander / stockpile cluster draws every pop). Tour
//!   stops are planned from those real signals by
//!   [`plan_tour_targets`].
//! * Pathfinding — **REAL but heavyweight** (A* built for utility-AI
//!   pops). The dignitary procession steps one tile per tick toward
//!   its stop (`GridPosition::direction_to`); if something pins the
//!   dignitary in place past [`STUCK_THRESHOLD`] ticks, the spec §7
//!   trapped rule fires: mood plummets until they move again.
//! * Legacy vs dignitary: the periodic ambient inspector keeps its
//!   notification-grade report untouched (`dignitary == false`).
//!   Only event-landed dignitaries tour, accumulate mood legs, and
//!   file the Layer-3 exit report — zero legacy behavior drift.
//!
//! # New constants (no existing balance constants were retuned)
//!
//! * [`START_MOOD`] `50.0`, [`CORPSE_MOOD_HIT`] `-10.0` (spec RED),
//!   [`CORPSE_SIGHT_RADIUS`] `5` (spec GREEN sketch),
//!   [`GLOWING_THRESHOLD`] `70.0` / [`DAMNING_THRESHOLD`] `30.0` with
//!   [`RELATION_GLOWING`] `+10` / [`RELATION_DAMNING`] `-10`
//!   (spec GREEN), [`FUNDING_GRANT`] `250.0`,
//!   [`INSPECTOR_STAY_TICKS`] `600`.

use crate::layer1::beauty::BeautyGrid;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::culture::funeral::Corpse;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::map::GridPosition;
use crate::layer1::notifications::{NotificationQueue, NotificationSeverity};
use crate::layer1::visitor::{Visitor, VisitorSource, VisitorState};
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Layer-3 faction id the dignitary reports to in
/// [`FactionRelations`] (next free id after 21/22/23).
pub const INSPECTOR_FACTION_ID: u32 = 24;
/// Mood every dignitary arrives with (spec RED test baseline).
pub const START_MOOD: f32 = 50.0;
/// Mood lost per distinct corpse sighted (spec RED/GREEN value).
pub const CORPSE_MOOD_HIT: f32 = -10.0;
/// Chebyshev radius at which a corpse is sighted.
pub const CORPSE_SIGHT_RADIUS: u32 = 5;
/// Ticks between beauty/squalor mood samples of the current tile.
pub const BEAUTY_SAMPLE_INTERVAL: u64 = 25;
/// Mood gained per beauty point at a sampled tile (beauty 10 -> +2).
pub const BEAUTY_MOOD_FACTOR: f32 = 0.2;
/// Beauty at or below this at a sampled tile counts as a squalor hit.
pub const SQUALOR_THRESHOLD: f32 = -2.0;
/// Mood at/above which the exit report glows (spec GREEN threshold).
pub const GLOWING_THRESHOLD: f32 = 70.0;
/// Mood at/below which the exit report damns (spec GREEN threshold).
pub const DAMNING_THRESHOLD: f32 = 30.0;
/// Relations delta of a glowing report (spec GREEN value).
pub const RELATION_GLOWING: i32 = 10;
/// Relations delta of a damning report (spec GREEN value).
pub const RELATION_DAMNING: i32 = -10;
/// Extra relations penalty when the tour was restricted: being
/// stage-managed insults the sending authority too.
pub const RESTRICTED_RELATION_PENALTY: i32 = -5;
/// Mood hit applied once at arrival when the tour is restricted.
pub const INSULT_MOOD_HIT: f32 = -5.0;
/// Credits granted to the real colony ledger by a glowing report.
pub const FUNDING_GRANT: f32 = 250.0;
/// How long a dignitary tours before departing.
pub const INSPECTOR_STAY_TICKS: u64 = 600;
/// Stockpile food at/above which the mid-tour table sample is a
/// fine meal; an empty stockpile is a missed meal (in between: plain).
pub const MEAL_FOOD_THRESHOLD: f32 = 10.0;
/// Mood gained from a fine mid-tour meal.
pub const MEAL_GOOD_MOOD: f32 = 5.0;
/// Mood lost from a missed mid-tour meal.
pub const MEAL_MISSED_MOOD: f32 = -5.0;
/// Ticks without movement (with a stop still pending) before the
/// dignitary counts as trapped (spec §7 gotcha).
pub const STUCK_THRESHOLD: u64 = 100;
/// Mood lost per tick while trapped past [`STUCK_THRESHOLD`].
pub const TRAPPED_MOOD_RATE: f32 = -1.0;
/// Maximum tour stops on an open tour.
pub const MAX_TOUR_STOPS: usize = 3;
/// Chronicle text stem for a glowing report.
pub const REPORT_GLOWING_TEXT: &str = "The visiting dignitary's report praises the colony.";
/// Chronicle text stem for a neutral report.
pub const REPORT_NEUTRAL_TEXT: &str = "The visiting dignitary's report is measured and noncommittal.";
/// Chronicle text stem for a damning report.
pub const REPORT_DAMNING_TEXT: &str = "The visiting dignitary's report condemns the colony.";

/// Generic dignitary titles (original/generic names only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorTitle {
    /// The default: an Inspector.
    #[default]
    Inspector,
    /// An Ambassador.
    Ambassador,
    /// An Auditor.
    Auditor,
}

impl InspectorTitle {
    /// Player-facing title.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Inspector => "Inspector",
            Self::Ambassador => "Ambassador",
            Self::Auditor => "Auditor",
        }
    }

    /// Parse a headless title word (case-insensitive).
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word.to_ascii_lowercase().as_str() {
            "inspector" => Some(Self::Inspector),
            "ambassador" => Some(Self::Ambassador),
            "auditor" => Some(Self::Auditor),
            _ => None,
        }
    }
}

/// How much of the colony the dignitary is allowed to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TourAccess {
    /// The full tour: every planned stop, ugly truths included.
    #[default]
    Open,
    /// Curated tour: only the prettiest stop. Safer — and insulting.
    Restricted,
}

/// Verdict of an exit report, derived from final mood (spec GREEN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportVerdict {
    /// Mood at/above [`GLOWING_THRESHOLD`].
    Glowing,
    /// Mood between the thresholds.
    Neutral,
    /// Mood at/below [`DAMNING_THRESHOLD`].
    Damning,
}

impl ReportVerdict {
    /// Player-facing verdict word.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Glowing => "glowing",
            Self::Neutral => "neutral",
            Self::Damning => "damning",
        }
    }
}

/// The verdict a final mood earns.
#[must_use]
pub fn verdict_for_mood(mood: f32) -> ReportVerdict {
    if mood >= GLOWING_THRESHOLD {
        ReportVerdict::Glowing
    } else if mood <= DAMNING_THRESHOLD {
        ReportVerdict::Damning
    } else {
        ReportVerdict::Neutral
    }
}

/// Relations delta an exit report applies (before clamping to the
/// documented −100..=100 scale).
#[must_use]
pub fn relation_delta_for(verdict: ReportVerdict, restricted: bool) -> i32 {
    let base = match verdict {
        ReportVerdict::Glowing => RELATION_GLOWING,
        ReportVerdict::Neutral => 0,
        ReportVerdict::Damning => RELATION_DAMNING,
    };
    if restricted {
        base + RESTRICTED_RELATION_PENALTY
    } else {
        base
    }
}

/// What the dignitary has seen, accumulated over the tour. This is
/// the spec §5 `Report` struct: the exit report explains itself.
#[derive(Debug, Clone, Default)]
pub struct InspectorReport {
    /// Distinct corpses sighted.
    pub corpses_seen: u32,
    /// Beauty mood samples taken.
    pub beauty_samples: u32,
    /// Sum of sampled beauty values.
    pub beauty_total: f32,
    /// Samples at/below [`SQUALOR_THRESHOLD`].
    pub squalor_hits: u32,
    /// Fine mid-tour meals.
    pub fine_meals: u32,
    /// Missed mid-tour meals.
    pub missed_meals: u32,
    /// Whether the tour was restricted.
    pub restricted: bool,
    /// Ticks spent trapped past [`STUCK_THRESHOLD`].
    pub trapped_ticks: u64,
}

impl InspectorReport {
    /// One-line human summary of the report (chronicle / panel text).
    #[must_use]
    pub fn summary(&self) -> String {
        let access = if self.restricted { ", tour restricted" } else { "" };
        format!(
            "saw {} corpse(s), sampled beauty {:.1} over {} sample(s), {} squalor hit(s), {} fine meal(s), {} missed meal(s), {} trapped tick(s){}",
            self.corpses_seen,
            self.beauty_total,
            self.beauty_samples,
            self.squalor_hits,
            self.fine_meals,
            self.missed_meals,
            self.trapped_ticks,
            access
        )
    }
}

/// The Inspector component tracks the evaluation state of an inspector visitor.
///
/// The first three fields are the legacy beauty-sampler state. The
/// remaining fields are the spec 410 dignitary state; they stay at
/// their defaults for periodic ambient inspectors (`dignitary` is
/// false), which keeps every legacy consumer untouched.
#[derive(Component, Debug, Clone)]
pub struct Inspector {
    /// Accumulated beauty score observed.
    pub beauty_score: f32,
    /// Number of samples taken.
    pub samples_taken: u32,
    /// Next tick to take a sample.
    pub next_sample_tick: u64,
    /// True for spec 410 event-landed dignitaries (vs the periodic
    /// ambient inspector, which files only a notification grade).
    pub dignitary: bool,
    /// Generic title of the dignitary.
    pub title: InspectorTitle,
    /// Layer-3 faction id the exit report moves.
    pub faction_id: u32,
    /// Current mood, 0.0..=100.0 (spec sketch `base_mood`, renamed).
    pub mood: f32,
    /// Tick the dignitary arrived.
    pub arrival_tick: u64,
    /// Tick the dignitary departs and files the report.
    pub departure_tick: u64,
    /// Tour access granted by the player.
    pub access: TourAccess,
    /// Accumulated spec-410 observations.
    pub report: InspectorReport,
    /// Planned tour stops (high-value first, colony heart last).
    pub tour_targets: Vec<GridPosition>,
    /// Index of the current tour stop.
    pub tour_index: usize,
    /// Corpses already counted (each corpse insults only once).
    pub seen_corpses: Vec<Entity>,
    /// Whether the mid-tour table sample has happened.
    pub meal_sampled: bool,
    /// Position at the last observed movement.
    pub last_pos: GridPosition,
    /// Tick of the last observed movement.
    pub last_move_tick: u64,
}

impl Default for Inspector {
    fn default() -> Self {
        Self {
            beauty_score: 0.0,
            samples_taken: 0,
            next_sample_tick: 0,
            dignitary: false,
            title: InspectorTitle::Inspector,
            faction_id: INSPECTOR_FACTION_ID,
            mood: START_MOOD,
            arrival_tick: 0,
            departure_tick: 0,
            access: TourAccess::Open,
            report: InspectorReport::default(),
            tour_targets: Vec::new(),
            tour_index: 0,
            seen_corpses: Vec::new(),
            meal_sampled: false,
            last_pos: GridPosition { x: 0, y: 0 },
            last_move_tick: 0,
        }
    }
}

impl Inspector {
    /// A fresh spec 410 dignitary at the landing point.
    #[must_use]
    pub fn new(
        title: InspectorTitle,
        faction_id: u32,
        access: TourAccess,
        tick: u64,
        at: GridPosition,
        tour_targets: Vec<GridPosition>,
    ) -> Self {
        let restricted = access == TourAccess::Restricted;
        Self {
            dignitary: true,
            title,
            faction_id,
            mood: if restricted {
                START_MOOD + INSULT_MOOD_HIT
            } else {
                START_MOOD
            },
            arrival_tick: tick,
            departure_tick: tick + INSPECTOR_STAY_TICKS,
            access,
            report: InspectorReport {
                restricted,
                ..Default::default()
            },
            tour_targets,
            last_pos: at,
            last_move_tick: tick,
            ..Default::default()
        }
    }

    /// Clamp mood into its documented range.
    pub fn clamp_mood(&mut self) {
        self.mood = self.mood.clamp(0.0, 100.0);
    }

    /// The current tour stop, if any remain.
    #[must_use]
    pub fn current_target(&self) -> Option<GridPosition> {
        self.tour_targets.get(self.tour_index).copied()
    }
}

/// Marker component indicating the inspector has submitted their report.
#[derive(Component)]
pub struct Reported;

/// Resource managing inspector spawning schedules.
#[derive(Resource, Default)]
pub struct InspectorSource {
    /// Tick when the next inspector will arrive.
    pub next_visit_tick: u64,
}

/// Fire to land a spec 410 dignitary (spec RED test 1 event, bridged:
/// the mock `FactionId::Empire` becomes the real `u32` relations id).
#[derive(Event, Debug, Clone, Copy)]
pub struct SpawnInspectorEvent {
    /// Layer-3 faction id the report will move.
    pub faction_id: u32,
    /// Generic title of the dignitary.
    pub title: InspectorTitle,
    /// Tour access granted.
    pub access: TourAccess,
}

impl Default for SpawnInspectorEvent {
    fn default() -> Self {
        Self {
            faction_id: INSPECTOR_FACTION_ID,
            title: InspectorTitle::Inspector,
            access: TourAccess::Open,
        }
    }
}

/// Fire to make a present dignitary depart now and file their report
/// (spec RED test 3 event).
#[derive(Event, Debug, Clone, Copy)]
pub struct InspectorDepartEvent {
    /// The dignitary entity departing.
    pub entity: Entity,
}

/// Spec 410 inspection ledger (first-tick pattern: initialized in
/// `simulation.rs`).
#[derive(Resource, Debug, Default)]
pub struct InspectionState {
    /// Lifetime dignitaries landed.
    pub total_visits: u32,
    /// Lifetime exit reports filed.
    pub reports_filed: u32,
    /// Lifetime glowing reports.
    pub glowing_reports: u32,
    /// Lifetime neutral reports.
    pub neutral_reports: u32,
    /// Lifetime damning reports.
    pub damning_reports: u32,
    /// Mood of the most recent report, if any.
    pub last_mood: Option<f32>,
    /// Faction of the most recent report.
    pub last_faction: u32,
    /// Relations delta of the most recent report.
    pub last_delta: i32,
    /// Summary text of the most recent report.
    pub last_report: String,
    /// Lifetime funding granted by glowing reports.
    pub funding_granted: f32,
}

/// Plan a tour from the real beauty signal: the highest-beauty tile
/// is the high-value stop; an open tour adds the runner-up beauty
/// tile (far from the first) and the colony heart (grid centre —
/// the high-traffic proxy). A restricted tour plans ONLY the
/// prettiest stop: the slum is never on the itinerary.
#[must_use]
pub fn plan_tour_targets(grid: &BeautyGrid, access: TourAccess) -> Vec<GridPosition> {
    let mut best: Option<(f32, GridPosition)> = None;
    for y in 0..grid.height {
        for x in 0..grid.width {
            #[allow(clippy::cast_possible_wrap)]
            let pos = GridPosition {
                x: x as i32,
                y: y as i32,
            };
            let v = grid.get(x, y);
            if best.is_none_or(|(bv, _)| v > bv) {
                best = Some((v, pos));
            }
        }
    }
    let Some((_, palace)) = best else {
        return Vec::new();
    };
    if access == TourAccess::Restricted {
        return vec![palace];
    }
    let mut targets = vec![palace];
    // Runner-up: best beauty at Chebyshev distance > 5 from the first.
    let mut second: Option<(f32, GridPosition)> = None;
    for y in 0..grid.height {
        for x in 0..grid.width {
            #[allow(clippy::cast_possible_wrap)]
            let pos = GridPosition {
                x: x as i32,
                y: y as i32,
            };
            if pos.distance_chebyshev(palace) <= 5 {
                continue;
            }
            let v = grid.get(x, y);
            if second.is_none_or(|(bv, _)| v > bv) {
                second = Some((v, pos));
            }
        }
    }
    if let Some((_, pos)) = second {
        targets.push(pos);
    }
    // Colony heart: grid centre, where traffic concentrates.
    #[allow(clippy::cast_possible_wrap)]
    let heart = GridPosition {
        x: (grid.width / 2) as i32,
        y: (grid.height / 2) as i32,
    };
    if !targets.contains(&heart) && targets.len() < MAX_TOUR_STOPS {
        targets.push(heart);
    }
    targets.truncate(MAX_TOUR_STOPS);
    targets
}

/// System to spawn new inspectors periodically. (Legacy, unchanged.)
pub fn spawn_inspector_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    visitor_source: Res<VisitorSource>,
    mut inspector_source: ResMut<InspectorSource>,
) {
    if time.tick >= inspector_source.next_visit_tick && !visitor_source.spawn_points.is_empty() {
        let mut rng = rand::thread_rng();
        let spawn_idx = rng.gen_range(0..visitor_source.spawn_points.len());
        let spawn_pos = visitor_source.spawn_points[spawn_idx];

        let stay_duration = rng.gen_range(1500..2500); // Stays longer than regular visitors

        commands.spawn((
            Inspector::default(),
            Visitor {
                state: VisitorState::Arriving,
                arrival_tick: time.tick,
                departure_tick: time.tick + stay_duration,
            },
            spawn_pos,
            crate::layer1::pop::PopName::random(&mut rng), // Reuse Name
            crate::layer1::needs::Needs::default(),        // Ensure visual representation works
                                                           // Add visual component here (e.g. specialized color/sprite) if we had one
        ));

        // Schedule next visit (long cooldown)
        inspector_source.next_visit_tick = time.tick + rng.gen_range(10000..20000);
    }
}

/// Spec RED test 1, bridged: a spawn event lands a real visitor
/// entity carrying the (extended) [`Inspector`], at the real visitor
/// spawn point, with a tour planned from the real [`BeautyGrid`].
pub fn spawn_dignitary_system(
    mut commands: Commands,
    mut events: EventReader<SpawnInspectorEvent>,
    time: Option<Res<SimulationTime>>,
    source: Option<Res<VisitorSource>>,
    beauty: Option<Res<BeautyGrid>>,
    mut state: ResMut<InspectionState>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    let tick = time.as_ref().map_or(0, |t| t.tick);
    let spawn_pos = source
        .as_deref()
        .and_then(|s| s.spawn_points.first().copied())
        .unwrap_or(GridPosition { x: 0, y: 0 });
    for event in events.read() {
        let targets = beauty
            .as_deref()
            .map(|grid| plan_tour_targets(grid, event.access))
            .unwrap_or_default();
        let mut rng = rand::thread_rng();
        commands.spawn((
            Visitor {
                state: VisitorState::Arriving,
                arrival_tick: tick,
                departure_tick: tick + INSPECTOR_STAY_TICKS,
            },
            Inspector::new(
                event.title,
                event.faction_id,
                event.access,
                tick,
                spawn_pos,
                targets,
            ),
            spawn_pos,
            crate::layer1::pop::PopName::random(&mut rng),
            crate::layer1::needs::Needs::default(),
        ));
        state.total_visits += 1;
        if let Some(events) = chronicle.as_deref_mut() {
            events.send(AddChronicleEvent {
                text: format!(
                    "A visiting {} has arrived by shuttle to tour the colony.",
                    event.title.name()
                ),
                importance: EventImportance::Major,
            });
        }
    }
}

/// System for inspectors to observe their surroundings and update their score. (Legacy, unchanged.)
pub fn observe_inspector_system(
    mut inspectors: Query<(&mut Inspector, &GridPosition)>,
    beauty_grid: Res<BeautyGrid>,
    time: Res<SimulationTime>,
) {
    for (mut inspector, pos) in &mut inspectors {
        if time.tick >= inspector.next_sample_tick {
            if let (Ok(x), Ok(y)) = (usize::try_from(pos.x), usize::try_from(pos.y)) {
                let beauty = beauty_grid.get(x, y);
                inspector.beauty_score += beauty;
                inspector.samples_taken += 1;
            }
            // Sample every 50 ticks (5 seconds at 10 TPS)
            inspector.next_sample_tick = time.tick + 50;
        }
    }
}

/// Spec 410 mood legs (dignitaries only): real [`Corpse`] entities in
/// sight radius (once each), [`BeautyGrid`] decor/squalor samples on
/// a cadence, and one mid-tour sample of the shared food stockpile.
/// The legacy beauty accumulation above runs alongside, untouched.
pub fn inspector_mood_system(
    time: Option<Res<SimulationTime>>,
    beauty: Option<Res<BeautyGrid>>,
    resources: Option<Res<ColonyResources>>,
    corpses: Query<(Entity, &GridPosition), With<Corpse>>,
    mut inspectors: Query<(&GridPosition, &mut Inspector)>,
) {
    let tick = time.as_ref().map_or(0, |t| t.tick);
    let sample_beauty = time
        .as_ref()
        .is_none_or(|t| t.tick % BEAUTY_SAMPLE_INTERVAL == 0);
    for (pos, mut inspector) in &mut inspectors {
        if !inspector.dignitary {
            continue;
        }
        // Corpses: each distinct real corpse in radius counts once.
        for (entity, corpse_pos) in &corpses {
            if inspector.seen_corpses.contains(&entity) {
                continue;
            }
            if pos.distance_chebyshev(*corpse_pos) <= CORPSE_SIGHT_RADIUS {
                inspector.seen_corpses.push(entity);
                inspector.report.corpses_seen += 1;
                inspector.mood += CORPSE_MOOD_HIT;
            }
        }
        // Beauty / squalor mood sample of the current tile.
        if sample_beauty {
            if let Some(grid) = beauty.as_deref() {
                if let (Ok(x), Ok(y)) = (usize::try_from(pos.x), usize::try_from(pos.y)) {
                    let value = grid.get(x, y);
                    inspector.report.beauty_samples += 1;
                    inspector.report.beauty_total += value;
                    if value <= SQUALOR_THRESHOLD {
                        inspector.report.squalor_hits += 1;
                    }
                    inspector.mood += value * BEAUTY_MOOD_FACTOR;
                }
            }
        }
        // Mid-tour table sample (once): the shared stockpile IS the
        // colony's food quality, as far as any real seam can tell.
        let mid_tour = tick >= inspector.arrival_tick + INSPECTOR_STAY_TICKS / 2;
        if !inspector.meal_sampled && mid_tour {
            if let Some(res) = resources.as_deref() {
                inspector.meal_sampled = true;
                if res.food >= MEAL_FOOD_THRESHOLD {
                    inspector.report.fine_meals += 1;
                    inspector.mood += MEAL_GOOD_MOOD;
                } else if res.food <= 0.0 {
                    inspector.report.missed_meals += 1;
                    inspector.mood += MEAL_MISSED_MOOD;
                }
            }
        }
        inspector.clamp_mood();
    }
}

/// The dignitary procession: one greedy step per tick toward the
/// current tour stop (via the real `GridPosition::direction_to`
/// seam). Reaching a stop advances the itinerary. If the dignitary
/// has a pending stop but has not moved for [`STUCK_THRESHOLD`]
/// ticks, the spec §7 trapped rule applies: mood falls every tick
/// until they move. Ambient inspectors plan no tour and never enter.
pub fn inspector_tour_system(
    time: Option<Res<SimulationTime>>,
    mut query: Query<(&mut GridPosition, &mut Inspector, &mut Visitor)>,
) {
    let tick = time.as_ref().map_or(0, |t| t.tick);
    for (mut pos, mut inspector, mut visitor) in &mut query {
        let Some(target) = inspector.current_target() else {
            continue;
        };
        if *pos == target {
            inspector.tour_index += 1;
            inspector.last_pos = *pos;
            inspector.last_move_tick = tick;
            continue;
        }
        // Trapped check happens before the step: it judges the gap
        // since the last OBSERVED movement, so a dignitary pinned by
        // an outside system bleeds mood even on the tick they finally
        // break free.
        if tick.saturating_sub(inspector.last_move_tick) > STUCK_THRESHOLD {
            inspector.mood += TRAPPED_MOOD_RATE;
            inspector.report.trapped_ticks += 1;
            inspector.clamp_mood();
        }
        let (dx, dy) = pos.direction_to(target);
        pos.x += dx;
        pos.y += dy;
        if *pos != inspector.last_pos {
            inspector.last_pos = *pos;
            inspector.last_move_tick = tick;
        }
        if visitor.state == VisitorState::Arriving {
            visitor.state = VisitorState::Loitering;
        }
        if *pos == target {
            inspector.tour_index += 1;
        }
    }
}

/// System to generate a report notification when the inspector departs. (Legacy, unchanged.)
pub fn inspector_report_system(
    mut commands: Commands,
    mut inspectors: Query<(Entity, &Inspector, &Visitor), Without<Reported>>,
    mut notifications: ResMut<NotificationQueue>,
    time: Res<SimulationTime>,
) {
    for (entity, inspector, visitor) in &mut inspectors {
        if visitor.state == VisitorState::Departing {
            let avg_score = if inspector.samples_taken > 0 {
                #[allow(clippy::cast_precision_loss)]
                {
                    inspector.beauty_score / inspector.samples_taken as f32
                }
            } else {
                0.0
            };

            // Grading scale based on typical beauty values (-5 to +10)
            let grade = if avg_score > 5.0 {
                "S"
            } else if avg_score > 2.0 {
                "A"
            } else if avg_score > 0.0 {
                "B"
            } else if avg_score > -2.0 {
                "C"
            } else {
                "F"
            };

            let message = format!("Inspector Report: Grade {grade}. Avg Beauty: {avg_score:.1}");
            let severity = if avg_score < 0.0 {
                NotificationSeverity::Warning
            } else {
                NotificationSeverity::Success
            };

            notifications.add(message, severity, time.tick);

            // Mark as reported so we don't spam notifications every tick while departing
            commands.entity(entity).insert(Reported);
        }
    }
}

/// Spec RED test 3, bridged: on departure (event-driven or at the
/// stay deadline; dignitaries only) the exit report moves the REAL
/// Layer-3 [`FactionRelations`] score for the dignitary's faction, a
/// glowing report also grants funding into the real credits ledger,
/// and the report explains itself in the chronicle +
/// [`InspectionState`]. Ambient inspectors keep the legacy
/// notification path above and are never despawned by this system.
#[allow(clippy::too_many_arguments)]
pub fn process_inspector_departure_system(
    mut commands: Commands,
    mut events: EventReader<InspectorDepartEvent>,
    time: Option<Res<SimulationTime>>,
    inspectors: Query<(Entity, &Inspector)>,
    mut relations: Option<ResMut<FactionRelations>>,
    mut resources: Option<ResMut<ColonyResources>>,
    mut state: ResMut<InspectionState>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    let tick = time.as_ref().map_or(0, |t| t.tick);
    let mut departing: Vec<Entity> = events.read().map(|e| e.entity).collect();
    for (entity, inspector) in &inspectors {
        if inspector.dignitary
            && tick >= inspector.departure_tick
            && !departing.contains(&entity)
        {
            departing.push(entity);
        }
    }
    for entity in departing {
        let Ok((_, inspector)) = inspectors.get(entity) else {
            continue;
        };
        if !inspector.dignitary {
            continue;
        }
        let verdict = verdict_for_mood(inspector.mood);
        let delta = relation_delta_for(verdict, inspector.report.restricted);
        if let Some(relations) = relations.as_deref_mut() {
            let score = relations.scores.entry(inspector.faction_id).or_insert(0);
            *score = (*score + delta).clamp(-100, 100);
        }
        if verdict == ReportVerdict::Glowing {
            if let Some(res) = resources.as_deref_mut() {
                res.add_credits(FUNDING_GRANT);
                state.funding_granted += FUNDING_GRANT;
            }
        }
        state.reports_filed += 1;
        match verdict {
            ReportVerdict::Glowing => state.glowing_reports += 1,
            ReportVerdict::Neutral => state.neutral_reports += 1,
            ReportVerdict::Damning => state.damning_reports += 1,
        }
        state.last_mood = Some(inspector.mood);
        state.last_faction = inspector.faction_id;
        state.last_delta = delta;
        state.last_report = inspector.report.summary();
        if let Some(events) = chronicle.as_deref_mut() {
            let stem = match verdict {
                ReportVerdict::Glowing => REPORT_GLOWING_TEXT,
                ReportVerdict::Neutral => REPORT_NEUTRAL_TEXT,
                ReportVerdict::Damning => REPORT_DAMNING_TEXT,
            };
            events.send(AddChronicleEvent {
                text: format!(
                    "{} ({}: mood {:.0}, relations {:+})",
                    stem,
                    inspector.title.name(),
                    inspector.mood,
                    delta
                ),
                importance: EventImportance::Major,
            });
        }
        commands.entity(entity).despawn();
    }
}

/// Census for the headless panel / STATS (dignitaries only).
#[derive(Debug, Clone, Default)]
pub struct InspectorCensus {
    /// Dignitaries currently in the colony.
    pub present: usize,
    /// Title of the (first) present dignitary, if any.
    pub title: Option<InspectorTitle>,
    /// Mood of the (first) present dignitary, if any.
    pub mood: Option<f32>,
    /// Lifetime reports filed.
    pub reports: u32,
    /// Lifetime glowing reports.
    pub glowing: u32,
    /// Lifetime damning reports.
    pub damning: u32,
    /// Relations delta of the most recent report.
    pub last_delta: i32,
    /// Mood of the most recent report.
    pub last_mood: Option<f32>,
}

/// Read the dignitary census from a raw world (headless helper pattern).
pub fn inspector_census(world: &mut World) -> InspectorCensus {
    let (present, title, mood) = {
        let mut q = world.query::<&Inspector>();
        let mut count = 0usize;
        let mut first: Option<(InspectorTitle, f32)> = None;
        for ins in q.iter(world) {
            if !ins.dignitary {
                continue;
            }
            count += 1;
            if first.is_none() {
                first = Some((ins.title, ins.mood));
            }
        }
        match first {
            Some((t, m)) => (count, Some(t), Some(m)),
            None => (count, None, None),
        }
    };
    let (reports, glowing, damning, last_delta, last_mood) = world
        .get_resource::<InspectionState>()
        .map_or((0, 0, 0, 0, None), |s| {
            (
                s.reports_filed,
                s.glowing_reports,
                s.damning_reports,
                s.last_delta,
                s.last_mood,
            )
        });
    InspectorCensus {
        present,
        title,
        mood,
        reports,
        glowing,
        damning,
        last_delta,
        last_mood,
    }
}

/// Compact STATS label: `<Title>:<mood>` while a visit runs, else
/// `none/<reports>`.
pub fn inspector_stats_label(world: &mut World) -> String {
    let c = inspector_census(world);
    match (c.title, c.mood) {
        (Some(title), Some(mood)) => format!("{}:{:.0}", title.name(), mood),
        _ => format!("none/{}", c.reports),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    // ---- Legacy tests (pre-410, unchanged) ----

    #[test]
    fn test_inspector_component_defaults() {
        let inspector = Inspector::default();
        assert_eq!(inspector.beauty_score, 0.0);
        assert_eq!(inspector.samples_taken, 0);
    }

    #[test]
    fn test_spawn_inspector_trigger() {
        let mut world = World::new();
        crate::setup::init_task_pools(); // Ensure task pools for commands if needed
        world.insert_resource(SimulationTime {
            tick: 1000,
            ..Default::default()
        });
        world.insert_resource(VisitorSource {
            spawn_points: vec![GridPosition { x: 0, y: 0 }],
            ..Default::default()
        });

        // We need a resource to track inspector spawn cooldown specifically
        world.insert_resource(InspectorSource {
            next_visit_tick: 1000,
        });

        // Run system
        world.run_system_once(spawn_inspector_system).unwrap();

        // Verify spawn
        let count = world.query::<(&Inspector, &Visitor)>().iter(&world).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_observation_accumulates_score() {
        let mut world = World::new();
        let mut beauty_grid = BeautyGrid::new(10, 10);
        beauty_grid.set(5, 5, 10.0); // High beauty at pos
        world.insert_resource(beauty_grid);
        world.insert_resource(SimulationTime {
            tick: 100,
            ..Default::default()
        });

        let inspector = world
            .spawn((
                Inspector::default(),
                Visitor::default(),
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        // Run observation system
        world.run_system_once(observe_inspector_system).unwrap();

        let stats = world.get::<Inspector>(inspector).unwrap();
        assert!(stats.beauty_score > 0.0);
        assert_eq!(stats.samples_taken, 1);
    }

    #[test]
    fn test_report_card_generation_on_departure() {
        let mut world = World::new();
        world.insert_resource(NotificationQueue::default());
        world.insert_resource(SimulationTime::default());

        // Spawn inspector in Departing state
        let inspector = world
            .spawn((
                Inspector {
                    beauty_score: 50.0,
                    samples_taken: 5, // Avg 10.0 -> Grade A
                    ..Default::default()
                },
                Visitor {
                    state: VisitorState::Departing,
                    ..Default::default()
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        // Run report system
        world.run_system_once(inspector_report_system).unwrap();

        // Verify notification
        let queue = world.resource::<NotificationQueue>();
        assert!(!queue.active.is_empty());
        assert!(queue.active[0].text.contains("Inspector Report"));

        // Verify component marked as reported to prevent double reporting
        assert!(world.get::<Reported>(inspector).is_some());
    }

    // ---- Spec 410 tests (RED tests adapted to real types) ----

    fn dignitary_at(mood: f32, pos: GridPosition) -> Inspector {
        let mut ins = Inspector::new(
            InspectorTitle::Inspector,
            INSPECTOR_FACTION_ID,
            TourAccess::Open,
            0,
            pos,
            Vec::new(),
        );
        ins.mood = mood;
        ins
    }

    /// Spec RED test 1, adapted: mock `FactionId::Empire` -> real
    /// `u32` id; mock `App` -> `World` + the real visitor bundle.
    #[test]
    fn test_inspector_spawning() {
        let mut world = World::new();
        world.init_resource::<Events<SpawnInspectorEvent>>();
        world.init_resource::<InspectionState>();
        world.insert_resource(VisitorSource {
            spawn_points: vec![GridPosition { x: 3, y: 4 }],
            next_spawn_tick: 0,
        });
        world
            .resource_mut::<Events<SpawnInspectorEvent>>()
            .send(SpawnInspectorEvent::default());
        world.run_system_once(spawn_dignitary_system).unwrap();
        let mut q = world.query::<(&Inspector, &Visitor, &GridPosition)>();
        let found: Vec<_> = q.iter(&world).collect();
        assert_eq!(found.len(), 1, "An inspector should spawn from the event");
        assert_eq!(found[0].2, &GridPosition { x: 3, y: 4 });
        assert_eq!(found[0].0.mood, START_MOOD);
        assert!(found[0].0.dignitary);
        assert_eq!(world.resource::<InspectionState>().total_visits, 1);
    }

    /// Spec RED test 2, adapted: mock bare `Corpse` + `Transform` ->
    /// real funeral `Corpse` + `GridPosition`.
    #[test]
    fn test_inspector_mood_influenced_by_surroundings() {
        let mut world = World::new();
        let inspector = world
            .spawn((
                dignitary_at(50.0, GridPosition { x: 10, y: 10 }),
                GridPosition { x: 10, y: 10 },
            ))
            .id();
        world.spawn((
            Corpse {
                name: "Forgotten".to_string(),
                decay: 0.5,
            },
            GridPosition { x: 11, y: 10 },
        ));
        world.run_system_once(inspector_mood_system).unwrap();
        let comp = world.get::<Inspector>(inspector).unwrap();
        assert!(
            comp.mood < 50.0,
            "Inspector mood should decrease when near a corpse"
        );
        assert_eq!(comp.mood, 40.0);
        assert_eq!(comp.report.corpses_seen, 1);
        // The same corpse never counts twice.
        world.run_system_once(inspector_mood_system).unwrap();
        assert_eq!(world.get::<Inspector>(inspector).unwrap().mood, 40.0);
    }

    /// Spec RED test 3, adapted: mock `FactionRelation` component ->
    /// real Layer-3 `FactionRelations` resource (scores map).
    #[test]
    fn test_inspector_exit_report() {
        let mut world = World::new();
        world.init_resource::<Events<InspectorDepartEvent>>();
        world.init_resource::<InspectionState>();
        let mut relations = FactionRelations::default();
        relations.scores.insert(INSPECTOR_FACTION_ID, 50);
        world.insert_resource(relations);
        let inspector = world
            .spawn(dignitary_at(80.0, GridPosition { x: 0, y: 0 }))
            .id();
        world
            .resource_mut::<Events<InspectorDepartEvent>>()
            .send(InspectorDepartEvent { entity: inspector });
        world
            .run_system_once(process_inspector_departure_system)
            .unwrap();
        let relations = world.resource::<FactionRelations>();
        assert_eq!(
            *relations.scores.get(&INSPECTOR_FACTION_ID).unwrap(),
            60,
            "High mood inspector should improve relations on departure"
        );
        assert!(world.get::<Inspector>(inspector).is_none(), "departed = despawned");
        let state = world.resource::<InspectionState>();
        assert_eq!(state.reports_filed, 1);
        assert_eq!(state.glowing_reports, 1);
        assert_eq!(state.last_delta, RELATION_GLOWING);
    }

    #[test]
    fn test_damning_report_lowers_relations_and_grants_no_funding() {
        let mut world = World::new();
        world.init_resource::<Events<InspectorDepartEvent>>();
        world.init_resource::<InspectionState>();
        world.insert_resource(FactionRelations::default());
        world.insert_resource(ColonyResources::default());
        let inspector = world
            .spawn(dignitary_at(20.0, GridPosition { x: 0, y: 0 }))
            .id();
        world
            .resource_mut::<Events<InspectorDepartEvent>>()
            .send(InspectorDepartEvent { entity: inspector });
        world
            .run_system_once(process_inspector_departure_system)
            .unwrap();
        assert_eq!(
            *world
                .resource::<FactionRelations>()
                .scores
                .get(&INSPECTOR_FACTION_ID)
                .unwrap(),
            -10
        );
        assert_eq!(world.resource::<ColonyResources>().credits, 0.0);
        assert_eq!(world.resource::<InspectionState>().damning_reports, 1);
    }

    #[test]
    fn test_glowing_report_grants_funding() {
        let mut world = World::new();
        world.init_resource::<Events<InspectorDepartEvent>>();
        world.init_resource::<InspectionState>();
        world.insert_resource(FactionRelations::default());
        world.insert_resource(ColonyResources::default());
        let inspector = world
            .spawn(dignitary_at(90.0, GridPosition { x: 0, y: 0 }))
            .id();
        world
            .resource_mut::<Events<InspectorDepartEvent>>()
            .send(InspectorDepartEvent { entity: inspector });
        world
            .run_system_once(process_inspector_departure_system)
            .unwrap();
        assert_eq!(world.resource::<ColonyResources>().credits, FUNDING_GRANT);
        assert_eq!(
            world.resource::<InspectionState>().funding_granted,
            FUNDING_GRANT
        );
    }

    #[test]
    fn test_neutral_report_moves_nothing() {
        assert_eq!(verdict_for_mood(50.0), ReportVerdict::Neutral);
        assert_eq!(relation_delta_for(ReportVerdict::Neutral, false), 0);
        assert_eq!(verdict_for_mood(70.0), ReportVerdict::Glowing);
        assert_eq!(verdict_for_mood(30.0), ReportVerdict::Damning);
    }

    #[test]
    fn test_beauty_raises_mood_and_squalor_lowers_it() {
        // Beautiful tile.
        let mut world = World::new();
        let mut grid = BeautyGrid::new(20, 20);
        grid.set(10, 10, 10.0);
        world.insert_resource(grid);
        let inspector = world
            .spawn((
                dignitary_at(50.0, GridPosition { x: 10, y: 10 }),
                GridPosition { x: 10, y: 10 },
            ))
            .id();
        world.run_system_once(inspector_mood_system).unwrap();
        let comp = world.get::<Inspector>(inspector).unwrap();
        assert!(comp.mood > 50.0, "decor should raise mood, got {}", comp.mood);
        assert_eq!(comp.report.squalor_hits, 0);
        // Squalid tile.
        let mut world = World::new();
        let mut grid = BeautyGrid::new(20, 20);
        grid.set(10, 10, -5.0);
        world.insert_resource(grid);
        let inspector = world
            .spawn((
                dignitary_at(50.0, GridPosition { x: 10, y: 10 }),
                GridPosition { x: 10, y: 10 },
            ))
            .id();
        world.run_system_once(inspector_mood_system).unwrap();
        let comp = world.get::<Inspector>(inspector).unwrap();
        assert!(comp.mood < 50.0, "squalor should lower mood, got {}", comp.mood);
        assert_eq!(comp.report.squalor_hits, 1);
    }

    #[test]
    fn test_restricted_tour_is_an_insult_and_plans_one_stop() {
        let mut grid = BeautyGrid::new(20, 20);
        grid.set(2, 2, 10.0);
        grid.set(18, 18, 8.0);
        let open = plan_tour_targets(&grid, TourAccess::Open);
        let restricted = plan_tour_targets(&grid, TourAccess::Restricted);
        assert_eq!(restricted.len(), 1, "restricted shows only the prettiest stop");
        assert_eq!(restricted[0], GridPosition { x: 2, y: 2 });
        assert!(open.len() > restricted.len(), "open tour sees more of the colony");
        assert_eq!(open[0], GridPosition { x: 2, y: 2 }, "high-value stop first");
        // The insult: arrival mood and the report delta both pay.
        let ins = Inspector::new(
            InspectorTitle::Ambassador,
            INSPECTOR_FACTION_ID,
            TourAccess::Restricted,
            0,
            GridPosition { x: 0, y: 0 },
            restricted,
        );
        assert_eq!(ins.mood, START_MOOD + INSULT_MOOD_HIT);
        assert_eq!(
            relation_delta_for(ReportVerdict::Glowing, true),
            RELATION_GLOWING + RESTRICTED_RELATION_PENALTY
        );
    }

    #[test]
    fn test_tour_steps_toward_stop_and_advances() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 1,
            ..Default::default()
        });
        let mut ins = dignitary_at(50.0, GridPosition { x: 0, y: 0 });
        ins.tour_targets = vec![GridPosition { x: 2, y: 0 }];
        let entity = world
            .spawn((
                ins,
                GridPosition { x: 0, y: 0 },
                Visitor {
                    state: VisitorState::Arriving,
                    arrival_tick: 0,
                    departure_tick: 600,
                },
            ))
            .id();
        world.run_system_once(inspector_tour_system).unwrap();
        assert_eq!(
            world.get::<GridPosition>(entity).copied(),
            Some(GridPosition { x: 1, y: 0 })
        );
        world.run_system_once(inspector_tour_system).unwrap();
        let comp = world.get::<Inspector>(entity).unwrap();
        assert_eq!(comp.tour_index, 1, "reaching the stop advances the itinerary");
    }

    #[test]
    fn test_trapped_dignitary_mood_plummets() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 500,
            ..Default::default()
        });
        let mut ins = dignitary_at(50.0, GridPosition { x: 0, y: 0 });
        ins.tour_targets = vec![GridPosition { x: 9, y: 9 }];
        ins.last_move_tick = 0; // pinned in place for 500 ticks
        let entity = world
            .spawn((ins, GridPosition { x: 0, y: 0 }, Visitor::default()))
            .id();
        world.run_system_once(inspector_tour_system).unwrap();
        let comp = world.get::<Inspector>(entity).unwrap();
        assert!(comp.mood < 50.0, "trapped mood should fall, got {}", comp.mood);
        assert_eq!(comp.report.trapped_ticks, 1);
    }

    #[test]
    fn test_mid_tour_meal_from_shared_stockpile() {
        // Stocked table -> fine meal.
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 300,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 50.0,
            ..Default::default()
        });
        let entity = world
            .spawn((
                dignitary_at(50.0, GridPosition { x: 0, y: 0 }),
                GridPosition { x: 0, y: 0 },
            ))
            .id();
        world.run_system_once(inspector_mood_system).unwrap();
        let comp = world.get::<Inspector>(entity).unwrap();
        assert_eq!(comp.report.fine_meals, 1);
        assert_eq!(comp.mood, 50.0 + MEAL_GOOD_MOOD);
        // Empty table -> missed meal.
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 300,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 0.0,
            ..Default::default()
        });
        let entity = world
            .spawn((
                dignitary_at(50.0, GridPosition { x: 0, y: 0 }),
                GridPosition { x: 0, y: 0 },
            ))
            .id();
        world.run_system_once(inspector_mood_system).unwrap();
        let comp = world.get::<Inspector>(entity).unwrap();
        assert_eq!(comp.report.missed_meals, 1);
        assert_eq!(comp.mood, 50.0 + MEAL_MISSED_MOOD);
    }

    #[test]
    fn test_auto_departure_at_stay_deadline() {
        let mut world = World::new();
        world.init_resource::<Events<InspectorDepartEvent>>();
        world.init_resource::<InspectionState>();
        world.insert_resource(FactionRelations::default());
        world.insert_resource(SimulationTime {
            tick: 600,
            ..Default::default()
        });
        let mut ins = dignitary_at(80.0, GridPosition { x: 0, y: 0 });
        ins.departure_tick = 600;
        let entity = world.spawn(ins).id();
        world
            .run_system_once(process_inspector_departure_system)
            .unwrap();
        assert!(world.get::<Inspector>(entity).is_none());
        assert_eq!(world.resource::<InspectionState>().reports_filed, 1);
    }

    #[test]
    fn test_ambient_inspector_never_files_layer3_report() {
        // The legacy periodic inspector (dignitary == false) must keep
        // its notification-only path: the 410 departure system skips it
        // even past its deadline.
        let mut world = World::new();
        world.init_resource::<Events<InspectorDepartEvent>>();
        world.init_resource::<InspectionState>();
        world.insert_resource(FactionRelations::default());
        world.insert_resource(SimulationTime {
            tick: 9999,
            ..Default::default()
        });
        let entity = world.spawn(Inspector::default()).id();
        world
            .run_system_once(process_inspector_departure_system)
            .unwrap();
        assert!(world.get::<Inspector>(entity).is_some(), "ambient inspector untouched");
        assert_eq!(world.resource::<InspectionState>().reports_filed, 0);
    }

    #[test]
    fn test_census_and_stats_label() {
        let mut world = World::new();
        world.init_resource::<InspectionState>();
        assert_eq!(inspector_stats_label(&mut world), "none/0");
        world.spawn(dignitary_at(62.0, GridPosition { x: 0, y: 0 }));
        let c = inspector_census(&mut world);
        assert_eq!(c.present, 1);
        assert_eq!(inspector_stats_label(&mut world), "Inspector:62");
    }

    /// IP guard: every new player-facing string is original/generic.
    #[test]
    fn test_ip_guard_inspector_strings() {
        let banned = [
            "star wars",
            "star trek",
            "empire",
            "imperial",
            "emperor",
            "coruscant",
            "tatooine",
            "mass effect",
            "citadel",
            "firefly",
            "serenity",
            "babylon 5",
            "dune",
            "arrakis",
            "warhammer",
            "imperium",
        ];
        let strings = [
            REPORT_GLOWING_TEXT,
            REPORT_NEUTRAL_TEXT,
            REPORT_DAMNING_TEXT,
            InspectorTitle::Inspector.name(),
            InspectorTitle::Ambassador.name(),
            InspectorTitle::Auditor.name(),
        ];
        for s in strings {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(!lower.contains(b), "IP-guard: '{b}' in '{s}'");
            }
        }
    }
}
