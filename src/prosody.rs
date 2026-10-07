//! Pitch-mark generation and unit concatenation -- Flite's `f0_targets_to_pm`
//! and `concat_units`, as Cepstral actually implements them.
//!
//! Traced from swift.dll:
//!   0x2a3f3  f0_targets_to_pm inner loop  (float32, not flite's double)
//!   0x2ad00  concat_units unit loop
//!   0x2a870  nearest_pm                   (integer abs, not flite's fabs)
//!   0x2afff  u_index update               (integer imul/idiv, not flite's float m)
//!
//! The integer arithmetic is the reason to transcribe from the binary rather
//! than from `cst_units.c`: truncation changes which frame `nearest_pm` picks.

use crate::db::Voice;
use crate::synth::{LpcState, GAIN_UNITY};

/// One point on the F0 contour. `pos` is seconds from the start of the utterance.
#[derive(Debug, Clone, Copy)]
pub struct F0Target {
    pub pos: f32,
    pub f0: f32,
}

/// A selected unit together with the time its segment is meant to end.
#[derive(Debug, Clone, Copy)]
pub struct TargetUnit {
    /// First frame, i.e. `unit_start` after any optimal-coupling adjustment.
    pub start: i32,
    /// One past the last frame, i.e. `unit_end`.
    pub end: i32,
    /// Segment end in samples: `(item_feat_float(s,"end") * sample_rate) as i32`.
    pub target_end: i32,
    /// `local_rescale` as 1.15 fixed point; `GAIN_UNITY` for none.
    pub gain: i32,
}

/// One emitted pitch period.
#[derive(Debug, Clone, Copy)]
pub struct Period {
    pub src_frame: usize,
    pub size: usize,
    pub gain: i32,
}

/// Flite's default lead-in F0 (`lf0 = 120; /* hmm */`).
pub const DEFAULT_LEAD_F0: f32 = 120.0;

/// swift.dll @0x2a300. Returns pitch-mark times in samples.
pub fn f0_targets_to_pm(v: &Voice, targets: &[F0Target], lead_f0: f32) -> Vec<i32> {
    let mut out = Vec::new();
    let mut lpos = 0.0f32;
    let mut lf0 = lead_f0;
    let mut time = 0.0f32;
    let rate = v.sps as f32;

    for t in targets {
        let pos = t.pos;
        // 0x2a326: a zero target would divide by zero downstream
        let f0 = if t.f0 == 0.0 { 1.0 } else { t.f0 };
        let d = pos - lpos;
        // 0x2a345: guard the degenerate span rather than producing an infinity
        let m = if d == 0.0 { f0 - lf0 } else { (f0 - lf0) / d };

        while time < pos {
            let period_hz = lf0 + (time - lpos) * m;
            if !(period_hz > 0.0) {
                break; // would stall or run backwards; the original would hang
            }
            time += 1.0f32 / period_hz;
            out.push((rate * time) as i32);
            if out.len() > 1_000_000 {
                break;
            }
        }
        lf0 = f0;
        lpos = pos;
    }
    out
}

/// `get_unit_size`: total samples across a unit's frames.
pub fn unit_size(v: &Voice, start: i32, end: i32) -> i32 {
    let mut n = 0i32;
    for i in start..end {
        n += v.frame_size(i as usize) as i32;
    }
    n
}

/// swift.dll @0x2a870. Integer throughout.
pub fn nearest_pm(v: &Voice, start: i32, end: i32, u_index: i32) -> i32 {
    let mut i_size = 0i32;
    for i in start..end {
        let n_size = i_size + v.frame_size(i as usize) as i32;
        if (u_index - i_size).abs() < (u_index - n_size).abs() {
            return i;
        }
        i_size = n_size;
    }
    end - 1
}

/// swift.dll @0x2ad00. Maps pitch marks onto source frames.
pub fn concat_units(v: &Voice, units: &[TargetUnit], pms: &[i32]) -> Vec<Period> {
    let mut out = Vec::with_capacity(pms.len());
    let mut target_start = 0i32;
    let mut pm_i = 0usize;

    for u in units {
        let usize_samples = unit_size(v, u.start, u.end);
        let denom = u.target_end - target_start;
        let mut u_index = 0i32;

        while pm_i < pms.len() && pms[pm_i] <= u.target_end {
            let nearest = nearest_pm(v, u.start, u.end, u_index);
            let prev = if pm_i > 0 { pms[pm_i - 1] } else { 0 };
            let size = pms[pm_i] - prev;
            if size > 0 {
                out.push(Period {
                    src_frame: nearest as usize,
                    size: size as usize,
                    gain: u.gain,
                });
            }
            // 0x2afff: integer, truncating. Flite does this in float.
            if denom != 0 {
                u_index += size.saturating_mul(usize_samples) / denom;
            }
            pm_i += 1;
        }
        target_start = u.target_end;
    }
    out
}

pub fn synth_periods(v: &Voice, periods: &[Period]) -> Vec<i16> {
    let mut st = LpcState::new(v);
    for p in periods {
        st.emit_period(v, p.src_frame, p.size, p.gain);
    }
    st.out
}

/// Full `join_units_modified_lpc`: F0 contour + units with durations -> samples.
pub fn join_units_modified_lpc(
    v: &Voice,
    units: &[TargetUnit],
    targets: &[F0Target],
    lead_f0: f32,
) -> Vec<i16> {
    let pms = f0_targets_to_pm(v, targets, lead_f0);
    let periods = concat_units(v, units, &pms);
    synth_periods(v, &periods)
}

/// `join_units_simple` / `asis_to_pm`: no prosodic modification at all.
pub fn join_units_simple(v: &Voice, units: &[TargetUnit]) -> Vec<i16> {
    join_units_streamed(v, units, &[units.len()])
}

/// The same, told where the engine's streaming broke the utterance up.
///
/// `clunits_synth` commits a prefix as soon as the whole beam agrees on it and
/// synthesises that prefix on its own: separate utterance, separate
/// `join_units` call, separate LPC scratch buffer. `pieces` holds the unit count
/// of each of those, which is `Selector::last_pieces`. Only the scratch buffer
/// resets at a boundary -- the filter history runs straight through -- so this
/// changes a handful of samples per piece and nothing else.
pub fn join_units_streamed(v: &Voice, units: &[TargetUnit], pieces: &[usize]) -> Vec<i16> {
    let mut st = LpcState::new(v);
    let mut at = 0usize;
    for &n in pieces {
        st.new_piece();
        for u in &units[at..(at + n).min(units.len())] {
            for i in u.start..u.end {
                let sz = v.frame_size(i as usize);
                st.emit_period(v, i as usize, sz, u.gain);
            }
        }
        at += n;
    }
    for u in &units[at.min(units.len())..] {
        for i in u.start..u.end {
            let sz = v.frame_size(i as usize);
            st.emit_period(v, i as usize, sz, u.gain);
        }
    }
    st.out
}

/// `join_units_streamed` with a duration stretch and an f0 factor per unit:
/// swift.dll's `simple_join` path in full. Per piece, @0x29ba0/@0x29a90 lay out
/// pitch marks -- with `b = stretch * f0`, `frames * b` periods, period `i`
/// sized `frame_size(start + i / b) / f0` -- and set each unit's `target_end`
/// to where its marks end; @0x2ad00 then maps those marks back onto source
/// frames. At b = 1 this is exactly the plain path, which is why that one can
/// skip it.
pub fn join_units_timed(
    v: &Voice,
    units: &[TargetUnit],
    stretch: &[f32],
    f0: &[f32],
    pieces: &[usize],
) -> Vec<i16> {
    let mut st = LpcState::new(v);
    let mut at = 0usize;
    let mut bounds: Vec<usize> = pieces.to_vec();
    let rest = units.len().saturating_sub(pieces.iter().sum());
    if rest > 0 {
        bounds.push(rest);
    }
    for n in bounds {
        let end = (at + n).min(units.len());
        let mut pms = Vec::new();
        let mut timed: Vec<TargetUnit> = Vec::with_capacity(end - at);
        let mut pos = 0i32;
        for ((u, &s), &f) in units[at..end].iter().zip(&stretch[at..end]).zip(&f0[at..end]) {
            let f = if f > 0.0 { f } else { 1.0 };
            let b = s * f;
            let b = if b > 0.0 { b } else { 1.0 };
            let nper = ((u.end - u.start) as f32 * b) as i32;
            for i in 0..nper {
                let fr = u.start + (i as f32 / b) as i32;
                pos += (v.frame_size(fr as usize) as f32 / f) as i32;
                pms.push(pos);
            }
            timed.push(TargetUnit { target_end: pos, ..*u });
        }
        st.new_piece();
        for p in concat_units(v, &timed, &pms) {
            st.emit_period(v, p.src_frame, p.size, p.gain);
        }
        at = end;
    }
    st.out
}

/// The stretch @0x297c0 gives a unit with an explicit duration: target samples
/// over the unit's own.
pub fn stretch_to(v: &Voice, start: i32, end: i32, target: i32) -> f32 {
    let own = unit_size(v, start, end) as f32;
    if own == 0.0 { 1.0 } else { target as f32 / own }
}

pub fn gain_from_rescale(rescale: f32) -> i32 {
    // 0x2ad72: mulss by 32768 then cvttss2si. A negative rescale
    // (volume="-150%") renders exactly as silent does; where the engine
    // floors it is not found, so relative values nested after one are unchecked.
    (rescale.max(0.0) * 32768.0) as i32
}

const _: () = assert!(GAIN_UNITY == 0x8000);
