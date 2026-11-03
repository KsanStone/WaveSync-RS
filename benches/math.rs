//! Dependency-free kernel benchmarks. See math-results.md for scope and commands.
use std::{hint::black_box as bb, time::Instant};
const RATE: f32 = 192_000.0;
const BINS: usize = 65;

fn signal(n: usize, phase: f32) -> Vec<f32> {
    let mut seed = 1234567u32;
    (0..n)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let t = i as f32 / RATE;
            0.45 * (t * 997.0 * std::f32::consts::TAU + phase).sin()
                + 0.2 * (t * 61.0 * std::f32::consts::TAU).sin()
                + 0.03 * (seed as f32 / u32::MAX as f32 - 0.5)
        })
        .collect()
}

#[inline(never)]
fn rms<const FAST: bool>(x: &[f32]) -> f32 {
    let sum = if FAST {
        x.iter()
            .fold(0.0f32, |s, &v| s.algebraic_add(v.algebraic_mul(v)))
    } else {
        x.iter().map(|v| v * v).sum()
    };
    (sum / x.len() as f32).sqrt()
}

#[inline(never)]
fn summary<const FAST: bool>(x: &[f32], chunk: usize, out: &mut Vec<[f32; 3]>) {
    out.clear();
    for samples in x.chunks(chunk) {
        let (mut min, mut max, mut sum) = (f32::INFINITY, f32::NEG_INFINITY, 0.0f32);
        for &v in samples {
            min = min.min(v);
            max = max.max(v);
            sum = if FAST {
                sum.algebraic_add(v.algebraic_mul(v))
            } else {
                sum + v * v
            };
        }
        out.push([min, max, (sum / samples.len() as f32).sqrt()]);
    }
}

// Same input state on each call: avoids a benchmark settling into a single branch.
// Reset copies are included identically in both variants.
#[inline(never)]
fn smooth<const FAST: bool, const FALLOFF: bool>(
    input: &[f32],
    initial: &[f32],
    state: &mut [f32],
    speed: &mut [f32],
    dt: f32,
    factor: f32,
) {
    state.copy_from_slice(initial);
    speed.fill(0.1);
    if FALLOFF {
        let f = (factor * 30.0).clamp(0.0, 30.0);
        let dt = dt.clamp(0.0, 1.0);
        if FAST {
            let increment = f * dt;
            for ((item, speed), &recent) in state.iter_mut().zip(speed).zip(input) {
                if recent > *item {
                    *item = recent;
                    *speed = 0.0;
                } else {
                    *item = (*item - (*speed * dt * *item)).clamp(0.0, 1.0);
                    *speed += increment;
                }
            }
        } else {
            for (i, item) in state.iter_mut().enumerate() {
                if input[i] > *item {
                    *item = input[i];
                    speed[i] = 0.0;
                } else {
                    *item = (*item - (speed[i] * dt * *item)).clamp(0.0, 1.0);
                    speed[i] += f * dt;
                }
            }
        }
    } else {
        let f = 1.0 - factor.clamp(0.0, 1.0);
        let dt = (dt * 100.0).clamp(0.0, 1.0);
        if FAST {
            let alpha = f * dt;
            for (item, &recent) in state.iter_mut().zip(input) {
                *item = recent
                    .algebraic_sub(*item)
                    .algebraic_mul(alpha)
                    .algebraic_add(*item)
                    .clamp(0.0, 1.0);
            }
        } else {
            for (i, item) in state.iter_mut().enumerate() {
                *item = (*item + (input[i] - *item) * f * dt).clamp(0.0, 1.0);
            }
        }
    }
}

#[inline(never)]
fn stereo<const FAST: bool>(l: &mut [f32], r: &mut [f32], bins: &mut [f32; BINS]) {
    bins.fill(0.0);
    if !FAST {
        for i in 0..l.len() {
            let (a, b) = (l[i], r[i]);
            l[i] = (a + b) / std::f32::consts::SQRT_2;
            r[i] = (a - b) / std::f32::consts::SQRT_2;
        }
    }
    for i in 0..l.len() {
        let (mut m, mut s) = if FAST {
            (
                (l[i] + r[i]) / std::f32::consts::SQRT_2,
                (l[i] - r[i]) / std::f32::consts::SQRT_2,
            )
        } else {
            (l[i], r[i])
        };
        if m < 0.0 {
            m = -m;
            s = -s;
        }
        let angle = s.atan2(m);
        let magnitude = (m * m + s * s).sqrt();
        let index = if FAST {
            ((angle + std::f32::consts::FRAC_PI_2) * (BINS as f32 / std::f32::consts::PI)).floor()
                as usize
        } else {
            ((angle + std::f32::consts::FRAC_PI_2) / std::f32::consts::PI * BINS as f32).floor()
                as usize
        }
        .min(BINS - 1);
        bins[index] += magnitude;
    }
    let inv = 1.0 / (l.len() as f32 * 0.3);
    for v in bins {
        *v = (*v * inv).clamp(0.0, 1.0);
    }
}

#[inline(never)]
fn energies<const FAST: bool>(out: &mut [f32], lambda: f32, exposure: f32) {
    let n = out.len();
    if FAST {
        // f64 recurrence avoids f32 multiplier error accumulating over thousands of samples.
        let step = (lambda as f64 / RATE as f64).exp();
        let mut energy = exposure as f64 * (-(lambda as f64) * (n - 1) as f64 / RATE as f64).exp();
        for v in out {
            *v = energy as f32;
            energy *= step;
        }
    } else {
        for (i, v) in out.iter_mut().enumerate() {
            let age = (n - 1 - i) as f32 / RATE;
            *v = exposure * (-lambda * age).exp();
        }
    }
}

#[inline(never)]
fn ellipse<const FAST: bool>(unit: &[[f32; 2]], out: &mut Vec<[f32; 2]>, count: usize) {
    out.clear();
    for ring in 1..=count {
        let size = [
            400.0 * ring as f32 / count as f32,
            200.0 * ring as f32 / count as f32,
        ];
        for i in 0..128 {
            let [x, y] = if FAST {
                unit[i]
            } else {
                let theta = 2.0 * std::f32::consts::PI * i as f32 / 128.0;
                [theta.cos(), theta.sin()]
            };
            out.push([500.0 + size[0] * x, 300.0 + size[1] * y]);
        }
    }
}

fn error(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max)
}

// Includes the actual linear [-1, 1] mapping, finite check, hypot-based
// Segment::with_energy weight and 48-byte segment output; excludes locks/GPU.
#[inline(never)]
fn segments<const FAST: bool>(l: &[f32], r: &[f32], out: &mut Vec<[f32; 12]>, lambda: f32) {
    out.clear();
    let n = l.len() - 1;
    let step = (lambda as f64 / RATE as f64).exp();
    let mut energy = (-(lambda as f64) * (n - 1) as f64 / RATE as f64).exp();
    for i in 0..n {
        let start = [
            (l[i] + 1.0) / 2.0 * 2.0 - 1.0,
            (-r[i] + 1.0) / 2.0 * 2.0 - 1.0,
        ];
        let end = [
            (l[i + 1] + 1.0) / 2.0 * 2.0 - 1.0,
            (-r[i + 1] + 1.0) / 2.0 * 2.0 - 1.0,
        ];
        let e = if FAST {
            let e = energy as f32;
            energy *= step;
            e
        } else {
            (-lambda * ((n - 1 - i) as f32 / RATE)).exp()
        };
        if start.iter().chain(end.iter()).all(|v| v.is_finite()) {
            let length =
                ((end[0] - start[0]) * 800.0 * 0.5).hypot((end[1] - start[1]) * 600.0 * 0.5);
            out.push([
                start[0],
                start[1],
                end[0],
                end[1],
                0.0,
                0.0,
                0.0,
                0.0,
                e / (length + std::f32::consts::PI / 3.0),
                f32::from_bits(1),
                0.0,
                0.0,
            ]);
        }
    }
}
fn timed(f: &mut impl FnMut(), n: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..n {
        f();
    }
    start.elapsed().as_secs_f64() * 1e6 / n as f64
}
fn median(x: &mut [f64]) -> f64 {
    x.sort_by(f64::total_cmp);
    x[x.len() / 2]
}
fn compare(name: &str, calls_sec: f64, mut base: impl FnMut(), mut fast: impl FnMut()) {
    if std::env::args().any(|arg| arg == "--check-only") {
        return;
    }
    let mut n = 1;
    while timed(&mut base, n) * (n as f64) < 10_000.0 {
        n *= 2;
    }
    let (mut a, mut b, mut ratios) = (Vec::new(), Vec::new(), Vec::new());
    let mut wins = 0;
    for round in 0..15 {
        let (x, y) = if round % 2 == 0 {
            (timed(&mut base, n), timed(&mut fast, n))
        } else {
            let y = timed(&mut fast, n);
            (timed(&mut base, n), y)
        };
        wins += usize::from(y < x);
        a.push(x);
        b.push(y);
        ratios.push(x / y);
    }
    let x = median(&mut a);
    let y = median(&mut b);
    let ratio = median(&mut ratios);
    println!(
        "{name},{x:.4},{y:.4},{ratio:.3},{wins}/15,{:.3},{:.3},{:.3}",
        (x - y) * calls_sec / 1000.0,
        ratios[1],
        ratios[13]
    );
}

fn main() {
    println!(
        "case,baseline_us,candidate_us,paired_speedup,wins,saved_ms_per_audio_second,speedup_p10,speedup_p90"
    );
    for n in [256, 1024, 3200] {
        let (l, r) = (signal(n, 0.0), signal(n, 0.73));
        assert!((rms::<false>(&l) - rms::<true>(&l)).abs() < 1e-5);
        compare(
            &format!("rms_stereo_{n}"),
            RATE as f64 / n as f64,
            || {
                bb((rms::<false>(bb(&l)), rms::<false>(bb(&r))));
            },
            || {
                bb((rms::<true>(bb(&l)), rms::<true>(bb(&r))));
            },
        );
    }
    for chunk in [32, 128, 512] {
        let data = signal(3200, 0.3);
        let (mut a, mut b) = (Vec::with_capacity(3200), Vec::with_capacity(3200));
        summary::<false>(&data, chunk, &mut a);
        summary::<true>(&data, chunk, &mut b);
        assert!(error(a.as_flattened(), b.as_flattened()) < 1e-5);
        compare(
            &format!("summary_mono_3200_chunk{chunk}"),
            60.0,
            || {
                summary::<false>(bb(&data), bb(chunk), &mut a);
                bb(&a);
            },
            || {
                summary::<true>(bb(&data), bb(chunk), &mut b);
                bb(&b);
            },
        );
    }
    for n in [65, 16384] {
        let input: Vec<_> = signal(n, 0.0).iter().map(|v| v.abs()).collect();
        let initial: Vec<_> = signal(n, 1.1).iter().map(|v| v.abs()).collect();
        for falloff in [false, true] {
            let (mut a, mut b, mut sa, mut sb) =
                (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
            macro_rules! run {
                ($fast:literal,$out:expr,$speed:expr) => {
                    if falloff {
                        smooth::<$fast, true>(
                            bb(&input),
                            bb(&initial),
                            $out,
                            $speed,
                            bb(1.0 / 60.0),
                            bb(0.7),
                        );
                    } else {
                        smooth::<$fast, false>(
                            bb(&input),
                            bb(&initial),
                            $out,
                            $speed,
                            bb(1.0 / 60.0),
                            bb(0.7),
                        );
                    }
                };
            }
            run!(false, &mut a, &mut sa);
            run!(true, &mut b, &mut sb);
            assert!(error(&a, &b) < 1e-6);
            assert!(error(&sa, &sb) < 1e-6);
            compare(
                &format!(
                    "smooth_{}_{n}",
                    if falloff { "falloff" } else { "multiplicative" }
                ),
                60.0,
                || {
                    run!(false, &mut a, &mut sa);
                    bb((&a, &sa));
                },
                || {
                    run!(true, &mut b, &mut sb);
                    bb((&b, &sb));
                },
            );
        }
    }
    for fps in [30, 60, 144] {
        let n = 192000 / fps;
        let (l, r) = (signal(n, 0.0), signal(n, 0.73));
        let (mut la, mut ra, mut lb, mut rb) = (l.clone(), r.clone(), l.clone(), r.clone());
        let (mut a, mut b) = ([0.0; BINS], [0.0; BINS]);
        stereo::<false>(&mut la, &mut ra, &mut a);
        stereo::<true>(&mut lb, &mut rb, &mut b);
        eprintln!(
            "stereo {fps}fps max normalized bin difference: {:.8}",
            error(&a, &b)
        );
        assert!(error(&a, &b) < 0.002);
        compare(
            &format!("stereo_{fps}fps_{n}"),
            fps as f64,
            || {
                la.copy_from_slice(bb(&l));
                ra.copy_from_slice(bb(&r));
                stereo::<false>(&mut la, &mut ra, &mut a);
                bb(&a);
            },
            || {
                lb.copy_from_slice(bb(&l));
                rb.copy_from_slice(bb(&r));
                stereo::<true>(&mut lb, &mut rb, &mut b);
                bb(&b);
            },
        );
        let count = n.min(4096);
        let (mut a, mut b) = (vec![0.0; count], vec![0.0; count]);
        let lambda = -0.8f32.ln() * 60.0;
        energies::<false>(&mut a, lambda, 1.0);
        energies::<true>(&mut b, lambda, 1.0);
        eprintln!("energy {fps}fps max abs difference: {:.9}", error(&a, &b));
        assert!(error(&a, &b) < 2e-6);
        compare(
            &format!("energy_{fps}fps_{count}"),
            fps as f64,
            || {
                energies::<false>(bb(&mut a), bb(lambda), bb(1.0));
                bb(&a);
            },
            || {
                energies::<true>(bb(&mut b), bb(lambda), bb(1.0));
                bb(&b);
            },
        );
        let (l, r) = (signal(count + 1, 0.0), signal(count + 1, 0.73));
        let (mut a, mut b) = (Vec::with_capacity(count), Vec::with_capacity(count));
        segments::<false>(&l, &r, &mut a, lambda);
        segments::<true>(&l, &r, &mut b, lambda);
        assert!(error(a.as_flattened(), b.as_flattened()) < 2e-6);
        compare(
            &format!("segments_{fps}fps_{count}"),
            fps as f64,
            || {
                segments::<false>(bb(&l), bb(&r), &mut a, bb(lambda));
                bb(&a);
            },
            || {
                segments::<true>(bb(&l), bb(&r), &mut b, bb(lambda));
                bb(&b);
            },
        );
    }
    let unit: Vec<_> = (0..128)
        .map(|i| {
            let t = 2.0 * std::f32::consts::PI * i as f32 / 128.0;
            [t.cos(), t.sin()]
        })
        .collect();
    for count in [1, 11] {
        let (mut a, mut b) = (Vec::with_capacity(1408), Vec::with_capacity(1408));
        ellipse::<false>(&unit, &mut a, count);
        ellipse::<true>(&unit, &mut b, count);
        assert_eq!(a, b);
        compare(
            &format!("ellipse_{count}_rings"),
            60.0,
            || {
                ellipse::<false>(bb(&unit), &mut a, bb(count));
                bb(&a);
            },
            || {
                ellipse::<true>(bb(&unit), &mut b, bb(count));
                bb(&b);
            },
        );
    }
    // Accuracy beyond the timing dataset: silence, tiny signals, phase and level changes.
    for n in [1, 32, 256, 3200, 4096, 32768] {
        for scale in [0.0f32, 1e-12, 0.01, 1.0] {
            let x: Vec<_> = signal(n, 1.7).iter().map(|v| v * scale).collect();
            let reference = (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
            let candidate = rms::<true>(&x) as f64;
            assert!((candidate - reference).abs() <= 1e-6 * reference.max(1e-30));
        }
    }
    eprintln!("All numerical checks passed.");
}
