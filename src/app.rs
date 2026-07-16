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
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
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
                window_vibrancy::clear_blur(window)
            }
            Self::Mica => {
                let _ = window_vibrancy::clear_blur(window);
                window_vibrancy::apply_mica(window, Some(true))
            }
            Self::Acrylic => {
                let _ = window_vibrancy::clear_mica(window);
                let _ = window_vibrancy::clear_blur(window);
                window_vibrancy::apply_acrylic(window, Some((24, 25, 34, 110)))
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
        if let Err(error) = unsafe {
            DwmExtendFrameIntoClientArea(HWND(handle.hwnd.get() as _), &margins)
        } {
            debug!("Could not extend DWM frame into client area: {error}");
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn apply(self, _: &Window) {}

    pub fn is_translucent(self) -> bool {
        !matches!(self, Self::Solid)
    }
}

pub struct AppState {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub surface: wgpu::Surface<'static>,
    pub scale_factor: f32,
    pub egui_renderer: EguiRenderer,
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

        let features = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
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
}

pub trait AppHandler {
    fn update(&mut self, ctx: &Context);

    fn save(&mut self, persistence: &mut Persistence);

    fn post_egui(&mut self, args: RenderArgs);

    fn window_background(&self) -> WindowBackground;
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

        let surface_texture = state.surface.get_current_texture();

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

        let mut encoder = state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

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
            state.egui_renderer.begin_frame(window);

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

        state.queue.submit(Some(encoder.finish()));
        surface_texture.present();

        // We could have a thread that saves every 30 seconds,
        // but why bother.
        if self.last_save.elapsed().as_secs() > 30 {
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
            debug!("Not saving window position because size is zero: {:?}", size);
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
                self.handle_redraw();

                self.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::Resized(new_size) => {
                self.handle_resized(new_size.width, new_size.height);
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
