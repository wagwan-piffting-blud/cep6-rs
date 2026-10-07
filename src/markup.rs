//! SAPI XML and Swift SSML markup, read into plain text plus the controls in
//! force at each character.
//!
//! The Cepstral SAPI engine does not act on SAPI XML itself:
//! `Swift_SAPI_XML_Converter::decorate_with_ssml` in SwiftTTSEngine.dll
//! rewrites each SAPI tag as the Swift SSML one, so both spellings are accepted
//! here and land on the same controls.
//!
//! | SAPI                 | Swift SSML                       |
//! | -------------------- | -------------------------------- |
//! | `<volume level>`     | `<prosody volume>`               |
//! | `<rate absspeed>`    | `<prosody rate>`                 |
//! | `<pitch absmiddle>`  | `<prosody pitch>`                |
//! | `<silence msec>`     | `<break time="Nms"/>`            |
//! | `<pron sym>`         | `<phoneme ph>`                   |
//! | `<emph>`             | `<emphasis level="moderate">`    |
//! | `<spell>`            | `<say-as type="spell-out">`      |
//! | `<bookmark mark>`    | `<mark name>`                    |
//!
//! SwiftTTSEngine's rewriting is reproduced exactly where it changes a value:
//! SAPI rate r becomes `rate="%.2f"` of 0.98 + 0.06 r below zero and
//! 1 + 0.15 r above, SAPI pitch p becomes `pitch="%+dst"`, and a level of 0
//! sends no tag at all (captured with `_out/run_speak.py`).
//!
//! Volume, breaks, rate, pitch and phonemes act. The rest are recognised, their
//! text is kept, and each is reported once in `Marked::ignored` rather than
//! silently dropped.

use std::fmt;

/// The controls in force over a stretch of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ctl {
    /// The unit residual rescale `<volume level="N">` sets, N / 100. Verified
    /// bit-exact against the engine at every level.
    pub volume: f32,
    /// The token's `local_duration_stretch`: unit length multiplier.
    pub stretch: f32,
    /// The token's `local_f0_shift`: pitch multiplier.
    pub f0: f32,
    /// The SAPI levels, -10..10, that relative `<rate speed>` and
    /// `<pitch middle>` add to.
    sapi_rate: i32,
    sapi_pitch: i32,
    sapi_emph: bool,
}

impl Default for Ctl {
    fn default() -> Ctl {
        Ctl { volume: 1.0, stretch: 1.0, f0: 1.0, sapi_rate: 0, sapi_pitch: 0, sapi_emph: false }
    }
}

impl Ctl {
    /// SwiftTTSEngine wraps each fragment's absolute `<prosody>` tags inside
    /// the `<emphasis>`, so an `<emph>` only slows the text when no SAPI rate
    /// is in force, and never changes its volume.
    fn sapi_stretch(&self) -> f32 {
        match (self.sapi_rate, self.sapi_emph) {
            (0, false) => 1.0,
            (0, true) => stretch_of("-25%", 1.0).unwrap_or(1.0),
            (l, _) => stretch_of(&sapi_rate_value(l), 1.0).unwrap_or(1.0),
        }
    }
}

/// What a `<break>` does, from swift.dll @0x3a370.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Break {
    /// A timed pause after the last token read, which is also marked `break`.
    Pause(f32),
    /// `weak` / `x-weak` / `small` with no time: the token is only marked
    /// `break`, which the phrasing tree turns into a phrase break.
    Phrase,
    /// `medium` (the default) or stronger: the utterance ends here. A strong
    /// one makes the next a paragraph, whose opening pause lasts 0.5 s.
    Utterance { paragraph: bool },
}

#[derive(Debug, Default)]
pub struct Marked {
    /// The text with every tag removed and entities decoded.
    pub text: String,
    /// (first char, controls from there on), ascending.
    changes: Vec<(usize, Ctl)>,
    /// (char position, what) for each `<break>` / `<silence>`.
    pub breaks: Vec<(usize, Break)>,
    /// The char position of every tag, ascending: each one ends a token.
    pub splits: Vec<usize>,
    /// (char position, `ph` as the engine received it, its words) for each
    /// `<phoneme>` / `<pron>` the engine would accept. Each word is its list of
    /// items: phones with an optional stress digit, lone digits, and `.`.
    pub phonemes: Vec<(usize, String, Vec<Vec<String>>)>,
    /// (first char, the `nsw` a `<say-as>` puts on the tokens read from there
    /// on), ascending.
    say_as: Vec<(usize, Option<String>)>,
    /// Recognised tags that do nothing yet, each named once.
    pub ignored: Vec<String>,
}

impl Marked {
    /// The controls at a char index of `text`.
    pub fn ctl_at(&self, at: usize) -> Ctl {
        match self.changes.partition_point(|(c, _)| *c <= at) {
            0 => Ctl::default(),
            k => self.changes[k - 1].1,
        }
    }

    /// The `nsw` of a token starting at a char index of `text`.
    pub fn nsw_at(&self, at: usize) -> Option<&str> {
        match self.say_as.partition_point(|(c, _)| *c <= at) {
            0 => None,
            k => self.say_as[k - 1].1.as_deref(),
        }
    }
}

#[derive(Debug)]
pub struct MarkupError(pub String);

impl fmt::Display for MarkupError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MarkupError {}

/// Text with no markup in it at all.
pub fn plain(text: &str) -> Marked {
    Marked { text: text.to_string(), ..Default::default() }
}

struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    close: bool,
    empty: bool,
}

impl Tag {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(a, _)| a.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
}

fn err<T>(msg: String) -> Result<T, MarkupError> {
    Err(MarkupError(msg))
}

fn decode_entity(ent: &str) -> Option<char> {
    match ent {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let n = ent.strip_prefix('#')?;
            let v = match n.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(v)
        }
    }
}

fn decode(s: &str) -> Result<String, MarkupError> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let Some(j) = tail.find(';') else { return err(format!("unterminated entity in {s:?}")) };
        match decode_entity(&tail[..j]) {
            Some(c) => out.push(c),
            None => return err(format!("unknown entity &{};", &tail[..j])),
        }
        rest = &tail[j + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn parse_tag(body: &str) -> Result<Tag, MarkupError> {
    let close = body.starts_with('/');
    let empty = body.ends_with('/');
    let inner = body.trim_start_matches('/').trim_end_matches('/').trim();
    let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    if name.is_empty() {
        return err(format!("empty tag <{body}>"));
    }
    let mut attrs = Vec::new();
    let mut rest = inner[name_end..].trim_start();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { return err(format!("bad attribute in <{body}>")) };
        let key = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let q = after.chars().next().unwrap_or(' ');
        if q != '"' && q != '\'' {
            // Windows PowerShell 5.1 strips the quotes out of an argument
            // handed to a native program, so `level="50"` arrives as `level=50`
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            attrs.push((key, decode(&after[..end])?));
            rest = after[end..].trim_start();
            continue;
        }
        let Some(end) = after[1..].find(q) else { return err(format!("unterminated attribute in <{body}>")) };
        attrs.push((key, decode(&after[1..1 + end])?));
        rest = after[end + 2..].trim_start();
    }
    Ok(Tag { name, attrs, close, empty })
}

/// `<prosody volume>`, @0x39e30: the new `local_rescale`. A signed value is
/// relative, in hundredths or with `%` in percent of the current rescale; a
/// plain number is hundredths; anything else is a keyword, and one that is
/// not in the table (`50%` among them) is 1.
fn rescale_of(v: &str, cur: f32) -> f32 {
    if v.starts_with('-') || v.starts_with('+') {
        let d = if v.ends_with('%') { atof(v) * cur as f64 } else { atof(v) };
        return cur + (d / 100.0) as f32;
    }
    if is_double(v) {
        return (atof(v) / 100.0) as f32;
    }
    [("silent", 0.0), ("x-soft", 0.25), ("soft", 0.5), ("medium", 1.0), ("default", 1.0),
     ("loud", 1.5), ("x-loud", 2.0)]
        .iter().find(|(k, _)| *k == v).map_or(1.0, |&(_, r)| r)
}

/// `<say-as>`, @0x3b230: the `nsw` its content handler puts on each token,
/// which ceplang_en.dll's say-as expander @0x42a58 reads.
fn say_as_nsw(tag: &Tag) -> Option<String> {
    if let Some(t) = tag.attr("type") {
        return Some(match t {
            "acronym" | "spell-out" => "letters".into(),
            t => t.into(),
        });
    }
    let i = tag.attr("interpret-as")?;
    Some(match (tag.attr("format"), i) {
        (Some(f), i) => format!("{i}:{f}"),
        (None, "ordinal" | "cardinal" | "digits") => format!("number:{i}"),
        (None, i) => i.into(),
    })
}

/// C's `atof`: the longest numeric prefix after leading whitespace, else 0.
pub(crate) fn atof(s: &str) -> f64 {
    let b = s.trim_start().as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let st = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > st
    };
    let mut any = digits(&mut i);
    if i < b.len() && b[i] == b'.' {
        i += 1;
        any |= digits(&mut i);
    }
    if !any {
        return 0.0;
    }
    let mut end = i;
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if digits(&mut j) {
            end = j;
        }
    }
    std::str::from_utf8(&b[..end]).ok().and_then(|x| x.parse().ok()).unwrap_or(0.0)
}

/// @0x3a2d0: `Nms` or `Ns` and nothing else; a bare number is not a time.
fn seconds_of(t: &str) -> Option<f32> {
    if t.len() >= 3 && t.ends_with("ms") {
        Some((atof(t) / 1000.0) as f32)
    } else if t.len() >= 2 && t.ends_with('s') {
        Some(atof(t) as f32)
    } else {
        None
    }
}

/// @0x3a370. `None` when the break does nothing.
fn break_of(time: Option<&str>, strength: Option<&str>, size: Option<&str>) -> Option<Break> {
    let strength = strength.or(size).unwrap_or("medium");
    let secs = time.and_then(seconds_of).unwrap_or(-1.0);
    if strength == "none" {
        return None;
    }
    if matches!(strength, "x-weak" | "weak" | "small") || secs > 0.0 {
        return Some(if secs > 0.0 { Break::Pause(secs) } else { Break::Phrase });
    }
    match strength {
        "medium" => Some(Break::Utterance { paragraph: false }),
        "strong" | "large" | "x-strong" => Some(Break::Utterance { paragraph: true }),
        _ => None,
    }
}

/// flite's `cst_rx_double`, anchored.
fn is_double(s: &str) -> bool {
    crate::rx!(r"^-?(([0-9]+\.[0-9]*)|([0-9]+)|(\.[0-9]+))([eE][-+]?[0-9]+)?$").is_match(s)
}

/// `<prosody rate>`, @0x39250: the new `local_duration_stretch`, or `None`
/// to leave it as it is. `speaking_rate` is never set here, so the base the
/// handler divides is 1.
fn stretch_of(v: &str, cur: f32) -> Option<f32> {
    let base = 1.0f32;
    if v.starts_with('-') || v.starts_with('+') {
        let x = atof(v) as f32;
        let mut s = cur;
        if v.ends_with('%') {
            if x != -100.0 && x != 0.0 {
                s = base / (x / 100.0 + 1.0);
            }
        } else if x > -1.0 && x != 0.0 {
            s = base / (x + 1.0);
        }
        return if s == 1.0 { None } else { Some(s) };
    }
    if is_double(v) {
        let x = atof(v) as f32;
        return if x <= 0.0 { None } else { Some(base / x) };
    }
    [("x-slow", 2.0), ("slow", 1.5), ("medium", 1.0), ("default", 1.0),
     ("fast", 0.75), ("x-fast", 0.5)]
        .iter().find(|(k, _)| *k == v).map(|&(_, s)| s)
}

/// `<prosody pitch>`, @0x39460: the new `local_f0_shift`, or `None` to leave
/// it. A semitone here is `2^(1/24)`. The absolute `Hz` forms switch the
/// engine to a pitch-contour join these voices cannot do, so they are
/// reported rather than followed.
fn f0_of(v: &str, cur: f32) -> Result<Option<f32>, &'static str> {
    if !v.starts_with('-') && !v.starts_with('+') {
        let kw = [("x-low", 0.5f32), ("low", 0.75), ("medium", 1.0), ("default", 1.0),
                  ("high", 2.0), ("x-high", 3.0)];
        return match kw.iter().find(|(k, _)| *k == v) {
            Some(&(_, f)) => Ok(if f != 1.0 { Some(f) } else { None }),
            None if v.ends_with("Hz") => Err("prosody pitch in Hz"),
            None => Ok(None),
        };
    }
    if v.ends_with('%') {
        return Ok(Some(cur + (atof(v) / 100.0) as f32 * cur));
    }
    if v.len() >= 3 && v.ends_with("st") {
        return Ok(Some(cur * 2f64.powf(atof(v) / 24.0) as f32));
    }
    if v.len() >= 3 && v.ends_with("Hz") {
        return Err("prosody pitch in Hz");
    }
    Ok(None)
}

/// swift.dll's SAMPA table @0x7d970, in its order, which is the order the
/// tokenizer @0x3aa90 tries them: the first key that is a prefix wins.
const SAMPA: [(&str, &str); 52] = [
    (" ", "|"), ("p", "p"), ("aI", "ay"), ("OI", "oy"), ("aU", "aw"), ("tS", "ch"),
    ("dZ", "jh"), ("b", "b"), ("t", "t"), ("d", "d"), ("k", "k"), ("g", "g"),
    ("f", "f"), ("v", "v"), ("T", "th"), ("D", "dh"), ("s", "s"), ("z", "z"),
    ("S", "sh"), ("Z", "zh"), ("h", "h"), ("j", "j"), ("m", "m"), ("n", "n"),
    ("N", "ng"), ("r", "r"), ("l", "l"), ("w", "w"), ("4", "d"), ("i", "i"),
    ("I", "ih"), ("e", "ey"), ("E", "eh"), ("A", "aa"), ("o", "ow"), ("O", "ao"),
    ("V", "ah"), ("u", "uw"), ("U", "uh"), ("1", "ih"), ("3`", "t"), ("@r", "er"),
    ("@", "ah"), ("\"", "1"), ("+", "1"), ("?", "pau"), ("{", "ae"), ("%", "0"),
    ("*", "0"), ("|", "."), ("$", "0"), ("#", "|"),
];

/// @0x3aa90: SAMPA to the table's keys. A run of whitespace is one `" "` key
/// (a word break) unless it leads or trails; anything no key matches fails.
fn sampa_keys(s: &str) -> Option<Vec<&'static str>> {
    let ws = |c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r';
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if ws(c) {
            let after = rest.trim_start_matches(ws);
            if !after.is_empty() && rest.len() != s.len() {
                out.push(" ");
            }
            rest = after;
            continue;
        }
        let (k, _) = SAMPA.iter().find(|(k, _)| rest.starts_with(k))?;
        out.push(*k);
        rest = &rest[k.len()..];
    }
    Some(out)
}

/// @0x3ac20: `ph` in `alphabet` to Cepstral items, split into words at `|`.
/// `None` when the engine rejects it: an unknown alphabet or an item that is
/// neither a phone of the phoneset (with an optional stress digit), a lone
/// 0..2, `.` nor `|`.
pub(crate) fn phoneme_items(ph: &str, alphabet: Option<&str>) -> Option<Vec<Vec<String>>> {
    let items: Vec<String> = match alphabet {
        None | Some("cepstral_en") => ph.split([' ', '\n', '\r', '\t', '\x0c'])
            .filter(|s| !s.is_empty()).map(str::to_string).collect(),
        Some("sampa") => sampa_keys(ph)?.into_iter()
            .map(|k| SAMPA.iter().find(|(s, _)| *s == k).unwrap().1.to_string())
            .collect(),
        Some(_) => return None,
    };
    for it in &items {
        let lone_digit = it.len() == 1 && matches!(it.as_bytes()[0], b'0'..=b'2');
        if lone_digit || it == "." || it == "|" {
            continue;
        }
        let base = if it.ends_with(|c: char| c.is_ascii_digit()) { &it[..it.len() - 1] } else { it };
        if !crate::phoneset::known(base) {
            return None;
        }
    }
    let mut words = vec![Vec::new()];
    for it in items {
        if it == "|" {
            if !words.last().unwrap().is_empty() {
                words.push(Vec::new());
            }
        } else {
            words.last_mut().unwrap().push(it);
        }
    }
    words.retain(|w| !w.is_empty());
    Some(words)
}

/// SwiftTTSEngine's rewriting of a SAPI `<pron sym>` into a `ph`: `iy` and `y`
/// become Cepstral's `i` and `j`, a stress digit joins the phone before it, the
/// syllable mark `-` becomes `.`, silence `_` becomes `pau`, and the other SAPI
/// marks are dropped.
fn sapi_ph(sym: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for s in sym.split_whitespace() {
        match s {
            "1" | "2" => match out.last_mut() {
                Some(p) => p.push_str(s),
                None => out.push(s.to_string()),
            },
            "-" => out.push(".".into()),
            "_" => out.push("pau".into()),
            "!" | "&" | "," | "." | "?" => {}
            "iy" => out.push("i".into()),
            "y" => out.push("j".into()),
            p => out.push(p.to_string()),
        }
    }
    out.join(" ")
}

/// SwiftTTSEngine's rendering of a SAPI rate level as a `<prosody rate>` value.
fn sapi_rate_value(level: i32) -> String {
    let r = level as f64;
    format!("{:.2}", if level < 0 { 0.98 + 0.06 * r } else { 1.0 + 0.15 * r })
}

fn sapi_level(tag: &Tag, abs: &str, rel: &str, cur: i32) -> Result<i32, MarkupError> {
    let num = |k: &str, v: &str| -> Result<i32, MarkupError> {
        v.trim().parse().map_err(|_| MarkupError(format!("<{} {k}={v:?}> is not a whole number", tag.name)))
    };
    let l = match (tag.attr(abs), tag.attr(rel)) {
        (Some(v), _) => num(abs, v)?,
        (None, Some(v)) => cur + num(rel, v)?,
        (None, None) => cur,
    };
    Ok(l.clamp(-10, 10))
}

/// Read SAPI XML or Swift SSML.
pub fn parse(src: &str) -> Result<Marked, MarkupError> {
    let mut m = Marked::default();
    let mut chars = 0usize;
    // the open tags, each with the controls it replaced
    let mut stack: Vec<(String, Ctl)> = Vec::new();
    let mut cur = Ctl::default();
    let mut rest = src;

    fn ignore(m: &mut Marked, what: &str) {
        if !m.ignored.iter().any(|x| x == what) {
            m.ignored.push(what.to_string());
        }
    }

    // inside a <phoneme> / <pron>, whose text is not read
    let mut skip = 0usize;
    let mut nsw: Option<String> = None;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if skip == 0 {
                m.text.push_str(&decode(rest)?);
            }
            break;
        };
        let t = decode(&rest[..lt])?;
        if skip == 0 {
            chars += t.chars().count();
            m.text.push_str(&t);
        }
        rest = &rest[lt..];

        if let Some(r) = rest.strip_prefix("<!--") {
            let Some(e) = r.find("-->") else { return err("unterminated comment".into()) };
            rest = &r[e + 3..];
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            let Some(e) = rest.find('>') else { return err("unterminated declaration".into()) };
            rest = &rest[e + 1..];
            continue;
        }
        let Some(gt) = rest.find('>') else { return err("unterminated tag".into()) };
        let tag = parse_tag(&rest[1..gt])?;
        rest = &rest[gt + 1..];
        if m.splits.last() != Some(&chars) {
            m.splits.push(chars);
        }

        if tag.close {
            if tag.name == "phoneme" || tag.name == "pron" {
                skip = skip.saturating_sub(1);
            }
            if tag.name == "say-as" || tag.name == "spell" {
                nsw = None;
            }
            match stack.pop() {
                Some((open, before)) if open == tag.name => cur = before,
                Some((open, _)) => return err(format!("</{}> closes <{open}>", tag.name)),
                None => return err(format!("</{}> with nothing open", tag.name)),
            }
        } else {
            let before = cur;
            match tag.name.as_str() {
                "volume" => {
                    if let Some(l) = tag.attr("level") {
                        let n: f64 = l.trim().parse().map_err(|_| {
                            MarkupError(format!("<volume level={l:?}> is not a number"))
                        })?;
                        cur.volume = (n.max(0.0) / 100.0) as f32;
                    }
                }
                "prosody" => {
                    if let Some(l) = tag.attr("volume") {
                        cur.volume = rescale_of(l.trim(), cur.volume);
                    }
                    if let Some(s) = tag.attr("rate").and_then(|r| stretch_of(r.trim(), cur.stretch)) {
                        cur.stretch = s;
                    }
                    if let Some(p) = tag.attr("pitch") {
                        match f0_of(p.trim(), cur.f0) {
                            Ok(Some(f)) => cur.f0 = f,
                            Ok(None) => {}
                            Err(what) => ignore(&mut m, what),
                        }
                    }
                    if tag.attr("range").is_some() || tag.attr("contour").is_some() {
                        ignore(&mut m, "prosody range/contour");
                    }
                }
                // SwiftTTSEngine sends each SAPI fragment with its own absolute
                // tags, never nested, so the base is always 1
                "rate" => {
                    cur.sapi_rate = sapi_level(&tag, "absspeed", "speed", cur.sapi_rate)?;
                    cur.stretch = cur.sapi_stretch();
                }
                "emph" => {
                    cur.sapi_emph = true;
                    cur.stretch = cur.sapi_stretch();
                }
                // @0x3a0f0: moderate is <prosody rate="-25%" volume="+25%">,
                // strong the same at 50%
                "emphasis" => {
                    let (rate, vol) = match tag.attr("level").map_or("moderate", str::trim) {
                        "moderate" => ("-25%", "+25%"),
                        "strong" => ("-50%", "+50%"),
                        _ => ("", ""),
                    };
                    if !rate.is_empty() {
                        if let Some(s) = stretch_of(rate, cur.stretch) {
                            cur.stretch = s;
                        }
                        cur.volume = rescale_of(vol, cur.volume);
                    }
                }
                // SwiftTTSEngine sends <spell> text as a bare <say-as>, outside
                // every other tag, so it is read at the defaults
                "spell" => {
                    nsw = Some("letters".into());
                    cur = Ctl { sapi_rate: cur.sapi_rate, sapi_pitch: cur.sapi_pitch,
                                ..Ctl::default() };
                }
                // One slot, not a stack: an inner <say-as> replaces the outer's
                // value and closing it clears the slot (@0x3b380).
                "say-as" => {
                    if let Some(s) = say_as_nsw(&tag) {
                        nsw = Some(s);
                    }
                }
                "pitch" => {
                    cur.sapi_pitch = sapi_level(&tag, "absmiddle", "middle", cur.sapi_pitch)?;
                    cur.f0 = match cur.sapi_pitch {
                        0 => 1.0,
                        l => f0_of(&format!("{l:+}st"), 1.0).ok().flatten().unwrap_or(1.0),
                    };
                    if tag.attr("range").is_some() || tag.attr("absrange").is_some() {
                        ignore(&mut m, "pitch range");
                    }
                }
                // SwiftTTSEngine writes <silence msec="N"/> as <break time="Nms" />
                "silence" => match tag.attr("msec") {
                    Some(ms) => {
                        let t = format!("{}ms", ms.trim());
                        if let Some(b) = break_of(Some(&t), None, None) {
                            m.breaks.push((chars, b));
                        }
                    }
                    None => return err("<silence> needs msec".into()),
                },
                "break" => {
                    if let Some(b) = break_of(tag.attr("time").map(str::trim),
                                              tag.attr("strength").map(str::trim),
                                              tag.attr("size").map(str::trim)) {
                        m.breaks.push((chars, b));
                    }
                }
                // The phones stand in for the element's text, which is not read.
                // SwiftTTSEngine rewrites <pron sym> as <phoneme ph></phoneme>,
                // always open and closed: swift.dll's tag table gives PHONEME no
                // empty-tag handler, so a self-closing <phoneme/> does nothing.
                "phoneme" if tag.empty => {}
                "phoneme" | "pron" => {
                    let ph = if tag.name == "pron" {
                        tag.attr("sym").map(sapi_ph)
                    } else {
                        tag.attr("ph").map(str::to_string)
                    };
                    let alphabet = if tag.name == "pron" { None } else { tag.attr("alphabet") };
                    if let Some(ph) = ph {
                        match phoneme_items(&ph, alphabet) {
                            Some(words) if !words.is_empty() => m.phonemes.push((chars, ph, words)),
                            _ => ignore(&mut m, "phoneme the engine rejects"),
                        }
                    }
                    if !tag.empty {
                        skip += 1;
                    }
                }
                "speak" | "sapi" | "s" | "p" | "voice" | "lang" | "context" | "partofsp" => {}
                "bookmark" | "mark" => {}
                "sub" | "audio" => ignore(&mut m, &tag.name),
                other => return err(format!("unknown tag <{other}>")),
            }
            if !tag.empty {
                stack.push((tag.name, before));
            } else {
                cur = before;
            }
        }
        if m.changes.last().map(|(_, c)| *c) != Some(cur) {
            match m.changes.last_mut() {
                Some(last) if last.0 == chars => last.1 = cur,
                _ => m.changes.push((chars, cur)),
            }
        }
        if m.say_as.last().map_or(None, |(_, s)| s.as_ref()) != nsw.as_ref() {
            match m.say_as.last_mut() {
                Some(last) if last.0 == chars => last.1 = nsw.clone(),
                _ => m.say_as.push((chars, nsw.clone())),
            }
        }
    }
    if let Some((open, _)) = stack.last() {
        return err(format!("<{open}> is never closed"));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sapi_and_ssml_volume_land_on_the_same_control() {
        for src in ["The quick <volume level=\"50\">fox jumps</volume> over.",
                    "<speak>The quick <prosody volume='50'>fox jumps</prosody> over.</speak>"] {
            let m = parse(src).unwrap();
            assert_eq!(m.text, "The quick fox jumps over.");
            assert_eq!(m.ctl_at(4).volume, 1.0);
            assert_eq!(m.ctl_at(10).volume, 0.5);
            assert_eq!(m.ctl_at(18).volume, 0.5);
            assert_eq!(m.ctl_at(20).volume, 1.0);
        }
    }

    #[test]
    fn nesting_restores_and_entities_decode() {
        let m = parse("a <volume level=\"20\">b <volume level=\"80\">c</volume> d</volume> &amp; e")
            .unwrap();
        assert_eq!(m.text, "a b c d & e");
        let v: Vec<f32> = [0, 2, 4, 6, 10].iter().map(|&i| m.ctl_at(i).volume).collect();
        assert_eq!(v, [1.0, 0.2, 0.8, 0.2, 1.0]);
    }

    #[test]
    fn breaks_in_both_spellings() {
        let m = parse("a <silence msec=\"500\"/> b <break time=\"1.5s\"/> c <break time='250ms'/>")
            .unwrap();
        assert_eq!(m.text, "a  b  c ");
        assert_eq!(m.breaks, [(2, Break::Pause(0.5)), (5, Break::Pause(1.5)),
                              (8, Break::Pause(0.25))]);
    }

    #[test]
    fn break_strength_follows_the_engine() {
        let b = |s: &str| parse(s).unwrap().breaks.first().map(|x| x.1);
        assert_eq!(b("a<break/>"), Some(Break::Utterance { paragraph: false }));
        assert_eq!(b("a<break strength='strong'/>"), Some(Break::Utterance { paragraph: true }));
        assert_eq!(b("a<break strength='weak'/>"), Some(Break::Phrase));
        // a time wins over any strength but none, and a bare number is no time
        assert_eq!(b("a<break strength='x-strong' time='20ms'/>"), Some(Break::Pause(0.02)));
        assert_eq!(b("a<break strength='none' time='2s'/>"), None);
        assert_eq!(b("a<break time='500'/>"), Some(Break::Utterance { paragraph: false }));
        assert_eq!(b("a<silence msec='0'/>"), Some(Break::Utterance { paragraph: false }));
    }

    #[test]
    fn rate_and_pitch_as_the_engine_reads_them() {
        let c = |s: &str| parse(s).unwrap().ctl_at(0);
        assert_eq!(c("<rate absspeed='5'>a</rate>").stretch, 1.0 / 1.75);
        assert_eq!(c("<rate absspeed='-10'>a</rate>").stretch, 1.0 / 0.38);
        assert_eq!(c("<rate absspeed='3'><rate speed='2'>a</rate></rate>").stretch, 1.0 / 1.75);
        assert_eq!(c("<prosody rate='x-slow'>a</prosody>").stretch, 2.0);
        assert_eq!(c("<prosody rate='+50%'>a</prosody>").stretch, 1.0 / 1.5);
        assert_eq!(c("<pitch absmiddle='-5'>a</pitch>").f0, 2f64.powf(-5.0 / 24.0) as f32);
        assert_eq!(c("<prosody pitch='high'>a</prosody>").f0, 2.0);
    }

    #[test]
    fn phonemes_as_the_engine_receives_them() {
        let m = parse("Say <pron sym=\"p ow 1 t ey 1 - t ow\">tomato</pron> now").unwrap();
        assert_eq!(m.text, "Say  now");
        assert_eq!(m.phonemes[0].1, "p ow1 t ey1 . t ow");
        assert_eq!(sapi_ph("h iy 1 y uw 2 _ ! l ow"), "h i1 j uw2 pau l ow");
        assert_eq!(phoneme_items("t@\"meItoU", Some("sampa")).unwrap(),
                   [["t", "ah", "1", "m", "ey", "ih", "t", "ow", "uh"]]);
        assert_eq!(phoneme_items("h eh | l ow", None).unwrap(), [["h", "eh"], ["l", "ow"]]);
        assert!(phoneme_items("h zz", None).is_none());
        assert!(phoneme_items("h eh", Some("ipa")).is_none());
    }

    #[test]
    fn emphasis_volume_words_and_say_as() {
        let c = |s: &str| parse(s).unwrap().ctl_at(0);
        let e = c("<emphasis>a</emphasis>");
        assert_eq!((e.stretch, e.volume), (1.0 / 0.75, 1.25));
        assert_eq!(c("<emphasis level='reduced'>a</emphasis>"), Ctl::default());
        // SAPI's <emph> sits outside the fragment's absolute volume and rate
        assert_eq!(c("<volume level='50'><emph>a</emph></volume>").volume, 0.5);
        assert_eq!(c("<rate absspeed='5'><emph>a</emph></rate>").stretch, 1.0 / 1.75);
        assert_eq!(c("<prosody volume='x-soft'>a</prosody>").volume, 0.25);
        assert_eq!(c("<prosody volume='50'><prosody volume='+50%'>a</prosody></prosody>").volume, 0.75);
        assert_eq!(c("<prosody volume='50%'>a</prosody>").volume, 1.0);
        let m = parse("a <say-as type='spell-out'>b <say-as interpret-as='ordinal'>1</say-as> c</say-as> d")
            .unwrap();
        let n: Vec<Option<&str>> = [0, 2, 4, 6, 8].iter().map(|&i| m.nsw_at(i)).collect();
        assert_eq!(n, [None, Some("letters"), Some("number:ordinal"), None, None]);
    }

    #[test]
    fn unsupported_tags_are_reported_not_dropped() {
        let m = parse("say <sub alias='x'>this</sub> <audio src='a.wav'>now</audio>").unwrap();
        assert_eq!(m.text, "say this now");
        assert_eq!(m.ignored, ["sub", "audio"]);
        assert!(parse("<volume level=\"50\">open").is_err());
        assert!(parse("<blink>no</blink>").is_err());
    }
}
