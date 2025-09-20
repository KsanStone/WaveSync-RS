#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![allow(rustdoc::missing_crate_level_docs)] // it's an example

use eframe::{egui, egui_wgpu, wgpu};
use eframe::egui::PaintCallbackInfo;
use eframe::egui_wgpu::RenderState;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

fn main() -> eframe::Result {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native(
        "My egui App",
        options,
        Box::new(|cc| {
            // This gives us image support:
            egui_extras::install_image_loaders(&cc.egui_ctx);

            Ok(Box::<MyApp>::default())
        }),
    )
}

struct MyApp {
    name: String,
    age: u32,
}

impl Default for MyApp {
    fn default() -> Self {
        Self {
            name: "Arthur".to_owned(),
            age: 42,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("My egui Application");
            ui.horizontal(|ui| {
                let name_label = ui.label("Your name: ");
                ui.text_edit_singleline(&mut self.name)
                    .labelled_by(name_label.id);
            });
            ui.add(egui::Slider::new(&mut self.age, 0..=120).text("age"));
            if ui.button("Increment").clicked() {
                self.age += 1;
            }
            ui.label(format!("Hello '{}', age {}", self.name, self.age));
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                egui::Rect::from_min_size(ui.min_rect().min, ui.min_rect().size()),
                CustomCallback {},
            ));
        });
    }
}

struct CustomCallback;

impl CallbackTrait for CustomCallback {
    fn paint(
        &self,
        _info: PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &CallbackResources,
    ) {
        // Assume we already created pipeline elsewhere (e.g. in resources)
        // Here we'll just set a pipeline and draw
        if let Some(pipeline) = _resources.get::<wgpu::RenderPipeline>() {
            pass.set_pipeline(pipeline);
            pass.draw(0..3, 0..1);
        }
    }

    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<wgpu::RenderPipeline>().is_none() {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("triangle shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../shader/triangle.wgsl").into()),
            });

            let pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("pipeline layout"),
                    bind_group_layouts: &[],
                    push_constant_ranges: &[],
                });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("custom_pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Option::from("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Option::from("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Bgra8Unorm,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

            resources.insert(pipeline);
        }
        Vec::new()
    }
}

