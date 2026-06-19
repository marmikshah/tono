All critique anchors are confirmed:

- `apply_step` (server.rs:1565) hard-codes a fixed set of replayable tools (17 listed), returns "not a replayable tool" for anything else.
- `is_child_array` (edit.rs:199-200) recognizes only `"inputs" | "stages" | "notes"`, plus special-cased `"tracks"`/`"master"`/`tracks[i].node`. New child-field names (`source`, `exciter`, `modulator`, `carrier`, `a`/`b`) are unaddressable.
- `is_processor` (dsl.rs:884), `validate_node` (dsl.rs:1202), `contains_tracks` (dsl.rs:1134) are the registration sites.
- `compare_sounds` (server.rs:1128) distance uses centroid + level (the `dist` formula at :1144).
- Export quantizer (audio.rs:82, :85) rounds with no dither.

I have everything needed. The critique is accurate on all load-bearing points. Producing the final dossier now.

# Sonarium: Toward Very High-Quality Sound Effects

*Research dossier — lead audio-DSP architect. Status: opinionated, ranked by leverage. All file:line citations verified against the working tree. This revision incorporates an adversarial determinism review: the "engine-version field" was corrected from "already exists" to "net-new plumbing," the analysis→audio boundary is now stated explicitly, edit-stability claims are scoped to the Tracks path, five dropped technique/measurement families are restored, three items are re-tiered, and per-node MCP-surface checklists are attached.*

---

## 1. Where we are — the honest quality ceiling

Sonarium is a well-architected, fully-deterministic synthesis graph with a clean MCP authoring loop. Its *vocabulary* is broad (PolyBLEP classics, 2-op FM, supersaw, Karplus-Strong, a SoundFont sampler, RBJ filters, Schroeder reverb, a 14-voice seq bank). Its *quality* is gated by five structural facts, each traceable to a specific place in the code.

**1a. Nothing is oversampled, so every nonlinearity aliases.** The engine runs sample-for-sample at the document rate — `render_node` (`render.rs:584`) and `apply_processor` (`render.rs:685`) have no upsample/process/downsample wrapper anywhere.

- **Waveshaper drive is the worst offender.** `drive_curve` (`render.rs:959`) applies `tanh`, hard-clip, and wavefold pointwise at base rate. Hard-clip and `Fold` generate effectively infinite-order harmonics that fold back into band. This is the canonical harsh-SFX aliaser.
- **FM is naive.** `fm_signal` (`render.rs:1250`) and the seq FM/piano/epiano voices (`render.rs:1410-1497`) compute `sin(2πφ_c + index·sin(2πφ_m))` directly. FM's sideband series is theoretically infinite; high index or `fm_ratio` — exactly the bright bell/tine settings — sprays partials past Nyquist that fold as inharmonic alias dirt.
- **RingMod** (`render.rs:771`), **Bitcrush** (`render.rs:728`), **Downsample/S&H** (`render.rs:736`) fold with no anti-image filtering; cowbell (`render.rs:1581`) and kit saturation alias the same way.
- The *clean* bright sources (PolyBLEP saw/square/super, highpassed-noise hats) are clean precisely because they have no above-Nyquist content to fold — so **crisp tonal brightness is currently only reachable through the aliasing paths.** That is the core tension.
- **Two aliasers the prior draft missed:** there is **no hard-sync source** at all (a classic aggressive-lead/riser primitive), and the **SoundFont/sampler one-shots alias when pitched up** (no per-octave band-limited mip tables). Both are real quality gaps, not just the FM/drive ones.

**1b. The filter is a Direct-Form-I RBJ biquad, modulated per-sample.** `biquad` (`render.rs:824`) recomputes coefficients every sample from a modulated cutoff `Value`. DF-I assumes LTI coefficients; under fast cutoff/Q modulation the state's *meaning* shifts and you get zipper/thump artifacts. It is 2nd-order only (12 dB/oct, no cascade, no ladder), cannot self-oscillate, and clamps cutoff to `nyq-100` (`render.rs:838`) — so "open and bright" tops out ~100 Hz below Nyquist. There is no transient shaper and no true compressor node.

**1c. The reverb is 1960s Schroeder/Freeverb.** `reverb` (`render.rs:906`) is 6 parallel combs + 4 series allpasses with **fixed** damping 0.2 and allpass g 0.5, exposing only `room` and `mix`. No pre-delay, no early reflections, no per-band decay, no diffusion, no modulation. The master bus fakes stereo width by calling Freeverb twice with a 23-sample offset (`render.rs:200-203`, where `rng.clone()` at :203 hands L/R identical RNG draws — the stereo-determinism mechanism). The metallic comb-flutter tail is inherent to so few combs. No convolution path exists.

**1d. Whole synthesis families are absent.** No modal/impact resonator bank (bells, glass, metal, ceramic, coins, physical-body impacts are faked with aliasing FM). No granular cloud, no sparse stochastic event generator (Seq is a fixed grid, Arp is periodic, noise is dense — *no* crackle/droplets/sparks path). No spectral engine (no inverse FFT in the audio path; `analysis.rs` has a forward STFT only for images/numbers). No random-walk modulator — the four modulators (`dsl.rs:312`) are Slide/Lfo/Arp/EnvMod, all periodic, so wind/fire/engine come out robotically periodic.

**1e. The analyzer can't yet judge most of these defects — and has measurement bugs of its own.** The `Analysis` struct (`analysis.rs:15`) reports one spectral descriptor — centroid (`analysis.rs:341`). There is no spectral flatness, HNR, THD, **no aliasing/inharmonicity detector** (critical given 1a), no sharpness, no per-onset transient analysis, no stereo/mix metrics. Worse, three *existing* measurements are biased: the spectrogram axis is **linear-frequency** (crushes bass detail; should be log/mel), LUFS K-weighting is **48 kHz-only** (wrong at the 44.1 kHz project default), and true-peak uses **4× linear** (not polyphase/sinc) interpolation, under-reading real inter-sample peaks. The LUFS bug is load-bearing because **LUFS feeds the output bytes** (see §1f). The agent's listen-and-fix loop is flying half-blind, and partly on a biased instrument.

**1f. Analysis is NOT purely read-only — it already feeds the rendered bytes.** `normalize_output` (`render.rs:381-397`) calls `analysis::loudness_lufs(samples)` when `target_lufs` is set and scales the buffer by the result, and `true_peak_limit` (`render.rs:417`) calls `analysis::true_peak`. So **analysis sits inside the byte-identical audio path whenever normalization is on.** This is safe today only because LUFS/true-peak don't touch the FFT. It is the single most important boundary for the analyzer roadmap below.

**The honest summary:** sonarium's *architecture* is excellent and its *determinism contract* is genuinely well-engineered. Its *DSP* is a generation behind, and its *analyzer* can't see the gap (and is biased on what it does see). Both are fixable — but the cost of "fixable" was previously underestimated, as §2 corrects.

---

## 2. The determinism contract — what it actually guarantees, and the cost it imposes

The prior draft leaned on a versioning mechanism that **does not exist**. Correcting this is the highest-priority change in the dossier, because it inflated the cheapness of the entire quick-wins tier.

**2a. There is no engine/kernel-version field, and `version` cannot be reused for one.** `SoundDoc.version: Option<u32>` (`dsl.rs:128`) is a **DSL *schema* version**. It is range-checked to `[1, SCHEMA_VERSION=2]` and used to **reject documents from a newer sonarium** (`dsl.rs:961-963`) and to switch per-track RNG stream semantics (`effective_version() >= 2`, `render.rs:107`). Freeverb constants are simply hard-coded — **there is no "Freeverb-tuning-versioning discipline" in the tree.** Therefore:

- Any DSP-kernel change (ADAA, SVF swap, oversampling, dither) that alters byte output **requires a NET-NEW field** — call it `engine: u32` — distinct from `version`. Reusing `version` would conflate schema compatibility with kernel revision and break the existing newer-doc-rejection semantics.
- This is real plumbing: a new field on `SoundDoc`, range/validation logic in `validate` (`dsl.rs:960`), the authoring tools that stamp it, the journal, and `apply_step`. It is **not** a free "already-applied pattern." Every Tier-A effort estimate below now includes its share of this one-time cost.

**2b. The analysis→audio boundary is a hard rule, not a footnote.** Because of §1f, the blanket claim "new analyzer metrics are byte-safe because they're pure functions of the deterministic buffer" is **only true while a metric stays in the reporting path.** The rule the roadmap obeys:

> A new analyzer metric is byte-safe **iff** it is never read by `normalize_output` (`render.rs:381`) or by any gain/limiter decision. The moment an **FFT-derived** metric (flatness, HNR, inharmonicity, sharpness) is wired into `normalize_output` or D3's `master_to_target`, it enters the audio bytes and inherits the **full rustfft nondeterminism hazard** (SIMD dispatch, FP reassociation). D3's "auto-judgeable author→measure→fix" must therefore drive only *editing decisions the agent makes*, never an *in-render* gain computed from an FFT metric — unless the spectral engine's hardening (§B7) has already landed.

**2c. Per-stream RNG isolation exists ONLY inside `render_tracks`.** `render_plain` uses **one shared `Rng::new(doc.seed)`** (`render.rs:303`) threaded sequentially through Mix/Mul/Chain in traversal order (`render.rs:584-665`). The path-seeded isolation primitives (`layer_stream_key`, `track_stream_seed`) live only in `render_tracks` (`render.rs:117,161`). Consequence for every new RNG-drawing source: **in a plain (non-Tracks) SFX document, adding or removing any RNG-drawing node shifts every subsequent sibling's draws — the siblings audibly change.** "Edit-stable" is therefore a *conditional* property, scoped precisely in §3 and §B2.

**2d. Asset bytes are outside the seed.** External `.sf2` content and (if convolution lands) IR/HRIR WAVs are outside `doc.seed`; the journal stores the **path, not a content hash** (`render.rs:240-258`). This is already flagged for SoundFonts and **must extend to convolution IRs/HRIRs** — content-addressing (checksum-id) is required for those, exactly as for SF2.

---

## 3. Where the quality is — what "very high-quality SFX" requires

Distilling the research families, "very high quality" decomposes into five requirements:

1. **No aliasing where you don't want it.** The biggest "cheap/digital" tell is inharmonic foldback from nonlinearities. Cures: **antiderivative anti-aliasing (ADAA)** for memoryless shapers, **polyBLAMP** for triangle corners / deep PWM, **minBLEP/BLEP-at-sync** for a new hard-sync source, **per-octave mip tables** for the sampler, and **2x/4x polyphase-halfband oversampling** for FM/ringmod/fold. Highest quality-per-effort lever in the report; mostly byte-deterministic.

2. **Filters that move without zippering and can scream.** A **ZDF/TPT state-variable filter** (Zavalishin/Simper) eliminates the DF-I modulation artifacts in ~40 lines with identical DSL surface; a **nonlinear Moog/diode ladder** adds 24 dB/oct fat resonance no biquad can produce.

3. **A physical body for impacts and resonant objects.** The recurring #1 recommendation across three research families (modal-impact, physical-modeling, neural-DDSP): a **modal resonator bank** — N parallel damped 2-pole resonators (reusing the biquad recurrence) excited by a **Hertzian impact pulse**. Unlocks bells, glass, metal, wood, ceramic, coins, breaking, and a physical body for every UI/impact sound. *The bank itself* is the deterministically safest transformative technique; its seed-driven shatter cloud and material-table expander are **not** the safe part (see §R1).

4. **Texture and time-domain transformation.** A **`dust` Poisson-impulse source** + a **`rand` random-walk modulator** unlock fire/rain/wind/electricity/crowds. **Granular** clouds + **SOLA/WSOLA** time-stretch/pitch-shift unlock slow-mo, monsters, debris, shimmer. **Spectral freeze + phase-vocoder** unlock drones, risers, morphs (gated on determinism-hardening). **FDN / Dattorro plate / convolution** replace the dated Schroeder tail.

5. **Ears that can grade it, and craft that ships it.** New analyzer metrics (flatness, HNR, inharmonicity/alias index, attack-slope, sharpness, per-onset transients, stereo width) **plus fixes to the existing biased meters** (log spectrogram, 44.1k-correct LUFS, polyphase true-peak). Plus production craft: **layered SFX scaffolding**, **round-robin variation**, **LUFS+true-peak mastering** ship-check, and a **`compare_sounds` distance upgrade** (currently centroid+level only).

**Unifying architectural insight:** prefer the **exciter→resonator split** over monolithic instrument nodes. A shared excitation (impulse, noise burst, pluck, dust train) feeding a resonator *processor* (modal bank, waveguide tube) composes through the existing `chain` plumbing and reuses code across techniques — matching the owner's documented no-presets, compositional-layers stance ([MEMORY.md: no-presets layered authoring]).

---

## 4. Gap analysis — limitation → closing technique

| # | Limitation (file:line) | Closing technique(s) | Determinism verdict |
|---|---|---|---|
| G1 | Drive aliases hard — `drive_curve` tanh/hard/fold at base rate (`render.rs:959`) | 1st-order **ADAA** (state = 1 prev sample) + DC blocker | Compatible **once `engine` field exists**. Hard/fold polynomial (byte-exact); tanh's `ln(cosh)` = same libm risk already shipping in `.tanh()`. |
| G2 | Naive FM folds sidebands — `fm_signal` (`render.rs:1250`), seq FM voices (`:1410-1497`) | **2x/4x polyphase-halfband oversampling** around the FM phase loop | Compatible with care. FIR halfband = fixed coeffs; never mix `mul_add` and `*`+`add`; flush denormals consistently. Pin a sine table for full safety. |
| G3 | RingMod/Bitcrush/Downsample fold (`render.rs:728-782`) | Same oversampling wrapper; Downsample needs anti-image LPF | Compatible (fixed-coefficient FIR). |
| G4 | DF-I biquad zippers under modulation, 12 dB/oct, no self-osc (`render.rs:824`, clamp `:838`) | **ZDF-TPT SVF** kernel swap + new **Moog ladder** node | SVF compatible. Ladder needs fixed-ratio oversampler + pinned float type. Both `engine`-gated. |
| G5 | Schroeder reverb dated/metallic, fixed damping, no ER/pre-delay (`render.rs:906`) | **Dattorro plate** + **8-line FDN**; **convolution** phase 2 | Plate/FDN compatible (pin matrix + interpolation + sr-rounding). Convolution needs scalar FFT **and content-addressed IRs** (§2d). |
| G6 | No modal/impact body — bells/glass/metal faked with aliasing FM | **Modal resonator bank** (parallel biquads) + **Hertzian impact exciter** | Bank+exciter compatible — safest transformative item. Shatter-cloud + material-table caveats in §R1. |
| G7 | Karplus-Strong pluck integer-tuned, no glide, fixed timbre (`render.rs:1428`) | **Extended KS**: fractional-delay tuning allpass + damping filter + pick comb + stiffness allpass | Compatible (strict superset of shipping pluck). |
| G8 | No sparse stochastic events (Seq=grid, Arp=periodic, noise=dense) | **`dust` Poisson-impulse source Node** | Compatible **only with path-seeded substream** (§C3); the weaker "same as `noise_signal`" contract is NOT edit-stable in plain docs. |
| G9 | Only 4 periodic modulators — wind/fire/engine robotic (`dsl.rs:312`) | **`rand` random-walk Modulator** (path-seeded SplitMix64 sub-stream) | Compatible with care — must thread a stable, per-instance seed key into `eval_value` (`render.rs:486`, currently `fn(v,n,sr)`, no Rng). See §S3. |
| G10 | No granular / time-domain transformation | **`granular` source** + **SOLA/WSOLA `timestretch`/`pitchshift`** | Granular compatible (path-seeded, onset-sorted OLA — §C3). SOLA safest **iff** cross-correlation argmax tie-break picks lowest lag (§C4). |
| G11 | No spectral synthesis (freeze/morph/cross-synth/PV) | Internal **STFT engine** → **Freeze**, **Timestretch**, **Crosssynth**, **Spectralmorph** | Needs hardening: `rustfft` `default-features=false`, fixed-poly `sin/cos/atan2`, byte-hash regression test. Gates §2b. |
| G12 | No dynamics: no compressor, no transient shaper | **Single-band compressor** + **dual-envelope transient shaper**; later LR multiband | Compatible (one-pole recursions, reuses duck/compress idiom `render.rs:697`). |
| G13 | No water/wind/crowd/comb/formant vocabulary | **`bubble`** source; wind/fire/rain/electricity as **DSL recipes** (need G8+G9); **comb**/**formant** nodes | Mostly compatible; recipes byte-exact by construction. |
| G14 | Analyzer reports only centroid (`analysis.rs:341`) | **spectral_flatness, HNR/THD, inharmonicity/alias index, attack_slope, sharpness, per-onset transients, stereo width** | Compatible **only in reporting path** (§2b). |
| G15 | Existing meters biased | **log/mel spectrogram axis, 44.1k-correct LUFS K-weighting, polyphase true-peak** | LUFS/true-peak fixes touch **output bytes** (`normalize_output`) → `engine`-gated. Spectrogram axis is image-only (safe). |
| G16 | No production craft scaffolding / variation / ship gate / weak compare | **`scaffold_layered_sfx`**, **`make_round_robin`**, **`master_to_target`** + ship-check, **`compare_sounds` distance upgrade** | Compatible (reuse seeded `vary.rs`, tracks mixer, normalize stage). MCP-surface obligations in §S1. |
| G17 | 8/16-bit export rounds without dither (`audio.rs:82,85`) | **TPDF dither** from seeded SplitMix64 at float→int | Compatible (seeded = byte-exact), `engine`-gated. Honestly a polish item (§Q2). |

**No clean fix under the contract — flag, don't chase:** the **SoundFont sampler** (external `.sf2` bytes + rustysynth floats outside `doc.seed`; journal stores path, not hash — `render.rs:240-258`); **convolution IR/HRIR assets** (same hazard, require content-addressing — §2d); and **render-time neural waveform generation** (RAVE / diffusion Foley / Stable Audio — break byte-identity and the single-binary rule; see §E2).

---

## 5. Prioritized roadmap

Ranked within each tier by **quality-per-effort × inverse-risk**.

### Tier A — Engine-quality foundation
*Lift the ceiling on sounds that already exist; no DSL-surface change. **Every item alters byte output and is gated behind the net-new `engine` field (§2a).***

> **Mandatory Tier-A prerequisite (A0):** add the `engine: u32` document field — distinct from `version` — with validation, tool-stamping, journal, and `apply_step` wiring. This is net-new plumbing, not an existing pattern. It is the prerequisite that lets "improve DSP quality" coexist with "byte-identical forever," and its cost is folded into A1's effort because A1 forces it.

**A1 — 1st-order ADAA on `Node::Drive`, plus A0.** *Impact: transformative. Effort: ~30-50 lines for ADAA + the A0 field plumbing (the larger half). Determinism: compatible once `engine` exists.*
**Map:** wrap `drive_curve` (`render.rs:959`) in the `Drive` arm of `apply_processor` (`render.rs:763`), carrying one `x_prev` state; add a one-pole DC blocker after. Opt-in `aa: bool` (default true for new `engine` revs) so old sounds stay bit-exact and authors can A/B.

**A2 — ZDF-TPT state-variable filter kernel swap.** *Impact: transformative (kills zipper on every modulated sweep). Effort: ~40 lines. Determinism: compatible, `engine`-gated.*
**Map:** in-place rewrite of `biquad`'s body (`render.rs:824`) — keep `FilterKind` and per-sample `cutoff` eval, replace DF-I state `(x1,x2,y1,y2)` with integrator charges `(ic1eq,ic2eq)`, `g=tan(π·fc/fs)`, `k=1/Q`. All seven filter Node types keep identical DSL/MCP surface.

**A4 — TPDF dither on integer export.** *Impact: polish (honest only for 8-bit and quiet reverb tails; 16-bit undithered rounding sits at ~-96 dBFS, inaudible for game SFX). Effort: ~15 lines. Determinism: compatible (seeded), `engine`-gated. Ships independently — its main virtue.*
**Map:** in the WAV/FLAC quantizer (`audio.rs:82,85`), draw two uniforms from a SplitMix64 stream seeded `doc.seed ⊕ "dither"`.

**A5 — polyBLAMP triangle/PWM band-limiting.** *Impact: moderate (removes triangle-corner / deep-PWM aliasing and the leaky-integrator DC droop). Effort: ~15 lines, pure polynomial, byte-exact. Determinism: compatible, `engine`-gated.*
**Map:** add a `poly_blamp` helper beside `poly_blep` (`render.rs:546`); apply at triangle corners and square/PWM edges.

> **Re-tiered out of Tier A (was A3):** **polyphase-halfband oversampling (G2/G3).** A *correct*, byte-identical, FMA-contraction-safe, denormal-safe wrapper plus its application to FM/RingMod/Drive plus per-node fixed-factor logic plus golden-WAV regression across three sites is a **multi-day effort with real traps**, not a quick win. It is the first **Tier A-minus / early Tier B** foundation item (see B0). Move 1 correctly picks ADAA over oversampling; the tiering now matches that judgment.

### Tier B — New synthesis primitives (graph nodes)
*Where sonarium gains new sonic territory. Build on the exciter→resonator architecture. **Every node-introducing item must touch all five registration sites — see §S4 — not just the render arm.***

**B0 — Polyphase-halfband oversampling wrapper (FM + RingMod + Drive).** *Impact: transformative for FM (no other cure exists). Effort: multi-day (wrapper + 3 application sites + golden tests). Determinism: compatible with care, `engine`-gated.*
**Map:** an `oversample(factor, |buf| f(buf))` helper applied inside `fm_signal` (`render.rs:1250`), the seq `fm` voices, `RingMod` (`render.rs:771`), and (with A1) `Drive`. Internal only; mono-f32-block contract unchanged. Fixed per-node factor (or index-driven with a *fixed* threshold). Hardcode the FIR halfband; never mix `mul_add` and `*`+`add`; flush denormals consistently for the IIR path.

**B1 — Modal resonator bank + Hertzian impact exciter.** *Impact: transformative (bells, glass, metal, wood, ceramic, coins, breaking, physical-body UI/impacts). Effort: medium (~150-250 lines, reuses biquad recurrence). Determinism: **bank+exciter are the safest transformative item**; shatter-cloud + material-tables ranked one notch lower — see §R1.*
**Map:** new **processor** Node `modal { modes:[{freq, decay_s|q, gain}], exciter? }` matched in `apply_processor` (catch-all `_ => input.to_vec()` at `render.rs:803` is the seam). Because Chain treats a stage as processor-iff-`is_processor()` (`render.rs:665`), **`modal` must be `is_processor()==true`** so `chain[ impact → modal ]` resonates the running buffer (§S5 — pick the processor form, do not also ship a source form). The impact exciter is a tiny `impact { hardness, velocity }` **source** (raised-cosine pulse, width = hardness). The **material expander** (`material:"glass"`+base_freq → an explicit editable `modes` array in the document — not a frozen preset, respecting the no-presets memory) is author-time **table expansion**: its tables are versioned constants, so changing a table changes bytes (caveat in §R1). Per-mode `set_param` works via existing path-addressing. **MCP sites:** Node variant, `is_processor` (`dsl.rs:884`), `validate_node` (`dsl.rs:1202`), render arm, and `is_child_array` must learn `"exciter"` (`edit.rs:200`) or the exciter is unaddressable.

**B2 — `dust` Poisson source + `rand` random-walk modulator.** *Impact: transformative together (fire/rain/wind/electricity/crowd — 5 of 7 texture families). Effort: dust ~25 lines, rand ~40 lines + the eval_value threading. Determinism: see below.*
**Map:** `dust` is a new source Node. `rand` is a new **Modulator** variant (`dsl.rs:312`) consumed in `eval_value` (`render.rs:486`). **The hard part (§S3):** `eval_value` is `fn(v,n,sr)` — pure, no Rng — and is called per-sample from `biquad` (`render.rs:825`) and from every processor/source reading a `Value`. The path-derived seed key must be **stable under `edit.rs` re-serialization** *and* **distinct per Value-param-instance**, which needs a deterministic path hash, not a one-line "EvalCtx" gloss. **Both `dust` and `rand` must use the path-seeded substream** (`track_stream_seed`/`layer_stream_key`). Otherwise, per §C3/§2c, in a plain (non-Tracks) doc they only get edit-stability when authored as **separate Tracks layers** — that caveat must surface in the tool docs.

**B3 — Extended Karplus-Strong string.** *Impact: high (strings, twangs, snaps, springy boings). Effort: low (~80-150 lines, strict superset of shipping pluck). Determinism: compatible.*
**Map:** enrich seq `pluck` (`render.rs:1428`) or add `Node::String` with `pick_pos`, `damping`, `stiffness` (fractional-delay allpass for exact pitch, one-pole damping, pick comb, 1-4 stiffness allpasses). Note its noise-burst excitation also wants the §C3 substream for edit-stability in plain docs.

**B4 — Dattorro plate reverb, then 8-line FDN.** *Impact: high→transformative (replaces metallic Schroeder; plate gives native stereo). Effort: plate ~1-2 days, FDN ~2-3 days. Determinism: compatible (pin matrix constants, fixed-formula sinusoidal modulation, consistent sr-rounding). Convolution (phase 2) additionally needs scalar FFT + content-addressed IRs (§2d).*
**Map:** new Node `plate{…}` / `fdn{…}` usable inline (like `Reverb`) *and* at the master bus, dropping in at `render.rs:200-203` where Freeverb is currently called twice with a 23-sample offset. Plate's figure-of-eight tank yields the decorrelated L/R pair directly.

**B5 — Granular cloud + SOLA/WSOLA time-stretch/pitch-shift.** *Impact: high (textures, debris, crowds, slow-mo, monster pitch, shimmer). Effort: granular ~250 lines, SOLA ~200 lines. Determinism: granular compatible **with the §C3 path-seeded substream**; SOLA safest **iff** the cross-correlation argmax **tie-break picks the lowest lag** (§C4) — float-equal correlation peaks across platforms are exactly where byte-divergence sneaks in.*
**Map:** `granular { source:Node, grain_ms, density, pitch:Value, jitter }` source (jitter 0 = synchronous, >0 = cloud); SOLA as `timestretch{factor}` / `pitchshift{semitones}` **processor** nodes with "stretch source to fill the node's duration" semantics so doc duration stays authoritative. Granular can optionally source grains from an SF2 region via the existing `load_soundfont` cache. **MCP sites:** `is_child_array` must learn `"source"` (`edit.rs:200`) or the granular child node is unaddressable (§S2).

**B6 — Dynamics: single-band compressor + transient shaper.** *Impact: high (no real compressor exists; drum/SFX punch control absent). Effort: ~60 + ~50 lines. Determinism: compatible (one-pole recursions, reuses duck/compress idiom `render.rs:697`).*
**Map:** new Nodes `compressor{threshold_db,ratio,attack_ms,release_ms,knee_db}` and `transient{attack,sustain}` (dual fast/slow envelope difference, threshold-free).

**B7 — Spectral engine (freeze → PV stretch/shift → cross-synth/morph).** *Impact: transformative (drones, risers, morphs, robot voices). Effort: high. Determinism: the gating risk — see §2b.*
**Map:** build **one** internal `src/spectral.rs` STFT engine (forward exists in `analysis.rs`; add inverse FFT + Hann OLA, identity round-trip as the correctness test). **Before any FFT touches audio output:** `rustfft` `default-features=false` (no SIMD dispatch), replace `sin/cos/atan2` in phase math with fixed in-crate approximations, gate behind a cross-build byte-hash test. **Ship `freeze` first as a near-Tier-B item (§R3):** spectral freeze needs only a single forward FFT + repeated IFFT — no continuous STFT, no PV phase bookkeeping — so it can land right after the scalar-FFT build flag, *separately from* the higher-effort `timestretch`/`pitchshift` (PV, identity phase-locking) and the two-input `crosssynth`/`spectralmorph` combinators (model on the `Duck{trigger,…}` two-input pattern). **MCP sites:** combinators introduce new child-field names (`modulator`/`carrier`, `a`/`b`) → extend `is_child_array` (§S2) and `contains_tracks` (`dsl.rs:1134`).

**B8 — Hard-sync source, `bubble` source, comb/formant nodes, sampler mip tables, supersaw phase upgrade, wind/fire/rain recipes.** *Impact: high for water/voice/leads; recipes need B2. Effort: low each. Determinism: compatible.*
**Map (restored from the research, dropped by the prior draft):**
- **`sync{ master_freq, slave_freq, wave }`** hard-sync source via minBLEP/BLEP-at-sync (classic aggressive lead/riser) — `engine`-gated like the other BLEP paths.
- **Sampler mip tables** — per-octave band-limited tables to stop pitched-up SF2/sampler one-shots aliasing (or explicitly defer with this reason recorded).
- **Supersaw free upgrade** — seeded per-voice random initial phase + stereo detune spread using the existing per-layer SplitMix64; quality win at **zero aliasing cost** (supersaw needs no anti-aliasing).
- **`bubble{radius_mm,rise}`** (Van den Doel: f0≈3.26/R, rising chirp, exp decay).
- **`comb{delay_ms,feedback,mix}`** and **`formant{vowel,morph,q}`** (formant best built on the A2 SVF bandpass).
- **Wind/fire/rain/electricity** ship as documented **DSL recipes + sound-designer skill archetypes** (no-presets stance), not nodes.

### Tier C — Feedback-loop upgrades (so agents can JUDGE quality)
*Cheap, pure functions of the existing buffer/STFT frames. **Reporting-path only — see §2b: none of these may be wired into `normalize_output` or a render-time gain decision without the §B7 FFT hardening.*** These multiply the value of every Tier A/B improvement.

**C1 — `spectral_flatness`** (tonal-vs-noisy, ~20 lines). **C2 — `attack_slope`/`transient_punch`** (~15 lines over the existing envelope in `transients`). **C3-metric — `hnr_db` + `thd_pct`** (medium; report only with a clear fundamental). **C4-metric — `inharmonicity_index` / alias-risk** (medium; the metric that surfaces the engine's aliasing tradeoff, so the agent can act on A1/B0). **C5 — `sharpness_acum`** (Bark-weighted; label "approximate"; defer roughness as research-grade). **C6 — per-onset transient pairs** (replace the single global attack/decay with per-onset attack/decay). **C7 — stereo metrics** (correlation/width/mono-compat) for mixer docs — the output is true stereo but `LayerStats` has zero stereo metrics; load-bearing for BGM/ambience mix quality.
**Map for all:** new fields on `Analysis` (`analysis.rs:15`) — they flow automatically to every analysis-returning tool and to `describe`/`compare_sounds`.

**C8 — `compare_sounds` distance upgrade.** *Promoted from a parenthetical (§M7): arguably higher leverage than any single new descriptor, because the entire convergence loop ranks similarity here.* Today the `dist` formula (`server.rs:1144`) uses only `centroid_delta_hz` + level. Reweight to include decay, true-peak, and the new spectral-shape terms (C1/C4) so "converge toward a reference" actually tracks timbre, not just brightness.

**C9 — Fix the existing biased meters (was §M5/M6).** *Two of three touch the audio bytes → `engine`-gated, not free.*
- **Log/mel spectrogram axis** — image-only, safe to ship anytime; fixes bass detail crush.
- **44.1k-correct LUFS K-weighting** — the meter is 48 kHz-only; at the 44.1 kHz default the loudness-matched **output bytes** (via `normalize_output`, §1f) are computed from a biased filter. `engine`-gated.
- **Polyphase/sinc true-peak** (replace 4× linear) — feeds `true_peak_limit` (`render.rs:417`) → output bytes. `engine`-gated.

### Tier D — Craft & MCP surface
*Zero new DSP; captures production craft as tools/recipes. **Per §R4, promote one of these into the first sprint — the layered-SFX template is rated transformative for *perceived* SFX quality at near-zero risk.***

**D1 — `scaffold_layered_sfx{archetype, base_freq, seed}`** → a 4-track Tracks doc with band-disciplined sub/body/top/transient layers pre-wired, each with archetype ADSR; pairs with the existing per-layer `LayerStats`. (Building atop Tracks also gives the new RNG sources their per-stream isolation for free — §2c.)
**D2 — `make_round_robin{source, count=7, pitch_cents_range, gain_db_range, seed}`** → N variants + a deterministic Fisher-Yates (SplitMix64) no-immediate-repeat order stored as bank metadata; reuses `vary.rs` + `create_bank`/`export_bank`.
**D3 — `master_to_target{lufs, ceiling_dbtp}` + ship-check** — the LUFS+true-peak stage exists (`render.rs:381`); add archetype loudness targets (-23/-24 console, -18 portable) and a gate flagging `true_peak_dbfs > -1 dBTP` and off-target loudness. **Hard constraint (§2b):** the gate may *read* C-tier metrics to advise the agent, but must **not** feed any FFT-derived metric into the in-render gain — and once C9's LUFS fix lands, even the LUFS-driven gain is `engine`-gated because it changes bytes.
**Map + §S1 hard checklist:** new `#[tool]` fns go in the `impl Sonarium` router (`server.rs:575`), routed through the shared build/journal chokepoint. **Every new MUTATING tool (D1/D2/D3, and any author-time expander) MUST be added to `apply_step` (`server.rs:1565`)** — it hard-codes the replayable set and returns `"not a replayable tool"` (`server.rs:1587`) for anything else, which **silently breaks `replay_session` byte-reproducibility** (the session half of the determinism contract). This is a per-tool blocking checklist item, not a footnote.

### Tier E — Research-grade / risky (explicit determinism verdict)

**E1 — DDSP (control-only).** *Verdict: render-time **possible but gated**; design-time **safe and recommended first**.* The net emits only frame-rate *control* (f0, harmonic amplitudes, noise envelope); the audio path is ordinary deterministic DSP. **Recommended:** a design-time MCP helper `suggest_harmonics{reference}` returning curves the agent pastes into existing additive/super/noise nodes — zero render-time risk. A render-time `ddsp` node is viable only via pure-Rust CPU single-thread `tract-onnx` on a tiny model, gated behind a pinned byte-exact test vector. *If you can't reproduce that vector across two machines, it doesn't ship.*

**E2 — RAVE / neural Foley diffusion / Stable Audio.** *Verdict: render-time **incompatible** — do not embed.* The network *generates the waveform*; byte-identity across machines/runtimes is effectively impossible (FP non-associativity, GPU reduction order, framework RNG ≠ SplitMix64), and a multi-GB model violates the single-binary rule. Legitimate use only as an **external, out-of-band asset generator** producing WAV/SF2 that sonarium then plays deterministically (and whose assets need content-addressing, §2d). **Capture the physics, not the network:** the modal/impact node (B1) delivers much of what neural-Foley papers chase, byte-exact, with no weights.

**E3 — FDTD membrane/plate, elasto-plastic friction/bowed, full CataRT, full SMS analysis.** *Verdict: deterministic **only with extreme care**; defer.* Achievable byte-exact *in principle* (pin float op-order, disable FMA contraction, forbid SIMD reassociation, pinned regression vectors) — but with two distinct worst cases that must be stated separately: **FDTD** is the hardest *low-bit-drift* case (compounding grid accumulation), while **friction/bowed** is a *control-flow-divergence* trap — its iterative implicit solver inside a feedback loop has a **variable iteration count**, which is an arguably worse byte-identity hazard than FDTD's drift because divergence is in the branch structure, not the low bits. Pursue only after B1/B3 land and only if drum-plate spatial realism / creaks-squeals / descriptor-driven texture are *stated* requirements. For SMS, ship the deterministic *synthesis subset* (additive partials + SplitMix64-seeded band-shaped noise) instead of analysis. Keep skepticism on **2nd-order ADAA** and **IFFT-additive synthesis**: diminishing perceived returns for SFX — the modal bank (B1) covers the inharmonic-partial use case far more cheaply (summing Sine nodes already works past tens of partials).

---

## 6. Recommended first moves

These four are deliberately one each from the engine foundation (A), new sonic territory (B), judgment (C), and craft (D) tiers — chosen so that after the first sprint sonarium can *make* a clean physical impact, *not alias* its aggressive drive, *prove* both with numbers, and *ship* it as a layered, game-ready asset.

**Move 1 — 1st-order ADAA on `Node::Drive` (A1), which forces the net-new `engine` field (A0).** Highest quality-per-effort change in the study: it fixes the single worst, most-audible aliaser (`drive_curve`, `render.rs:959`), byte-deterministic for the hard-clip and fold shapes that fold worst. **Critically — and unlike the prior draft — the gating mechanism it needs does not exist yet.** Doing ADAA first forces you to build the `engine: u32` field (distinct from the schema `version`, §2a) with its validation/tool-stamping/journal/`apply_step` plumbing. That field is the prerequisite that lets every subsequent Tier-A/B kernel change coexist with the byte-identical contract, so build it under the cheapest, highest-payoff change. Budget Move 1 as "small DSP + real plumbing," not "30 lines."

**Move 2 — Modal resonator bank + impact exciter (B1).** The most *transformative* single addition and the *deterministically safest* transformative technique — a parallel set of the biquad recurrence sonarium already pins. Independently recommended by three research families; replaces the aliasing-FM fakery across bells/glass/metal/wood/ceramic/coin/impact; gives every UI and percussive SFX a physical body; and via the author-time material expander fits the no-presets/layered direction. It establishes the exciter→resonator architecture that B3/B4/B5/B8 reuse. **Ship the bank + impact exciter only in this move** (the safe part); defer the seed-driven shatter cloud until the §C3 path-seeded substream exists (§R1), and treat material tables as versioned constants under the `engine` field. **Do the full §S4 five-site registration** (Node, `is_processor`, `validate_node`, render arm, `is_child_array` += `"exciter"`) and make `modal` a **processor** so `chain[ impact → modal ]` resonates (§S5).

**Move 3 — Analyzer metrics + meter fixes: `spectral_flatness` + `attack_slope` + `inharmonicity_index` (C1/C2/C4), plus the log-frequency spectrogram axis (C9, image-only).** Ship these *with* Move 1 so the agent can *measure* that ADAA reduced foldback (C4 is the alias detector) and that a modal hit is punchy and the right tonal character (C1/C2). ~50 lines, pure functions of the deterministic buffer/STFT frames, **strictly in the reporting path (§2b)** — they must not feed `normalize_output`. The log spectrogram axis is the one meter fix that is image-only and therefore free to ship now (the LUFS/true-peak fixes in C9 touch output bytes and wait for the `engine` field). The whole project's premise is the listen-and-fix loop; right now that loop is blind to the exact failures Moves 1 and 2 fix.

**Move 4 — `scaffold_layered_sfx` (D1).** Promoted into the first sprint (§R4): pure craft, zero DSP risk, rated transformative for *perceived* SFX quality, and it ships today against the existing Tracks/banks surface. It also routes the new RNG sources through Tracks-based per-stream isolation (§2c), so it pays forward into B2/B5. **Blocking checklist:** add it to `apply_step` (§S1) or `replay_session` silently drops it.

The strong fifth move is **B2 (`dust` + `rand`)** — the cheapest path to the entire texture/environmental category — but build it *after* the loop can see quality (Move 3) and *with* the §C3 path-seeded substream and the §S3 stable-path-hash design for `eval_value`, not the weaker "same as `noise_signal`" contract.

---

## 7. Proposal-to-cost summary

| Proposal | Impact | Effort | Determinism-risk | MCP surface |
|---|---|---|---|---|
| A0 `engine` field | enabling | small-medium (net-new plumbing) | n/a (the *enabler*) | `validate`, tool-stamping, journal, `apply_step` |
| A1 ADAA on Drive | transformative | small (+A0) | low, `engine`-gated | none (opt-in `aa` flag) |
| A2 ZDF-TPT SVF | transformative | small (~40 ln) | low, `engine`-gated | none (same `FilterKind`) |
| A4 TPDF dither | polish (8-bit) | trivial | low, `engine`-gated | none |
| A5 polyBLAMP | moderate | trivial | low, `engine`-gated | none |
| B0 oversampling (FM/RM/Drive) | transformative (FM) | multi-day | medium (FMA/denormal traps), `engine`-gated | none (internal) |
| B1 modal + impact exciter | transformative | medium | bank: lowest; shatter/tables: medium (§R1) | Node + is_processor + validate_node + render + `edit.rs += "exciter"` |
| B2 `dust` + `rand` | transformative (texture) | low DSP + nontrivial seed-hash | medium — needs path-seeded substream (§C3) + stable eval_value path hash (§S3) | new Source + new Modulator; eval_value threading |
| B3 extended KS | high | low | low (needs §C3 for plain-doc edit-stability) | enrich pluck / new `String` node (5 sites) |
| B4 plate → FDN (→ convolution) | high→transformative | days | plate/FDN low; convolution needs scalar FFT + content-addressed IRs (§2d) | new Node, master-bus drop-in |
| B5 granular + SOLA | high | granular ~250 / SOLA ~200 | granular: §C3 substream; SOLA: pin tie-break to lowest lag (§C4) | new Source/Processor; `edit.rs += "source"` |
| B6 compressor + transient | high | low-medium | low | new Nodes (5 sites) |
| B7 spectral (freeze→PV→morph) | transformative | high | **gating** — scalar FFT + pinned transcendentals + byte-hash; never into `normalize_output` (§2b) | new Nodes; `edit.rs += "a"/"b"/"carrier"/"modulator"`; `contains_tracks` |
| B8 sync / bubble / comb / formant / mips / supersaw-phase | high (leads, water, voice) | low each | low, BLEP/mip paths `engine`-gated | new Nodes / recipes |
| C1-C7 analyzer descriptors | high (closes the loop) | low-medium | low **iff reporting-path only (§2b)** | new `Analysis` fields (auto-flow) |
| C8 `compare_sounds` distance | high | low | low | reweight `server.rs:1144` |
| C9 meter fixes | high | medium | log-axis: safe; LUFS/true-peak: `engine`-gated (touch bytes) | none |
| D1 scaffold_layered_sfx | transformative (perceived) | low | low | new tool **+ apply_step (§S1)** |
| D2 make_round_robin | high | low | low (seeded) | new tool **+ apply_step (§S1)** |
| D3 master_to_target + ship-check | high | low | low **iff no FFT metric feeds in-render gain (§2b)** | new tool **+ apply_step (§S1)** |
| E1 DDSP control-only | high | high | design-time safe; render-time gated on pinned vector | `suggest_harmonics` design-time tool |
| E2 neural waveform gen | n/a | n/a | **incompatible at render time** — external asset only | none |
| E3 FDTD / friction / CataRT / SMS | research | very high | extreme care; friction = control-flow divergence (worse than FDTD drift) | deferred |

---

*Determinism is the load-bearing constraint throughout — and it is more expensive than the first draft assumed. There is no engine-version field today: one must be built (A0) before any kernel change ships. Analysis is not read-only: LUFS and true-peak already feed `normalize_output`, so new FFT metrics must stay in the reporting path until the spectral engine is hardened. Per-stream RNG isolation exists only inside `render_tracks`, so every new RNG-drawing source needs an explicit path-seeded substream to be "edit-stable" in a plain SFX doc. Every new node touches five registration sites and every new mutating tool must enter `apply_step`, or the surgical-edit and replay halves of the contract silently break. Honor those four facts — net-new `engine` gating, the analysis→audio boundary, path-seeded substreams, and full MCP registration — and the byte-identical contract survives the entire roadmap.*
