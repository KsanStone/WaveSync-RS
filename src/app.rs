use std::collections::HashMap;
use crate::egui_tools::EguiRenderer;
use egui::{Context, IconData};
use egui_wgpu::wgpu::SurfaceError;
use egui_wgpu::{ScreenDescriptor, wgpu};
use std::sync::Arc;
use std::time::Instant;
use log::{debug, info};
use serde::{Deserialize, Serialize};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};
use crate::persistance::{Persistence, WINDOW_KEY};
use crate::ui::visualizer::visualizer_widget::RenderArgs;

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
            alpha_mode: swapchain_capabilities.alpha_modes[0],
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

pub struct App {
    instance: wgpu::Instance,
    state: Option<AppState>,
    window: Option<Arc<Window>>,
    _icon_data: IconData,
    persistence: Persistence,
    handler: Box<dyn AppHandler>,
    name: &'static str,
    last_save: Instant,
}

pub trait AppHandler {
    fn update(&mut self, ctx: &Context);

    fn save(&mut self, persistence: &mut Persistence);

    fn post_egui(
        &mut self,
        args: RenderArgs,
    );
}

impl App {
    pub fn new<F>(name: &'static str, icon: IconData, handler_creator: F) -> Self
    where
        F: FnOnce(&mut Persistence) -> Box<dyn AppHandler>,
    {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let mut persistence = Persistence::new(name, "WaveSync");

        Self {
            handler: handler_creator(&mut persistence),
            name,
            instance,
            state: None,
            window: None,
            _icon_data: icon,
            persistence,
            last_save: Instant::now(),
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

        let window = self.window.as_ref().unwrap();

        {
            state.egui_renderer.begin_frame(window);

            let ctx = state.egui_renderer.context();
            self.handler.update(ctx);

            state.egui_renderer.end_frame_and_draw(
                &state.device,
                &state.queue,
                &mut encoder,
                window,
                &surface_view,
                &screen_descriptor,
            );

            let callback_resources = &state.egui_renderer.renderer.callback_resources;
            self.handler.post_egui(
                RenderArgs {
                    encoder: &mut encoder,
                    window,
                    queue: &state.queue,
                    device: &state.device,
                    resources: callback_resources,
                    window_surface_view: &surface_view,
                    screen_descriptor: &screen_descriptor,
                }
            );
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
        if let Ok(position) = position {
            debug!("Saving window position: {:?} {:?}", position, size);
            let window_data = WindowData {
                windows: HashMap::from([
                    ("main".into(), WindowRect {
                        x: position.x,
                        y: position.y,
                        width: size.width,
                        height: size.height,
                    })
                ]),
            };
            self.persistence.set(WINDOW_KEY, &window_data);
        }
        self.persistence.save();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {

        let mut window_attributes = Window::default_attributes().with_title(self.name);
        if let Some(window_data) = self.persistence.get::<WindowData>(WINDOW_KEY)
            && let Some(window_rect) = window_data.windows.get("main") {
                debug!("Restoring window position: {:?}", window_rect);
                window_attributes = window_attributes
                    .with_inner_size(PhysicalSize::new(window_rect.width, window_rect.height))
                    .with_position(PhysicalPosition::new(window_rect.x, window_rect.y));
            }

        let window = event_loop
            .create_window(window_attributes)
            .unwrap();
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