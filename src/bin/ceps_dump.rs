//! cep6 <voice-directory> <command> [args]
//!
//! Commands:
//!   info
//!   types  [substring]
//!   frames <first> <count> <out.wav>
//!   unit   <index> <out.wav>
//!   chain  <first_unit> <out.wav>
//!   say    "type1 type2 ..." <out.wav>
//!   speak  <score.txt> <out.wav>                             # prosody path (f0 + durations)
//!   pace   "type1 type2 ..." <secs/unit> <f0> <out.wav>
//!   pipeline "pau h eh:1 l ow:1 pau" <out.wav>               # full chain
//!   text   "some words" <out.wav>                            # the whole front end
//!   file   <in.txt> <out.wav>                                # the same, from a file
//!   names / target-cost / dur / f0 / self-test               # model dumps
//!   bench  [frames]
//!
//! text and file also take:
//!   --markup      read SAPI XML or Swift SSML: volume, breaks (time and
//!                 strength), rate, pitch, phonemes, emphasis and say-as act
//!                 (<volume>, <rate>, <pitch>, <silence>, <pron sym>, <emph>,
//!                 <spell>, <bookmark>, <prosody volume|rate|pitch>, <break>,
//!                 <phoneme ph alphabet="cepstral_en|sampa">, <emphasis level>,
//!                 <say-as type|interpret-as format>, <mark>)
//!   --volume N    the port volume, a percentage (what SpVoice.Volume sets)
//!   --rate N      SpVoice.Rate, -10..10
//!   --wpm N       the engine's speech/rate directly (170 is normal)
//!   --pitch-shift F   speech/pitch/shift, a multiplier
//!
//! `file` is the command to use for any long inputs. A shell caps how much text can
//! go in an argument with text -- Windows around 32 KB, and a long book is megabytes,
//! and `file` also streams its audio to disk an utterance at a time instead of
//! holding all of it, so input length is limited by nothing in particular.

use ceps::db::UNIT_NONE;
use ceps::{synth, wav, Voice};
use std::time::Instant;


/// Phones for `text`, from Cepstral's own lexicon and LTS when the pack is
/// available. The festival packs are a visibly different front end, so which one
/// is in use is worth printing rather than inferring.
fn front_end(text: &str, style: ceps::DigitStyle, force_festival: bool)
    -> (Vec<ceps::Phone>, &'static str)
{
    if !force_festival {
        if let Some(c) = load_pack("CEPS_CEPLEX", "ceplex.bin", EMBEDDED_CEPLEX)
            .and_then(|(b, _)| ceps::CepLex::from_bytes(b))
        {
            let (ph, _, missing) = ceps::text_to_phones_cepstral(&c, text, style);
            if !missing.is_empty() {
                println!("  unpronounceable: {}", missing.join(" "));
            }
            return (ph, "Cepstral");
        }
    }
    let lexpath = std::env::var("CEPS_LEXICON")
        .unwrap_or_else(|_| r"..\..\packs\cmulex.bin".to_string());
    let ltspath = std::env::var("CEPS_LTS")
        .unwrap_or_else(|_| r"..\..\packs\cmults.bin".to_string());
    let lex = ceps::Lexicon::open(&lexpath).expect("lexicon");
    let lts = ceps::Lts::open(&ltspath).ok();
    let (ph, _, missing) = ceps::text_to_phones_styled(&lex, lts.as_ref(), text, style);
    if !missing.is_empty() {
        println!("  unpronounceable: {}", missing.join(" "));
    }
    (ph, "festival")
}

/// Where to look for a data pack when its environment variable is unset: beside
/// the executable first, so a copy shipped to someone else needs no configuration,
/// then the working directory, then the development tree.
/// Say so when this build's target model does not match the voice, rather than
/// rendering something subtly wrong and leaving it to be noticed by ear.
///
/// Swift 4 and 5 voices used to trip this, from `phonedist` being parsed as a
/// `cond` clause rather than as the registered function it is; they now score 0
/// like 6.2 does. It stays because it is the one check that runs on every voice
/// before a word is rendered, and it is how that bug would have been caught
/// years earlier than by ear.
fn warn_if_voice_mismatched(v: &ceps::Voice) {
    let Some(unp) = v.unit_name_params() else { return };
    let Some(tc) = ceps::TargetCost::parse(v.image(), unp) else { return };
    let sc = tc.self_cost(v, 16);
    if sc.is_exact() {
        return;
    }
    eprintln!("warning: this voice's target model is not the one this build \
               implements (self-cost {} over {} units, expected 0)", sc.worst, sc.probed);
    if !sc.unimplemented.is_empty() {
        eprintln!("         features with no direct implementation: {}",
                  sc.unimplemented.join(", "));
    }
    eprintln!("         selection will not match the engine; timing is the \
               usual symptom. See HANDOFF \"What is left\" item 7.");
}

/// The lexicon and the tagger, built into the binary under `embed-packs`, so a
/// copy handed to someone else is one file plus their own voice directory.
#[cfg(feature = "embed-packs")]
const EMBEDDED_CEPLEX: &[u8] = include_bytes!("../../packs/ceplex.bin");
#[cfg(feature = "embed-packs")]
const EMBEDDED_POS: &[u8] = include_bytes!("../../packs/pos.bin");
#[cfg(not(feature = "embed-packs"))]
const EMBEDDED_CEPLEX: &[u8] = &[];
#[cfg(not(feature = "embed-packs"))]
const EMBEDDED_POS: &[u8] = &[];

/// A pack from disk if there is one, else the built-in copy.
///
/// An explicit environment variable always wins, so a newer `ceplex.bin` can be
/// dropped in without rebuilding; then a file beside the executable; then what
/// is compiled in. Returns the bytes and where they came from.
fn load_pack(var: &str, name: &str, embedded: &'static [u8]) -> Option<(Vec<u8>, String)> {
    if let Ok(p) = std::env::var(var) {
        return match std::fs::read(&p) {
            Ok(b) => Some((b, p)),
            Err(e) => {
                eprintln!("{var} is set to {p} but it cannot be read: {e}");
                std::process::exit(1);
            }
        };
    }
    let p = default_path(name);
    if let Ok(b) = std::fs::read(&p) {
        return Some((b, p));
    }
    if embedded.is_empty() {
        None
    } else {
        Some((embedded.to_vec(), format!("built in, {} KB", embedded.len() / 1024)))
    }
}

/// `--markup`: read the input as SAPI XML or Swift SSML.
fn markup_flag(a: &[String], text: &str) -> Option<ceps::Marked> {
    if !a.iter().any(|x| x == "--markup") {
        return None;
    }
    let m = ceps::markup::parse(text).unwrap_or_else(|e| {
        eprintln!("markup: {e}");
        std::process::exit(1);
    });
    if !m.ignored.is_empty() {
        println!("markup: not implemented yet, text kept: {}", m.ignored.join(", "));
    }
    Some(m)
}

/// `--volume N`: the port's audio/volume, a percentage.
fn volume_flag(a: &[String]) -> u32 {
    match a.iter().position(|x| x == "--volume") {
        None => 100,
        Some(i) => a.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
            eprintln!("--volume wants a whole percentage, e.g. --volume 50");
            std::process::exit(1);
        }),
    }
}

/// `--rate N` (SpVoice.Rate, -10..10), `--wpm N` (the engine's speech/rate)
/// and `--pitch-shift F` (speech/pitch/shift).
fn port_flags(a: &[String]) -> ceps::speak::Port {
    let val = |k: &str| a.iter().position(|x| x == k).map(|i| {
        a.get(i + 1).cloned().unwrap_or_else(|| {
            eprintln!("{k} wants a value");
            std::process::exit(1);
        })
    });
    let bad = |k: &str, v: &str| -> ! {
        eprintln!("{k} {v}: not a number");
        std::process::exit(1);
    };
    let mut port = ceps::speak::Port::default();
    if let Some(v) = val("--rate") {
        let r: i32 = v.parse().unwrap_or_else(|_| bad("--rate", &v));
        port = port.with_wpm(ceps::speak::Port::sapi_wpm(r));
    }
    if let Some(v) = val("--wpm") {
        port = port.with_wpm(v.parse().unwrap_or_else(|_| bad("--wpm", &v)));
    }
    if let Some(v) = val("--pitch-shift") {
        port.f0_shift = v.parse().unwrap_or_else(|_| bad("--pitch-shift", &v));
    }
    port
}

fn default_path(name: &str) -> String {
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        let p = dir.join(name);
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }
    if std::path::Path::new(name).exists() {
        return name.to_string();
    }
    format!(r"..\..\packs\{name}")
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let help = a.iter().skip(1).any(|x| x == "--help" || x == "-h");
    if help || a.len() < 3 {
        eprintln!("{}", include_str!("ceps_dump.rs").lines()
            .take_while(|l| l.starts_with("//!"))
            .map(|l| l.trim_start_matches("//!").trim_end())
            .collect::<Vec<_>>().join("\n"));
        std::process::exit(if help { 0 } else { 1 });
    }
    let t0 = Instant::now();
    let v = match Voice::open(&a[1]) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("cannot open voice {}: {e}", a[1]);
            std::process::exit(1);
        }
    };
    eprintln!("loaded {:.1} MB in {:.2?}", v.total_bytes() as f64 / 1e6, t0.elapsed());

    match a[2].as_str() {
        "info" => info(&v),
        "types" => {
            let filt = a.get(3).map(|s| s.as_str()).unwrap_or("");
            for (i, t) in v.types.iter().enumerate() {
                if filt.is_empty() || t.name.contains(filt) {
                    println!("{i:4}  {:<12} start={:<7} count={}", t.name, t.start, t.count);
                }
            }
        }
        "frames" => {
            let first: usize = a[3].parse().unwrap();
            let count: usize = a[4].parse().unwrap();
            let t = Instant::now();
            let pcm = synth::synth_frames(&v, first, first + count);
            report(&v, &pcm, t.elapsed());
            wav::write(&a[5], &pcm, v.sps).unwrap();
            println!("wrote {}", a[5]);
        }
        "unit" => {
            let i: usize = a[3].parse().unwrap();
            let u = v.unit(i);
            println!("unit {i} type={} frames [{},{}) prev={} next={}",
                     v.types[u.type_id as usize].name, u.start, u.end, u.prev, u.next);
            let pcm = synth::synth_units(&v, &[i]);
            wav::write(&a[4], &pcm, v.sps).unwrap();
            println!("wrote {}", a[4]);
        }
        "chain" => {
            let first: usize = a[3].parse().unwrap();
            let ch = v.chain(first, 10_000);
            let names: Vec<&str> = ch.iter()
                .map(|&u| v.types[v.unit(u).type_id as usize].name.as_str()).collect();
            println!("{} units: {}", ch.len(), names.join(" "));
            let t = Instant::now();
            let pcm = synth::synth_units(&v, &ch);
            report(&v, &pcm, t.elapsed());
            wav::write(&a[4], &pcm, v.sps).unwrap();
            println!("wrote {}", a[4]);
        }
        "longest" => {
            let mut best = (0usize, 0usize);
            for u in 0..v.num_units {
                if v.unit(u).prev != UNIT_NONE {
                    continue;
                }
                let n = v.chain(u, 10_000).len();
                if n > best.1 {
                    best = (u, n);
                }
            }
            println!("longest chain: unit {} ({} units)", best.0, best.1);
        }
        "say" => {
            let names: Vec<&str> = a[3].split_whitespace().collect();
            let mut units = Vec::new();
            for n in &names {
                match v.type_id(n) {
                    Some(t) => units.push(v.candidates(t).start),
                    None => {
                        eprintln!("unknown type {n}");
                        std::process::exit(1);
                    }
                }
            }
            let pcm = synth::synth_units(&v, &units);
            wav::write(&a[4], &pcm, v.sps).unwrap();
            println!("wrote {}", a[4]);
        }
        "speak" | "pace" => {
            let sc = if a[2] == "speak" {
                let text = std::fs::read_to_string(&a[3]).unwrap_or_else(|e| {
                    eprintln!("cannot read score {}: {e}", a[3]);
                    std::process::exit(1);
                });
                ceps::score::parse(&v, &text)
            } else {
                let names: Vec<&str> = a[3].split_whitespace().collect();
                ceps::score::flat(&v, &names, a[4].parse().unwrap(), a[5].parse().unwrap())
            };
            let mut sc = sc.unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let outp = if a[2] == "speak" { &a[4] } else { &a[6] };
            let do_select = a.iter().any(|x| x == "--select");

            if do_select {
                if !sc.selectable {
                    eprintln!("--select needs every unit named by type, not pinned with #index");
                    std::process::exit(1);
                }
                let mut p = ceps::SelectParams::from_voice(&v);
                if let Some(i) = a.iter().position(|x| x == "--beam") {
                    p.beam = a[i + 1].parse().unwrap_or(p.beam);
                }
                if let Some(i) = a.iter().position(|x| x == "--target-weight") {
                    p.target_weight = a[i + 1].parse().unwrap_or(p.target_weight);
                }
                println!("selecting: optimal_coupling={} extend_selections={} f0_weight={} beam={} target_weight={}",
                         p.optimal_coupling, p.extend_selections, p.f0_weight, p.beam, p.target_weight);
                let t = Instant::now();
                let mut sel = ceps::Selector::new(&v, p);
                let chosen = sel.select(&sc.types);
                println!("selected {} units in {:.3?} ({} join costs evaluated)",
                         chosen.len(), t.elapsed(), sel.joins_evaluated);

                let mut natural = 0;
                for w in chosen.windows(2) {
                    if v.unit(w[0].unit).next == w[1].unit as i32 {
                        natural += 1;
                    }
                }
                println!("naturally consecutive joins: {natural}/{}", chosen.len().saturating_sub(1));

                for (i, c) in chosen.iter().enumerate() {
                    sc.units[i].start = c.start;
                    sc.units[i].end = c.end;
                    sc.names[i] = format!("{}#{}", v.types[v.unit(c.unit).type_id as usize].name, c.unit);
                }
            }

            let t = Instant::now();
            let pms = ceps::f0_targets_to_pm(&v, &sc.targets, sc.lead_f0);
            let periods = ceps::concat_units(&v, &sc.units, &pms);
            let pcm = ceps::synth_periods(&v, &periods);
            let dt = t.elapsed();

            println!("{} units, {} pitch marks, {} periods emitted",
                     sc.units.len(), pms.len(), periods.len());
            let stretched = periods.iter()
                .filter(|p| p.size != v.frame_size(p.src_frame)).count();
            println!("periods resized by concat_units: {stretched}/{} ({:.0}%)",
                     periods.len(),
                     100.0 * stretched as f64 / periods.len().max(1) as f64);

            // how hard is add_residual having to work?
            let mut ratios: Vec<f64> = periods.iter()
                .map(|p| p.size as f64 / v.frame_size(p.src_frame).max(1) as f64)
                .collect();
            ratios.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let pct = |q: f64| ratios[((ratios.len() - 1) as f64 * q) as usize];
            let padded: usize = periods.iter()
                .map(|p| p.size.saturating_sub(v.frame_size(p.src_frame))).sum();
            let total: usize = periods.iter().map(|p| p.size).sum();
            println!("targ/src period ratio: p10 {:.2}  median {:.2}  p90 {:.2}",
                     pct(0.10), pct(0.50), pct(0.90));
            println!("excitation that is inserted silence: {:.1}%",
                     100.0 * padded as f64 / total.max(1) as f64);

            // frame reuse: repeating one pitch period is what buzzes
            let mut runs = Vec::new();
            let mut i = 0;
            while i < periods.len() {
                let mut j = i;
                while j + 1 < periods.len() && periods[j + 1].src_frame == periods[i].src_frame {
                    j += 1;
                }
                runs.push(j - i + 1);
                i = j + 1;
            }
            let distinct = runs.len();
            let worst = runs.iter().copied().max().unwrap_or(0);
            println!("distinct source frames: {distinct}/{} periods, longest repeat run {worst}",
                     periods.len());
            println!("units: {}", sc.names.join(" "));
            report(&v, &pcm, dt);
            wav::write(outp, &pcm, v.sps).unwrap();
            println!("wrote {outp}");
        }
        "tcost" => {
            let unp = v.unit_name_params().expect("no unit_name_params");
            let tc = ceps::TargetCost::parse(v.image(), unp).expect("parse target_cost");
            println!("{} features, {} env slots, cand_beam_width {}",
                     tc.features.len(), tc.num_slots(), tc.beam_width);
            println!("voice_d.dat width is {} -> {}", v.unit_feat_width,
                     if tc.features.len() == v.unit_feat_width { "matches" } else { "MISMATCH" });
            if !tc.missing.is_empty() {
                println!("names in the expression with no unit_features entry: {:?}", tc.missing);
            }
            // value histogram per column, so target vectors can be built in the
            // encoding the database actually uses
            let mut hist: Vec<std::collections::BTreeMap<u8, usize>> =
                vec![Default::default(); tc.features.len()];
            for u in 0..v.num_units {
                for (k, &x) in v.unit_feats(u).iter().enumerate() {
                    *hist[k].entry(x).or_insert(0) += 1;
                }
            }
            for (k, f) in tc.features.iter().enumerate() {
                let h = &hist[k];
                let mut vals: Vec<(u8, usize)> = h.iter().map(|(&a, &b)| (a, b)).collect();
                vals.sort_by(|a, b| b.1.cmp(&a.1));
                let shown: Vec<String> = vals.iter().take(6)
                    .map(|(x, n)| format!("{x}:{:.0}%", 100.0 * *n as f64 / v.num_units as f64))
                    .collect();
                println!("  {k:2}  {f:<46} {} distinct  {}",
                         h.len(), shown.join(" "));
            }

            // Score a unit against its own recorded features: every diff is 0.
            let mut env = vec![0.0f32; tc.num_slots()];
            let probe: Vec<usize> = (0..v.num_units).step_by(v.num_units / 6).take(6).collect();
            println!("\nself-cost (target == the unit's own features), should be minimal:");
            for &u in &probe {
                let own: Vec<i32> = v.unit_feats(u).iter().map(|&x| x as i32).collect();
                let self_cost = tc.score(&mut env, &own, v.unit_feats(u));
                // and against a different unit of the same type
                let t = v.unit(u).type_id as usize;
                let r = v.candidates(t);
                let other = if r.start == u { r.end - 1 } else { r.start };
                let other_cost = tc.score(&mut env, &own, v.unit_feats(other));
                println!("  unit {u:<7} type {:<12} self {self_cost:<8} vs other unit {other:<7} {other_cost}",
                         v.types[t].name);
            }
        }
        // Which slots the expression actually reads. self-cost passes the same
        // vector as target and candidate, so every Diff cancels and only a Targ
        // or a Cand can leave a nonzero score behind.
        "expr" => {
            let unp = v.unit_name_params().expect("no unit_name_params");
            let tc = ceps::TargetCost::parse(v.image(), unp).expect("no target cost");
            println!("{} features, {} slots", tc.features.len(), tc.num_slots());
            if !tc.missing.is_empty() {
                println!("missing from unit_features: {:?}", tc.missing);
            }
            let mut kinds = std::collections::BTreeMap::new();
            for (i, s) in tc.slots.iter().enumerate() {
                let (kind, k) = match *s {
                    ceps::expr::Slot::Diff(k) => ("diff", k),
                    ceps::expr::Slot::Targ(k) => ("targ", k),
                    ceps::expr::Slot::Cand(k) => ("cand", k),
                };
                *kinds.entry(kind).or_insert(0usize) += 1;
                let name = tc.features.get(k).map(|s| s.as_str()).unwrap_or("?");
                println!("  slot {i:3}  {kind} {k:2}  {name}");
            }
            println!("\nslot kinds: {kinds:?}");
            println!("\n{:?}", tc.expr);
        }
        "names" => {
            let unp = v.unit_name_params().expect("no unit_name_params");
            let r = ceps::NameRules::parse(v.image(), unp);
            println!("stressvowels   {:?}", {
                let mut x: Vec<_> = r.stress_vowels.iter().cloned().collect();
                x.sort();
                x
            });
            println!("fwords         {:?}", r.fwords);
            println!("unvoicedstops  {:?}", r.unvoiced_stops);
            println!("palatalizables {:?}", r.palatalizables);
            println!("dlist          {:?}", r.dlist);
            println!("blist          {} heads", r.blist.len());

            // coverage: can the rules name every type this voice actually has?
            let phones: Vec<String> = v.types.iter().map(|t| t.name.clone()).collect();
            let gen = r.generate(&phones);
            let have: Vec<&str> = v.types.iter().map(|t| t.name.as_str()).collect();
            let miss: Vec<&&str> = have.iter().filter(|n| !gen.contains(**n)).collect();
            println!("\ngenerated {} names; covers {}/{} actual types ({:.1}%)",
                     gen.len(), have.len() - miss.len(), have.len(),
                     100.0 * (have.len() - miss.len()) as f64 / have.len() as f64);
            if !miss.is_empty() {
                println!("unreachable: {:?}", miss);
            }
        }
        "dur" => {
            let blob = v.dur_cart().expect("no dur_cart");
            let c = ceps::Cart::parse(v.image(), blob);
            println!("dur_cart: {} nodes, {} features", c.num_nodes, c.feats.len());
            for (i, f) in c.feats.iter().enumerate() {
                println!("  feat {i:2}  {f}");
            }
            let ds = ceps::DurStats::parse(
                v.image(), v.dur_stats().expect("no dur_stats"), 256);
            println!("\ndur_stats: {} phones (seconds)", ds.stats.len());
            for st in ds.stats.iter().take(8) {
                println!("  {:<6} mean {:.4}  sd {:.4}", st.phone, st.mean, st.stddev);
            }

            // Interpret for a handful of made-up contexts.
            println!("\npredicted durations:");
            for (phone, brk, stress) in [("pau", "BB", "0"), ("pau", "NB", "0"),
                                         ("ah", "NB", "1"), ("s", "NB", "0"),
                                         ("ay", "NB", "1"), ("t", "NB", "0")] {
                let f = |name: &str| -> ceps::FeatVal {
                    match name {
                        "name" => ceps::FeatVal::Str(phone.into()),
                        "R:SylStructure.parent.stress" => ceps::FeatVal::Str(stress.into()),
                        "p.R:SylStructure.parent.parent.pbreak" => ceps::FeatVal::Str(brk.into()),
                        "R:SylStructure.parent.syl_break" => ceps::FeatVal::Str("0".into()),
                        "n.name" | "p.name" => ceps::FeatVal::Str("ax".into()),
                        _ => ceps::FeatVal::Num(0.0),
                    }
                };
                let z = c.interpret(&f).as_num();
                match ds.get(phone) {
                    Some(st) => println!("  {phone:<5} pbreak={brk:<3} stress={stress}  \
z={z:>7.3}  -> {:.4} s", ceps::cart::segment_duration(z, st, 1.0)),
                    None => println!("  {phone:<5} no dur_stats entry"),
                }
            }
        }
        "f0" => {
            let nat = ceps::natural_f0(&v, 40_000);
            let dflt = ceps::FlatProsody::default();
            let tuned = ceps::FlatProsody::for_voice(&v);
            println!("recorded F0 (median pitch period): {nat:.0} Hz");
            println!("settings.txt says PROSODY \"none\": the engine applies no F0");
            println!("model at all. Everything below is what flat_prosody WOULD do,");
            println!("and is only reachable through `pipeline --f0` or a score file.");
            println!("engine default flat_prosody:  mean {:.0}  stddev {:.0}  shift {:.1}",
                     dflt.mean, dflt.stddev, dflt.shift);
            for (label, fp) in [("engine default", dflt), ("centred on this voice", tuned)] {
                let t = fp.targets(2.0);
                println!("  {label:<22} targets: {:.0} Hz at 0.0s -> {:.0} Hz at 2.0s",
                         t[0].f0, t[1].f0);
                // how badly would that stretch this voice's periods?
                let want = v.sps as f32 / (fp.mean * fp.shift);
                let have = v.sps as f32 / nat;
                println!("  {:<22} period {:.0} vs recorded {:.0} samples -> {:.2}x stretch",
                         "", want, have, want / have);
            }
        }
        "pipeline" => {
            // phone sequence -> naming -> duration -> F0 -> selection -> audio.
            // Input: "pau hh eh:1 l ow:1 pau", ":n" is stress.
            let spec: Vec<&str> = a[3].split_whitespace().collect();
            let outp = &a[4];
            let phones: Vec<(String, u8)> = spec.iter().map(|t| {
                let (p, st) = t.split_once(':').unwrap_or((t, "0"));
                (p.to_string(), st.parse().unwrap_or(0))
            }).collect();

            let unp = v.unit_name_params().expect("unit_name_params");
            let nr = ceps::NameRules::parse(v.image(), unp);
            let tc = ceps::TargetCost::parse(v.image(), unp).expect("target_cost");
            let dc = ceps::Cart::parse(v.image(), v.dur_cart().expect("dur_cart"));
            let ds = ceps::DurStats::parse(v.image(), v.dur_stats().expect("dur_stats"), 256);

            // 1. naming. next_in_syllable is maximal onset: a consonant joins a
            // following vowel, a vowel keeps a following consonant only when that
            // consonant is not itself the onset of the next syllable. The recorded
            // inventory is sparse (William has fah but no fow, h but no hh), so a
            // fused name that is not a type in this voice falls back to unfused.
            let seq: Vec<ceps::Phone> = phones.iter()
                .map(|(p, st)| ceps::Phone::new(p, *st)).collect();
            let names = nr.name_utterance(&seq, |n| v.type_id(n).is_some());
            println!("names:  {}", names.join(" "));

            // 2. durations
            let mut ends = Vec::with_capacity(phones.len());
            let mut end = 0.0f32;
            for (i, (p, st)) in phones.iter().enumerate() {
                let prev = if i == 0 { "pau" } else { phones[i - 1].0.as_str() };
                let next = phones.get(i + 1).map(|(n, _)| n.as_str()).unwrap_or("pau");
                let f = |n: &str| -> ceps::FeatVal {
                    match n {
                        "name" => ceps::FeatVal::Str(p.clone()),
                        "p.name" => ceps::FeatVal::Str(prev.into()),
                        "n.name" => ceps::FeatVal::Str(next.into()),
                        "R:SylStructure.parent.stress" => ceps::FeatVal::Num(*st as f32),
                        "p.R:SylStructure.parent.parent.pbreak" =>
                            ceps::FeatVal::Str(if i == 0 { "BB" } else { "NB" }.into()),
                        "R:SylStructure.parent.syl_break" => ceps::FeatVal::Num(0.0),
                        _ => ceps::FeatVal::Num(0.0),
                    }
                };
                let z = dc.interpret(&f).as_num();
                let d = match ds.get(p) {
                    Some(st) => ceps::cart::segment_duration(z, st, 1.0),
                    None => 0.08,
                };
                end += d.clamp(0.01, 0.8);
                ends.push(end);
            }
            println!("duration: {:.2} s total", end);

            // 3. F0. settings.txt says PROSODY "none" on all four voices and the
            // SDK's own output confirms it: Allison renders at 195 Hz median with
            // a 137-248 Hz spread, matching her recorded 204 Hz / 134-269 Hz, not
            // any declination. --f0 opts into the modified-LPC path anyway.
            let want_f0 = a.iter().any(|x| x == "--f0");
            let fp = ceps::FlatProsody::for_voice(&v);
            let targets = fp.targets(end);
            if want_f0 {
                println!("f0:     {:.0} Hz -> {:.0} Hz  (mean {:.0})",
                         targets[0].f0, targets[1].f0, fp.mean);
            } else {
                println!("f0:     natural (PROSODY \"none\"), ~{:.0} Hz recorded",
                         ceps::natural_f0(&v, 20_000));
            }

            // 4. selection, with the target cost driven from each phone's id
            let types: Vec<usize> = names.iter()
                .map(|n| v.type_id(n).unwrap_or_else(|| {
                    eprintln!("no such type: {n}");
                    std::process::exit(1);
                })).collect();
            let mut sp = ceps::SelectParams::from_voice(&v);
            if let Some(i) = a.iter().position(|x| x == "--beam") {
                sp.beam = a[i + 1].parse().unwrap_or(sp.beam);
            }
            // Target features drive the prosody. PROSODY "none" means pitch is
            // never modified, so a falling phrase end can only come from picking
            // units that were recorded falling -- lisp_finalityp, at weight 4589,
            // is what steers the search to them.
            let table = ceps::PhoneTable::learn(&v, &tc);
            let tfeats = ceps::build_targets(&v, &table, &nr, &tc, &seq);
            let fin = tc.features.iter().position(|f| f == "lisp_finalityp");
            if let Some(k) = fin {
                let row: Vec<String> = tfeats.iter().map(|t| t[k].to_string()).collect();
                println!("finality: {}", row.join(" "));
            }
            let t = Instant::now();
            let mut sel = ceps::Selector::new(&v, sp);
            let chosen = if a.iter().any(|x| x == "--no-target") {
                sel.select(&types)
            } else {
                sel.select_with_targets(&types, Some(&tfeats))
            };
            println!("select: {} units in {:.1?} ({} joins)",
                     chosen.len(), t.elapsed(), sel.joins_evaluated);

            // 5. prosody + synthesis
            let units: Vec<ceps::TargetUnit> = chosen.iter().zip(&ends).map(|(c, &e)| {
                ceps::TargetUnit {
                    start: c.start, end: c.end,
                    target_end: (e * v.sps as f32) as i32,
                    gain: ceps::GAIN_UNITY,
                }
            }).collect();
            let pcm = if want_f0 {
                let pms = ceps::f0_targets_to_pm(&v, &targets, fp.mean);
                let periods = ceps::concat_units(&v, &units, &pms);
                let pcm = ceps::synth_periods(&v, &periods);
                let padded: usize = periods.iter()
                    .map(|p| p.size.saturating_sub(v.frame_size(p.src_frame))).sum();
                let total: usize = periods.iter().map(|p| p.size).sum();
                println!("synth:  {} periods, {:.1}% inserted silence",
                         periods.len(), 100.0 * padded as f64 / total.max(1) as f64);
                pcm
            } else {
                let n: usize = units.iter()
                    .map(|u| (u.start..u.end).map(|i| v.frame_size(i as usize)).sum::<usize>())
                    .sum();
                println!("synth:  {n} samples from recorded periods, no padding");
                ceps::join_units_simple(&v, &units)
            };
            wav::write(outp, &pcm, v.sps).unwrap();
            println!("wrote {outp}  {:.2} s", pcm.len() as f64 / v.sps as f64);
        }
        "finality" => {
            // What each lisp_finalityp / lisp_phone_break value means, read off
            // the database: co-occurrence with pause context, and the recorded F0
            // of the units carrying it. A terminal fall exists in the corpus only
            // if the "final" units are actually recorded lower.
            let unp = v.unit_name_params().expect("no unit_name_params");
            let tc = ceps::TargetCost::parse(v.image(), unp).expect("target_cost");
            let col = |n: &str| tc.features.iter().position(|f| f == n);
            let (cf, cb, cnp, cpp, cvow) = (
                col("lisp_finalityp").expect("lisp_finalityp"),
                col("lisp_phone_break").expect("lisp_phone_break"),
                col("lisp_next_pau").expect("lisp_next_pau"),
                col("lisp_prev_pau").expect("lisp_prev_pau"),
                col("lisp_vowel").expect("lisp_vowel"),
            );

            // median recorded F0 of a unit = sps / median frame size over the unit
            let unit_f0 = |u: usize| -> Option<f32> {
                let x = v.unit(u);
                if x.end <= x.start { return None; }
                let mut sz: Vec<usize> = (x.start..x.end)
                    .map(|i| v.frame_size(i as usize)).filter(|&n| n > 0).collect();
                if sz.is_empty() { return None; }
                sz.sort_unstable();
                let m = sz[sz.len() / 2] as f32;
                let f = v.sps as f32 / m;
                if (50.0..500.0).contains(&f) { Some(f) } else { None }
            };

            for (label, c) in [("lisp_finalityp", cf), ("lisp_phone_break", cb)] {
                println!("\n=== {label} ===");
                println!("  {:>5} {:>8} {:>9} {:>9} {:>10} {:>10}",
                         "value", "units", "next_pau", "prev_pau", "vowel F0", "all F0");
                let mut vals: Vec<u8> = (0..v.num_units).map(|u| v.unit_feats(u)[c]).collect();
                vals.sort_unstable();
                vals.dedup();
                for val in vals {
                    let mut n = 0usize;
                    let (mut np, mut pp) = (0usize, 0usize);
                    let (mut vf, mut af) = (Vec::new(), Vec::new());
                    for u in 0..v.num_units {
                        let f = v.unit_feats(u);
                        if f[c] != val { continue; }
                        n += 1;
                        np += (f[cnp] != 0) as usize;
                        pp += (f[cpp] != 0) as usize;
                        if let Some(x) = unit_f0(u) {
                            af.push(x);
                            if f[cvow] != 0 { vf.push(x); }
                        }
                    }
                    let med = |mut x: Vec<f32>| -> f32 {
                        if x.is_empty() { return 0.0; }
                        x.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        x[x.len() / 2]
                    };
                    println!("  {val:>5} {n:>8} {:>8.0}% {:>8.0}% {:>9.1} {:>9.1}",
                             100.0 * np as f64 / n.max(1) as f64,
                             100.0 * pp as f64 / n.max(1) as f64,
                             med(vf), med(af));
                }
            }
        }
        "phonemap" => {
            // lisp_phone_nameid is intrinsic to the phone, so every unit type built
            // on the same phone shares it. Group types by nameid and the common
            // prefix of each group names the phone -- no hardcoded phone list.
            let unp = v.unit_name_params().expect("no unit_name_params");
            let tc = ceps::TargetCost::parse(v.image(), unp).expect("target_cost");
            let idc = tc.features.iter().position(|f| f == "lisp_phone_nameid").unwrap();

            let mut by_id: std::collections::BTreeMap<u8, Vec<&str>> = Default::default();
            for t in 0..v.types.len() {
                let r = v.candidates(t);
                if r.start >= r.end { continue; }
                let mut h: std::collections::BTreeMap<u8, usize> = Default::default();
                for u in r.clone() {
                    *h.entry(v.unit_feats(u)[idc]).or_insert(0) += 1;
                }
                let modal = h.iter().max_by_key(|(_, &n)| n).map(|(&k, _)| k).unwrap();
                let pure = h.len() == 1;
                if !pure {
                    println!("  NOTE type {} spans {} nameids", v.types[t].name, h.len());
                }
                by_id.entry(modal).or_default().push(&v.types[t].name);
            }

            println!("{} distinct nameids over {} types", by_id.len(), v.types.len());
            for (id, names) in &by_id {
                let first = names[0];
                let mut pre = first.len();
                for n in names {
                    let common = first.bytes().zip(n.bytes()).take_while(|(a, b)| a == b).count();
                    pre = pre.min(common);
                }
                println!("  {id:>3}  phone {:<10} {:>4} types  e.g. {}",
                         &first[..pre], names.len(),
                         names.iter().take(5).cloned().collect::<Vec<_>>().join(" "));
            }
        }
        "breaks" => {
            // What each lisp_phone_break value means. The type names carry ground
            // truth: a name ending in a function word ("dahthe", "nand", "uh0to")
            // marks a unit recorded with that word following it, so the unit is
            // word-final by construction. Break values concentrated on those types
            // are word-level; the rest are not.
            let unp = v.unit_name_params().expect("no unit_name_params");
            let tc = ceps::TargetCost::parse(v.image(), unp).expect("target_cost");
            let r = ceps::NameRules::parse(v.image(), unp);
            let cb = tc.features.iter().position(|f| f == "lisp_phone_break").unwrap();
            let cf = tc.features.iter().position(|f| f == "lisp_finalityp").unwrap();
            let cnp = tc.features.iter().position(|f| f == "lisp_next_pau").unwrap();
            let csn = tc.features.iter()
                .position(|f| f == "R:SylStructure.parent.syl_numphones").unwrap();

            // longest function-word suffix, if any
            let fword_suffix = |name: &str| -> Option<&str> {
                r.fwords.iter()
                    .filter(|w| name.len() > w.len() && name.ends_with(w.as_str()))
                    .max_by_key(|w| w.len())
                    .map(|w| w.as_str())
            };
            let mut is_fw = vec![false; v.types.len()];
            let mut n_fw_types = 0usize;
            for t in 0..v.types.len() {
                if fword_suffix(&v.types[t].name).is_some() {
                    is_fw[t] = true;
                    n_fw_types += 1;
                }
            }
            println!("{n_fw_types} of {} types end in a function word", v.types.len());

            let unit_f0 = |u: usize| -> Option<f32> {
                let x = v.unit(u);
                let mut sz: Vec<usize> = (x.start..x.end)
                    .map(|i| v.frame_size(i as usize)).filter(|&n| n > 0).collect();
                if sz.is_empty() { return None; }
                sz.sort_unstable();
                let f = v.sps as f32 / sz[sz.len() / 2] as f32;
                if (40.0..500.0).contains(&f) { Some(f) } else { None }
            };
            let med = |mut x: Vec<f32>| -> f32 {
                if x.is_empty() { return 0.0; }
                x.sort_by(|a, b| a.partial_cmp(b).unwrap());
                x[x.len() / 2]
            };

            println!("\n{:>5} {:>8} {:>9} {:>9} {:>9} {:>8} {:>8}",
                     "break", "units", "word-fin", "next_pau", "final>0", "F0", "sylph");
            let mut vals: Vec<u8> = (0..v.num_units).map(|u| v.unit_feats(u)[cb]).collect();
            vals.sort_unstable();
            vals.dedup();
            for val in vals {
                let (mut n, mut fw, mut np, mut fin) = (0usize, 0usize, 0usize, 0usize);
                let (mut f0s, mut sn) = (Vec::new(), 0usize);
                for u in 0..v.num_units {
                    let f = v.unit_feats(u);
                    if f[cb] != val { continue; }
                    n += 1;
                    if is_fw[v.unit(u).type_id as usize] { fw += 1; }
                    np += (f[cnp] != 0) as usize;
                    fin += (f[cf] != 0) as usize;
                    sn += f[csn] as usize;
                    if let Some(x) = unit_f0(u) { f0s.push(x); }
                }
                let pct = |k: usize| 100.0 * k as f64 / n.max(1) as f64;
                println!("{val:>5} {n:>8} {:>8.1}% {:>8.0}% {:>8.0}% {:>8.1} {:>8.2}",
                         pct(fw), pct(np), pct(fin), med(f0s), sn as f64 / n.max(1) as f64);
            }

            // the decisive view: where do the word-final units actually land?
            println!("\nbreak distribution over the units that end in a function word:");
            let mut h: std::collections::BTreeMap<u8, usize> = Default::default();
            let mut tot = 0usize;
            for u in 0..v.num_units {
                if is_fw[v.unit(u).type_id as usize] {
                    *h.entry(v.unit_feats(u)[cb]).or_insert(0) += 1;
                    tot += 1;
                }
            }
            for (k, c) in &h {
                println!("  break {k} -> {c:>7} ({:.1}%)", 100.0 * *c as f64 / tot.max(1) as f64);
            }
            // which function words take a break and which cliticise, and whether a
            // fused type (two phones in one unit) hides the boundary
            println!("\nper function word, break at the boundary before it:");
            println!("  {:<6} {:>7} {:>8} {:>8} {:>8}  {}",
                     "word", "units", "brk 0", "brk 3", "fused", "example types");
            let mut words: Vec<String> = r.fwords.clone();
            words.sort();
            for w in &words {
                let mut per: std::collections::BTreeMap<u8, usize> = Default::default();
                let (mut n, mut fused) = (0usize, 0usize);
                let mut examples: Vec<&str> = Vec::new();
                for t in 0..v.types.len() {
                    let name = &v.types[t].name;
                    if fword_suffix(name) != Some(w.as_str()) { continue; }
                    // a fused type carries two phones, so its break sits inside the unit
                    let stem = &name[..name.len() - w.len()];
                    let multi = stem.len() > 1 && !stem.ends_with(|c: char| c.is_ascii_digit());
                    if examples.len() < 3 { examples.push(name); }
                    for u in v.candidates(t) {
                        n += 1;
                        if multi { fused += 1; }
                        *per.entry(v.unit_feats(u)[cb]).or_insert(0) += 1;
                    }
                }
                if n == 0 { continue; }
                let g = |k: u8| 100.0 * *per.get(&k).unwrap_or(&0) as f64 / n as f64;
                println!("  {w:<6} {n:>7} {:>7.0}% {:>7.0}% {:>7.0}%  {}",
                         g(0), g(3), 100.0 * fused as f64 / n as f64, examples.join(" "));
            }

            println!("\nand over every other unit:");
            let mut h2: std::collections::BTreeMap<u8, usize> = Default::default();
            let mut tot2 = 0usize;
            for u in 0..v.num_units {
                if !is_fw[v.unit(u).type_id as usize] {
                    *h2.entry(v.unit_feats(u)[cb]).or_insert(0) += 1;
                    tot2 += 1;
                }
            }
            for (k, c) in &h2 {
                println!("  break {k} -> {c:>7} ({:.1}%)", 100.0 * *c as f64 / tot2.max(1) as f64);
            }
        }
        "file" => {
            // The production path, and the only one with no length limit: the
            // text comes off disk rather than out of a shell argument, and the
            // audio goes to disk an utterance at a time rather than piling up.
            let inp = &a[3];
            let outp = &a[4];
            let text = std::fs::read_to_string(inp).unwrap_or_else(|e| {
                eprintln!("cannot read {inp}: {e}");
                std::process::exit(1);
            });
            let style = if a.iter().any(|x| x == "--digits") {
                ceps::DigitStyle::Digits
            } else {
                ceps::DigitStyle::Cardinal
            };
            let Some((lexbytes, lexfrom)) = load_pack("CEPS_CEPLEX", "ceplex.bin", EMBEDDED_CEPLEX)
            else {
                eprintln!("no Cepstral lexicon: this binary was built without embed-packs");
                eprintln!("put ceplex.bin beside it, or set CEPS_CEPLEX");
                std::process::exit(1);
            };
            let Some(cep) = ceps::CepLex::from_bytes(lexbytes) else {
                eprintln!("the lexicon at {lexfrom} is not a ceplex pack");
                std::process::exit(1);
            };
            let pos = if a.iter().any(|x| x == "--no-pos") {
                None
            } else {
                load_pack("CEPS_POS", "pos.bin", EMBEDDED_POS)
                    .and_then(|(b, _)| ceps::PosTag::from_bytes(b))
            };
            println!("{} bytes of text, lexicon {} entries ({lexfrom}), tagger {}",
                     text.len(), cep.len(),
                     match &pos { Some(p) => format!("{} features", p.feature_count()),
                                  None => "none".to_string() });

            warn_if_voice_mismatched(&v);

            let marked = markup_flag(&a, &text);
            let volume = volume_flag(&a);
            let t0 = std::time::Instant::now();
            let (seq, ctls, missing) = match &marked {
                Some(m) => {
                    let (s, c, _, mi) = ceps::lex::text_to_phones_marked(&cep, pos.as_ref(), m, style);
                    (s, c, mi)
                }
                None => {
                    let (s, _, mi) = ceps::lex::text_to_phones_tagged(&cep, pos.as_ref(), &text, style);
                    (s, Vec::new(), mi)
                }
            };
            if !missing.is_empty() {
                println!("unpronounceable (skipped): {}", missing.len());
            }
            let real = seq.iter().filter(|p| !p.is_pause()).count();
            println!("phones: {real} in {:.1}s", t0.elapsed().as_secs_f32());

            let mut w = ceps::wav::Writer::create(outp, v.sps).unwrap_or_else(|e| {
                eprintln!("cannot write {outp}: {e}");
                std::process::exit(1);
            });
            let t1 = std::time::Instant::now();
            let (mut utts, mut units) = (0usize, 0usize);
            let mut failed = None;
            ceps::speak::say_phones_each_ctl(&v, &seq, port_flags(&a), ceps::speak::word_ctl(&ctls), |said| {
                if failed.is_some() {
                    return;
                }
                utts += 1;
                units += said.units.len();
                let mut pcm = said.pcm;
                ceps::speak::apply_volume(&mut pcm, volume);
                if let Err(e) = w.push(&pcm) {
                    failed = Some(e);
                    return;
                }
                if utts % 25 == 0 {
                    let secs = w.samples() as f32 / v.sps as f32;
                    println!("  {utts} utterances, {units} units, {secs:.0}s of audio");
                }
            });
            if let Some(e) = failed {
                eprintln!("write failed after {utts} utterances: {e}");
                std::process::exit(1);
            }
            let n = w.finish().unwrap_or_else(|e| {
                eprintln!("cannot finish {outp}: {e}");
                std::process::exit(1);
            });
            let secs = n as f32 / v.sps as f32;
            let el = t1.elapsed().as_secs_f32();
            println!("{utts} utterances, {units} units, {secs:.2}s of audio in {el:.1}s \
                      ({:.0}x realtime)", secs / el.max(1e-6));
            println!("wrote {outp}");
        }
        "text" => {
            // text -> lexicon -> naming -> duration -> selection -> audio
            let marked = markup_flag(&a, &a[3]);
            let volume = volume_flag(&a);
            let text: &str = marked.as_ref().map_or(&a[3], |m| &m.text);
            let outp = &a[4];
            let style = if a.iter().any(|x| x == "--digits") {
                ceps::DigitStyle::Digits
            } else {
                ceps::DigitStyle::Cardinal
            };
            // Cepstral's own data when it is available; the festival packs are
            // a fallback, and a visibly different one
            let cep = if a.iter().any(|x| x == "--festival") {
                None
            } else {
                load_pack("CEPS_CEPLEX", "ceplex.bin", EMBEDDED_CEPLEX)
                    .and_then(|(b, _)| ceps::CepLex::from_bytes(b))
            };
            // Only open the festival packs if we are actually going to use them.
            // They are built from a festival install, not from Cepstral's, so
            // loading them eagerly would make cmulex.bin a hard requirement for
            // anyone who just wants to run a voice they already own.
            let fest = if cep.is_some() {
                None
            } else {
                let lexpath = std::env::var("CEPS_LEXICON")
                    .unwrap_or_else(|_| default_path("cmulex.bin"));
                let lex = ceps::Lexicon::open(&lexpath).unwrap_or_else(|e| {
                    eprintln!("no Cepstral lexicon available, and no festival one \
                               at {lexpath}: {e}");
                    eprintln!("point CEPS_CEPLEX at ceplex.bin, or pass --festival \
                               with CEPS_LEXICON set");
                    std::process::exit(1);
                });
                let ltspath = std::env::var("CEPS_LTS")
                    .unwrap_or_else(|_| default_path("cmults.bin"));
                let lts = ceps::Lts::open(&ltspath).ok();
                if lts.is_none() {
                    println!("no LTS pack at {ltspath}; unknown words will be skipped");
                }
                Some((lex, lts))
            };
            // The tagger only touches utterances holding one of its 237 listed
            // homographs, so loading it changes nothing else.
            // `load_pack`, not a hardcoded path: this used to name one absolute
            // Windows path with no fallback, so `text` quietly ran untagged on
            // every other platform while `file` -- which did use load_pack --
            // tagged normally. Homographs were the only visible symptom, which
            // is why it survived a bit-exactness check on the Pi.
            let pos = if a.iter().any(|x| x == "--no-pos") {
                None
            } else {
                load_pack("CEPS_POS", "pos.bin", EMBEDDED_POS)
                    .and_then(|(b, _)| ceps::PosTag::from_bytes(b))
            };
            let mut ctls = Vec::new();
            let (seq, guessed, missing) = match &cep {
                Some(c) => {
                    println!("lexicon: Cepstral, {} entries, {} LTS nodes",
                             c.len(), c.nodes());
                    match &pos {
                        Some(p) => println!("tagger: {} features, {} homographs",
                                            p.feature_count(), p.homographs().len()),
                        None => println!("tagger: none ('0' entries throughout)"),
                    }
                    match &marked {
                        Some(m) => {
                            let (s, cs, g, mi) =
                                ceps::lex::text_to_phones_marked(c, pos.as_ref(), m, style);
                            ctls = cs;
                            (s, g, mi)
                        }
                        None => ceps::lex::text_to_phones_tagged(c, pos.as_ref(), text, style),
                    }
                }
                None => {
                    if marked.is_some() {
                        println!("markup: the festival front end ignores it; text only");
                    }
                    let (lex, lts) = fest.as_ref().expect("festival fallback");
                    ceps::text_to_phones_styled(lex, lts.as_ref(), text, style)
                }
            };
            if let (Some(p), Some(c)) = (&pos, &cep) {
                let shown: Vec<String> = ceps::lex::tags_for_text(c, p, text, style)
                    .into_iter()
                    .filter_map(|(w, t)| t.map(|t| format!("{w}/{t}")))
                    .collect();
                if !shown.is_empty() {
                    println!("tagged: {}", shown.join(" "));
                }
            }
            if !guessed.is_empty() {
                let w: Vec<&str> = guessed.iter().map(|(w, _)| w.as_str()).collect();
                println!("letter-to-sound (not in dictionary): {}", w.join(" "));
            }
            if !missing.is_empty() {
                println!("unpronounceable (skipped): {}", missing.join(" "));
            }
            let real = seq.iter().filter(|p| !p.is_pause()).count();
            println!("phones:  {real} ({} pauses) from {}", seq.len() - real,
                     if cep.is_some() { "Cepstral's own data" } else { "the festival packs" });
            if real == 0 {
                eprintln!("nothing to say");
                std::process::exit(1);
            }

            let unp = v.unit_name_params().expect("unit_name_params");
            let nr = ceps::NameRules::parse(v.image(), unp);
            let tc = ceps::TargetCost::parse(v.image(), unp).expect("target_cost");
            let dc = ceps::Cart::parse(v.image(), v.dur_cart().expect("dur_cart"));
            let ds = ceps::DurStats::parse(v.image(), v.dur_stats().expect("dur_stats"), 256);

            let names = nr.name_utterance(&seq, |n| v.type_id(n).is_some());
            let mut unknown: Vec<&str> = names.iter()
                .filter(|n| v.type_id(n).is_none()).map(|s| s.as_str()).collect();
            unknown.sort();
            unknown.dedup();
            if !unknown.is_empty() {
                eprintln!("no such types: {}", unknown.join(" "));
                std::process::exit(1);
            }
            if a.iter().any(|x| x == "--names") {
                println!("names: {}", names.join(" "));
            }

            let mut ends = Vec::with_capacity(seq.len());
            let mut end = 0.0f32;
            for (i, ph) in seq.iter().enumerate() {
                let prev = if i == 0 { "pau" } else { seq[i - 1].phone.as_str() };
                let next = seq.get(i + 1).map(|q| q.phone.as_str()).unwrap_or("pau");
                let f = |n: &str| -> ceps::FeatVal {
                    match n {
                        "name" => ceps::FeatVal::Str(ph.phone.clone()),
                        "p.name" => ceps::FeatVal::Str(prev.into()),
                        "n.name" => ceps::FeatVal::Str(next.into()),
                        "R:SylStructure.parent.stress" => ceps::FeatVal::Num(ph.stress as f32),
                        "p.R:SylStructure.parent.parent.pbreak" =>
                            ceps::FeatVal::Str(if i == 0 { "BB" } else { "NB" }.into()),
                        _ => ceps::FeatVal::Num(0.0),
                    }
                };
                let z = dc.interpret(&f).as_num();
                let d = match ds.get(&ph.phone) {
                    Some(st) => ceps::cart::segment_duration(z, st, 1.0),
                    None => 0.08,
                };
                end += d.clamp(0.01, 0.8);
                ends.push(end);
            }

            let table = ceps::PhoneTable::learn(&v, &tc);
            let tfeats = ceps::build_targets(&v, &table, &nr, &tc, &seq);
            let types: Vec<usize> = names.iter().filter_map(|n| v.type_id(n)).collect();
            let mut sp = ceps::SelectParams::from_voice(&v);
            if let Some(i) = a.iter().position(|x| x == "--beam") {
                sp.beam = a[i + 1].parse().unwrap_or(sp.beam);
            }
            let _ = (&tfeats, &types, &sp, &ends);
            // One utterance at a time, as the engine does: see src/speak.rs.
            let t = Instant::now();
            let mut said = Vec::new();
            ceps::speak::say_phones_each_ctl(&v, &seq, port_flags(&a), ceps::speak::word_ctl(&ctls),
                                              |s| said.push(s));
            let sel_dt = t.elapsed();
            let t2 = Instant::now();
            let mut pcm: Vec<i16> = said.iter().flat_map(|s| s.pcm.iter().copied()).collect();
            ceps::speak::apply_volume(&mut pcm, volume);
            let units: usize = said.iter().map(|s| s.units.len()).sum();
            let pieces: usize = said.iter().map(|s| s.pieces.len()).sum();
            let secs = pcm.len() as f64 / v.sps as f64;
            println!("select: {units} units in {sel_dt:.1?}, {} utterances, {pieces} pieces",
                     said.len());
            println!("synth:  {:.2} s of audio in {:.1?}", secs, t2.elapsed());
            println!("total:  {:.0}x realtime",
                     secs / (sel_dt + t2.elapsed()).as_secs_f64());
            wav::write(outp, &pcm, v.sps).unwrap();
            println!("wrote {outp}");
        }
        "lex" => {
            let lexpath = std::env::var("CEPS_LEXICON")
                .unwrap_or_else(|_| r"..\..\packs\cmulex.bin".to_string());
            let lex = ceps::Lexicon::open(&lexpath).expect("open lexicon");
            println!("{} entries, {} phones", lex.len(), lex.phone_names().len());
            for w in a[3].split_whitespace() {
                match lex.lookup(w) {
                    Some(syls) => {
                        let shown: Vec<String> = syls.iter()
                            .map(|s| format!("({}){}", s.phones.join(" "), s.stress))
                            .collect();
                        println!("  {w:<16} {}", shown.join(" "));
                    }
                    None => println!("  {w:<16} NOT FOUND"),
                }
            }
        }
        "lts" => {
            let ltspath = std::env::var("CEPS_LTS")
                .unwrap_or_else(|_| r"..\..\packs\cmults.bin".to_string());
            let lts = ceps::Lts::open(&ltspath).expect("open LTS pack");
            let lexpath = std::env::var("CEPS_LEXICON")
                .unwrap_or_else(|_| r"..\..\packs\cmulex.bin".to_string());
            let lex = ceps::Lexicon::open(&lexpath).ok();
            println!("{} letter trees", lts.letters());
            for w in a[3].split_whitespace() {
                let p = lts.predict(w);
                let shown: Vec<String> = p.iter()
                    .map(|x| format!("{}{}", x.phone, x.stress)).collect();
                let known = lex.as_ref().and_then(|l| l.lookup(w)).is_some();
                println!("  {w:<16} {:<6} {}",
                         if known { "[dict]" } else { "[lts]" }, shown.join(" "));
                // when the dictionary has it too, show what it says for comparison
                if let Some(syls) = lex.as_ref().and_then(|l| l.lookup(w)) {
                    let d: Vec<String> = syls.iter()
                        .flat_map(|s| s.phones.iter().map(move |p| format!("{}{}", p, s.stress)))
                        .collect();
                    println!("  {:<16} {:<6} {}", "", "dict:", d.join(" "));
                }
            }
        }
        "utts" => {
            // One line per utterance: "UTT <index> <units per streamed piece>".
            // _out/piece_diff.py compares it against the engine's own walk of
            // the Unit relation.
            let ceppath = std::env::var("CEPS_CEPLEX")
                .unwrap_or_else(|_| r"..\..\packs\ceplex.bin".to_string());
            let cep = ceps::CepLex::open(&ceppath).expect("ceplex");
            for (i, s) in ceps::say(&v, &cep, &a[3], ceps::DigitStyle::Cardinal)
                .iter().enumerate()
            {
                println!("UTT {i} {}", s.pieces.iter().map(|n| n.to_string())
                         .collect::<Vec<_>>().join(" "));
            }
        }
        "phones" => {
            // Just the phone sequence, one line, for diffing against the
            // engine's Segment relation (see _out/seg_diff.py).
            let style = if a.iter().any(|x| x == "--digits") {
                ceps::DigitStyle::Digits
            } else {
                ceps::DigitStyle::Cardinal
            };
            let (ph, _) = front_end(&a[3], style, a.iter().any(|x| x == "--festival"));
            println!("phones: {}", ph.iter()
                     .map(|p| if p.is_pause() { "pau".to_string() } else { p.phone.clone() })
                     .collect::<Vec<_>>().join(" "));
        }
        "norm" => {
            // With the lexicon, as `text` runs it: several rules ask whether a
            // token is a dictionary word before spelling it.
            let cep = if a.iter().any(|x| x == "--no-lex") {
                None
            } else {
                load_pack("CEPS_CEPLEX", "ceplex.bin", EMBEDDED_CEPLEX)
                    .and_then(|(b, _)| ceps::CepLex::from_bytes(b))
            };
            for style in [ceps::DigitStyle::Cardinal, ceps::DigitStyle::Digits] {
                let toks = match &cep {
                    Some(c) => ceps::normalize_with(&a[3], style, |w| c.lookup(w).is_some()),
                    None => ceps::normalize(&a[3], style),
                };
                let mut shown = String::new();
                for t in &toks {
                    match t {
                        ceps::Token::Word(w) => {
                            if !shown.is_empty() { shown.push(' '); }
                            shown.push_str(w);
                        }
                        ceps::Token::Break(hard, _) => {
                            shown.push_str(if *hard { " |" } else { " ," });
                        }
                    }
                }
                println!("{:<9} {shown}", format!("{style:?}:"));
            }
        }
        "trace" => {
            // Recover the engine's own frame and unit sequence from one of its
            // WAVs. Ground truth for validating unit selection.
            let refp = &a[3];
            let secs: f64 = a.get(4).and_then(|x| x.parse().ok()).unwrap_or(3.0);
            let (pcm, sps) = wav::read(refp).unwrap_or_else(|e| {
                eprintln!("cannot read {refp}: {e}");
                std::process::exit(1);
            });
            if sps != v.sps {
                eprintln!("sample rate mismatch: {refp} is {sps} Hz, voice is {} Hz", v.sps);
                std::process::exit(1);
            }
            let t0 = Instant::now();
            let tr = ceps::Tracer::new(&v, &pcm);
            println!("{} frames decoded in {:.1?}, {} threads",
                     tr.frames(), t0.elapsed(), tr.threads);

            let t1 = Instant::now();
            let trace = tr.run((secs * sps as f64) as usize);
            let dt = t1.elapsed();
            println!("{} frame transitions over {:.1} s in {:.2?}",
                     trace.hits.len(), secs, dt);
            println!("  {} probes, {} exhaustive searches ({:.1}%)",
                     trace.probes, trace.globals,
                     100.0 * trace.globals as f64 / trace.probes.max(1) as f64);
            if trace.globals > 0 {
                println!("  {:.1?} per exhaustive search over {} frames",
                         dt / trace.globals as u32, tr.frames());
            }

            let steps: Vec<i64> = trace.hits.windows(2)
                .map(|w| w[1].frame as i64 - w[0].frame as i64).collect();
            let consec = steps.iter().filter(|&&d| d == 1).count();
            println!("  consecutive (+1) frame steps: {consec}/{} ({:.1}%)",
                     steps.len(), 100.0 * consec as f64 / steps.len().max(1) as f64);
            let worst = trace.hits.iter().map(|h| h.score).fold(0.0f32, f32::max);
            println!("  worst accepted score {worst:.2} (threshold {})", ceps::trace::GOOD);

            let units = tr.units(&trace);
            println!("\n{} units recovered:", units.len());
            let names: Vec<String> = units.iter()
                .map(|&(_, u)| v.types[v.unit(u).type_id as usize].name.clone())
                .collect();
            println!("{}", names.join(" "));

            if a.iter().any(|x| x == "--verbose") {
                println!("\n{:>9}  {:>8}  {:<14} {}", "time", "unit", "type", "frames");
                for &(pos, u) in &units {
                    let x = v.unit(u);
                    println!("{:>8.3}s  {u:>8}  {:<14} {}..{}",
                             pos as f64 / sps as f64,
                             v.types[x.type_id as usize].name, x.start, x.end);
                }
            }
        }
        "parity" => {
            // Compare the engine's own choices against ours.
            //
            // With --text the real front end runs on the real text, so word
            // boundaries and syllable structure are right; the two type sequences
            // are then aligned and only the matched positions are compared. That
            // separates the two questions: how much aligns measures the FRONT END,
            // and how many aligned units agree measures SELECTION.
            //
            // Without --text the phone sequence is reconstructed from the traced
            // type names, which has no word boundaries at all -- useful for a
            // quick look, useless as a parity number.
            let refp = &a[3];
            let secs: f64 = a.get(4).and_then(|x| x.parse().ok()).unwrap_or(3.0);
            let text = a.iter().position(|x| x == "--text").map(|i| a[i + 1].clone());
            let (pcm, sps) = wav::read(refp).expect("read reference");
            if sps != v.sps {
                eprintln!("sample rate mismatch: {sps} vs {}", v.sps);
                std::process::exit(1);
            }
            let tr = ceps::Tracer::new(&v, &pcm);
            let t0 = Instant::now();
            let trace = tr.run((secs * sps as f64) as usize);
            let truth = tr.units(&trace);
            println!("traced {} units over {:.1} s in {:.2?} ({} exhaustive searches)",
                     truth.len(), secs, t0.elapsed(), trace.globals);
            let steps: Vec<i64> = trace.hits.windows(2)
                .map(|w| w[1].frame as i64 - w[0].frame as i64).collect();
            let consec = steps.iter().filter(|&&d| d == 1).count();
            println!("  trace quality: {:.1}% consecutive frame steps",
                     100.0 * consec as f64 / steps.len().max(1) as f64);
            if truth.len() < 2 {
                eprintln!("not enough recovered units to compare");
                std::process::exit(1);
            }

            let unp = v.unit_name_params().expect("unit_name_params");
            let nr = ceps::NameRules::parse(v.image(), unp);
            let tc = ceps::TargetCost::parse(v.image(), unp).expect("target_cost");
            let table = ceps::PhoneTable::learn(&v, &tc);

            let engine_types: Vec<usize> =
                truth.iter().map(|&(_, u)| v.unit(u).type_id as usize).collect();

            // our phone sequence: from the text where given, else reconstructed
            let (seq, from_text) = match &text {
                Some(t) => {
                    let style = if a.iter().any(|x| x == "--digits") {
                        ceps::DigitStyle::Digits
                    } else {
                        ceps::DigitStyle::Cardinal
                    };
                    let (ph, src) = front_end(t, style,
                                              a.iter().any(|x| x == "--festival"));
                    println!("  front end: {src}");
                    (ph, true)
                }
                None => (tr.phones(&table, &truth), false),
            };
            let names = nr.name_utterance(&seq, |n| v.type_id(n).is_some());
            let our_types: Vec<usize> = names.iter().filter_map(|n| v.type_id(n)).collect();
            let tfeats = ceps::build_targets(&v, &table, &nr, &tc, &seq);
            println!("  our front end: {} phones from {}",
                     our_types.len(), if from_text { "the text" } else { "the traced names" });

            // longest common subsequence over type ids, so a front-end insertion
            // or deletion costs only itself
            let (n0, n1) = (engine_types.len(), our_types.len());
            let mut dp = vec![0u32; (n0 + 1) * (n1 + 1)];
            let at = |i: usize, j: usize| i * (n1 + 1) + j;
            for i in (0..n0).rev() {
                for j in (0..n1).rev() {
                    dp[at(i, j)] = if engine_types[i] == our_types[j] {
                        dp[at(i + 1, j + 1)] + 1
                    } else {
                        dp[at(i + 1, j)].max(dp[at(i, j + 1)])
                    };
                }
            }
            let mut pairs: Vec<(usize, usize)> = Vec::new();
            let (mut i, mut j) = (0usize, 0usize);
            while i < n0 && j < n1 {
                if engine_types[i] == our_types[j] {
                    pairs.push((i, j));
                    i += 1;
                    j += 1;
                } else if dp[at(i + 1, j)] >= dp[at(i, j + 1)] {
                    i += 1;
                } else {
                    j += 1;
                }
            }
            println!("\nFRONT END: {}/{} of the engine's types aligned ({:.1}%)",
                     pairs.len(), n0, 100.0 * pairs.len() as f64 / n0 as f64);

            let mut sp = ceps::SelectParams::from_voice(&v);
            if let Some(k) = a.iter().position(|x| x == "--beam") {
                sp.beam = a[k + 1].parse().unwrap_or(sp.beam);
            }
            let mut sel = ceps::Selector::new(&v, sp.clone());
            let ours = sel.select_with_targets(&our_types, Some(&tfeats));
            let mut plain_sel = ceps::Selector::new(&v, sp);
            let plain = plain_sel.select(&our_types);

            let m = pairs.len().max(1);
            let exact = pairs.iter()
                .filter(|&&(i, j)| ours.get(j).map(|c| c.unit) == Some(truth[i].1)).count();
            let exact_plain = pairs.iter()
                .filter(|&&(i, j)| plain.get(j).map(|c| c.unit) == Some(truth[i].1)).count();
            println!("SELECTION: on the aligned positions,");
            println!("  with target cost   {exact}/{m}  ({:.1}%)",
                     100.0 * exact as f64 / m as f64);
            println!("  without            {exact_plain}/{m}  ({:.1}%)",
                     100.0 * exact_plain as f64 / m as f64);

            // cost function or search? price the engine's path with ours
            let e_path: Vec<usize> = pairs.iter().map(|&(i, _)| truth[i].1).collect();
            let o_path: Vec<usize> = pairs.iter()
                .filter_map(|&(_, j)| ours.get(j).map(|c| c.unit)).collect();
            let e_feats: Vec<Vec<i32>> = pairs.iter()
                .filter_map(|&(_, j)| tfeats.get(j).cloned()).collect();
            if e_path.len() == o_path.len() && e_path.len() == e_feats.len() {
                let mut sc = ceps::Selector::new(&v, ceps::SelectParams::from_voice(&v));
                let (et, ej, eg) = sc.path_cost(&e_path, Some(&e_feats));
                let (ot, oj, og) = sc.path_cost(&o_path, Some(&e_feats));
                println!("\ncost of each path under OUR cost function:");
                println!("  {:<8} {:>14} {:>14} {:>14}", "", "total", "join", "target");
                println!("  {:<8} {et:>14} {ej:>14} {eg:>14}", "engine");
                println!("  {:<8} {ot:>14} {oj:>14} {og:>14}", "ours");
                println!("  -> {}", if ot == et {
                    "identical: our cost function and the engine's agree exactly"
                } else if ot < et {
                    "our search wins on our own cost: the COST FUNCTION differs"
                } else {
                    "the engine's path scores better under our cost: our SEARCH is losing it"
                });
            }

            // which target features we still get wrong, on the aligned positions
            println!("\ntarget features still differing from the engine's own units:");
            let mut rows: Vec<(usize, i64, &str)> = Vec::new();
            for (k, name) in tc.features.iter().enumerate() {
                let (mut wrong, mut mag) = (0usize, 0i64);
                for &(i, j) in &pairs {
                    let Some(f) = tfeats.get(j) else { continue };
                    let want = v.unit_feats(truth[i].1)[k] as i32;
                    if want != f[k] {
                        wrong += 1;
                        mag += (want - f[k]).abs() as i64;
                    }
                }
                if wrong > 0 {
                    rows.push((wrong, mag, name.as_str()));
                }
            }
            rows.sort_by(|x, y| y.0.cmp(&x.0));
            println!("  {:>6} {:>9}  {}", "wrong", "total |d|", "feature");
            for (wrong, mag, name) in rows.iter().take(10) {
                println!("  {wrong:>5}/{m:<4} {mag:>9}  {name}");
            }
            println!("  {}/{} columns match everywhere",
                     tc.features.len() - rows.len(), tc.features.len());
        }
        "engine" => {
            // Map an exact frame sequence captured from the running engine
            // (swift.dll+0x2ab40 emit_period, see _out/run_hook.py) onto units,
            // and compare against what we would have selected for the same text.
            //
            // This supersedes the inverse-filter tracer as ground truth: the
            // tracer is ~90% reliable and costs seconds per second of audio,
            // while these indices come straight out of the engine.
            let path = &a[3];
            let raw = std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            });
            let periods: Vec<(usize, usize, i32)> = raw.lines()
                .filter_map(|l| {
                    let mut it = l.split_whitespace();
                    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?,
                          it.next().and_then(|x| x.parse().ok()).unwrap_or(0)))
                }).collect();
            if periods.is_empty() {
                eprintln!("no periods in {path}");
                std::process::exit(1);
            }

            let mut starts: Vec<(i32, usize)> =
                (0..v.num_units).map(|u| (v.unit(u).start, u)).collect();
            starts.sort_unstable();
            let unit_of = |f: usize| -> Option<usize> {
                let key = f as i32;
                let i = starts.partition_point(|&(s, _)| s <= key);
                if i == 0 { return None; }
                let u = starts[i - 1].1;
                let x = v.unit(u);
                if x.start <= key && key < x.end { Some(u) } else { None }
            };

            let mut seq: Vec<(usize, usize, usize)> = Vec::new();  // unit, periods, samples
            for &(f, sz, _) in &periods {
                match unit_of(f) {
                    Some(u) => match seq.last_mut() {
                        Some(r) if r.0 == u => { r.1 += 1; r.2 += sz; }
                        _ => seq.push((u, 1, sz)),
                    },
                    None => println!("frame {f} belongs to no unit"),
                }
            }
            let total: usize = periods.iter().map(|p| p.1).sum();
            println!("{} periods -> {} units, {} samples ({:.3} s at {} Hz)",
                     periods.len(), seq.len(), total,
                     total as f64 / v.sps as f64, v.sps);
            println!("\n{:>8} {:>9} {:>8} {:>8}  {:<14} {}",
                     "unit", "frames", "periods", "samples", "type", "candidates");
            for &(u, np, ns) in &seq {
                let x = v.unit(u);
                let t = x.type_id as usize;
                let r = v.candidates(t);
                println!("{u:>8} {:>4}..{:<4} {np:>8} {ns:>8}  {:<14} {}",
                         x.start, x.end, v.types[t].name, r.end - r.start);
            }
            println!("\nengine types: {}",
                     seq.iter().map(|&(u, _, _)| {
                         v.types[v.unit(u).type_id as usize].name.clone()
                     }).collect::<Vec<_>>().join(" "));

            if let Some(i) = a.iter().position(|x| x == "--text") {
                let style = if a.iter().any(|x| x == "--digits") {
                    ceps::DigitStyle::Digits
                } else {
                    ceps::DigitStyle::Cardinal
                };
                let (ph, src) = front_end(&a[i + 1], style,
                                          a.iter().any(|x| x == "--festival"));
                println!("front end: {src}, {} phones", ph.len());
                let unp = v.unit_name_params().expect("unit_name_params");
                let nr = ceps::NameRules::parse(v.image(), unp);
                let tc = ceps::TargetCost::parse(v.image(), unp).expect("target_cost");
                let table = ceps::PhoneTable::learn(&v, &tc);
                let names = nr.name_utterance(&ph, |n| v.type_id(n).is_some());
                println!("ours types:   {}", names.join(" "));

                let types: Vec<usize> = names.iter().filter_map(|n| v.type_id(n)).collect();
                let tf = ceps::build_targets(&v, &table, &nr, &tc, &ph);
                let mut sel = ceps::Selector::new(&v, ceps::SelectParams::from_voice(&v));
                let ours = sel.select_with_targets(&types, Some(&tf));

                let engine: Vec<usize> = seq.iter().map(|&(u, _, _)| u).collect();
                let n = engine.len().min(ours.len());
                let same = ours.iter().zip(&engine).filter(|(c, &u)| c.unit == u).count();
                let same_type = ours.iter().zip(&engine)
                    .filter(|(c, &u)| v.unit(c.unit).type_id == v.unit(u).type_id).count();
                println!("\ntypes  {same_type}/{n}   units  {same}/{n}");

                // Natural chains: the engine found "world" as four consecutive
                // units from one recording. If it prefers those far more than we
                // do, the join cost is what differs.
                let runs = |p: &[usize]| p.windows(2)
                    .filter(|w| v.unit(w[1]).start == v.unit(w[0]).end).count();
                let op: Vec<usize> = ours.iter().map(|c| c.unit).collect();
                println!("consecutive-unit joins: engine {}/{}, ours {}/{}",
                         runs(&engine), engine.len().saturating_sub(1),
                         runs(&op), op.len().saturating_sub(1));
                println!("streamed pieces ({}): {}", sel.last_pieces.len(),
                         sel.last_pieces.iter().map(|n| n.to_string())
                             .collect::<Vec<_>>().join(" "));
                println!("prefix depth per step: {}",
                         sel.last_prefix.iter().map(|n| n.to_string())
                             .collect::<Vec<_>>().join(" "));
                if let Some(k) = a.iter().position(|x| x == "--beam") {
                    let all: Vec<usize> = (0..sel.last_beam.len()).collect();
                    let sel_pos: Vec<usize> = if a[k + 1] == "all" {
                        all
                    } else {
                        a[k + 1].split(',').map(|p| p.parse().unwrap()).collect()
                    };
                    for p in sel_pos {
                        for (i, &(u, s)) in sel.last_beam[p].iter().enumerate() {
                            println!("BEAM {p} {i} {u} {s}");
                        }
                        for (i, &(u, s)) in sel.last_cands[p].iter().enumerate() {
                            println!("CAND {p} {i} {u} {s}");
                        }
                    }
                }

                // Cost function or search? Price the engine's own path with ours.
                let mut sc = ceps::Selector::new(&v, ceps::SelectParams::from_voice(&v));
                let ef: Vec<Vec<i32>> = tf.iter().take(n).cloned().collect();
                let (et, ej, eg) = sc.path_cost(&engine[..n], Some(&ef));
                let (ot, oj, og) = sc.path_cost(&op[..n], Some(&ef));
                println!("\ncost under OUR cost function:");
                println!("  {:<8} {:>13} {:>13} {:>13}", "", "total", "join", "target");
                println!("  {:<8} {et:>13} {ej:>13} {eg:>13}", "engine");
                println!("  {:<8} {ot:>13} {oj:>13} {og:>13}", "ours");
                println!("  -> {}", if ot == et {
                    "identical: our cost function and the engine's agree exactly"
                } else if ot < et {
                    "our search wins on our own cost: the COST FUNCTION differs"
                } else {
                    "the engine's path is cheaper for us too: our SEARCH is losing it"
                });

                // Is the engine's answer even reachable? cand_beam_width prunes on
                // target cost, so a unit pruned there can never be selected however
                // good its joins are.
                let mut pruned = 0usize;
                let mut rank_sum = 0usize;
                let mut ranked = 0usize;
                for (i, &u) in engine.iter().take(n).enumerate() {
                    let t = v.unit(u).type_id as usize;
                    let r = v.candidates(t);
                    if v.unit(ours[i].unit).type_id as usize != t {
                        continue;
                    }
                    let mut costs: Vec<(i32, usize)> = r.clone()
                        .map(|c| (sc.target_cost(&tf[i], c), c)).collect();
                    costs.sort();
                    let pos = costs.iter().position(|&(_, c)| c == u).unwrap_or(usize::MAX);
                    if pos == usize::MAX { continue; }
                    ranked += 1;
                    rank_sum += pos;
                    if pos >= tc.beam_width as usize && tc.beam_width > 0 {
                        pruned += 1;
                    }
                }
                if ranked > 0 {
                    println!("\nengine's unit ranked by OUR target cost, among its type's candidates:");
                    println!("  mean rank {:.0} of {} positions scored", rank_sum as f64 / ranked as f64, ranked);
                    println!("  pruned by cand_beam_width={}: {pruned}/{ranked}",
                             tc.beam_width);
                }

                // How should the two costs be balanced? Sweep rather than guess.
                println!("\ntarget-cost weight sweep:");
                println!("  {:>7} {:>9} {:>11} {:>13}", "scale", "units", "consec", "our total");
                for scale in [0.0f64, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0] {
                    let mut sp2 = ceps::SelectParams::from_voice(&v);
                    sp2.target_scale = scale;
                    let mut s2 = ceps::Selector::new(&v, sp2);
                    let pick = s2.select_with_targets(&types, Some(&tf));
                    let pv: Vec<usize> = pick.iter().map(|c| c.unit).collect();
                    let hit = pick.iter().zip(&engine).filter(|(c, &u)| c.unit == u).count();
                    let cons = runs(&pv);
                    let mut sc3 = ceps::Selector::new(&v, ceps::SelectParams::from_voice(&v));
                    let (tot, _, _) = sc3.path_cost(&pv[..n.min(pv.len())], Some(&ef));
                    println!("  {scale:>7.2} {hit:>6}/{n:<3} {cons:>7}/{:<3} {tot:>13}",
                             pv.len().saturating_sub(1));
                }
                println!("  (engine: {}/{} consecutive)",
                         runs(&engine), engine.len().saturating_sub(1));
            }
        }
        "cep" => {
            let ceppath = std::env::var("CEPS_CEPLEX")
                .unwrap_or_else(|_| r"..\..\packs\ceplex.bin".to_string());
            let c = ceps::CepLex::open(&ceppath).expect("open ceplex pack");
            println!("{} entries, {} LTS nodes, letters {:?}",
                     c.len(), c.nodes(), c.letters());
            for w in a[3].split_whitespace() {
                let (ph, from_lts) = c.phones(w);
                println!("  {w:<16} {:<6} {}",
                         if from_lts { "[lts]" } else { "[dict]" }, ph.join(" "));
                let all = c.lookup_all(w);
                if all.len() > 1 {
                    for (pos, p) in all {
                        println!("       pos {:?} -> {}", pos as char, p.join(" "));
                    }
                }
            }
        }
        "selftest" => {
            // Take a real recorded chain, keep only its type sequence, and see
            // whether selection finds its way back to the original units.
            // Consecutive units cost 0 to join, so a correct selector should.
            let first: usize = a[3].parse().unwrap();
            let truth = v.chain(first, 64);
            let types: Vec<usize> = truth.iter().map(|&u| v.unit(u).type_id as usize).collect();
            let cands: usize = types.iter().map(|&t| v.candidates(t).len()).sum();
            println!("chain of {} units, {} candidates total, search space 10^{:.0}",
                     truth.len(), cands,
                     types.iter().map(|&t| (v.candidates(t).len() as f64).log10()).sum::<f64>());

            let beams: Vec<usize> = if a.len() > 4 {
                a[4..].iter().filter_map(|x| x.parse().ok()).collect()
            } else {
                vec![8, 32, 128]
            };
            let tws: Vec<i32> = if a.len() > 4 { vec![0] } else { vec![0, 10, 70] };
            // Cepstral's target cost, with the true units' own feature vectors
            // as the targets -- which is what the front end will supply.
            let tvecs: Vec<Vec<i32>> = truth.iter()
                .map(|&u| v.unit_feats(u).iter().map(|&x| x as i32).collect())
                .collect();
            for &beam in &beams {
                let mut p = ceps::SelectParams::from_voice(&v);
                p.beam = beam;
                let t = Instant::now();
                let mut sel = ceps::Selector::new(&v, p);
                let got = sel.select_with_targets(&types, Some(&tvecs));
                let hit = got.iter().zip(&truth).filter(|(g, &t)| g.unit == t).count();
                println!("  TARGET COST      beam={beam:<4} recovered {hit}/{} ({:>3.0}%)  \
path cost {:<9}  {:>7.1?}  {} joins",
                         truth.len(), 100.0 * hit as f64 / truth.len() as f64,
                         sel.last_score, t.elapsed(), sel.joins_evaluated);
            }

            for tw in tws {
                for &beam in &beams {
                    let mut p = ceps::SelectParams::from_voice(&v);
                    p.target_weight = tw;
                    p.beam = beam;
                    let t = Instant::now();
                    let mut sel = ceps::Selector::new(&v, p);
                    let got = sel.select(&types);
                    let hit = got.iter().zip(&truth).filter(|(g, &t)| g.unit == t).count();
                    println!("  target_weight={tw:<3} beam={beam:<4} recovered {hit}/{} ({:>3.0}%)  \
path cost {:<9}  {:>7.1?}  {} joins",
                             truth.len(), 100.0 * hit as f64 / truth.len() as f64,
                             sel.last_score, t.elapsed(), sel.joins_evaluated);
                }
            }
        }
        "bench" => {
            let n: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(20_000);
            let first = v.num_sts / 3;
            for pass in 0..3 {
                let t = Instant::now();
                let pcm = synth::synth_frames(&v, first, first + n);
                let dt = t.elapsed();
                let dur = pcm.len() as f64 / v.sps as f64;
                println!("pass {pass}: {n} frames -> {dur:.2}s audio in {dt:.3?}  ({:.0}x realtime)",
                         dur / dt.as_secs_f64());
            }
        }
        "render" => {
            // Emit exactly the periods the engine emitted, from an emit_period
            // capture. Nothing else is in play -- no selection, no naming, no
            // durations -- so a difference against the engine's own wav for the
            // same run is the LPC path and only the LPC path.
            let path = &a[3];
            let outp = &a[4];
            let raw = std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            });
            // The engine streams, and every piece it streams gets a fresh LPC
            // scratch buffer, so a faithful replay needs the boundaries too:
            // _out/pieces.txt, captured alongside frames.txt.
            let starts: Vec<usize> = a.iter().position(|x| x == "--pieces")
                .map(|i| std::fs::read_to_string(&a[i + 1]).expect("pieces file"))
                .map(|s| s.lines().filter(|l| !l.starts_with('#'))
                     .filter_map(|l| l.trim().parse().ok()).collect())
                .unwrap_or_default();
            let mut st = ceps::LpcState::new(&v);
            let mut n = 0usize;
            for line in raw.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() < 2 {
                    continue;
                }
                let (frame, size) = (f[0].parse::<usize>().unwrap(), f[1].parse::<usize>().unwrap());
                let want = v.frame_size(frame);
                if want != size {
                    println!("period {n}: frame {frame} recorded {want}, engine emitted {size}");
                }
                if starts.contains(&n) {
                    st.new_piece();
                }
                st.emit_period(&v, frame, size, ceps::GAIN_UNITY);
                n += 1;
            }
            println!("{n} periods in {} pieces -> {} samples",
                     starts.len().max(1), st.out.len());
            wav::write(outp, &st.out, v.sps).unwrap();
            println!("wrote {outp}");
        }
        "costs" => {
            // Replay the engine's own cost calls, captured by _out/run_cost.py.
            //
            // Rows are ["tcost"|"fdist"|"shortpen"|"ffint", seq, a, b, c, result].
            // The join costs are checked directly. The `ffint` rows carry the
            // engine's target feature vector for each position, which is the one
            // thing our front end cannot be checked against any other way: with
            // it we can separate "the target cost expression is wrong" from
            // "the features fed to it are wrong".
            let path = &a[3];
            let raw = std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            });
            let mut rows: Vec<(String, i64, String, i64, i64, i64)> = Vec::new();
            for line in raw.lines() {
                let t = line.trim().trim_start_matches('[').trim_end_matches(']');
                let f: Vec<&str> = t.split(',').map(|x| x.trim()).collect();
                if f.len() < 6 {
                    continue;
                }
                let s = |x: &str| x.trim_matches('"').to_string();
                let n = |x: &str| x.trim_matches('"').parse::<i64>().unwrap_or(0);
                rows.push((s(f[0]), n(f[1]), s(f[2]), n(f[3]), n(f[4]), n(f[5])));
            }
            rows.sort_by_key(|r| r.1);
            println!("{} captured calls from {path}", rows.len());

            let unp = v.unit_name_params().expect("unit_name_params");
            let tc = ceps::TargetCost::parse(v.image(), unp).expect("target_cost");

            // join costs
            let mut sel = ceps::Selector::new(&v, ceps::SelectParams::from_voice(&v));
            let (mut jhit, mut jmiss) = (0usize, 0usize);
            for r in rows.iter().filter(|r| r.0 == "fdist") {
                let (u0, u1) = (r.2.parse::<usize>().unwrap_or(0), r.3 as usize);
                // sub_220a0 alone: the short-unit penalty is a separate call
                let mut p = ceps::SelectParams::from_voice(&v);
                p.optimal_coupling = 2;
                let mut s2 = ceps::Selector::new(&v, p);
                if s2.join_cost(u0, u1) as i64 == r.5 { jhit += 1 } else { jmiss += 1 }
            }
            println!("frame distance   {jhit} match, {jmiss} differ");

            // target features, position by position
            let feats = &tc.features;
            let mut engine_vecs: Vec<Vec<i32>> = Vec::new();
            let mut cur: Vec<i32> = Vec::new();
            // The stream carries nested lookups too -- `lisp_pal_lat` asks for
            // `phone_id` while it is being evaluated -- so take names that match
            // the next expected feature and ignore the rest. A fresh
            // `lisp_phone_nameid` starts a new position.
            for r in rows.iter().filter(|r| r.0 == "ffint") {
                if r.2 == feats[0] {
                    cur.clear();
                    cur.push(r.5 as i32);
                    continue;
                }
                let k = cur.len();
                if k > 0 && k < feats.len() && r.2 == feats[k] {
                    cur.push(r.5 as i32);
                    if cur.len() == feats.len() {
                        engine_vecs.push(std::mem::take(&mut cur));
                    }
                }
            }
            println!("engine target vectors recovered: {}", engine_vecs.len());

            // group tcost rows into the same positions: a run of rising unit
            // indices per position, split where the type changes
            let mut groups: Vec<Vec<(usize, i64)>> = Vec::new();
            let mut g: Vec<(usize, i64)> = Vec::new();
            let mut gt: Option<u16> = None;
            for r in rows.iter().filter(|r| r.0 == "tcost") {
                let u = r.2.parse::<usize>().unwrap_or(0);
                if u >= v.num_units {
                    continue;
                }
                let t = v.unit(u).type_id;
                if gt.is_some() && gt != Some(t) {
                    groups.push(std::mem::take(&mut g));
                }
                gt = Some(t);
                g.push((u, r.5));
            }
            if !g.is_empty() {
                groups.push(g);
            }
            println!("tcost groups: {}", groups.len());

            for (i, grp) in groups.iter().enumerate() {
                let Some(tv) = engine_vecs.get(i) else { continue };
                let t = v.unit(grp[0].0).type_id as usize;
                let (mut hit, mut miss) = (0usize, 0usize);
                let mut first_bad = None;
                for &(u, want) in grp {
                    let got = sel.target_cost(tv, u) as i64;
                    if got == want {
                        hit += 1;
                    } else {
                        miss += 1;
                        if first_bad.is_none() {
                            first_bad = Some((u, want, got));
                        }
                    }
                }
                print!("  pos {i:2} type {:<12} {hit:>5} match {miss:>5} differ",
                       v.types[t].name);
                match first_bad {
                    Some((u, want, got)) => println!("   e.g. unit {u}: engine {want}, ours {got}"),
                    None => println!(),
                }
            }

            // and our own front end's vectors against the engine's
            if let Some(i) = a.iter().position(|x| x == "--text") {
                let style = if a.iter().any(|x| x == "--digits") {
                    ceps::DigitStyle::Digits
                } else {
                    ceps::DigitStyle::Cardinal
                };
                let (ph, src) = front_end(&a[i + 1], style, false);
                let nr = ceps::NameRules::parse(v.image(), unp);
                let table = ceps::PhoneTable::learn(&v, &tc);
                let names = nr.name_utterance(&ph, |n| v.type_id(n).is_some());
                let ours = ceps::build_targets(&v, &table, &nr, &tc, &ph);
                println!("\nfront end: {src}, {} names, {} vectors", names.len(), ours.len());

                // Which of our positions the engine's vectors line up with is
                // not fixed: some captures score the pause units and some do
                // not, and a capture can start mid-utterance. Feature 0 is
                // `lisp_phone_nameid`, the unit type, so the alignment is the
                // one that agrees with it most often -- guessing it wrong makes
                // every column look broken by one position.
                let all: Vec<usize> = (0..names.len()).collect();
                let nopau: Vec<usize> = (0..names.len())
                    .filter(|&k| !names[k].contains("pau"))
                    .collect();
                let score_of = |cand: &[usize], off: usize| -> usize {
                    (0..engine_vecs.len().min(cand.len().saturating_sub(off)))
                        .filter(|&k| engine_vecs[k][0] == ours[cand[k + off]][0])
                        .count()
                };
                let mut best = (0usize, all.clone(), 0usize);
                for cand in [&all, &nopau] {
                    for off in 0..4.min(cand.len()) {
                        let s = score_of(cand, off);
                        if s > best.0 {
                            best = (s, cand[off..].to_vec(), off);
                        }
                    }
                }
                let scored = best.1;
                println!("alignment: {} of {} type ids agree (offset {}, {} positions)",
                         best.0, engine_vecs.len(), best.2, scored.len());
                println!("  {}", scored.iter()
                    .map(|&k| format!("{}:{}{}", k, ph[k].phone, ph[k].stress))
                    .collect::<Vec<_>>().join(" "));
                let mut wrong = vec![0usize; feats.len()];
                let n = engine_vecs.len().min(scored.len());
                for k in 0..n {
                    let e = &engine_vecs[k];
                    let o = &ours[scored[k]];
                    for c in 0..feats.len() {
                        if e[c] != o.get(c).copied().unwrap_or(0) {
                            wrong[c] += 1;
                        }
                    }
                }
                println!("\ntarget features vs the engine's own, over {n} positions:");
                let mut rank: Vec<(usize, usize)> =
                    (0..feats.len()).map(|c| (wrong[c], c)).filter(|x| x.0 > 0).collect();
                rank.sort_by(|x, y| y.0.cmp(&x.0));
                for (n_wrong, c) in rank.iter().take(15) {
                    let e: Vec<String> = (0..n).map(|k| engine_vecs[k][*c].to_string()).collect();
                    let o: Vec<String> = (0..n)
                        .map(|k| ours[scored[k]].get(*c).copied().unwrap_or(0).to_string())
                        .collect();
                    println!("  {n_wrong:>3}/{n}  {:<46}", feats[*c]);
                    println!("        engine {}", e.join(" "));
                    println!("        ours   {}", o.join(" "));
                }
                println!("  {}/{} columns match everywhere",
                         feats.len() - rank.len(), feats.len());
            }
        }
        other => {
            eprintln!("unknown command {other}");
            std::process::exit(1);
        }
    }
}

fn report(v: &Voice, pcm: &[i16], dt: std::time::Duration) {
    let dur = pcm.len() as f64 / v.sps as f64;
    println!("{} samples, {dur:.2}s audio in {dt:.3?} ({:.0}x realtime)",
             pcm.len(), dur / dt.as_secs_f64());
}

fn info(v: &Voice) {
    println!("name             {}", v.name());
    println!("sample rate      {} Hz", v.sps);
    println!("LPC order        {}", v.order);
    println!("mcep channels    {}", v.mcep_ch);
    println!("residual fold    {}", v.fold);
    println!("frames           {}", v.num_sts);
    println!("units            {}", v.num_units);
    println!("types            {}", v.types.len());
    println!("unit features    {}", v.unit_feat_width);
    println!("join weights     {:?}", v.join_weights);
    println!("total bytes      {:.1} MB", v.total_bytes() as f64 / 1e6);
}
