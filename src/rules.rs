//! Unit naming: phone plus context -> one of the voice's type names.
//!
//! The rule lists live in `voice_u.dat` under `unit_name_params`, as the same
//! type 0x37 cons cells the target cost uses:
//!
//! ```text
//! stressvowels    vowels that carry a stress digit        aa ae ah ao ...
//! blist           head phone -> followers that fuse       (k w aa ae ...) (aa1 l r)
//! fwords          function words with dedicated units     and for to the of at an are
//! dlist           nasal-context splits                    (dh n ng) (dhthe n ng)
//! unvoicedstops   get a <stop>S release unit              p k t
//! palatalizables  get an ONSET unit                       jh zh
//! ```
//!
//! Generating every name the rules allow covers 345/348 of Allison's types,
//! 332/334 of David's and 334/336 of William's. The stragglers are `s` and `z`,
//! phones that appear in no rule at all; the phone inventory proper belongs to
//! the language module, not to these lists.
//!
//! One condition is not in the data. `blist` licenses `eh1` + `l`, but the
//! engine's own output for "telephone" (te-le-phone) uses `eh1`, not `eh1l`,
//! while fusing `t`+`eh` and `l`+`ah` in the same word. So a vowel only absorbs
//! a following consonant when that consonant is in the same syllable. Callers
//! pass that in as `next_in_syllable`; see `no_sylfinalv`, which is `(filler)`
//! in all four of these voices.

use crate::expr::list_items;
use crate::val::{cstr, i32le, u16le};
use std::collections::{HashMap, HashSet};

const T_STRING: u16 = 0x33;
const T_CONS: u16 = 0x37;

#[derive(Debug, Default, Clone)]
pub struct NameRules {
    pub stress_vowels: HashSet<String>,
    pub blist: HashMap<String, Vec<String>>,
    pub fwords: Vec<String>,
    pub dlist: Vec<Vec<String>>,
    pub unvoiced_stops: Vec<String>,
    pub palatalizables: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pau {
    Start,
    Mid,
    End,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PhoneCtx<'a> {
    pub phone: &'a str,
    pub stress: u8,
    pub next: Option<&'a str>,
    /// The phone before this one, for the `dlist` nasal context. It crosses word
    /// boundaries: the engine names the `dh` of "the" in "during the outage"
    /// `dhthe_ng`, taking the `ng` that ends the previous word.
    pub prev: Option<&'a str>,
    /// Whether `next` belongs to this phone's syllable. Gates vowel fusion.
    pub next_in_syllable: bool,
    /// Whether `prev` belongs to this phone's syllable. The `S` rule asks for
    /// `R:SylStructure.p.name`, which is NULL at a syllable start, so a stop
    /// opening a syllable after a coda /s/ is not an s-cluster.
    pub prev_in_syllable: bool,
    /// The word this phone belongs to, lowercased, for the `fwords` rule.
    pub word: Option<&'a str>,
    pub pau: Option<Pau>,
}

fn str_at(b: &[u8], o: usize) -> Option<String> {
    if u16le(b, o) != T_STRING {
        return None;
    }
    Some(cstr(b, (o as i64 + i32le(b, o + 4) as i64) as usize).to_string())
}

fn string_list(b: &[u8], o: usize) -> Vec<String> {
    list_items(b, o).into_iter().filter_map(|i| str_at(b, i)).collect()
}

fn list_of_lists(b: &[u8], o: usize) -> Vec<Vec<String>> {
    list_items(b, o)
        .into_iter()
        .filter(|&i| u16le(b, i) == T_CONS)
        .map(|i| string_list(b, i))
        .filter(|r| !r.is_empty())
        .collect()
}

impl NameRules {
    pub fn parse(image: &[u8], unit_name_params: usize) -> NameRules {
        let table = crate::val::walk(image, unit_name_params);
        let find = |k: &str| table.iter().find(|(n, _)| *n == k).map(|(_, o)| *o);
        let mut r = NameRules::default();
        if let Some(o) = find("stressvowels") {
            r.stress_vowels = string_list(image, o).into_iter().collect();
        }
        if let Some(o) = find("fwords") {
            r.fwords = string_list(image, o);
        }
        if let Some(o) = find("unvoicedstops") {
            r.unvoiced_stops = string_list(image, o);
        }
        if let Some(o) = find("palatalizables") {
            r.palatalizables = string_list(image, o);
        }
        if let Some(o) = find("dlist") {
            r.dlist = list_of_lists(image, o);
        }
        if let Some(o) = find("blist") {
            for row in list_of_lists(image, o) {
                let (head, rest) = row.split_first().unwrap();
                r.blist.insert(head.clone(), rest.to_vec());
            }
        }
        r
    }

    pub fn is_vowel(&self, p: &str) -> bool {
        self.stress_vowels.contains(p)
    }

    /// The type name for one phone in context.
    pub fn unit_name(&self, c: &PhoneCtx) -> String {
        if let Some(p) = c.pau {
            return match p {
                Pau::Start => "paustart",
                Pau::Mid => "paumid",
                Pau::End => "pauend",
            }
            .to_string();
        }

        let mut base = if self.is_vowel(c.phone) {
            format!("{}{}", c.phone, c.stress.min(2))
        } else {
            c.phone.to_string()
        };

        // An unvoiced stop *after* /s/ is unaspirated and gets its own unit.
        // ceplang_en.dll @0x4d95c: `member(name, unvoicedstops)` and then
        // `streq(ffeature_string(item, "R:SylStructure.p.name"), "s")` -- the
        // phone before, inside the syllable, not the phone after. William has
        // 926 `tS` units against 26 `tao`, which is the same fact from the
        // database side.
        if self.unvoiced_stops.iter().any(|s| s == c.phone)
            && c.prev == Some("s")
            && c.prev_in_syllable
        {
            base.push('S');
        }

        // fuse a following phone when blist licenses it; for a vowel head the
        // follower must be in the same syllable
        if let Some(nx) = c.next {
            if let Some(follows) = self.blist.get(&base) {
                let vowel_ok = !self.is_vowel(c.phone) || c.next_in_syllable;
                if vowel_ok && follows.iter().any(|f| f == nx) {
                    base.push_str(nx);
                }
            }
        }

        // function words get their own units
        if let Some(w) = c.word {
            if self.fwords.iter().any(|f| f == w) {
                base.push_str(w);
            }
        }

        // dlist appends a nasal context marker, after any fword suffix. The
        // context is the phone *before*, which is what the engine's own output
        // for "during the outage" shows: `ng dhthe_ng i1the`.
        if let Some(pv) = c.prev {
            for row in &self.dlist {
                if row[0] == base && row[1..].iter().any(|n| n == pv) {
                    base.push('_');
                    base.push_str(pv);
                    break;
                }
            }
        }

        // palatalizable onsets
        if self.palatalizables.iter().any(|p| p == c.phone) && c.next_in_syllable && base == c.phone
        {
            base.push_str("ONSET");
        }

        base
    }

    /// Every name these rules can produce, given a phone inventory.
    pub fn generate(&self, phones: &[String]) -> HashSet<String> {
        let mut bases: HashSet<String> = phones.iter().cloned().collect();
        bases.extend(self.blist.keys().cloned());
        for f in self.blist.values() {
            bases.extend(f.iter().cloned());
        }
        bases.extend(self.unvoiced_stops.iter().cloned());
        bases.extend(self.palatalizables.iter().cloned());
        for row in &self.dlist {
            bases.insert(row[0].clone());
        }
        for v in &self.stress_vowels {
            for s in 0..3 {
                bases.insert(format!("{v}{s}"));
            }
        }

        let mut stage: HashSet<String> = bases.clone();
        for (head, follows) in &self.blist {
            for f in follows {
                stage.insert(format!("{head}{f}"));
            }
        }
        for s in &self.unvoiced_stops {
            let sr = format!("{s}S");
            stage.insert(sr.clone());
            if let Some(follows) = self.blist.get(&sr) {
                for f in follows {
                    stage.insert(format!("{sr}{f}"));
                }
            }
        }

        let mut out = stage.clone();
        for p in &self.palatalizables {
            out.insert(format!("{p}ONSET"));
        }
        for n in &stage {
            for w in &self.fwords {
                out.insert(format!("{n}{w}"));
            }
        }
        for row in &self.dlist {
            for nasal in &row[1..] {
                out.insert(format!("{}_{}", row[0], nasal));
                for w in &self.fwords {
                    out.insert(format!("{}{}_{}", row[0], w, nasal));
                }
            }
        }
        out.insert("paustart".into());
        out.insert("paumid".into());
        out.insert("pauend".into());
        out
    }
}

/// One phone of an utterance: its symbol, its stress, the word it came from, and
/// whether it closes its syllable.
///
/// `syl_end` is `None` when nothing knows, and maximal onset is used instead. The
/// lexicon fills it in, which is strictly better: English syllabification is not
/// fully predictable from the phone string, so a dictionary boundary beats any
/// rule. "hello" is `/h ah/0 /l ow/1` there, and no onset rule would put the
/// boundary anywhere else, but plenty of words are less obliging.
#[derive(Debug, Clone, PartialEq)]
pub struct Phone {
    pub phone: String,
    pub stress: u8,
    pub word: Option<String>,
    /// Which word occurrence this phone belongs to, counted from 1.
    ///
    /// The text alone cannot say: "nine one one" is three words and two of them
    /// are spelled the same, so comparing `word` merges them into a single
    /// six-phone word and every structural feature downstream follows it.
    pub word_id: u32,
    pub syl_end: Option<bool>,
    /// An explicit duration in seconds, the segment `duration` feature. Only
    /// the silence `<break>` inserts carries one; its unit is stretched to fit.
    pub dur: Option<f32>,
}

impl Phone {
    pub fn new(phone: &str, stress: u8) -> Phone {
        Phone { phone: phone.to_string(), stress, word: None, word_id: 0, syl_end: None, dur: None }
    }

    pub fn pause() -> Phone {
        Phone { phone: "pau".into(), stress: 0, word: None, word_id: 0, syl_end: Some(true), dur: None }
    }

    pub fn is_pause(&self) -> bool {
        self.phone == "pau"
    }
}

/// The onset clusters Cepstral licenses, from ceplex_us.dll: the bigram table at
/// RVA 0x6a3a0 and the trigram table at 0x6a4e0.
///
/// These are flite's `cmulex_onset_bigrams` and `_trigrams` in Cepstral's phone
/// set -- `j` where cmulex writes `y`, `h` where it writes `hh`. They live in the
/// US English lexicon module, so every en-US voice shares them.
pub const ONSET_BIGRAMS: [&str; 39] = [
    "bj", "bl", "br", "dr", "dw", "fj", "fl", "fr", "gl", "gr", "gw", "hj", "kj",
    "kl", "kr", "kw", "mj", "pj", "pl", "pr", "pw", "sl", "sw", "sp", "st", "sk",
    "sf", "sm", "sn", "shj", "shr", "tr", "tw", "thj", "thr", "thw", "vj", "vw",
    "zw",
];
pub const ONSET_TRIGRAMS: [&str; 8] =
    ["skj", "skl", "skr", "skw", "spj", "spl", "spr", "str"];

/// Does a syllable boundary fall after `phones[i]`?
///
/// ceplex_us.dll @0x150c, which is flite's `cmu_syl_boundary_mo`: maximal onset,
/// bounded by the cluster tables above. `phones` is one word -- the engine
/// syllabifies a lexicon entry at a time, so the following word never pulls a
/// consonant across. `syl_start` is where the syllable being built began.
///
/// Checked against the engine's own `syl_numphones`: "Mister" splits `m ih1` +
/// `s t er0`, "Abstract" `ae0 b` + `s t r ae1 k t`, "extra" `eh1 k` +
/// `s t r ah0`, "monsters" `m aa1 n` + `s t er0 z`, and "Calls" stays one
/// syllable of four because nothing after `ao1` is a vowel.
pub fn syl_boundary(phones: &[&str], i: usize, syl_start: usize) -> bool {
    let rest = &phones[i + 1..];
    let Some(&first) = rest.first() else { return true };
    if first == "pau" {
        return true;
    }
    if !rest.iter().any(|p| crate::phoneset::is_vowel(p)) {
        return false; // no more vowels, so the rest is all coda
    }
    if !phones[syl_start..=i].iter().any(|p| crate::phoneset::is_vowel(p)) {
        return false; // this syllable has no vowel yet
    }
    if crate::phoneset::is_vowel(first) {
        return true;
    }
    if first == "ng" {
        return false; // cannot open a word-internal syllable
    }
    let d2v = rest.iter().position(|p| crate::phoneset::is_vowel(p)).unwrap_or(usize::MAX);
    match d2v {
        0 | 1 => true,
        2 => {
            let s = format!("{}{}", rest[0], rest[1]);
            ONSET_BIGRAMS.contains(&s.as_str())
        }
        3 => {
            let s = format!("{}{}{}", rest[0], rest[1], rest[2]);
            ONSET_TRIGRAMS.contains(&s.as_str())
        }
        _ => false,
    }
}

impl NameRules {
    /// Maximal onset. A consonant belongs with a following vowel; a vowel keeps a
    /// following consonant only when that consonant is not itself the onset of the
    /// next syllable. Matches the engine's own naming of "telephone" as
    /// `teh eh1 lah ah0 f ow0 n`: /eh/ releases the /l/ because /l/ opens the next
    /// syllable, while /t/ and /l/ each take the vowel after them.
    pub fn next_in_syllable(&self, phones: &[Phone], i: usize) -> bool {
        let Some(next) = phones.get(i + 1) else { return false };
        // A syllable never spans two words. Without this, maximal onset pulls the
        // final consonant of one word onto the first vowel of the next: "civil
        // authority" came out `ah0 lao` where the engine has `ah0l l`, because
        // the /l/ closing "civil" was treated as the onset of "authority".
        if let (Some(a), Some(b)) = (&phones[i].word, &next.word) {
            if a != b {
                return false;
            }
        }
        // the lexicon knows better than any rule, so defer to it when it spoke
        if let Some(ends) = phones[i].syl_end {
            return !ends && !next.is_pause();
        }
        let this_v = self.is_vowel(&phones[i].phone);
        if self.is_vowel(&next.phone) {
            return !this_v;
        }
        // A vowel keeps a following consonant unless that consonant opens the
        // next syllable -- but only a syllable of the SAME word can claim it.
        // The /l/ ending "civil" is a coda however "authority" begins, which is
        // why the engine has `ah0l l` where an unbounded lookahead gives `ah0 lao`.
        let steals = match (phones.get(i + 2), &next.word) {
            (Some(after), w) => {
                let same = match (&after.word, w) {
                    (Some(_), Some(_)) => after.word_id == next.word_id,
                    _ => true,
                };
                same && self.is_vowel(&after.phone)
            }
            (None, _) => false,
        };
        this_v && !steals
    }

    /// Name every phone of an utterance, marking pauses by position.
    ///
    /// The recorded inventory is sparse -- William has `fah` but no `fow`, `h` but
    /// no `heh` in some voices -- so `exists` is consulted and a fused name that is
    /// not in this voice falls back to the unfused form. Pass `|_| true` to see the
    /// rules' unconstrained output.
    pub fn name_utterance<F: Fn(&str) -> bool>(&self, phones: &[Phone], exists: F) -> Vec<String> {
        let n = phones.len();
        let plan = crate::target::plan(self, phones);
        (0..n)
            .map(|i| {
                let p = &phones[i];
                // The follower a blist head fuses with is
                // `R:SylStructure.n.name` (ceplang_en.dll @0x4dbb1): the next
                // phone only when it is in the same syllable, for consonant
                // heads as much as vowels. So /l/ ending "civil" never fuses
                // with "authority", and neither does a coda consonant with the
                // vowel after it, which only a <phoneme> can produce.
                let next = phones.get(i + 1)
                    .filter(|_| !plan[i].syl_final)
                    .map(|q| q.phone.as_str());
                // A sentence boundary is TWO pause units in the engine's output,
                // `pauend` closing one utterance and `paustart` opening the next,
                // so a pause's role has to come from its neighbours rather than
                // from its position alone. Emitting a single `paumid` there
                // shifts every later unit by one and destroys any alignment.
                let pau = if p.phone == "pau" {
                    let prev_pau = i > 0 && phones[i - 1].is_pause();
                    let next_pau = i + 1 < n && phones[i + 1].is_pause();
                    Some(if i == 0 || prev_pau {
                        Pau::Start
                    } else if i + 1 == n || next_pau {
                        Pau::End
                    } else {
                        Pau::Mid
                    })
                } else {
                    None
                };
                let in_syl = self.next_in_syllable(phones, i);
                let mk = |f| {
                    self.unit_name(&PhoneCtx {
                        phone: &p.phone,
                        stress: p.stress,
                        next,
                        prev: if i > 0 { Some(phones[i - 1].phone.as_str()) } else { None },
                        next_in_syllable: f,
                        prev_in_syllable: i > 0 && !plan[i - 1].syl_final,
                        word: p.word.as_deref(),
                        pau,
                    })
                };
                let nm = mk(in_syl);
                if exists(&nm) {
                    return nm;
                }
                let alt = mk(!in_syl);
                if exists(&alt) {
                    alt
                } else {
                    nm
                }
            })
            .collect()
    }
}
