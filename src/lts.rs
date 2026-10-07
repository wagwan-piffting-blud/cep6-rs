//! Letter to sound, for words the lexicon does not hold.
//!
//! festival's `cmu_lts_rules.scm` is one CART per letter, each testing the three
//! letters either side of the current one and leafing to the phones that letter
//! spells -- `_epsilon_` where it spells nothing, and occasionally several phones
//! at once, since `x` is /k s/ and `u` after some letters is /j uw/.
//!
//! Its own build log puts it at 89.8% of phones and 51.9% of whole words correct,
//! which is why it is the fallback and not the primary: the lexicon answers first
//! and this only runs on what is left. Names and acronyms are exactly the hard
//! case, and exactly what a warning message is for.
//!
//! `_out/pack_lts.py` flattens the 50,952 nodes to 224 KB, folding the alphabet
//! the same way the lexicon does.

use crate::rules::Phone;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

const MAGIC: &[u8; 8] = b"CEPLTS01";
/// Padding outside the word, as festival writes it in the rules.
const PAD: u8 = b'#';

pub struct Lts {
    buf: Vec<u8>,
    letters: BTreeMap<u8, usize>,
    groups: Vec<Vec<(String, u8)>>,
    nodes_off: usize,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

impl Lts {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Lts> {
        let buf = std::fs::read(path)?;
        Lts::from_bytes(buf)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad LTS pack"))
    }

    pub fn from_bytes(buf: Vec<u8>) -> Option<Lts> {
        if buf.len() < 24 || &buf[..8] != MAGIC {
            return None;
        }
        let n_letters = u32le(&buf, 8) as usize;
        let n_groups = u32le(&buf, 12) as usize;
        let letters_off = u32le(&buf, 16) as usize;
        let nodes_off = u32le(&buf, 20) as usize;

        let mut o = 24;
        let mut groups = Vec::with_capacity(n_groups);
        for _ in 0..n_groups {
            let count = *buf.get(o)? as usize;
            o += 1;
            let mut g = Vec::with_capacity(count);
            for _ in 0..count {
                let len = *buf.get(o)? as usize;
                o += 1;
                let name = String::from_utf8_lossy(buf.get(o..o + len)?).into_owned();
                o += len;
                let stress = *buf.get(o)?;
                o += 1;
                g.push((name, stress));
            }
            groups.push(g);
        }

        let mut letters = BTreeMap::new();
        for i in 0..n_letters {
            let p = letters_off + 5 * i;
            letters.insert(*buf.get(p)?, u32le(&buf, p + 1) as usize);
        }
        Some(Lts { buf, letters, groups, nodes_off })
    }

    pub fn letters(&self) -> usize {
        self.letters.len()
    }

    /// The six context letters around position `i`, in the order the pack encodes
    /// them: p.p.p, p.p, p, n, n.n, n.n.n. Outside the word is `#`.
    fn context(w: &[u8], i: usize) -> [u8; 6] {
        let at = |d: i64| -> u8 {
            let k = i as i64 + d;
            if k < 0 || k >= w.len() as i64 {
                PAD
            } else {
                w[k as usize]
            }
        };
        [at(-3), at(-2), at(-1), at(1), at(2), at(3)]
    }

    fn walk(&self, mut off: usize, ctx: &[u8; 6]) -> Option<&[(String, u8)]> {
        let b = &self.buf;
        loop {
            let p = self.nodes_off + off;
            match *b.get(p)? {
                0 => return self.groups.get(*b.get(p + 1)? as usize).map(|g| g.as_slice()),
                1 => {
                    let feat = *b.get(p + 1)? as usize;
                    let val = *b.get(p + 2)?;
                    if ctx.get(feat).copied() == Some(val) {
                        off += 7;
                    } else {
                        off = u32le(b, p + 3) as usize;
                    }
                }
                _ => return None,
            }
        }
    }

    /// Phones for a word not in the lexicon. Letters with no tree -- digits,
    /// apostrophes, anything non-ASCII -- are skipped rather than guessed at.
    ///
    /// Syllable boundaries are left unset: the rules predict phones, not
    /// structure, so `NameRules` falls back to maximal onset for these words.
    pub fn predict(&self, word: &str) -> Vec<Phone> {
        let w: Vec<u8> = word
            .to_lowercase()
            .bytes()
            .filter(|c| self.letters.contains_key(c))
            .collect();
        let mut out = Vec::new();
        for i in 0..w.len() {
            let Some(&root) = self.letters.get(&w[i]) else { continue };
            let ctx = Lts::context(&w, i);
            let Some(group) = self.walk(root, &ctx) else { continue };
            for (name, stress) in group {
                out.push(Phone {
                    phone: name.clone(),
                    stress: *stress,
                    word: Some(word.to_lowercase()),
                    word_id: 0,
                    syl_end: None,
                    dur: None,
                });
            }
        }
        out
    }
}
