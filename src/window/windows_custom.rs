//! The kiss3d window.
/*
 * FIXME: this file is too big. Some heavy refactoring need to be done here.
 */
use std::cell::RefCell;
use std::iter::repeat;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use instant::Instant;
use na::{Point2, Point3, Vector2, Vector3};

use crate::camera::Camera;
use crate::context::Context;
use crate::event::{Action, EventManager, Key, WindowEvent};
use crate::light::Light;
use crate::planar_camera::PlanarCamera;
use crate::post_processing::PostProcessingEffect;
#[cfg(feature = "conrod")]
use crate::renderer::ConrodRenderer;
use crate::renderer::{PlanarRenderer, Renderer};
use crate::resource::{FramebufferManager, RenderTarget, Texture, TextureManager};
use crate::text::{Font, TextRenderer};
use crate::window::canvas::CanvasSetup;
use crate::window::{Canvas, ExtendedState};
use image::imageops;
use image::{GenericImage, Pixel};
use image::{ImageBuffer, Rgb};

#[cfg(feature = "conrod")]
use std::collections::HashMap;

static DEFAULT_WIDTH: u32 = 800u32;
static DEFAULT_HEIGHT: u32 = 600u32;

#[cfg(feature = "conrod")]
struct ConrodContext {
    renderer: ConrodRenderer,
    textures: conrod::image::Map<(Rc<Texture>, (u32, u32))>,
    texture_ids: HashMap<String, conrod::image::Id>,
}

#[cfg(feature = "conrod")]
impl ConrodContext {
    fn new(width: f64, height: f64) -> Self {
        Self {
            renderer: ConrodRenderer::new(width, height),
            textures: conrod::image::Map::new(),
            texture_ids: HashMap::new(),
        }
    }
}

// Rendering mode used by the program, you can only switch
// if two d data is provided.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RenderMode {
    ThreeD,
    TwoD,
}

/// Structure representing a window and a 3D scene.
///
/// This is the main interface with the 3d engine.
pub struct CustomWindow {
    events: Rc<Receiver<WindowEvent>>,
    unhandled_events: Rc<RefCell<Vec<WindowEvent>>>,
    canvas: Canvas,
    max_dur_per_frame: Option<Duration>,
    light_mode: Light, // FIXME: move that to the scene graph
    background: Vector3<f32>,
    text_renderer: TextRenderer,
    framebuffer_manager: FramebufferManager,
    post_process_render_target: RenderTarget,
    #[cfg(not(target_arch = "wasm32"))]
    curr_time: Instant,
    should_close: bool,
    rendering_mode: RenderMode,
    #[cfg(feature = "conrod")]
    conrod_context: ConrodContext,
}

impl CustomWindow {
    /// Indicates whether this window should be closed.
    #[inline]
    pub fn should_close(&self) -> bool {
        self.should_close
    }

    /// The window width.
    #[inline]
    pub fn width(&self) -> u32 {
        self.canvas.size().0
    }

    /// The window height.
    #[inline]
    pub fn height(&self) -> u32 {
        self.canvas.size().1
    }

    /// The size of the window.
    #[inline]
    pub fn size(&self) -> Vector2<u32> {
        let (w, h) = self.canvas.size();
        Vector2::new(w, h)
    }

    /// Sets the maximum number of frames per second. Cannot be 0. `None` means there is no limit.
    #[inline]
    pub fn set_framerate_limit(&mut self, fps: Option<u64>) {
        self.max_dur_per_frame = fps.map(|f| {
            assert!(f != 0);
            Duration::from_millis(1000 / f)
        })
    }

    /// Switch the rendering mode
    #[inline]
    pub fn switch_rendering_mode(&mut self) {
        self.rendering_mode = match self.rendering_mode {
            RenderMode::ThreeD => RenderMode::TwoD,
            RenderMode::TwoD => RenderMode::ThreeD,
        }
    }

    /// Set window title
    pub fn set_title(&mut self, title: &str) {
        self.canvas.set_title(title)
    }

    /// Set the window icon. On wasm this does nothing.
    ///
    /// ```rust,should_panic
    /// # extern crate kiss3d;
    /// # extern crate image;
    /// # use kiss3d::window::Window;
    ///
    /// # fn main() -> Result<(), image::ImageError> {
    /// #    let mut window = Window::new("");
    /// window.set_icon(image::open("foo.ico")?);
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_icon(&mut self, icon: impl GenericImage<Pixel = impl Pixel<Subpixel = u8>>) {
        self.canvas.set_icon(icon)
    }

    /// Set the cursor grabbing behaviour.
    ///
    /// If cursor grabbing is on, the cursor is prevented from leaving the window.
    /// Does nothing on web platforms.
    pub fn set_cursor_grab(&self, grab: bool) {
        self.canvas.set_cursor_grab(grab);
    }

    /// Closes the window.
    #[inline]
    pub fn close(&mut self) {
        self.should_close = true;
    }

    /// Hides the window, without closing it. Use `show` to make it visible again.
    #[inline]
    pub fn hide(&mut self) {
        self.canvas.hide()
    }

    /// Makes the window visible. Use `hide` to hide it.
    #[inline]
    pub fn show(&mut self) {
        self.canvas.show()
    }

    /// Sets the background color.
    #[inline]
    pub fn set_background_color(&mut self, r: f32, g: f32, b: f32) {
        self.background.x = r;
        self.background.y = g;
        self.background.z = b;
    }

    /// Adds a string to be drawn during the next frame.
    #[inline]
    pub fn draw_text(
        &mut self,
        text: &str,
        pos: &Point2<f32>,
        scale: f32,
        font: &Rc<Font>,
        color: &Point3<f32>,
    ) {
        self.text_renderer.draw_text(text, pos, scale, font, color);
    }

    /// Load a texture from a file and return a reference to it.
    pub fn add_texture(&mut self, path: &Path, name: &str) -> Rc<Texture> {
        TextureManager::get_global_manager(|tm| tm.add(path, name))
    }

    /// Returns whether this window is closed or not.
    pub fn is_closed(&self) -> bool {
        false // FIXME
    }

    /// The hidpi factor of this screen.
    pub fn hidpi_factor(&self) -> f64 {
        self.canvas.hidpi_factor()
    }

    /// Sets the light mode. Only one light is supported.
    pub fn set_light(&mut self, pos: Light) {
        self.light_mode = pos;
    }

    /// Retrieve a mutable reference to the UI based on Conrod.
    #[cfg(feature = "conrod")]
    pub fn conrod_ui_mut(&mut self) -> &mut conrod::Ui {
        self.conrod_context.renderer.ui_mut()
    }

    /// Attributes a conrod ID to the given texture and returns it if it exists.
    #[cfg(feature = "conrod")]
    pub fn conrod_texture_id(&mut self, name: &str) -> Option<conrod::image::Id> {
        let tex = TextureManager::get_global_manager(|tm| tm.get_with_size(name))?;
        let textures = &mut self.conrod_context.textures;
        Some(
            *self
                .conrod_context
                .texture_ids
                .entry(name.to_string())
                .or_insert_with(|| textures.insert(tex)),
        )
    }

    /// Retrieve a reference to the UI based on Conrod.
    #[cfg(feature = "conrod")]
    pub fn conrod_ui(&self) -> &conrod::Ui {
        self.conrod_context.renderer.ui()
    }

    /// Returns `true` if the mouse is currently interacting with a Conrod widget.
    #[cfg(feature = "conrod")]
    pub fn is_conrod_ui_capturing_mouse(&self) -> bool {
        let ui = self.conrod_ui();
        let state = &ui.global_input().current;
        let window_id = Some(ui.window);

        state.widget_capturing_mouse.is_some() && state.widget_capturing_mouse != window_id
    }

    /// Returns `true` if the keyboard is currently interacting with a Conrod widget.
    #[cfg(feature = "conrod")]
    pub fn is_conrod_ui_capturing_keyboard(&self) -> bool {
        let ui = self.conrod_ui();
        let state = &ui.global_input().current;
        let window_id = Some(ui.window);

        state.widget_capturing_keyboard.is_some() && state.widget_capturing_keyboard != window_id
    }

    /// Opens a window, hide it then calls a user-defined procedure.
    ///
    /// # Arguments
    /// * `title` - the window title
    pub fn new_hidden(title: &str) -> CustomWindow {
        CustomWindow::do_new(title, true, DEFAULT_WIDTH, DEFAULT_HEIGHT, None)
    }

    /// Opens a window then calls a user-defined procedure.
    ///
    /// # Arguments
    /// * `title` - the window title
    pub fn new(title: &str) -> CustomWindow {
        CustomWindow::do_new(title, false, DEFAULT_WIDTH, DEFAULT_HEIGHT, None)
    }

    /// Opens a window with a custom size then calls a user-defined procedure.
    ///
    /// # Arguments
    /// * `title` - the window title.
    /// * `width` - the window width.
    /// * `height` - the window height.
    pub fn new_with_size(title: &str, width: u32, height: u32) -> CustomWindow {
        CustomWindow::do_new(title, false, width, height, None)
    }

    // FIXME: make this pub?
    fn do_new(
        title: &str,
        hide: bool,
        width: u32,
        height: u32,
        setup: Option<CanvasSetup>,
    ) -> CustomWindow {
        let (event_send, event_receive) = mpsc::channel();
        let canvas = Canvas::open(title, hide, width, height, setup, event_send);

        init_gl();

        let mut usr_window = CustomWindow {
            should_close: false,
            max_dur_per_frame: None,
            canvas: canvas,
            events: Rc::new(event_receive),
            unhandled_events: Rc::new(RefCell::new(Vec::new())),
            light_mode: Light::Absolute(Point3::new(0.0, 10.0, 0.0)),
            background: Vector3::new(0.0, 0.0, 0.0),
            text_renderer: TextRenderer::new(),
            #[cfg(feature = "conrod")]
            conrod_context: ConrodContext::new(width as f64, height as f64),
            post_process_render_target: FramebufferManager::new_render_target(
                width as usize,
                height as usize,
                true,
            ),
            framebuffer_manager: FramebufferManager::new(),
            #[cfg(not(target_arch = "wasm32"))]
            curr_time: Instant::now(),
            rendering_mode: RenderMode::ThreeD,
        };

        if hide {
            usr_window.canvas.hide()
        }

        // usr_window.framebuffer_size_callback(DEFAULT_WIDTH, DEFAULT_HEIGHT);
        let light = usr_window.light_mode.clone();
        usr_window.set_light(light);

        usr_window
    }

    // FIXME: give more options for the snap size and offset.
    /// Read the pixels currently displayed to the screen.
    ///
    /// # Arguments:
    /// * `out` - the output buffer. It is automatically resized.
    pub fn snap(&self, out: &mut Vec<u8>) {
        let (width, height) = self.canvas.size();
        self.snap_rect(out, 0, 0, width as usize, height as usize)
    }

    /// Read a section of pixels from the screen
    ///
    /// # Arguments:
    /// * `out` - the output buffer. It is automatically resized
    /// * `x, y, width, height` - the rectangle to capture
    pub fn snap_rect(&self, out: &mut Vec<u8>, x: usize, y: usize, width: usize, height: usize) {
        let size = (width * height * 3) as usize;

        if out.len() < size {
            let diff = size - out.len();
            out.extend(repeat(0).take(diff));
        } else {
            out.truncate(size)
        }

        // FIXME: this is _not_ the fastest way of doing this.
        let ctxt = Context::get();
        ctxt.pixel_storei(Context::PACK_ALIGNMENT, 1);
        ctxt.read_pixels(
            x as i32,
            y as i32,
            width as i32,
            height as i32,
            Context::RGB,
            Some(out),
        );
    }

    /// Get the current screen as an image
    pub fn snap_image(&self) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
        let (width, height) = self.canvas.size();
        let mut buf = Vec::new();
        self.snap(&mut buf);
        let img_opt = ImageBuffer::from_vec(width as u32, height as u32, buf);
        let img = img_opt.expect("Buffer created from window was not big enough for image.");
        imageops::flip_vertical(&img)
    }

    /// Gets the events manager that gives access to an event iterator.
    pub fn events(&self) -> EventManager {
        EventManager::new(self.events.clone(), self.unhandled_events.clone())
    }

    /// Gets the status of a key.
    pub fn get_key(&self, key: Key) -> Action {
        self.canvas.get_key(key)
    }

    /// Gets the last known position of the mouse.
    ///
    /// The position of the mouse is automatically updated when the mouse moves over the canvas.
    pub fn cursor_pos(&self) -> Option<(f64, f64)> {
        self.canvas.cursor_pos()
    }

    #[inline]
    fn handle_events(
        &mut self,
        camera: &mut Option<&mut dyn Camera>,
        planar_camera: &mut Option<&mut dyn PlanarCamera>,
    ) {
        let unhandled_events = self.unhandled_events.clone(); // FIXME: could we avoid the clone?
        let events = self.events.clone(); // FIXME: could we avoid the clone?

        for event in unhandled_events.borrow().iter() {
            self.handle_event(camera, planar_camera, event)
        }

        for event in events.try_iter() {
            self.handle_event(camera, planar_camera, &event)
        }

        unhandled_events.borrow_mut().clear();
        self.canvas.poll_events();
    }

    fn handle_event(
        &mut self,
        camera: &mut Option<&mut dyn Camera>,
        planar_camera: &mut Option<&mut dyn PlanarCamera>,
        event: &WindowEvent,
    ) {
        match *event {
            WindowEvent::Key(Key::Escape, Action::Release, _) | WindowEvent::Close => {
                self.close();
            }
            WindowEvent::FramebufferSize(w, h) => {
                self.update_viewport(w as f32, h as f32);
            }
            _ => {}
        }

        #[cfg(feature = "conrod")]
        fn window_event_to_conrod_input(
            event: WindowEvent,
            size: Vector2<u32>,
            hidpi: f64,
        ) -> Option<conrod::event::Input> {
            use conrod::event::Input;
            use conrod::input::{Button, Key as CKey, Motion, MouseButton};

            let transform_coords = |x: f64, y: f64| {
                (
                    (x - size.x as f64 / 2.0) / hidpi,
                    -(y - size.y as f64 / 2.0) / hidpi,
                )
            };

            match event {
                WindowEvent::FramebufferSize(w, h) => {
                    Some(Input::Resize(w as f64 / hidpi, h as f64 / hidpi))
                }
                WindowEvent::Focus(focus) => Some(Input::Focus(focus)),
                WindowEvent::CursorPos(x, y, _) => {
                    let (x, y) = transform_coords(x, y);
                    Some(Input::Motion(Motion::MouseCursor { x, y }))
                }
                WindowEvent::Scroll(x, y, _) => Some(Input::Motion(Motion::Scroll { x, y: -y })),
                WindowEvent::MouseButton(button, action, _) => {
                    let button = match button {
                        crate::event::MouseButton::Button1 => MouseButton::Left,
                        crate::event::MouseButton::Button2 => MouseButton::Right,
                        crate::event::MouseButton::Button3 => MouseButton::Middle,
                        crate::event::MouseButton::Button4 => MouseButton::X1,
                        crate::event::MouseButton::Button5 => MouseButton::X2,
                        crate::event::MouseButton::Button6 => MouseButton::Button6,
                        crate::event::MouseButton::Button7 => MouseButton::Button7,
                        crate::event::MouseButton::Button8 => MouseButton::Button8,
                    };

                    match action {
                        Action::Press => Some(Input::Press(Button::Mouse(button))),
                        Action::Release => Some(Input::Release(Button::Mouse(button))),
                    }
                }
                WindowEvent::Key(key, action, _) => {
                    let key = match key {
                        Key::Key1 => CKey::D1,
                        Key::Key2 => CKey::D2,
                        Key::Key3 => CKey::D3,
                        Key::Key4 => CKey::D4,
                        Key::Key5 => CKey::D5,
                        Key::Key6 => CKey::D6,
                        Key::Key7 => CKey::D7,
                        Key::Key8 => CKey::D8,
                        Key::Key9 => CKey::D9,
                        Key::Key0 => CKey::D0,
                        Key::A => CKey::A,
                        Key::B => CKey::B,
                        Key::C => CKey::C,
                        Key::D => CKey::D,
                        Key::E => CKey::E,
                        Key::F => CKey::F,
                        Key::G => CKey::G,
                        Key::H => CKey::H,
                        Key::I => CKey::I,
                        Key::J => CKey::J,
                        Key::K => CKey::K,
                        Key::L => CKey::L,
                        Key::M => CKey::M,
                        Key::N => CKey::N,
                        Key::O => CKey::O,
                        Key::P => CKey::P,
                        Key::Q => CKey::Q,
                        Key::R => CKey::R,
                        Key::S => CKey::S,
                        Key::T => CKey::T,
                        Key::U => CKey::U,
                        Key::V => CKey::V,
                        Key::W => CKey::W,
                        Key::X => CKey::X,
                        Key::Y => CKey::Y,
                        Key::Z => CKey::Z,
                        Key::Escape => CKey::Escape,
                        Key::F1 => CKey::F1,
                        Key::F2 => CKey::F2,
                        Key::F3 => CKey::F3,
                        Key::F4 => CKey::F4,
                        Key::F5 => CKey::F5,
                        Key::F6 => CKey::F6,
                        Key::F7 => CKey::F7,
                        Key::F8 => CKey::F8,
                        Key::F9 => CKey::F9,
                        Key::F10 => CKey::F10,
                        Key::F11 => CKey::F11,
                        Key::F12 => CKey::F12,
                        Key::F13 => CKey::F13,
                        Key::F14 => CKey::F14,
                        Key::F15 => CKey::F15,
                        Key::F16 => CKey::F16,
                        Key::F17 => CKey::F17,
                        Key::F18 => CKey::F18,
                        Key::F19 => CKey::F19,
                        Key::F20 => CKey::F20,
                        Key::F21 => CKey::F21,
                        Key::F22 => CKey::F22,
                        Key::F23 => CKey::F23,
                        Key::F24 => CKey::F24,
                        Key::Pause => CKey::Pause,
                        Key::Insert => CKey::Insert,
                        Key::Home => CKey::Home,
                        Key::Delete => CKey::Delete,
                        Key::End => CKey::End,
                        Key::PageDown => CKey::PageDown,
                        Key::PageUp => CKey::PageUp,
                        Key::Left => CKey::Left,
                        Key::Up => CKey::Up,
                        Key::Right => CKey::Right,
                        Key::Down => CKey::Down,
                        Key::Return => CKey::Return,
                        Key::Space => CKey::Space,
                        Key::Caret => CKey::Caret,
                        Key::Numpad0 => CKey::NumPad0,
                        Key::Numpad1 => CKey::NumPad1,
                        Key::Numpad2 => CKey::NumPad2,
                        Key::Numpad3 => CKey::NumPad3,
                        Key::Numpad4 => CKey::NumPad4,
                        Key::Numpad5 => CKey::NumPad5,
                        Key::Numpad6 => CKey::NumPad6,
                        Key::Numpad7 => CKey::NumPad7,
                        Key::Numpad8 => CKey::NumPad8,
                        Key::Numpad9 => CKey::NumPad9,
                        Key::Add => CKey::Plus,
                        Key::At => CKey::At,
                        Key::Backslash => CKey::Backslash,
                        Key::Calculator => CKey::Calculator,
                        Key::Colon => CKey::Colon,
                        Key::Comma => CKey::Comma,
                        Key::Equals => CKey::Equals,
                        Key::LBracket => CKey::LeftBracket,
                        Key::LControl => CKey::LCtrl,
                        Key::LShift => CKey::LShift,
                        Key::Mail => CKey::Mail,
                        Key::MediaSelect => CKey::MediaSelect,
                        Key::Minus => CKey::Minus,
                        Key::Mute => CKey::Mute,
                        Key::NumpadComma => CKey::NumPadComma,
                        Key::NumpadEnter => CKey::NumPadEnter,
                        Key::NumpadEquals => CKey::NumPadEquals,
                        Key::Period => CKey::Period,
                        Key::Power => CKey::Power,
                        Key::RAlt => CKey::RAlt,
                        Key::RBracket => CKey::RightBracket,
                        Key::RControl => CKey::RCtrl,
                        Key::RShift => CKey::RShift,
                        Key::Semicolon => CKey::Semicolon,
                        Key::Slash => CKey::Slash,
                        Key::Sleep => CKey::Sleep,
                        Key::Stop => CKey::Stop,
                        Key::Tab => CKey::Tab,
                        Key::VolumeDown => CKey::VolumeDown,
                        Key::VolumeUp => CKey::VolumeUp,
                        Key::Copy => CKey::Copy,
                        Key::Paste => CKey::Paste,
                        Key::Cut => CKey::Cut,
                        _ => CKey::Unknown,
                    };

                    match action {
                        Action::Press => Some(Input::Press(Button::Keyboard(key))),
                        Action::Release => Some(Input::Release(Button::Keyboard(key))),
                    }
                }
                _ => None,
            }
        }

        #[cfg(feature = "conrod")]
        {
            let (size, hidpi) = (self.size(), self.hidpi_factor());
            let conrod_ui = self.conrod_ui_mut();
            if let Some(input) = window_event_to_conrod_input(*event, size, hidpi) {
                conrod_ui.handle_event(input);
            }

            let state = &conrod_ui.global_input().current;
            let window_id = Some(conrod_ui.window);

            if event.is_keyboard_event()
                && state.widget_capturing_keyboard.is_some()
                && state.widget_capturing_keyboard != window_id
            {
                return;
            }

            if event.is_mouse_event()
                && state.widget_capturing_mouse.is_some()
                && state.widget_capturing_mouse != window_id
            {
                return;
            }
        }

        // Only handle events for the current mode.
        match self.rendering_mode {
            RenderMode::ThreeD => match *camera {
                Some(ref mut cam) => cam.handle_event(&self.canvas, event),
                None => (),
            },
            RenderMode::TwoD => match *planar_camera {
                Some(ref mut cam) => cam.handle_event(&self.canvas, event),
                None => (),
            },
        }
    }

    /// Runs the render and event loop until the window is closed.
    pub fn render_loop<S: ExtendedState>(mut self, mut state: S) {
        Canvas::render_loop(move |_| self.do_render_with_state(&mut state))
    }

    /// Render one frame using the specified state.
    #[cfg(not(any(target_arch = "wasm32", target_arch = "asmjs")))]
    pub fn render_with_state<S: ExtendedState>(&mut self, state: &mut S) -> bool {
        self.do_render_with_state(state)
    }

    fn do_render_with_state<S: ExtendedState>(&mut self, state: &mut S) -> bool {
        {
            let (camera, planar_camera, renderer, planar_renderer, effect) =
                state.cameras_and_effect_and_renderers();
            self.should_close =
                !self.do_render_with(camera, planar_camera, renderer, planar_renderer, effect);
        }

        if !self.should_close {
            state.step(self)
        }

        !self.should_close
    }

    fn do_render_with(
        &mut self,
        camera: Option<&mut dyn Camera>,
        planar_camera: Option<&mut dyn PlanarCamera>,
        renderer: Option<&mut dyn Renderer>,
        planar_renderer: Option<&mut dyn PlanarRenderer>,
        post_processing: Option<&mut dyn PostProcessingEffect>,
    ) -> bool {
        let mut camera = camera;
        let mut planar_camera = planar_camera;
        self.handle_events(&mut camera, &mut planar_camera);

        match (camera, planar_camera) {
            (Some(cam), Some(cam_planar)) => self.render_single_frame(
                cam,
                cam_planar,
                renderer,
                planar_renderer,
                post_processing,
            ),
            // TODO: Fallback to basic camera instead of crashing
            _ => panic!("No cameras available"),
        }
    }

    fn render_single_frame(
        &mut self,
        camera: &mut dyn Camera,
        planar_camera: &mut dyn PlanarCamera,
        mut renderer: Option<&mut dyn Renderer>,
        mut planar_renderer: Option<&mut dyn PlanarRenderer>,
        mut post_processing: Option<&mut dyn PostProcessingEffect>,
    ) -> bool {
        let w = self.width();
        let h = self.height();

        planar_camera.handle_event(&self.canvas, &WindowEvent::FramebufferSize(w, h));
        camera.handle_event(&self.canvas, &WindowEvent::FramebufferSize(w, h));
        planar_camera.update(&self.canvas);
        camera.update(&self.canvas);

        match self.light_mode {
            Light::StickToCamera => self.set_light(Light::StickToCamera),
            _ => {}
        }

        if post_processing.is_some() {
            // if we need post-processing, render to our own frame buffer
            self.framebuffer_manager
                .select(&self.post_process_render_target);
        } else {
            self.framebuffer_manager
                .select(&FramebufferManager::screen());
        }

        self.clear_screen();

        match self.rendering_mode {
            RenderMode::ThreeD => {
                // Draw the 3D scene
                if let Some(ref mut renderer) = renderer {
                    renderer.render(1usize, camera)
                }
            }
            RenderMode::TwoD => {
                // Draw the 2D scene
                if let Some(ref mut renderer) = planar_renderer {
                    renderer.render(planar_camera);
                }
            }
        }

        let (znear, zfar) = camera.clip_planes();

        if let Some(ref mut p) = post_processing {
            // switch back to the screen framebuffer …
            self.framebuffer_manager
                .select(&FramebufferManager::screen());
            // … and execute the post-process
            // FIXME: use the real time value instead of 0.016!
            p.update(0.016, w as f32, h as f32, znear, zfar);
            p.draw(&self.post_process_render_target);
        }

        // TODO: Seperate ui rendering based on viewing mode?
        self.text_renderer.render(w as f32, h as f32);
        #[cfg(feature = "conrod")]
        self.conrod_context.renderer.render(
            w as f32,
            h as f32,
            self.canvas.hidpi_factor() as f32,
            &self.conrod_context.textures,
        );

        // We are done: swap buffers
        self.canvas.swap_buffers();

        #[cfg(not(target_arch = "wasm32"))]
        {
            // Limit the fps if needed.
            if let Some(dur) = self.max_dur_per_frame {
                let elapsed = self.curr_time.elapsed();
                if elapsed < dur {
                    thread::sleep(dur - elapsed);
                }
            }

            self.curr_time = Instant::now();
        }

        // self.transparent_objects.clear();
        // self.opaque_objects.clear();

        !self.should_close()
    }

    fn clear_screen(&self) {
        let ctxt = Context::get();
        // Activate the default texture
        verify!(ctxt.active_texture(Context::TEXTURE0));
        // Clear the screen to black
        verify!(ctxt.clear_color(self.background.x, self.background.y, self.background.z, 1.0));
        verify!(ctxt.clear(Context::COLOR_BUFFER_BIT));
        verify!(ctxt.clear(Context::DEPTH_BUFFER_BIT));
    }

    fn update_viewport(&mut self, w: f32, h: f32) {
        // Update the viewport
        verify!(Context::get().scissor(0, 0, w as i32, h as i32));
        FramebufferManager::screen().resize(w, h);
        self.post_process_render_target.resize(w, h);
    }
}

fn init_gl() {
    /*
     * Misc configurations
     */
    let ctxt = Context::get();
    verify!(ctxt.front_face(Context::CCW));
    verify!(ctxt.enable(Context::DEPTH_TEST));
    verify!(ctxt.enable(Context::SCISSOR_TEST));
    #[cfg(not(any(target_arch = "wasm32", target_arch = "asmjs")))]
    {
        verify!(ctxt.enable(Context::PROGRAM_POINT_SIZE));
    }
    verify!(ctxt.depth_func(Context::LEQUAL));
    verify!(ctxt.front_face(Context::CCW));
    verify!(ctxt.enable(Context::CULL_FACE));
    verify!(ctxt.cull_face(Context::BACK));
}
