// Cepstral's phone set, the engine's rules for a <phoneme ph> it will accept,
// and a readable respelling.
//
// swift.dll (@0x3ae60 / @0x3ac20) splits ph on whitespace and drops the whole
// element, silently, unless every item is a phone of the phoneset with an
// optional digit, a lone 0-2, "." (syllable break) or "|" (word break). A
// self-closing <phoneme .../> does nothing at all: the tag table gives
// PHONEME no empty-tag handler. Everything here exists to catch those.

"use strict";

const Phonemes = (() => {
    // All 121 phones (ceplang_en.dll @0xa8618), case-sensitive.
    const phoneset = new Set([
        "pau", "p", "pH", "b", "t", "tH", "d", "tr", "dr", "tq", "dq", "gq", "dx", "k", "kH",
        "g", "q", "Q", "qq", "pf", "ts", "dz", "ch", "jh", "f", "v", "th", "dh", "s", "z", "sh",
        "zh", "sq", "zq", "c", "cH", "x", "wh", "xq", "h", "hh", "hq", "m", "n", "nr", "nj",
        "ng", "r", "rr", "rx", "rq", "l", "ll", "lq", "j", "y", "w", "yw", "em", "en", "el",
        "i", "I", "ih", "ue", "UE", "e", "E", "eh", "EH", "eo", "oe", "OE", "er", "ae", "AE",
        "ie", "IE", "ah", "AH", "ax", "a", "A", "aa", "oa", "ao", "AO", "o", "O", "uh", "u", "U",
        "uw", "ehn", "oen", "aan", "aon", "in", "eN", "aN", "oN", "uN", "ay", "aw", "oy", "ow",
        "ey", "xi", "xu", "brth", "tsH", "tcH", "tc", "ueh",
    ]);

    // The English voices' phones, for the builder. Cepstral spells the glide
    // `j` and h `h`, and "see" is `i`.
    // [code, kind, example, sound in example, respelling]
    const inventory = [
        ["p", "c", "pen", "p", "p"], ["b", "c", "but", "b", "b"], ["t", "c", "two", "t", "t"],
        ["d", "c", "do", "d", "d"], ["ch", "c", "chair", "ch", "ch"], ["jh", "c", "joy", "j", "j"],
        ["k", "c", "cat", "c", "k"], ["g", "c", "get", "g", "g"], ["f", "c", "fool", "f", "f"],
        ["v", "c", "voice", "v", "v"], ["th", "c", "think", "th", "th"], ["dh", "c", "this", "th", "dh"],
        ["s", "c", "see", "s", "s"], ["z", "c", "zoo", "z", "z"], ["sh", "c", "she", "sh", "sh"],
        ["zh", "c", "pleasure", "s", "zh"], ["h", "c", "ham", "h", "h"], ["m", "c", "man", "m", "m"],
        ["n", "c", "no", "n", "n"], ["ng", "c", "ring", "ng", "ng"], ["l", "c", "left", "l", "l"],
        ["r", "c", "run", "r", "r"], ["w", "c", "we", "w", "w"], ["j", "c", "yes", "y", "y"],

        ["aa", "v", "father", "a", "ah"], ["ae", "v", "cat", "a", "a"], ["ah", "v", "run", "u", "uh"],
        ["ao", "v", "law", "aw", "aw"], ["eh", "v", "bed", "e", "eh"], ["er", "v", "bird", "ir", "ur"],
        ["ih", "v", "city", "i", "ih"], ["i", "v", "see", "ee", "ee"], ["uh", "v", "put", "u", "uu"],
        ["uw", "v", "soon", "oo", "oo"],

        ["ey", "d", "day", "ay", "ay"], ["ay", "d", "my", "y", "y"], ["oy", "d", "boy", "oy", "oy"],
        ["ow", "d", "no", "o", "oh"], ["aw", "d", "now", "ow", "ow"],
    ].map(([code, kind, example, sound, respelling]) =>
        ({ code, kind, example, sound, respelling, stressed: kind !== "c" }));

    const byCode = new Map(inventory.map((p) => [p.code, p]));

    const escapeAttr = (s) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");

    // Open and closed: the element's text is not spoken, the phones are.
    const tag = (ph, word = "") => `<phoneme ph="${escapeAttr(ph)}">${word}</phoneme>`;
    const templatePrefix = '<phoneme ph="';

    const items = (ph) => ph.split(/\s+/).filter((s) => s.length > 0);

    // Mirrors markup::phoneme_items.
    function accepts(item) {
        if (item === "." || item === "|" || item === "0" || item === "1" || item === "2") {
            return true;
        }
        const base = /[0-9]$/.test(item) ? item.slice(0, -1) : item;
        return phoneset.has(base);
    }

    // Why the engine would drop this ph, or [] if it's fine.
    function problems(ph) {
        const all = items(ph);
        if (all.every((s) => s === "|")) {
            return ["no phones"];
        }
        const bad = all.filter((s) => !accepts(s));
        if (bad.length === 0) {
            return [];
        }
        const list = bad.map((s) => `“${s}”`).join(", ");
        return [`unknown ${bad.length === 1 ? "phone" : "phones"} ${list} (case matters)`];
    }

    // Rough syllabified respelling with the primary-stressed syllable in
    // capitals: "t ah0 m ey1 t ow0" -> "tuh-MAY-toh". "|" separates words.
    // Between two vowels a single consonant starts the next syllable; with two
    // or more, the first closes the previous one.
    function respellWord(raw) {
        const tokens = [];
        for (const item of raw) {
            let base = item;
            let stress = 0;
            if (base.length > 1 && /[0-9]$/.test(base)) {
                stress = Number(base.slice(-1));
                base = base.slice(0, -1);
            }
            const p = byCode.get(base);
            if (p) {
                tokens.push({ p, stress });
            }
        }
        const nuclei = tokens.map((t, i) => (t.p.stressed ? i : -1)).filter((i) => i >= 0);
        if (nuclei.length === 0) {
            return tokens.map((t) => t.p.respelling).join("");
        }
        const starts = [0];
        for (let k = 1; k < nuclei.length; k++) {
            const gap = nuclei[k] - nuclei[k - 1] - 1;
            starts.push(gap >= 2 ? nuclei[k - 1] + 2 : nuclei[k] - gap);
        }
        return starts.map((start, i) => {
            const end = i + 1 < starts.length ? starts[i + 1] : tokens.length;
            let syl = "";
            for (let idx = start; idx < end; idx++) {
                const p = tokens[idx].p;
                // "eye" alone ("EYE-dee-uh"), "y" after an onset ("MY").
                syl += p.code === "ay" && idx === start ? "eye" : p.respelling;
            }
            return tokens[nuclei[i]].stress === 1 ? syl.toUpperCase() : syl;
        }).join("-");
    }

    const respell = (ph) => ph.split("|").map((w) => respellWord(items(w)))
        .filter((s) => s.length > 0).join(" ");

    // Every <phoneme> in a document, with where its ph value sits.
    function elements(text) {
        const out = [];
        const re = /<phoneme\b([^>]*?)(\/>|>[\s\S]*?<\/phoneme\s*>)/g;
        let m;
        while ((m = re.exec(text)) !== null) {
            const attrs = m[1];
            const attrsAt = m.index + "<phoneme".length;
            const ph = /\bph\s*=\s*"([^"]*)"/.exec(attrs);
            const alphabet = /\balphabet\s*=\s*"([^"]*)"/.exec(attrs);
            out.push({
                start: m.index,
                end: m.index + m[0].length,
                ph: ph ? ph[1] : null,
                phStart: ph ? attrsAt + ph.index + ph[0].indexOf('"') + 1 : -1,
                alphabet: alphabet ? alphabet[1] : null,
                selfClosing: m[2] === "/>",
                source: m[0],
            });
        }
        return out;
    }

    const isCepstral = (el) => el.alphabet === null || el.alphabet === "cepstral_en";

    // One line per <phoneme> the engine would skip. SAMPA tags aren't checked.
    function documentProblems(text) {
        return elements(text).flatMap((el) => {
            let issues = [];
            if (el.selfClosing) {
                issues = ['a self-closing <phoneme/> does nothing; use <phoneme ph="…"></phoneme>'];
            } else if (isCepstral(el)) {
                issues = el.ph === null ? ["no ph attribute"] : problems(el.ph);
            } else if (el.alphabet !== "sampa") {
                issues = [`unknown alphabet “${el.alphabet}”`];
            }
            if (issues.length === 0) {
                return [];
            }
            const snippet = el.source.length > 60 ? el.source.slice(0, 57) + "…" : el.source;
            return [`${snippet} — ${issues.join("; ")}`];
        });
    }

    return { inventory, tag, templatePrefix, items, problems, respell, elements, isCepstral,
             documentProblems };
})();
