#![deny(unsafe_code)]
//! SCALE WASM entry point - runs the game in a browser via Ratzilla.

#[cfg(target_arch = "wasm32")]
use ratzilla::backend::dom::DomBackend;
#[cfg(target_arch = "wasm32")]
use ratzilla::ratatui::Terminal;
#[cfg(target_arch = "wasm32")]
use ratzilla::WebRenderer;
#[cfg(target_arch = "wasm32")]
use scale::layer1::map::update_camera_smooth;
#[cfg(target_arch = "wasm32")]
use scale::setup::setup_world;
#[cfg(target_arch = "wasm32")]
use scale::shared::keyboard::{GameKeyEvent, GameMouseEvent};
#[cfg(target_arch = "wasm32")]
use scale::shared::state::GameState;
#[cfg(target_arch = "wasm32")]
use scale::shared::time::{SimSpeed, SimulationTime, WallTime};
#[cfg(target_arch = "wasm32")]
use scale::simulation::run_simulation_tick;
#[cfg(target_arch = "wasm32")]
use scale::ui::input::{route_root_input, route_root_mouse_input};
#[cfg(target_arch = "wasm32")]
use scale::ui::map::update_render_cache;
#[cfg(target_arch = "wasm32")]
use scale::ui::render_with_shell;
#[cfg(target_arch = "wasm32")]
use scale::ui::shell::build_default_shell;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("This binary is for WASM only.");
}

#[cfg(target_arch = "wasm32")]
fn main() -> std::io::Result<()> {
    let world = Rc::new(RefCell::new(setup_world()));
    let config = world
        .borrow()
        .resource::<scale::ui::shell::ShellConfig>()
        .clone();
    let shell = Rc::new(RefCell::new(build_default_shell(Rc::clone(&world), config)));

    let backend = DomBackend::new()?;
    let mut terminal = Terminal::new(backend)?;

    terminal.on_key_event({
        let world = world.clone();
        let shell = shell.clone();
        move |key_event| {
            if let Ok(game_key) = GameKeyEvent::try_from(key_event) {
                route_root_input(&world, &mut shell.borrow_mut(), game_key);
            }
        }
    })?;

    terminal.on_mouse_event({
        let world = world.clone();
        let shell = shell.clone();
        move |mouse_event| {
            if let Ok(game_mouse) = GameMouseEvent::try_from(mouse_event) {
                route_root_mouse_input(&world, &shell.borrow(), game_mouse);
            }
        }
    })?;

    let frame_count = Rc::new(RefCell::new(0u32));

    terminal.draw_web({
        let world = world;
        let shell = shell;
        move |frame| {
            {
                let mut world = world.borrow_mut();

                world.resource_mut::<WallTime>().0 += 1.0 / 60.0;

                let mut count = frame_count.borrow_mut();
                *count += 1;

                if *count >= 6 {
                    *count = 0;
                    if *world.resource::<GameState>() == GameState::Running {
                        let speed = world.resource::<SimulationTime>().speed;
                        if speed != SimSpeed::Paused {
                            run_simulation_tick(&mut world);
                        }
                    }
                }

                update_camera_smooth(&mut world);
                update_render_cache(&mut world);
            }

            let world = world.borrow();
            let mut shell = shell.borrow_mut();
            render_with_shell(&world, &mut shell, frame);
        }
    });

    Ok(())
}
