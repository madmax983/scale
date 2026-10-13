#![deny(unsafe_code)]
//! SCALE WASM entry point - runs the game in a browser via Ratzilla.

#[cfg(target_arch = "wasm32")]
use ratzilla::backend::dom::DomBackend;
#[cfg(target_arch = "wasm32")]
use ratzilla::event::{KeyEvent, MouseEvent};
#[cfg(target_arch = "wasm32")]
use ratzilla::ratatui::backend::{Backend, ClearType, WindowSize};
#[cfg(target_arch = "wasm32")]
use ratzilla::ratatui::buffer::Cell;
#[cfg(target_arch = "wasm32")]
use ratzilla::ratatui::layout::{Position, Size};
#[cfg(target_arch = "wasm32")]
use ratzilla::ratatui::Terminal;
#[cfg(target_arch = "wasm32")]
use ratzilla::{WebEventHandler, WebRenderer};
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

/// [`DomBackend`] that reports the size of the grid it actually built.
///
/// `DomBackend::size` guesses from the window size assuming 10x20px cells,
/// while the grid itself is built from the measured font. With a smaller font
/// the UI only fills part of the window; with a larger one ratatui draws past
/// the end of the grid and the backend panics.
#[cfg(target_arch = "wasm32")]
struct GridSizedDom {
    inner: DomBackend,
    grid: Size,
}

#[cfg(target_arch = "wasm32")]
impl GridSizedDom {
    fn new() -> std::io::Result<Self> {
        let mut inner = DomBackend::new()?;
        let grid = inner.window_size()?.columns_rows;
        Ok(Self { inner, grid })
    }
}

#[cfg(target_arch = "wasm32")]
impl Backend for GridSizedDom {
    type Error = std::io::Error;

    fn draw<'a, I>(&mut self, content: I) -> std::io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        // The backend rebuilds its grid lazily, inside `draw`, after a window
        // resize. Trigger that first so cells laid out for the old size can be
        // dropped instead of indexing past the new grid; ratatui sees the new
        // size and repaints on the next frame.
        self.inner.draw(std::iter::empty::<(u16, u16, &Cell)>())?;
        let grid = self.inner.window_size()?.columns_rows;
        self.grid = grid;
        self.inner
            .draw(content.filter(|&(x, y, _)| x < grid.width && y < grid.height))
    }

    fn hide_cursor(&mut self) -> std::io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> std::io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> std::io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> std::io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> std::io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> std::io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> std::io::Result<Size> {
        Ok(self.grid)
    }

    fn window_size(&mut self) -> std::io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(target_arch = "wasm32")]
impl WebEventHandler for GridSizedDom {
    fn on_mouse_event<F>(&mut self, callback: F) -> Result<(), ratzilla::error::Error>
    where
        F: FnMut(MouseEvent) + 'static,
    {
        self.inner.on_mouse_event(callback)
    }

    fn clear_mouse_events(&mut self) {
        self.inner.clear_mouse_events();
    }

    fn on_key_event<F>(&mut self, callback: F) -> Result<(), ratzilla::error::Error>
    where
        F: FnMut(KeyEvent) + 'static,
    {
        self.inner.on_key_event(callback)
    }

    fn clear_key_events(&mut self) {
        self.inner.clear_key_events();
    }
}

/// Route every `keydown` on the document to `callback`, which returns whether
/// it handled the key.
///
/// `DomBackend::on_key_event` listens on the grid element instead, which only
/// sees keys once something focuses it, and which is replaced (dropping the
/// listener) on every window resize.
#[cfg(target_arch = "wasm32")]
fn on_document_key_event(
    mut callback: impl FnMut(KeyEvent) -> bool + 'static,
) -> std::io::Result<()> {
    use ratzilla::web_sys::wasm_bindgen::{prelude::Closure, JsCast};
    use ratzilla::web_sys::{window, KeyboardEvent};

    let document = window()
        .and_then(|window| window.document())
        .ok_or_else(|| std::io::Error::other("no document"))?;
    let closure = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
        let handled = callback(event.clone().into());
        // Stop Tab, arrows, space etc. from moving focus or scrolling the page,
        // but leave browser shortcuts (reload, devtools, ...) alone.
        if handled && !event.ctrl_key() && !event.meta_key() {
            event.prevent_default();
        }
    });
    document
        .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
        .map_err(|err| std::io::Error::other(format!("{err:?}")))?;
    // The listener lives for the lifetime of the page.
    closure.forget();
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() -> std::io::Result<()> {
    let world = Rc::new(RefCell::new(setup_world()));
    let config = world
        .borrow()
        .resource::<scale::ui::shell::ShellConfig>()
        .clone();
    let shell = Rc::new(RefCell::new(build_default_shell(Rc::clone(&world), config)));

    let backend = GridSizedDom::new()?;
    let mut terminal = Terminal::new(backend)?;

    on_document_key_event({
        let world = world.clone();
        let shell = shell.clone();
        move |key_event| {
            GameKeyEvent::try_from(key_event).is_ok_and(|game_key| {
                route_root_input(&world, &mut shell.borrow_mut(), game_key);
                true
            })
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
