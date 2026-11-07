//! Spec 402: Ideological Pressure — a stronger rival neighbor's ethic
//! slowly erodes your pops' hold on the home ethic; pops who come to
//! hold the rival ethic suffer dissent (mood + colony unrest).
//!
//! ## Grounding audit (§8 adaptation pattern — spec sketch vs. real code)
//!
//! The spec sketch assumed a `layer1::ideology` module with
//! `ColonyIdeology { current: EthicType, strength }`,
//! `PopEthics { current: EthicType, adherence: 0..100 }`, and a
//! `Mood { value: 0..100 }` component. None of those exist under those
//! names/shapes. What the codebase actually has:
//!
//! * **Pop ethics (real):** [`PopEthics`] in
//!   `layer1::social::indoctrination` — `{ ethic: Ethic,
//!   stubbornness: 0.0..1.0 }`, where [`Ethic`] is
//!   `Pacifist | Militarist | Collectivist | Individualist |
//!   FreeThinker | StateLoyalist`. There is no `EthicType`. The
//!   indoctrination system already shifts `ethic` by eroding
//!   `stubbornness` to 0 — so `stubbornness` is the real
//!   adherence-analog and this module erodes it the same way instead
//!   of inventing a parallel adherence field.
//! * **Colony ideology (real, but a different axis):** Spec 197 landed
//!   as [`ActiveIdeology`](crate::layer1::social::civic_ideology::ActiveIdeology)
//!   over `IdeologyType::{None, Survivalist, Profit, Knowledge}` — a
//!   colony *philosophy* with no strength value, whose variants are
//!   not ethics a pop can hold (a pop cannot convert to "Profit").
//!   It cannot serve as the home-ethic reference for conversion, so
//!   this module adds the one missing resource,
//!   [`ColonyEthicAnchor`] — the spec's `ColonyIdeology` adapted into
//!   the [`Ethic`] space pops actually live in. The layer-2
//!   `ColonyIdeology` component in `layer2::defector` (an
//!   `Egalitarian/Authoritarian` pair on colony entities) was also
//!   audited and not reused: different layer, different enum, no
//!   strength, and reusing the name would collide.
//! * **Mood (real):** there is no `Mood { value }` on pops. Mood flows
//!   through [`Morale`] + [`MoodModifier`] (`layer1::social::morale`);
//!   `Morale.value` is a cache recomputed from Needs + modifiers, so
//!   dissent is expressed as a refreshed negative `MoodModifier`
//!   (the pattern every recent spec uses), never by writing the cache.
//! * **Unrest (real):** the global [`Unrest`] resource
//!   (`layer1::social::unrest`, 0.0..1.0 + timed modifiers). Dissent
//!   feeds it via a single non-stacking [`UnrestModifier`].
//! * **Spec test 2 adaptation:** the sketch penalized *any* pop whose
//!   ethic differed from the colony's. In the real game pops spawn
//!   with heterogeneous random ethics ([`PopBundle::random`]), so
//!   that rule would permanently penalize most of a vanilla colony.
//!   "Opposed" here means *aligned with the pressing rival*: the
//!   dissent scenario inserts the `NeighborPressure` the sketch's
//!   first test already implied, and only rival-aligned pops are
//!   penalized, only while a rival pressure is actually configured
//!   (`strength > 0`).
//! * **§5 mitigation (real seams, no new building/policy types):**
//!   "Propaganda Broadcaster" is the existing
//!   [`BuildingType::MediaStation`] ("Media station that broadcasts
//!   State Ideology") — each one adds [`MEDIA_STATION_STRENGTH_BONUS`]
//!   to the effective home strength. "Censorship Firewall" is the
//!   existing [`Policy::CensorBroadcasts`] edict — the same policy
//!   `radio_nostalgia` already uses to block broadcast effects —
//!   which halves effective neighbor pressure. Trade-route/hyperlane
//!   pressure routing (§5) and dynamic Layer-3 pressure calculation
//!   (§7) are deferred: the spec itself blesses a static
//!   [`NeighborPressure`] resource for the MVP.
//!
//! All names are original/generic (concept-only); see the IP-guard
//! test at the bottom of this file.
//!
//! [`PopBundle::random`]: crate::layer1::entities::pop::PopBundle::random

use bevy_ecs::prelude::*;

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::edicts::{ColonyPolicies, Policy};
use crate::layer1::pop::Pop;
use crate::layer1::social::indoctrination::{Ethic, PopEthics};
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::unrest::{Unrest, UnrestModifier};

/// Mood/unrest modifier label for ideological dissent. Public strings
/// from this module are collected in [`ideological_pressure_public_strings`]
/// for the IP-guard test.
pub const DISSENT_LABEL: &str = "Ideological Dissent";

/// Stubbornness a freshly converted pop starts with (the spec's
/// `adherence = 10.0` on its 0..100 scale, mapped to 0..1).
pub const CONVERSION_STUBBORNNESS: f32 = 0.1;

/// Duration (ticks) of the refreshed dissent mood modifier. The
/// dissent system re-applies it while the pop stays rival-aligned,
/// so a short duration self-cleans when the pressure lifts.
pub const DISSENT_MODIFIER_DURATION: u32 = 2;

/// Duration (ticks) of the dissent unrest modifier.
pub const DISSENT_UNREST_DURATION: u32 = 10;

/// §5 mitigation: effective home-strength bonus per Media Station
/// (the real "broadcasts State Ideology" building).
pub const MEDIA_STATION_STRENGTH_BONUS: f32 = 20.0;

/// §5 mitigation: multiplier on neighbor pressure while the
/// CensorBroadcasts policy is active.
pub const CENSORSHIP_PRESSURE_FACTOR: f32 = 0.5;

/// The colony's home ethic and how strongly the colony holds it —
/// the spec's `ColonyIdeology { current, strength }` resource,
/// adapted to the real [`Ethic`] space (see module docs).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct ColonyEthicAnchor {
    /// The home ethic pops are anchored to.
    pub ethic: Ethic,
    /// Cultural strength of the home ethic (same 0..100 scale as
    /// [`NeighborPressure::strength`]).
    pub strength: f32,
}

impl Default for ColonyEthicAnchor {
    fn default() -> Self {
        Self {
            // StateLoyalist is the real Ethic that means "loyal to
            // the colony/state" — the natural home anchor.
            ethic: Ethic::StateLoyalist,
            strength: 10.0,
        }
    }
}

/// Cultural pressure emitted by a rival neighbor civilization —
/// the spec's `NeighborPressure` resource, with `EthicType`
/// adapted to the real [`Ethic`].
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct NeighborPressure {
    /// The rival's ethic, pressing in from across the border.
    pub ethic: Ethic,
    /// The rival's cultural strength (0..100). `0.0` (default)
    /// means no rival pressure is configured and both systems
    /// in this module stay inert.
    pub strength: f32,
}

/// Parses an ethic name for the headless console (case-insensitive).
#[must_use]
pub fn parse_ethic(name: &str) -> Option<Ethic> {
    match name.to_ascii_lowercase().as_str() {
        "pacifist" => Some(Ethic::Pacifist),
        "militarist" => Some(Ethic::Militarist),
        "collectivist" => Some(Ethic::Collectivist),
        "individualist" => Some(Ethic::Individualist),
        "freethinker" | "free_thinker" | "free-thinker" => Some(Ethic::FreeThinker),
        "stateloyalist" | "state_loyalist" | "state-loyalist" | "loyalist" => {
            Some(Ethic::StateLoyalist)
        }
        _ => None,
    }
}

/// Short label for an ethic (headless / STATS output).
#[must_use]
pub fn ethic_label(ethic: Ethic) -> &'static str {
    match ethic {
        Ethic::Pacifist => "Pacifist",
        Ethic::Militarist => "Militarist",
        Ethic::Collectivist => "Collectivist",
        Ethic::Individualist => "Individualist",
        Ethic::FreeThinker => "FreeThinker",
        Ethic::StateLoyalist => "StateLoyalist",
    }
}

/// Effective home strength: anchor strength plus the §5 Media
/// Station broadcast bonus.
#[must_use]
pub fn effective_home_strength(anchor: &ColonyEthicAnchor, media_stations: usize) -> f32 {
    anchor.strength + media_stations as f32 * MEDIA_STATION_STRENGTH_BONUS
}

/// Effective neighbor strength: raw pressure, halved while the
/// §5 CensorBroadcasts policy ("Censorship Firewall") is active.
#[must_use]
pub fn effective_neighbor_strength(
    neighbor: &NeighborPressure,
    policies: Option<&ColonyPolicies>,
) -> f32 {
    if policies.is_some_and(|p| p.is_active(Policy::CensorBroadcasts)) {
        neighbor.strength * CENSORSHIP_PRESSURE_FACTOR
    } else {
        neighbor.strength
    }
}

/// Per-tick stubbornness drift. The spec's GREEN formula is
/// `(neighbor - home) * 0.01` per tick on a 0..100 adherence scale;
/// stubbornness lives on 0..1, so the same rate is `/ 100` again.
#[must_use]
pub fn drift_per_tick(home_strength: f32, neighbor_strength: f32) -> f32 {
    if neighbor_strength > home_strength {
        (neighbor_strength - home_strength) * 0.0001
    } else {
        0.0
    }
}

/// Spec 402 pressure system: while a rival neighbor with a
/// different ethic out-pressures the home anchor, home-aligned pops
/// lose stubbornness; at zero they convert to the rival ethic
/// (starting weakly held), and rival-aligned pops are reinforced.
pub fn apply_ideological_pressure_system(
    anchor: Option<Res<ColonyEthicAnchor>>,
    neighbor: Option<Res<NeighborPressure>>,
    policies: Option<Res<ColonyPolicies>>,
    buildings: Query<&Building>,
    mut pops: Query<&mut PopEthics, With<Pop>>,
) {
    let (Some(anchor), Some(neighbor)) = (anchor, neighbor) else {
        return;
    };
    if neighbor.ethic == anchor.ethic {
        return; // A neighbor preaching the home ethic is no pressure.
    }
    let media_stations = buildings
        .iter()
        .filter(|b| b.building_type == BuildingType::MediaStation)
        .count();
    let drift = drift_per_tick(
        effective_home_strength(&anchor, media_stations),
        effective_neighbor_strength(&neighbor, policies.as_deref()),
    );
    if drift <= 0.0 {
        return;
    }
    for mut ethics in pops.iter_mut() {
        if ethics.ethic == anchor.ethic {
            ethics.stubbornness -= drift;
            if ethics.stubbornness <= 0.0 {
                ethics.ethic = neighbor.ethic;
                ethics.stubbornness = CONVERSION_STUBBORNNESS;
            }
        } else if ethics.ethic == neighbor.ethic {
            ethics.stubbornness = (ethics.stubbornness + drift).min(1.0);
        }
    }
}

/// Spec 402 dissent system: pops aligned with the pressing rival's
/// ethic (and not the home ethic) carry a refreshed negative
/// [`MoodModifier`] — magnitude `stubbornness * 0.05`, the spec's
/// `adherence * 0.05` mapped onto the real scales — and their share
/// of the colony feeds the global [`Unrest`] resource through a
/// single non-stacking modifier.
pub fn calculate_ideological_dissent_system(
    anchor: Option<Res<ColonyEthicAnchor>>,
    neighbor: Option<Res<NeighborPressure>>,
    unrest: Option<ResMut<Unrest>>,
    mut pops: Query<(&PopEthics, &mut Morale), With<Pop>>,
) {
    let (Some(anchor), Some(neighbor)) = (anchor, neighbor) else {
        return;
    };
    if neighbor.strength <= 0.0 || neighbor.ethic == anchor.ethic {
        return;
    }
    let mut total = 0usize;
    let mut dissenters = 0usize;
    for (ethics, mut morale) in pops.iter_mut() {
        total += 1;
        if ethics.ethic != neighbor.ethic {
            continue;
        }
        dissenters += 1;
        let penalty = -(ethics.stubbornness * 0.05);
        morale.modifiers.retain(|m| m.label != DISSENT_LABEL);
        morale.add_modifier(MoodModifier {
            label: DISSENT_LABEL.to_string(),
            value: penalty,
            duration: DISSENT_MODIFIER_DURATION,
        });
    }
    if dissenters > 0 {
        if let Some(mut unrest) = unrest {
            if !unrest.modifiers.iter().any(|m| m.label == DISSENT_LABEL) {
                #[allow(clippy::cast_precision_loss)]
                let share = dissenters as f32 / total.max(1) as f32;
                unrest.modifiers.push(UnrestModifier {
                    value: share * 0.2,
                    duration: DISSENT_UNREST_DURATION,
                    label: DISSENT_LABEL.to_string(),
                });
            }
        }
    }
}

/// Census for the headless console / STATS.
#[derive(Debug, Default, Clone, Copy)]
pub struct PressureCensus {
    /// Total pops carrying ethics + morale.
    pub pops: usize,
    /// Pops holding the home ethic.
    pub home_aligned: usize,
    /// Pops holding the rival ethic (the dissenting bloc).
    pub rival_aligned: usize,
}

/// Counts home- vs rival-aligned pops in a raw world (headless).
#[must_use]
pub fn pressure_census(world: &mut World) -> PressureCensus {
    let anchor = world.get_resource::<ColonyEthicAnchor>().copied();
    let neighbor = world.get_resource::<NeighborPressure>().copied();
    let mut census = PressureCensus::default();
    let mut query = world.query::<&PopEthics>();
    for ethics in query.iter(world) {
        census.pops += 1;
        if let Some(anchor) = anchor {
            if ethics.ethic == anchor.ethic {
                census.home_aligned += 1;
            }
        }
        if let Some(neighbor) = neighbor {
            if neighbor.strength > 0.0 && ethics.ethic == neighbor.ethic {
                census.rival_aligned += 1;
            }
        }
    }
    census
}

/// One-token STATS label, e.g. `pressure=Pacifist:50/dissent3`
/// (full panel: the headless `pressure` command).
#[must_use]
pub fn pressure_stats_label(world: &mut World) -> String {
    let Some(neighbor) = world.get_resource::<NeighborPressure>().copied() else {
        return "none".to_string();
    };
    if neighbor.strength <= 0.0 {
        return "none".to_string();
    }
    let census = pressure_census(world);
    format!(
        "{}:{:.0}/dissent{}",
        ethic_label(neighbor.ethic),
        neighbor.strength,
        census.rival_aligned
    )
}

/// Every public string this module can emit (IP-guard test).
#[must_use]
pub fn ideological_pressure_public_strings() -> Vec<String> {
    vec![
        DISSENT_LABEL.to_string(),
        ethic_label(Ethic::Pacifist).to_string(),
        ethic_label(Ethic::Militarist).to_string(),
        ethic_label(Ethic::Collectivist).to_string(),
        ethic_label(Ethic::Individualist).to_string(),
        ethic_label(Ethic::FreeThinker).to_string(),
        ethic_label(Ethic::StateLoyalist).to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn world_with(home: Ethic, home_strength: f32, rival: Ethic, rival_strength: f32) -> World {
        let mut world = World::new();
        world.insert_resource(ColonyEthicAnchor {
            ethic: home,
            strength: home_strength,
        });
        world.insert_resource(NeighborPressure {
            ethic: rival,
            strength: rival_strength,
        });
        world
    }

    // Spec RED test 1, adapted: ColonyIdeology{Survivalist, 10} ->
    // ColonyEthicAnchor{StateLoyalist, 10} (Survivalist is an
    // IdeologyType, not an Ethic a pop can hold); NeighborPressure
    // {Pacifist, 50} is real as-is; PopEthics{current, adherence:100}
    // -> PopEthics{ethic, stubbornness:1.0}.
    #[test]
    fn test_ideological_pressure_drifts_pop_ethics() {
        let mut world = world_with(Ethic::StateLoyalist, 10.0, Ethic::Pacifist, 50.0);
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::StateLoyalist,
                    stubbornness: 1.0,
                },
            ))
            .id();
        world
            .run_system_once(apply_ideological_pressure_system)
            .unwrap();
        let ethics = world.get::<PopEthics>(pop).unwrap();
        assert!(
            ethics.stubbornness < 1.0,
            "Pop adherence (stubbornness) to the home ethic should weaken under pressure"
        );
        assert_eq!(ethics.ethic, Ethic::StateLoyalist, "No conversion yet");
    }

    #[test]
    fn test_pressure_converts_pop_at_zero_stubbornness() {
        let mut world = world_with(Ethic::StateLoyalist, 10.0, Ethic::Pacifist, 50.0);
        // Drift per tick is (50-10)*0.0001 = 0.004; a pop holding
        // its home ethic this weakly converts in one tick.
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::StateLoyalist,
                    stubbornness: 0.003,
                },
            ))
            .id();
        world
            .run_system_once(apply_ideological_pressure_system)
            .unwrap();
        let ethics = world.get::<PopEthics>(pop).unwrap();
        assert_eq!(ethics.ethic, Ethic::Pacifist, "Pop should convert");
        assert!(
            (ethics.stubbornness - CONVERSION_STUBBORNNESS).abs() < f32::EPSILON,
            "Converted pop starts weakly held (spec adherence 10/100)"
        );
    }

    #[test]
    fn test_pressure_reinforces_rival_aligned_pops() {
        let mut world = world_with(Ethic::StateLoyalist, 10.0, Ethic::Pacifist, 50.0);
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::Pacifist,
                    stubbornness: 0.5,
                },
            ))
            .id();
        world
            .run_system_once(apply_ideological_pressure_system)
            .unwrap();
        let ethics = world.get::<PopEthics>(pop).unwrap();
        assert!(ethics.stubbornness > 0.5, "Rival-aligned pops dig in");
    }

    #[test]
    fn test_no_drift_when_neighbor_not_stronger() {
        let mut world = world_with(Ethic::StateLoyalist, 60.0, Ethic::Pacifist, 50.0);
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::StateLoyalist,
                    stubbornness: 1.0,
                },
            ))
            .id();
        world
            .run_system_once(apply_ideological_pressure_system)
            .unwrap();
        let ethics = world.get::<PopEthics>(pop).unwrap();
        assert_eq!(ethics.stubbornness, 1.0);
    }

    #[test]
    fn test_no_drift_when_neighbor_shares_home_ethic() {
        let mut world = world_with(Ethic::Pacifist, 10.0, Ethic::Pacifist, 90.0);
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::Pacifist,
                    stubbornness: 0.5,
                },
            ))
            .id();
        world
            .run_system_once(apply_ideological_pressure_system)
            .unwrap();
        let ethics = world.get::<PopEthics>(pop).unwrap();
        assert_eq!(ethics.stubbornness, 0.5);
        assert_eq!(ethics.ethic, Ethic::Pacifist);
    }

    // Spec RED test 2, adapted: Mood{value:50} -> Morale +
    // MoodModifier (Morale.value is a recomputed cache; the penalty
    // must arrive as a modifier), PopEthics{Pacifist, adherence:80}
    // -> PopEthics{Pacifist, stubbornness:0.8}, plus the
    // NeighborPressure resource the scenario implies (see module
    // docs) and the global Unrest seam.
    #[test]
    fn test_high_drift_causes_unrest() {
        let mut world = world_with(Ethic::StateLoyalist, 10.0, Ethic::Pacifist, 50.0);
        world.insert_resource(Unrest::default());
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::Pacifist,
                    stubbornness: 0.8,
                },
                Morale::default(),
            ))
            .id();
        world
            .run_system_once(calculate_ideological_dissent_system)
            .unwrap();
        let morale = world.get::<Morale>(pop).unwrap();
        let dissent = morale
            .modifiers
            .iter()
            .find(|m| m.label == DISSENT_LABEL)
            .expect("rival-aligned pop should carry a dissent mood modifier");
        assert!(dissent.value < 0.0, "Dissent modifier is a mood penalty");
        assert!(
            (dissent.value - (-0.04)).abs() < 0.001,
            "Penalty is stubbornness * 0.05 (spec adherence * 0.05, rescaled): got {}",
            dissent.value
        );
        let unrest = world.resource::<Unrest>();
        assert!(
            unrest
                .modifiers
                .iter()
                .any(|m| m.label == DISSENT_LABEL && m.value > 0.0),
            "Dissent should feed the global Unrest resource"
        );
    }

    #[test]
    fn test_aligned_pop_suffers_no_dissent() {
        let mut world = world_with(Ethic::StateLoyalist, 10.0, Ethic::Pacifist, 50.0);
        world.insert_resource(Unrest::default());
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::StateLoyalist,
                    stubbornness: 0.8,
                },
                Morale::default(),
            ))
            .id();
        // A third-ethic pop is likewise not part of the rival bloc.
        let bystander = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::Collectivist,
                    stubbornness: 0.8,
                },
                Morale::default(),
            ))
            .id();
        world
            .run_system_once(calculate_ideological_dissent_system)
            .unwrap();
        for entity in [pop, bystander] {
            let morale = world.get::<Morale>(entity).unwrap();
            assert!(
                morale.modifiers.iter().all(|m| m.label != DISSENT_LABEL),
                "Only rival-aligned pops dissent"
            );
        }
        assert!(world.resource::<Unrest>().modifiers.is_empty());
    }

    #[test]
    fn test_dissent_modifiers_do_not_stack() {
        let mut world = world_with(Ethic::StateLoyalist, 10.0, Ethic::Pacifist, 50.0);
        world.insert_resource(Unrest::default());
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::Pacifist,
                    stubbornness: 0.8,
                },
                Morale::default(),
            ))
            .id();
        for _ in 0..3 {
            world
                .run_system_once(calculate_ideological_dissent_system)
                .unwrap();
        }
        let morale = world.get::<Morale>(pop).unwrap();
        assert_eq!(
            morale
                .modifiers
                .iter()
                .filter(|m| m.label == DISSENT_LABEL)
                .count(),
            1,
            "Dissent mood modifier is refreshed, not stacked"
        );
        assert_eq!(
            world
                .resource::<Unrest>()
                .modifiers
                .iter()
                .filter(|m| m.label == DISSENT_LABEL)
                .count(),
            1,
            "Dissent unrest modifier does not stack while active"
        );
    }

    #[test]
    fn test_no_dissent_without_configured_pressure() {
        // Vanilla colony: heterogeneous random ethics, no rival
        // pressure configured — nobody may be penalized.
        let mut world = world_with(Ethic::StateLoyalist, 10.0, Ethic::Pacifist, 0.0);
        world.insert_resource(Unrest::default());
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::Pacifist,
                    stubbornness: 0.8,
                },
                Morale::default(),
            ))
            .id();
        world
            .run_system_once(calculate_ideological_dissent_system)
            .unwrap();
        assert!(world.get::<Morale>(pop).unwrap().modifiers.is_empty());
        assert!(world.resource::<Unrest>().modifiers.is_empty());
    }

    // §5 mitigation via the real Media Station building seam.
    #[test]
    fn test_media_stations_block_drift() {
        let mut world = world_with(Ethic::StateLoyalist, 10.0, Ethic::Pacifist, 50.0);
        for _ in 0..3 {
            world.spawn(Building {
                building_type: BuildingType::MediaStation,
            });
        }
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::StateLoyalist,
                    stubbornness: 1.0,
                },
            ))
            .id();
        world
            .run_system_once(apply_ideological_pressure_system)
            .unwrap();
        // Effective home strength 10 + 3*20 = 70 > 50: no drift.
        assert_eq!(world.get::<PopEthics>(pop).unwrap().stubbornness, 1.0);
    }

    // §5 mitigation via the real CensorBroadcasts policy seam.
    #[test]
    fn test_censorship_halves_pressure() {
        let mut policies = ColonyPolicies::default();
        policies.toggle(Policy::CensorBroadcasts);
        let mut world = world_with(Ethic::StateLoyalist, 10.0, Ethic::Pacifist, 50.0);
        world.insert_resource(policies);
        let pop = world
            .spawn((
                Pop,
                PopEthics {
                    ethic: Ethic::StateLoyalist,
                    stubbornness: 1.0,
                },
            ))
            .id();
        world
            .run_system_once(apply_ideological_pressure_system)
            .unwrap();
        // Censored: effective neighbor 25, drift (25-10)*0.0001.
        let expected = 1.0 - 15.0 * 0.0001;
        let got = world.get::<PopEthics>(pop).unwrap().stubbornness;
        assert!(
            (got - expected).abs() < 0.000_01,
            "Censorship should halve pressure: expected {expected}, got {got}"
        );
    }

    #[test]
    fn test_parse_ethic_roundtrip() {
        for ethic in [
            Ethic::Pacifist,
            Ethic::Militarist,
            Ethic::Collectivist,
            Ethic::Individualist,
            Ethic::FreeThinker,
            Ethic::StateLoyalist,
        ] {
            assert_eq!(parse_ethic(ethic_label(ethic)), Some(ethic));
        }
        assert_eq!(parse_ethic("loyalist"), Some(Ethic::StateLoyalist));
        assert_eq!(parse_ethic("nonsense"), None);
    }

    // --- IP guard ---
    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Concept-only: generic ethic names, nothing lifted from
        // outside fiction or other games.
        let banned = [
            "stellaris",
            "ethics wheel",
            "fanatic",
            "xenophile",
            "xenophobe",
            "gestalt",
            "hive mind",
            "sad king billy",
            "windsor",
            "dune",
            "imperium",
        ];
        for s in ideological_pressure_public_strings() {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(
                    !lower.contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
