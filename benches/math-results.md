# Math kernel measurements

Measured 2026-10-27 on AMD Ryzen 9 9950X, Windows, rustc 1.99.1
(58cf8f9c3 2026-10-13), optimization level 3. Measurements preserve the original
baseline kernels. The RMS, waveform-summary, smoother and vectorscope-energy
candidates have since been applied to production. Ellipse caching and stereo
fusion remain benchmark-only experiments. Production numerical regression tests
pass in debug and release, including circular-buffer cache updates, repeated
smoothing frames/resizes, and energy recurrence across 22.05–384 kHz.

## Workload and procedure

- 192,000 samples/second **per channel**, two channels. Deterministic multitone
  audio plus noise, with a phase difference between channels; generated outside timing.
- RMS: stereo callback blocks of 256, 1,024 and 3,200 samples per channel.
- Render work: 30/60/144 FPS = 6,400/3,200/1,333 samples per channel per frame.
  Vectorscope obeys the application's 4,096-segment cap.
- Smoothers: 65 direction bins or 16,384 bins (half the default 32,768 FFT).
  These cases assume one smoothing update per rendered frame at 60 FPS.
- Waveform summaries: 3,200 mono samples divided into 32/128/512-sample columns.
  Multiply costs by two if two independent channels are summarized.
- Ellipses: one rim or eleven ellipses (rim plus ten concentric rings), 128 points each.
- Fifteen alternating baseline/candidate timing pairs; calibration doubles the
  iteration count until a baseline batch takes at least 10 ms. Calibration is
  also warm-up. Buffers retain capacity; black_box protects inputs and outputs.
- CSV times are medians in microseconds per invocation. Speedup is the median
  of paired ratios, with p10/p90 observations, **not confidence intervals**.
  Savings are median baseline minus median candidate, scaled by calls per second.
- Smoother states and stereo scratch are reset identically within both timed
  variants. This avoids repeated calls converging to unrepresentative states;
  it includes reset-copy costs in reported timings.

These are extracted standalone kernels based on the current application source,
not calls into a running app. They isolate mathematical work, excluding audio
capture, mutexes, UI text, draw calls, GPU uploads and GPU time. Summary inputs
are contiguous slices rather than circular-buffer iterators; actual iterator
layout may affect vectorization. They must not be interpreted as FPS gains.
The deterministic input fits in cache and repeats between iterations. Scheduling,
frequency changes and other applications can affect measurements.

## Main results (default compiler CPU target)

| Operation / call | Baseline µs | Candidate µs | Paired speedup | Saved ms per second |
|---|---:|---:|---:|---:|
| RMS, stereo 3,200 samples/channel | 2.3494 | 0.2844 | 8.40× | 0.124 |
| Waveform summary, 3,200 samples, columns of 128 | 3.5544 | 0.6544 | 5.49× | 0.174 |
| Multiplicative smoother, 16,384 bins | 2.6639 | 2.5408 | 1.05× | 0.007 |
| Falloff smoother, 16,384 bins | 12.4866 | 4.1597 | 2.93× | 0.500 |
| Stereo rotation + histogram, 60 FPS | 23.5289 | 28.0076 | 0.84× | -0.269 |
| Vectorscope energies alone, 60 FPS | 6.6166 | 1.7916 | 3.63× | 0.290 |
| Vectorscope segments including geometry, 60 FPS | 17.7169 | 12.7979 | 1.40× | 0.295 |
| One ellipse | 0.8226 | 0.0770 | 10.66× | 0.045 |
| Eleven ellipses | 9.0773 | 0.8455 | 10.71× | 0.494 |

All candidates above except stereo beat baseline in at least 14/15 pairs;
multiplicative smoothing is small enough that its practical value is doubtful.
Do not add energies and full-segment savings: they measure overlapping work.

An earlier default-target run independently showed the same direction for all
original cases (before the full-segment case was added). The checked-in CSVs
contain the final runs. A separate native-target run also agrees on the useful
changes: stereo RMS 3,200 is 2.3412 → 0.0723 µs, falloff smoothing 16,384 is
10.7950 → 1.8787 µs, full vectorscope segments at 60 FPS are 17.1125 → 12.4390 µs.
Native targeting is a separate build choice; those binaries require this CPU's
features. Both runs show no benefit from the proposed stereo fusion.

## Candidates and accuracy checks

- RMS and summaries: algebraic add/multiply in the sum of squares. Summary
  min/max stay ordinary operations. Summary maximum absolute output difference
  must be <1e-5. RMS is additionally checked against an f64 reference at lengths
  1 through 32,768, amplitudes 0/1e-12/0.01/1, with relative tolerance 1e-6.
- Multiplicative smoother: zipped slices, precomputed alpha, algebraic
  operations. Falloff: zipped three slices and hoisted frame-constant increment,
  **ordinary float arithmetic**. Thus its gain does not require relaxed float math.
  State and speed differences must be <1e-6 for mixed rising/falling inputs.
- Stereo: fused rotation/histogram loop and combined angle-to-bin factor.
  Normalized histogram outputs were identical for these inputs; the allowed
  absolute difference is <0.002. This is not a proof about points at bin boundaries.
- Vectorscope: geometric progression using **f64** initial value and multiplier,
  casting energy to f32 per segment. A naive f32 recurrence is deliberately not
  proposed, because its multiplier rounding accumulates. Maximum observed energy
  difference was 5.96e-8; tests require <2e-6. Full-segment weights also pass 2e-6.
  Segment benchmark includes axis mapping, finite checks, hypot normalization,
  and a 48-byte record matching the production segment layout.
- Ellipse: cached unit-circle coordinates, scaled/translated for each ring.
  All generated coordinates match exactly. Cache construction is one-time,
  excluded; point generation/output remains timed. Egui shape submission is excluded.

Algebraic float APIs relax rounding and special-value guarantees. These checks
cover normal finite audio and silence, not a guarantee of NaN/Inf equivalence.
There are no speed thresholds in the correctness checks.

## Reproduce

From the repository root in PowerShell, with no extra dependencies:

```powershell
rustc --edition 2024 -C opt-level=3 benches/math.rs -o target/math-bench.exe
./target/math-bench.exe
./target/math-bench.exe --check-only

rustc --edition 2024 -C opt-level=3 -C target-cpu=native benches/math.rs -o target/math-bench-native.exe
./target/math-bench-native.exe
```

Or use the registered Cargo benchmark: `cargo bench --bench math`.
Raw results: `math-default.csv`, `math-native.csv`. Numerical check output:
`math-default-checks.txt`, `math-native-checks.txt`.

Recommendation: RMS/summary algebraic reductions and the ordinary-arithmetic
falloff rewrite are simple candidates; the f64 energy recurrence is measurable
even with geometry included. Keep stereo unchanged. Ellipse caching is measurable
but its absolute cost is small; leaving that code simple is a reasonable choice.
