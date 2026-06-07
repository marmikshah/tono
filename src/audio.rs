//! Audio file output: WAV (PCM, with optional `smpl` loop chunk), FLAC
//! (lossless, pure-Rust `flacenc`), and OGG Vorbis (lossy, `vorbis_rs`) — the
//! compressed formats games actually ship for BGM / ambience.

use std::path::Path;

/// Interleave per-channel `f32` buffers (in [-1, 1]) into `i32` PCM at `bits`.
fn interleave_i32(channels: &[&[f32]], bits: u16) -> Vec<i32> {
    let scale = ((1i64 << (bits - 1)) - 1) as f32;
    let n = channels.iter().map(|c| c.len()).min().unwrap_or(0);
    let mut out = Vec::with_capacity(n * channels.len());
    for i in 0..n {
        for ch in channels {
            out.push((ch[i].clamp(-1.0, 1.0) * scale).round() as i32);
        }
    }
    out
}

/// Write channels as a FLAC file (lossless). `bits` is 8 or 16.
pub fn write_flac(
    path: &Path,
    channels: &[&[f32]],
    sample_rate: u32,
    bits: u16,
) -> anyhow::Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let bits = if bits == 8 { 8u16 } else { 16u16 };
    let interleaved = interleave_i32(channels, bits);
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| anyhow::anyhow!("flac config: {e}"))?;
    let source = flacenc::source::MemSource::from_samples(
        &interleaved,
        channels.len(),
        bits as usize,
        sample_rate as usize,
    );
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| anyhow::anyhow!("flac encode: {e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| anyhow::anyhow!("flac write: {e:?}"))?;
    std::fs::write(path, sink.as_slice())?;
    Ok(())
}

/// Write channels as an OGG Vorbis file (lossy VBR). `quality` is the Vorbis
/// target quality in [0, 1] (~0.5 ≈ transparent for game audio).
pub fn write_ogg(
    path: &Path,
    channels: &[&[f32]],
    sample_rate: u32,
    quality: f32,
) -> anyhow::Result<()> {
    use std::num::{NonZeroU8, NonZeroU32};
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = vorbis_rs::VorbisEncoderBuilder::new(
        NonZeroU32::new(sample_rate).ok_or_else(|| anyhow::anyhow!("sample_rate must be > 0"))?,
        NonZeroU8::new(channels.len() as u8).ok_or_else(|| anyhow::anyhow!("no channels"))?,
        &mut file,
    )?
    .bitrate_management_strategy(vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr {
        target_quality: quality.clamp(0.0, 1.0),
    })
    .build()?;
    encoder.encode_audio_block(channels)?;
    encoder.finish()?;
    Ok(())
}

/// Write one sample to the writer at the given bit depth (8 or 16).
fn write_sample(
    writer: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    x: f32,
    bits: u16,
) -> anyhow::Result<()> {
    if bits == 8 {
        // hound maps signed i8 to unsigned 8-bit WAV automatically.
        let v = (x.clamp(-1.0, 1.0) * 127.0).round().clamp(-128.0, 127.0) as i8;
        writer.write_sample(v)?;
    } else {
        let v = (x.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        writer.write_sample(v)?;
    }
    Ok(())
}

fn spec(channels: u16, sample_rate: u32, bits: u16) -> hound::WavSpec {
    hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: bits,
        sample_format: hound::SampleFormat::Int,
    }
}

/// Write mono `f32` samples (in [-1, 1]) to a WAV file at `path`.
/// `bits` is 8 or 16; anything else falls back to 16.
pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32, bits: u16) -> anyhow::Result<()> {
    let bits = if bits == 8 { 8 } else { 16 };
    let mut writer = hound::WavWriter::create(path, spec(1, sample_rate, bits))?;
    for &x in samples {
        write_sample(&mut writer, x, bits)?;
    }
    writer.finalize()?;
    Ok(())
}

/// Write interleaved-stereo `f32` samples (left/right in [-1, 1]) to `path`.
pub fn write_wav_stereo(
    path: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
    bits: u16,
) -> anyhow::Result<()> {
    let bits = if bits == 8 { 8 } else { 16 };
    let mut writer = hound::WavWriter::create(path, spec(2, sample_rate, bits))?;
    let n = left.len().min(right.len());
    for i in 0..n {
        write_sample(&mut writer, left[i], bits)?;
        write_sample(&mut writer, right[i], bits)?;
    }
    writer.finalize()?;
    Ok(())
}

/// Append a `smpl` chunk with one forward loop to an existing WAV file, so a
/// game engine (Godot / Unity / FMOD) loops at sample-accurate points without
/// manual setup. `loop_start` / `loop_end` are sample-frame indices (inclusive
/// end). Patches the RIFF size to include the new chunk.
pub fn append_smpl_loop(
    path: &Path,
    sample_rate: u32,
    loop_start: u32,
    loop_end: u32,
) -> anyhow::Result<()> {
    let mut bytes = std::fs::read(path)?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        anyhow::bail!("not a RIFF/WAVE file: {}", path.display());
    }
    let sample_period = 1_000_000_000u32.checked_div(sample_rate).unwrap_or(0);
    // smpl header (9 u32) + one loop (6 u32) = 60 bytes of chunk data.
    let mut chunk: Vec<u8> = Vec::with_capacity(68);
    chunk.extend_from_slice(b"smpl");
    chunk.extend_from_slice(&60u32.to_le_bytes());
    // manufacturer, product, samplePeriod, MIDIUnityNote, MIDIPitchFraction,
    // SMPTEFormat, SMPTEOffset, numSampleLoops, samplerData.
    for v in [0u32, 0, sample_period, 60, 0, 0, 0, 1, 0] {
        chunk.extend_from_slice(&v.to_le_bytes());
    }
    // loop: cuePointId, type(0=forward), start, end, fraction, playCount(0=inf).
    for v in [0u32, 0, loop_start, loop_end, 0, 0] {
        chunk.extend_from_slice(&v.to_le_bytes());
    }
    bytes.extend_from_slice(&chunk);
    let riff_size = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("sonarium_audio_test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| (i as f32 / n as f32) * 2.0 - 1.0).collect()
    }

    #[test]
    fn wav_roundtrips_through_hound() {
        let samples = ramp(1000);
        let path = tmp("mono.wav");
        write_wav(&path, &samples, 44_100, 16).unwrap();
        let reader = hound::WavReader::open(&path).unwrap();
        let s = reader.spec();
        assert_eq!(
            (s.channels, s.sample_rate, s.bits_per_sample),
            (1, 44_100, 16)
        );
        assert_eq!(reader.len(), 1000);
    }

    #[test]
    fn stereo_wav_interleaves_both_channels() {
        let path = tmp("stereo.wav");
        write_wav_stereo(&path, &ramp(500), &ramp(500), 48_000, 16).unwrap();
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.len(), 1000); // 500 frames × 2 channels
    }

    #[test]
    fn smpl_chunk_appends_60_bytes_and_patches_riff() {
        let path = tmp("loop.wav");
        write_wav(&path, &ramp(100), 44_100, 16).unwrap();
        let before = std::fs::read(&path).unwrap().len();
        append_smpl_loop(&path, 44_100, 0, 99).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), before + 68); // "smpl" + size + 60 data bytes
        let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(riff as usize, bytes.len() - 8);
        assert!(bytes.windows(4).any(|w| w == b"smpl"));
    }

    #[test]
    fn flac_and_ogg_write_nonempty_files() {
        let samples = ramp(4410);
        let flac = tmp("out.flac");
        write_flac(&flac, &[&samples], 44_100, 16).unwrap();
        assert!(std::fs::metadata(&flac).unwrap().len() > 0);

        let ogg = tmp("out.ogg");
        write_ogg(&ogg, &[&samples], 44_100, 0.5).unwrap();
        let bytes = std::fs::read(&ogg).unwrap();
        assert_eq!(&bytes[0..4], b"OggS");
    }
}
