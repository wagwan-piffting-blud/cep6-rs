//! Text normalisation: the engine's own, not a set of English rules.
//!
//! `normalize` is a thin driver over `crate::token`, which ports flite's
//! `us_tokentowords` and reads ceplang_en.dll's two decision trees. The rules
//! there are frequently not the conventional readings -- "2017" is "two thousand
//! seventeen" and "10:00" drops the minutes entirely -- and every one of them is
//! checked against the engine rather than chosen.
//!
//! `cardinal` and `ordinal` are kept for callers that want a number spelled out
//! directly. They are not on the synthesis path.
//!
//! `DigitStyle` is the one thing here the engine has no equivalent of. The
//! number tree decides "911" from its neighbours -- "nine hundred eleven" in
//! `The number is 911.`, "nine one one" in `A 911 telephone outage emergency.`
//! -- and `DigitStyle::Digits` overrides it to the code reading for a bare two-
//! or three-digit token, which is what alerting text wants.

use crate::lex::Token;

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen",
    "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
const SCALES: [(u64, &str); 4] = [
    (1_000_000_000, "billion"),
    (1_000_000, "million"),
    (1_000, "thousand"),
    (100, "hundred"),
];
const ORDINAL_ONES: [&str; 20] = [
    "zeroth", "first", "second", "third", "fourth", "fifth", "sixth", "seventh",
    "eighth", "ninth", "tenth", "eleventh", "twelfth", "thirteenth", "fourteenth",
    "fifteenth", "sixteenth", "seventeenth", "eighteenth", "nineteenth",
];
const ORDINAL_TENS: [&str; 10] = [
    "", "", "twentieth", "thirtieth", "fortieth", "fiftieth", "sixtieth",
    "seventieth", "eightieth", "ninetieth",
];

/// How to read a run of digits that could be a quantity or a code.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum DigitStyle {
    /// 911 is "nine hundred eleven". Right for prose.
    #[default]
    Cardinal,
    /// 911 is "nine one one". Right for codes, phone numbers and alerting text.
    Digits,
}

pub fn cardinal(n: u64, out: &mut Vec<String>) {
    if n < 20 {
        out.push(ONES[n as usize].into());
        return;
    }
    if n < 100 {
        let (t, r) = (n / 10, n % 10);
        if r == 0 {
            out.push(TENS[t as usize].into());
        } else {
            out.push(format!("{}-{}", TENS[t as usize], ONES[r as usize]));
        }
        return;
    }
    for (scale, name) in SCALES {
        if n >= scale {
            cardinal(n / scale, out);
            out.push(name.into());
            let r = n % scale;
            if r > 0 {
                cardinal(r, out);
            }
            return;
        }
    }
}

pub fn ordinal(n: u64, out: &mut Vec<String>) {
    if n < 20 {
        out.push(ORDINAL_ONES[n as usize].into());
        return;
    }
    if n < 100 {
        let (t, r) = (n / 10, n % 10);
        if r == 0 {
            out.push(ORDINAL_TENS[t as usize].into());
        } else {
            out.push(format!("{}-{}", TENS[t as usize], ORDINAL_ONES[r as usize]));
        }
        return;
    }
    // "one hundred first": only the final group becomes ordinal
    let mut head = Vec::new();
    for (scale, name) in SCALES {
        if n >= scale {
            cardinal(n / scale, &mut head);
            head.push(name.into());
            let r = n % scale;
            out.append(&mut head);
            if r > 0 {
                ordinal(r, out);
            } else {
                let last = out.pop().unwrap();
                out.push(format!("{last}th"));
            }
            return;
        }
    }
}

/// Normalise text into the token stream the lexicon expects.
///
/// Without a dictionary to consult, every all-capitals token is spelled out. That
/// is the conservative reading: "n a s a" is merely odd, while a wrong guess at an
/// unknown acronym is unintelligible.
pub fn normalize(text: &str, style: DigitStyle) -> Vec<Token> {
    normalize_with(text, style, |_| false)
}

/// Normalise, consulting `known` for whether a lowercased token is a real word.
/// Pass the lexicon here so acronyms resolve the way the dictionary sees them.
pub fn normalize_with<F: Fn(&str) -> bool>(
    text: &str,
    style: DigitStyle,
    known: F,
) -> Vec<Token> {
    let toks = crate::token::tokenize(text);
    normalize_engine(toks, &known, style == DigitStyle::Digits, |_| {})
}

/// What markup left on one `Token::Word`.
#[derive(Debug, Clone, Default)]
pub struct WordMarks {
    /// The controls of the first character of the token it came from.
    pub ctl: crate::markup::Ctl,
    /// Seconds of silence after it. swift.dll puts a `<break>` on the last
    /// token read before it (@0x3a220), and `default_pause_insertion` puts the
    /// pause after that token's last word.
    pub break_after: Option<f32>,
    /// Utterance breaks after it: what opens the utterances that follow.
    pub opens: Option<Opens>,
    /// A `<phoneme>` word's items, which take the lexicon's place.
    pub phones: Option<Vec<String>>,
}

/// What follows an utterance break (@0x38e70): the utterance open at the time
/// is synthesised even when nothing has been read into it, and an empty one
/// renders as a lone pause.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Opens {
    /// The duration of each empty utterance's lone pause, in order.
    pub empties: Vec<Option<f32>>,
    /// The duration of the next utterance's opening pause.
    pub first: Option<f32>,
}

/// The `duration` default_pause_insertion gives a paragraph's opening pause:
/// the float cst_val at swift.dll 0x760b0.
pub const PARAGRAPH_PAUSE: f32 = 0.5;

/// The same, also returning what the markup left on each `Token::Word`, and
/// what opens the text when breaks come before any token. `Opens::first` is
/// then the first utterance's opening pause.
pub fn normalize_marked<F: Fn(&str) -> bool>(
    m: &crate::markup::Marked,
    style: DigitStyle,
    known: F,
) -> (Vec<Token>, Vec<WordMarks>, Opens) {
    use crate::markup::Break;
    let (mut toks, mut starts) = crate::token::tokenize_split(&m.text, &m.splits);
    for (t, &s) in toks.iter_mut().zip(&starts) {
        t.nsw = m.nsw_at(s).map(str::to_string);
    }
    // @0x3ae60 appends a <phoneme> to the Token relation as it is read: named
    // with its `ph`, no whitespace, no punctuation
    for (at, ph, words) in &m.phonemes {
        let k = starts.partition_point(|&s| s < *at);
        let t = crate::token::Tok { name: ph.clone(), phones: Some(words.clone()),
                                    ..Default::default() };
        toks.insert(k, t);
        starts.insert(k, *at);
    }

    // Walk the breaks in order, grouped by the last token read before each.
    // Until an utterance break the token takes them; after one the new
    // utterance is empty, a timed break goes onto an empty token in it
    // (becoming its opening pause), and a further utterance break synthesises
    // it as a lone pause. A paragraph's 0.5 s is set last, so it wins.
    struct Group {
        tok: Option<usize>,
        pause: Option<f32>,
        opens: Option<Opens>,
    }
    let mut groups: Vec<Group> = Vec::new();
    // Some((paragraph, pause)) while the open utterance is still empty
    let mut empty: Option<(bool, Option<f32>)> = None;
    for &(at, b) in &m.breaks {
        let tok = starts.partition_point(|&s| s < at).checked_sub(1);
        if groups.last().map_or(true, |g| g.tok != tok) {
            // before any token the first utterance is already open and empty
            empty = if tok.is_none() { Some((false, None)) } else { None };
            let opens = if tok.is_none() { Some(Opens::default()) } else { None };
            groups.push(Group { tok, pause: None, opens });
        }
        let g = groups.last_mut().unwrap();
        match (b, &mut empty, tok) {
            (Break::Pause(s), None, Some(t)) => {
                g.pause = Some(s);
                toks[t].brk = true;
            }
            (Break::Phrase, None, Some(t)) => toks[t].brk = true,
            (Break::Pause(s), Some((_, pause)), _) => *pause = Some(s),
            (Break::Utterance { paragraph }, e, t) => {
                let o = g.opens.get_or_insert_with(Opens::default);
                match (e.take(), t) {
                    (Some((para, pause)), _) => {
                        o.empties.push(if para { Some(PARAGRAPH_PAUSE) } else { pause });
                    }
                    (None, Some(t)) => toks[t].utt_end = true,
                    (None, None) => {}
                }
                *e = Some((paragraph, None));
            }
            _ => {}
        }
        if let (Some(o), Some((para, pause))) = (&mut g.opens, &empty) {
            o.first = if *para { Some(PARAGRAPH_PAUSE) } else { *pause };
        }
    }
    let lead = groups.first()
        .filter(|g| g.tok.is_none())
        .and_then(|g| g.opens.clone())
        .unwrap_or_default();

    let phones: Vec<Option<Vec<Vec<String>>>> = toks.iter().map(|t| t.phones.clone()).collect();
    let mut marks: Vec<WordMarks> = Vec::new();
    let mut word_tok: Vec<usize> = Vec::new();
    let out = normalize_engine(toks, &known, style == DigitStyle::Digits, |tok| {
        // which of a <phoneme>'s `|` words this is
        let nth = word_tok.iter().rev().take_while(|&&t| t == tok).count();
        marks.push(WordMarks {
            ctl: m.ctl_at(starts[tok]),
            phones: phones[tok].as_ref().and_then(|w| w.get(nth).cloned()),
            ..Default::default()
        });
        word_tok.push(tok);
    });
    for g in groups.into_iter().filter(|g| g.tok.is_some()) {
        let t = g.tok.unwrap();
        if let Some(w) = word_tok.iter().rposition(|&w| w <= t) {
            // a second break on the same token replaces the first, as item_set does
            if g.pause.is_some() {
                marks[w].break_after = g.pause;
            }
            if g.opens.is_some() {
                marks[w].opens = g.opens;
            }
        }
    }
    (out, marks, lead)
}

/// The engine's own path: flite's tokenizer and `us_tokentowords`, with
/// ceplang_en.dll's two decision trees. See `crate::token`.
///
/// A hard break is a sentence end, which the engine renders as two pause units
/// because it starts a new utterance; a phrase break inside one is a single
/// pause. `on_word` hears each `Token::Word` as it is pushed, with the index
/// over the whole text of the token it came from.
fn normalize_engine<F: Fn(&str) -> bool>(
    toks: Vec<crate::token::Tok>,
    known: &F,
    digits: bool,
    mut on_word: impl FnMut(usize),
) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let utts = crate::token::utterances(toks);
    let n = utts.len();
    let mut base = 0usize;
    for (u, toks) in utts.iter().enumerate() {
        let from = out.len();
        for w in crate::token::utterance_words(toks, &|s: &str| known(s), digits) {
            on_word(base + w.tok);
            // "prince-edward" reaches the lexicon whole, as the engine sends it
            out.push(Token::Word(w.text.clone()));
            if w.brk {
                out.push(Token::Break(false, w.punc.clone()));
            }
        }
        // the mark that ended the utterance is on the break we are about to
        // drop; the tagger wants it even though the phone stream does not
        // Only this utterance's own: one with no words (`cat .` puts the `.` in
        // an utterance of its own) must not eat the break that ended the last.
        let mut mark = String::new();
        while out.len() > from {
            let Some(Token::Break(_, m)) = out.last() else { break };
            if !m.is_empty() {
                mark = m.clone();
            }
            out.pop();
        }
        // Nothing is pushed after the last utterance: a trailing break would add
        // pauses the engine does not make. Its closing mark is therefore the one
        // the tagger never sees, and `PosTag::tag_utterance` assumes a full stop.
        if u + 1 < n {
            out.push(Token::Break(true, mark));
        }
        base += toks.len();
    }
    out
}
