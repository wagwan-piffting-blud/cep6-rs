// Ceps front end: the text box, the voice controls, Speak/Pause/Continue +
// Stop, Save to File, the Markup menus and the Phoneme Builder. Synthesis is
// the Rust side's `speak` / `save`; this only edits text and plays the WAV.

"use strict";

const { invoke } = window.__TAURI__.core;
const dialog = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);
const text = $("text");
const voiceSel = $("voice");
const readMarkup = $("readMarkup");
const speakBtn = $("speakBtn");
const stopBtn = $("stopBtn");
const saveBtn = $("saveBtn");
const spinner = $("spinner");
const status = $("status");

// The loaded audio and the request it was made from. Speak replays or resumes
// it while they still match, and re-synthesizes once they don't.
let audio = null;
let audioKey = null;
let playback = "idle"; // idle | playing | paused
let busy = false;

// ------------------------------------------------------------------ settings

const SETTINGS = [
    { key: "rate", def: 0, min: -10, max: 10 },
    { key: "pitch", def: 100, min: 50, max: 200 },
    { key: "volume", def: 100, min: 0, max: 100 },
];

function stored(key, def) {
    try {
        const v = localStorage.getItem("ceps." + key);
        return v === null ? def : JSON.parse(v);
    } catch {
        return def;
    }
}

function store(key, v) {
    try {
        localStorage.setItem("ceps." + key, JSON.stringify(v));
    } catch {
        // private window or blocked storage: settings just aren't remembered
    }
}

const clamp = (v, s) => Math.min(s.max, Math.max(s.min, Math.round(v)));

function setValue(s, v) {
    v = clamp(v, s);
    $(s.key).value = v;
    $(s.key + "N").value = v;
    store(s.key, v);
    refreshTransport();
}

for (const s of SETTINGS) {
    const range = $(s.key);
    const num = $(s.key + "N");
    range.addEventListener("input", () => setValue(s, Number(range.value)));
    // Unparseable input snaps back; out-of-range input clamps.
    num.addEventListener("change", () => {
        const v = Number(num.value);
        setValue(s, Number.isFinite(v) && num.value.trim() !== "" ? v : Number(range.value));
    });
    setValue(s, stored(s.key, s.def));
}
for (let r = -10; r <= 10; r++) {
    $("rateTicks").append(new Option("", String(r)));
}

readMarkup.checked = stored("markup", true);
readMarkup.addEventListener("change", () => {
    store("markup", readMarkup.checked);
    refreshTransport();
});
voiceSel.addEventListener("change", () => {
    store("voice", voiceSel.value);
    refreshTransport();
});

$("defaultsBtn").addEventListener("click", () => {
    for (const s of SETTINGS) {
        setValue(s, s.def);
    }
});

function enableMarkup() {
    if (!readMarkup.checked) {
        readMarkup.checked = true;
        store("markup", true);
    }
}

function request(t, forceMarkup = false) {
    return {
        text: t,
        voice: voiceSel.value,
        markup: readMarkup.checked || forceMarkup,
        rate: Number($("rate").value),
        pitch: Number($("pitch").value),
        volume: Number($("volume").value),
    };
}

// ----------------------------------------------------------------- transport

function requestKey(t) {
    return JSON.stringify(request(t));
}

function setPlayback(p) {
    playback = p;
    refreshTransport();
}

function refreshTransport() {
    const stale = audioKey !== requestKey(text.value.trim());
    speakBtn.textContent = playback === "playing" ? "Pause"
        : playback === "paused" && !stale ? "Continue" : "Speak";
    stopBtn.disabled = playback === "idle";
    speakBtn.disabled = saveBtn.disabled = busy || voiceSel.options.length === 0;
}

function setBusy(b) {
    busy = b;
    spinner.hidden = !b;
    refreshTransport();
}

function showStatus(msg, isError = false) {
    status.textContent = msg;
    status.title = msg;
    status.classList.toggle("error", isError);
}

function describe(notes) {
    const parts = [`${notes.seconds.toFixed(2)} s of audio`];
    if (notes.ignored.length) {
        parts.push("kept as text only: " + notes.ignored.join(", "));
    }
    if (notes.missing.length) {
        parts.push("could not pronounce: " + notes.missing.join(", "));
    }
    return parts.join(" · ");
}

function stopAudio() {
    if (audio) {
        audio.pause();
        URL.revokeObjectURL(audio.src);
    }
    audio = null;
    audioKey = null;
}

async function speakText(t, forceMarkup = false) {
    stopAudio();
    setPlayback("idle");
    const req = request(t, forceMarkup);
    const key = JSON.stringify(req);
    setBusy(true);
    try {
        let wav = await invoke("speak", { req });
        // raw bytes come back as an ArrayBuffer over the ipc: protocol, but as
        // a plain array of numbers if Tauri had to fall back to postMessage
        if (Array.isArray(wav)) {
            wav = new Uint8Array(wav);
        }
        const url = URL.createObjectURL(new Blob([wav], { type: "audio/wav" }));
        const a = new Audio(url);
        a.addEventListener("ended", () => {
            if (audio === a) {
                setPlayback("idle");
            }
        });
        audio = a;
        // a builder preview is not the document's audio
        audioKey = forceMarkup ? null : key;
        showStatus(describe(await invoke("notes")));
        await a.play();
        setPlayback("playing");
    } catch (e) {
        showStatus(String(e), true);
    } finally {
        setBusy(false);
    }
}

async function speak() {
    if (busy) {
        return;
    }
    if (playback === "playing") {
        audio.pause();
        setPlayback("paused");
        return;
    }
    const t = text.value.trim();
    if (!t) {
        return;
    }
    if (audio && audioKey === requestKey(t)) {
        if (playback === "idle") {
            audio.currentTime = 0;
        }
        await audio.play();
        setPlayback("playing");
        return;
    }
    if (await confirmMarkup(t, "Speak")) {
        await speakText(t);
    }
}

function stop() {
    if (audio) {
        audio.pause();
        audio.currentTime = 0;
    }
    setPlayback("idle");
}

async function save() {
    const t = text.value.trim();
    if (busy || !t || !(await confirmMarkup(t, "Save"))) {
        return;
    }
    const path = await dialog.save({
        defaultPath: `${voiceSel.value || "ceps"}.wav`,
        filters: [{ name: "WAV audio", extensions: ["wav"] }],
    });
    if (!path) {
        return;
    }
    setBusy(true);
    try {
        const notes = await invoke("save", { req: request(t), path });
        showStatus(`Saved ${path.split(/[\\/]/).pop()} · ` + describe(notes));
    } catch (e) {
        showStatus(String(e), true);
    } finally {
        setBusy(false);
    }
}

speakBtn.addEventListener("click", speak);
stopBtn.addEventListener("click", stop);
saveBtn.addEventListener("click", save);
text.addEventListener("input", refreshTransport);

// A parse error and any <phoneme> the engine would drop, before rendering.
async function confirmMarkup(t, verb) {
    if (!readMarkup.checked) {
        return true;
    }
    const check = await invoke("check", { text: t });
    if (check.error) {
        showStatus("markup: " + check.error, true);
        return false;
    }
    const issues = Phonemes.documentProblems(t);
    if (issues.length === 0) {
        return true;
    }
    return ask(issues.length === 1 ? "A phoneme tag will be skipped"
                                    : `${issues.length} phoneme tags will be skipped`,
               "The engine silently drops a tag it can’t read, so that word will be missing "
               + "from the audio.\n\n" + issues.slice(0, 6).join("\n") + (issues.length > 6 ? "\n…" : ""),
               `${verb} Anyway`);
}

function ask(title, body, okLabel) {
    const d = $("confirm");
    $("confirmTitle").textContent = title;
    $("confirmBody").textContent = body;
    $("confirmOk").textContent = okLabel;
    d.returnValue = "cancel";
    d.showModal();
    return new Promise((resolve) => {
        d.addEventListener("close", () => resolve(d.returnValue === "ok"), { once: true });
    });
}

// ------------------------------------------------------------------- editing

// execCommand keeps the insertion on the textarea's undo stack.
function replaceRange(start, end, str) {
    text.focus();
    text.setSelectionRange(start, end);
    if (!document.execCommand("insertText", false, str)) {
        text.setRangeText(str, start, end, "end");
    }
    refreshTransport();
}

function wrap(open, close) {
    const { selectionStart: s, selectionEnd: e } = text;
    const selected = text.value.slice(s, e);
    replaceRange(s, e, open + selected + close);
    const at = s + open.length;
    text.setSelectionRange(at, close ? at + selected.length : at);
    enableMarkup();
}

// --------------------------------------------------------------------- menus

// [title, open, close] rows; null is a separator. What each does is in
// HANDOFF item 8.
const MENUS = {
    "Pause": [
        ["250 ms", '<break time="250ms"/>', ""],
        ["500 ms", '<break time="500ms"/>', ""],
        ["1 second", '<break time="1s"/>', ""],
        ["2 seconds", '<break time="2s"/>', ""],
        null,
        ["Sentence break", "<break/>", ""],
        ["Paragraph break", '<break strength="strong"/>', ""],
        ["Phrase break (no pause of its own)", '<break strength="weak"/>', ""],
    ],
    "Rate": ["x-slow", "slow", "fast", "x-fast"]
        .map((k) => [k, `<prosody rate="${k}">`, "</prosody>"]),
    "Pitch": [
        ...["x-low", "low", "high", "x-high"].map((k) => [k, `<prosody pitch="${k}">`, "</prosody>"]),
        null,
        ["Up 4 semitones", '<prosody pitch="+4st">', "</prosody>"],
        ["Down 4 semitones", '<prosody pitch="-4st">', "</prosody>"],
    ],
    "Volume": ["silent", "x-soft", "soft", "loud", "x-loud"]
        .map((k) => [k, `<prosody volume="${k}">`, "</prosody>"]),
    "Say As": [
        ["Spell out", '<say-as type="spell-out">', "</say-as>"],
        ["Spell out with punctuation", '<say-as type="literal">', "</say-as>"],
        ["Digits", '<say-as interpret-as="digits">', "</say-as>"],
        ["Cardinal number", '<say-as interpret-as="cardinal">', "</say-as>"],
        ["Ordinal number", '<say-as interpret-as="ordinal">', "</say-as>"],
        ["Date (month/day/year)", '<say-as interpret-as="date" format="mdy">', "</say-as>"],
        ["Date (day/month/year)", '<say-as interpret-as="date" format="dmy">', "</say-as>"],
        ["Date (year-month-day)", '<say-as interpret-as="date" format="ymd">', "</say-as>"],
        ["Time", '<say-as type="time">', "</say-as>"],
        ["Address", '<say-as interpret-as="address">', "</say-as>"],
        ["Measure", '<say-as type="measure">', "</say-as>"],
        ["Plain words", '<say-as type="words">', "</say-as>"],
        null,
        ["Strong emphasis", '<emphasis level="strong">', "</emphasis>"],
    ],
    "Phoneme": [
        ["Insert phoneme template   Ctrl/⌘ ⇧ T", insertPhonemeTemplate],
        ["Phoneme Builder…   Ctrl/⌘ ⇧ P", openBuilder],
    ],
};

function closeMenus(except = null) {
    for (const ul of document.querySelectorAll(".menu ul")) {
        if (ul !== except) {
            ul.hidden = true;
        }
    }
}

for (const holder of document.querySelectorAll(".menu")) {
    const title = holder.dataset.title;
    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = title;
    const ul = document.createElement("ul");
    ul.hidden = true;
    for (const row of MENUS[title]) {
        const li = document.createElement("li");
        if (row === null) {
            li.className = "sep";
        } else {
            const b = document.createElement("button");
            b.type = "button";
            b.textContent = row[0];
            b.addEventListener("click", () => {
                closeMenus();
                if (typeof row[1] === "function") {
                    row[1]();
                } else {
                    wrap(row[1], row[2]);
                }
            });
            li.append(b);
        }
        ul.append(li);
    }
    // keep the textarea's selection: the menu must not take focus on press
    holder.addEventListener("mousedown", (e) => e.preventDefault());
    btn.addEventListener("click", () => {
        closeMenus(ul);
        ul.hidden = !ul.hidden;
    });
    holder.append(btn, ul);
}
document.addEventListener("mousedown", (e) => {
    if (!e.target.closest(".menu")) {
        closeMenus();
    }
});

const emphasisBtn = document.querySelector("nav#markup > button[data-wrap]");
emphasisBtn.addEventListener("mousedown", (e) => e.preventDefault());
emphasisBtn.addEventListener("click", () => wrap(...JSON.parse(emphasisBtn.dataset.wrap)));

// ------------------------------------------------------------------ phonemes

function insertPhonemeTemplate() {
    const { selectionStart: s, selectionEnd: e } = text;
    replaceRange(s, e, Phonemes.tag("", text.value.slice(s, e)));
    const at = s + Phonemes.templatePrefix.length;
    text.setSelectionRange(at, at);
    enableMarkup();
}

// With the caret inside a <phoneme>, the builder edits that tag's ph;
// otherwise it inserts a new one, wrapping a selected plain word.
let editing = null;

function elementAtCaret() {
    const { selectionStart: s, selectionEnd: e } = text;
    return Phonemes.elements(text.value).find((el) =>
        Phonemes.isCepstral(el) && el.ph !== null && s > el.start && e < el.end) || null;
}

const builder = $("builder");
const phField = $("phField");

function buildGrids() {
    Phonemes.inventory.forEach((p) => {
        const b = document.createElement("button");
        b.type = "button";
        b.title = `${p.code} as in “${p.example}”`
            + (p.code === "ah" ? " — unstressed ah0 is the “a” in “about”" : "");
        const i = p.example.toLowerCase().indexOf(p.sound);
        const ex = i < 0 ? p.example
            : `${p.example.slice(0, i)}<em>${p.example.slice(i, i + p.sound.length)}</em>`
              + p.example.slice(i + p.sound.length);
        b.innerHTML = `<b>${p.code}</b><small>${ex}</small>`;
        b.addEventListener("click", () => {
            const stress = builder.querySelector("input[name=stress]:checked").value;
            appendItem(p.stressed ? p.code + stress : p.code);
        });
        builder.querySelector(`.grid[data-kind=${p.kind}]`).append(b);
    });
}

function appendItem(item) {
    const cur = phField.value.trim();
    phField.value = cur ? `${cur} ${item}` : item;
    refreshBuilder();
}

function refreshBuilder() {
    const ph = phField.value;
    const issues = Phonemes.problems(ph);
    const respelled = Phonemes.respell(ph);
    $("readsAs").textContent = respelled ? "Reads as:  " + respelled : " ";
    const st = $("phStatus");
    if (!ph.trim()) {
        st.textContent = " ";
        st.classList.remove("bad");
    } else if (issues.length === 0) {
        st.textContent = `${Phonemes.items(ph).length} items`;
        st.classList.remove("bad");
    } else {
        st.textContent = issues.join("; ") + " — the engine would skip this word entirely";
        st.classList.add("bad");
    }
    $("phPreview").disabled = issues.length > 0;
    $("phCommit").disabled = issues.length > 0;
}

function openBuilder() {
    if (builder.open) {
        return;
    }
    editing = elementAtCaret();
    phField.value = editing ? editing.ph : "";
    $("phCommit").textContent = editing ? "Update" : "Insert";
    refreshBuilder();
    builder.showModal();
    phField.focus();
}

function commitBuilder() {
    const ph = Phonemes.items(phField.value).join(" ");
    if (Phonemes.problems(ph).length > 0) {
        return;
    }
    builder.close();
    enableMarkup();
    if (editing) {
        replaceRange(editing.phStart, editing.phStart + editing.ph.length, ph);
        return;
    }
    const { selectionStart: s, selectionEnd: e } = text;
    const selected = text.value.slice(s, e);
    if (/[<>]/.test(selected)) {
        // Selection holds markup; don't nest tags, just add ours after it.
        replaceRange(e, e, Phonemes.tag(ph));
    } else {
        replaceRange(s, e, Phonemes.tag(ph, selected));
    }
}

buildGrids();
phField.addEventListener("input", refreshBuilder);
phField.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
        e.preventDefault();
        commitBuilder();
    }
});
$("sylBreak").addEventListener("click", () => appendItem("."));
$("wordBreak").addEventListener("click", () => appendItem("|"));
$("phRemove").addEventListener("click", () => {
    const items = Phonemes.items(phField.value);
    items.pop();
    phField.value = items.join(" ");
    refreshBuilder();
});
$("phClear").addEventListener("click", () => {
    phField.value = "";
    refreshBuilder();
});
$("phPreview").addEventListener("click", () =>
    speakText(Phonemes.tag(Phonemes.items(phField.value).join(" ")), true));
$("phCancel").addEventListener("click", () => builder.close());
$("phCommit").addEventListener("click", commitBuilder);
builder.addEventListener("close", () => text.focus());
$("phonemesBtn").addEventListener("click", openBuilder);

// ----------------------------------------------------------------- shortcuts

document.addEventListener("keydown", (e) => {
    const mod = e.ctrlKey || e.metaKey;
    if (!mod || builder.open || $("confirm").open) {
        return;
    }
    const k = e.key.toLowerCase();
    if (e.key === "Enter") {
        e.preventDefault();
        speak();
    } else if (e.key === ".") {
        e.preventDefault();
        stop();
    } else if (k === "e" && !e.shiftKey) {
        e.preventDefault();
        wrap("<emphasis>", "</emphasis>");
    } else if (k === "t" && e.shiftKey) {
        e.preventDefault();
        insertPhonemeTemplate();
    } else if (k === "p" && e.shiftKey) {
        e.preventDefault();
        openBuilder();
    } else if (k === "s" && !e.shiftKey) {
        e.preventDefault();
        save();
    }
});

// ----------------------------------------------------------------------- go

// A voiceless build ships none: the voices come from a folder the user picks
// (remembered), else from a Cepstral install or $CEPS_VOICE_ROOT.
async function loadVoices(root) {
    const list = await invoke("voices", { root });
    voiceSel.replaceChildren(...list.names.map((n) => new Option(n, n)));
    const saved = stored("voice", "William");
    voiceSel.value = list.names.includes(saved) ? saved : list.names[0] || "";
    const none = list.names.length === 0;
    if (none) {
        showStatus("No voices found. Press Voices… and choose the folder your Cepstral voices are in "
                   + "(the one holding William, Allison, …), or a single voice's folder.", true);
    } else if (root && list.root === root) {
        showStatus(`Voices from ${root}`);
    }
    voiceSel.disabled = none;
    refreshTransport();
    return list;
}

$("voicesBtn").addEventListener("click", async () => {
    const dir = await dialog.open({ directory: true, title: "Choose your Cepstral voices folder" });
    if (!dir) {
        return;
    }
    const list = await loadVoices(dir);
    if (list.root === dir) {
        store("voiceRoot", dir);
    } else {
        showStatus(`No voices in ${dir}: pick the folder holding the voice folders, or one voice's folder.`, true);
    }
});

(async () => {
    await loadVoices(stored("voiceRoot", null));
    text.focus();
})();
