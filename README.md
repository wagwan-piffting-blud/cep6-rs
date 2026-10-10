# cep6-rs

A small, fast, dependency-free Rust engine for **Cepstral Swift** voice banks
(William, Allison, David, Jean-Pierre and friends). It reads a Cepstral voice
folder directly and turns text into speech without the Cepstral runtime,
SAPI, or Windows: the same voice runs on Windows, macOS, Linux and a
Raspberry Pi.

Every stage is a port of Cepstral's own code, read out of the original DLLs:
the tokenizer and number/date/address rules, the lexicon and letter-to-sound
rules, the syllabifier, the part-of-speech tagger, unit selection and the LPC
synthesiser. For Swift 6.2 voices the output is not "close to" the original
engine, it is the same: the test suite compares rendered audio with
Cepstral's own, sample for sample.

> **No voices are included.** You need your own copy of a Cepstral voice.

- [What you need](#what-you-need)
- [Quick start](#quick-start)
- [Command-line usage](#command-line-usage)
- [Markup: SSML and SAPI XML](#markup-ssml-and-sapi-xml)
- [Common recipes](#common-recipes)
- [The GUI](#the-gui)
- [Troubleshooting](#troubleshooting)
- [How accurate is it?](#how-accurate-is-it)
- [Building from source](#building-from-source)
- [Using it as a library](#using-it-as-a-library)
- [Diagnostic commands](#diagnostic-commands)
- [License](#license)

---

## What you need

1. **The `cep6` binary** for your platform, or the GUI (see [The GUI](#the-gui)).
   Prebuilt binaries are named for what they run on:

   | File | Runs on |
   |---|---|
   | `cep6-windows-amd64.exe` | 64-bit Windows 10/11 |
   | `cep6-windows-x86.exe` | 32-bit Windows |
   | `cep6-macos-universal` | macOS 11+, Apple silicon and Intel (signed and notarized) |
   | `cep6-linux-amd64` | 64-bit Linux PC, server, Docker, WSL |
   | `cep6-linux-arm64` | 64-bit Raspberry Pi OS: Pi 3/4/5, Zero 2 W |
   | `cep6-linux-armv7` | 32-bit Raspberry Pi OS on Pi 2/3/4 |
   | `cep6-linux-armv6` | Pi 1 and Pi Zero / Zero W |

   Each is a single file. The Linux builds are fully static and need nothing
   installed; the lexicon and tagger are built in.

2. **A Cepstral voice folder.** This is the folder named after the voice that
   holds `voice.idx` and the `voice_*.dat` files:

   ```text
   David\
       voice.idx
       voice_a.dat  voice_c.dat  voice_d.dat  voice_f.dat  voice_u.dat
       settings.txt lexicon.txt  (license.txt)
   ```

   If Cepstral Swift is installed, the voices are already on disk:

   | Install | Voice folder |
   |---|---|
   | Windows | `C:\Program Files\Cepstral\voices\David` |
   | Windows, an older 32-bit Swift on 64-bit Windows | `C:\Program Files (x86)\Cepstral\voices\David` |
   | Linux | usually `/opt/swift/voices/David` |

   32-bit Windows has no `Program Files (x86)` folder at all, so a path that
   names it will not work there.

   You can also copy a voice folder anywhere; nothing else from the install is
   used. Pass the **voice's own folder**, not the `voices` folder above it.

   If Swift is installed, the voice's **name** is enough: `cep6 David text …`
   uses `David` from the current folder if there is one, and otherwise
   `$CEPS_VOICE_ROOT\David` or the installed `Cepstral\voices\David`. Both
   `Program Files` folders are checked, so this works the same on 32-bit and
   64-bit Windows and with either build of `cep6`.

Swift 6.2 voices are the main target. Swift 4 and 5 voices also load and
speak; their output is very close to the original engine but not yet
sample-identical.

---

## Quick start

Windows (Command Prompt or PowerShell), with Cepstral Swift installed:

```bat
cep6.exe David text "Hello, this is David." hello.wav
```

or with the voice folder given in full:

```bat
cep6.exe "C:\Program Files\Cepstral\voices\David" text "Hello, this is David." hello.wav
```

macOS / Linux:

```sh
./cep6 /opt/swift/voices/David text "Hello, this is David." hello.wav
```

That writes `hello.wav`: 16-bit mono PCM at the voice's own sample rate
(16 kHz for William and Jean-Pierre, 22.05 kHz for Allison and David).

For anything longer than a sentence or two, put the text in a file:

```sh
./cep6 /opt/swift/voices/David file chapter1.txt chapter1.wav
```

---

## Command-line usage

```text
cep6 <voice-folder> <command> [arguments] [options]
```

The two commands you will use:

| Command | What it does |
|---|---|
| `text "<text>" <out.wav>` | Speak the text given on the command line. `speak` and `say` are the same command. |
| `file <in.txt> <out.wav>` | Speak a UTF-8 text file. Use this for anything long: shells cap argument length (about 32 KB on Windows), and `file` writes audio to disk one sentence at a time, so input length is unlimited and memory use stays flat. |

Options (after the command's arguments, in any order):

| Option | Meaning |
|---|---|
| `--markup` | Read the input as SSML or SAPI XML, so tags act instead of being spoken. See [Markup](#markup-ssml-and-sapi-xml). |
| `--rate N` | Speaking rate as in SAPI's `SpVoice.Rate`, -10 (slowest) to 10 (fastest); 0 is normal. |
| `--wpm N` | Speaking rate in words per minute, set directly (170 is normal). |
| `--pitch-shift F` | Pitch multiplier, e.g. `1.1` for 10% higher, `0.9` for 10% lower. |
| `--volume N` | Output volume in percent, 0 to 100, as in SAPI's `SpVoice.Volume`. |
| `--digits` | Read bare numbers digit by digit: 911 as "nine one one" rather than "nine hundred eleven". Right for codes, phone numbers and alert text; the default suits prose. |
| `--no-pos` | Skip the part-of-speech tagger (homographs such as "read" or "record" then take their default pronunciation). |

`--rate`, `--volume` and `--pitch-shift` behave exactly like the
corresponding SAPI settings on the original engine; the rate scale, including
its uneven steps, is the engine's own.

Run `cep6 --help` for the full list, including the diagnostic commands.

### Quoting and paths

- Quote any path with spaces: `"C:\Program Files\Cepstral\voices\David"`.
- **On Windows, never end a quoted path with a backslash.** `"...\David\"`
  makes the shell treat `\"` as a literal quote, so the path swallows the
  following arguments. Write `"...\David"`.
- Relative paths are taken from the folder you run `cep6` in, not from where
  `cep6` itself is. When in doubt, use full paths.
- The output folder must already exist; `cep6` creates the `.wav`, not the
  folders above it.

---

## Markup: SSML and SAPI XML

With `--markup`, the input may carry Swift SSML or SAPI XML. Both are read
the way Cepstral's SAPI driver reads them, with the same results. Text
outside tags is spoken normally, and you do not need a `<speak>` wrapper.

| Control | SSML | SAPI XML |
|---|---|---|
| Pause | `<break time="500ms"/>`, `<break time="1.5s"/>` | `<silence msec="500"/>` |
| Sentence / paragraph break | `<break/>`, `<break strength="strong"/>` | |
| Rate | `<prosody rate="slow">`, `"x-fast"`, `"+20%"`, `"0.8"` | `<rate absspeed="-3">`, `<rate speed="2">` |
| Pitch | `<prosody pitch="high">`, `"+4st"`, `"-10%"` | `<pitch absmiddle="5">`, `<pitch middle="-2">` |
| Volume | `<prosody volume="loud">`, `"soft"`, `"+20%"`, `"50"` | `<volume level="50">` |
| Emphasis | `<emphasis>`, `<emphasis level="strong">` | `<emph>` |
| Spell out | `<say-as type="spell-out">` | `<spell>` |
| Read as… | `<say-as interpret-as="…">`: `ordinal`, `cardinal`, `digits`, `date` (`format="mdy"` / `"dmy"` / `"ymd"`), `time`, `address`, `measure`, `literal`, `words` | |
| Pronunciation | `<phoneme ph="t ah0 m ey1 t ow0">tomato</phoneme>` | `<pron sym="t ah m ey 1 t ow"/>` |
| Bookmark | `<mark name="m1"/>` | `<bookmark mark="m1"/>` |

Notes on the less obvious ones:

- **Phonemes** use Cepstral's phone set, which is CMU-like but not identical:
  `i` (not `iy`) as in "see", `j` (not `y`) as in "yes", `h` (not `hh`).
  Vowels take a stress digit: `0` unstressed, `1` primary, `2` secondary.
  `.` marks a syllable break and `|` a word break. `alphabet="sampa"` accepts
  SAMPA instead. If any phone in `ph` is unknown, the engine silently drops
  the whole tag (the word disappears from the audio), so check spelling and
  case. A self-closing `<phoneme ph="…"/>` does nothing; always close it with
  `</phoneme>`.
- **Emphasis** slows the words by a quarter and raises their volume by a
  quarter (half, for `strong`). SAPI's `<emph>` is weaker on the original
  engine, and here too: it never changes volume.
- **SAPI `<spell>`** is read at the default rate, pitch and volume, even
  inside other SAPI tags. That is the original engine's behaviour.
- `<sub>` and `<audio>` are not supported; their text is kept and spoken, and
  `cep6` says so.

---

## Common recipes

Read a long document to a file:

```sh
./cep6 voices/David file book.txt book.wav
```

Slower and a little lower:

```sh
./cep6 voices/William text "Please stand by." out.wav --rate -3 --pitch-shift 0.95
```

A pause and an emphasised word:

```sh
./cep6 voices/Allison text "Wait<break time=\"700ms\"/> that is <emphasis>mine</emphasis>." out.wav --markup
```

Dates, times, phone numbers and addresses read the way you mean them:

```sh
./cep6 voices/David text "Meet at <say-as interpret-as=\"address\">1234 N Main St</say-as> on <say-as interpret-as=\"date\" format=\"mdy\">12/25/2026</say-as> at <say-as type=\"time\">7:30</say-as>." out.wav --markup
```

A name the dictionary gets wrong:

```sh
./cep6 voices/William text "Hello, <phoneme ph=\"s ah0 b r ih1 n ah0\">Sabrina</phoneme>." out.wav --markup
```

Spell something out:

```sh
./cep6 voices/David text "Your code is <say-as type=\"spell-out\">XK4</say-as>." out.wav --markup
```

On Windows, markup is easier to keep in a file (`file in.txt out.wav --markup`)
than to escape on the command line.

---

## The GUI

`gui/` is a desktop app (Tauri, so Rust and cross-platform) with the same
engine inside:

- a text box, Speak / Pause / Stop, and Save to File;
- a voice picker and sliders for rate, pitch and volume;
- a **Read markup** switch and a Markup toolbar that wraps the selection in
  the tags above (emphasis, pauses, rate/pitch/volume presets, every say-as);
- a **Phoneme Builder**: click phones to build a pronunciation, see a
  readable respelling, preview it, and insert it. It flags any `<phoneme>`
  the engine would silently drop before you speak or save.

Installers come in two flavours:

| Flavour | Size | Voices |
|---|---|---|
| full (`Ceps_*`) | ~310-430 MB | William, Allison, David and Jean-Pierre built in |
| voiceless (`Ceps-voiceless_*`) | ~8-11 MB (~85 MB AppImage) | none; uses yours |

The voiceless app looks for voices in a folder you pick with **Voices…**
(remembered; it may hold several voice folders or be one voice's folder),
then in `$CEPS_VOICE_ROOT`, then in a Cepstral Swift install
(`%ProgramFiles%\Cepstral\voices`, `/opt/swift/voices`).

| Platform | Installers |
|---|---|
| Windows 10/11 x64 | NSIS setup `.exe`, `.msi` |
| macOS 11+ (universal) | `.dmg`, signed and notarized |
| Linux x64, glibc 2.35+ (Ubuntu 22.04+, Debian 12+) | `.deb`, `.AppImage` |
| Linux arm64 (Raspberry Pi 4/5, 64-bit OS) | `.deb`, `.AppImage` |

---

## Troubleshooting

**`cannot open voice …: The system cannot find the path specified. (os error 3)`**
A folder in the path does not exist. `cep6` names the part that is missing
(for example `C:\ has no Program Files (x86)`) and, for a bare voice name,
the install folders it also looked in. Common causes: a typo in the voice
name; a voice that is not installed (give its folder in full, or set
`CEPS_VOICE_ROOT` to the folder holding it); a `Program Files (x86)` path on
32-bit Windows, which only has `C:\Program Files`; or a relative path run
from a different folder.

**`Access is denied` when writing the `.wav`**
Running from a folder you cannot write to (such as `C:\Program Files\...`
without admin rights) with a bare output name like `out.wav` sends the file
to your Desktop instead, and `cep6` says so. If you named a folder in the
output path, it does not fall back: pick a folder you can write to.

**`… (os error 2)` with "that folder holds no voice.idx"**
You pointed at the `voices` folder; point at the voice inside it, e.g.
`...\voices\David`.

**`… (os error 123)` and a path with a `"` in it**
A quoted Windows path ended in a backslash (`"...\David\"`). Drop the
trailing backslash.

**`cannot write out.wav: … (os error 3)`**
The folder you are writing into does not exist. Create it first.

**A word is missing from the audio**
A `<phoneme>` tag had a phone the engine does not know (or used the wrong
case), so the whole tag was dropped. The GUI's Phoneme Builder catches this;
on the command line, compare against the phone list in
[Markup](#markup-ssml-and-sapi-xml).

**`no Cepstral lexicon: this binary was built without embed-packs`**
Only a custom build without the default feature says this. Put
`ceplex.bin` (from `packs/`) beside `cep6`, or set `CEPS_CEPLEX` to its path.

---

## How accurate is it?

For Swift 6.2 voices, `cep6` is checked against Cepstral's own engine at
every stage, and the final audio is sample-identical: a 47-second,
seven-sentence passage full of numbers, times, dates, abbreviations and
punctuation comes out with all 761,806 samples equal. Markup is held to the
same standard; `tests/markup_engine.tsv` holds 148 engine renders covering
every tag above, and SAPI rate and volume are checked at every level.

Things that are deliberately not reproduced:

- Swift 4 and 5 voices use an older cost model; they sound right and are
  within about 1% on timing, but are not yet sample-identical.
- `<prosody pitch>` in Hz, `range` and `contour` need a synthesis mode these
  voices do not have in the original engine either; they are reported and
  ignored.

---

## Building from source

The library and CLI need only a Rust toolchain; there are no dependencies.

```sh
cargo build --release              # target/release/cep6
cargo test --release               # needs CEPS_VOICE_ROOT for the voice tests
```

Tests that render audio need the voices: set `CEPS_VOICE_ROOT` to a folder
holding `William`, `Allison`, `David` and `Jean-Pierre`. They skip silently
otherwise.

Release builds:

| Script | Builds |
|---|---|
| `build_targets.ps1` (Windows, PowerShell) | the CLI for all seven targets into `dist/`: Windows x64/x86 and the four Linux targets from this machine (static CRT, musl linked with Rust's own `rust-lld`, no cross toolchain needed), macOS by running `build_macos.sh` on the Mac over SSH. `-NoMac`, `-Only <target>`. `-Check` renders all four voices (from the repo's parent folder), markup and prosody through every binary (Linux under WSL and `qemu-user`, both macOS slices on the Mac) and demands each output's SHA256 equal `windows-amd64`'s; `-NoBuild -Check` checks what is already in `dist/` |
| `build_macos.sh` (on a Mac) | the universal macOS CLI, signed and notarized; signing identity and notary key are stored at its top |
| `gui/build_targets.ps1` | every GUI installer: Windows locally, Linux in Docker (x64 locally, arm64 on a Pi), macOS over SSH. `-Flavor full\|voiceless\|both`, `-Only <target>` |
| `gui/build_macos.sh` (on a Mac) | the GUI `.app` and `.dmg`, signed and notarized; `VOICELESS=1` for the voiceless one |

For a GUI development build: `cd gui && cargo build`, then run
`gui/target/debug/ceps-gui`. Debug builds find the voices in the repo's
parent folder; set `CEPS_VOICE_ROOT` otherwise.

---

## Using it as a library

The `ceps` crate does everything `cep6` does. `ceps::render` is the whole
text-to-audio path in one call:

```rust
use ceps::render::{render, Options};
use ceps::{CepLex, PosTag, Voice};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let voice = Voice::open("voices/David")?;
    let lexicon = CepLex::open("packs/ceplex.bin")?;
    let tagger = PosTag::open("packs/pos.bin").ok();

    let opts = Options { markup: true, rate: -2, ..Options::default() };
    let (pcm, report) = render(&voice, &lexicon, tagger.as_ref(),
                               "Hello <emphasis>there</emphasis>.", &opts)?;
    if !report.missing.is_empty() {
        eprintln!("could not pronounce: {:?}", report.missing);
    }
    ceps::wav::write("hello.wav", &pcm, voice.sps)?;
    Ok(())
}
```

`ceps::render::render_each` hands you the audio one sentence at a time
instead, for streaming or for very long input.

---

## Diagnostic commands

`cep6` also carries the tools used to build and verify the port. They are not
needed for normal use:

| Command | What it does |
|---|---|
| `info` | Voice parameters: sample rate, unit and type counts, model settings. |
| `types [substring]` | List unit types (diphone-like units), optionally filtered. |
| `unit <index> <out.wav>` | Render one unit from the database. |
| `frames <first> <count> <out.wav>` | Render raw frames. |
| `chain <first_unit> <out.wav>` | Render the units recorded after one another from a starting unit, as they were spoken in the source recording. |
| `say-types "<type> <type> …" <out.wav>` | Render a given unit-type sequence. |
| `pipeline "pau h eh:1 l ow:1 pau" <out.wav>` | Phones in, audio out, skipping the text front end. |
| `speak-score <score.txt> <out.wav>` | Render from a score with explicit F0 and durations (`examples/hello.score`). |
| `pace "<types>" <secs/unit> <f0> <out.wav>` | Render a type sequence at a fixed pace and pitch. |
| `names`, `target-cost`, `dur`, `f0`, `self-test` | Dump the voice's models and check them. |
| `bench [frames]` | Synthesis speed. |

---

## License

GPL-3.0; see [LICENSE](LICENSE). Cepstral voices are not covered by this license and are not included; use only voices you are licensed to use.

## GenAI disclosure: Portions of this repository have been generated using generative AI tools (Claude, Claude Code).
