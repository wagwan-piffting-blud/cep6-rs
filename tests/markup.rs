//! Markup against the engine: FNV-1a hashes of SAPI renders. `markup_engine.tsv`
//! comes from `_fe/markup_check.ps1` + `_fe/markup_hashes.py`; the port volume
//! rows from `_fe/volume_sweep.ps1`. Needs CEPS_VOICE_ROOT; skips silently
//! otherwise.

use ceps::speak::Port;
use ceps::{CepLex, DigitStyle, PosTag, Voice};

fn fnv1a(pcm: &[i16]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &s in pcm {
        for b in s.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

fn william() -> Option<Voice> {
    let root = std::env::var("CEPS_VOICE_ROOT").ok()?;
    Voice::open(std::path::Path::new(&root).join("William")).ok()
}

fn render(v: &Voice, src: &str, port: Port, port_volume: u32) -> Vec<i16> {
    let cep = CepLex::open(std::env::var("CEPS_CEPLEX")
        .unwrap_or_else(|_| "packs/ceplex.bin".into())).expect("ceplex pack");
    let pos = PosTag::open("packs/pos.bin").ok();
    let m = ceps::markup::parse(src).expect("markup");
    let mut pcm = Vec::new();
    ceps::speak::say_marked_each(v, &cep, pos.as_ref(), &m, DigitStyle::Cardinal, port,
                                 |s| pcm.extend(s.pcm));
    ceps::speak::apply_volume(&mut pcm, port_volume);
    pcm
}

#[test]
fn markup_matches_the_engine() {
    let Some(v) = william() else { return };
    let mut failed = Vec::new();
    let mut n = 0;
    for line in include_str!("markup_engine.tsv").lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let (name, src) = (f[0], f[1]);
        let len: usize = f[2].parse().unwrap();
        let want = u64::from_str_radix(f[3], 16).unwrap();
        let pcm = render(&v, src, Port::default(), 100);
        if pcm.len() != len || fnv1a(&pcm) != want {
            failed.push(format!("{name}: {} samples, want {len}", pcm.len()));
        }
        n += 1;
    }
    assert!(n > 0);
    assert!(failed.is_empty(), "{} of {n} differ:\n{}", failed.len(), failed.join("\n"));
}

#[test]
fn port_volume_matches_the_engine() {
    let Some(v) = william() else { return };
    // integer x * p / 100 is wrong at 13 and 21; the engine multiplies in f32
    for (port, n, want) in [(13, 29042, 0xd78420b835d73341u64),
                            (21, 29042, 0xedc0b40f1d774b19),
                            (50, 29042, 0x1654cd6ad0db5efc)] {
        let pcm = render(&v, "Brown fox jumps.", Port::default(), port);
        assert_eq!(pcm.len(), n, "{port}%");
        assert_eq!(fnv1a(&pcm), want, "{port}%");
    }
}

/// SpVoice.Rate, alone and under markup: `_out/run_params.py` renders hashed by
/// `_fe/rate_hashes.py`. The combined line has a markup rate, a break and a
/// pitch, so the break's length scaling with the port is covered too.
#[test]
fn port_rate_matches_the_engine() {
    let Some(v) = william() else { return };
    let plain = "The quick brown fox jumps over the lazy dog.";
    let combo = "The quick <rate absspeed=\"4\">brown fox</rate> <silence msec=\"500\"/> \
                 jumps <pitch absmiddle=\"6\">over</pitch> the lazy dog.";
    for (which, rate, n, want) in [("vrate", -10, 138488, 0x586a726c9c825e6eu64),
                                   ("vrate", -8, 107472, 0x50d962518eaffc00),
                                   ("vrate", -3, 64772, 0xeb4eb8f03e7e1d72),
                                   ("vrate", 1, 43178, 0x49800b74847707ef),
                                   ("vrate", 5, 27500, 0x0508f45101358737),
                                   ("vrate", 10, 19084, 0x985b5abf5bdeb3da),
                                   ("combo", -6, 92182, 0x886bdf82fc6f0a1a),
                                   ("combo", 3, 37338, 0x70138cff7f018464)] {
        let src = if which == "vrate" { plain } else { combo };
        let port = Port::default().with_wpm(Port::sapi_wpm(rate));
        let pcm = render(&v, src, port, 100);
        assert_eq!(pcm.len(), n, "{which} at rate {rate}");
        assert_eq!(fnv1a(&pcm), want, "{which} at rate {rate}");
    }
}

/// `ceps::render`, which the GUI calls, against the same hashes: every markup
/// case, SpVoice.Rate and SpVoice.Volume through `Options`.
#[test]
fn render_api_matches_the_engine() {
    let Some(v) = william() else { return };
    let cep = CepLex::open(std::env::var("CEPS_CEPLEX")
        .unwrap_or_else(|_| "packs/ceplex.bin".into())).expect("ceplex pack");
    let pos = PosTag::open("packs/pos.bin").ok();
    let run = |src: &str, o: ceps::render::Options| {
        ceps::render::render(&v, &cep, pos.as_ref(), src, &o).expect("markup").0
    };
    let marked = ceps::render::Options { markup: true, ..Default::default() };
    for line in include_str!("markup_engine.tsv").lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let pcm = run(f[1], marked);
        assert_eq!((pcm.len(), fnv1a(&pcm)),
                   (f[2].parse().unwrap(), u64::from_str_radix(f[3], 16).unwrap()), "{}", f[0]);
    }
    let pcm = run("The quick brown fox jumps over the lazy dog.",
                  ceps::render::Options { rate: -8, ..marked });
    assert_eq!((pcm.len(), fnv1a(&pcm)), (107472, 0x50d962518eaffc00));
    let pcm = run("Brown fox jumps.", ceps::render::Options { volume: 21, ..Default::default() });
    assert_eq!((pcm.len(), fnv1a(&pcm)), (29042, 0xedc0b40f1d774b19));
}

#[test]
fn sapi_rate_becomes_the_engines_wpm() {
    let got: Vec<u32> = (-10..=10).map(Port::sapi_wpm).collect();
    assert_eq!(got, [64, 74, 85, 95, 105, 115, 125, 136, 146, 156, 170,
                     195, 221, 246, 272, 297, 323, 348, 374, 399, 425]);
}
