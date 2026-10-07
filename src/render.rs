//! Text (plain or marked up) to audio in one call, with the port settings
//! SAPI exposes: the path `cep6 ... file` takes, for callers that link the
//! library instead of running the binary.

use crate::ceplex::CepLex;
use crate::markup::MarkupError;
use crate::postag::PosTag;
use crate::speak::Port;
use crate::{DigitStyle, Voice};

/// What SpVoice and the markup switch set. The defaults leave the engine's
/// plain path exactly as it is.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Read the text as SAPI XML or Swift SSML.
    pub markup: bool,
    /// SpVoice.Rate, -10..10.
    pub rate: i32,
    /// speech/pitch/shift, a multiplier.
    pub pitch_shift: f32,
    /// SpVoice.Volume, a percentage.
    pub volume: u32,
    pub digits: DigitStyle,
}

impl Default for Options {
    fn default() -> Options {
        Options { markup: false, rate: 0, pitch_shift: 1.0, volume: 100,
                  digits: DigitStyle::Cardinal }
    }
}

impl Options {
    pub fn port(&self) -> Port {
        let mut port = Port::default();
        if self.rate != 0 {
            port = port.with_wpm(Port::sapi_wpm(self.rate));
        }
        port.f0_shift = self.pitch_shift;
        port
    }
}

/// What the render left out.
#[derive(Debug, Default)]
pub struct Report {
    /// Markup kept only as text.
    pub ignored: Vec<String>,
    /// Words neither the lexicon nor letter-to-sound could say.
    pub missing: Vec<String>,
    pub samples: usize,
}

/// Render `text`, handing `each` the audio an utterance at a time.
pub fn render_each<F: FnMut(&[i16])>(
    v: &Voice,
    cep: &CepLex,
    pos: Option<&PosTag>,
    text: &str,
    o: &Options,
    mut each: F,
) -> Result<Report, MarkupError> {
    let mut report = Report::default();
    let (seq, ctls) = if o.markup {
        let m = crate::markup::parse(text)?;
        report.ignored = m.ignored.clone();
        let (s, c, _, mi) = crate::lex::text_to_phones_marked(cep, pos, &m, o.digits);
        report.missing = mi;
        (s, c)
    } else {
        let (s, _, mi) = crate::lex::text_to_phones_tagged(cep, pos, text, o.digits);
        report.missing = mi;
        (s, Vec::new())
    };
    crate::speak::say_phones_each_ctl(v, &seq, o.port(), crate::speak::word_ctl(&ctls), |said| {
        let mut pcm = said.pcm;
        crate::speak::apply_volume(&mut pcm, o.volume);
        report.samples += pcm.len();
        each(&pcm);
    });
    Ok(report)
}

/// The same, all at once.
pub fn render(
    v: &Voice,
    cep: &CepLex,
    pos: Option<&PosTag>,
    text: &str,
    o: &Options,
) -> Result<(Vec<i16>, Report), MarkupError> {
    let mut pcm = Vec::new();
    let r = render_each(v, cep, pos, text, o, |p| pcm.extend_from_slice(p))?;
    Ok((pcm, r))
}
