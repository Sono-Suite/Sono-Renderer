//! Domain-neutral helpers for inspecting audio media used by editors.
//!
//! This module owns bounded waveform summaries and the PCM WAV inspection
//! needed by generic media workflows. Device enumeration, capture lifecycle,
//! and recovery remain in `audio_input`.

use anyhow::{bail, Context, Result};
use std::{
    fs::{self, File},
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
    process::{Command, Stdio},
};

pub(crate) const PCM_WAV_HEADER_BYTES: u64 = 44;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PcmWavFormat {
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
}

/// Read the fixed-layout signed 16-bit PCM WAV format supported by the
/// existing media inspector and return its complete frame count.
pub(crate) fn read_pcm_wav_format(path: &Path) -> Result<(PcmWavFormat, u64)> {
    let metadata =
        fs::metadata(path).with_context(|| format!("reading recorded audio {}", path.display()))?;
    if !metadata.is_file() || metadata.len() < PCM_WAV_HEADER_BYTES {
        bail!("recorded audio is shorter than a PCM WAV header");
    }
    let mut file = BufReader::with_capacity(64 * 1024, File::open(path)?);
    let mut header = [0u8; PCM_WAV_HEADER_BYTES as usize];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF"
        || &header[8..12] != b"WAVE"
        || &header[12..16] != b"fmt "
        || &header[36..40] != b"data"
    {
        bail!("recorded audio has an invalid or unsupported WAV layout");
    }
    if u16::from_le_bytes([header[20], header[21]]) != 1
        || u16::from_le_bytes([header[34], header[35]]) != 16
    {
        bail!("recorded audio is not signed 16-bit PCM WAV");
    }
    let format = PcmWavFormat {
        channels: u16::from_le_bytes([header[22], header[23]]),
        sample_rate: u32::from_le_bytes(header[24..28].try_into().unwrap()),
    };
    if format.channels == 0 || format.sample_rate == 0 {
        bail!("recorded WAV has an invalid channel count or sample rate");
    }
    let bytes_in_file = metadata.len().saturating_sub(PCM_WAV_HEADER_BYTES);
    let declared = u64::from(u32::from_le_bytes(header[40..44].try_into().unwrap()));
    let data_bytes = declared.min(bytes_in_file);
    let alignment = u64::from(format.channels) * 2;
    let data_bytes = data_bytes - data_bytes % alignment;
    Ok((format, data_bytes / alignment))
}

/// Return a bounded peak envelope for a supported signed 16-bit PCM WAV.
pub fn waveform_peaks(path: &Path, bins: usize) -> Result<Vec<f32>> {
    if bins == 0 || bins > 16_384 {
        bail!("waveform resolution must be in 1..=16384 bins");
    }
    let (format, frames) = read_pcm_wav_format(path)?;
    if frames == 0 {
        return Ok(Vec::new());
    }
    let mut file = BufReader::with_capacity(64 * 1024, File::open(path)?);
    file.seek(SeekFrom::Start(PCM_WAV_HEADER_BYTES))?;
    let mut peaks = vec![0.0f32; bins.min(frames as usize)];
    let mut frame = vec![0u8; usize::from(format.channels) * 2];
    for index in 0..frames {
        file.read_exact(&mut frame)?;
        let peak = frame
            .chunks_exact(2)
            .map(|bytes| {
                f32::from(i16::from_le_bytes([bytes[0], bytes[1]])).abs() / i16::MAX as f32
            })
            .fold(0.0, f32::max);
        let bin = ((index as u128 * peaks.len() as u128) / frames as u128) as usize;
        let bin = bin.min(peaks.len() - 1);
        peaks[bin] = peaks[bin].max(peak);
    }
    Ok(peaks)
}

/// Decode any FFmpeg-supported audio source to a small mono peak summary.
/// The decoded stream is consumed incrementally and never retained in memory.
pub fn waveform_for_media(path: &Path, duration_seconds: f64, bins: usize) -> Result<Vec<f32>> {
    let tools = crate::ffmpeg::resolve()?;
    waveform_for_media_with_tools(path, duration_seconds, bins, &tools)
}

/// Existing-installation-only variant for workflows that must not provision
/// optional tools as a side effect.
pub(crate) fn waveform_for_media_with_tools(
    path: &Path,
    duration_seconds: f64,
    bins: usize,
    tools: &crate::ffmpeg::FfmpegInstallation,
) -> Result<Vec<f32>> {
    if bins == 0 || bins > 16_384 || !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        bail!("waveform source duration or resolution is invalid");
    }
    let mut child = crate::export_control::hide_child_window(&mut Command::new(&tools.ffmpeg))
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "file,pipe,crypto,data",
            "-i",
        ])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "1000", "-f", "s16le", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("starting audio waveform decode for {}", path.display()))?;
    let mut output = child
        .stdout
        .take()
        .context("FFmpeg waveform decoder has no output pipe")?;
    let estimated_samples = (duration_seconds * 1000.0).ceil().max(1.0) as u64;
    let mut peaks = vec![0.0f32; bins.min(estimated_samples as usize).max(1)];
    let mut read_buffer = [0u8; 8192];
    let mut trailing = None;
    let mut sample_index = 0u64;
    loop {
        let count = output
            .read(&mut read_buffer)
            .context("reading decoded audio for waveform")?;
        if count == 0 {
            break;
        }
        let mut start = 0;
        if let Some(first) = trailing.take() {
            let value = i16::from_le_bytes([first, read_buffer[0]]);
            record_peak(&mut peaks, sample_index, estimated_samples, value);
            sample_index += 1;
            start = 1;
        }
        let even_count = (count - start) & !1;
        for bytes in read_buffer[start..start + even_count].chunks_exact(2) {
            let value = i16::from_le_bytes([bytes[0], bytes[1]]);
            record_peak(&mut peaks, sample_index, estimated_samples, value);
            sample_index += 1;
        }
        if (count - start) % 2 == 1 {
            trailing = Some(read_buffer[count - 1]);
        }
    }
    let status = child.wait().context("waiting for audio waveform decoder")?;
    if !status.success() {
        bail!(
            "FFmpeg could not decode audio waveform source {}",
            path.display()
        );
    }
    if sample_index == 0 {
        return Ok(Vec::new());
    }
    // If the probed duration was slightly short, fold overflow into the last
    // bin instead of indexing beyond the bounded summary.
    Ok(peaks)
}

fn record_peak(peaks: &mut [f32], index: u64, estimated_samples: u64, sample: i16) {
    let bin = ((u128::from(index) * peaks.len() as u128) / u128::from(estimated_samples)) as usize;
    let bin = bin.min(peaks.len() - 1);
    let level = f32::from(sample).abs() / f32::from(i16::MAX);
    peaks[bin] = peaks[bin].max(level);
}

/// Inspect a finalized PCM WAV and return its format, frame count, and peak
/// summary for use as ordinary project media.
pub fn inspect_pcm_wav(path: &Path) -> Result<(u32, u16, u64, Vec<f32>)> {
    let (format, frames) = read_pcm_wav_format(path)?;
    if frames == 0 {
        bail!("recorded WAV contains no complete audio frames");
    }
    let peaks = waveform_peaks(path, 512)?;
    Ok((format.sample_rate, format.channels, frames, peaks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn pcm_wav_inspection_returns_bounded_peaks_and_frame_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.wav");
        let samples = [0i16, 16_384, -32_768, 8_192];
        let data_bytes = (samples.len() * 2) as u32;
        let mut file = File::create(&path).unwrap();
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + data_bytes).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16u32.to_le_bytes()).unwrap();
        file.write_all(&1u16.to_le_bytes()).unwrap();
        file.write_all(&1u16.to_le_bytes()).unwrap();
        file.write_all(&48_000u32.to_le_bytes()).unwrap();
        file.write_all(&96_000u32.to_le_bytes()).unwrap();
        file.write_all(&2u16.to_le_bytes()).unwrap();
        file.write_all(&16u16.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&data_bytes.to_le_bytes()).unwrap();
        for sample in samples {
            file.write_all(&sample.to_le_bytes()).unwrap();
        }

        let (sample_rate, channels, frames, peaks) = inspect_pcm_wav(&path).unwrap();
        assert_eq!((sample_rate, channels, frames), (48_000, 1, 4));
        assert_eq!(peaks.len(), 4);
        assert!(peaks[2] > 1.0);
        assert_eq!(waveform_peaks(&path, 2).unwrap().len(), 2);
    }

    #[test]
    fn waveform_resolution_is_bounded() {
        assert!(waveform_peaks(Path::new("unused.wav"), 0).is_err());
        assert!(waveform_peaks(Path::new("unused.wav"), 16_385).is_err());
    }
}
