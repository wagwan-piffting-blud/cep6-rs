//! Render text with one residual gain on every unit, to test how the engine's
//! `<prosody volume>` reaches the LPC filter.
//!
//!     cargo run --release --example gain_probe -- ../William 0.5 "text" out.wav [word,word]

use ceps::expr::TargetCost;
use ceps::prosody::{gain_from_rescale, join_units_streamed, TargetUnit};
use ceps::select::{SelectParams, Selector};
use ceps::{CepLex, NameRules, PhoneTable, PosTag, Voice};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let v = Voice::open(&a[1]).expect("voice");
    let gain = gain_from_rescale(a[2].parse().expect("gain"));
    let cep = CepLex::open("packs/ceplex.bin").expect("ceplex");
    let pos = PosTag::open("packs/pos.bin").ok();
    let (seq, _, _) = ceps::lex::text_to_phones_tagged(&cep, pos.as_ref(), &a[3],
                                                       ceps::DigitStyle::Cardinal);

    let unp = v.unit_name_params().expect("unit_name_params");
    let nr = NameRules::parse(v.image(), unp);
    let tc = TargetCost::parse(v.image(), unp).expect("target_cost");
    let table = PhoneTable::learn(&v, &tc);
    // an optional comma list: only the units of these words get the gain
    let only: Vec<&str> = a.get(5).map(|s| s.split(',').collect()).unwrap_or_default();
    let mut pcm = Vec::new();
    for phones in ceps::speak::utterances(&seq) {
        let names = nr.name_utterance(&phones, |n| v.type_id(n).is_some());
        let types: Vec<usize> = names.iter().filter_map(|n| v.type_id(n)).collect();
        let tf = ceps::target::build(&v, &table, &nr, &tc, &phones);
        let mut sel = Selector::new(&v, SelectParams::from_voice(&v));
        let chosen = sel.select_with_targets(&types, Some(&tf));
        let units: Vec<TargetUnit> = chosen.iter().zip(&phones).map(|(c, p)| {
            let hit = only.is_empty()
                || p.word.as_deref().is_some_and(|w| only.contains(&w));
            TargetUnit {
                start: c.start, end: c.end, target_end: 0,
                gain: if hit { gain } else { ceps::GAIN_UNITY },
            }
        }).collect();
        pcm.extend(join_units_streamed(&v, &units, &sel.last_pieces));
    }
    ceps::wav::write(&a[4], &pcm, v.sps).expect("write");
}
