use crate::ui::plot::PlotData;

pub trait Visualizer {
    fn get_plot_data(&self) -> PlotData;

    fn set_plot_data(&self, plot_data: PlotData);

    fn accept_fft(&self, _fft_data: Vec<Vec<f32>>, _fft_size: usize) {}
}
