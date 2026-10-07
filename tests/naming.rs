//! Unit naming rules. Needs CEPS_VOICE_ROOT; skips silently otherwise.

use ceps::{NameRules, Pau, Phone, PhoneCtx, Voice};

fn voice(name: &str) -> Option<Voice> {
    let root = std::env::var("CEPS_VOICE_ROOT").ok()?;
    let d = std::path::Path::new(&root).join(name);
    if d.join("voice_u.dat").exists() {
        Voice::open(d).ok()
    } else {
        None
    }
}

fn rules(v: &Voice) -> NameRules {
    NameRules::parse(v.image(), v.unit_name_params().expect("unit_name_params"))
}

/// The engine's own output for "telephone" (recovered by _out/trace_ref.py)
/// was `teh eh1 lah ah0 f ow0 n` for /t eh1 l ah0 f ow0 n/. Consonants fuse
/// with the following vowel; the stressed vowel does not absorb the /l/,
/// because that /l/ opens the next syllable.
#[test]
fn matches_the_engines_own_naming_for_telephone() {
    let Some(v) = voice("William") else { return };
    let r = rules(&v);
    let cases: [(&str, u8, Option<&str>, bool, &str); 7] = [
        ("t", 0, Some("eh"), true, "teh"),
        ("eh", 1, Some("l"), false, "eh1"),
        ("l", 0, Some("ah"), true, "lah"),
        ("ah", 0, Some("f"), false, "ah0"),
        ("f", 0, Some("ow"), false, "f"),
        ("ow", 0, Some("n"), false, "ow0"),
        ("n", 0, None, false, "n"),
    ];
    for (phone, stress, next, in_syl, want) in cases {
        let got = r.unit_name(&PhoneCtx {
            phone, stress, next, prev: None, next_in_syllable: in_syl,
            prev_in_syllable: false, word: None, pau: None,
        });
        assert_eq!(got, want, "{phone} (stress {stress}, next {next:?})");
        assert!(v.type_id(&got).is_some(), "{got} is not a type in this voice");
    }
}

/// Same vowel, same follower, different syllable membership.
#[test]
fn vowel_fusion_needs_the_follower_in_the_same_syllable() {
    let Some(v) = voice("Allison") else { return };
    let r = rules(&v);
    let mk = |in_syl| r.unit_name(&PhoneCtx {
        phone: "eh", stress: 1, next: Some("l"), prev: None,
        next_in_syllable: in_syl, prev_in_syllable: false, word: None, pau: None,
    });
    assert_eq!(mk(false), "eh1");
    assert_eq!(mk(true), "eh1l");
    assert!(v.type_id("eh1").is_some() && v.type_id("eh1l").is_some());
}

#[test]
fn function_words_and_nasal_context() {
    let Some(v) = voice("Allison") else { return };
    let r = rules(&v);
    let name = |phone, next, prev, word, in_syl| r.unit_name(&PhoneCtx {
        phone, stress: 0, next, prev, next_in_syllable: in_syl, prev_in_syllable: true,
        word, pau: None,
    });
    // dlist keys on the phone BEFORE, not after: the engine's output for
    // "during the outage" is `ng dhthe_ng i1the`, where the `ng` ending the
    // previous word is what marks the `dh`.
    assert_eq!(name("dh", None, None, Some("the"), false), "dhthe");
    assert_eq!(name("dh", None, Some("n"), None, false), "dh_n");
    assert_eq!(name("dh", Some("i"), Some("ng"), Some("the"), false), "dhthe_ng");
    assert_eq!(name("ah", None, None, Some("the"), false), "ah0the");
    for n in ["dhthe", "dh_n", "dhthe_ng", "ah0the"] {
        assert!(v.type_id(n).is_some(), "{n} missing from the voice");
    }
}

#[test]
fn pauses_and_stop_releases() {
    let Some(v) = voice("Allison") else { return };
    let r = rules(&v);
    for (p, want) in [(Pau::Start, "paustart"), (Pau::Mid, "paumid"), (Pau::End, "pauend")] {
        let got = r.unit_name(&PhoneCtx { phone: "pau", pau: Some(p), ..Default::default() });
        assert_eq!(got, want);
        assert!(v.type_id(want).is_some());
    }
    // An unvoiced stop *after* /s/ in the same syllable becomes <stop>S, which
    // is what ceplang_en.dll @0x4d95c asks for: `R:SylStructure.p.name == "s"`.
    // A stop opening a syllable after a coda /s/ does not qualify, and neither
    // does one that merely precedes an /s/.
    let sclust = |prev, in_syl| r.unit_name(&PhoneCtx {
        phone: "t", stress: 0, next: Some("ao"), prev, next_in_syllable: true,
        prev_in_syllable: in_syl, word: None, pau: None,
    });
    assert_eq!(sclust(Some("s"), true), "tS");
    assert_eq!(sclust(Some("s"), false), "tao");
    assert_eq!(sclust(Some("ah"), true), "tao");
    assert!(v.type_id("tS").is_some());
}

/// Every type the voice actually has must be reachable from the rules.
#[test]
fn rules_cover_every_type() {
    for name in ["Allison", "David", "William", "Jean-Pierre"] {
        let Some(v) = voice(name) else { continue };
        let r = rules(&v);
        let phones: Vec<String> = v.types.iter().map(|t| t.name.clone()).collect();
        let gen = r.generate(&phones);
        let missing: Vec<&str> = v.types.iter()
            .map(|t| t.name.as_str())
            .filter(|n| !gen.contains(*n))
            .collect();
        assert!(missing.is_empty(), "{name}: unreachable type names {missing:?}");
    }
}

fn seq(spec: &[(&str, u8)]) -> Vec<Phone> {
    spec.iter().map(|(p, s)| Phone::new(p, *s)).collect()
}

/// name_utterance must reproduce the engine's own naming of "telephone" from
/// nothing but the phone string -- no hand-supplied syllable flags.
#[test]
fn maximal_onset_reproduces_the_engine_naming() {
    let Some(v) = voice("William") else { return };
    let r = rules(&v);
    let got = r.name_utterance(
        &seq(&[("t", 0), ("eh", 1), ("l", 0), ("ah", 0), ("f", 0), ("ow", 0), ("n", 0)]),
        |n| v.type_id(n).is_some(),
    );
    assert_eq!(got, ["teh", "eh1", "lah", "ah0", "f", "ow0", "n"]);
}

/// /h eh1 l ow1/ + /w er1 l d/. Every onset consonant takes its vowel; every
/// stressed vowel releases the consonant that opens the next syllable.
#[test]
fn hello_world_names_into_real_types() {
    for name in ["Allison", "William"] {
        let Some(v) = voice(name) else { continue };
        let r = rules(&v);
        let got = r.name_utterance(
            &seq(&[("pau", 0), ("h", 0), ("eh", 1), ("l", 0), ("ow", 1),
                   ("w", 0), ("er", 1), ("l", 0), ("d", 0), ("pau", 0)]),
            |n| v.type_id(n).is_some(),
        );
        assert_eq!(got,
                   ["paustart", "heh", "eh1", "low", "ow1", "wer", "er1", "l", "d", "pauend"],
                   "{name}");
        for n in &got {
            assert!(v.type_id(n).is_some(), "{name}: {n} is not a type");
        }
    }
}

/// The recorded inventory is sparse. `f` before `ow` would fuse to `fow`, which
/// no voice has, so naming must fall back rather than emit a dead type.
#[test]
fn missing_fused_types_fall_back_to_the_bare_phone() {
    let Some(v) = voice("William") else { return };
    let r = rules(&v);
    assert!(v.type_id("fah").is_some(), "fah should exist");
    assert!(v.type_id("fow").is_none(), "fow should not exist");

    let unconstrained = r.name_utterance(&seq(&[("f", 0), ("ow", 0)]), |_| true);
    let constrained = r.name_utterance(&seq(&[("f", 0), ("ow", 0)]),
                                       |n| v.type_id(n).is_some());
    assert!(v.type_id(&constrained[0]).is_some(),
            "fallback produced a dead type: {} (unconstrained {})",
            constrained[0], unconstrained[0]);
}

/// Every name the rules produce for a plausible phone string must be a real type.
/// This is the guard that keeps the front end honest: a dead type name would fail
/// selection at runtime rather than at build time.
#[test]
fn naming_never_emits_a_type_the_voice_lacks() {
    let cons = ["b", "d", "f", "g", "k", "l", "m", "n", "p", "r", "s", "t", "v", "w", "z"];
    for name in ["Allison", "David", "William"] {
        let Some(v) = voice(name) else { continue };
        let r = rules(&v);
        let mut vowels: Vec<String> = r.stress_vowels.iter().cloned().collect();
        vowels.sort();
        let mut checked = 0usize;
        for c in cons {
            for vw in &vowels {
                let got = r.name_utterance(
                    &seq(&[("pau", 0), (c, 0), (vw, 1), ("t", 0), ("pau", 0)]),
                    |n| v.type_id(n).is_some(),
                );
                for n in &got {
                    assert!(v.type_id(n).is_some(),
                            "{name}: /{c} {vw}1 t/ produced {n}, not a type");
                }
                checked += 1;
            }
        }
        assert!(checked > 100, "{name}: only {checked} combinations exercised");
    }
}

/// Jean-Pierre is not built like the English voices. It carries 56 unit types
/// against their 348, an empty `stressvowels` list and an empty `fwords` list, so
/// no demi-syllable fusion happens at all -- every unit is a plain phone. Any
/// front end must branch on this rather than assume the English scheme.
#[test]
fn jean_pierre_is_a_plain_phone_voice() {
    let Some(jp) = voice("Jean-Pierre") else { return };
    let r = rules(&jp);
    assert!(r.stress_vowels.is_empty(), "expected no stress vowels, got {:?}", r.stress_vowels);
    assert!(jp.types.len() < 100, "expected a small inventory, got {}", jp.types.len());

    // with no vowel list, naming can only ever return the phone itself
    for p in ["a", "e", "i", "o", "u", "t", "l"] {
        let got = r.name_utterance(&seq(&[(p, 1), ("t", 0)]), |_| true);
        assert_eq!(got[0], p, "{p} must not fuse in a plain-phone voice");
    }

    if let Some(w) = voice("William") {
        assert!(w.types.len() > 4 * jp.types.len(),
                "the English voices should be far richer: {} vs {}",
                w.types.len(), jp.types.len());
    }
}

fn worded(spec: &[(&str, u8, &str)]) -> Vec<Phone> {
    // word_id counts occurrences, so a repeated spelling still reads as two
    // words; these specs never repeat one, so the running count is enough.
    let mut id = 0u32;
    let mut last = "";
    spec.iter()
        .map(|(p, st, w)| {
            if *w != last {
                id += 1;
                last = w;
            }
            Phone {
                phone: p.to_string(),
                stress: *st,
                word: Some(w.to_string()),
                word_id: id,
                syl_end: None,
                dur: None,
            }
        })
        .collect()
}

/// A syllable never spans two words, and neither does naming context.
///
/// "civil authority" is the case that exposed this. Unit naming fuses a
/// consonant with a following vowel unconditionally -- `next_in_syllable` only
/// gates the vowel case -- so with unbounded context the /l/ closing "civil"
/// became `lao`, taking the /ao/ that opens "authority". The engine emits
/// `ah0l l ao0`, treating that /l/ as a coda. Withholding the cross-word
/// follower reproduces it.
#[test]
fn naming_context_stops_at_the_word_edge() {
    let Some(v) = voice("William") else { return };
    let r = rules(&v);

    // /s ih1 v ah0 l/ + /ao0 th ao1 r ih0 t i0/
    let ph = worded(&[
        ("s", 1, "civil"), ("ih", 1, "civil"), ("v", 0, "civil"),
        ("ah", 0, "civil"), ("l", 0, "civil"),
        ("ao", 0, "authority"), ("th", 0, "authority"), ("ao", 1, "authority"),
    ]);
    let got = r.name_utterance(&ph, |n| v.type_id(n).is_some());
    assert_eq!(got[..6], ["s", "ih1", "vah", "ah0l", "l", "ao0"],
               "got {got:?}");
    assert_ne!(got[4], "lao", "the /l/ closing civil must not take authority's vowel");

    // inside one word the same consonant does fuse
    let one = worded(&[("l", 0, "lao"), ("ao", 1, "lao")]);
    let got2 = r.name_utterance(&one, |n| v.type_id(n).is_some());
    assert_eq!(got2[0], "lao", "within a word the onset still fuses");
}

/// The vowel case needs the same bound: whether a consonant is a coda or the
/// next onset can only be decided by what follows it *in its own word*.
#[test]
fn coda_decision_ignores_the_next_word() {
    let Some(v) = voice("William") else { return };
    let r = rules(&v);

    // /ah/ /l/ where the next vowel belongs to another word: /l/ is a coda
    let across = worded(&[
        ("ah", 0, "civil"), ("l", 0, "civil"), ("ao", 0, "authority"),
    ]);
    assert!(r.next_in_syllable(&across, 0),
            "the vowel must keep a word-final consonant");

    // the same shape inside one word: /l/ opens the next syllable
    let within = worded(&[
        ("ah", 0, "salami"), ("l", 0, "salami"), ("aa", 1, "salami"),
    ]);
    assert!(!r.next_in_syllable(&within, 0),
            "within a word the consonant opens the next syllable");
}

/// A sentence boundary is TWO pause units, `pauend` then `paustart`.
///
/// The engine's own output for "…normally. Thank you…" is
/// `li i0 pauend paustart thae ae1ng`. Emitting a single `paumid` there shifts
/// every later unit by one position, which showed up as type agreement
/// collapsing from 50/52 to 21/51 -- an alignment artifact, not a front-end
/// failure, and exactly the kind of thing that hides a good result.
#[test]
fn sentence_boundaries_take_two_pause_units() {
    let Some(v) = voice("William") else { return };
    let r = rules(&v);

    let ph = seq(&[("pau", 0), ("k", 0), ("ae", 1), ("t", 0),
                   ("pau", 0), ("pau", 0),
                   ("d", 0), ("aa", 1), ("g", 0), ("pau", 0)]);
    let got = r.name_utterance(&ph, |n| v.type_id(n).is_some());
    assert_eq!(got[0], "paustart");
    assert_eq!(got[4], "pauend", "the first of a pair closes the utterance");
    assert_eq!(got[5], "paustart", "the second opens the next");
    assert_eq!(got[9], "pauend");

    // a lone interior pause is still a mid pause
    let one = seq(&[("pau", 0), ("k", 0), ("pau", 0), ("d", 0), ("pau", 0)]);
    let g2 = r.name_utterance(&one, |n| v.type_id(n).is_some());
    assert_eq!(g2[2], "paumid", "a single interior pause is not a boundary");
}
