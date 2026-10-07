//! ceplang_en.dll's `us_tokentowords` (FUN_00148464), rule for rule in the
//! engine's own order.
//!
//! The rule numbers are those of `_fe/t2w_cepstral.md`, which is the spec: it
//! was read instruction by instruction from the DLL, and every quirk kept here
//! (the timezone that reads its hours twice, "dollars" after any currency's
//! "million", "yr" whose plural is "year") is the engine's. The first rule
//! that matches wins, so order matters as much as the rules do.

use crate::carts::{NUMS_FEATS, NUMS_NODES};
use crate::token::{add_break, cart_interpret, exp_digits, exp_id, exp_letters, exp_number,
                   exp_ordinal, exp_real, token_pos_guess, words, Tok, Word};
use crate::token_tables::{ABBREVIATIONS, ACRONYMS, STATES};
use std::cell::RefCell;

/// The pseudo-word rules 43, 44 and 71 emit. swift.dll copies its features --
/// the break -- onto the word before it and never makes it a word.
pub(crate) const DIRECTIVE: &str = "directive_add_feats_to_prev";

/// Item features the rules write on a token while its words are made. `None`
/// is "as the tokenizer left it".
#[derive(Clone, Default)]
pub(crate) struct Feats {
    pub nsw: String,
    pub punc: Option<String>,
    pub pre: Option<String>,
    pub numtype: Option<String>,
    pub name: Option<String>,
    /// Set on the token after a lone direction letter by the say-as address
    /// reader: "N St" makes that St a street.
    pub street: bool,
}

thread_local! {
    static FEATS: RefCell<Vec<Feats>> = RefCell::new(Vec::new());
}

pub(crate) fn begin(n: usize) {
    FEATS.with(|f| *f.borrow_mut() = vec![Feats::default(); n]);
}

pub(crate) fn end() -> Vec<Feats> {
    FEATS.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

fn feat<R>(i: usize, get: impl FnOnce(&mut Feats) -> R) -> Option<R> {
    FEATS.with(|f| f.borrow_mut().get_mut(i).map(get))
}

/// One token's words. `begin` must have been called for the utterance.
pub(crate) fn token_words(toks: &[Tok], i: usize, known: &dyn Fn(&str) -> bool,
                          digits_style: bool) -> Vec<Word>
{
    let cx = Cx { toks, known, digits_style };
    let name = cx.tname(i);
    t2w(&cx, i, &name, 0)
}

struct Cx<'a> {
    toks: &'a [Tok],
    known: &'a dyn Fn(&str) -> bool,
    digits_style: bool,
}

impl Cx<'_> {
    fn has(&self, i: usize, off: isize) -> bool {
        let j = i as isize + off;
        j >= 0 && (j as usize) < self.toks.len()
    }

    fn tname(&self, j: usize) -> String {
        feat(j, |f| f.name.clone()).flatten().unwrap_or_else(|| self.toks[j].name.clone())
    }

    /// `ffeature_string(t, "p.name")` and the like: "0" when there is no token.
    fn name_at(&self, i: usize, off: isize) -> String {
        if self.has(i, off) { self.tname((i as isize + off) as usize) } else { "0".into() }
    }

    fn punc(&self, j: usize) -> String {
        feat(j, |f| f.punc.clone()).flatten().unwrap_or_else(|| self.toks[j].punc.clone())
    }

    fn punc_at(&self, i: usize, off: isize) -> String {
        if self.has(i, off) { self.punc((i as isize + off) as usize) } else { "0".into() }
    }

    fn pre(&self, j: usize) -> String {
        feat(j, |f| f.pre.clone()).flatten().unwrap_or_else(|| self.toks[j].pre.clone())
    }

    fn ws_at(&self, i: usize, off: isize) -> String {
        if self.has(i, off) { self.toks[(i as isize + off) as usize].ws.clone() } else { "0".into() }
    }

    fn nsw(&self, j: usize) -> String {
        feat(j, |f| f.nsw.clone()).filter(|s| !s.is_empty())
            .or_else(|| self.toks.get(j).and_then(|t| t.nsw.clone()))
            .unwrap_or_default()
    }

    fn numtype(&self, j: usize) -> Option<String> {
        feat(j, |f| f.numtype.clone()).flatten()
    }

    fn set_punc(&self, j: usize, s: &str) {
        feat(j, |f| f.punc = Some(s.to_string()));
    }

    fn set_pre(&self, j: usize, s: &str) {
        feat(j, |f| f.pre = Some(s.to_string()));
    }

    fn set_nsw(&self, j: usize, s: &str) {
        feat(j, |f| f.nsw = s.to_string());
    }

    fn set_numtype(&self, j: usize, s: &str) {
        feat(j, |f| f.numtype = Some(s.to_string()));
    }

    fn set_name(&self, j: usize, s: &str) {
        feat(j, |f| f.name = Some(s.to_string()));
    }

    fn known(&self, w: &str) -> bool {
        (self.known)(w)
    }
}

// ---------------------------------------------------------------- patterns

fn digits(s: &str) -> bool {
    crate::rx!("^[0-9]+$").is_match(s)
}

fn double(s: &str) -> bool {
    crate::rx!(r"^-?(([0-9]+\.[0-9]*)|([0-9]+)|(\.[0-9]+))([eE][---+]?[0-9]+)?$").is_match(s)
}

fn number(s: &str) -> bool {
    crate::rx!(r"^[+-]?[0-9]+(,[0-9]{3})*(\.[0-9]+)*$").is_match(s)
}

fn phone7(s: &str) -> bool {
    crate::rx!(r"^[0-9]{3}[.-][0-9]{4}$").is_match(s)
}

fn phone(s: &str) -> bool {
    crate::rx!(r"^(1[.-]?)?(\(?[0-9]{3}[)./-]+)?[0-9]{3}[.-][0-9]{4}$").is_match(s)
}

fn fraction(s: &str) -> bool {
    crate::rx!(r"^[+-]*[1-9][0-9]*/[1-9][0-9]*$").is_match(s)
}

fn mixed(s: &str) -> bool {
    crate::rx!(r"^[+-]*[1-9]+-[1-9][0-9]*/[1-9][0-9]*$").is_match(s)
}

fn hms(s: &str) -> bool {
    crate::rx!(r"^[0-2]?[0-9]:[0-5][0-9]:[0-5][0-9]$").is_match(s)
}

fn ampm(s: &str) -> bool {
    crate::rx!(r"^[APap]\.?[Mm]$").is_match(s)
}

fn tz(s: &str) -> bool {
    crate::rx!(r"^[+-][0-2][0-9][0-5][0-9]$").is_match(s)
}

// The engine's third currency byte is 0xa4: the euro in iso8859-15, but SAPI
// hands the engine cp1252, where 0xa4 is '¤' and '€' is 0x80. Through SAPI
// "€5.50" is "five dot fifty", so '¤' is the character that matches here.
fn money(s: &str) -> bool {
    crate::rx!(r"^[$£¤][,0-9]+(\.[0-9]+)?$").is_match(s)
}

fn illion(s: &str) -> bool {
    crate::rx!(r"^.*illion$").is_match(s)
}

fn year3(s: &str) -> bool {
    crate::rx!(r"^[12]?[0-9]{3}$").is_match(s)
}

fn upper(s: &str) -> bool {
    crate::rx!(r"^[A-Z]+$").is_match(s)
}

fn alpha(s: &str) -> bool {
    crate::rx!(r"^[A-Za-z]+$").is_match(s)
}

fn amount_a(s: &str) -> bool {
    crate::rx!(r"^[$A-Z£¤]+([0-9]+,)*[0-9]+(\.[0-9]+)?$").is_match(s)
}

fn amount_b(s: &str) -> bool {
    crate::rx!(r"^[$A-Z]+([0-9]+,)*[0-9]+(\.[0-9]+)?-?[A-Za-z]+$").is_match(s)
}

fn amount_c(s: &str) -> bool {
    crate::rx!(r"^([0-9]+,)*[0-9]+(\.[0-9]+)?[$A-Z]+$").is_match(s)
}

/// `^(1[0-2]|0[1-9]|[1-9])[/-](3[01]|[12][0-9]|0[1-9]|[1-9])[/-][0-9][0-9]([0-9][0-9])?$`
fn date(s: &str) -> bool {
    crate::rx!(r"^(1[0-2]|0[1-9]|[1-9])[/-](3[01]|[12][0-9]|0[1-9]|[1-9])[/-][0-9][0-9]([0-9][0-9])?$")
        .is_match(s)
}

/// sub_045bf0.
fn is_time(s: &str) -> bool {
    crate::rx!(r"^[01]?[0-9]:[0-5][0-9]$").is_match(s)
        || crate::rx!(r"^[01]?[0-9][.:][0-5][0-9][APap]\.?[Mm]$").is_match(s)
        || ampm(s)
        || crate::rx!(r"^[01]?[0-9]\.[0-5][0-9]$").is_match(s)
        || hms(s)
        || tz(s)
        || crate::rx!(r"^[0-2][0-9]:?[0-5][0-9]$").is_match(s)
}

/// sub_045cb8.
fn is_quantity(s: &str) -> bool {
    double(s) || fraction(s) || mixed(s)
}

/// sub_045b8c.
fn is_amount(s: &str) -> bool {
    amount_a(s) || amount_b(s) || amount_c(s)
}

const WORDCHARS: &str = "abcdefghijklmnopqrstuvwxyz'~ABCDEFGHIJKLMNOPQRSTUVWXYZ\
àáâãäåæçèéêëìíîïðñòóôõöøùúûüýþÿÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÑÒÓÔÕÖØÙÚÛÜÝß";
const LETTERS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\
àáâãäåæçèéêëìíîïðñòóôõöøùúûüýþÿÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÑÒÓÔÕÖØÙÚÛÜÝß";

/// sub_0474f0 (@0xb1070).
fn is_wordchar(c: char) -> bool {
    WORDCHARS.contains(c)
}

/// sub_047558: may the token be split after `ch[k]`? A missing next character
/// is the C string's NUL, which strchr finds in every set.
fn may_split(ch: &[char], k: usize) -> bool {
    let n = ch.get(k + 1).copied();
    let letter = |c: char| LETTERS.contains(c);
    if letter(ch[k]) && n.map_or(true, letter) {
        return false;
    }
    !(ch[k].is_ascii_digit() && n.map_or(true, |c| c.is_ascii_digit()))
}

/// sub_04740c.
fn same_run(s: &str) -> usize {
    let b = s.as_bytes();
    if !b.is_empty() && b.iter().all(|&c| c == b[0]) { b.len() } else { 0 }
}

fn atoi(s: &str) -> i64 {
    let t = s.trim_start();
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let n = t.bytes().take_while(|c| c.is_ascii_digit())
        .fold(0i64, |a, c| a.saturating_mul(10).saturating_add((c - b'0') as i64));
    if neg { -n } else { n }
}

fn cat(mut a: Vec<Word>, b: Vec<Word>) -> Vec<Word> {
    a.extend(b);
    a
}

fn cons(w: &str, rest: Vec<Word>) -> Vec<Word> {
    cat(words(&[w]), rest)
}

// ------------------------------------------------------------------ tables

/// @0xb57dc0, member tests are case-sensitive.
const MONTH_NAMES: [&str; 35] = [
    "Jan", "January", "january", "Feb", "February", "february", "Mar", "March", "march",
    "Apr", "April", "april", "May", "may", "Jun", "June", "june", "Jul", "July", "july",
    "Aug", "August", "august", "Sept", "September", "september", "Oct", "October",
    "october", "Nov", "November", "november", "Dec", "December", "december",
];

/// @0xa3b40: expansion, abbreviation, needs context (rule 62).
const MONTH_ABBREVS: [(&str, &str, bool); 12] = [
    ("january", "jan", true), ("february", "feb", false), ("march", "mar", true),
    ("april", "apr", true), ("may", "may", false), ("june", "jun", false),
    ("july", "jul", false), ("august", "aug", true), ("september", "sep", false),
    ("october", "oct", false), ("november", "nov", false), ("december", "dec", true),
];

/// @0xafce0 (rule 63).
const DAY_ABBREVS: [(&str, &str, bool); 7] = [
    ("sunday", "sun", true), ("monday", "mon", true), ("tuesday", "tue", false),
    ("wednesday", "wed", true), ("thursday", "thu", false), ("friday", "fri", false),
    ("saturday", "sat", true),
];

/// @0xb56300: key, ambiguous, word (rule 30).
const ADDRESS: [(&str, bool, &str); 20] = [
    ("Ave", true, "avenue"), ("Bvd", true, "boulevard"), ("Blvd", false, "boulevard"),
    ("Cir", false, "circle"), ("Ct", true, "court"), ("Ctr", false, "center"),
    ("Ext", true, "extension"), ("Hts", false, "heights"), ("Hwy", false, "highway"),
    ("Ln", false, "lane"), ("Pk", true, "park"), ("Pkw", false, "parkway"),
    ("Pkwy", false, "parkway"), ("Pl", true, "place"), ("Rd", true, "road"),
    ("Rdy", true, "roadway"), ("Sq", true, "square"), ("Ter", false, "terrace"),
    ("Tpk", false, "turnpike"), ("Trl", false, "trail"),
];

/// @0xafda0 (rule 47).
const DIRECTIONS: [(&str, &str); 8] = [
    ("N", "north"), ("S", "south"), ("E", "east"), ("W", "west"), ("NE", "northeast"),
    ("SE", "southeast"), ("SW", "southwest"), ("NW", "northwest"),
];

/// @0xafe30: key, singular, plural (rule 25). "yr" and "wk" really are
/// "year"/"week" in both columns, and "mm" is spelled "milimeter".
const MEASURES: [(&str, &str, &str); 36] = [
    ("in", "inch", "inches"), ("ft", "foot", "feet"), ("yd", "yard", "yards"),
    ("yds", "yard", "yards"), ("yr", "year", "year"), ("yrs", "year", "years"),
    ("wk", "week", "week"), ("wks", "week", "weeks"), ("mi", "mile", "miles"),
    ("km", "kilometer", "kilometers"), ("acs", "acre", "acres"), ("bu", "bushel", "bushels"),
    ("bbl", "barrel", "barrels"), ("gl", "gallon", "gallons"), ("lb", "pound", "pounds"),
    ("oz", "ounce", "ounces"), ("kg", "kilogram", "kilograms"),
    ("kgm", "kilogram", "kilograms"), ("g", "gram", "grams"), ("gm", "gram", "grams"),
    ("mg", "milligram", "milligrams"), ("m", "meter", "meters"),
    ("cm", "centimeter", "centimeters"), ("mm", "milimeter", "milimeters"),
    ("mph", "mile-per-hour", "miles-per-hour"),
    ("km/h", "kilometer-per-hour", "kilometers-per-hour"),
    ("khz", "kilohertz", "kilohertz"), ("kHz", "kilohertz", "kilohertz"),
    ("m", "million", "million"), ("M", "million", "million"), ("mn", "million", "million"),
    ("b", "billion", "billion"), ("bn", "billion", "billion"), ("tn", "trillion", "trillion"),
    ("cu", "cubic", "cubic"), ("sq", "square", "square"),
];

/// @0xb02d0: currency, singular, plural (fn_047dc0).
const CURRENCIES: [(&str, &str, &str); 47] = [
    ("EUR", "euro", "euros"),
    ("$A", "australian-dollar", "australian-dollars"),
    ("$AUD", "australian-dollar", "australian-dollars"),
    ("$C", "canadian-dollar", "canadian-dollars"),
    ("$CAD", "canadian-dollar", "canadian-dollars"),
    ("$CAN", "canadian-dollar", "canadian-dollars"),
    ("$CDN", "canadian-dollar", "canadian-dollars"),
    ("$HK", "hong-kong-dollar", "hong-kong-dollars"),
    ("$HKD", "hong-kong-dollar", "hong-kong-dollars"),
    ("$NZ", "new-zealand-dollar", "new-zealand-dollars"),
    ("$NZD", "new-zealand-dollar", "new-zealand-dollars"),
    ("$SG", "singapore-dollar", "singapore-dollars"),
    ("$SG", "singapore-dollar", "singapore-dollars"),
    ("$SGD", "singapore-dollar", "singapore-dollars"),
    ("$US", "u-s-dollar", "u-s-dollars"),
    ("$USD", "u-s-dollar", "u-s-dollars"),
    ("A$", "australian-dollar", "australian-dollars"),
    ("AU$", "australian-dollar", "australian-dollars"),
    ("AUD", "australian-dollar", "australian-dollars"),
    ("AUD$", "australian-dollar", "australian-dollars"),
    ("C$", "canadian-dollar", "canadian-dollars"),
    ("CAD", "canadian-dollar", "canadian-dollars"),
    ("CAD$", "canadian-dollar", "canadian-dollars"),
    ("CAN", "canadian-dollar", "canadian-dollars"),
    ("CAN$", "canadian-dollar", "canadian-dollars"),
    ("CDN", "canadian-dollar", "canadian-dollars"),
    ("CDN$", "canadian-dollar", "canadian-dollars"),
    ("GBP", "great-britian-pound", "great-britain-pounds"),
    ("GB£", "great-britian-pound", "great-britain-pounds"),
    ("£", "pound", "pounds"),
    ("HK$", "hong-kong-dollar", "hong-kong-dollars"),
    ("HKD", "hong-kong-dollar", "hong-kong-dollars"),
    ("HKD$", "hong-kong-dollar", "hong-kong-dollars"),
    ("JPY", "japanese-yen", "japanese-yen"),
    ("RM", "malaysian-ringgit", "malaysian-ringgit"),
    ("NZ$", "new-zealand-dollar", "new-zealand-dollars"),
    ("NZD", "new-zealand-dollar", "new-zealand-dollars"),
    ("NZD$", "new-zealand-dollar", "new-zealand-dollars"),
    ("SG$", "singapore-dollar", "singapore-dollars"),
    ("SG$", "singapore-dollar", "singapore-dollars"),
    ("SGD", "singapore-dollar", "singapore-dollars"),
    ("SGD$", "singapore-dollar", "singapore-dollars"),
    ("US$", "u-s-dollar", "u-s-dollars"),
    ("US$", "u-s-dollar", "u-s-dollars"),
    ("USD", "u-s-dollar", "u-s-dollars"),
    ("USD$", "u-s-dollar", "u-s-dollars"),
    ("$", "dollar", "dollars"),
];

/// @0xb08d0.
const MULTIPLIERS: [(&str, &str); 6] = [
    ("m", "million"), ("M", "million"), ("mn", "million"), ("b", "billion"),
    ("bn", "billion"), ("tn", "trillion"),
];

/// @0xb57f60.
const ILLIONS: [&str; 9] = ["M", "m", "mn", "million", "b", "bn", "billion", "tn", "trillion"];

/// sub_045d20: a roman numeral after one of these names, or two after one of
/// these titles, is a regnal number. The spellings are the DLL's.
const REGNAL_NAMES: [&str; 28] = [
    "louis", "henry", "charles", "philip", "george", "edward", "pious", "william",
    "richard", "ptolemy", "john", "paul", "peter", "nicholas", "frederick", "james",
    "alfonso", "ivan", "napolean", "leo", "gregory", "catherine", "alexandria",
    "pierre", "elizabeth", "benedict", "mary", "barone",
];
const REGNAL_TITLES: [&str; 16] = [
    "king", "queen", "pope", "duke", "tsar", "emperor", "shah", "ceasar", "duchess",
    "tsarina", "empress", "baron", "baroness", "sultan", "count", "countess",
];

/// sub_0462b4.
const SECTION_WORDS: [&str; 14] = [
    "section", "chapter", "part", "phrase", "verse", "scene", "act", "book", "volume",
    "chap", "war", "apollo", "trek", "fortran",
];

// ------------------------------------------------------------------- rules

fn t2w(cx: &Cx, i: usize, name: &str, depth: usize) -> Vec<Word> {
    // the engine has no limit; this only stops a pathological token
    if name.is_empty() || depth > 256 {
        return Vec::new();
    }
    let lower = name.to_ascii_lowercase();
    // the prologue: a token with an nsw goes to the say-as expander first,
    // and whatever it cannot say falls through to the rules
    let nsw = cx.nsw(i);
    if nsw == "words" {
        return words(&[&lower]);
    }
    if !nsw.is_empty() {
        if let Some(r) = say_as(cx, i, name, &nsw).filter(|r| !r.is_empty()) {
            return r;
        }
    }
    rules_2_20(cx, i, name, &lower, depth)
        .or_else(|| rules_21_50(cx, i, name, &lower, depth))
        .unwrap_or_else(|| rules_51_75(cx, i, name, &lower, depth))
}

fn rules_2_20(cx: &Cx, i: usize, name: &str, lower: &str, depth: usize) -> Option<Vec<Word>> {
    let rec = |s: &str| t2w(cx, i, s, depth + 1);
    let nide = cx.nsw(i) == "nide";

    // 2
    if (name == "a" || name == "A")
        && (!cx.has(i, 1) || name != cx.tname(i) || !cx.punc(i).is_empty())
    {
        return Some(words(&["_a"]));
    }
    // 3
    if let Some(r) = abbreviation(cx, i, name) {
        return Some(r);
    }
    // 4
    if crate::rx!(r"^[A-Za-z](\.[A-Za-z])+\.?$").is_match(name) {
        return Some(exp_letters(&name.replace('.', "")));
    }
    // 5: cst_rx_commaint. The captures need the comma: "911" is not one.
    if crate::rx!(r"^[0-9]{1,3}(,[0-9]{3})+(\.[0-9]+)?$").is_match(name) {
        return Some(exp_real(name));
    }
    // 6
    if phone7(name) {
        let at = name.find('-').or_else(|| name.find('.')).unwrap();
        return Some(cat(add_break(exp_digits(&name[..at])), exp_digits(&name[at + 1..])));
    }
    // 7, 8: 800 and 900 numbers, with and without the leading 1
    let one_phone = |s: &str| {
        crate::rx!(r"^1[.-]?\(?[0-9]{3}[)./-]+[0-9]{3}[.-][0-9]{4}$").is_match(s)
    };
    if crate::rx!(r"^(1[.-]?)?[89]00[./][0-9]{3}[.-][0-9]{4}$").is_match(name) {
        if one_phone(name) {
            let sep = matches!(name.as_bytes()[1], b'-' | b'.');
            let (sub, rest) = if sep { (&name[2..5], &name[6..]) } else { (&name[1..4], &name[5..]) };
            let head = cat(words(&["one"]), add_break(exp_number(sub)));
            return Some(cat(head, rec(rest)));
        }
        let rest = rec(&name[4..]);
        return Some(cat(add_break(exp_number(&name[..3])), rest));
    }
    if one_phone(name) {
        let rest = if matches!(name.as_bytes()[1], b'.' | b'-') { &name[2..] } else { &name[1..] };
        let r = rec(rest);
        return Some(cat(add_break(words(&["one"])), r));
    }
    // 9
    if crate::rx!(r"^[0-9]{3}[)./-]+[0-9]{3}[.-][0-9]{4}$").is_match(name) {
        let b = name.as_bytes();
        let rest = if b[3] == b')' && matches!(b[4], b'-' | b'.' | b'/') {
            &name[5..]
        } else if (b[3] == b')' && phone7(&name[4..])) || matches!(b[3], b'-' | b'.' | b'/') {
            &name[4..]
        } else {
            ""
        };
        let area = &name[..3];
        let r = rec(rest);
        let a = if area == "800" || area == "900" { exp_number(area) } else { exp_digits(area) };
        return Some(cat(add_break(a), r));
    }
    // 10, 11: extensions after a phone number
    if crate::rx!(r"^e?xt?.?[0-9]+$").is_match(name) && cx.has(i, -1)
        && phone(&cx.name_at(i, -1))
    {
        let b = name.as_bytes();
        let mut k = 0;
        while b.len() > 1 && k < b.len() - 1 && !b[k].is_ascii_digit() {
            k += 1;
        }
        return Some(cons("extension", exp_id(&name[k..])));
    }
    if matches!(lower, "ext" | "xt" | "ex") && cx.has(i, -1) && phone(&cx.name_at(i, -1))
        && cx.has(i, 1) && number(&cx.name_at(i, 1))
    {
        return Some(words(&["extension"]));
    }
    // 12: one group of a phone number written with spaces. The engine marks the
    // group's end with a comma and drops the "(" of an area code.
    let d3 = |s: &str| crate::rx!("^[0-9]{3}$").is_match(s);
    let d4 = |s: &str| crate::rx!("^[0-9]{4}$").is_match(s);
    let (p, n) = (cx.name_at(i, -1), cx.name_at(i, 1));
    let (pp, nn) = (cx.name_at(i, -2), cx.name_at(i, 2));
    let group = (d3(name)
        && (((!cx.has(i, -1) || !digits(&p)) && d3(&n) && d4(&nn))
            || phone7(&n)
            || (!(digits(&pp) && cx.has(i, -2)) && d3(&p) && d4(&n))))
        || (d4(name) && !(digits(&n) && cx.has(i, 1)) && d3(&p) && d3(&pp));
    if group {
        if cx.punc(i).is_empty() {
            cx.set_punc(i, ",");
        }
        if cx.pre(i) == "(" {
            cx.set_pre(i, "");
        }
        return Some(add_break(exp_digits(name)));
    }
    // 13: 12:40PM
    if crate::rx!(r"^[01]?[0-9][.:][0-5][0-9][APap]\.?[Mm]$").is_match(name) && !nide {
        let cut = name.find(':').or_else(|| name.find('.')).unwrap();
        let (hh, mmx) = (&name[..cut], &name[cut + 1..]);
        let q = ['a', 'A', 'P', 'p'].iter().find_map(|&c| mmx.find(c)).unwrap();
        let mut r = exp_number(hh);
        minutes(cx, i, &mut r, &mmx[..q]);
        r.extend(exp_letters(&format!("{}m", &mmx[q..q + 1])));
        return Some(r);
    }
    // 14
    let hm = crate::rx!(r"^[01]?[0-9]:[0-5][0-9]$").is_match(name)
        || (crate::rx!(r"^[01]?[0-9]\.[0-5][0-9]$").is_match(name) && ampm(&n));
    if hm && !nide {
        let cut = name.find(':').or_else(|| name.find('.')).unwrap();
        let mut r = exp_number(&name[..cut]);
        minutes(cx, i, &mut r, &name[cut + 1..]);
        return Some(r);
    }
    // 15
    if hms(name) && (crate::rx!("^[A-Z]{3}$").is_match(&n) || tz(&n) || ampm(&n)) {
        return Some(time_secs(cx, i, name, depth));
    }
    // 16
    if ampm(name) && is_time(&p) {
        return Some(exp_letters(name));
    }
    // 17
    if tz(name) && cx.has(i, -1) && hms(&cx.tname(i - 1)) {
        return Some(tz_offset(name));
    }
    // 18: only ":1" is a ratio; 3:2 goes to the splitter
    if crate::rx!(r"^[0-9]+:1$").is_match(name) {
        let (a, b) = name.split_once(':').unwrap();
        return Some(cat(cons_after(exp_number(a), "to"), exp_number(b)));
    }
    // 19
    if crate::rx!(r"^[0-9]+(-[0-9]+)+-[0-9]+$").is_match(name) && !date(name) {
        return Some(name.split('-').flat_map(|g| add_break(exp_digits(g))).collect());
    }
    // 20
    if digits(name) {
        return Some(digit_string(cx, i, name, depth));
    }
    None
}

fn cons_after(mut a: Vec<Word>, w: &str) -> Vec<Word> {
    a.push(Word::new(w));
    a
}

/// Rules 13 and 14 after the hour: the minutes, or "o'clock" on the hour after
/// "it's".
fn minutes(cx: &Cx, i: usize, r: &mut Vec<Word>, mm: &str) {
    if mm != "00" {
        r.extend(exp_id(mm));
    } else if cx.name_at(i, -1).to_ascii_lowercase() == "it's"
        && cx.name_at(i, 1).to_ascii_lowercase() != "o'clock"
    {
        r.push(Word::new("o'clock"));
    }
}

/// fn_046c70.
fn time_secs(cx: &Cx, i: usize, name: &str, depth: usize) -> Vec<Word> {
    let (hm, ss) = match name.len() {
        8 => (&name[..5], &name[6..]),
        7 => (&name[..4], &name[5..]),
        _ => return Vec::new(),
    };
    let mut r = t2w(cx, i, hm, depth + 1);
    if ss != "00" {
        r.push(Word::new("and"));
        r.extend(exp_number(ss));
        r.push(Word::new(if ss == "01" { "second" } else { "seconds" }));
    }
    r
}

/// fn_046df0. The minutes are read from the HOURS string -- the engine's own
/// bug, so +0530 is "plus zero five five".
fn tz_offset(name: &str) -> Vec<Word> {
    let (mut r, rest) = match name.as_bytes()[0] {
        b'-' => (words(&["minus"]), &name[1..]),
        b'+' => (words(&["plus"]), &name[1..]),
        _ => (Vec::new(), name),
    };
    let (hh, mm) = (&rest[..2], &rest[2..]);
    r.extend(if hh.starts_with('0') { exp_digits(hh) } else { exp_number(hh) });
    if mm == "00" {
        r.push(Word::new("hundred"));
    } else if mm.starts_with('0') {
        r.extend(exp_digits(hh));
    } else {
        r.extend(exp_number(hh));
    }
    r
}

/// Rule 20: a bare digit string.
fn digit_string(cx: &Cx, i: usize, name: &str, depth: usize) -> Vec<Word> {
    if cx.nsw(i) == "nide" {
        return exp_id(name);
    }
    let cls = nums_cart(cx, i, name);
    let b = name.as_bytes();
    if b.len() == 2 && b[0] == b'0' && cx.pre(i) == "'" {
        return exp_id(name);
    }
    if b.len() > 4 && b.len() < 9 && name.ends_with("000") {
        return exp_number(name);
    }
    // `DigitStyle::Digits` is ours, not the engine's: it forces a bare two- or
    // three-digit token to read as a code, which is what alerting text wants.
    // Top level only, so "100%" and "$120" keep their quantity reading.
    if cx.digits_style && depth == 0 && (2..=3).contains(&b.len()) {
        return exp_digits(name);
    }
    if cls == "ordinal" {
        let month = |j: usize| MONTH_NAMES.contains(&cx.tname(j).as_str());
        if cx.has(i, 1) && month(i + 1) && (!cx.has(i, -1) || !month(i - 1))
            && !cx.punc(i).contains(',')
        {
            return cons("the", cons_after(exp_ordinal(name), "of"));
        }
        return exp_ordinal(name);
    }
    if cls == "digits" || b[0] == b'0' {
        return exp_digits(name);
    }
    if cls == "year" {
        if cx.numtype(i).is_none() {
            cx.set_numtype(i, "year");
        }
        let r = exp_id(name);
        return if hms(&cx.name_at(i, 1)) { add_break(r) } else { r };
    }
    if cx.name_at(i, -1).to_ascii_lowercase() == "interstate" {
        exp_id(name)
    } else {
        exp_number(name)
    }
}

/// flite's nums_cart, evaluated as the engine does with the token's name set to
/// the piece being read.
fn nums_cart(cx: &Cx, i: usize, name: &str) -> String {
    let pos = |k: isize| -> String {
        if cx.has(i, k) {
            token_pos_guess(&cx.tname((i as isize + k) as usize)).to_string()
        } else {
            "0".to_string()
        }
    };
    let get = |f: &str| -> String {
        match f {
            "num_digits" => name.chars().count().to_string(),
            "month_range" => {
                let v: i64 = name.parse().unwrap_or(0);
                if v > 0 && v < 32 { "1".into() } else { "0".into() }
            }
            "name" => name.to_string(),
            "p.token_pos_guess" => pos(-1),
            "n.token_pos_guess" => pos(1),
            "pp.token_pos_guess" => pos(-2),
            "nn.token_pos_guess" => pos(2),
            _ => String::new(),
        }
    };
    cart_interpret(&NUMS_NODES, &NUMS_FEATS, &get)
}

fn rules_21_50(cx: &Cx, i: usize, name: &str, lower: &str, depth: usize) -> Option<Vec<Word>> {
    let rec = |s: &str| t2w(cx, i, s, depth + 1);
    let nide = cx.nsw(i) == "nide";
    let (p, n) = (cx.name_at(i, -1), cx.name_at(i, 1));

    // 21: "zzzzz" is "five z 's"
    let run = same_run(lower);
    if run > 4 {
        return Some(cat(rec(&run.to_string()), words(&[&lower[..1], "'s"])));
    }
    // 22
    if crate::rx!("^(I{1,3}|IV|VI{0,3}|IX|X[IVX]*)$").is_match(name) && same_run(name) < 4 {
        return Some(roman(cx, i, name));
    }
    // 23
    if lower == "po" && n.to_ascii_lowercase() == "box" {
        return Some(exp_letters(name));
    }
    // 24
    if lower == "ft" {
        if cx.has(i, -1) && is_quantity(&p) && cx.punc_at(i, -1) != "," && name != "Ft" {
            return Some(words(&[if p == "1" || fraction(&p) { "foot" } else { "feet" }]));
        }
        return Some(words(&["ft"]));
    }
    // 25
    if let Some(r) = measure(cx, i, name, lower, depth).filter(|r| !r.is_empty()) {
        return Some(r);
    }
    // 26, 27: I-95, US-66
    if let Some(d) = lower.strip_prefix("i-").filter(|d| digits(d)) {
        return Some(cons("i", exp_id(d)));
    }
    if let Some(d) = lower.strip_prefix("us-").filter(|d| digits(d)) {
        return Some(cons("u", cons("s", exp_id(d))));
    }
    // 28
    if lower == "ste" {
        let next = cx.has(i, 1).then(|| cx.tname(i + 1));
        let suite = next.is_some_and(|n| {
            n.chars().next().map_or(true, |c| "#1234567890".contains(c))
                || (n.len() == 1 && n.as_bytes()[0].is_ascii_uppercase())
        });
        return Some(words(&[if suite { "suite" } else { "saint" }]));
    }
    // 29
    if crate::rx!("^([Dd][Rr]|[Ss][Tt])$").is_match(name) && !nide {
        return Some(words(&[street_or_title(cx, i, name)]));
    }
    // 30
    if let Some(r) = address(cx, i, name) {
        return Some(r);
    }
    // 31: acronyms the engine spells even where the state rule would expand
    if ACRONYMS.contains(&name) && !upper(&p) && !upper(&n) {
        return Some(exp_letters(name));
    }
    // 32
    const TITLES: [&str; 13] = ["mr", "ms", "mrs", "etc", "ltd", "dept", "vs", "thur", "thurs",
                                "weds", "tues", "vol", "yld"];
    if TITLES.contains(&lower) {
        let w = match (name, lower) {
            ("Mr", _) => "mr",
            ("Mrs", _) => "mrs",
            ("Ms", _) => "mizz",
            (_, "ltd") => "limited",
            (_, "dept") => "department",
            (_, "vs") => "versus",
            (_, "thur" | "thurs") => "thursday",
            (_, "weds") => "wednesday",
            (_, "tues") => "tuesday",
            (_, "vol") => "volume",
            (_, "yld") => "yield",
            _ => lower,
        };
        return Some(words(&[w]));
    }
    // 33
    if lower == "20/20" {
        return Some(words(&["twenty", "twenty"]));
    }
    if lower == "24/7" {
        return Some(words(&["twenty-four", "seven"]));
    }
    // 34
    let colon = matches!(cx.punc(i).as_str(), ":" | ".:");
    if lower == "tel" && colon {
        return Some(words(&["telephone"]));
    }
    if lower == "ph" && colon {
        return Some(words(&["phone"]));
    }
    // 35: homographs by neighbour
    let lp = p.to_ascii_lowercase();
    if lower == "read" {
        const BEFORE: [&str; 13] = ["be", "was", "were", "are", "been", "have", "had", "i",
                                    "he", "she", "they", "we", "you"];
        return Some(words(&[if BEFORE.contains(&lp.as_str()) { "red" } else { "reed" }]));
    }
    if lower == "does" {
        let duzz = ["he", "she", "it", "what", "why", "where"].contains(&lp.as_str())
            || n.to_ascii_lowercase() == "not";
        return Some(words(&[if duzz { "duzz" } else { "does" }]));
    }
    if lower == "supposed" {
        let d = ["a", "the"].contains(&lp.as_str());
        return Some(words(&[if d { "suppozid" } else { "suppost" }]));
    }
    // 36
    if name == "EST" && (ampm(&p) || is_time(&p)) {
        return Some(exp_letters(name));
    }
    // 37
    if name == "AD" {
        let year = (cx.has(i, 1) && year3(&cx.tname(i + 1)))
            || (cx.has(i, -1) && year3(&cx.tname(i - 1)))
            || (cx.has(i, -1) && cx.numtype(i - 1).as_deref() == Some("year"));
        return Some(if year { exp_letters(lower) } else { words(&["ad"]) });
    }
    // 38: emoticons; a bare ":" says nothing
    if (name == ":-" || name == ":") && !nide {
        return Some(match cx.punc(i).chars().next() {
            Some(')') => add_break(words(&["smily"])),
            Some('(') => add_break(words(&["sigh"])),
            _ if name == ":-" => add_break(words(&["colon", "dash"])),
            _ => Vec::new(),
        });
    }
    // 39
    if name.len() == 1 && name == cx.tname(i) && ".,?!\"';:".contains(name) {
        return Some(Vec::new());
    }
    // 40
    if name == "/" && cx.punc(i) == "." && name == cx.tname(i) {
        return Some(words(&["slash", "dot"]));
    }
    // 41, 42
    if let Some(d) = name.strip_prefix('#').filter(|d| digits(d)) {
        return Some(cons("number", rec(d)));
    }
    if crate::rx!("^[0-9]+#$").is_match(name) {
        return Some(cons_after(exp_number(&name[..name.len() - 1]), "pound"));
    }
    // 43
    if name == "-" {
        if cx.has(i, -1) || !cx.has(i, 1) {
            return Some(add_break(words(&[DIRECTIVE])));
        }
        let n = cx.tname(i + 1);
        if number(&n) || fraction(&n) || n.ends_with('%') {
            return Some(words(&["minus"]));
        }
        return Some(add_break(words(&[DIRECTIVE])));
    }
    // 44: a '.' split off a URL or an address is "dot"
    if name == "." {
        if cx.punc(i) == ".." {
            return Some(add_break(words(&[DIRECTIVE])));
        }
        return Some(if nide { words(&["dot"]) } else { Vec::new() });
    }
    // 45
    if (name == "," || name == ";") && !nide {
        return Some(Vec::new());
    }
    // 46
    let lb = lower.as_bytes();
    if name.len() > 1 && b"<>".contains(&lb[0]) && b"1234567890-+$.".contains(&lb[1]) {
        let tail = rec(&name[1..]);
        return Some(cat(rec(&lower[..1]), tail));
    }
    // 47
    if !nide {
        if let Some(r) = direction(cx, i, name) {
            return Some(r);
        }
    }
    // 48: an initial, when one space and another capital follow
    if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() && cx.ws_at(i, 1) == " "
        && n.as_bytes().first().is_some_and(|c| c.is_ascii_uppercase())
    {
        return Some(words(&[if lower == "a" { "_a" } else { lower }]));
    }
    // 49: "-5" is "minus five" here
    if double(name) && !nide {
        return Some(exp_real(name));
    }
    // 50
    if crate::rx!("^[1-9][,0-9]*(th|TH|st|ST|nd|ND|rd|RD)$").is_match(name) {
        return Some(exp_ordinal(&name[..name.len() - 2]));
    }
    None
}

/// Rule 22 after the pattern.
fn roman(cx: &Cx, i: usize, name: &str) -> Vec<Word> {
    let p = cx.name_at(i, -1);
    if p == "OS" && name == "X" {
        return words(&["ten"]);
    }
    if !cx.punc_at(i, -1).is_empty() {
        return exp_letters(name);
    }
    let v = exp_roman(name).to_string();
    let lp = p.to_ascii_lowercase();
    let lpp = cx.name_at(i, -2).to_ascii_lowercase();
    if REGNAL_NAMES.contains(&lp.as_str()) || REGNAL_TITLES.contains(&lpp.as_str()) {
        return cons("the", exp_ordinal(&v));
    }
    if SECTION_WORDS.contains(&lp.as_str()) {
        return exp_number(&v);
    }
    exp_letters(name)
}

/// swift.dll `cst_exp_roman`: a left-to-right sum in which only IV and IX
/// subtract.
fn exp_roman(s: &str) -> u32 {
    let b = s.as_bytes();
    let (mut n, mut k) = (0, 0);
    while k < b.len() {
        match (b[k], b.get(k + 1)) {
            (b'X', _) => n += 10,
            (b'V', _) => n += 5,
            (b'I', Some(b'V')) => { n += 4; k += 1; }
            (b'I', Some(b'X')) => { n += 9; k += 1; }
            (b'I', _) => n += 1,
            _ => {}
        }
        k += 1;
    }
    n
}

/// Rule 29. strchr finds a NUL in every set, which is what the "0" of a
/// missing neighbour's first character never is.
fn street_or_title(cx: &Cx, i: usize, name: &str) -> &'static str {
    let (street, title) = if name.starts_with(['s', 'S']) {
        ("street", "saint")
    } else {
        ("drive", "doctor")
    };
    if !cx.has(i, 1) || cx.punc(i).contains(',') {
        return street;
    }
    let p0 = cx.name_at(i, -1).chars().next();
    let n0 = cx.name_at(i, 1).chars().next();
    let upper_or_nul = |c: Option<char>| c.map_or(true, |c| c.is_ascii_uppercase());
    let is = |c: Option<char>, f: fn(&char) -> bool| c.as_ref().is_some_and(f);
    if is(p0, char::is_ascii_uppercase) && !upper_or_nul(n0) {
        return street;
    }
    if is(p0, char::is_ascii_digit) && is(n0, char::is_ascii_lowercase) {
        return street;
    }
    if !upper_or_nul(p0) && is(n0, char::is_ascii_uppercase) {
        return title;
    }
    if cx.ws_at(i, 1) == " " { title } else { street }
}

/// fn_040394.
fn address(cx: &Cx, i: usize, name: &str) -> Option<Vec<Word>> {
    for &(key, ambiguous, word) in &ADDRESS {
        if key != name {
            continue;
        }
        let expand = !ambiguous || cx.nsw(i) == "address" || {
            let p = cx.name_at(i, -1);
            p.chars().next().map_or(true, |c| "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456789".contains(c))
                && p.len() > 2
                && name == cx.tname(i)
                && (!cx.has(i, 1) || matches!(cx.punc(i).as_str(), "." | ".," | ","))
        };
        if expand {
            return Some(words(&[word]));
        }
    }
    None
}

/// fn_046684: "5 E Main".
fn direction(cx: &Cx, i: usize, name: &str) -> Option<Vec<Word>> {
    if !cx.has(i, -1) || !crate::rx!("^-?[0-9]+$").is_match(&cx.tname(i - 1)) || !cx.has(i, 1) {
        return None;
    }
    let n0 = cx.tname(i + 1).chars().next();
    if !n0.map_or(true, |c| "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456789".contains(c)) {
        return None;
    }
    DIRECTIONS.iter().find(|d| d.0 == name).map(|d| words(&[d.1]))
}

/// sub_0475f0: units of measure, attached ("5km") or on their own after a
/// number ("5 km"). "in" is only inches when the context says so.
fn measure(cx: &Cx, i: usize, name: &str, lower: &str, depth: usize) -> Option<Vec<Word>> {
    if is_amount(lower) {
        return None;
    }
    let rec = |s: &str| t2w(cx, i, s, depth + 1);
    let ch: Vec<char> = name.chars().collect();
    let mut num: i64 = 0;
    let mut cut = 0usize;
    let mut numpart = lower.to_string();
    if crate::rx!("[.0-9]+-*[a-zà-öø-ÿ]+").is_match(lower) {
        let mut k = 0;
        while k < ch.len() {
            if is_wordchar(ch[k]) {
                let prefix: String = ch[..k].iter().collect();
                num = if prefix.contains('/') {
                    1
                } else if prefix.contains('.') {
                    2
                } else {
                    atoi(&prefix)
                };
                break;
            }
            if ch[k] == '-' {
                num = atoi(&ch[..k].iter().collect::<String>());
                k += 1;
                break;
            }
            k += 1;
        }
        cut = k;
        if cut > 0 {
            numpart = ch[..cut].iter().collect();
            if numpart.ends_with('-') {
                numpart.pop();
            }
        }
    }
    let unit: String = if num != 0 { ch[cut..].iter().collect() } else { name.to_string() };
    let &(key, one, many) = MEASURES.iter().find(|m| m.0 == unit)?;
    if key == "in" && !lower.contains('-') {
        let next_ok = cx.has(i, 1)
            && ["high", "tall", "wide", "deep", "long", "of"].contains(&cx.tname(i + 1).as_str());
        if !next_ok {
            let pb = cx.punc(i).into_bytes();
            if !cx.has(i, 1) && pb.first() == Some(&b'.') && pb.get(1) != Some(&b'.') {
                return None;
            }
            let area = cx.has(i, -1)
                && ["sq", "square", "cu", "cubic"].contains(&cx.tname(i - 1).as_str());
            if !area {
                if cx.has(i, 1) {
                    return None;
                }
                let article = cx.has(i, -1) && matches!(cx.tname(i - 1).as_str(), "an" | "a");
                let of = article && cx.has(i, -2) && cx.tname(i - 2) == "of";
                if !of && !(cx.has(i, -1) && number(&cx.tname(i - 1))) {
                    return None;
                }
            }
        }
    }
    if num != 0 {
        let word = if fraction(&numpart) {
            cx.set_numtype(i, "fraction");
            one
        } else if num == 1 {
            one
        } else {
            many
        };
        let u = rec(word);
        return Some(cat(rec(&numpart), u));
    }
    if !cx.has(i, -1) {
        return None;
    }
    let p = cx.tname(i - 1);
    if number(&p) || ["sq", "square", "cu", "cubic"].contains(&p.as_str()) {
        return Some(rec(if p == "1" { one } else { many }));
    }
    if fraction(&p) {
        return Some(rec(one));
    }
    if mixed(&p) {
        return Some(rec(many));
    }
    if matches!(p.as_str(), "an" | "a") && cx.has(i, -2) && cx.tname(i - 2) == "of" {
        return Some(rec(one));
    }
    None
}

fn rules_51_75(cx: &Cx, i: usize, name: &str, lower: &str, depth: usize) -> Vec<Word> {
    let rec = |s: &str| t2w(cx, i, s, depth + 1);
    let nide = cx.nsw(i) == "nide";
    let (p, n) = (cx.name_at(i, -1), cx.name_at(i, 1));
    let chars = name.chars().count();

    // 51
    if (ILLIONS.contains(&n.as_str()) || n == "thousand") && money(name) {
        return amount(cx, i, name, depth);
    }
    // 52: the unit is read from this token's first character, not the money
    // token's, so it is always "dollars"
    if illion(name) && money(&p) {
        let unit = match name.chars().next() {
            Some('£') => "pounds",
            Some('¤') => "euros",
            _ => "dollars",
        };
        return words(&[name, unit]);
    }
    // 53
    if (money(name) || crate::rx!(r"^[$£¤]\.[0-9]+$").is_match(name)) && !nide {
        return money_words(cx, i, name);
    }
    // 54
    if let Some(s) = name.strip_suffix('%') {
        if fraction(s) && cx.numtype(i).is_none() {
            cx.set_numtype(i, "fraction");
            return cat(rec(s), words(&["of", "a", "percent"]));
        }
        return cons_after(rec(s), "percent");
    }
    // 55: 1990s
    if crate::rx!("^[0-9]+s$").is_match(name) {
        return cons_after(exp_id(&name[..name.len() - 1]), "'s");
    }
    // 56
    if name == "No" && digits(&n) && cx.has(i, 1) && cx.punc(i) == "." {
        return words(&["number"]);
    }
    // 57: a possessive's apostrophe is not a closing quote
    if name.ends_with(['s', 'S']) {
        let punc = cx.punc(i);
        if punc.starts_with('\'') && !punc[1..].starts_with('\'')
            && (0..=i).all(|j| cx.pre(j) != "'")
        {
            cx.set_punc(i, &punc[1..]);
            return words(&[lower]);
        }
    }
    // 59
    if let Some(at) = lower.find("://") {
        let rest = rec(&lower[at + 3..]);
        return cat(cat(exp_letters(&lower[..at]), words(&["colon", "slash", "slash"])), rest);
    }
    // 60
    if lower.len() > 3 && lower.ends_with("/hr") {
        return cat(rec(&lower[..lower.len() - 3]), words(&["per", "hour"]));
    }
    // 61, 62, 63
    if !nide {
        if let Some(r) = state_name(cx, i, name) {
            return r;
        }
        if let Some(r) = month_abbrev(cx, i, lower) {
            return r;
        }
        if let Some(r) = day_abbrev(cx, i, lower) {
            return r;
        }
    }
    // 64
    if name == "©" {
        return words(&["copyright"]);
    }
    if chars > 1 && name.starts_with('©') {
        return cons("copyright", rec(&name['©'.len_utf8()..]));
    }
    if name == "&#8453" {
        return words(&["in", "care", "of"]);
    }
    if name == "&#8364" {
        return words(&["euro"]);
    }
    // 65: a UK postcode's first half
    if crate::rx!("^[A-Z]{2}[1-9][0-9]?$").is_match(name)
        && crate::rx!("^[1-9][A-Z]{2}$").is_match(&n) && !nide
    {
        cx.set_nsw(i, "nide");
        return add_break(rec(name));
    }
    // 66: a split-off piece under three characters is spelled -- which is why
    // "to" in a URL path is "t o"
    if nide && chars < 3 {
        return exp_letters(name);
    }
    if cx.known(lower) || cx.known(name) {
        return words(&[lower]);
    }
    if is_amount(name) {
        return amount(cx, i, name, depth);
    }
    if let Some(rest) = name.strip_prefix('-').filter(|r| is_amount(r)) {
        return cons("minus", amount(cx, i, rest, depth));
    }
    if let Some(at) = name.rfind('\'') {
        let lp = name[at..].to_ascii_lowercase();
        if ["'s", "'ll", "'ve", "'d"].contains(&lp.as_str()) {
            let head = name[..at].strip_suffix('.').unwrap_or(&name[..at]);
            return cons_after(rec(head), &lp);
        }
        if &name[at..] == "'tve" {
            return cons_after(rec(&name[..at + 2]), "'ve");
        }
        if digits(&name[at + 1..]) {
            cx.set_nsw(i, "nide");
            let num = exp_number(&lp[1..]);
            return cat(rec(&name[..at]), num);
        }
        return rec(&format!("{}{}", &name[..at], &name[at + 1..]));
    }
    // 68
    if date(name) && name == cx.tname(i) {
        return expand_date(name).unwrap_or_default();
    }
    // 69
    if mixed(name) {
        let d = name.find('-').unwrap();
        cx.set_numtype(i, "fraction");
        let frac = cons("and", rec(&name[d + 1..]));
        return cat(exp_number(&name[..d]), frac);
    }
    // 70
    if fraction(name)
        && (cx.numtype(i).as_deref() == Some("fraction") || name == cx.tname(i))
    {
        if let Some(r) = lower.strip_prefix('-') {
            return cons("minus", rec(r));
        }
        if let Some(r) = lower.strip_prefix('+') {
            return cons("plus", rec(r));
        }
        let (num, den) = name.split_once('/').unwrap();
        let r = if num == "1" && den == "2" {
            words(&["a", "half"])
        } else if name == "9/11" {
            words(&["nine", "eleven"])
        } else if atoi(num) < atoi(den) {
            let mut r = cat(exp_number(num), exp_ordinal(den));
            if atoi(num) > 1 {
                r.push(Word::new("'s"));
            }
            r
        } else {
            cat(cons_after(exp_number(num), "slash"), exp_number(den))
        };
        return if digits(&p) && cx.has(i, -1) { cons("and", r) } else { r };
    }
    // 71
    if name == "--" {
        return words(&["--"]);
    }
    if crate::rx!(r"^[*-]+$").is_match(name) {
        return add_break(words(&[DIRECTIVE]));
    }
    if let Some(d) = name.find('-') {
        if name[d + 1..].starts_with('-') && !name[d + 2..].starts_with('-') {
            let tail = rec(&name[d + 2..]);
            return cat(add_break(rec(&name[..d])), tail);
        }
    }
    if let Some(d) = name.find('.') {
        if name[d + 1..].starts_with("..") && !name[d + 3..].starts_with('.') {
            let tail = rec(&name[d + 3..]);
            return cat(add_break(rec(&name[..d])), tail);
        }
    }
    if chars > 2 {
        if let Some(d) = name.find('-') {
            let (head, tail) = (&name[..d], &name[d + 1..]);
            if crate::rx!("^[0-9]+-[A-Za-z]+-[0-9]+$").is_match(name) {
                return day_month_year(cx, i, name, depth);
            }
            if crate::rx!("^[0-9]{5}-[0-9]{4}$").is_match(name) {
                return cat(exp_digits(head), exp_digits(tail));
            }
            if digits(head) && digits(tail) {
                // the token's name is each piece while it is read, and the
                // whole piece afterwards
                cx.set_name(i, tail);
                let tw = rec(tail);
                cx.set_name(i, head);
                let w = if cx.nsw(i) == "address" { "dash" } else { "to" };
                let hw = rec(head);
                cx.set_name(i, name);
                return cat(cons_after(hw, w), tw);
            }
            let tw = if digits(tail) { exp_id(tail) } else { rec(tail) };
            return cat(rec(head), tw);
        }
        if let Some(c) = name.find(',') {
            let tail = rec(&name[c + 1..]);
            return cat(add_break(rec(&name[..c])), tail);
        }
    }
    // 75: the tail
    if chars > 1 && !alpha(name) && !name.chars().all(is_wordchar) {
        let ch: Vec<char> = name.chars().collect();
        let k = (0..ch.len()).find(|&k| may_split(&ch, k)).unwrap_or(ch.len());
        let head: String = ch[..(k + 1).min(ch.len())].iter().collect();
        let tail: String = ch[(k + 1).min(ch.len())..].iter().collect();
        cx.set_nsw(i, "nide");
        let tw = rec(&tail);
        return cat(rec(&head), tw);
    }
    if chars > 1 && alpha(name) && !cx.known(lower) && !crate::aswd::is_word(lower) {
        let b = name.as_bytes();
        if b[b.len() - 1] == b's' && !b[b.len() - 2].is_ascii_lowercase() {
            return cons_after(exp_letters(&name[..name.len() - 1]), "'s");
        }
        return exp_letters(name);
    }
    if crate::rx!("^[A-Z][a-z]+[A-Z][a-z]+.*$").is_match(name) {
        let k = (2..name.len()).find(|&k| name.as_bytes()[k].is_ascii_uppercase()).unwrap();
        let rest = rec(&name[k..]);
        return cons(&lower[..k], rest);
    }
    words(&[lower])
}

/// fn_047dc0: a currency or unit written onto a number, from the tables at
/// @0xb02d0 and @0xb08d0. It can consume the next token: "$5 million" is read
/// here whole, and "million" is left with an empty name.
fn amount(cx: &Cx, i: usize, name: &str, depth: usize) -> Vec<Word> {
    let rec = |s: &str| t2w(cx, i, s, depth + 1);
    let ch: Vec<char> = name.chars().collect();
    let first_digit = ch.iter().position(|c| c.is_ascii_digit()).unwrap_or(ch.len());
    let s = |a: usize, b: usize| ch[a..b].iter().collect::<String>();
    let (currency, amount, mut suffix) = if amount_a(name) {
        (s(0, first_digit), s(first_digit, ch.len()), None)
    } else if amount_c(name) {
        let k = ch.iter().position(|c| "ABCDEFGHIJKLMNOPQRSTUVWXYZ$£¤".contains(*c)).unwrap();
        (s(k, ch.len()), s(0, k), None)
    } else if amount_b(name) {
        let rest = &ch[first_digit..];
        let mut split = None;
        for j in 0..rest.len() {
            if is_wordchar(rest[j]) {
                split = Some((j, j));
                break;
            }
            if rest[j] == '-' && rest.get(j + 1).map_or(true, |&c| is_wordchar(c)) {
                split = Some((j, j + 1));
                break;
            }
        }
        let cur = s(0, first_digit);
        match split {
            Some((a, b)) => (cur, rest[..a].iter().collect(), Some(rest[b..].iter().collect())),
            None => (cur, rest.iter().collect(), None),
        }
    } else {
        return Vec::new();
    };
    let unit = CURRENCIES.iter().find(|c| c.0 == currency);
    if suffix.is_none() && unit.is_some() && cx.has(i, 1) {
        let n = cx.tname(i + 1);
        if ILLIONS.contains(&n.as_str()) || n == "thousand" {
            cx.set_name(i + 1, "");
            suffix = Some(n);
        }
    }
    let mult = |s: &str| MULTIPLIERS.iter().find(|m| m.0 == s).map_or(s.to_string(), |m| m.1.into());
    match unit {
        Some(&(_, one, many)) => {
            let mut r = if amount == "1" { words(&["one"]) } else { rec(&amount) };
            if let Some(sfx) = &suffix {
                r.extend(rec(&mult(sfx)));
            }
            r.extend(rec(if amount == "1" { one } else { many }));
            r
        }
        None if currency.contains('$') => {
            let mut r = rec(&amount);
            if let Some(sfx) = &suffix {
                r.extend(rec(&mult(sfx)));
            }
            r.extend(rec(&currency));
            r
        }
        None if amount_b(name) || amount_a(name) => {
            let tail = rec(&s(currency.chars().count(), ch.len()));
            cat(rec(&currency), tail)
        }
        None if amount_c(name) => {
            let c = rec(&currency);
            cat(rec(&amount), c)
        }
        None => Vec::new(),
    }
}

/// Rule 53.
fn money_words(cx: &Cx, i: usize, name: &str) -> Vec<Word> {
    let sym = name.chars().next().unwrap();
    let (one, many, c1, cn) = match sym {
        '£' => ("pound", "pounds", "penny", "pence"),
        '¤' => ("euro", "euros", "cent", "cents"),
        _ => ("dollar", "dollars", "cent", "cents"),
    };
    let body = &name[sym.len_utf8()..];
    if illion(&cx.name_at(i, 1)) {
        return exp_real(body);
    }
    let Some(dot) = name.find('.') else {
        return cons_after(exp_real(body), if body == "1" { one } else { many });
    };
    let decimals = &name[dot..];
    if decimals.len() == 2 || decimals.len() > 3 {
        return cons_after(exp_real(body), many);
    }
    let c = &decimals[1..];
    let cents = |c: &str| cons_after(exp_number(c), if c == "01" { c1 } else { cn });
    if name.len() == 4 && sym == '$' && name.as_bytes()[1] == b'.' {
        return cents(c);
    }
    let whole: String = name[sym.len_utf8()..dot].chars().filter(|&c| c != ',').collect();
    let r = cons_after(exp_number(&whole), if whole == "1" { one } else { many });
    if c == "00" { r } else { cat(r, cents(c)) }
}

/// fn_046930: 17-Feb-2026, 2026-Feb-17, 17-Feb-05.
fn day_month_year(cx: &Cx, i: usize, name: &str, depth: usize) -> Vec<Word> {
    let rec = |s: &str| t2w(cx, i, s, depth + 1);
    let d1 = name.find('-').unwrap();
    let (head, after) = (&name[..d1], &name[d1 + 1..]);
    let d2 = after.find('-').unwrap();
    let (mon, last) = (&after[..d2], &after[d2 + 1..]);
    let month = || month_abbrev(cx, i, &mon.to_ascii_lowercase()).unwrap_or_default();
    if MONTH_NAMES.contains(&mon) {
        if year3(head) {
            if last.len() <= 2 {
                let ord = exp_ordinal(last);
                return cat(cat(month(), ord), exp_number(head));
            }
        } else if year3(last) {
            if head.len() <= 2 {
                let ord = exp_ordinal(head);
                return cat(cat(month(), ord), exp_number(last));
            }
        } else if atoi(head) < 32 && head.len() <= 2 {
            let ord = exp_ordinal(head);
            let year = if last.starts_with('0') {
                cons("oh", exp_number(last))
            } else {
                exp_number(last)
            };
            return cat(cat(month(), ord), year);
        }
    }
    let m = rec(mon);
    let h = rec(head);
    cat(cat(h, m), rec(last))
}

/// fn_0409b4: month abbreviations. The ambiguous ones (jan, mar, apr, aug,
/// dec) need a number, "of" after a number, or a hyphen beside them.
fn month_abbrev(cx: &Cx, i: usize, lower: &str) -> Option<Vec<Word>> {
    let &(full, key, ambiguous) = MONTH_ABBREVS.iter().find(|m| m.1 == lower)?;
    let (expand, keep) = (Some(words(&[full])), Some(words(&[key])));
    if !ambiguous {
        return expand;
    }
    let (p, n) = (cx.name_at(i, -1), cx.name_at(i, 1));
    if cx.has(i, -1) && double(&p) {
        return expand;
    }
    if cx.has(i, -1) && p == "of" && cx.has(i, -2)
        && cx.tname(i - 2).chars().next().map_or(true, |c| c.is_ascii_digit())
    {
        return expand;
    }
    if cx.has(i, 1) && double(&n) {
        return expand;
    }
    if p == "last" && cx.punc(i) == "." {
        return expand;
    }
    if p == "saw" && cx.punc(i) == "." {
        return keep;
    }
    let tn = cx.tname(i);
    if tn == lower {
        return keep;
    }
    let ln = tn.to_ascii_lowercase();
    match ln.find(lower) {
        Some(at) if at > 0 => {
            let b = ln.as_bytes();
            let end = at + lower.len();
            if (at >= 2 && b[at - 1] == b'-') || (end < b.len() && b[end] == b'-') {
                expand
            } else {
                keep
            }
        }
        _ => keep,
    }
}

/// fn_0467ec: day abbreviations; sun, mon, wed and sat need a month or a
/// number after them.
fn day_abbrev(cx: &Cx, i: usize, lower: &str) -> Option<Vec<Word>> {
    let &(full, key, ambiguous) = DAY_ABBREVS.iter().find(|d| d.1 == lower)?;
    let n = cx.name_at(i, 1);
    if !ambiguous || (cx.has(i, 1) && (MONTH_NAMES.contains(&n.as_str()) || double(&n))) {
        Some(words(&[full]))
    } else {
        Some(words(&[key]))
    }
}

/// fn_046488: the abbreviation table, tried before any other rule. A key's
/// trailing '.' is optional when the token's punc holds a '.', since the
/// tokenizer has already moved the period there.
fn abbreviation(cx: &Cx, i: usize, name: &str) -> Option<Vec<Word>> {
    let lower = name.to_lowercase();
    let dot = cx.punc(i).contains('.');
    for (key, exp) in ABBREVIATIONS {
        let k = if dot { key.strip_suffix('.').unwrap_or(key) } else { key };
        if name == k || lower == k {
            return Some(exp.split(' ').map(Word::new).collect());
        }
    }
    None
}

/// fn_0418b8 with format "mdy": the month by name, the day as an ordinal, a
/// phrase break, then the year. The split is on '-' if the token has one
/// anywhere, else '/', so a mixed "02/17-2026" splits into two and says
/// nothing at all -- which is what the engine does with it.
fn expand_date(name: &str) -> Option<Vec<Word>> {
    const MONTHS: [&str; 12] = [
        "january", "february", "march", "april", "may", "june", "july", "august",
        "september", "october", "november", "december",
    ];
    let sep = if name.contains('-') { '-' } else { '/' };
    let f: Vec<&str> = name.split(sep).collect();
    if f.len() != 3 {
        return Some(Vec::new());
    }
    let m: usize = f[0].parse().ok()?;
    let r = add_break(cat(words(&[MONTHS.get(m.wrapping_sub(1))?]), exp_ordinal(f[1])));
    let year = if f[2] == "00" { words(&["two", "thousand"]) } else { exp_id(f[2]) };
    Some(cat(r, year))
}

/// fn_040618, flite's `state_name` with Cepstral's additions: an "address"
/// token always expands, the token must be whole, a Canadian postal code's
/// first half counts as a following ZIP, and expanding clears the previous
/// token's comma so "Springfield, IL" does not pause before the state.
///
/// An ambiguous abbreviation (IL, OR, IN, ME ...) needs a capitalised,
/// alphabetic word of more than three letters before it and, after it, a
/// lowercase word, the end of the text, a full stop on itself, or a ZIP. That
/// is why the engine leaves the first IL of "Peoria, IL, Springfield, IL"
/// unexpanded: a comma and a capital follow it.
fn state_name(cx: &Cx, i: usize, name: &str) -> Option<Vec<Word>> {
    let near = |off: isize| if cx.has(i, off) { cx.tname((i as isize + off) as usize) } else { String::new() };
    for (ab, ambiguous, ws) in STATES {
        if *ab != name {
            continue;
        }
        let expand = !ambiguous || cx.nsw(i) == "address" || {
            let (p, n) = (near(-1), near(1));
            let postal = n.len() == 3 && {
                let b = n.as_bytes();
                b[0].is_ascii_uppercase() && b[1].is_ascii_digit() && b[2].is_ascii_uppercase()
            };
            p.starts_with(|c: char| c.is_ascii_uppercase())
                && p.len() > 3
                && name == cx.tname(i)
                && alpha(&p)
                && (n.starts_with(|c: char| c.is_ascii_lowercase())
                    || !cx.has(i, 1)
                    || cx.punc(i) == "."
                    || postal
                    || ((n.len() == 5 || n.len() == 10) && digits(&n)))
        };
        if !expand {
            continue;
        }
        if i > 0 && cx.punc(i - 1) == "," {
            cx.set_punc(i - 1, "");
        }
        return Some(words(ws));
    }
    None
}

// ------------------------------------------------------------------ say-as

/// @0xa3900, what `letters` reads a punctuation mark as. `=` really is "dash".
const PUNC_NAMES: [(char, &str); 32] = [
    ('`', "backquote"), ('~', "tilde"), ('!', "exclamation point"), ('@', "at"),
    ('#', "hash"), ('$', "dollar"), ('%', "percent"), ('^', "carat"), ('&', "ampersand"),
    ('*', "asterisk"), ('(', "open parenthesis"), (')', "close parenthesis"),
    ('_', "underscore"), ('-', "dash"), ('=', "dash"), ('+', "plus"),
    ('[', "open square bracket"), (']', "close square bracket"), ('{', "open bracket"),
    ('}', "close bracket"), ('\\', "back slash"), ('|', "vertical bar"), (';', "semicolon"),
    (':', "colon"), ('\'', "apostrophe"), ('"', "quote"), (',', "comma"), ('<', "less than"),
    ('.', "dot"), ('>', "greater than"), ('/', "slash"), ('?', "question mark"),
];

/// fn_04022c: a word per character. Unlike flite's it keeps whitespace, and
/// with `punc` it names punctuation instead of passing it through.
fn letters(s: &str, punc: bool) -> Vec<Word> {
    let mut r = Vec::new();
    for c in s.chars().map(|c| c.to_ascii_lowercase()) {
        if let Some(d) = c.to_digit(10) {
            r.push(Word::new(crate::token::DIGIT2NUM[d as usize]));
        } else if let Some(&(_, w)) = PUNC_NAMES.iter().find(|p| punc && p.0 == c) {
            r.extend(w.split(' ').map(Word::new));
        } else if c == 'a' {
            r.push(Word::new("_a"));
        } else {
            r.push(Word::new(&c.to_string()));
        }
    }
    r
}

/// swift.dll's `cst_val_string_split`: every field, empty ones included.
fn split_any<'a>(s: &'a str, seps: &str) -> Vec<&'a str> {
    s.split(|c| seps.contains(c)).collect()
}

/// @0x42a58, ceplang's reading of a `<say-as>` token's `nsw`. `None` (or
/// nothing said) sends the token on through the ordinary rules.
fn say_as(cx: &Cx, i: usize, name: &str, nsw: &str) -> Option<Vec<Word>> {
    match nsw {
        "phones" => return Some(words(&[name])),
        "letters" => return Some(letters(name, false)),
        "literal" => {
            let whole = format!("{}{}{}{}", cx.toks[i].ws, cx.pre(i), name, cx.punc(i));
            return Some(letters(&whole, true));
        }
        "measure" => return say_measure(cx, i, name),
        "address" => return say_address(cx, i, name),
        _ => {}
    }
    if nsw.starts_with("number") {
        let n = if digits(name) {
            name.to_string()
        } else {
            match exp_roman(name) {
                0 => return None,
                v => v.to_string(),
            }
        };
        let kind = if nsw.as_bytes().get(6) == Some(&b':') { &nsw[7..] } else { "" };
        return Some(match kind {
            "ordinal" => exp_ordinal(&n),
            "cardinal" => exp_number(&n),
            // fn_041b8c; n is digits by now, so this is only ever the number
            "fraction" => {
                let p = split_any(&n, "/");
                match p.get(1) {
                    Some(d) => cat(exp_number(p[0]), exp_ordinal(d)),
                    None => exp_number(p[0]),
                }
            }
            "digits" | "digit" => exp_digits(&n),
            _ => exp_real(&n),
        });
    }
    if nsw.starts_with("date") {
        let parts = split_any(name, "/-.,");
        let fmt = if nsw.as_bytes().get(4) == Some(&b':') {
            &nsw[5..]
        } else {
            match parts.len() {
                3 => {
                    let s = parts[0];
                    if crate::rx!("^-?[0-9]+$").is_match(s) && (s.len() == 4 || atoi(s) > 31) {
                        "ymd"
                    } else {
                        "mdy"
                    }
                }
                2 => match atoi(parts[0]) {
                    v if v > 31 => "ym",
                    v if v <= 12 => "md",
                    _ => "dm",
                },
                1 => {
                    let m = MONTH_ABBREVS.iter().find(|m| m.1.eq_ignore_ascii_case(name))?;
                    return Some(words(&[m.0]));
                }
                _ => return None,
            }
        };
        return date_fmt(&parts, fmt);
    }
    if nsw.starts_with("time") {
        if name.len() == 4 && digits(name) {
            let mm = if &name[2..] == "00" { words(&["hundred"]) } else { exp_id(&name[2..]) };
            return Some(cat(exp_id(&name[..2]), mm));
        }
        let parts = split_any(name, ":.");
        let mut r = exp_number(parts[0]);
        for k in 1..parts.len() {
            // trailing "00"s say nothing, but a "00" before anything else is read
            if parts[k..].iter().all(|p| *p == "00") {
                break;
            }
            r.extend(exp_id(parts[k]));
        }
        return Some(r);
    }
    None
}

/// fn_0418b8 with its format: the parts named by `y`, `m` and `d`, month
/// first, then day, then year, with a break after the day when all three
/// are there.
fn date_fmt(parts: &[&str], fmt: &str) -> Option<Vec<Word>> {
    if parts.len() != fmt.len() {
        return None;
    }
    let (mut month, mut day, mut year) = (Vec::new(), Vec::new(), Vec::new());
    for (c, s) in fmt.bytes().zip(parts) {
        match c {
            b'y' => year = if *s == "00" { words(&["two", "thousand"]) } else { exp_id(s) },
            b'm' => {
                let v = atoi(s);
                let full = if (1..=12).contains(&v) {
                    MONTH_ABBREVS[v as usize - 1].0
                } else {
                    MONTH_ABBREVS.iter().find(|m| m.1.eq_ignore_ascii_case(s))?.0
                };
                month = words(&[full]);
            }
            b'd' => {
                if !(1..=31).contains(&atoi(s)) {
                    return None;
                }
                day = exp_ordinal(s);
            }
            _ => {}
        }
    }
    let mut r = month;
    if !day.is_empty() {
        let dated = !r.is_empty();
        r = cat(r, day);
        if dated && !year.is_empty() {
            r = add_break(r);
        }
    }
    Some(cat(r, year))
}

/// sub_040d40: each N, S, E or W its own word, anything else skipped.
fn compass(s: &str) -> Vec<Word> {
    s.chars().filter_map(|c| match c {
        'E' => Some(Word::new("east")),
        'N' => Some(Word::new("north")),
        'S' => Some(Word::new("south")),
        'W' => Some(Word::new("west")),
        _ => None,
    }).collect()
}

/// sub_04233c, the say-as address reader. Directions, "St" after one, a
/// Canadian "CA", states, street words, then house numbers and ZIP codes.
fn say_address(cx: &Cx, i: usize, name: &str) -> Option<Vec<Word>> {
    let dir = |s: &str| crate::rx!("^[ENSW][ENSW]?$").is_match(s);
    let zip = |s: &str| crate::rx!(r"^[0-9]{5}(-[0-9]{4})?$").is_match(s);
    if cx.ws_at(i, 1).contains('\n') {
        cx.set_punc(i, "\n");
    }
    let rest = || -> Option<Vec<Word>> { address_rest(cx, i, name) };
    if dir(name) {
        if name == "NE" && ((cx.has(i, 1) && zip(&cx.tname(i + 1))) || cx.punc_at(i, -1) == ",") {
            return rest();
        }
        if name.len() != 1 {
            return Some(compass(name));
        }
        if cx.punc(i) != "." && cx.has(i, 1)
            && crate::rx!("^[Ss][Tt]([Rr][Ee][Ee][Tt])?$").is_match(&cx.tname(i + 1))
        {
            feat(i + 1, |f| f.street = true);
            return rest();
        }
        if cx.punc_at(i, -1).is_empty() && cx.has(i, -1)
            && crate::rx!("^[Aa][Vv]([Ee][Nn][Uu][Ee]|[Ee])?$").is_match(&cx.tname(i - 1))
        {
            return rest();
        }
        return Some(compass(name));
    }
    let b = name.as_bytes();
    if crate::rx!(r"^[A-Za-z](\.[A-Za-z])+\.?$").is_match(name) && b.len() == 3
        && b"NSEW".contains(&b[0]) && b"NSEW".contains(&b[2])
    {
        return Some(compass(&format!("{}{}", b[0] as char, b[2] as char)));
    }
    rest()
}

const CANADIAN_CITIES: [&str; 8] = [
    "ottawa", "toronto", "montreal", "vancouver", "quebec", "calgary", "edmonton", "winnipeg",
];

fn address_rest(cx: &Cx, i: usize, name: &str) -> Option<Vec<Word>> {
    let dir = |s: &str| crate::rx!("^[ENSW][ENSW]?$").is_match(s);
    let zip = |s: &str| crate::rx!(r"^[0-9]{5}(-[0-9]{4})?$").is_match(s);
    if name.to_ascii_lowercase() == "st" {
        let street = feat(i, |f| f.street).unwrap_or(false)
            || (cx.has(i, 1) && dir(&cx.tname(i + 1)));
        return street.then(|| words(&["street"]));
    }
    if name == "CA" {
        let p = cx.name_at(i, -1).to_ascii_lowercase();
        if crate::rx!("^[A-Z][0-9][A-Z]$").is_match(&cx.name_at(i, 1))
            || CANADIAN_CITIES.contains(&p.as_str())
        {
            return Some(words(&["canada"]));
        }
    }
    if let Some(r) = state_name(cx, i, name) {
        return Some(r);
    }
    if let Some(r) = address(cx, i, name) {
        return Some(r);
    }
    if !digits(name) {
        let dash = name.find('-').filter(|_| zip(name))?;
        return Some(cat(exp_digits(&name[..dash]), cons("dash", exp_digits(&name[dash + 1..]))));
    }
    let len = name.len();
    if crate::rx!("^[1-9]?[1-9]00$").is_match(name) {
        return Some(cat(exp_number(&name[..len - 2]), words(&["hundred"])));
    }
    if crate::rx!("^[1-9]?[1-9]000$").is_match(name) {
        return Some(exp_number(name));
    }
    match len {
        4 if name.as_bytes()[1] == b'0' || zip(&cx.name_at(i, -1)) => Some(exp_digits(name)),
        4 => Some(exp_id(name)),
        n if n > 3 => Some(exp_digits(name)),
        3 => {
            let tail = &name[1..];
            let t = if tail.starts_with('0') { exp_digits(tail) } else { exp_number(tail) };
            Some(cat(exp_digits(&name[..1]), t))
        }
        _ => None,
    }
}

/// fn_041da0. A trailing `'` or `"` is feet or inches, and is taken off the
/// token's punctuation even when the answer is nothing.
fn say_measure(cx: &Cx, i: usize, name: &str) -> Option<Vec<Word>> {
    let n = atoi(name);
    let b = name.as_bytes();
    let len = b.len();
    let unit = |one: &str, many: &str| if n == 1 { one.to_string() } else { many.to_string() };
    let with = |num: &str, w: String| {
        if num.is_empty() { words(&[&w]) } else { cat(exp_number(num), words(&[&w])) }
    };
    let mark = cx.punc(i);
    if mark == "'" {
        cx.set_punc(i, "");
        return Some(with(name, unit("foot", "feet")));
    }
    if mark == "\"" {
        cx.set_punc(i, "");
        if name.contains('\'') {
            return None;
        }
        return Some(with(name, unit("inch", "inches")));
    }
    match b.last() {
        Some(b'm') => {
            let prefix = if len > 1 { "nmck".find(b[len - 2] as char) } else { None };
            let (w, num) = match prefix {
                Some(k) => (["nano", "milli", "centi", "kilo"][k].to_string()
                                + &unit("meter", "meters"), name.get(..len - 2)?),
                None => (unit("meter", "meters"), &name[..len - 1]),
            };
            Some(with(num, w))
        }
        Some(b'l' | b'L') => {
            if len > 1 && b[len - 2] == b'm' {
                Some(with(name.get(..len - 2)?, unit("milliliter", "milliliters")))
            } else {
                Some(with(&name[..len - 1], unit("liter", "liters")))
            }
        }
        _ if len <= 2 => None,
        _ => {
            let w = match name.get(len - 2..)? {
                "mi" => unit("mile", "miles"),
                "in" => unit("inch", "inches"),
                "ft" => unit("foot", "feet"),
                _ => return None,
            };
            Some(with(&name[..len - 2], w))
        }
    }
}
