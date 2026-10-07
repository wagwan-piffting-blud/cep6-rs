//! Recovering the engine's own unit choices from its output.
//!
//! The synthesis filter is `y[n] = e[n] + sum_k a[k]*y[n-1-k]`, so inverse
//! filtering the SDK's audio with a candidate frame's coefficients recovers what
//! the excitation must have been if that frame were the one used. The residual is
//! stored at half rate and expanded by zero-order hold, which means the true
//! excitation satisfies `e[2m] == e[2m+1]` within a pitch period. Scoring that
//! identity separates the right frame from every other one: the correct frame
//! scores 0.3 to 1.2 against 64 to 174 for the wrong phase, out of 791,464
//! candidates.
//!
//! This is ground truth. It gives the exact frame and unit sequence behind any
//! Swift WAV without knowing anything about unit selection, so it is what the
//! selection stage gets validated against.
//!
//! `concat_units` walks frames consecutively inside a unit, so a four-candidate
//! local probe almost always suffices and the exhaustive search only runs at unit
//! boundaries. That search is the whole cost, and it is pure data parallelism over
//! independent frames -- which is why this lives in Rust rather than in the Python
//! original, where it was minutes per second of audio.

use crate::db::Voice;
use std::sync::atomic::{AtomicU64, Ordering};

/// Samples scored at each probe position.
pub const WIN: usize = 24;
/// Probe every `STRIDE` samples. Also the tolerance when comparing a recovered
/// period against a frame's natural length.
pub const STRIDE: usize = 4;
/// A ZOH score at or below this means "inside this frame's period".
pub const GOOD: f32 = 3.0;
/// A unit visited for at most this many probes, between two visits to one other
/// unit, is a mis-trace rather than a selection.
pub const NOISE_RUN: usize = 1;

pub struct Hit {
    /// Sample offset in the reference audio where this frame starts.
    pub pos: usize,
    pub frame: usize,
    pub score: f32,
    /// Whether the exhaustive search was needed here.
    pub global: bool,
}

pub struct Trace {
    pub hits: Vec<Hit>,
    pub globals: usize,
    pub probes: usize,
}

/// All LPC coefficients, decoded once, row-major by frame.
pub struct Coeffs {
    pub data: Vec<f32>,
    pub order: usize,
    pub frames: usize,
}

impl Coeffs {
    pub fn build(v: &Voice) -> Coeffs {
        let order = v.order;
        let frames = v.num_sts;
        let mut data = vec![0.0f32; frames * order];
        let mut raw = vec![0u16; order];
        let scale = (1.0f32 / 65536.0) * v.coeff_range;
        for i in 0..frames {
            v.frame(i, &mut raw);
            let row = &mut data[i * order..(i + 1) * order];
            for k in 0..order {
                row[k] = (raw[k] as f32) * scale + v.coeff_min;
            }
        }
        Coeffs { data, order, frames }
    }

    #[inline]
    fn row(&self, j: usize) -> &[f32] {
        &self.data[j * self.order..(j + 1) * self.order]
    }
}

/// History matrix, transposed so the inner loop walks contiguous memory in both
/// operands: `ht[n][k] = y[p-1-k+n]`.
fn history(y: &[f32], p: usize, order: usize, ht: &mut [f32]) {
    for n in 0..WIN {
        let base = p + n;
        let dst = &mut ht[n * order..(n + 1) * order];
        for (k, d) in dst.iter_mut().enumerate() {
            *d = y[base - 1 - k];
        }
    }
}

/// Zero-order-hold score for one candidate: the excitation this frame implies
/// should repeat in pairs, so the mean absolute difference across pairs is near
/// zero for the right frame and large for any other.
#[inline]
fn zoh_score(y: &[f32], p: usize, coeffs: &[f32], ht: &[f32], order: usize) -> f32 {
    zoh_score_bounded(y, p, coeffs, ht, order, f32::MAX)
}

/// The same score, abandoned as soon as it cannot beat `bound`.
///
/// This is exact, not an approximation: the terms are absolute values, so a
/// partial sum only ever grows, and once it passes `bound * pairs` the candidate
/// is out whatever the remaining pairs hold. It matters because the score
/// separates so sharply -- the right frame lands near 1 while the rest sit in the
/// dozens to low hundreds -- so once any thread has seen a good frame almost every
/// later candidate is rejected after one or two pairs instead of twenty-four.
#[inline]
fn zoh_score_bounded(
    y: &[f32],
    p: usize,
    coeffs: &[f32],
    ht: &[f32],
    order: usize,
    bound: f32,
) -> f32 {
    let mut best = f32::MAX;
    for phase in 0..2 {
        let pairs = (WIN - phase) / 2;
        if pairs == 0 {
            continue;
        }
        let limit = if bound == f32::MAX {
            f32::MAX
        } else {
            bound * pairs as f32
        };
        let mut sum = 0.0f32;
        let mut prev = excite(y, p, coeffs, ht, order, phase);
        let mut ok = true;
        for m in 0..pairs {
            let a = if m == 0 { prev } else { excite(y, p, coeffs, ht, order, phase + 2 * m) };
            let b = excite(y, p, coeffs, ht, order, phase + 2 * m + 1);
            sum += (a - b).abs();
            if sum >= limit {
                ok = false;
                break;
            }
            prev = b;
        }
        if ok {
            let s = sum / pairs as f32;
            if s < best {
                best = s;
            }
        }
    }
    best
}

/// The excitation this frame implies at offset `n`: one inverse-filter step.
#[inline(always)]
fn excite(y: &[f32], p: usize, coeffs: &[f32], ht: &[f32], order: usize, n: usize) -> f32 {
    let h = &ht[n * order..(n + 1) * order];
    let mut acc = y[p + n];
    for k in 0..order {
        acc -= coeffs[k] * h[k];
    }
    acc
}

pub struct Tracer<'a> {
    v: &'a Voice,
    c: Coeffs,
    y: Vec<f32>,
    /// Unit starts sorted, with the unit index alongside, for frame -> unit.
    starts: Vec<(i32, usize)>,
    pub threads: usize,
}

impl<'a> Tracer<'a> {
    pub fn new(v: &'a Voice, pcm: &[i16]) -> Tracer<'a> {
        let y: Vec<f32> = pcm.iter().map(|&s| s as f32).collect();
        let mut starts: Vec<(i32, usize)> = (0..v.num_units)
            .map(|u| (v.unit(u).start, u))
            .collect();
        starts.sort_unstable();
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        Tracer { v, c: Coeffs::build(v), y, starts, threads }
    }

    pub fn frames(&self) -> usize {
        self.c.frames
    }

    /// Best of a handful of named candidates. Cheap; this is the common path.
    fn local(&self, p: usize, cands: &[usize], ht: &[f32]) -> (f32, usize) {
        let mut best = (f32::MAX, usize::MAX);
        for &j in cands {
            if j >= self.c.frames {
                continue;
            }
            let s = zoh_score(&self.y, p, self.c.row(j), ht, self.c.order);
            if s < best.0 {
                best = (s, j);
            }
        }
        best
    }

    /// Every frame in the database, split across all cores.
    fn global(&self, p: usize, ht: &[f32]) -> (f32, usize) {
        let order = self.c.order;
        let n = self.c.frames;
        let nthreads = self.threads.max(1);
        let chunk = n.div_ceil(nthreads);

        // (score bits, frame) packed so a single atomic min keeps them together;
        // f32 scores here are non-negative, so their bit patterns order correctly
        // (score, index) packed into one word. It must be u64 and not usize: on a
        // 32-bit target -- like armv7 -- a usize holds neither half.
        let best = AtomicU64::new(u64::MAX);
        std::thread::scope(|scope| {
            for t in 0..nthreads {
                let lo = t * chunk;
                let hi = (lo + chunk).min(n);
                if lo >= hi {
                    continue;
                }
                let best = &best;
                let y = &self.y;
                let c = &self.c;
                scope.spawn(move || {
                    let mut local = (f32::MAX, usize::MAX);
                    for j in lo..hi {
                        // refresh the bound from the other threads now and then;
                        // a relaxed load is nearly free and the bound only ever
                        // tightens, so a stale one costs work but never accuracy
                        if j & 0x3FF == 0 {
                            let packed = best.load(Ordering::Relaxed);
                            if packed != u64::MAX {
                                let shared = f32::from_bits((packed >> 32) as u32);
                                if shared < local.0 {
                                    local.0 = shared;
                                    local.1 = (packed & 0xFFFF_FFFF) as usize;
                                }
                            }
                        }
                        let s = zoh_score_bounded(y, p, c.row(j), ht, order, local.0);
                        if s < local.0 {
                            local = (s, j);
                        }
                    }
                    if local.1 != usize::MAX {
                        let packed =
                            ((local.0.to_bits() as u64) << 32) | (local.1 as u64 & 0xFFFF_FFFF);
                        best.fetch_min(packed, Ordering::Relaxed);
                    }
                });
            }
        });

        let packed = best.load(Ordering::Relaxed);
        if packed == u64::MAX {
            return (f32::MAX, usize::MAX);
        }
        (
            f32::from_bits((packed >> 32) as u32),
            (packed & 0xFFFF_FFFF) as usize,
        )
    }

    /// Walk the audio, recording every frame change.
    pub fn run(&self, max_samples: usize) -> Trace {
        let order = self.c.order;
        let mut ht = vec![0.0f32; WIN * order];
        let limit = self.y.len().min(max_samples);

        let first = self.y.iter().position(|&s| s != 0.0).unwrap_or(0);
        let mut p = first + order + 1;
        let mut cur: Option<usize> = None;
        let mut hits = Vec::new();
        let mut globals = 0usize;
        let mut probes = 0usize;

        while p + WIN < limit {
            history(&self.y, p, order, &mut ht);
            probes += 1;

            let (mut s, mut j) = match cur {
                Some(c) => self.local(p, &[c, c + 1, c + 2, c + 3], &ht),
                None => (f32::MAX, usize::MAX),
            };
            let mut used_global = false;
            if s > GOOD {
                let (gs, gj) = self.global(p, &ht);
                globals += 1;
                used_global = true;
                s = gs;
                j = gj;
            }
            if j != usize::MAX && Some(j) != cur && s <= GOOD {
                hits.push(Hit { pos: p, frame: j, score: s, global: used_global });
                cur = Some(j);
            }
            p += STRIDE;
        }
        Trace { hits, globals, probes }
    }

    /// Which unit owns a frame, if any.
    pub fn unit_of(&self, frame: usize) -> Option<usize> {
        let f = frame as i32;
        let i = self.starts.partition_point(|&(s, _)| s <= f);
        if i == 0 {
            return None;
        }
        let u = self.starts[i - 1].1;
        let x = self.v.unit(u);
        if x.start <= f && f < x.end {
            Some(u)
        } else {
            None
        }
    }

    /// Collapse a frame trace into the unit sequence behind it, discarding
    /// momentary excursions.
    ///
    /// The engine walks frames consecutively inside a unit, so a one-hit visit
    /// sandwiched between two visits to the *same* other unit cannot be a real
    /// selection -- it is the exhaustive search briefly preferring a lookalike
    /// frame. Tracing "hello world" in William shows the pattern plainly:
    /// `heh gih heh` and `low l low`, where `gih` and `l` are single hits between
    /// two runs of one unit. Left in, each costs an alignment slot and drags two
    /// neighbour features with it.
    pub fn units(&self, t: &Trace) -> Vec<(usize, usize)> {
        let mut runs: Vec<(usize, usize, usize)> = Vec::new();
        for h in &t.hits {
            if let Some(u) = self.unit_of(h.frame) {
                match runs.last_mut() {
                    Some(r) if r.1 == u => r.2 += 1,
                    _ => runs.push((h.pos, u, 1)),
                }
            }
        }

        let mut out: Vec<(usize, usize)> = Vec::new();
        for (i, r) in runs.iter().enumerate() {
            let spurious = r.2 <= NOISE_RUN
                && i > 0
                && i + 1 < runs.len()
                && runs[i - 1].1 == runs[i + 1].1;
            if spurious {
                continue;
            }
            // dropping the interruption makes the two halves adjacent again
            if out.last().map(|x| x.1) != Some(r.1) {
                out.push((r.0, r.1));
            }
        }
        out
    }
}

impl Tracer<'_> {
    /// The phone sequence behind a recovered unit list, read off the type names.
    ///
    /// This deliberately does NOT read the units' own recorded features. Those
    /// are the answer; using them to build the target features would make any
    /// parity check circular. The type name is what a correct front end would
    /// itself have produced, so starting from it isolates selection.
    pub fn phones(&self, table: &crate::target::PhoneTable, units: &[(usize, usize)])
        -> Vec<crate::rules::Phone>
    {
        units
            .iter()
            .map(|&(_, u)| {
                let name = &self.v.types[self.v.unit(u).type_id as usize].name;
                let base = table
                    .names
                    .values()
                    .filter(|p| name.starts_with(p.as_str()))
                    .max_by_key(|p| p.len())
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                // the stress digit follows the base phone in a vowel type name
                let stress = name[base.len()..]
                    .chars()
                    .next()
                    .and_then(|c| c.to_digit(10))
                    .unwrap_or(0) as u8;
                let phone = if base.starts_with("pau") { "pau".to_string() } else { base };
                crate::rules::Phone { phone, stress, word: None, word_id: 0, syl_end: None, dur: None }
            })
            .collect()
    }
}
