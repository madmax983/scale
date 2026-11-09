//! Rendering logic for Layer 2: System View.

use crate::layer2::fleet::{Fleet, InOrbit, InTransit};
use crate::layer2::system::{Orbit, OrbitalBody};
use bevy_ecs::prelude::*;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

/// Renders the System View (Layer 2).
///
/// This function is called when `ViewMode` is `System`.
/// It renders the system map, including the sun and orbital bodies.
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render into.
/// * `area` - The screen area to render within.
/// * `world` - The ECS world containing system state.
pub fn render_system_view(frame: &mut Frame, area: Rect, world: &World) {
    let block = Block::default().title(" System Map ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Center is (inner.width/2, inner.height/2)
    let center_x = inner.x + inner.width / 2;
    let center_y = inner.y + inner.height / 2;

    // Helper to calculate orbital position for an entity
    // Returns (x, y) relative to screen (not relative to parent)
    let get_position = |entity_id: Entity| -> (f32, f32) {
        world.get::<Orbit>(entity_id).map_or_else(
            || (f32::from(center_x), f32::from(center_y)),
            |orbit| {
                // Simple polar to cartesian projection
                // Terminal cells are roughly 1:2 aspect ratio, so multiply X by 2 for circular appearance
                let radius_x = orbit.radius * 2.0;
                let radius_y = orbit.radius;

                let x = radius_x.mul_add(orbit.angle.cos(), f32::from(center_x));
                let y = radius_y.mul_add(orbit.angle.sin(), f32::from(center_y));
                (x, y)
            },
        )
    };

    // Draw the sun (center)
    let sun_pos = (center_x, center_y);
    if inner.contains(ratatui::layout::Position {
        x: sun_pos.0,
        y: sun_pos.1,
    }) {
        frame.buffer_mut().set_string(
            sun_pos.0,
            sun_pos.1,
            "☼", // Sun symbol
            Style::default().fg(Color::Yellow),
        );
    }

    // Draw orbital bodies (Planets, Moons, Fleets)
    // We iterate over everything that has an OrbitalBody component (which defines char/color)
    for entity in world.iter_entities() {
        if let Some(body) = entity.get::<OrbitalBody>() {
            // Apply sensor ambiguity visual override
            let (render_char, render_color) =
                if entity.contains::<crate::layer2::sensor_ambiguity::UnidentifiedContact>() {
                    ('?', Color::DarkGray)
                } else {
                    (body.char, body.color)
                };

            let (x, y) = if entity.contains::<Orbit>() {
                get_position(entity.id())
            } else if entity.contains::<Fleet>() {
                if let Some(in_orbit) = entity.get::<InOrbit>() {
                    get_position(in_orbit.parent)
                } else if let Some(transit) = entity.get::<InTransit>() {
                    let start = get_position(transit.origin);
                    let end = get_position(transit.destination);
                    // Linear interpolation with mul_add for better precision
                    let x = (end.0 - start.0).mul_add(transit.progress, start.0);
                    let y = (end.1 - start.1).mul_add(transit.progress, start.1);
                    (x, y)
                } else {
                    continue;
                }
            } else {
                continue;
            };

            // Convert to screen coords
            #[allow(clippy::cast_possible_truncation)]
            let x_i32 = x.round() as i32;
            #[allow(clippy::cast_possible_truncation)]
            let y_i32 = y.round() as i32;

            if x_i32 >= 0 && y_i32 >= 0 {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let pos_x = x_i32 as u16;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let pos_y = y_i32 as u16;

                if inner.contains(ratatui::layout::Position { x: pos_x, y: pos_y }) {
                    frame.buffer_mut().set_string(
                        pos_x,
                        pos_y,
                        render_char.to_string(),
                        Style::default().fg(render_color),
                    );
                }
            }

            // Spec 407: a ringed body draws a faint dotted annulus at
            // its outer edge (same 2:1 terminal aspect as orbits).
            if let Some(ring) = entity.get::<crate::layer2::planetary_rings::PlanetaryRing>() {
                let radius_x = ring.radius_max * 2.0;
                let radius_y = ring.radius_max;
                for k in 0..16 {
                    #[allow(clippy::cast_precision_loss)]
                    let angle = (k as f32) * std::f32::consts::TAU / 16.0;
                    let px = radius_x.mul_add(angle.cos(), x);
                    let py = radius_y.mul_add(angle.sin(), y);
                    #[allow(clippy::cast_possible_truncation)]
                    let (px_i, py_i) = (px.round() as i32, py.round() as i32);
                    if px_i >= 0 && py_i >= 0 {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let (ux, uy) = (px_i as u16, py_i as u16);
                        if inner.contains(ratatui::layout::Position { x: ux, y: uy }) {
                            frame.buffer_mut().set_string(
                                ux,
                                uy,
                                "·",
                                Style::default().fg(Color::DarkGray),
                            );
                        }
                    }
                }
            }
        }
    }

    // Instructions
    let instructions = "Press [Tab] to return to Colony View";
    frame.render_widget(
        Paragraph::new(instructions).alignment(Alignment::Center),
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
    );
}
