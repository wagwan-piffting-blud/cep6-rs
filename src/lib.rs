//! Reader and bit-exact clunits synthesiser for Cepstral Swift 6.2 voices.

pub mod cart;
pub mod aswd;
pub mod carts;
pub mod rx;
pub mod token;
mod t2w;
pub mod token_tables;
pub mod dur;
pub mod phonedist;
pub mod phoneset;
pub mod db;
pub mod expr;
pub mod f0;
pub mod ceplex;
pub mod lex;
pub mod lts;
pub mod markup;
pub mod norm;
pub mod postag;
pub mod postlex;
pub mod prosody;
pub mod render;
pub mod rules;
pub mod score;
pub mod select;
pub mod speak;
pub mod synth;
pub mod trace;
pub mod target;
pub mod ulaw;
pub mod val;
pub mod wav;

pub use cart::{Cart, DurStat, DurStats, FeatVal, Features};
pub use db::{Unit, UnitType, Voice, UNIT_NONE};
pub use prosody::{concat_units, f0_targets_to_pm, join_units_modified_lpc,
                  join_units_simple, join_units_streamed, synth_periods, F0Target, Period,
                  TargetUnit};
pub use expr::TargetCost;
pub use f0::{natural_f0, FlatProsody};
pub use rules::{NameRules, Pau, Phone, PhoneCtx};
pub use score::Score;
pub use select::{Choice, SelectParams, Selector};
pub use synth::{synth_frames, synth_units, LpcState, GAIN_UNITY};
pub use target::{build as build_targets, plan, PhoneTable, Pos};
pub use lex::{text_to_phones, text_to_phones_cepstral, text_to_phones_styled,
              text_to_phones_with,
              tokenize, Lexicon, Source,
              Syl, Token};
pub use lts::Lts;
pub use markup::{Ctl, Marked};
pub use norm::{normalize, normalize_with, DigitStyle};
pub use trace::{Coeffs, Hit, Trace, Tracer};
pub use ceplex::CepLex;
pub use postag::PosTag;
pub use speak::{say, say_pcm, say_phones, say_tagged, Said};
