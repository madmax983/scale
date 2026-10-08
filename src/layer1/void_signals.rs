//! Experimental Feature: Void Signals (Radio Silence)
//!
//! Adds a signal interception mechanic to Observatories.
//! Pops assigned to Observatories will passively decrypt signals from the void,
//! yielding Knowledge, Resources, or Lore.

use crate::layer1::actions::{AssignedTo, AssignmentType};
use crate::layer1::map::{CameraTarget, GridPosition};
use crate::layer1::resources::{ColonyResources, ResourceType};
use crate::shared::log::MessageLog;
use crate::shared::narrative::NarrativeGenerator;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Represents a signal intercepted from deep space.
#[derive(Debug, Clone)]
pub struct VoidSignal {
    /// Unique ID for the signal.
    pub id: u64,
    /// Display name (e.g., "Faint Beeping").
    pub name: String,
    /// Flavor text describing the signal.
    pub flavor_text: String,
    /// Decryption progress (0.0 to 100.0).
    pub progress: f32,
    /// Difficulty modifier (1.0 = standard speed).
    pub difficulty: f32,
    /// The reward for decrypting this signal.
    pub reward: SignalReward,
}

/// The reward granted upon signal decryption.
#[derive(Debug, Clone)]
pub enum SignalReward {
    /// Direct Knowledge boost.
    Knowledge(f32),
    /// A cache of resources (Supply Drop).
    Resources(ResourceType, f32),
    /// Lore entry (flavor only).
    Lore(String),
}

/// Resource managing the network of intercepted signals.
#[derive(Resource, Default)]
pub struct SignalNetwork {
    /// List of currently available signals.
    pub signals: Vec<VoidSignal>,
    /// The ID of the signal currently being decrypted.
    pub active_signal_id: Option<u64>,
    /// Counter for unique IDs.
    pub next_id: u64,
}

impl std::fmt::Display for SignalNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use comfy_table::{presets::UTF8_FULL, Cell, Color as TableColor, Table};
        #[cfg(not(target_arch = "wasm32"))]
        use crossterm::style::{Color, Stylize};
        #[cfg(target_arch = "wasm32")]
        use crate::wasm_style::{Color, Stylize};

        writeln!(
            f,
            "{}",
            "╭── Void Signals (Network) ───────────────────────╮".with(Color::Cyan)
        )?;

        if self.signals.is_empty() {
            let text = format!("{:<47}", "No active signals detected.");
            writeln!(f, "│ {} │", text.with(Color::DarkGrey))?;
            writeln!(
                f,
                "{}",
                "╰─────────────────────────────────────────────────╯".with(Color::Cyan)
            )?;
            return Ok(());
        } else {
            let text = format!(
                "{:<47}",
                format!("{} active signals detected.", self.signals.len())
            );
            writeln!(f, "│ {} │", text.with(Color::White))?;
            writeln!(
                f,
                "{}",
                "╰─────────────────────────────────────────────────╯".with(Color::Cyan)
            )?;
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
            .set_content_arrangement(comfy_table::ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("ID").add_attribute(comfy_table::Attribute::Bold),
                Cell::new("Name").add_attribute(comfy_table::Attribute::Bold),
                Cell::new("Status").add_attribute(comfy_table::Attribute::Bold),
                Cell::new("Progress").add_attribute(comfy_table::Attribute::Bold),
                Cell::new("Reward").add_attribute(comfy_table::Attribute::Bold),
            ]);

        for signal in &self.signals {
            let is_active = self.active_signal_id == Some(signal.id);
            let status_text = if is_active { "Decrypting" } else { "Idle" };
            let status_color = if is_active {
                TableColor::Green
            } else {
                TableColor::DarkGrey
            };

            let reward_text = match &signal.reward {
                SignalReward::Knowledge(amt) => format!("Knowledge ({:.1})", amt),
                SignalReward::Resources(res, amt) => format!("{:?} ({:.1})", res, amt),
                SignalReward::Lore(_) => "Lore".to_string(),
            };

            let reward_color = match &signal.reward {
                SignalReward::Knowledge(_) => TableColor::Cyan,
                SignalReward::Resources(_, _) => TableColor::Yellow,
                SignalReward::Lore(_) => TableColor::Magenta,
            };

            table.add_row(vec![
                Cell::new(signal.id.to_string()).fg(TableColor::DarkGrey),
                Cell::new(&signal.name).fg(TableColor::White),
                Cell::new(status_text).fg(status_color),
                Cell::new(format!("{:.1}%", signal.progress)).fg(if is_active {
                    TableColor::Green
                } else {
                    TableColor::DarkGrey
                }),
                Cell::new(reward_text).fg(reward_color),
            ]);
        }

        write!(f, "{}", table)
    }
}

impl SignalNetwork {
    /// adds a new signal to the network.
    pub fn add_signal(&mut self, name: String, flavor: String, reward: SignalReward) {
        let id = self.next_id;
        self.next_id += 1;
        self.signals.push(VoidSignal {
            id,
            name,
            flavor_text: flavor,
            progress: 0.0,
            difficulty: 1.0,
            reward,
        });
    }
}

/// Scans for new signals in the cosmic background.
///
/// Runs occasionally to populate the signal list.
pub fn scan_for_signals_system(
    mut network: ResMut<SignalNetwork>,
    mut log: Option<ResMut<MessageLog>>,
    generator: Res<NarrativeGenerator>,
) {
    // Cap at 5 active signals
    if network.signals.len() >= 5 {
        return;
    }

    let mut rng = rand::thread_rng();

    // 1% chance per tick to find a signal
    if rng.gen_bool(0.01) {
        let signal_type = rng.gen_range(0..3);
        let (name, flavor, reward) = match signal_type {
            0 => {
                // Knowledge Signal
                let prefix = generator
                    .get_random_fragment("STAR_PREFIX")
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string());
                (
                    format!("{prefix} Transmission"),
                    "A complex data stream from a distant star.".to_string(),
                    SignalReward::Knowledge(rng.gen_range(10.0..50.0)),
                )
            }
            1 => {
                // Resource Signal
                let res_type = if rng.gen_bool(0.5) {
                    ResourceType::Metal
                } else {
                    ResourceType::Fuel
                };
                (
                    "Supply Drop Beacon".to_string(),
                    "Coordinates for a supply pod dropped by a passing freighter.".to_string(),
                    SignalReward::Resources(res_type, rng.gen_range(20.0..100.0)),
                )
            }
            _ => {
                // Lore Signal
                (
                    "Ghost Echo".to_string(),
                    "A voice whispering in a dead language.".to_string(),
                    SignalReward::Lore("The void stares back...".to_string()),
                )
            }
        };

        network.add_signal(name.clone(), flavor, reward);

        if let Some(log) = &mut log {
            log.add(format!("Scanner detected new signal: {name}"));
        }
    }
}

/// Auto-tunes the observatory to the first available signal if none is selected.
pub fn auto_tune_system(mut network: ResMut<SignalNetwork>) {
    if network.active_signal_id.is_none() && !network.signals.is_empty() {
        network.active_signal_id = Some(network.signals[0].id);
    }
}

/// Decrypts the active signal using Observatory computing power.
pub fn decrypt_signals_system(
    mut network: ResMut<SignalNetwork>,
    observatories: Query<(Entity, Option<&crate::layer1::observatory::Observatory>)>,
    mut pops: Query<&AssignedTo>,
    mut resources: ResMut<ColonyResources>,
    mut log: Option<ResMut<MessageLog>>,
    mut commands: Commands,
    camera_target: Option<Res<CameraTarget>>,
) {
    let target_id = if let Some(id) = network.active_signal_id {
        id
    } else {
        return;
    };

    // Calculate total computing power from workers
    let mut computing_power = 0.0;
    for assignment in &mut pops {
        if assignment.assignment_type == AssignmentType::ObservatoryWorker {
            // Validate the entity is actually an observatory
            if observatories.get(assignment.entity).is_ok() {
                computing_power += 0.05; // 5% per worker per tick
            }
        }
    }

    if computing_power <= 0.0 {
        return;
    }

    // Find and update the signal
    let mut completed_signal = None;
    let mut remove_index = None;

    if let Some((index, signal)) = network
        .signals
        .iter_mut()
        .enumerate()
        .find(|(_, s)| s.id == target_id)
    {
        signal.progress += computing_power / signal.difficulty;
        if signal.progress >= 100.0 {
            completed_signal = Some(signal.clone());
            remove_index = Some(index);
        }
    }

    // Handle completion
    if let Some(signal) = completed_signal {
        match signal.reward {
            SignalReward::Knowledge(amount) => {
                resources.knowledge =
                    (resources.knowledge + amount).clamp(0.0, resources.max_knowledge);
                if let Some(log) = &mut log {
                    log.add(format!(
                        "Decrypted '{}': Gained {:.1} Knowledge",
                        signal.name, amount
                    ));
                }
            }
            SignalReward::Resources(res_type, amount) => {
                match res_type {
                    ResourceType::Metal => resources.metal += amount,
                    ResourceType::Fuel => resources.fuel += amount,
                    _ => {} // Handle others if needed
                }
                if let Some(log) = &mut log {
                    log.add(format!(
                        "Decrypted '{}': Recovered {:.1} {:?}",
                        signal.name, amount, res_type
                    ));
                }
            }
            SignalReward::Lore(text) => {
                if let Some(log) = &mut log {
                    log.add(format!("Decrypted '{}': \"{}\"", signal.name, text));
                }
            }
        }

        // Juice!
        if let Some(target) = camera_target {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            spawn_confetti(
                &mut commands,
                GridPosition {
                    x: target.x as i32,
                    y: target.y as i32,
                },
            );
        }

        // Remove from list
        if let Some(idx) = remove_index {
            network.signals.remove(idx);
        }
        network.active_signal_id = None;
    }
}

fn spawn_confetti(commands: &mut Commands, pos: GridPosition) {
    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();
    let colors = [
        ratatui::style::Color::Red,
        ratatui::style::Color::Green,
        ratatui::style::Color::Blue,
        ratatui::style::Color::Yellow,
        ratatui::style::Color::Magenta,
        ratatui::style::Color::Cyan,
        ratatui::style::Color::White,
    ];
    let chars = ['*', '.', '+', 'x', 'o'];

    for _ in 0..30 {
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let speed = rng.gen_range(0.5..1.5);
        let dx = angle.cos() * speed;
        let dy = angle.sin() * speed;

        if let (Some(color), Some(char)) = (colors.choose(&mut rng), chars.choose(&mut rng)) {
            let color = *color;
            let char = *char;
            let lifetime = rng.gen_range(20..40);

            commands.spawn((
                crate::layer1::particles::Particle {
                    char,
                    color,
                    lifetime,
                },
                pos,
                crate::layer1::particles::ParticleVelocity { dx, dy },
                crate::layer1::particles::ParticleAccumulator::default(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::actions::{AssignedTo, AssignmentType};
    use crate::layer1::observatory::Observatory;
    use crate::layer1::pop::Pop;

    #[test]
    fn test_signal_network_add() {
        let mut net = SignalNetwork::default();
        net.add_signal(
            "Test".into(),
            "Flavor".into(),
            SignalReward::Knowledge(10.0),
        );
        assert_eq!(net.signals.len(), 1);
        assert_eq!(net.next_id, 1);
    }

    #[test]
    fn test_auto_tune() {
        let mut world = World::new();
        let mut net = SignalNetwork::default();
        net.add_signal("S1".into(), "".into(), SignalReward::Knowledge(10.0));
        world.insert_resource(net);

        let mut schedule = Schedule::default();
        schedule.add_systems(auto_tune_system);
        schedule.run(&mut world);

        let net = world.resource::<SignalNetwork>();
        assert_eq!(net.active_signal_id, Some(0));
    }

    #[test]
    fn test_decrypt_progress() {
        let mut world = World::new();
        world.insert_resource(SignalNetwork::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(MessageLog::default());

        // Add signal
        world.resource_mut::<SignalNetwork>().add_signal(
            "Test".into(),
            "".into(),
            SignalReward::Knowledge(100.0),
        );
        world.resource_mut::<SignalNetwork>().active_signal_id = Some(0);

        // Spawn Observatory & Worker
        let observatory = world.spawn(Observatory { efficiency: 100.0 }).id();
        world.spawn((
            Pop,
            AssignedTo {
                entity: observatory,
                assignment_type: AssignmentType::ObservatoryWorker,
            },
        ));

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(decrypt_signals_system);

        // 0.05 per tick. 100 progress. Needs 2000 ticks.
        // Run 100 ticks -> 5 progress.
        for _ in 0..100 {
            schedule.run(&mut world);
        }

        let net = world.resource::<SignalNetwork>();
        let signal = &net.signals[0];
        assert!((signal.progress - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_decrypt_completion() {
        let mut world = World::new();
        world.insert_resource(SignalNetwork::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(MessageLog::default());

        // Add easy signal
        world.resource_mut::<SignalNetwork>().add_signal(
            "Easy".into(),
            "".into(),
            SignalReward::Knowledge(50.0),
        );
        world.resource_mut::<SignalNetwork>().active_signal_id = Some(0);

        // Set progress to 99.95 (0.05 step will complete it)
        world.resource_mut::<SignalNetwork>().signals[0].progress = 99.95;

        // Spawn Observatory & Worker
        let observatory = world.spawn(Observatory { efficiency: 100.0 }).id();
        world.spawn((
            Pop,
            AssignedTo {
                entity: observatory,
                assignment_type: AssignmentType::ObservatoryWorker,
            },
        ));

        // Run system ONCE
        let mut schedule = Schedule::default();
        schedule.add_systems(decrypt_signals_system);
        schedule.run(&mut world);

        // Should be gone
        let net = world.resource::<SignalNetwork>();
        assert!(net.signals.is_empty());
        assert!(net.active_signal_id.is_none());

        // Should have knowledge
        let res = world.resource::<ColonyResources>();
        assert!((res.knowledge - 50.0).abs() < 0.001);
    }
}
