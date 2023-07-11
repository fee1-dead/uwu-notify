use std::sync::Arc;

use client::globals::registry_queue_init;
use client::globals::{GlobalList, GlobalListContents};
use client::protocol::*;
use client::Connection;
use client::{self, Dispatch, Proxy, QueueHandle};
use egui::{Event, NumExt, Pos2, Ui, PointerButton};
use glow::HasContext;
use glutin::display::{Display, DisplayApiPreference, GetGlDisplay};
use glutin::prelude::{GlDisplay, NotCurrentGlContextSurfaceAccessor};
use glutin::surface::GlSurface;
use protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;
use protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use sctk::compositor::{CompositorHandler, CompositorState, Surface, SurfaceData};
use sctk::globals::GlobalData;
use sctk::output::{OutputHandler, OutputState};
use sctk::registry::{ProvidesRegistryState, RegistryHandler, RegistryState};
use sctk::seat::keyboard::{keysyms, KeyEvent, KeyboardHandler, Modifiers};
use sctk::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use sctk::seat::{Capability, SeatHandler, SeatState};
use sctk::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceData,
};
use sctk::shell::WaylandSurface;
use sctk::{delegate_compositor, delegate_registry, delegate_seat, registry_handlers};
use sctk::{delegate_keyboard, delegate_layer, delegate_output, delegate_pointer, reexports::*};
use smithay_client_toolkit as sctk;
use wl_compositor::WlCompositor;
use wl_registry::WlRegistry;
use wl_surface::WlSurface;

type GlutinSurface = glutin::surface::Surface<glutin::surface::WindowSurface>;

pub struct State {
    // states
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,

    // things
    keyboard: Option<wl_keyboard::WlKeyboard>,
    keyboard_focus: bool,
    pointer: Option<wl_pointer::WlPointer>,
    layer: LayerSurface,

    pub width: u32,
    pub height: u32,

    gl: Arc<glow::Context>,
    gl_context: glutin::context::PossiblyCurrentContext,
    gl_surface: GlutinSurface,
    painter: crate::paint::EguiGlow,

    exit: bool,
}

impl State {
    pub fn new(
        layer: LayerSurface,
        global_list: &GlobalList,
        qh: &QueueHandle<Self>,
        gl: glow::Context,
        gl_context: glutin::context::PossiblyCurrentContext,
        gl_surface: GlutinSurface,
    ) -> Self {
        let gl = Arc::new(gl);
        let gl2 = gl.clone();
        Self {
            layer,
            exit: false,
            registry_state: RegistryState::new(global_list),
            seat_state: SeatState::new(&global_list, qh),
            output_state: OutputState::new(&global_list, qh),
            keyboard: None,
            keyboard_focus: false,
            width: 256,
            height: 256,
            pointer: None,
            gl,
            gl_context,
            gl_surface,
            painter: crate::paint::EguiGlow::new(gl2, None),
        }
    }

    pub fn wl_surface(&self) -> &wl_surface::WlSurface {
        self.layer.wl_surface()
    }

    pub fn draw(&mut self, qh: &QueueHandle<Self>) {
        self.painter.run([self.width, self.height], |egui_ctx| {
            egui::SidePanel::left("my_side_panel").show(egui_ctx, |ui| {
                ui.heading("Hello World!");
                if ui.button("something").clicked() {
                    println!("hi");
                }
            });
        });
        unsafe {
            self.gl.clear_color(1.0, 0.1, 0.1, 1.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
            self.gl.flush();
            self.painter.paint([self.width, self.height]);

            // draw things on top of egui here

            self.gl_surface.swap_buffers(&self.gl_context).unwrap();
        }
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, self.width as i32, self.height as i32);
        self.layer
            .wl_surface()
            .frame(qh, self.layer.wl_surface().clone());

        self.layer.commit();
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_seat!(State);

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {}
    fn remove_seat(&mut self, conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                let keyboard = self
                    .seat_state
                    .get_keyboard(qh, &seat, None)
                    .expect("Failed to create keyboard");
                self.keyboard = Some(keyboard);
            }
            Capability::Pointer if self.pointer.is_none() => {
                let pointer = self
                    .seat_state
                    .get_pointer(qh, &seat)
                    .expect("Failed to create pointer");
                self.pointer = Some(pointer);
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard => {
                if let Some(keyboard) = self.keyboard.take() {
                    keyboard.release()
                }
            }
            Capability::Pointer => {
                if let Some(pointer) = self.pointer.take() {
                    pointer.release()
                }
            }
            _ => {}
        }
    }
}

delegate_keyboard!(State);

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        keysyms: &[u32],
    ) {
        if self.layer.wl_surface() == surface {
            println!("Keyboard focus on window with pressed syms: {keysyms:?}");
            self.keyboard_focus = true;
        }
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if self.layer.wl_surface() == surface {
            println!("Release keyboard focus on window");
            self.keyboard_focus = false;
        }
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        println!("Key press: {event:?}");
        // press 'esc' to exit
        // TODO
        if event.keysym == keysyms::XKB_KEY_Escape {
            self.exit = true;
        }
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        println!("Key release: {event:?}");
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
    ) {
        println!("Update modifiers: {modifiers:?}");
    }
}

delegate_pointer!(State);

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        use PointerEventKind::*;
        for event in events {
            // Ignore events for other surfaces
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            match event.kind {
                Enter { .. } => {
                    println!("Pointer entered @{:?}", event.position);
                }
                Leave { .. } => {
                    self.painter.input.events.push(Event::PointerGone);
                    println!("Pointer left");
                }
                Motion { .. } => self
                    .painter
                    .input
                    .events
                    .push(Event::PointerMoved(Pos2::new(
                        event.position.0 as f32,
                        event.position.1 as f32,
                    ))),
                Press { button, .. } => {
                    self.painter.input.events.push(Event::PointerButton {
                        pos: Pos2::new(event.position.0 as f32, event.position.1 as f32),
                        // TOOD lol, lmao
                        button: PointerButton::Primary,
                        pressed: true,
                        // TODO impl modifiers
                        modifiers: egui::Modifiers::default(),
                    });
                    println!("Press {:x} @ {:?}", button, event.position);
                }
                Release { button, .. } => {
                    self.painter.input.events.push(Event::PointerButton {
                        pos: Pos2::new(event.position.0 as f32, event.position.1 as f32),
                        // TOOD lol, lmao
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    });
                    println!("Release {:x} @ {:?}", button, event.position);
                }
                Axis {
                    horizontal,
                    vertical,
                    ..
                } => {
                    println!("Scroll H:{horizontal:?}, V:{vertical:?}");
                }
            }
        }
    }
}

delegate_output!(State);

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
    }
}

delegate_layer!(State);

impl LayerShellHandler for State {
    fn closed(
        &mut self,
        conn: &Connection,
        qh: &client::QueueHandle<Self>,
        layer: &sctk::shell::wlr_layer::LayerSurface,
    ) {
    }
    fn configure(
        &mut self,
        conn: &Connection,
        qh: &client::QueueHandle<Self>,
        layer: &sctk::shell::wlr_layer::LayerSurface,
        configure: sctk::shell::wlr_layer::LayerSurfaceConfigure,
        serial: u32,
    ) {
        if configure.new_size.0 == 0 || configure.new_size.1 == 0 {
            self.width = 256;
            self.height = 256;
        } else {
            self.width = configure.new_size.0;
            self.height = configure.new_size.1;
        }
        self.draw(qh)
    }
}

delegate_registry!(State);
delegate_compositor!(State);

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // TODO
        // Not needed for this example.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // TODO
        self.draw(qh);
    }
}

pub fn main() -> color_eyre::Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init::<State>(&conn)?;
    let handle = queue.handle();

    let compositor = CompositorState::bind(&globals, &handle)?;
    let layer_shell = LayerShell::bind(&globals, &handle)?;

    let surface = Surface::new(&compositor, &handle)?;
    let layer_surface =
        layer_shell.create_layer_surface(&handle, surface, Layer::Top, Some("uwu-notify"), None);

    layer_surface.set_anchor(Anchor::TOP | Anchor::RIGHT);
    // TODO do we need keyboard interactivity?
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
    // TODO
    let width = 300;
    let height = 300;
    layer_surface.set_size(width, height);

    layer_surface.commit();

    let display = conn.display();
    let ptr = display.id().as_ptr();

    let mut wl = WaylandDisplayHandle::empty();
    wl.display = ptr.cast();
    let display =
        unsafe { Display::new(RawDisplayHandle::Wayland(wl), DisplayApiPreference::Egl)? };

    let mut window_handle = WaylandWindowHandle::empty();
    window_handle.surface = layer_surface.wl_surface().id().as_ptr().cast();
    let window_handle = RawWindowHandle::Wayland(window_handle);

    let config_template_builder = glutin::config::ConfigTemplateBuilder::new()
        .prefer_hardware_accelerated(None)
        .with_depth_size(0)
        .with_stencil_size(0)
        .with_transparency(false);

    let configs = unsafe { display.find_configs(config_template_builder.build()) };
    let gl_config = configs?.next().unwrap();
    let gl_display = gl_config.display();
    let context_attributes =
        glutin::context::ContextAttributesBuilder::new().build(Some(window_handle));
    // by default, glutin will try to create a core opengl context. but, if it is not available, try to create a gl-es context using this fallback attributes
    let fallback_context_attributes = glutin::context::ContextAttributesBuilder::new()
        .with_context_api(glutin::context::ContextApi::Gles(None))
        .build(Some(window_handle));
    let not_current_gl_context = unsafe {
        gl_display
            .create_context(&gl_config, &context_attributes)
            .unwrap_or_else(|_| {
                        tracing::debug!("failed to create gl_context with attributes: {:?}. retrying with fallback context attributes: {:?}",
                            &context_attributes,
                            &fallback_context_attributes);
                        gl_config
                            .display()
                            .create_context(&gl_config, &fallback_context_attributes)
                            .expect("failed to create context even with fallback attributes")
        })
    };

    let width = std::num::NonZeroU32::new(width.at_least(1)).unwrap();
    let height = std::num::NonZeroU32::new(height.at_least(1)).unwrap();
    let surface_attributes = glutin::surface::SurfaceAttributesBuilder::<
        glutin::surface::WindowSurface,
    >::new()
    .build(window_handle, width, height);
    tracing::debug!(
        "creating surface with attributes: {:?}",
        &surface_attributes
    );
    let gl_surface = unsafe {
        gl_display
            .create_window_surface(&gl_config, &surface_attributes)
            .unwrap()
    };
    tracing::debug!("surface created successfully: {gl_surface:?}.making context current");
    let gl_context = not_current_gl_context.make_current(&gl_surface).unwrap();

    gl_surface
        .set_swap_interval(
            &gl_context,
            glutin::surface::SwapInterval::Wait(std::num::NonZeroU32::new(1).unwrap()),
        )
        .unwrap();

    let glow_context =
        unsafe { glow::Context::from_loader_function_cstr(|x| gl_display.get_proc_address(x)) };

    let mut queue = queue;
    let mut state = State::new(
        layer_surface,
        &globals,
        &handle,
        glow_context,
        gl_context,
        gl_surface,
    );
    /*
    queue.roundtrip(&mut state)?;
    unsafe {
        glow_context.clear_color(1.0, 0.1, 0.1, 1.0);
        glow_context.clear(1);
    }
    gl_surface.swap_buffers(&gl_context)?;
    state.wl_surface().commit();*/

    //let painter = egui_glow::Painter::new(Arc::new(glow_context), "", None)
    //    .map_err(|x| color_eyre::eyre::eyre!("GL error: {x}"))?;

    while !state.exit {
        // TODO use calloop
        queue.blocking_dispatch(&mut state)?;
    }

    Ok(())
}
