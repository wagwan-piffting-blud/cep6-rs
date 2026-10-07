//! Text to samples, the way the engine runs it.
//!
//! The one structural thing this adds over calling the pieces by hand: the
//! engine works **one utterance at a time**. Each sentence gets its own
//! `cst_utterance`, its own clunits search and its own `lpc_state`, so the
//! search beam does not see across a sentence boundary and the filter history
//! starts from zero at each one. Running a single search over a whole paragraph
//! picks the same units on short input and drifts on long input, and it puts the
//! streamed piece boundaries in the wrong places either way.

use crate::ceplex::CepLex;
use crate::db::Voice;
use crate::expr::TargetCost;
use crate::norm::DigitStyle;
use crate::markup::Ctl;
use crate::prosody::{gain_from_rescale, join_units_streamed, join_units_timed, stretch_to,
                     TargetUnit};
use crate::rules::{NameRules, Phone};
use crate::select::{SelectParams, Selector};
use crate::synth::GAIN_UNITY;
use crate::target::PhoneTable;

/// Split a phone sequence at its sentence boundaries.
///
/// A boundary is two pauses in a row -- `pauend` closing one utterance and
/// `paustart` opening the next -- so the cut goes between them and each piece
/// keeps a pause at both ends, which is what the engine's Segment relations
/// look like.
pub fn utterances(phones: &[Phone]) -> Vec<Vec<Phone>> {
    let mut out: Vec<Vec<Phone>> = Vec::new();
    let mut cur: Vec<Phone> = Vec::new();
    for (i, p) in phones.iter().enumerate() {
        cur.push(p.clone());
        // A break's timed pause takes the place of the pause it lands on
        // rather than adding one, so a pair still only occurs at a boundary.
        // A lone pause is an empty utterance an SSML break left behind.
        let boundary = p.is_pause() && phones.get(i + 1).is_some_and(|q| q.is_pause());
        if boundary {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// What one utterance produced, for callers that want more than the samples.
pub struct Said {
    pub phones: Vec<Phone>,
    pub names: Vec<String>,
    pub units: Vec<usize>,
    /// Units per streamed piece; see `Selector::last_pieces`.
    pub pieces: Vec<usize>,
    pub pcm: Vec<i16>,
}

/// Synthesise `text`, utterance by utterance.
pub fn say(v: &Voice, cep: &CepLex, text: &str, style: DigitStyle) -> Vec<Said> {
    say_tagged(v, cep, None, text, style)
}

/// The same with the part-of-speech tagger in front of the lexicon.
///
/// Only utterances holding one of its 237 listed homographs are tagged at all,
/// so everything else comes out exactly as `say` produces it.
pub fn say_tagged(
    v: &Voice,
    cep: &CepLex,
    pos: Option<&crate::postag::PosTag>,
    text: &str,
    style: DigitStyle,
) -> Vec<Said> {
    let (seq, _, _) = crate::lex::text_to_phones_tagged(cep, pos, text, style);
    say_phones(v, &seq)
}

/// The same from a phone sequence that has already been through the front end.
pub fn say_phones(v: &Voice, seq: &[Phone]) -> Vec<Said> {
    let mut out = Vec::new();
    say_phones_each(v, seq, |said| out.push(said));
    out
}

/// The same, handing each utterance over as it is finished.
///
/// Whole-book input is the reason this exists: `say_phones` holds every
/// utterance's audio at once, which for a long file is gigabytes of PCM, while a
/// caller writing each one straight to disk never holds more than one.
pub fn say_phones_each<F: FnMut(Said)>(v: &Voice, seq: &[Phone], each: F) {
    say_phones_each_ctl(v, seq, Port::default(), |_| Ctl::default(), each)
}

/// The port parameters that shape every unit, as SpVoice's Rate reaches them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Port {
    /// `duration_stretch`: `speech/rate` sets it to VOXWPM / wpm (@0x3e0b0),
    /// and no voice sets VOXWPM, so it is 170 / wpm.
    pub duration_stretch: f32,
    /// `f0_shift`, which `speech/pitch/shift` sets directly.
    pub f0_shift: f32,
}

impl Default for Port {
    fn default() -> Port {
        Port { duration_stretch: 1.0, f0_shift: 1.0 }
    }
}

impl Port {
    pub fn with_wpm(self, wpm: u32) -> Port {
        Port { duration_stretch: 170.0 / wpm.max(1) as f32, ..self }
    }

    /// The wpm SwiftTTSEngine.dll sends for SpVoice.Rate, -10..10: integer
    /// throughout, which is what gives 85 rather than 84 at -8.
    pub fn sapi_wpm(rate: i32) -> u32 {
        let r = rate.clamp(-10, 10);
        let pct = if r < 0 { 98 + 6 * r } else if r > 0 { 100 + 15 * r } else { 100 };
        (170 * pct / 100) as u32
    }
}

/// The same with markup controls per phone. All of them act after selection,
/// which none of them changes: volume as each unit's residual gain, rate and
/// pitch through the pitch marks laid out for it, on top of the port's.
pub fn say_phones_each_ctl<C, F>(v: &Voice, seq: &[Phone], port: Port, ctl: C, mut each: F)
where
    C: Fn(&Phone) -> Ctl,
    F: FnMut(Said),
{
    let unp = match v.unit_name_params() {
        Some(u) => u,
        None => return,
    };
    let nr = NameRules::parse(v.image(), unp);
    let Some(tc) = TargetCost::parse(v.image(), unp) else { return };
    let table = PhoneTable::learn(v, &tc);

    utterances(seq)
        .into_iter()
        .for_each(|phones| {
            let names = nr.name_utterance(&phones, |n| v.type_id(n).is_some());
            let types: Vec<usize> = names.iter().filter_map(|n| v.type_id(n)).collect();
            let kept: Vec<&Phone> = phones.iter().zip(&names)
                .filter(|(_, n)| v.type_id(n).is_some())
                .map(|(p, _)| p)
                .collect();
            let tf = crate::target::build(v, &table, &nr, &tc, &phones);
            let mut sel = Selector::new(v, SelectParams::from_voice(v));
            let chosen = sel.select_with_targets(&types, Some(&tf));
            let ctls: Vec<Ctl> = kept.iter().map(|p| ctl(p)).collect();
            let units: Vec<TargetUnit> = chosen.iter().zip(&ctls).map(|(c, k)| TargetUnit {
                start: c.start, end: c.end, target_end: 0,
                gain: if k.volume == 1.0 { GAIN_UNITY } else { gain_from_rescale(k.volume) },
            }).collect();
            // A timed pause's segment spans its duration times the port's
            // stretch (cart_duration), so its unit is stretched to that many
            // samples. The engine takes the span from the difference of two
            // truncated segment ends, which can be one sample either way.
            let stretch: Vec<f32> = units.iter().zip(&kept).zip(&ctls).map(|((u, p), k)| {
                match p.dur {
                    Some(d) => {
                        let secs = d * port.duration_stretch;
                        stretch_to(v, u.start, u.end, (secs * v.sps as f32) as i32)
                    }
                    None => k.stretch * port.duration_stretch,
                }
            }).collect();
            let f0: Vec<f32> = ctls.iter().map(|k| k.f0 * port.f0_shift).collect();
            let pcm = if stretch.iter().chain(&f0).all(|&s| s == 1.0) {
                join_units_streamed(v, &units, &sel.last_pieces)
            } else {
                join_units_timed(v, &units, &stretch, &f0, &sel.last_pieces)
            };
            each(Said {
                phones,
                names,
                units: chosen.iter().map(|c| c.unit).collect(),
                pieces: sel.last_pieces.clone(),
                pcm,
            });
        });
}

/// Synthesise marked-up text (SAPI XML or Swift SSML, see `crate::markup`),
/// handing each utterance over as it is finished.
pub fn say_marked_each<F: FnMut(Said)>(
    v: &Voice,
    cep: &CepLex,
    pos: Option<&crate::postag::PosTag>,
    m: &crate::markup::Marked,
    style: DigitStyle,
    port: Port,
    each: F,
) {
    let (seq, ctls, _, _) = crate::lex::text_to_phones_marked(cep, pos, m, style);
    say_phones_each_ctl(v, &seq, port, word_ctl(&ctls), each)
}

/// Each phone's controls from its word's, as returned by
/// `text_to_phones_marked`. Pauses belong to no word -- the engine reaches the
/// token through `R:SylStructure`, which a pause has none of -- so they keep
/// the defaults.
pub fn word_ctl(ctls: &[Ctl]) -> impl Fn(&Phone) -> Ctl + '_ {
    move |p: &Phone| match p.word_id {
        0 => Ctl::default(),
        id => ctls.get(id as usize - 1).copied().unwrap_or_default(),
    }
}

/// The port's `audio/volume` (what SpVoice.Volume sets), applied to the output
/// samples. The multiply is single precision: integer `x * p / 100` is wrong
/// at 11 of the 101 levels, this at none.
pub fn apply_volume(pcm: &mut [i16], percent: u32) {
    if percent == 100 {
        return;
    }
    let f = percent as f32 / 100.0;
    for s in pcm.iter_mut() {
        *s = (*s as f32 * f) as i16;
    }
}

/// Just the samples.
pub fn say_pcm(v: &Voice, cep: &CepLex, text: &str, style: DigitStyle) -> Vec<i16> {
    say(v, cep, text, style).into_iter().flat_map(|s| s.pcm).collect()
}
