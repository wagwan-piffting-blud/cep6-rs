//! Print each utterance's unit names and chosen start frames for marked-up
//! text, to line up against `_out/run_split.py`'s walk rows.
//!
//!     cargo run --release --example units_probe -- ../William "markup"

use ceps::expr::TargetCost;
use ceps::select::{SelectParams, Selector};
use ceps::{CepLex, NameRules, PhoneTable, PosTag, Voice};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let v = Voice::open(&a[1]).expect("voice");
    let cep = CepLex::open("packs/ceplex.bin").expect("ceplex");
    let pos = PosTag::open("packs/pos.bin").ok();
    let m = ceps::markup::parse(&a[2]).expect("markup");
    let (seq, _, _, _) = ceps::lex::text_to_phones_marked(&cep, pos.as_ref(), &m,
                                                          ceps::DigitStyle::Cardinal);
    let unp = v.unit_name_params().expect("unit_name_params");
    let nr = NameRules::parse(v.image(), unp);
    let tc = TargetCost::parse(v.image(), unp).expect("target_cost");
    let table = PhoneTable::learn(&v, &tc);
    let shown: Vec<String> = seq.iter().map(|p| format!("{}{}{}", p.phone, p.stress,
        match p.syl_end { Some(true) => "|", _ => "" })).collect();
    println!("phones: {}", shown.join(" "));
    for phones in ceps::speak::utterances(&seq) {
        let names = nr.name_utterance(&phones, |n| v.type_id(n).is_some());
        let types: Vec<usize> = names.iter().filter_map(|n| v.type_id(n)).collect();
        let tf = ceps::target::build(&v, &table, &nr, &tc, &phones);
        let mut sel = Selector::new(&v, SelectParams::from_voice(&v));
        let chosen = sel.select_with_targets(&types, Some(&tf));
        println!("utt: {}", names.join(" "));
        let starts: Vec<String> = chosen.iter().map(|c| c.start.to_string()).collect();
        println!("     {}", starts.join(" "));
    }
}
