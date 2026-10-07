//! Tokenisation and text normalisation, as swift.dll and ceplang_en.dll do it.
//!
//! The tokenizer, flite's number expansions, and phrasing. The rules that turn
//! a token into words are ceplang_en.dll's own, in `crate::t2w`; they are not
//! conventional English, and the differences are load-bearing: "911" is "nine
//! hundred eleven" in `The number is 911.` and "nine one one" in `A 911
//! telephone outage emergency.`, and the only thing that decides it is a
//! decision tree trained on newspaper text. That tree, and the one that places
//! phrase breaks, are read out of ceplang_en.dll into `carts.rs`.
//!
//! `_out/probe_norm.txt` and `_out/probe_break.txt` hold the captures behind the
//! expansions here; `_out/run_split.py` prints the engine's own Token, Word and
//! Segment relations for any text.

use crate::carts::{Node, PHRASING_FEATS, PHRASING_NODES};

pub(crate) const DIGIT2NUM: [&str; 10] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
];
const DIGIT2TEEN: [&str; 10] = [
    "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen",
    "seventeen", "eighteen", "nineteen",
];
const DIGIT2ENTY: [&str; 10] = [
    "zero", "ten", "twenty", "thirty", "forty", "fifty", "sixty", "seventy",
    "eighty", "ninety",
];
const ORD2NUM: [&str; 10] = [
    "zeroth", "first", "second", "third", "fourth", "fifth", "sixth", "seventh",
    "eighth", "ninth",
];
const ORD2TEEN: [&str; 10] = [
    "tenth", "eleventh", "twelfth", "thirteenth", "fourteenth", "fifteenth",
    "sixteenth", "seventeenth", "eighteenth", "nineteenth",
];
const ORD2ENTY: [&str; 10] = [
    "zeroth", "tenth", "twentieth", "thirtieth", "fortieth", "fiftieth",
    "sixtieth", "seventieth", "eightieth", "ninetieth",
];

const MONTHS: [&str; 24] = [
    "jan", "january", "feb", "february", "mar", "march", "apr", "april",
    "may", "jun", "june", "jul", "july", "aug", "august", "sep", "sept",
    "september", "oct", "october", "nov", "november", "dec", "december",
];
const DAYS: [&str; 15] = [
    "sun", "sunday", "mon", "monday", "tue", "tues", "tuesday", "wed",
    "wednesday", "thu", "thurs", "thursday", "fri", "friday", "saturday",
];

// ceplang_en.dll @0xb57da0..: its text_singlecharsymbols is empty, where flite's
// default is "(){}[]", so "(555)" stays one token.
const PREPUNCT: &str = "\"'`({[";
const POSTPUNCT: &str = "\"'`.,:;!?(){}[]";

/// One token as the engine's tokenizer produces it: the punctuation is split off
/// but kept, because `punc` is what the phrasing tree reads.
#[derive(Debug, Clone, Default)]
pub struct Tok {
    pub pre: String,
    pub name: String,
    pub punc: String,
    /// The whitespace *before* this token. `utt_break` needs it.
    pub ws: String,
    /// Marked `break` by an SSML `<break>`, which the phrasing tree reads.
    pub brk: bool,
    /// An SSML `<break>` of medium strength or more ends the utterance here.
    pub utt_end: bool,
    /// A `<phoneme>` token (swift.dll @0x3ae60): its name is the raw `ph` and
    /// these are its words' phone items. The text rules never see it as text.
    pub phones: Option<Vec<Vec<String>>>,
    /// The `nsw` a `<say-as>` put on it (swift.dll @0x3ba50).
    pub nsw: Option<String>,
}

/// One word out of a token, with the phrase break flag `add_break` sets.
#[derive(Debug, Clone)]
pub struct Word {
    pub text: String,
    pub brk: bool,
    /// The punctuation that followed this word, on the last word of a token's
    /// expansion. The phone stream has no use for it, but the part-of-speech
    /// tagger does -- the model is full of features like `W0+1_County_,`.
    pub punc: String,
    /// The index of the token this word came from, within its utterance.
    pub tok: usize,
}

impl Word {
    pub(crate) fn new(t: &str) -> Word {
        Word { text: t.to_string(), brk: false, punc: String::new(), tok: 0 }
    }
}

pub(crate) fn words(list: &[&str]) -> Vec<Word> {
    list.iter().map(|w| Word::new(w)).collect()
}

/// flite's `add_break`: the flag goes on the last word of the list.
pub(crate) fn add_break(mut l: Vec<Word>) -> Vec<Word> {
    if let Some(w) = l.last_mut() {
        w.brk = true;
    }
    l
}

// ---------------------------------------------------------------- tokenizer

pub fn tokenize(text: &str) -> Vec<Tok> {
    tokenize_split(text, &[]).0
}

/// The same with a token boundary forced at each char index in `splits`,
/// ascending, also returning the char index each token starts at. swift.dll's
/// SSML reader tokenizes the text between two tags on its own, so
/// `fo<break/>x` is the tokens "fo" and "x", and whitespace before a tag is
/// the end of one chunk rather than part of the next token's.
pub fn tokenize_split(text: &str, splits: &[usize]) -> (Vec<Tok>, Vec<usize>) {
    let split = |i: usize| splits.binary_search(&i).is_ok();
    let ch: Vec<char> = text.chars().collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i < ch.len() {
        let mut ws = String::new();
        while i < ch.len() && ch[i].is_whitespace() {
            if split(i) {
                ws.clear();
            }
            ws.push(ch[i]);
            i += 1;
        }
        if i >= ch.len() {
            break;
        }
        if split(i) {
            ws.clear();
        }
        starts.push(i);
        let mut pre = String::new();
        while i < ch.len() && PREPUNCT.contains(ch[i]) {
            pre.push(ch[i]);
            i += 1;
        }
        let mut name = String::new();
        while i < ch.len() && !ch[i].is_whitespace() && !(split(i) && !name.is_empty()) {
            name.push(ch[i]);
            i += 1;
        }
        // postpunctuation comes off the end, but never the whole token
        let mut punc = String::new();
        while name.chars().count() > 1 {
            let c = name.chars().next_back().unwrap();
            if !POSTPUNCT.contains(c) {
                break;
            }
            name.pop();
            punc.insert(0, c);
        }
        out.push(Tok { pre, name, punc, ws, ..Tok::default() });
    }
    (out, starts)
}

/// ceplang_en.dll's utterance breaker @0x4e220, which the voice registers
/// with `breakfunc_val` in place of swift.dll's `default_utt_break`. Does the
/// utterance end before `toks[i + 1]`?
///
/// Its tests are C's `strchr` into a set of letters, and `strchr` finds the
/// terminator, so an empty string's "first character" is in every set.
pub fn utt_break(toks: &[Tok], i: usize) -> bool {
    let last = &toks[i];
    let Some(next) = toks.get(i + 1) else { return true };
    let first = |s: &str| s.bytes().next();
    let upper = |c: Option<u8>| c.map_or(true, |c| c.is_ascii_uppercase());
    let lower = |c: Option<u8>| c.map_or(true, |c| c.is_ascii_lowercase());
    let (ws, lname, lpunc, name) = (&next.ws, &last.name, &last.punc, &next.name);

    // a blank line: strchr and strrchr find different newlines
    if ws.find('\n') != ws.rfind('\n') {
        return true;
    }
    if ws.contains('\n') && upper(first(name)) && next.punc == ":" {
        return true;
    }
    if lpunc.contains([':', ';', '?', '!']) {
        return true;
    }
    if lpunc.contains('.') && (ws.len() > 1 || ws == "\n") {
        return true;
    }
    // a full stop before a capital, unless the word looks like an abbreviation:
    // one ending in a capital, a short capitalised one, or a short one with a
    // dot inside
    if lpunc.contains('.') && upper(first(name)) && !upper(lname.bytes().last())
        && (lname.len() >= 4 || (!upper(first(lname)) && !lname.contains('.')))
    {
        return true;
    }
    if lpunc.contains('.') && lower(first(lname)) && lname.len() > 3 {
        return true;
    }
    if name.len() > 42 || lname.len() > 42 {
        return true;
    }
    // a token that is nothing but one of these starts an utterance of its own
    matches!(name.as_str(), ":" | ";" | "?" | "!" | ".")
}

/// Split into utterances at the points `utt_break` marks.
pub fn utterances(toks: Vec<Tok>) -> Vec<Vec<Tok>> {
    let mut out: Vec<Vec<Tok>> = Vec::new();
    let mut cur: Vec<Tok> = Vec::new();
    for i in 0..toks.len() {
        let brk = toks[i].utt_end || utt_break(&toks, i);
        cur.push(toks[i].clone());
        if brk {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ------------------------------------------------------------- predicates

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// `-?[0-9]+(\.[0-9]+)?`, flite's `cst_rx_double`.
fn is_double(s: &str) -> bool {
    let b = s.strip_prefix('-').or_else(|| s.strip_prefix('+')).unwrap_or(s);
    match b.split_once('.') {
        Some((a, c)) => all_digits(a) && all_digits(c),
        None => all_digits(b),
    }
}

// ------------------------------------------------------------- expansions

/// flite's `en_exp_number`; like ceplang_en.dll's fn_040ddc it says nothing
/// for a string that is not all digits.
pub fn exp_number(n: &str) -> Vec<Word> {
    let d: Vec<char> = n.chars().collect();
    let len = d.len();
    if len == 0 || !d.iter().all(|c| c.is_ascii_digit()) {
        return Vec::new();
    }
    if len == 1 {
        return exp_digits(n);
    }
    if len == 2 {
        let (a, b) = (d[0] as usize - 48, d[1] as usize - 48);
        if d[0] == '0' {
            return if d[1] == '0' { Vec::new() } else { words(&[DIGIT2NUM[b]]) };
        }
        if d[1] == '0' {
            return words(&[DIGIT2ENTY[a]]);
        }
        if d[0] == '1' {
            return words(&[DIGIT2TEEN[b]]);
        }
        let mut r = words(&[DIGIT2ENTY[a]]);
        r.extend(exp_digits(&n[1..]));
        return r;
    }
    if len == 3 {
        if d[0] == '0' {
            return exp_number(&n[1..]);
        }
        let mut r = words(&[DIGIT2NUM[d[0] as usize - 48], "hundred"]);
        r.extend(exp_number(&n[1..]));
        return r;
    }
    for (limit, split, name) in [(7usize, 3usize, "thousand"), (10, 6, "million"),
                                 (13, 9, "billion")] {
        if len < limit {
            let cut = len - split;
            let head = exp_number(&n[..cut]);
            if head.is_empty() {
                return exp_number(&n[cut..]);
            }
            let mut r = head;
            r.push(Word::new(name));
            r.extend(exp_number(&n[cut..]));
            return r;
        }
    }
    exp_digits(n)
}

/// ceplang_en.dll fn_04015c: flite's `en_exp_digits`, except that anything
/// not a digit is skipped rather than read as "umpty".
pub fn exp_digits(n: &str) -> Vec<Word> {
    n.chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| Word::new(DIGIT2NUM[c as usize - 48]))
        .collect()
}

/// flite's `en_exp_letters`: "VI" is "v i", and a lone "a" is the letter, `_a`.
pub fn exp_letters(s: &str) -> Vec<Word> {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| {
            let l = c.to_ascii_lowercase();
            if l.is_ascii_digit() {
                Word::new(DIGIT2NUM[l as usize - 48])
            } else if l == 'a' {
                Word::new("_a")
            } else {
                Word::new(&l.to_string())
            }
        })
        .collect()
}

/// ceplang_en.dll fn_041190: the cardinal with its last word made ordinal.
/// Anything but digits and commas says nothing, and a cardinal whose last word
/// has no ordinal -- "million" -- comes back in the reversed order the engine
/// searched it in, so 1000000th is "million one".
pub fn exp_ordinal(raw: &str) -> Vec<Word> {
    if !raw.chars().all(|c| c == ',' || c.is_ascii_digit()) {
        return Vec::new();
    }
    let n: String = raw.chars().filter(|c| *c != ',').collect();
    let mut card = exp_number(&n);
    let Some(last) = card.last().map(|w| w.text.clone()) else { return card };
    let ord = DIGIT2NUM.iter().position(|w| *w == last).map(|i| ORD2NUM[i])
        .or_else(|| DIGIT2TEEN.iter().position(|w| *w == last).map(|i| ORD2TEEN[i]))
        .or_else(|| DIGIT2ENTY.iter().position(|w| *w == last).map(|i| ORD2ENTY[i]))
        .or(match last.as_str() {
            "hundred" => Some("hundredth"),
            "thousand" => Some("thousandth"),
            "billion" => Some("billionth"),
            _ => None,
        });
    match ord {
        Some(o) => {
            let n = card.len();
            card[n - 1] = Word::new(o);
            card
        }
        None => {
            card.reverse();
            card
        }
    }
}

/// ceplang_en.dll fn_041448, the engine's `en_exp_id`: years and ids, read in
/// pairs. It differs from flite's in ways the captures show: 2017 is "two
/// thousand seventeen" (only the second digit need be zero), 500 is "five zero
/// zero" (no three-digit "hundred"), a pair with a leading zero is itself an id
/// ("oh five"), and a four-digit string that starts with a zero is digits.
pub fn exp_id(n: &str) -> Vec<Word> {
    let d = n.as_bytes();
    let len = d.len();
    if !d.iter().all(|c| c.is_ascii_digit()) {
        return Vec::new();
    }
    let lead0 = || if d[0] == b'0' { exp_digits(n) } else { exp_number(n) };
    if len == 4 && d[2] == b'0' && d[3] == b'0' {
        if d[0] == b'0' {
            return exp_digits(n);
        }
        if d[1] == b'0' {
            return exp_number(n);
        }
        let mut r = exp_number(&n[..2]);
        r.push(Word::new("hundred"));
        return r;
    }
    if len == 4 && d[1] == b'0' {
        return lead0();
    }
    if len == 2 {
        return match (d[0], d[1]) {
            (b'0', b'0') => exp_digits(n),
            (b'0', _) => {
                let mut r = words(&["oh"]);
                r.extend(exp_digits(&n[1..]));
                r
            }
            _ => exp_number(n),
        };
    }
    if len < 3 {
        return if len == 0 { exp_number(n) } else { lead0() };
    }
    if len % 2 == 1 {
        let mut r = words(&[DIGIT2NUM[(d[0] - b'0') as usize]]);
        r.extend(exp_id(&n[1..]));
        return r;
    }
    let mut r = if d[0] == b'0' { exp_id(&n[..2]) } else { exp_number(&n[..2]) };
    r.extend(exp_id(&n[2..]));
    r
}

/// ceplang_en.dll fn_041634: flite's `en_exp_real`, keeping only digits and
/// points once the sign and any exponent are read, so commas drop out.
pub fn exp_real(n: &str) -> Vec<Word> {
    if let Some(rest) = n.strip_prefix('-') {
        let mut r = words(&["minus"]);
        r.extend(exp_real(rest));
        return r;
    }
    if let Some(rest) = n.strip_prefix('+') {
        let mut r = words(&["plus"]);
        r.extend(exp_real(rest));
        return r;
    }
    if let Some(at) = n.find('e').or_else(|| n.find('E')) {
        let mut r = exp_real(&n[..at]);
        r.push(Word::new("e"));
        r.extend(exp_real(&n[at + 1..]));
        return r;
    }
    let kept: String = n.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    if let Some((a, b)) = kept.split_once('.') {
        let mut r = exp_number(a);
        r.push(Word::new("point"));
        r.extend(exp_digits(b));
        return r;
    }
    exp_number(&kept)
}

// --------------------------------------------------------------- features

pub fn token_pos_guess(name: &str) -> &'static str {
    let dc = name.to_ascii_lowercase();
    if all_digits(&dc) {
        return "numeric";
    }
    if is_double(&dc) {
        return "number";
    }
    if MONTHS.contains(&dc.as_str()) {
        return "month";
    }
    if DAYS.contains(&dc.as_str()) {
        return "day";
    }
    match dc.as_str() {
        "a" => "a",
        "flight" => "flight",
        "to" => "to",
        _ => "_other_",
    }
}

pub(crate) fn cart_interpret(nodes: &[Node], feats: &[&str], get: &dyn Fn(&str) -> String)
    -> String
{
    let mut i = 0usize;
    loop {
        let n = &nodes[i];
        if n.op == 255 {
            return n.sval.to_string();
        }
        let f = get(feats[n.feat as usize]);
        let yes = match n.op {
            0 => f == n.sval,
            2 => f.parse::<f32>().unwrap_or(0.0) < n.fval,
            3 => f.parse::<f32>().unwrap_or(0.0) > n.fval,
            _ => false,
        };
        i = if yes { i + 1 } else { n.no as usize };
    }
}

// ---------------------------------------------------------------- phrasing

/// The phrasing tree's answer for one word: `true` is a phrase break.
///
/// `last` says the word is the last one its token produced, which is the tree's
/// `R:Token.n.name` test, and `final_word` that nothing follows it at all.
fn phrase_break(tok: &Tok, next: Option<&Tok>, brk: bool, last: bool, final_word: bool)
    -> bool
{
    let get = |f: &str| -> String {
        match f {
            "break" => if brk { "1".into() } else { "0".into() },
            "R:Token.n.name" => if last { "0".into() } else { "x".into() },
            // a name emptied by a rule ("$5 million" takes "million") is not
            // punctuation, though no character in it is alphanumeric
            "R:Token.parent.n.gpos" => match next {
                Some(t) if !t.name.is_empty() && t.name.chars().all(|c| !c.is_alphanumeric()) =>
                    "punc".into(),
                _ => "0".into(),
            },
            "R:Token.parent.break" => if tok.brk { "1".into() } else { "0".into() },
            "R:Token.parent.n.name" => next.map(|t| t.name.clone()).unwrap_or_default(),
            "R:Token.parent.n.prepunctuation" =>
                next.map(|t| t.pre.clone()).unwrap_or_default(),
            "R:Token.parent.punc" => tok.punc.clone(),
            "n.name" => if final_word { "0".into() } else { "x".into() },
            _ => String::new(),
        }
    };
    cart_interpret(&PHRASING_NODES, &PHRASING_FEATS, &get) == "BB"
}

/// One utterance's tokens to words, with the phrase breaks marked.
pub fn utterance_words(toks: &[Tok], known: &dyn Fn(&str) -> bool, digits_style: bool)
    -> Vec<Word>
{
    crate::t2w::begin(toks.len());
    // a <phoneme> token is named with its `ph` and gives one word per `|`
    // group, which the lexicon never looks up
    let per_token: Vec<Vec<Word>> = (0..toks.len())
        .map(|i| match &toks[i].phones {
            Some(ws) if ws.len() == 1 => vec![Word::new(&toks[i].name)],
            Some(ws) => ws.iter().map(|w| Word::new(&w.join(" "))).collect(),
            None => crate::t2w::token_words(toks, i, known, digits_style),
        })
        .collect();
    // phrasing reads the token features as the rules left them
    let edited: Vec<Tok> = toks
        .iter()
        .zip(crate::t2w::end())
        .map(|(t, f)| Tok {
            pre: f.pre.unwrap_or_else(|| t.pre.clone()),
            name: f.name.unwrap_or_else(|| t.name.clone()),
            punc: f.punc.unwrap_or_else(|| t.punc.clone()),
            ..t.clone()
        })
        .collect();
    let toks = &edited[..];
    // swift.dll never makes the directive a word: its break goes onto the word
    // before it, whichever token that came from
    let mut flat: Vec<(usize, Word)> = Vec::new();
    for (i, ws) in per_token.into_iter().enumerate() {
        for w in ws {
            if w.text == crate::t2w::DIRECTIVE {
                if let Some(prev) = flat.last_mut() {
                    prev.1.brk |= w.brk;
                }
            } else {
                flat.push((i, w));
            }
        }
    }
    let total = flat.len();
    let mut out: Vec<Word> = Vec::with_capacity(total);
    for (k, (i, w)) in flat.iter().enumerate() {
        let last = flat.get(k + 1).map_or(true, |(j, _)| j != i);
        let brk = phrase_break(&toks[*i], toks.get(i + 1), w.brk, last, k + 1 == total);
        // the token's punctuation belongs to the last word it expanded into
        let punc = if last { toks[*i].punc.clone() } else { String::new() };
        out.push(Word { text: w.text.clone(), brk, punc, tok: *i });
    }
    out
}
