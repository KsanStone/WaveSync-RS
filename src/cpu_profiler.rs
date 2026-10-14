//! Optional scoped CPU captures. No backend is compiled in ordinary builds.
#[cfg(feature = "cpu-profiling")]
pub fn start() -> Option<puffin_http::Server> {
    match puffin_http::Server::new("127.0.0.1:8585") {
        Ok(server) => {
            log::info!("CPU profiler ready at 127.0.0.1:8585; enable capture in Settings");
            Some(server)
        }
        Err(error) => {
            log::warn!("Could not start CPU profiler: {error}");
            None
        }
    }
}
#[cfg(not(feature = "cpu-profiling"))]
pub fn start() {}

pub fn settings_ui(ui: &mut egui::Ui) {
    #[cfg(feature = "cpu-profiling")]
    {
        let mut enabled = profiling::puffin::are_scopes_on();
        if ui.checkbox(&mut enabled, "Capture CPU profile").changed() {
            profiling::puffin::set_scopes_on(enabled);
        }
        ui.label("Puffin viewer: 127.0.0.1:8585");
    }
    #[cfg(not(feature = "cpu-profiling"))]
    ui.label("CPU profiler: build with --features cpu-profiling");
}

/// Profiles one upload by category and destination. Scopes are no-ops without
/// a profiling backend, and the upload expression is evaluated exactly once.
#[macro_export]
macro_rules! profile_upload {
    ($category:literal, $destination:literal, $upload:expr) => {{
        profiling::scope!($category);
        {
            profiling::scope!($destination);
            $upload
        }
    }};
}
