//! Continuous Presentation Studio playback. FFmpeg remains the authority for
//! composition video and audio filtering; CPAL supplies the audible output
//! clock used to place timeline time and select preview frames.

use anyhow::{bail, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, SampleFormat, SizedSample,
};
use std::{
    io::{BufReader, Read},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const AUDIO_BLOCK_FRAMES: usize = 1024;
const AUDIO_QUEUE_BLOCKS: usize = 12;
const VIDEO_QUEUE_FRAMES: usize = 3;

#[derive(Debug)]
pub struct VideoFrame {
    pub index: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct PlaybackClock {
    sample_rate: u32,
    position_frames: AtomicU64,
    playing: AtomicBool,
    ended: AtomicBool,
    underflows: AtomicU64,
    master_rms: AtomicU32,
    master_peak: AtomicU32,
    master_clipping: AtomicBool,
    error: Mutex<Option<String>>,
}

#[derive(Default)]
struct AudioMeterAccumulator {
    sum_squares: f64,
    samples: usize,
    peak: f32,
}

impl AudioMeterAccumulator {
    fn push(&mut self, sample: f32) {
        self.peak = self.peak.max(sample.abs());
        self.sum_squares += f64::from(sample) * f64::from(sample);
        self.samples += 1;
    }

    fn levels(&self) -> (f32, f32, bool) {
        let rms = if self.samples == 0 {
            0.0
        } else {
            (self.sum_squares / self.samples as f64).sqrt() as f32
        };
        (rms, self.peak, self.peak >= 0.999)
    }
}

impl PlaybackClock {
    fn new(sample_rate: u32, start_frame: u64) -> Arc<Self> {
        Arc::new(Self {
            sample_rate,
            position_frames: AtomicU64::new(start_frame),
            playing: AtomicBool::new(true),
            ended: AtomicBool::new(false),
            underflows: AtomicU64::new(0),
            master_rms: AtomicU32::new(0.0f32.to_bits()),
            master_peak: AtomicU32::new(0.0f32.to_bits()),
            master_clipping: AtomicBool::new(false),
            error: Mutex::new(None),
        })
    }

    fn seconds(&self) -> f64 {
        self.position_frames.load(Ordering::Relaxed) as f64 / f64::from(self.sample_rate)
    }
}

struct PcmPipe {
    receiver: Option<Receiver<Vec<f32>>>,
    first_block: Option<Vec<f32>>,
    child: Arc<Mutex<Option<Child>>>,
    reader: Option<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
}

impl PcmPipe {
    fn start(
        project: &crate::composition::CompositionProject,
        rate: u32,
        start_frame: u64,
        work: &std::path::Path,
    ) -> Result<Self> {
        let (program, args) = crate::composition::playback_audio_command(project, rate, work)?;
        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("starting the composition audio preview stream")?;
        let stdout = child
            .stdout
            .take()
            .context("FFmpeg audio preview has no output pipe")?;
        let child = Arc::new(Mutex::new(Some(child)));
        let error = Arc::new(Mutex::new(None));
        let (tx, receiver) = mpsc::sync_channel(AUDIO_QUEUE_BLOCKS);
        let worker_child = child.clone();
        let worker_error = error.clone();
        let reader = thread::Builder::new()
            .name("presentation-audio-decode".into())
            .spawn(move || {
                let result = read_pcm_blocks(stdout, tx, start_frame);
                if let Err(error) = result {
                    if let Ok(mut slot) = worker_error.lock() {
                        *slot = Some(format!("{error:#}"));
                    }
                }
                if let Ok(mut slot) = worker_child.lock() {
                    if let Some(mut child) = slot.take() {
                        if let Ok(status) = child.wait() {
                            if !status.success() {
                                if let Ok(mut error) = worker_error.lock() {
                                    error.get_or_insert_with(|| {
                                        format!("FFmpeg audio preview exited with {status}")
                                    });
                                }
                            }
                        }
                    }
                }
            })
            .context("starting the composition audio decoder")?;
        Ok(Self {
            receiver: Some(receiver),
            first_block: None,
            child,
            reader: Some(reader),
            error,
        })
    }

    fn prebuffer(&mut self) -> Result<()> {
        let receiver = self
            .receiver
            .as_ref()
            .context("audio preview stream was already consumed")?;
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(block) => {
                self.first_block = Some(block);
                Ok(())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => bail!("timed out preparing composition audio"),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let detail = self.error.lock().ok().and_then(|error| error.clone());
                bail!(
                    "composition audio preview ended before playback started{}",
                    detail
                        .map(|detail| format!(": {detail}"))
                        .unwrap_or_default()
                );
            }
        }
    }

    fn take_receiver(&mut self) -> Result<(Receiver<Vec<f32>>, Option<Vec<f32>>)> {
        Ok((
            self.receiver
                .take()
                .context("audio preview stream was already consumed")?,
            self.first_block.take(),
        ))
    }

    fn stop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for PcmPipe {
    fn drop(&mut self) {
        self.stop();
    }
}

fn read_pcm_blocks<R: Read>(
    mut stdout: R,
    tx: SyncSender<Vec<f32>>,
    start_frame: u64,
) -> Result<()> {
    let mut remaining_samples = start_frame.saturating_mul(2);
    let mut bytes = vec![0u8; AUDIO_BLOCK_FRAMES * 2 * std::mem::size_of::<f32>()];
    loop {
        let count = read_block(&mut stdout, &mut bytes)?;
        if count == 0 {
            break;
        }
        let sample_count = count / 4;
        let discard = remaining_samples.min(sample_count as u64) as usize;
        remaining_samples -= discard as u64;
        let first_byte = discard * 4;
        if first_byte == count {
            continue;
        }
        let samples = bytes[first_byte..count]
            .chunks_exact(4)
            .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
            .collect::<Vec<_>>();
        if tx.send(samples).is_err() {
            break;
        }
    }
    Ok(())
}

fn read_block(reader: &mut impl Read, buffer: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(filled - (filled % 8))
}

struct AudioOutput {
    _stream: Option<cpal::Stream>,
    clock: Arc<PlaybackClock>,
}

impl AudioOutput {
    fn create(
        receiver: Receiver<Vec<f32>>,
        first_block: Option<Vec<f32>>,
        start_frame: u64,
    ) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no system audio output device is available")?;
        let supported = device
            .default_output_config()
            .context("the default audio output has no usable configuration")?;
        let config: cpal::StreamConfig = supported.clone().into();
        let channels = usize::from(config.channels);
        let rate = config.sample_rate;
        let clock = PlaybackClock::new(rate, start_frame);
        let callback_clock = clock.clone();
        let callback_error = clock.clone();
        let mut initial = first_block;
        let stream = match supported.sample_format() {
            SampleFormat::I8 => build_output::<i8>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::I16 => build_output::<i16>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::I24 => build_output::<cpal::I24>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::I32 => build_output::<i32>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::I64 => build_output::<i64>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::U8 => build_output::<u8>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::U16 => build_output::<u16>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::U24 => build_output::<cpal::U24>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::U32 => build_output::<u32>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::U64 => build_output::<u64>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::F32 => build_output::<f32>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            SampleFormat::F64 => build_output::<f64>(
                &device,
                config,
                channels,
                receiver,
                &mut initial,
                callback_clock,
                callback_error,
            ),
            format => bail!("unsupported audio output sample format {format:?}"),
        }
        .context("building the composition audio output stream")?;
        stream
            .play()
            .context("starting the composition audio output")?;
        Ok(Self {
            _stream: Some(stream),
            clock,
        })
    }
}

fn build_output<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    receiver: Receiver<Vec<f32>>,
    initial: &mut Option<Vec<f32>>,
    clock: Arc<PlaybackClock>,
    error_clock: Arc<PlaybackClock>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let mut block = initial.take().unwrap_or_default();
    let mut offset = 0usize;
    let mut disconnected = false;
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                let channel_count = channels.max(1);
                let frame_count = output.len() / channel_count;
                let playing = clock.playing.load(Ordering::Relaxed);
                if !playing {
                    output.fill(T::from_sample(0.0));
                    clock.master_rms.store(0.0f32.to_bits(), Ordering::Relaxed);
                    clock.master_peak.store(0.0f32.to_bits(), Ordering::Relaxed);
                    clock.master_clipping.store(false, Ordering::Relaxed);
                    return;
                }
                let mut missed = false;
                let mut meter = AudioMeterAccumulator::default();
                for frame in output.chunks_mut(channel_count) {
                    if offset + 1 >= block.len() {
                        match receiver.try_recv() {
                            Ok(next) => {
                                block = next;
                                offset = 0;
                            }
                            Err(TryRecvError::Empty) => {
                                missed = true;
                                for sample in frame.iter_mut() {
                                    *sample = T::from_sample(0.0);
                                }
                                continue;
                            }
                            Err(TryRecvError::Disconnected) => {
                                disconnected = true;
                                clock.ended.store(true, Ordering::Relaxed);
                                clock.playing.store(false, Ordering::Relaxed);
                                for sample in frame.iter_mut() {
                                    *sample = T::from_sample(0.0);
                                }
                                continue;
                            }
                        }
                    }
                    let left = block[offset];
                    let right = block[offset + 1];
                    offset += 2;
                    if let Some(sample) = frame.first_mut() {
                        let mono = (left + right) * 0.5;
                        let first = if channels == 1 { mono } else { left };
                        *sample = T::from_sample(first);
                        meter.push(first);
                    }
                    if channels > 1 {
                        frame[1] = T::from_sample(right);
                        meter.push(right);
                    }
                    for sample in frame.iter_mut().skip(2) {
                        *sample = T::from_sample(0.0);
                    }
                }
                if missed {
                    clock.underflows.fetch_add(1, Ordering::Relaxed);
                }
                let (rms, peak, clipping) = meter.levels();
                clock.master_rms.store(rms.to_bits(), Ordering::Relaxed);
                clock.master_peak.store(peak.to_bits(), Ordering::Relaxed);
                clock.master_clipping.store(clipping, Ordering::Relaxed);
                if !disconnected {
                    clock
                        .position_frames
                        .fetch_add(frame_count as u64, Ordering::Relaxed);
                }
            },
            move |error| {
                if let Ok(mut slot) = error_clock.error.lock() {
                    *slot = Some(format!("audio output failed: {error}"));
                }
            },
            None,
        )
        .map_err(Into::into)
}

struct VideoPipe {
    receiver: Option<Receiver<VideoFrame>>,
    child: Arc<Mutex<Option<Child>>>,
    worker: Option<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
    finished: Arc<AtomicBool>,
}

impl VideoPipe {
    fn start(
        project: &crate::composition::CompositionProject,
        start_frame: u64,
        work: &std::path::Path,
    ) -> Result<(Self, u32, u32)> {
        let (program, args, width, height) =
            crate::composition::playback_video_command(project, work)?;
        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("starting the continuous composition video preview")?;
        let stdout = child
            .stdout
            .take()
            .context("FFmpeg video preview has no output pipe")?;
        let child = Arc::new(Mutex::new(Some(child)));
        let error = Arc::new(Mutex::new(None));
        let finished = Arc::new(AtomicBool::new(false));
        let (tx, receiver) = mpsc::sync_channel(VIDEO_QUEUE_FRAMES);
        let thread_child = child.clone();
        let thread_error = error.clone();
        let thread_finished = finished.clone();
        let frame_bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .context("preview frame dimensions overflow memory limits")?;
        let worker = thread::Builder::new()
            .name("presentation-video-decode".into())
            .spawn(move || {
                let result = read_video_frames(stdout, tx, frame_bytes, width, height, start_frame);
                if let Err(error) = result {
                    if let Ok(mut slot) = thread_error.lock() {
                        *slot = Some(format!("{error:#}"));
                    }
                }
                if let Ok(mut slot) = thread_child.lock() {
                    if let Some(mut child) = slot.take() {
                        if let Ok(status) = child.wait() {
                            if !status.success() {
                                if let Ok(mut error) = thread_error.lock() {
                                    error.get_or_insert_with(|| {
                                        format!("FFmpeg video preview exited with {status}")
                                    });
                                }
                            }
                        }
                    }
                }
                thread_finished.store(true, Ordering::Release);
            })
            .context("starting the composition video decoder")?;
        Ok((
            Self {
                receiver: Some(receiver),
                child,
                worker: Some(worker),
                error,
                finished,
            },
            width,
            height,
        ))
    }

    fn try_recv(&self) -> Result<Option<VideoFrame>> {
        match self
            .receiver
            .as_ref()
            .context("video preview stream has stopped")?
            .try_recv()
        {
            Ok(frame) => Ok(Some(frame)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                if let Some(error) = self.error.lock().ok().and_then(|error| error.clone()) {
                    bail!("{error}");
                }
                Ok(None)
            }
        }
    }

    fn stop(&mut self) {
        self.receiver.take();
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for VideoPipe {
    fn drop(&mut self) {
        self.stop();
    }
}

fn read_video_frames<R: Read>(
    mut stdout: R,
    tx: SyncSender<VideoFrame>,
    frame_bytes: usize,
    width: u32,
    height: u32,
    start_frame: u64,
) -> Result<()> {
    let mut reader = BufReader::with_capacity(frame_bytes.min(8 * 1024 * 1024), &mut stdout);
    let mut index = 0u64;
    loop {
        let mut rgba = vec![0; frame_bytes];
        match reader.read_exact(&mut rgba) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
        if index >= start_frame {
            if tx
                .send(VideoFrame {
                    index,
                    width,
                    height,
                    rgba,
                })
                .is_err()
            {
                break;
            }
        }
        index = index.saturating_add(1);
    }
    Ok(())
}

/// A playing composition owns one FFmpeg video pipe and one FFmpeg audio pipe
/// feeding the system output device. The CPAL callback's submitted audio frame
/// count is the playback clock; GUI repaint cadence only samples that clock.
pub struct TimelinePlayback {
    pcm: Option<PcmPipe>,
    output: Option<AudioOutput>,
    video: Option<VideoPipe>,
    fps: u32,
    duration: f64,
    width: u32,
    height: u32,
    pending_frame: Option<VideoFrame>,
    _work: tempfile::TempDir,
}

impl TimelinePlayback {
    pub fn start(
        project: &crate::composition::CompositionProject,
        start_seconds: f64,
        paused: bool,
    ) -> Result<Self> {
        project.validate()?;
        let start_seconds = start_seconds.clamp(0.0, project.duration);
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no system audio output device is available")?;
        let output_config = device
            .default_output_config()
            .context("the default audio output has no usable configuration")?;
        let sample_rate = output_config.sample_rate();
        let start_frame = (start_seconds * f64::from(sample_rate)).round() as u64;
        let work = tempfile::tempdir().context("creating the composition playback workspace")?;
        let mut pcm = PcmPipe::start(project, sample_rate, start_frame, work.path())?;
        pcm.prebuffer()?;
        let (receiver, first_block) = pcm.take_receiver()?;
        let output = AudioOutput::create(receiver, first_block, start_frame)?;
        let (video, width, height) = VideoPipe::start(
            project,
            (start_seconds * f64::from(project.fps)).floor() as u64,
            work.path(),
        )?;
        if paused {
            output.clock.playing.store(false, Ordering::Relaxed);
        }
        Ok(Self {
            pcm: Some(pcm),
            output: Some(output),
            video: Some(video),
            fps: project.fps,
            duration: project.duration,
            width,
            height,
            pending_frame: None,
            _work: work,
        })
    }

    pub fn set_playing(&self, playing: bool) {
        if let Some(output) = &self.output {
            output.clock.playing.store(playing, Ordering::Relaxed);
        }
    }

    pub fn is_playing(&self) -> bool {
        self.output
            .as_ref()
            .is_some_and(|output| output.clock.playing.load(Ordering::Relaxed))
    }

    pub fn position_seconds(&self) -> f64 {
        self.output
            .as_ref()
            .map_or(0.0, |output| output.clock.seconds().min(self.duration))
    }

    pub fn is_ended(&self) -> bool {
        self.output
            .as_ref()
            .is_some_and(|output| output.clock.ended.load(Ordering::Relaxed))
    }

    pub fn underflow_count(&self) -> u64 {
        self.output
            .as_ref()
            .map_or(0, |output| output.clock.underflows.load(Ordering::Relaxed))
    }

    /// Returns the most recent output callback's RMS/peak and a near-full-scale
    /// clipping indication. Atomics keep the real-time callback lock-free.
    pub fn master_meter(&self) -> (f32, f32, bool) {
        self.output.as_ref().map_or((0.0, 0.0, false), |output| {
            let clock = &output.clock;
            (
                f32::from_bits(clock.master_rms.load(Ordering::Relaxed)),
                f32::from_bits(clock.master_peak.load(Ordering::Relaxed)),
                clock.master_clipping.load(Ordering::Relaxed),
            )
        })
    }

    pub fn take_error(&self) -> Option<String> {
        if let Some(output) = &self.output {
            if let Ok(mut error) = output.clock.error.lock() {
                if error.is_some() {
                    return error.take();
                }
            }
        }
        self.pcm
            .as_ref()
            .and_then(|pcm| pcm.error.lock().ok()?.take())
            .or_else(|| {
                self.video
                    .as_ref()
                    .and_then(|video| video.error.lock().ok()?.take())
            })
    }

    pub fn frame_dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn video_preview_finished(&self) -> bool {
        self.video
            .as_ref()
            .is_some_and(|video| video.finished.load(Ordering::Acquire))
    }

    pub fn next_video_frame(&mut self) -> Result<Option<VideoFrame>> {
        let desired = (self.position_seconds() * f64::from(self.fps)).floor() as u64;
        let mut newest = None;
        loop {
            let frame = if let Some(pending) = self.pending_frame.take() {
                Some(pending)
            } else {
                self.video
                    .as_ref()
                    .context("video preview stopped")?
                    .try_recv()?
            };
            let Some(frame) = frame else { break };
            if frame.index > desired {
                self.pending_frame = Some(frame);
                break;
            }
            newest = Some(frame);
        }
        Ok(newest)
    }

    pub fn frame_dropped_by_renderer(&self) -> bool {
        self.position_seconds() >= self.duration
    }

    pub fn stop(&mut self) {
        self.output.take();
        self.pcm.take();
        self.video.take();
    }
}

impl Drop for TimelinePlayback {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn pcm_reader_discards_a_seek_prefix_and_keeps_stereo_frame_boundaries() {
        let samples = (0..16)
            .map(|n| (n as f32).to_le_bytes())
            .flatten()
            .collect::<Vec<_>>();
        let (tx, rx) = mpsc::sync_channel(2);
        read_pcm_blocks(Cursor::new(samples), tx, 2).unwrap();
        let decoded = rx.try_iter().flatten().collect::<Vec<_>>();
        assert_eq!(decoded, (4..16).map(|n| n as f32).collect::<Vec<_>>());
    }

    #[test]
    fn preview_dimensions_preserve_aspect_ratio_and_bound_memory() {
        assert_eq!(
            crate::composition::playback_dimensions(1920, 1080),
            (960, 540)
        );
        assert_eq!(
            crate::composition::playback_dimensions(1080, 1920),
            (540, 960)
        );
        assert_eq!(
            crate::composition::playback_dimensions(801, 533),
            (800, 532)
        );
    }

    #[test]
    fn output_meter_tracks_rms_peak_and_near_full_scale_clipping() {
        let mut meter = AudioMeterAccumulator::default();
        meter.push(0.5);
        meter.push(-0.5);
        let (rms, peak, clipping) = meter.levels();
        assert!((rms - 0.5).abs() < 0.0001);
        assert_eq!(peak, 0.5);
        assert!(!clipping);

        meter.push(1.0);
        let (_, peak, clipping) = meter.levels();
        assert_eq!(peak, 1.0);
        assert!(clipping);
    }

    #[test]
    #[ignore = "requires the already-installed app-managed FFmpeg/FFprobe pair"]
    fn video_pipe_keeps_animation_and_text_filter_files_alive_until_decode_finishes() -> Result<()>
    {
        let tools = crate::ffmpeg::inspect_existing()?
            .context("this integration test requires the managed FFmpeg pair")?;
        let directory = tempfile::tempdir()?;
        let video = directory.path().join("ordinary-video.mp4");
        let mut generate = Command::new(tools.ffmpeg);
        generate
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=30:duration=1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&video);
        let generated = crate::export_control::ExportControl::default()
            .output(crate::export_control::hide_child_window(&mut generate))?;
        anyhow::ensure!(
            generated.status.success(),
            "could not generate the video fixture"
        );

        let mut project = crate::composition::CompositionProject::new();
        project.width = 640;
        project.height = 360;
        project.duration = 1.0;
        let source_id = project.add_source(&video)?;
        let clip_id = project.add_clip(&source_id, crate::composition::ClipKind::Video, 0.0)?;
        let clip = project
            .clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
            .unwrap();
        clip.animated_properties.insert(
            crate::composition::AnimatedProperty::Blur,
            vec![
                crate::composition::ScalarKeyframe {
                    time: 0.0,
                    value: 0.0,
                    easing_to_next: crate::composition::Easing::Linear,
                },
                crate::composition::ScalarKeyframe {
                    time: 0.5,
                    value: 2.0,
                    easing_to_next: crate::composition::Easing::Linear,
                },
            ],
        );
        clip.animated_properties.insert(
            crate::composition::AnimatedProperty::CropLeft,
            vec![
                crate::composition::ScalarKeyframe {
                    time: 0.0,
                    value: 0.0,
                    easing_to_next: crate::composition::Easing::Linear,
                },
                crate::composition::ScalarKeyframe {
                    time: 0.5,
                    value: 0.1,
                    easing_to_next: crate::composition::Easing::Linear,
                },
            ],
        );
        project.add_text_clip("Preview", 0.0, 1.0)?;
        let work = tempfile::tempdir()?;
        let (mut pipe, _, _) = VideoPipe::start(&project, 0, work.path())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let frame = loop {
            if let Some(frame) = pipe.try_recv()? {
                break frame;
            }
            if std::time::Instant::now() >= deadline {
                bail!("timed out waiting for a continuous preview frame");
            }
            thread::sleep(Duration::from_millis(5));
        };
        anyhow::ensure!(!frame.rgba.is_empty(), "continuous preview frame is empty");
        pipe.stop();
        Ok(())
    }
}
