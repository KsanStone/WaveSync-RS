use crate::sound::audio_service::AudioService;
use crate::ui::plot::PlotData;
use eframe::egui::PaintCallbackInfo;
use eframe::wgpu::{CommandBuffer, CommandEncoder, Device, Queue, RenderPass};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use std::sync::Arc;

#[derive(Clone)]
pub struct SpectrumVisualizer(Arc<Inner>);

pub struct Inner {
    audio_service: AudioService,
}

impl SpectrumVisualizer {
    pub fn new(audio_service: AudioService) -> Self {
        Self(Arc::new(Inner { audio_service }))
    }

    pub fn update_axis(&self, plot_data: &mut PlotData) {
        plot_data.x_axis = Some(crate::ui::plot::Axis {
            min: 0.0,
            max: 20000.0,
            logarithmic: true,
        });
        plot_data.y_axis = Some(crate::ui::plot::Axis {
            min: 0.0,
            max: 1.0,
            logarithmic: false,
        });
    }
}

pub struct SpectrumVisualizerCallback {
    visualizer: SpectrumVisualizer,
}

impl SpectrumVisualizerCallback {
    pub(crate) fn new(visualizer: SpectrumVisualizer) -> Self {
        Self { visualizer }
    }
}

impl CallbackTrait for SpectrumVisualizerCallback {
    fn prepare(
        &self,
        _device: &Device,
        _queue: &Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut CommandEncoder,
        _callback_resources: &mut CallbackResources,
    ) -> Vec<CommandBuffer> {
        vec![]
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
    }
}
