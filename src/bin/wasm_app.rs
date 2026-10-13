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


/// Spec 1383: route `pointerdown` taps on the document to `callback`
/// with the tap's CSS-pixel coordinates; the callback returns whether
/// it handled the tap (handled taps are `prevent_default`-ed so the
/// browser does not also synthesize a mouse click for them).
#[cfg(target_arch = "wasm32")]
fn on_document_pointer_event(
    mut callback: impl FnMut(f32, f32) -> bool + 'static,
) -> std::io::Result<()> {
    use ratzilla::web_sys::wasm_bindgen::{prelude::Closure, JsCast};
    use ratzilla::web_sys::{window, MouseEvent};

    let document = window()
        .and_then(|window| window.document())
        .ok_or_else(|| std::io::Error::other("no document"))?;
    let closure = Closure::<dyn FnMut(MouseEvent)>::new(move |event: MouseEvent| {
        #[allow(clippy::cast_precision_loss)]
        let handled = callback(event.client_x() as f32, event.client_y() as f32);
        if handled {
            event.prevent_default();
        }
    });
    document
        .add_event_listener_with_callback("pointerdown", closure.as_ref().unchecked_ref())
        .map_err(|err| std::io::Error::other(format!("{err:?}")))?;
    closure.forget();
    Ok(())
}

/// Spec 1383: the pixel rect the terminal grid actually occupies.
/// Prefers the grid container's own bounding rect (which captures any
/// letterboxing/offset and the space the shell reserves for the touch
/// cluster); falls back to the full window.
#[cfg(target_arch = "wasm32")]
fn grid_pixel_rect() -> Option<scale::ui::mobile::PixelRect> {
    use ratzilla::web_sys::window;
    let window = window()?;
    if let Some(document) = window.document() {
        if let Ok(Some(element)) = document.query_selector("body > div") {
            let rect = element.get_bounding_client_rect();
            if rect.width() > 0.0 && rect.height() > 0.0 {
                #[allow(clippy::cast_possible_truncation)]
                return Some(scale::ui::mobile::PixelRect::new(
                    rect.left() as f32,
                    rect.top() as f32,
                    rect.width() as f32,
                    rect.height() as f32,
                ));
            }
        }
    }
    let width = window.inner_width().ok()?.as_f64()? as f32;
    let height = window.inner_height().ok()?.as_f64()? as f32;
    Some(scale::ui::mobile::PixelRect::new(0.0, 0.0, width, height))
}

#[cfg(target_arch = "wasm32")]
fn main() -> std::io::Result<()> {
    let world = Rc::new(RefCell::new(setup_world()));

    // Spec 1382: IndexedDB save bridge + persistent-storage request,
    // plus settings crumbs pre-selecting the last origin (localStorage).
    scale::persistence::wasm_idb::request_persistent_storage();
    let save_bridge = Rc::new(RefCell::new(
        scale::persistence::wasm_idb::shared_bridge(),
    ));
    save_bridge.borrow().request_list();
    {
        let settings = scale::persistence::load_settings();
        if let Some(origin_id) = settings.last_origin.as_deref() {
            if let Some(kind) =
                scale::layer1::culture::origins::OriginKind::parse(origin_id)
            {
                world.borrow_mut().insert_resource(
                    scale::layer1::culture::origins::OriginChoice {
                        chosen: Some(kind),
                    },
                );
            }
        }
    }
    let config = world
        .borrow()
        .resource::<scale::ui::shell::ShellConfig>()
        .clone();
    let shell = Rc::new(RefCell::new(build_default_shell(Rc::clone(&world), config)));

    let grid_size = Rc::new(RefCell::new((0u16, 0u16)));
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


    // Spec 1383 tap layer: taps on the main menu are translated into
    // the keyboard sequences a keyboard user would produce (select,
    // then confirm on a second tap of the selected row; setup rows
    // focus/cycle). In game, taps fall through to ratzilla's existing
    // mouse synthesis -> route_root_mouse_input, exactly like a
    // physical click, so nothing is double-routed.
    let _ = on_document_pointer_event({
        let world = world.clone();
        let shell = shell.clone();
        let grid_size = grid_size.clone();
        move |px, py| {
            use scale::ui::mobile::{
                menu_rows, menu_tap_keys, setup_row_tap_keys, tap_to_cell, SetupRow,
            };
            let (cols, rows) = *grid_size.borrow();
            if cols == 0 || rows == 0 {
                return false;
            }
            let is_main_menu = {
                let world = world.borrow();
                *world.resource::<GameState>() == GameState::MainMenu
            };
            if !is_main_menu {
                return false;
            }
            let Some(rect) = grid_pixel_rect() else {
                return false;
            };
            let Some(cell) = tap_to_cell(px, py, rect, cols, rows) else {
                return false;
            };
            let (selected, count, scenario_focused) = {
                let world = world.borrow();
                let menu = world.resource::<scale::ui::menu_state::MenuState>();
                (
                    menu.selected_index,
                    menu.options.len(),
                    menu.selector == scale::ui::menu_state::MenuSelector::Scenario,
                )
            };
            let area = ratzilla::ratatui::layout::Rect::new(0, 0, cols, rows);
            let (option_rows, scenario_row, origin_row) = menu_rows(area, count);
            let keys = if let Some(idx) = option_rows.iter().position(|&row| row == cell.1) {
                menu_tap_keys(idx, selected, count)
            } else if scenario_row == Some(cell.1) {
                setup_row_tap_keys(SetupRow::Scenario, scenario_focused)
            } else if origin_row == Some(cell.1) {
                setup_row_tap_keys(SetupRow::Origin, scenario_focused)
            } else {
                return false;
            };
            if keys.is_empty() {
                return false;
            }
            for key in keys {
                route_root_input(&world, &mut shell.borrow_mut(), key);
            }
            true
        }
    });

    let frame_count = Rc::new(RefCell::new(0u32));

    terminal.draw_web({
        let world = world;
        let shell = shell;
        let save_bridge = save_bridge;
        let grid_size = grid_size;
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
                    // Spec 1382: autosave to the reserved slot via the
                    // IndexedDB bridge, and drain bridge completions
                    // into the world's event stream.
                    {
                        let mut bridge = save_bridge.borrow_mut();
                        let now_unix = (js_sys::Date::now() / 1000.0) as u64;
                        let _ = scale::persistence::maybe_autosave(
                            &mut world,
                            &mut *bridge,
                            now_unix,
                        );
                        let completions = bridge.poll_completions();
                        if !completions.is_empty() {
                            // Menu "Load Autosave": a completed load for
                            // the pending slot is applied to the world
                            // and the game starts.
                            if let Some(slot) = bridge.peek_pending_apply() {
                                let loaded = completions.iter().any(|c| {
                                    c.ok
                                        && c.kind
                                            == scale::persistence::SaveLoadKind::Load
                                        && c.slot == slot
                                });
                                if loaded {
                                    let _ = bridge.take_pending_apply();
                                    let store = bridge.clone();
                                    if scale::persistence::load_world(
                                        &mut world, &store, &slot,
                                    )
                                    .is_ok()
                                    {
                                        *world.resource_mut::<GameState>() =
                                            GameState::Running;
                                        world
                                            .resource_mut::<
                                                scale::ui::input::InputContextStack,
                                            >()
                                            .push(scale::ui::input::InputContext::Normal);
                                    }
                                }
                            }
                            // Refresh the menu's saves line from the
                            // bridge cache whenever anything completes.
                            {
                                use scale::persistence::SaveStore;
                                if let Ok(list) = bridge.list() {
                                    let summary = if list.is_empty() {
                                        "No saves yet".to_string()
                                    } else {
                                        list.iter()
                                            .map(|m| {
                                                format!(
                                                    "{} · tick {} · {}",
                                                    m.slot, m.tick, m.colony_name
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                            .join("  |  ")
                                    };
                                    if let Some(mut menu) = world.get_resource_mut::<
                                        scale::ui::menu_state::MenuState,
                                    >() {
                                        menu.save_summary = summary;
                                    }
                                }
                            }
                            if let Some(mut events) = world.get_resource_mut::<
                                bevy_ecs::event::Events<
                                    scale::persistence::SaveLoadCompleted,
                                >,
                            >() {
                                for completion in completions {
                                    events.send(completion);
                                }
                            }
                        }
                    }
                }

                update_camera_smooth(&mut world);
                update_render_cache(&mut world);
            }

            *grid_size.borrow_mut() = (frame.area().width, frame.area().height);
            let world = world.borrow();
            let mut shell = shell.borrow_mut();
            render_with_shell(&world, &mut shell, frame);
        }
    });

    Ok(())
}
