//! The pronunciation lexicon, and text to phones.
//!
//! Cepstral compresses its own lexicon inside `ceplex_us.dll`, but nothing needs
//! to be recovered from it: festival ships the same CMU dictionary already
//! compiled, in plain text, at `lib/dicts/cmu/cmudict-0.4.out`, and it carries
//! more than the raw dictionary does -- explicit syllable boundaries and
//! per-syllable stress. `_out/pack_lex.py` flattens those 105,901 entries into the
//! binary this module reads.
//!
//! Only the alphabet needs translating. Cepstral's inventory is a reduced CMU set
//! (`h` not `hh`, `i` not `iy`, `j` not `y`) with no separate schwa, so `ax` folds
//! into `ah` and the stress digit carries the distinction. The engine's own naming
//! of "telephone" as `teh eh1 lah ah0 f ow0 n`, recovered by inverse-filtering an
//! SDK render, matches festival's `/t eh/1 /l ah/0 /f ow n/1` under exactly that
//! fold -- which is what licenses it.
//!
//! Having real syllable boundaries matters beyond pronunciation: they replace the
//! maximal-onset guess in `NameRules::next_in_syllable`, so unit names and target
//! features both follow the dictionary rather than a heuristic.

use crate::rules::Phone;
use std::io;
use std::path::Path;

const MAGIC: &[u8; 8] = b"CEPLEX01";

#[derive(Debug, Clone, PartialEq)]
pub struct Syl {
    pub stress: u8,
    pub phones: Vec<String>,
}

pub struct Lexicon {
    buf: Vec<u8>,
    count: usize,
    phones: Vec<String>,
    pos: Vec<String>,
    index_off: usize,
    data_off: usize,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn read_table(b: &[u8], o: &mut usize, n: usize) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let len = *b.get(*o)? as usize;
        *o += 1;
        let s = b.get(*o..*o + len)?;
        out.push(String::from_utf8_lossy(s).into_owned());
        *o += len;
    }
    Some(out)
}

impl Lexicon {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Lexicon> {
        let buf = std::fs::read(path)?;
        Lexicon::from_bytes(buf)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad lexicon pack"))
    }

    pub fn from_bytes(buf: Vec<u8>) -> Option<Lexicon> {
        if buf.len() < 28 || &buf[..8] != MAGIC {
            return None;
        }
        let count = u32le(&buf, 8) as usize;
        let n_phones = u32le(&buf, 12) as usize;
        let n_pos = u32le(&buf, 16) as usize;
        let index_off = u32le(&buf, 20) as usize;
        let data_off = u32le(&buf, 24) as usize;
        let mut o = 28;
        let phones = read_table(&buf, &mut o, n_phones)?;
        let pos = read_table(&buf, &mut o, n_pos)?;
        if index_off + 4 * count > buf.len() {
            return None;
        }
        Some(Lexicon { buf, count, phones, pos, index_off, data_off })
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn phone_names(&self) -> &[String] {
        &self.phones
    }

    /// Index entries are offsets into the data block, not into the file.
    fn entry_at(&self, i: usize) -> usize {
        self.data_off + u32le(&self.buf, self.index_off + 4 * i) as usize
    }

    fn word_at(&self, i: usize) -> &[u8] {
        let o = self.entry_at(i);
        let n = self.buf[o] as usize;
        &self.buf[o + 1..o + 1 + n]
    }

    fn pos_at(&self, i: usize) -> &str {
        let o = self.entry_at(i);
        let n = self.buf[o] as usize;
        &self.pos[self.buf[o + 1 + n] as usize]
    }

    fn syls_at(&self, i: usize) -> Vec<Syl> {
        let o = self.entry_at(i);
        let wl = self.buf[o] as usize;
        let mut p = o + 1 + wl + 1;
        let nsyl = self.buf[p] as usize;
        p += 1;
        let mut out = Vec::with_capacity(nsyl);
        for _ in 0..nsyl {
            let stress = self.buf[p];
            let n = self.buf[p + 1] as usize;
            p += 2;
            let phones = self.buf[p..p + n]
                .iter()
                .map(|&x| self.phones[x as usize].clone())
                .collect();
            p += n;
            out.push(Syl { stress, phones });
        }
        out
    }

    /// First index whose word is >= `w`.
    fn lower_bound(&self, w: &[u8]) -> usize {
        let (mut lo, mut hi) = (0usize, self.count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.word_at(mid) < w {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Syllables for a word. Homographs are stored under different POS tags; with
    /// none requested the untagged `nil` entry wins, since that is the general
    /// pronunciation and the tagged ones are the exceptions.
    pub fn lookup(&self, word: &str) -> Option<Vec<Syl>> {
        if let Some(s) = self.lookup_pos(word, None) {
            return Some(s);
        }
        // the dictionary drops some apostrophes -- "o'clock" is stored "oclock" --
        // so retry without them before giving up to letter-to-sound
        if word.contains('\'') {
            let bare: String = word.chars().filter(|&c| c != '\'').collect();
            return self.lookup_pos(&bare, None);
        }
        None
    }

    pub fn lookup_pos(&self, word: &str, want: Option<&str>) -> Option<Vec<Syl>> {
        let w = word.to_lowercase();
        let wb = w.as_bytes();
        let mut i = self.lower_bound(wb);
        if i >= self.count || self.word_at(i) != wb {
            return None;
        }
        let mut best = None;
        while i < self.count && self.word_at(i) == wb {
            let p = self.pos_at(i);
            if let Some(t) = want {
                if p == t {
                    return Some(self.syls_at(i));
                }
            }
            if best.is_none() || p == "nil" {
                best = Some(i);
            }
            i += 1;
        }
        best.map(|i| self.syls_at(i))
    }

    /// A word's phones, carrying the dictionary's own syllable boundaries.
    pub fn word_phones(&self, word: &str) -> Option<Vec<Phone>> {
        let syls = self.lookup(word)?;
        let mut out = Vec::new();
        for s in &syls {
            for (k, p) in s.phones.iter().enumerate() {
                out.push(Phone {
                    phone: p.clone(),
                    stress: s.stress,
                    word: Some(word.to_lowercase()),
                    word_id: 0,
                    syl_end: Some(k + 1 == s.phones.len()),
                    dur: None,
                });
            }
        }
        Some(out)
    }
}

/// One token of input text: a word to pronounce, or a break.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Word(String),
    /// Punctuation that ends a phrase; `true` for sentence-final. The second
    /// field is the mark itself, empty when the break was predicted rather than
    /// punctuated -- only the part-of-speech tagger reads it.
    Break(bool, String),
}

/// A tag per word of `toks`, `None` wherever the engine would not have tagged.
///
/// The decision is per utterance, and an utterance ends at a sentence-final
/// break -- the same boundary `speak::utterances` finds in the phone stream, so
/// the two agree on what a sentence is.
fn utterance_tags(pos: &crate::postag::PosTag, toks: &[Token]) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = Vec::new();
    // `is_word` marks which entries the tagger should hand back a tag for; the
    // punctuation it is given is context, and the caller has no slot for it.
    let mut run: Vec<(String, bool)> = Vec::new();
    let flush = |run: &mut Vec<(String, bool)>, out: &mut Vec<Option<String>>| {
        let toks: Vec<&str> = run.iter().map(|(t, _)| t.as_str()).collect();
        let words: Vec<&str> = run.iter().filter(|(_, w)| *w).map(|(t, _)| t.as_str()).collect();
        match pos.tag_tokens_if_triggered(&toks, &words) {
            Some(tags) => out.extend(
                tags.into_iter().zip(run.iter()).filter(|(_, (_, w))| *w).map(|(t, _)| Some(t)),
            ),
            None => out.extend(std::iter::repeat(None).take(words.len())),
        }
        run.clear();
    };
    for t in toks {
        match t {
            Token::Word(w) => run.push((w.clone(), true)),
            Token::Break(hard, mark) => {
                if !mark.is_empty() {
                    run.push((mark.clone(), false));
                }
                if *hard {
                    flush(&mut run, &mut out);
                }
            }
        }
    }
    // the last utterance has no trailing break, so its full stop is assumed
    run.push((".".to_string(), false));
    flush(&mut run, &mut out);
    out
}

/// Split text into words and breaks. Apostrophes stay inside words because the
/// dictionary holds contractions ("don't", "it's"); every other punctuation mark
/// either ends a phrase or is dropped.
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<Token>| {
        if !cur.is_empty() {
            out.push(Token::Word(std::mem::take(cur)));
        }
    };
    for c in text.chars() {
        if c.is_alphanumeric() || c == '\'' {
            cur.push(c.to_ascii_lowercase());
        } else {
            flush(&mut cur, &mut out);
            let hard = matches!(c, '.' | '!' | '?');
            let soft = matches!(c, ',' | ';' | ':' | '-' | '(' | ')' | '"');
            if hard || soft {
                // collapse runs of punctuation, keeping the strongest
                if let Some(Token::Break(prev, mark)) = out.last_mut() {
                    *prev |= hard;
                    *mark = c.to_string();
                } else {
                    out.push(Token::Break(hard, c.to_string()));
                }
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// What produced a word's pronunciation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    /// Straight from the dictionary, syllable boundaries and all.
    Lexicon,
    /// Predicted letter by letter. Roughly half of whole words come out right,
    /// so these are worth surfacing.
    Lts,
}

/// Text to a phone sequence, with pauses at phrase boundaries and at both ends.
///
/// The lexicon answers first; `lts` covers what is left. Words that neither can
/// pronounce are reported rather than invented. Pass `None` for `lts` to see
/// exactly which words fall outside the dictionary.
pub fn text_to_phones_with(
    lex: &Lexicon,
    lts: Option<&crate::lts::Lts>,
    text: &str,
) -> (Vec<Phone>, Vec<(String, Source)>, Vec<String>) {
    text_to_phones_styled(lex, lts, text, crate::norm::DigitStyle::default())
}

/// Text to phones using Cepstral's own lexicon and letter-to-sound.
///
/// Prefer this over the festival-derived pair: those agreed with the engine on
/// 42.9% of a 4000-word sample, because they are a different lexicon and a
/// different LTS, not an approximation of these.
pub fn text_to_phones_cepstral(
    cep: &crate::ceplex::CepLex,
    text: &str,
    style: crate::norm::DigitStyle,
) -> (Vec<Phone>, Vec<(String, Source)>, Vec<String>) {
    text_to_phones_tagged(cep, None, text, style)
}

/// The same with Cepstral's part-of-speech tagger in front of the lexicon.
///
/// The tagger runs per utterance and only for an utterance holding one of its
/// 237 listed homographs; everything else takes the untagged path and is
/// unchanged, which is what keeps ordinary prose bit-exact. Passing `None` for
/// `pos` is exactly `text_to_phones_cepstral`.
/// Every word the front end produced for `text`, with the tag the engine's
/// tagger would have given it, or `None` for an utterance it would have skipped.
///
/// The same path `text_to_phones_tagged` takes, stopping before the lexicon.
pub fn tags_for_text(
    cep: &crate::ceplex::CepLex,
    pos: &crate::postag::PosTag,
    text: &str,
    style: crate::norm::DigitStyle,
) -> Vec<(String, Option<String>)> {
    let toks = crate::norm::normalize_with(text, style, |w| cep.lookup(w).is_some());
    let tags = utterance_tags(pos, &toks);
    let mut out = Vec::new();
    let mut seen = 0usize;
    for t in &toks {
        if let Token::Word(w) = t {
            out.push((w.clone(), tags.get(seen).cloned().flatten()));
            seen += 1;
        }
    }
    out
}

pub fn text_to_phones_tagged(
    cep: &crate::ceplex::CepLex,
    pos: Option<&crate::postag::PosTag>,
    text: &str,
    style: crate::norm::DigitStyle,
) -> (Vec<Phone>, Vec<(String, Source)>, Vec<String>) {
    let toks = crate::norm::normalize_with(text, style, |w| cep.lookup(w).is_some());
    let (out, _, guessed, missing) = phones_from_tokens(cep, pos, toks, &[], Default::default());
    (out, guessed, missing)
}

/// The same from marked-up text. The second list holds each word's controls,
/// indexed by `Phone::word_id - 1`.
pub fn text_to_phones_marked(
    cep: &crate::ceplex::CepLex,
    pos: Option<&crate::postag::PosTag>,
    m: &crate::markup::Marked,
    style: crate::norm::DigitStyle,
) -> (Vec<Phone>, Vec<crate::markup::Ctl>, Vec<(String, Source)>, Vec<String>) {
    let (toks, marks, lead) =
        crate::norm::normalize_marked(m, style, |w| cep.lookup(w).is_some());
    phones_from_tokens(cep, pos, toks, &marks, lead)
}

/// swift.dll's `default_lexical_insertion` @0x14380 for a word whose phones are
/// given. With any `.` in the list, `.` closes a syllable; otherwise the
/// lexicon's boundary test (maximal onset) does, after each phone. A stress
/// digit, on a phone or alone, sets the stress of the syllable it is in; the
/// last one wins and a syllable without one is `0`. The syllable's stress goes
/// on its vowels, which is where `target` reads it back from.
pub fn phoneme_word(items: &[String], word: &str) -> Vec<Phone> {
    let dots = items.iter().any(|x| x == "." || x == "SB");
    // (phone, syllable) for each segment, and each syllable's stress
    let mut segs: Vec<(String, usize)> = Vec::new();
    let mut stress: Vec<u8> = Vec::new();
    let mut open = false;
    for (k, it) in items.iter().enumerate() {
        if !open {
            stress.push(0);
            open = true;
        }
        let syl = stress.len() - 1;
        if dots && (it == "." || it == "SB") {
            open = false;
            continue;
        }
        let (name, digit) = match it.char_indices().last() {
            Some((i, c)) if c.is_ascii_digit() && !crate::phoneset::known(it) => (&it[..i], Some(c)),
            _ => (it.as_str(), None),
        };
        if let Some(d) = digit {
            stress[syl] = d as u8 - b'0';
        }
        if name.is_empty() {
            continue;
        }
        segs.push((name.to_string(), syl));
        if !dots {
            // The boundary test (ceplex_us.dll @0x150c) sees the raw items still
            // to come. A lone stress digit is one of them, and not a vowel --
            // its vowel test is the first letter being in "aeiou" -- so it can
            // hold a consonant in the syllable before.
            let names: Vec<&str> = segs.iter().map(|(p, _)| p.as_str())
                .chain(items[k + 1..].iter().map(|x| {
                    let lone = x.len() == 1 && x.as_bytes()[0].is_ascii_digit();
                    if lone { x.as_str() } else { x.trim_end_matches(|c: char| c.is_ascii_digit()) }
                }))
                .collect();
            let i = segs.len() - 1;
            let start = segs.iter().position(|&(_, s)| s == syl).unwrap_or(i);
            if crate::rules::syl_boundary(&names, i, start) {
                open = false;
            }
        }
    }
    let lower = word.to_lowercase();
    let mut out: Vec<Phone> = Vec::with_capacity(segs.len());
    for (i, (p, syl)) in segs.iter().enumerate() {
        let last = segs.get(i + 1).map_or(true, |(_, s)| s != syl);
        let vowelless = !segs.iter().any(|(q, s)| s == syl && crate::phoneset::is_vowel(q));
        let first = i == 0 || segs[i - 1].1 != *syl;
        let carries = crate::phoneset::is_vowel(p) || (vowelless && first);
        out.push(Phone {
            phone: p.clone(),
            stress: if carries { stress[*syl] } else { 0 },
            word: Some(lower.clone()),
            word_id: 0,
            syl_end: Some(last),
            dur: None,
        });
    }
    out
}

fn timed_pause(secs: f32) -> Phone {
    Phone { dur: Some(secs), ..Phone::pause() }
}

/// `marks` is one entry per `Token::Word`, or empty for none.
fn phones_from_tokens(
    cep: &crate::ceplex::CepLex,
    pos: Option<&crate::postag::PosTag>,
    toks: Vec<Token>,
    marks: &[crate::norm::WordMarks],
    lead: crate::norm::Opens,
) -> (Vec<Phone>, Vec<crate::markup::Ctl>, Vec<(String, Source)>, Vec<String>) {
    let tags = pos.map(|p| utterance_tags(p, &toks)).unwrap_or_default();
    let pause = |d: Option<f32>| d.map_or_else(Phone::pause, timed_pause);

    // default_pause_insertion adds a break's timed pause before any phrase or
    // utterance pause exists, and those then see a pause already in place and
    // add none: the timed pause takes the place of the one that would be there.
    // An empty utterance is a lone pause, which `speak::utterances` splits off.
    let mut word_ctls = Vec::new();
    let mut out: Vec<Phone> = lead.empties.iter().map(|&d| pause(d)).collect();
    out.push(pause(lead.first));
    let mut guessed = Vec::new();
    let mut missing = Vec::new();
    let mut word_id = 0u32;
    let mut seen = 0usize;
    let mut pending: Option<f32> = None;
    let mut opens: Option<crate::norm::Opens> = None;
    // No word yet in the utterance being built. The engine synthesises an
    // utterance with no words too -- `cat .` ends one before the `.` -- and it
    // renders as the lone pause that opened it.
    let mut fresh = true;
    for t in toks {
        match t {
            Token::Word(w) => {
                out.extend(pending.take().map(timed_pause));
                opens = None;
                let tag = tags.get(seen).and_then(|t| t.as_deref());
                seen += 1;
                let mark = marks.get(seen - 1).cloned().unwrap_or_default();
                if let Some(items) = &mark.phones {
                    let p = phoneme_word(items, &w);
                    if !p.is_empty() {
                        word_id += 1;
                        word_ctls.push(mark.ctl);
                        out.extend(p.into_iter().map(|mut q| { q.word_id = word_id; q }));
                        fresh = false;
                    }
                    pending = mark.break_after;
                    opens = mark.opens;
                    continue;
                }
                let (ph, from_lts) = cep.phones_tag(&w, tag);
                if ph.is_empty() {
                    missing.push(w);
                    continue;
                }
                if from_lts {
                    guessed.push((w.clone(), Source::Lts));
                }
                if let Some(p) = cep.word_phones_tag(&w, tag) {
                    word_id += 1;
                    word_ctls.push(mark.ctl);
                    out.extend(p.into_iter().map(|mut q| { q.word_id = word_id; q }));
                    pending = mark.break_after;
                    opens = mark.opens;
                    fresh = false;
                }
            }
            // a sentence end is two pauses: one closing, one opening
            Token::Break(hard, _) => {
                if !matches!(out.last(), Some(p) if p.phone == "pau") {
                    out.push(pause(pending.take()));
                    if hard {
                        let o = opens.take().unwrap_or_default();
                        out.extend(o.empties.iter().map(|&d| pause(d)));
                        out.push(pause(o.first));
                        fresh = true;
                    }
                } else if hard && fresh {
                    // an utterance with no words ends: its opening pause stays
                    // as it is, and the next one opens
                    out.push(Phone::pause());
                }
            }
        }
    }
    if !matches!(out.last(), Some(p) if p.phone == "pau") {
        out.push(pause(pending.take()));
    }
    // a trailing pair from a final full stop collapses back to one, unless the
    // last utterance is one with no words
    while !fresh
        && out.len() >= 2
        && out[out.len() - 1].is_pause() && out[out.len() - 1].dur.is_none()
        && out[out.len() - 2].is_pause()
    {
        out.pop();
    }
    // an utterance break at the very end leaves an empty utterance open, and
    // the engine synthesises that too
    if let Some(o) = opens.take() {
        out.extend(o.empties.iter().map(|&d| pause(d)));
        out.push(pause(o.first));
    }
    crate::postlex::apply(&mut out);
    (out, word_ctls, guessed, missing)
}

pub fn text_to_phones_styled(
    lex: &Lexicon,
    lts: Option<&crate::lts::Lts>,
    text: &str,
    style: crate::norm::DigitStyle,
) -> (Vec<Phone>, Vec<(String, Source)>, Vec<String>) {
    let mut out = vec![Phone::pause()];
    let mut guessed = Vec::new();
    let mut missing = Vec::new();
    for t in crate::norm::normalize_with(text, style, |w| lex.lookup(w).is_some()) {
        match t {
            Token::Word(w) => {
                if let Some(ph) = lex.word_phones(&w) {
                    out.extend(ph);
                    continue;
                }
                match lts.map(|l| l.predict(&w)) {
                    Some(ph) if !ph.is_empty() => {
                        guessed.push((w, Source::Lts));
                        out.extend(ph);
                    }
                    _ => missing.push(w),
                }
            }
            Token::Break(_, _) => {
                if !matches!(out.last(), Some(p) if p.phone == "pau") {
                    out.push(Phone::pause());
                }
            }
        }
    }
    if !matches!(out.last(), Some(p) if p.phone == "pau") {
        out.push(Phone::pause());
    }
    (out, guessed, missing)
}

/// Lexicon only, no letter-to-sound.
pub fn text_to_phones(lex: &Lexicon, text: &str) -> (Vec<Phone>, Vec<String>) {
    let (ph, _, missing) = text_to_phones_with(lex, None, text);
    (ph, missing)
}
