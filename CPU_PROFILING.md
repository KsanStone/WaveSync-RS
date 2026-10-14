# CPU profiling

Run an optimized build with the optional profiler:

```powershell
cargo run --release --features cpu-profiling
```

Install the separate viewer once, then connect it:

```powershell
cargo install puffin_viewer --locked
puffin_viewer 127.0.0.1:8585
```

Enable **Capture CPU profile** in the main Settings window. The server binds
only to localhost. Capture starts disabled; disable it again when finished.
If port 8585 is occupied, the app logs a warning and continues normally.

The timeline separates UI construction, egui tessellation, texture/buffer
uploads, each visualizer's prepare/paint callbacks, vectorscope stages,
command finishing, queue submission, surface acquisition, and presentation.
`wait.*` scopes show elapsed waits; they are not active CPU execution time.
Visualizer scopes include audio-buffer access and any mutex waits on the render
thread. The audio thread itself is not instrumented.

Use identical audio, graph dimensions, and FPS settings when comparing captures.
The profiler UI runs in a separate process, keeping its drawing out of WaveSync's
CPU timings. The existing in-app current/p95/p99 panel remains available.

Add scopes with `profiling::scope!("descriptive.name")` inside a lexical block,
or `profiling::function_scope!()` at function entry. Ordinary builds omit the
Puffin backend, viewer server, and recording overhead; these macros are no-ops:

```powershell
cargo run --release
```

Buffer writes have nested category scopes (`upload.uniform`, `upload.geometry`,
`upload.mapping`) and destination scopes such as
`extended_waveform.uniform_buffer_sample`. Texture writes use `upload.texture`.
Filter by `upload.uniform` to isolate uniform writes. Read the inclusive/total
scope duration in the flamegraph: self time excludes nested staging/copy work
and therefore does not represent the full upload cost. Category and destination
scopes overlap intentionally; do not add their durations together.
