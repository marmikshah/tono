//! tono-desktop — the pattern station.
//!
//! The walking skeleton of the DAW: an FL-style step grid over catalog
//! instruments, looping live through the native audio deck. Click a cell and
//! hear it on the next block; every edit is undoable; the project saves as an
//! ordinary tono [`Song`](tono_core::song::Song) wrapped with its grid rows.
//!
//! Not part of the default build or CI; built only via `make desktop`. Two
//! entry points on one engine:
//! - `tono-desktop` (no args) launches the station window.
//! - `tono-desktop play FILE.json [SECS]` is a headless native preview (loops).
//!
//! State lives in Rust ([`studio::Station`]); the webview is a pure view that
//! sends commands and re-renders from the returned [`StudioState`].

mod audio;
mod studio;

use std::sync::Mutex;

use audio::AudioHandle;
use base64::Engine as _;
use serde::Serialize;
use studio::Station;
use tauri::State;
use tono_core::{analysis, render};

/// App state: the station (project + undo) and the lazily-created audio deck
/// (built on first play, since it needs an output device).
struct App {
    station: Mutex<Station>,
    audio: Mutex<Option<AudioHandle>>,
}

impl Default for App {
    fn default() -> Self {
        App {
            station: Mutex::new(Station::new()),
            audio: Mutex::new(None),
        }
    }
}

/// One grid lane as the UI renders it.
#[derive(Serialize)]
struct RowState {
    label: String,
    track: String,
    pitch: String,
    steps: Vec<bool>,
}

/// One mixer channel as the UI renders it.
#[derive(Serialize)]
struct TrackState {
    name: String,
    gain: f32,
    pan: f32,
    muted: bool,
}

/// The whole view model — every command returns a fresh one, so the frontend
/// is a pure render of this.
#[derive(Serialize)]
struct StudioState {
    bpm: f32,
    swing: f32,
    steps: u32,
    steps_per_beat: u32,
    rows: Vec<RowState>,
    tracks: Vec<TrackState>,
    can_undo: bool,
    can_redo: bool,
    /// A compile/audio problem worth surfacing, if any.
    error: Option<String>,
}

/// Rebuild the view model and push the recompiled loop to the audio deck.
fn refresh(app: &App) -> StudioState {
    let station = app.station.lock().unwrap_or_else(|p| p.into_inner());
    let project = &station.project;
    let mut error = None;

    match project.loop_doc() {
        Ok(doc) => {
            if let Ok(slot) = app.audio.lock()
                && let Some(deck) = slot.as_ref()
            {
                deck.set_doc(doc);
            }
        }
        Err(e) => error = Some(e),
    }

    let steps = project.steps();
    let (undo_depth, redo_depth) = station.depths();
    StudioState {
        bpm: project.song.bpm,
        swing: project.song.swing,
        steps,
        steps_per_beat: project.song.steps_per_beat,
        rows: project
            .rows
            .iter()
            .map(|r| RowState {
                label: r.label.clone(),
                track: r.track.clone(),
                pitch: r.pitch.clone(),
                steps: (0..steps).map(|s| project.cell(r, s)).collect(),
            })
            .collect(),
        tracks: project
            .song
            .tracks
            .iter()
            .map(|t| TrackState {
                name: t.name.clone(),
                gain: t.gain,
                pan: t.pan,
                muted: project.muted.contains(&t.name),
            })
            .collect(),
        can_undo: undo_depth > 0,
        can_redo: redo_depth > 0,
        error,
    }
}

/// Run an undoable edit, then rebuild the view.
fn edit(app: &App, change: impl FnOnce(&mut studio::Project)) -> StudioState {
    {
        let mut station = app.station.lock().unwrap_or_else(|p| p.into_inner());
        station.edit(change);
    }
    refresh(app)
}

/// The full current view model (the frontend's initial render).
#[tauri::command]
fn state(app: State<App>) -> StudioState {
    refresh(&app)
}

/// Flip a grid cell.
#[tauri::command]
fn toggle_step(row: usize, step: u32, app: State<App>) -> StudioState {
    edit(&app, |p| p.toggle(row, step))
}

/// Re-pitch a melodic lane (its notes move with it).
#[tauri::command]
fn set_row_pitch(row: usize, pitch: String, app: State<App>) -> StudioState {
    edit(&app, |p| {
        p.set_row_pitch(row, pitch.trim());
    })
}

/// Set the tempo.
#[tauri::command]
fn set_bpm(bpm: f32, app: State<App>) -> StudioState {
    edit(&app, |p| p.song.bpm = bpm.clamp(30.0, 300.0))
}

/// Set the song-wide swing (0 = straight).
#[tauri::command]
fn set_swing(swing: f32, app: State<App>) -> StudioState {
    edit(&app, |p| p.song.swing = swing.clamp(0.0, 1.0))
}

/// Mixer move: a track's fader, pan, and mute in one call.
#[tauri::command]
fn set_track(name: String, gain: f32, pan: f32, muted: bool, app: State<App>) -> StudioState {
    edit(&app, |p| {
        if let Some(t) = p.song.tracks.iter_mut().find(|t| t.name == name) {
            t.gain = gain.clamp(0.0, 2.0);
            t.pan = pan.clamp(-1.0, 1.0);
        }
        match muted {
            true => p.muted.insert(name.clone()),
            false => p.muted.remove(&name),
        };
    })
}

/// Step back / forward through the edit history.
#[tauri::command]
fn undo(app: State<App>) -> StudioState {
    {
        let mut station = app.station.lock().unwrap_or_else(|p| p.into_inner());
        station.undo();
    }
    refresh(&app)
}

/// See [`undo`].
#[tauri::command]
fn redo(app: State<App>) -> StudioState {
    {
        let mut station = app.station.lock().unwrap_or_else(|p| p.into_inner());
        station.redo();
    }
    refresh(&app)
}

/// Save the project (Song + grid rows) as JSON.
#[tauri::command]
fn save_project(path: String, app: State<App>) -> Result<(), String> {
    let station = app.station.lock().unwrap_or_else(|p| p.into_inner());
    station.save(&path)
}

/// Load a project, replacing the current one (undoable).
#[tauri::command]
fn load_project(path: String, app: State<App>) -> Result<StudioState, String> {
    {
        let mut station = app.station.lock().unwrap_or_else(|p| p.into_inner());
        station.load(&path)?;
    }
    Ok(refresh(&app))
}

/// Transport: `"play"` (spins the audio deck up on first use), `"pause"`,
/// `"stop"`. Returns an error string when no output device is available.
#[tauri::command]
fn transport(action: String, app: State<App>) -> Result<(), String> {
    let mut slot = app.audio.lock().unwrap_or_else(|p| p.into_inner());
    if slot.is_none() && action == "play" {
        match audio::spawn() {
            Ok(deck) => *slot = Some(deck),
            Err(e) => return Err(format!("audio unavailable: {e}")),
        }
        drop(slot);
        refresh(&app); // push the current loop to the fresh deck
        slot = app.audio.lock().unwrap_or_else(|p| p.into_inner());
    }
    if let Some(deck) = slot.as_ref() {
        deck.transport(&action);
    }
    Ok(())
}

/// The playhead for the grid highlight: `(playing, current step)`.
#[derive(Serialize)]
struct Playhead {
    playing: bool,
    step: u32,
}

/// Where the loop currently is (polled by the UI at ~20 Hz).
#[tauri::command]
fn playhead(app: State<App>) -> Playhead {
    let steps = {
        let station = app.station.lock().unwrap_or_else(|p| p.into_inner());
        station.project.steps().max(1)
    };
    let slot = app.audio.lock().unwrap_or_else(|p| p.into_inner());
    let (playing, pos, len) = slot.as_ref().map(|d| d.playhead()).unwrap_or((false, 0, 0));
    let step = match len {
        0 => 0,
        _ => ((pos as f64 / len as f64) * steps as f64) as u32 % steps,
    };
    Playhead { playing, step }
}

/// The analysis strip: level numbers plus the two feedback images for the
/// current loop (rendered offline — the same bytes the deck plays).
#[derive(Serialize)]
struct AnalysisResult {
    ok: bool,
    error: Option<String>,
    lufs: f32,
    true_peak_dbtp: f32,
    peak_dbfs: f32,
    duration: f32,
    spectrogram_png: String,
    waveform_png: String,
}

/// Analyze the current pattern (the UI debounces this behind edits).
#[tauri::command]
fn analyze(app: State<App>) -> AnalysisResult {
    let empty = |error: Option<String>| AnalysisResult {
        ok: false,
        error,
        lufs: -120.0,
        true_peak_dbtp: -120.0,
        peak_dbfs: -120.0,
        duration: 0.0,
        spectrogram_png: String::new(),
        waveform_png: String::new(),
    };
    let doc = {
        let station = app.station.lock().unwrap_or_else(|p| p.into_inner());
        match station.project.loop_doc() {
            Ok(Some(doc)) => doc,
            Ok(None) => return empty(None),
            Err(e) => return empty(Some(e)),
        }
    };
    let product = render::render_product(&doc);
    // One STFT feeds both the numeric stats and the spectrogram image.
    let frames = analysis::spectral_frames(&product.mono);
    let stats = match &product.stereo {
        Some((l, r)) => analysis::stats_stereo_with(l, r, doc.sample_rate, &frames),
        None => analysis::stats_with(&product.mono, doc.sample_rate, &frames),
    };
    let b64 = |bytes: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(bytes);
    AnalysisResult {
        ok: true,
        error: None,
        lufs: stats.loudness_lufs,
        true_peak_dbtp: stats.true_peak_dbfs,
        peak_dbfs: stats.peak_dbfs,
        duration: stats.duration_secs,
        spectrogram_png: b64(analysis::spectrogram_png_with(&frames).unwrap_or_default()),
        waveform_png: b64(analysis::waveform_png(&product.mono).unwrap_or_default()),
    }
}

const HELP: &str = "tono-desktop — the tono pattern station.

USAGE:
    tono-desktop                         launch the station window (real-time audio)
    tono-desktop play FILE.json [SECS]   headless: loop a graph through the default device";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("play") => {
            if let Err(e) = play_cli(&args[2..]) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Some("--help" | "-h") => println!("{HELP}"),
        _ => run_studio(),
    }
}

/// Launch the Tauri window with the pattern-station frontend.
fn run_studio() {
    tauri::Builder::default()
        .manage(App::default())
        .invoke_handler(tauri::generate_handler![
            state,
            toggle_step,
            set_row_pitch,
            set_bpm,
            set_swing,
            set_track,
            undo,
            redo,
            save_project,
            load_project,
            transport,
            playhead,
            analyze
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch the tono studio window");
}

/// `tono-desktop play FILE [SECS]` — loop a graph natively via cpal.
fn play_cli(args: &[String]) -> anyhow::Result<()> {
    let path = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("usage: tono-desktop play FILE.json [SECS]"))?;
    let doc: tono_core::dsl::SoundDoc = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    doc.validate().map_err(|e| anyhow::anyhow!(e))?;
    let secs = args
        .get(1)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or_else(|| doc.duration.max(0.5));
    let deck = audio::spawn()?;
    deck.set_doc(Some(doc));
    deck.transport("play");
    std::thread::sleep(std::time::Duration::from_secs_f32(secs));
    Ok(())
}
