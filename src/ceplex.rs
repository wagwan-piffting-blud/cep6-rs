//! Cepstral's own pronunciation front end, lifted from `ceplex_us.dll`.
//!
//! This replaces the festival-derived lexicon and letter-to-sound model. Against
//! a 4000-word sample those agreed with Cepstral on only 42.9% of words, because
//! the two systems are simply not the same: Cepstral's dictionary is 13,387
//! entries and its LTS answers everything else.
//!
//! The split is deliberate and self-explaining. The LTS predicts `colonel` as
//! `k aa1 er1 n ah0 l`, which is wrong, and `colonel` is one of the 13,387
//! dictionary entries at `k er1 n ah0 l`. The dictionary holds precisely what the
//! LTS gets wrong, which is why `the`, `of`, `one` and `said` are absent from it
//! while `colonel` and `sword` are present.
//!
//! `_out/pack_ceplex.py` decodes both out of the DLL at fixed RVAs. Decoding
//! happens there, so nothing here needs the huffman tables.
//!
//! Phones arrive with stress attached (`eh1`, `ah0`), and there is no syllable
//! structure in this lexicon, so `Phone::syl_end` is left unset and the naming
//! rules fall back to maximal onset.

use crate::rules::Phone;
use std::io;
use std::path::Path;

const MAGIC: &[u8; 8] = b"CEPSLEX1";
const NODE: usize = 6;
/// Word boundary and padding, as the model's own comparisons spell them.
const HASH: u8 = b'#';
const PAD: u8 = b'0';
/// Letters of context either side. Window 3 and 5 both fail every test word.
const WINDOW: usize = 4;

pub struct CepLex {
    buf: Vec<u8>,
    n_entries: usize,
    phones: Vec<String>,
    entries_off: usize,
    data_off: usize,
    letters: Vec<u8>,
    roots: Vec<u16>,
    lts_phones: Vec<String>,
    models_off: usize,
    n_nodes: usize,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn strtab(b: &[u8], o: &mut usize, n: usize) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let len = *b.get(*o)? as usize;
        *o += 1;
        out.push(String::from_utf8_lossy(b.get(*o..*o + len)?).into_owned());
        *o += len;
    }
    Some(out)
}

/// Split `eh1` into `("eh", 1)`. A phone with no digit is unstressed.
fn split_stress(p: &str) -> (&str, u8) {
    match p.as_bytes().last() {
        Some(c) if c.is_ascii_digit() => (&p[..p.len() - 1], c - b'0'),
        _ => (p, 0),
    }
}

impl CepLex {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<CepLex> {
        let buf = std::fs::read(path)?;
        CepLex::from_bytes(buf)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad ceplex pack"))
    }

    pub fn from_bytes(buf: Vec<u8>) -> Option<CepLex> {
        if buf.len() < 52 || &buf[..8] != MAGIC {
            return None;
        }
        let n_entries = u32le(&buf, 8) as usize;
        let n_phones = u32le(&buf, 12) as usize;
        let entries_off = u32le(&buf, 16) as usize;
        let data_off = u32le(&buf, 20) as usize;
        let letters_off = u32le(&buf, 24) as usize;
        let n_roots = u32le(&buf, 28) as usize;
        let roots_off = u32le(&buf, 32) as usize;
        let n_lts_phones = u32le(&buf, 36) as usize;
        let lts_phones_off = u32le(&buf, 40) as usize;
        let models_off = u32le(&buf, 44) as usize;
        let n_nodes = u32le(&buf, 48) as usize;

        let mut o = 52;
        let phones = strtab(&buf, &mut o, n_phones)?;

        // latin-1 bytes, kept as bytes: the table ends with accented letters and
        // UTF-8 decoding would mangle them into replacement characters
        let llen = *buf.get(letters_off)? as usize;
        let letters = buf.get(letters_off + 1..letters_off + 1 + llen)?.to_vec();
        let roots: Vec<u16> = (0..n_roots).map(|i| u16le(&buf, roots_off + 2 * i)).collect();
        let mut p = lts_phones_off;
        let lts_phones = strtab(&buf, &mut p, n_lts_phones)?;

        if models_off + n_nodes * NODE > buf.len() {
            return None;
        }
        Some(CepLex {
            buf, n_entries, phones, entries_off, data_off,
            letters, roots, lts_phones, models_off, n_nodes,
        })
    }

    pub fn len(&self) -> usize {
        self.n_entries
    }

    pub fn is_empty(&self) -> bool {
        self.n_entries == 0
    }

    pub fn nodes(&self) -> usize {
        self.n_nodes
    }

    pub fn letters(&self) -> String {
        self.letters.iter().map(|&b| b as char).collect()
    }

    fn letter_root(&self, b: u8) -> Option<usize> {
        self.letters.iter().position(|&x| x == b).map(|k| self.roots[k] as usize)
    }

    fn entry(&self, i: usize) -> usize {
        self.data_off + u32le(&self.buf, self.entries_off + 4 * i) as usize
    }

    fn word_at(&self, i: usize) -> &[u8] {
        let o = self.entry(i);
        let n = self.buf[o] as usize;
        &self.buf[o + 1..o + 1 + n]
    }

    fn pos_at(&self, i: usize) -> u8 {
        let o = self.entry(i);
        self.buf[o + 1 + self.buf[o] as usize]
    }

    fn phones_at(&self, i: usize) -> Vec<String> {
        let o = self.entry(i);
        let wl = self.buf[o] as usize;
        let n = self.buf[o + wl + 2] as usize;
        let base = o + wl + 3;
        (0..n)
            .map(|k| self.phones[self.buf[base + k] as usize].clone())
            .collect()
    }

    fn lower_bound(&self, w: &[u8]) -> usize {
        let (mut lo, mut hi) = (0usize, self.n_entries);
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

    /// Dictionary lookup.
    ///
    /// Entries are keyed on a part-of-speech byte followed by the word, and
    /// flite's `lex_lookup` builds that key as `'0' + word` when the caller has
    /// no tag (`cst_lexicon.c`, `cst_sprintf(wp, "%c%s", pos ? pos[0] : '0', word)`).
    /// So the `'0'` entry is the default, not simply the first or last: the
    /// engine says `ae1 b s t r ae0 k t` for "abstract" and
    /// `p r eh1 z ah0 n t` for "present", which are their `'0'` entries, while
    /// the `'v'` entries carry the verb stress.
    ///
    /// Among several `'0'` entries the last wins, which is what the engine
    /// returns for "hello" (`h eh1 l ow0`, the second of its two). Which of a
    /// tied pair `find_full_match` lands on depends on where the binary search
    /// stopped, so that half of the rule is measured rather than derived.
    /// The untagged lookup, which is what the synthesis path uses: the last
    /// `'0'` entry, else the last entry.
    ///
    /// Deliberately *not* `lookup_tag(word, None)`. That runs the engine's own
    /// `find_full_match`, whose fallback is the first entry of the word's block,
    /// and our pack does not preserve the DLL's order within a block -- so for a
    /// word with two `'0'` entries the two rules disagree. This one is the rule
    /// that reproduces the engine's samples over every sentence measured.
    pub fn lookup(&self, word: &str) -> Option<Vec<String>> {
        if let Some(p) = Self::addendum(word) {
            return Some(p);
        }
        let all = self.lookup_all(word);
        if all.is_empty() {
            return None;
        }
        all.iter()
            .rev()
            .find(|(pos, _)| *pos == b'0')
            .or_else(|| all.last())
            .map(|(_, p)| p.clone())
    }

    /// The entry the engine would pick for `word` given a part-of-speech tag.
    ///
    /// swift.dll tags every word with a Penn tag before it looks anything up --
    /// `record/vbp` against `record/nn` -- and `lex_lookup` keys on the tag's
    /// **first character**, which is flite's `cst_sprintf(wp, "%c%s", pos[0],
    /// word)`. So `nn` and `nns` both read the `n` entries, `vb`/`vbp`/`vbd` the
    /// `v` entries, `jj` the `j` entries, `dt` the `d` entries. A tag with no
    /// entry falls back to `'0'`, which is also what a caller with no tagger
    /// gets by passing `None`.
    ///
    /// Among several entries under one character the **last** wins: "evening"
    /// has two under `n` and the engine says `i1 v ah0 n ih0 ng`, the second.
    ///
    /// `ceps-rs` has no tagger -- see HANDOFF's "What is left" -- so the
    /// synthesis path always passes `None`. This is the half that is settled,
    /// and `tests/pos.rs` replays the engine's own tagged lookups against it.
    pub fn lookup_tag(&self, word: &str, tag: Option<&str>) -> Option<Vec<String>> {
        if let Some(p) = Self::addendum(word) {
            return Some(p);
        }
        let Some(tag) = tag else { return self.lookup(word) };
        // The lexicon's part-of-speech characters are lowercase, and the tagger's
        // outcomes are uppercase Penn tags -- swift.dll passes `nn`, not `NN`.
        let want = tag.bytes().next().unwrap_or(b'0').to_ascii_lowercase();
        let lower = word.to_lowercase();
        let wb = lower.as_bytes();

        // `lex_lookup_bsearch` compares the *word only* -- `lex_match_entry` is
        // `strcmp(a+1, b+1)`, skipping the part-of-speech character -- so it can
        // land on any entry of the word's block. `find_full_match` then walks
        // back from there and, failing that, forward, taking the first exact
        // (word, pos) it meets. Which of several entries under one character it
        // reaches therefore depends on where the search landed, and that is not
        // something a "first" or "last" rule reproduces: `lead/v` is the first
        // of its two and `evening/n` the second.
        let (mut start, mut end) = (0usize, self.n_entries);
        let mut hit = None;
        while start < end {
            let mid = (start + end) / 2;
            match self.word_at(mid).cmp(wb) {
                std::cmp::Ordering::Equal => {
                    hit = Some(mid);
                    break;
                }
                std::cmp::Ordering::Greater => end = mid,
                std::cmp::Ordering::Less => start = mid + 1,
            }
        }
        let hit = hit?;

        let mut fallback = hit;
        let mut w = hit;
        loop {
            if self.word_at(w) != wb {
                break;
            }
            if self.pos_at(w) == want {
                return Some(self.phones_at(w));
            }
            fallback = w;
            if w == 0 {
                break;
            }
            w -= 1;
        }
        let mut w = hit;
        while w < self.n_entries && self.word_at(w) == wb {
            if self.pos_at(w) == want {
                return Some(self.phones_at(w));
            }
            w += 1;
        }
        Some(self.phones_at(fallback))
    }

    /// Every entry filed under `word`, as `(pos byte, phones)`. Diagnostic: the
    /// lexicon files homographs by part of speech and `lookup` has to pick one.
    pub fn lookup_all(&self, word: &str) -> Vec<(u8, Vec<String>)> {
        let lower = word.to_lowercase();
        let wb = lower.as_bytes();
        let mut i = self.lower_bound(wb);
        let mut out = Vec::new();
        while i < self.n_entries && self.word_at(i) == wb {
            out.push((self.pos_at(i), self.phones_at(i)));
            i += 1;
        }
        out
    }

    pub fn lookup_pos(&self, word: &str, want: Option<u8>) -> Option<Vec<String>> {
        let lower = word.to_lowercase();
        let wb = lower.as_bytes();
        let mut i = self.lower_bound(wb);
        if i >= self.n_entries || self.word_at(i) != wb {
            return None;
        }
        let mut best = None;
        while i < self.n_entries && self.word_at(i) == wb {
            if let Some(t) = want {
                if self.pos_at(i) == t {
                    return Some(self.phones_at(i));
                }
            }
            best = Some(i);
            i += 1;
        }
        best.map(|i| self.phones_at(i))
    }

    fn node(&self, i: usize) -> (u8, u8, u16, u16) {
        let o = self.models_off + i * NODE;
        (self.buf[o], self.buf[o + 1], u16le(&self.buf, o + 2), u16le(&self.buf, o + 4))
    }

    fn apply_model(&self, vals: &[u8], start: usize) -> Option<u8> {
        let (mut feat, mut val, mut qt, mut qf) = self.node(start);
        for _ in 0..2000 {
            if feat == 255 {
                return Some(val);
            }
            let next = if vals.get(feat as usize) == Some(&val) { qt } else { qf };
            if next as usize >= self.n_nodes {
                return None;
            }
            let n = self.node(next as usize);
            feat = n.0;
            val = n.1;
            qt = n.2;
            qf = n.3;
        }
        None
    }

    /// Letter to sound, for words the dictionary does not hold.
    ///
    /// Letters go in as raw ASCII with `#` marking the word edge and `'0'` as
    /// padding -- the model's own comparisons are against 97..122, 35 and 48,
    /// which is what settles the encoding. Prediction runs right to left so the
    /// phones come out already in order. `epsilon` means the letter spells
    /// nothing, and a phone containing `-` is two phones.
    pub fn lts(&self, word: &str) -> Vec<String> {
        let lower = word.to_lowercase();
        // map each char to its latin-1 byte, and drop anything with no tree
        let codes: Vec<u8> = lower
            .chars()
            .filter_map(|c| u8::try_from(c as u32).ok())
            .filter(|b| self.letters.contains(b))
            .collect();
        if codes.is_empty() {
            return Vec::new();
        }

        let mut full = vec![PAD; WINDOW - 1];
        full.push(HASH);
        full.extend_from_slice(&codes);
        full.push(HASH);
        full.extend(std::iter::repeat(PAD).take(WINDOW - 1));

        let mut out: Vec<String> = Vec::new();
        let mut i = WINDOW + codes.len() - 1;
        while full[i] != HASH {
            let mut vals = Vec::with_capacity(WINDOW * 2);
            vals.extend_from_slice(&full[i - WINDOW..i]);
            vals.extend_from_slice(&full[i + 1..i + 1 + WINDOW]);

            let idx = self
                .letter_root(full[i])
                .and_then(|r| self.apply_model(&vals, r));
            i -= 1;

            let Some(k) = idx else { continue };
            let Some(ph) = self.lts_phones.get(k as usize) else { continue };
            if ph == "epsilon" {
                continue;
            }
            match ph.split_once('-') {
                Some((a, b)) => {
                    out.insert(0, b.to_string());
                    out.insert(0, a.to_string());
                }
                None => out.insert(0, ph.clone()),
            }
        }
        out
    }

    /// Dictionary first, letter to sound second -- the order `lex_lookup` uses.
    /// Entries the packed blob does not hold.
    ///
    /// swift.dll keeps two lexicons: the compressed one this reader decodes, and
    /// an addenda list with `data`/`num_bytes` NULL whose entries live as plain
    /// C arrays: `_a`, the letter A that `en_exp_letters` produces, and the
    /// punctuation marks, which reach the lexicon as words when a token is
    /// nothing else or a `<say-as>` spells one out. These are the engine's own
    /// answers, captured with `_out/run_lexdump.py`; `"`, `(` and `)` have none.
    fn addendum(word: &str) -> Option<Vec<String>> {
        let p = match word {
            "_a" => "ey1",
            "!" => "eh0 k s k l ah0 m ey1 sh ah0 n p oy1 n t",
            "#" => "h ae1 sh",
            "$" => "d aa1 l er0",
            "%" => "p er0 s eh1 n t",
            "&" => "ae1 n d",
            "'" => "ah0 p aa1 s t r ah0 f i0",
            "*" => "ae1 s t ah0 r ih0 s k",
            "+" => "p l ah1 s",
            "," => "k aa1 m ah0",
            "-" => "d ae1 sh",
            "." => "d aa1 t",
            "/" => "s l ae1 sh",
            ":" => "k ow1 l ah0 n",
            ";" => "s eh1 m i0 k ow1 l ah0 n",
            "<" => "l eh1 s dh ae1 n",
            "=" => "i1 k w ah0 l z",
            ">" => "g r ey1 t er0 dh ae1 n",
            "?" => "k w eh1 s ch ah0 n m aa1 r k",
            "@" => "ae1 t",
            "[" => "ow1 p ah0 n s k w eh1 r b r ae1 k ah0 t",
            "\\" => "b ae1 k s l ae0 sh",
            "]" => "k l ow1 z s k w eh1 r b r ae1 k ah0 t",
            "^" => "k eh1 r ih0 t",
            "_" => "ah1 n d er0 s k ao0 r",
            "`" => "b ae1 k k w ow1 t",
            "{" => "ow1 p ah0 n b r ey1 s",
            "|" => "b aa1 r",
            "}" => "k l ow1 z b r ey1 s",
            "~" => "t ih1 l d ah0",
            _ => return None,
        };
        Some(p.split(' ').map(str::to_string).collect())
    }

    pub fn phones(&self, word: &str) -> (Vec<String>, bool) {
        self.phones_tag(word, None)
    }

    /// `phones` for a word the tagger reached. `None` is the untagged rule.
    pub fn phones_tag(&self, word: &str, tag: Option<&str>) -> (Vec<String>, bool) {
        match self.lookup_tag(word, tag) {
            Some(p) => (p, false),
            None => (self.lts(word), true),
        }
    }

    /// A word's phones as `Phone`s. Stress comes off the phone name; this
    /// lexicon records no syllable boundaries, so `syl_end` stays unset and the
    /// naming rules fall back to maximal onset.
    pub fn word_phones(&self, word: &str) -> Option<Vec<Phone>> {
        self.word_phones_tag(word, None)
    }

    /// `word_phones` for a word the tagger reached.
    pub fn word_phones_tag(&self, word: &str, tag: Option<&str>) -> Option<Vec<Phone>> {
        let (ph, _) = self.phones_tag(word, tag);
        if ph.is_empty() {
            return None;
        }
        let lower = word.to_lowercase();
        Some(
            ph.iter()
                .map(|p| {
                    let (base, stress) = split_stress(p);
                    Phone {
                        phone: base.to_string(),
                        stress,
                        word: Some(lower.clone()),
                        word_id: 0,
                        syl_end: None,
                        dur: None,
                    }
                })
                .collect(),
        )
    }
}
