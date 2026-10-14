use crate::egui_tools::EguiRenderer;
use crate::persistance::{Persistence, WINDOW_KEY};
use crate::ui::visualizer::visualizer_widget::RenderArgs;
use egui::{Context, IconData};
use egui_wgpu::wgpu::SurfaceError;
use egui_wgpu::{ScreenDescriptor, wgpu};
use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::event_loop::ControlFlow;
#[cfg(target_os = "windows")]
use winit::platform::windows::WindowAttributesExtWindows;
#[cfg(target_os = "windows")]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Icon, Window, WindowId};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum WindowBackground {
    #[default]
    Solid,
    Mica,
    Acrylic,
}

impl WindowBackground {
    #[cfg(target_os = "windows")]
    fn apply(self, window: &Window) {
        self.extend_dwm_frame(window);

        let result = match self {
            Self::Solid => {
                let _ = window_vibrancy::clear_mica(window);
                let _ = window_vibrancy::clear_acrylic(window);
                let _ = apply_legacy_acrylic(window, None);
                window_vibrancy::clear_blur(window).map_err(|error| error.to_string())
            }
            Self::Mica => {
                let _ = window_vibrancy::clear_blur(window);
                let _ = apply_legacy_acrylic(window, None);
                window_vibrancy::apply_mica(window, Some(true)).map_err(|error| error.to_string())
            }
            Self::Acrylic => {
                let _ = window_vibrancy::clear_mica(window);
                let _ = window_vibrancy::clear_blur(window);
                apply_legacy_acrylic(window, Some((24, 25, 34, 110)))
            }
        };

        if let Err(error) = result {
            debug!("Could not apply {self:?} window background: {error}");
        }
    }

    #[cfg(target_os = "windows")]
    fn extend_dwm_frame(self, window: &Window) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
        use windows::Win32::UI::Controls::MARGINS;

        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };

        // A negative margin tells DWM to paint its backdrop through the whole client area,
        // not only the title bar.
        let margins = if self.is_translucent() {
            MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            }
        } else {
            MARGINS::default()
        };
        if let Err(error) =
            unsafe { DwmExtendFrameIntoClientArea(HWND(handle.hwnd.get() as _), &margins) }
        {
            debug!("Could not extend DWM frame into client area: {error}");
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn apply(self, _: &Window) {}

    pub fn is_translucent(self) -> bool {
        !matches!(self, Self::Solid)
    }
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct WindowCompositionAttribData {
    attribute: u32,
    data: *mut std::ffi::c_void,
    data_size: usize,
}

/// Uses the legacy acrylic compositor directly. Unlike the Windows 11 system backdrop, this is
/// the path that can be re-applied after a window loses focus, matching Windows Terminal's
/// compatibility behavior. `SetWindowCompositionAttribute` is undocumented, so failures simply
/// leave the standard system backdrop in place.
#[cfg(target_os = "windows")]
fn apply_legacy_acrylic(window: &Window, color: Option<(u8, u8, u8, u8)>) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    use windows::core::PCSTR;

    type SetWindowCompositionAttribute =
        unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> i32;

    let handle = window.window_handle().map_err(|error| error.to_string())?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return Err("Window is not a Win32 window".into());
    };
    let module = unsafe { LoadLibraryA(PCSTR(c"user32.dll".as_ptr().cast())) }
        .map_err(|error| error.to_string())?;
    let proc = unsafe {
        GetProcAddress(
            module,
            PCSTR(c"SetWindowCompositionAttribute".as_ptr().cast()),
        )
    }
    .ok_or("SetWindowCompositionAttribute is unavailable")?;
    let set_attribute: SetWindowCompositionAttribute = unsafe { std::mem::transmute(proc) };

    let (r, g, b, a) = color.unwrap_or((0, 0, 0, 0));
    let mut policy = AccentPolicy {
        state: if color.is_some() { 4 } else { 0 }, // ACCENT_ENABLE_ACRYLICBLURBEHIND
        flags: 0,
        gradient_color: u32::from(r)
            | (u32::from(g) << 8)
            | (u32::from(b) << 16)
            | (u32::from(a.max(1)) << 24),
        animation_id: 0,
    };
    let mut data = WindowCompositionAttribData {
        attribute: 0x13, // WCA_ACCENT_POLICY
        data: (&mut policy as *mut AccentPolicy).cast(),
        data_size: std::mem::size_of::<AccentPolicy>(),
    };
    let success = unsafe { set_attribute(HWND(handle.hwnd.get() as _), &mut data) };
    (success != 0)
        .then_some(())
        .ok_or("SetWindowCompositionAttribute failed".into())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct FrameSettings {
    pub vsync: bool,
    pub fps_target: u32,
    /// Missing in older settings, where a nonzero target enabled the cap.
    #[serde(default)]
    pub fps_limit_enabled: Option<bool>,
}
impl Default for FrameSettings {
    fn default() -> Self {
        Self {
            vsync: true,
            fps_target: 60,
            fps_limit_enabled: Some(false),
        }
    }
}
impl FrameSettings {
    fn interval(self) -> Option<Duration> {
        self.fps_limit_enabled
            .unwrap_or(self.fps_target > 0)
            .then(|| Duration::from_secs_f64(1.0 / self.fps_target.clamp(10, 1000) as f64))
    }
    fn present_mode(self) -> wgpu::PresentMode {
        if self.vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        }
    }
}

pub struct AppState {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub surface: wgpu::Surface<'static>,
    pub scale_factor: f32,
    pub egui_renderer: EguiRenderer,
    performance: crate::performance::Performance,
}

impl AppState {
    async fn new(
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        window: &Window,
        width: u32,
        height: u32,
    ) -> Self {
        let power_pref = wgpu::PowerPreference::None;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: power_pref,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .expect("Failed to find an appropriate adapter");

        let timing_features =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        let features = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
            | (adapter.features() & timing_features);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: features,
                required_limits: Default::default(),
                memory_hints: Default::default(),
                trace: Default::default(),
            })
            .await
            .expect("Failed to create device");

        let swapchain_capabilities = surface.get_capabilities(&adapter);
        let alpha_mode = swapchain_capabilities
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::CompositeAlphaMode::PreMultiplied)
            .or_else(|| {
                swapchain_capabilities
                    .alpha_modes
                    .iter()
                    .copied()
                    .find(|mode| *mode == wgpu::CompositeAlphaMode::PostMultiplied)
            });
        if alpha_mode.is_none() {
            log::warn!(
                "The active graphics backend does not support a transparent window surface; supported alpha modes: {:?}",
                swapchain_capabilities.alpha_modes
            );
        }
        let selected_format = wgpu::TextureFormat::Bgra8Unorm;
        let swapchain_format = swapchain_capabilities
            .formats
            .iter()
            .find(|d| **d == selected_format)
            .expect("failed to select proper surface texture format!");

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *swapchain_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 0,
            alpha_mode: alpha_mode.unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
        };

        surface.configure(&device, &surface_config);

        let egui_renderer = EguiRenderer::new(&device, surface_config.format, None, 1, window);
        let performance = crate::performance::Performance::new(&device, &queue);
        let scale_factor = 1.0;

        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        egui_renderer.context().set_fonts(fonts);

        Self {
            device,
            queue,
            surface,
            surface_config,
            egui_renderer,
            performance,
            scale_factor,
        }
    }

    fn resize_surface(&mut self, width: u32, height: u32) {
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct WindowData {
    windows: HashMap<String, WindowRect>,
}

fn window_rect_is_tiny(window_rect: &WindowRect) -> bool {
    window_rect.width < 50 || window_rect.height < 50
}

fn window_rect_is_visible(
    window_rect: &WindowRect,
    monitors: impl IntoIterator<Item = WindowRect>,
) -> bool {
    let window_left = i64::from(window_rect.x);
    let window_top = i64::from(window_rect.y);
    let window_right = window_left + i64::from(window_rect.width);
    let window_bottom = window_top + i64::from(window_rect.height);

    monitors.into_iter().any(|monitor| {
        let monitor_left = i64::from(monitor.x);
        let monitor_top = i64::from(monitor.y);
        let monitor_right = monitor_left + i64::from(monitor.width);
        let monitor_bottom = monitor_top + i64::from(monitor.height);

        window_left < monitor_right
            && window_right > monitor_left
            && window_top < monitor_bottom
            && window_bottom > monitor_top
    })
}

pub struct App {
    instance: wgpu::Instance,
    state: Option<AppState>,
    window: Option<Arc<Window>>,
    icon_data: IconData,
    persistence: Persistence,
    handler: Box<dyn AppHandler>,
    name: &'static str,
    last_save: Instant,
    window_background: WindowBackground,
    next_frame: Instant,
}

pub trait AppHandler {
    fn update(&mut self, ctx: &Context);

    fn save(&mut self, persistence: &mut Persistence);

    fn post_egui(&mut self, args: RenderArgs);

    fn window_background(&self) -> WindowBackground;

    fn frame_settings(&self) -> FrameSettings;
}

impl App {
    pub fn new<F>(name: &'static str, icon: IconData, handler_creator: F) -> Self
    where
        F: FnOnce(&mut Persistence) -> Box<dyn AppHandler>,
    {
        #[cfg(target_os = "windows")]
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            // WGPU's usual DXGI HWND swapchain is opaque on many Windows drivers. The OpenGL
            // backend uses a composited surface that can preserve the alpha cleared by egui.
            backends: wgpu::Backends::GL,
            ..Default::default()
        });
        #[cfg(not(target_os = "windows"))]
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let mut persistence = Persistence::new(name, "WaveSync");

        Self {
            handler: handler_creator(&mut persistence),
            name,
            instance,
            state: None,
            window: None,
            icon_data: icon,
            persistence,
            last_save: Instant::now(),
            window_background: WindowBackground::Solid,
            next_frame: Instant::now(),
        }
    }

    async fn set_window(&mut self, window: Window) {
        let window = Arc::new(window);

        let initial_width = window.inner_size().width;
        let initial_height = window.inner_size().height;

        let surface = self
            .instance
            .create_surface(window.clone())
            .expect("Failed to create surface!");

        let state = AppState::new(
            &self.instance,
            surface,
            &window,
            initial_width,
            initial_height,
        )
        .await;

        self.window.get_or_insert(window);
        self.state.get_or_insert(state);
    }

    fn handle_resized(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.state.as_mut().unwrap().resize_surface(width, height);
        }
    }

    fn handle_redraw(&mut self) {
        #[cfg(feature = "cpu-profiling")]
        if profiling::puffin::are_scopes_on() {
            profiling::finish_frame!();
        }
        profiling::scope!("render.frame");
        // Attempt to handle minimizing window
        if let Some(window) = self.window.as_ref()
            && let Some(min) = window.is_minimized()
            && min
        {
            return;
        }

        let state = self.state.as_mut().unwrap();

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [state.surface_config.width, state.surface_config.height],
            pixels_per_point: self.window.as_ref().unwrap().scale_factor() as f32
                * state.scale_factor,
        };

        let surface_texture = {
            profiling::scope!("wait.acquire_surface");
            state.surface.get_current_texture()
        };

        match surface_texture {
            Err(SurfaceError::Outdated) => {
                // Ignoring outdated to allow resizing and minimization
                println!("wgpu surface outdated");
                return;
            }
            Err(_) => {
                surface_texture.expect("Failed to acquire next swap chain texture");
                return;
            }
            Ok(_) => {}
        };

        let surface_texture = surface_texture.unwrap();

        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let cpu_start = Instant::now();
        let mut encoder = state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        let measured = state.performance.begin(&state.device, &mut encoder);
        state.egui_renderer.context().data_mut(|data| {
            data.insert_temp(egui::Id::new("performance"), state.performance.snapshot())
        });

        // Start every frame with transparent pixels so DWM can compose Mica/Acrylic behind the UI.
        {
            let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear transparent window background"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        let window = self.window.as_ref().unwrap();

        {
            {
                profiling::scope!("egui.begin_frame");
                state.egui_renderer.begin_frame(window);
            }

            let ctx = state.egui_renderer.context();
            self.handler.update(ctx);

            let requested_background = self.handler.window_background();
            if requested_background != self.window_background {
                requested_background.apply(window);
                self.window_background = requested_background;
            }

            state.egui_renderer.end_frame_and_draw(
                &state.device,
                &state.queue,
                &mut encoder,
                window,
                &surface_view,
                &screen_descriptor,
            );

            self.handler.post_egui(RenderArgs {
                encoder: &mut encoder,
                window,
                queue: &state.queue,
                device: &state.device,
                window_surface_view: &surface_view,
                screen_descriptor: &screen_descriptor,
            });
        }

        state.performance.end_gpu(&mut encoder, measured);
        let command_buffer = {
            profiling::scope!("wgpu.finish_encoder");
            encoder.finish()
        };
        // Submission/presentation may block on driver pacing or vsync. Measure
        // frame preparation separately so those waits do not inflate CPU work.
        let cpu_ms = cpu_start.elapsed().as_secs_f64() * 1000.0;
        {
            profiling::scope!("wgpu.submit");
            state.queue.submit(Some(command_buffer));
        }
        state.performance.submitted(measured, cpu_ms);
        {
            profiling::scope!("wait.present");
            surface_texture.present();
        }

        // We could have a thread that saves every 30 seconds,
        // but why bother.
        if self.last_save.elapsed().as_secs() > 30 {
            profiling::scope!("settings.save");
            self.save();
            self.last_save = Instant::now();
        }
    }

    fn save(&mut self) {
        self.handler.save(&mut self.persistence);
        // fetch the window size, and position
        let window = self.window.as_ref().unwrap();
        let size = window.inner_size();
        let position = window.outer_position();

        if size.width == 0 || size.height == 0 {
            debug!(
                "Not saving window position because size is zero: {:?}",
                size
            );
            return;
        }

        if let Ok(position) = position {
            debug!("Saving window position: {:?} {:?}", position, size);
            let window_data = WindowData {
                windows: HashMap::from([(
                    "main".into(),
                    WindowRect {
                        x: position.x,
                        y: position.y,
                        width: size.width,
                        height: size.height,
                    },
                )]),
            };
            self.persistence.set(WINDOW_KEY, &window_data);
        }
        self.persistence.save();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let mut window_attributes = Window::default_attributes()
            .with_title(self.name)
            .with_transparent(true);
        if let Some(window_data) = self.persistence.get::<WindowData>(WINDOW_KEY)
            && let Some(window_rect) = window_data.windows.get("main")
        {
            if !window_rect_is_tiny(window_rect) {
                window_attributes = window_attributes
                    .with_inner_size(PhysicalSize::new(window_rect.width, window_rect.height));
            } else {
                debug!(
                    "Saved window size is too small, using default size instead: {:?}",
                    window_rect
                );
            }

            let monitors = event_loop.available_monitors().map(|monitor| {
                let position = monitor.position();
                let size = monitor.size();
                WindowRect {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                }
            });

            if window_rect_is_visible(window_rect, monitors) {
                debug!("Restoring window position: {:?}", window_rect);
                window_attributes = window_attributes
                    .with_position(PhysicalPosition::new(window_rect.x, window_rect.y));
            } else {
                debug!(
                    "Saved window position is outside the current monitor layout or is too small: {:?}",
                    window_rect
                );
            }
        }
        let icon = Icon::from_rgba(
            self.icon_data.rgba.clone(),
            self.icon_data.width,
            self.icon_data.height,
        )
        .unwrap();
        window_attributes = window_attributes.with_window_icon(Some(icon.clone()));

        #[cfg(target_os = "windows")]
        {
            window_attributes = window_attributes.with_taskbar_icon(Some(icon));
        }

        let window = event_loop.create_window(window_attributes).unwrap();
        let requested_background = self.handler.window_background();
        requested_background.apply(&window);
        self.window_background = requested_background;
        pollster::block_on(self.set_window(window));
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            if window.is_minimized() == Some(true) {
                event_loop.set_control_flow(ControlFlow::Wait);
            } else if Instant::now() >= self.next_frame {
                window.request_redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        // let egui render to process the event first
        self.state
            .as_mut()
            .unwrap()
            .egui_renderer
            .handle_input(self.window.as_ref().unwrap(), &event);

        match event {
            WindowEvent::CloseRequested => {
                info!("The close button was pressed; stopping");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                let frame_start = Instant::now();
                if frame_start < self.next_frame {
                    return;
                }
                let settings = self.handler.frame_settings();
                if let Some(state) = self.state.as_mut() {
                    let mode = settings.present_mode();
                    if state.surface_config.present_mode != mode {
                        state.surface_config.present_mode = mode;
                        state
                            .surface
                            .configure(&state.device, &state.surface_config);
                    }
                }
                self.handle_redraw();
                self.next_frame = self
                    .handler
                    .frame_settings()
                    .interval()
                    .map_or(frame_start, |interval| frame_start + interval);
            }
            WindowEvent::Resized(new_size) => {
                self.next_frame = Instant::now();
                self.handle_resized(new_size.width, new_size.height);
            }
            WindowEvent::Focused(_) if self.window_background == WindowBackground::Acrylic => {
                // Windows can drop legacy acrylic when activation changes; request it again,
                // as Windows Terminal does for its unfocused-acrylic compatibility mode.
                self.window_background.apply(self.window.as_ref().unwrap());
            }
            _ => (),
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::{WindowRect, window_rect_is_visible};

    #[test]
    fn frame_settings_default_to_vsync_without_a_cap() {
        let settings = super::FrameSettings::default();
        assert!(settings.interval().is_none());
        assert_eq!(
            settings.present_mode(),
            egui_wgpu::wgpu::PresentMode::AutoVsync
        );
        let restored: super::FrameSettings = ron::from_str("(fps_target:0)").unwrap();
        assert!(restored.vsync);
        assert_eq!(restored.fps_target, 0);
        assert!(restored.interval().is_none());
    }

    #[test]
    fn fps_cap_has_a_lower_bound_and_preserves_legacy_settings() {
        let mut settings = super::FrameSettings {
            vsync: false,
            fps_target: 1,
            fps_limit_enabled: Some(true),
        };
        assert_eq!(
            settings.interval(),
            Some(std::time::Duration::from_millis(100))
        );
        settings.fps_limit_enabled = Some(false);
        assert!(settings.interval().is_none());
        let restored: super::FrameSettings = ron::from_str("(fps_target:100)").unwrap();
        assert_eq!(
            restored.interval(),
            Some(std::time::Duration::from_millis(10))
        );
    }

    #[test]
    fn frame_target_and_vsync_are_independent() {
        let settings = super::FrameSettings {
            vsync: false,
            fps_target: 100,
            fps_limit_enabled: Some(true),
        };
        assert_eq!(
            settings.interval(),
            Some(std::time::Duration::from_millis(10))
        );
        assert_eq!(
            settings.present_mode(),
            egui_wgpu::wgpu::PresentMode::AutoNoVsync
        );
    }

    fn monitor(x: i32, y: i32, width: u32, height: u32) -> WindowRect {
        WindowRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn accepts_window_inside_monitor() {
        let window = monitor(100, 100, 800, 600);

        assert!(window_rect_is_visible(&window, [monitor(0, 0, 1920, 1080)]));
    }

    #[test]
    fn accepts_window_on_monitor_with_negative_coordinates() {
        let window = monitor(-1800, 100, 800, 600);

        assert!(window_rect_is_visible(
            &window,
            [monitor(-1920, 0, 1920, 1080), monitor(0, 0, 1920, 1080),]
        ));
    }

    #[test]
    fn accepts_partially_visible_window() {
        let window = monitor(1800, 900, 800, 600);

        assert!(window_rect_is_visible(&window, [monitor(0, 0, 1920, 1080)]));
    }

    #[test]
    fn rejects_window_outside_current_monitor_layout() {
        let window = monitor(-1800, 100, 800, 600);

        assert!(!window_rect_is_visible(
            &window,
            [monitor(0, 0, 1920, 1080)]
        ));
    }

    #[test]
    fn touching_monitor_edge_is_not_visible() {
        let window = monitor(1920, 100, 800, 600);

        assert!(!window_rect_is_visible(
            &window,
            [monitor(0, 0, 1920, 1080)]
        ));
    }
}
