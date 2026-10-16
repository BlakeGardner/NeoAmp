//! Reading one audio file or stream with symphonia: its tags and length, and its audio as
//! stereo `f32` at its own sample rate.

use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result, bail};
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, StandardTag};
use symphonia::core::units::{Duration, Time, TimeBase, Timestamp};

use super::{FileDetails, Tags, TrackInfo};

/// How much audio the bitrate readout averages over, in seconds.
const BITRATE_WINDOW: f64 = 0.5;

/// An open file being decoded.
pub(super) struct Source {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    /// The file's sample rate in Hz.
    pub rate: u32,
    pub info: TrackInfo,
    /// Frames still to drop after a seek so playback starts exactly where asked.
    skip: u64,
    /// One decoded packet, interleaved in the file's own channel layout.
    decoded: Vec<f32>,
    /// Packet bytes and seconds since the bitrate was last measured.
    bitrate_bytes: u64,
    bitrate_secs: f64,
}

/// What [`Source::read`] found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Read {
    /// A packet's audio.
    Audio,
    /// The end of the file, or of what a stream's connection delivered.
    End,
    /// A new stream starts in the same bytes, with tracks and decoders of its own: the next
    /// song of a chained Ogg stream, as Icecast sends them. See [`Source::next_stream`].
    NewStream,
}

/// A probed file before a decoder is attached.
struct Opened {
    format: Box<dyn FormatReader>,
    track_id: u32,
    time_base: Option<TimeBase>,
    info: TrackInfo,
}

fn open_format(path: &Path) -> Result<(Opened, FileDetails)> {
    let file = File::open(path)?;
    let size = file.metadata().map(|m| m.len()).ok();
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    probe_format(mss, &hint, size)
}

/// Recognise what `mss` holds, `size` bytes of it if that is known, and read its tags.
fn probe_format(
    mss: MediaSourceStream<'static>,
    hint: &Hint,
    size: Option<u64>,
) -> Result<(Opened, FileDetails)> {
    let mut format = symphonia::default::get_probe()
        .probe(
            hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .context("unsupported or unrecognised format")?;
    let (track, details) = describe(format.as_mut(), size, false)?;
    let opened = Opened {
        format,
        track_id: track.0,
        time_base: track.1,
        info: details.info.clone(),
    };
    Ok((opened, details))
}

/// The audio track `format` plays (its id and time base), and what it holds. The tags are those
/// of every metadata revision so far, or with `latest`, only of the newest: those of the song
/// a chained stream has just started.
fn describe(
    format: &mut dyn FormatReader,
    size: Option<u64>,
    latest: bool,
) -> Result<((u32, Option<TimeBase>), FileDetails)> {
    let track = format
        .default_track(TrackType::Audio)
        .context("no audio track")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .context("no audio codec parameters")?;
    let Some(rate) = params.sample_rate.filter(|r| *r > 0) else {
        bail!("unknown sample rate");
    };
    let channels = params.channels.as_ref().map_or(2, |c| c.count());
    let seconds = track
        .time_base
        .zip(track.duration)
        .and_then(|(tb, d)| tb.calc_duration(d))
        .map(|t| t.as_secs_f64())
        // Frame counts stay far below 2^52, so the conversion is exact.
        .or_else(|| track.num_frames.map(|n| n as f64 / f64::from(rate)))
        .filter(|s| *s > 0.0);
    let kbps = size.zip(seconds).map(|(bytes, secs)| {
        // Both are positive and the result is a few thousand at most.
        (bytes as f64 * 8.0 / secs / 1000.0).round() as u32
    });
    let codec = symphonia::default::get_codecs()
        .get_audio_decoder(params.codec)
        .map_or_else(String::new, |d| d.codec.info.long_name.to_owned());
    let bits_per_sample = params.bits_per_sample;
    let track_id = track.id;
    let time_base = track.time_base;
    let tags = read_tags(format, latest);
    let info = TrackInfo {
        title: tags.playlist_title(),
        seconds,
        sample_rate: rate,
        channels,
        kbps,
    };
    let details = FileDetails {
        tags,
        format: format.format_info().long_name.to_owned(),
        codec,
        info,
        bits_per_sample,
        size,
    };
    Ok(((track_id, time_base), details))
}

/// Read a file's tags and length without decoding it.
pub(super) fn probe(path: &Path) -> Result<TrackInfo> {
    open_format(path).map(|(o, _)| o.info)
}

/// Read everything the file info dialog shows, without decoding the file.
pub(super) fn details(path: &Path) -> Result<FileDetails> {
    open_format(path).map(|(_, d)| d)
}

/// The tags in every metadata revision read so far, or with `latest`, in the newest only.
/// Where a tag appears more than once, the first non-empty value counts, from the oldest
/// revision on, and in each the media's tags before the track's.
fn read_tags(format: &mut dyn FormatReader, latest: bool) -> Tags {
    let mut tags = Tags::default();
    let mut track_number = None;
    let mut track_total = None;
    let mut log = format.metadata();
    if latest {
        log.skip_to_latest();
    }
    loop {
        if let Some(rev) = log.current() {
            let per_track = rev.per_track.iter().flat_map(|t| &t.metadata.tags);
            for tag in rev.media.tags.iter().chain(per_track) {
                let Some(std) = &tag.std else {
                    continue;
                };
                let (slot, value) = match std {
                    StandardTag::TrackTitle(v) => (&mut tags.title, v.to_string()),
                    StandardTag::Artist(v) => (&mut tags.artist, v.to_string()),
                    StandardTag::Album(v) => (&mut tags.album, v.to_string()),
                    StandardTag::Genre(v) => (&mut tags.genre, v.to_string()),
                    // Vorbis comments often hold the comment as DESCRIPTION.
                    StandardTag::Comment(v) | StandardTag::Description(v) => {
                        (&mut tags.comment, v.to_string())
                    }
                    StandardTag::RecordingDate(v) | StandardTag::ReleaseDate(v) => {
                        (&mut tags.year, v.to_string())
                    }
                    StandardTag::RecordingYear(y) | StandardTag::ReleaseYear(y) => {
                        (&mut tags.year, y.to_string())
                    }
                    StandardTag::TrackNumber(n) => (&mut track_number, n.to_string()),
                    StandardTag::TrackTotal(n) => (&mut track_total, n.to_string()),
                    _ => continue,
                };
                let value = value.trim();
                if slot.is_none() && !value.is_empty() {
                    *slot = Some(value.to_owned());
                }
            }
        }
        if log.pop().is_none() {
            break;
        }
    }
    tags.track = match (track_number, track_total) {
        (Some(n), Some(total)) => Some(format!("{n}/{total}")),
        (n, _) => n,
    };
    tags
}

impl Source {
    pub(super) fn open(path: &Path) -> Result<Source> {
        let (opened, _) = open_format(path)?;
        Source::decode(opened)
    }

    /// Start reading a stream from `media`, which cannot seek. `hint` may give its media type
    /// and the extension its URL ends in. Returns the source and what the stream holds.
    pub(super) fn open_stream(
        media: Box<dyn MediaSource>,
        hint: &Hint,
    ) -> Result<(Source, FileDetails)> {
        let mss = MediaSourceStream::new(media, Default::default());
        let (opened, details) = probe_format(mss, hint, None)?;
        Ok((Source::decode(opened)?, details))
    }

    /// After [`Read::NewStream`]: decode the stream that has started, whose tracks and tags
    /// the format reader has read already (it has moved on to them).
    pub(super) fn next_stream(mut self) -> Result<(Source, FileDetails)> {
        let ((track_id, time_base), details) = describe(self.format.as_mut(), None, true)?;
        let opened = Opened {
            format: self.format,
            track_id,
            time_base,
            info: details.info.clone(),
        };
        Ok((Source::decode(opened)?, details))
    }

    /// Attach a decoder to a probed file or stream.
    fn decode(opened: Opened) -> Result<Source> {
        let track = opened
            .format
            .tracks()
            .iter()
            .find(|t| t.id == opened.track_id)
            .context("audio track vanished")?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .context("no audio codec parameters")?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .with_context(|| {
                if params.codec == CODEC_ID_OPUS {
                    "unsupported codec (Opus)".to_owned()
                } else {
                    format!("unsupported codec ({})", params.codec)
                }
            })?;
        Ok(Source {
            format: opened.format,
            decoder,
            track_id: opened.track_id,
            time_base: opened.time_base,
            rate: opened.info.sample_rate,
            info: opened.info,
            skip: 0,
            decoded: Vec::new(),
            bitrate_bytes: 0,
            bitrate_secs: 0.0,
        })
    }

    fn seconds(&self, ts: Timestamp) -> f64 {
        match self.time_base {
            Some(tb) => tb.calc_time_saturating(ts).as_secs_f64(),
            // Timestamps stay far below 2^52, so the conversion is exact.
            None => ts.get() as f64 / f64::from(self.rate),
        }
    }

    fn duration_seconds(&self, dur: Duration) -> f64 {
        match self.time_base {
            Some(tb) => tb.calc_duration_saturating(dur).as_secs_f64(),
            None => dur.get() as f64 / f64::from(self.rate),
        }
    }

    /// Decode the next packet and append it to `out` as interleaved stereo.
    pub(super) fn read(&mut self, out: &mut Vec<f32>) -> Result<Read> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(Read::End),
                Err(Error::ResetRequired) => return Ok(Read::NewStream),
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => {
                    return Ok(Read::End);
                }
                Err(e) => return Err(e.into()),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            // usize always fits in u64 on supported targets.
            self.bitrate_bytes += packet.data.len() as u64;
            self.bitrate_secs += self.duration_seconds(packet.dur);
            let buf = match self.decoder.decode(&packet) {
                Ok(buf) => buf,
                // A damaged packet: skip it, as every player does.
                Err(Error::DecodeError(e)) => {
                    log::debug!("skipping an undecodable packet: {e}");
                    continue;
                }
                Err(Error::IoError(e)) => {
                    log::debug!("skipping a packet: {e}");
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let channels = buf.spec().channels().count();
            let frames = buf.frames();
            buf.copy_to_vec_interleaved(&mut self.decoded);
            let skip = usize::try_from(self.skip).unwrap_or(usize::MAX).min(frames);
            self.skip -= skip as u64;
            if skip == frames || channels == 0 {
                continue;
            }
            to_stereo(&self.decoded[skip * channels..], channels, out);
            return Ok(Read::Audio);
        }
    }

    /// Jump to `seconds` and return where playback now starts.
    pub(super) fn seek(&mut self, seconds: f64) -> Result<f64> {
        let time = Time::try_from_secs_f64(seconds.max(0.0)).context("seek time out of range")?;
        let to = self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(self.track_id),
            },
        )?;
        self.decoder.reset();
        let required = self.seconds(to.required_ts);
        let actual = self.seconds(to.actual_ts);
        // The reader lands on the packet at or before the requested time; decode from there and
        // drop the frames in between. Non-negative and small, so the cast cannot truncate.
        self.skip = ((required - actual) * f64::from(self.rate))
            .round()
            .max(0.0) as u64;
        Ok(required)
    }

    /// The average bitrate since the last call, once enough audio has been read to say.
    pub(super) fn take_kbps(&mut self) -> Option<u32> {
        if self.bitrate_secs < BITRATE_WINDOW {
            return None;
        }
        // Positive and a few thousand at most.
        let kbps = (self.bitrate_bytes as f64 * 8.0 / self.bitrate_secs / 1000.0).round() as u32;
        self.bitrate_bytes = 0;
        self.bitrate_secs = 0.0;
        Some(kbps)
    }
}

/// Append interleaved `samples` with `channels` per frame to `out` as stereo: mono is doubled,
/// and beyond two channels only the front left and right are kept.
fn to_stereo(samples: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        0 => {}
        1 => out.extend(samples.iter().flat_map(|&s| [s, s])),
        2 => out.extend_from_slice(samples),
        _ => {
            for frame in samples.chunks_exact(channels) {
                out.extend_from_slice(&frame[..2]);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(super) mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Write a 16-bit PCM WAV of `frames` frames per channel; sample `i` of channel `c` is
    /// `value(i, c)`.
    pub(crate) fn write_wav(
        name: &str,
        rate: u32,
        channels: u16,
        frames: u32,
        value: impl Fn(u32, u16) -> i16,
    ) -> PathBuf {
        let path = std::env::temp_dir().join(format!("neoamp-{}-{name}.wav", std::process::id()));
        let data_len = frames * u32::from(channels) * 2;
        let mut b = Vec::with_capacity(44 + data_len as usize);
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        b.extend_from_slice(&(channels * 2).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            for c in 0..channels {
                b.extend_from_slice(&value(i, c).to_le_bytes());
            }
        }
        std::fs::write(&path, b).unwrap();
        path
    }

    fn read_all(src: &mut Source) -> Vec<f32> {
        let mut out = Vec::new();
        while src.read(&mut out).unwrap() == Read::Audio {}
        out
    }

    #[test]
    fn wav_probes_and_decodes_to_stereo() {
        let path = write_wav("mono", 8000, 1, 8000, |i, _| (i % 100) as i16 * 100);
        let info = probe(&path).unwrap();
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
        assert!((info.seconds.unwrap() - 1.0).abs() < 1e-9);
        // 8000 Hz x 16 bits, plus the 44-byte header spread over a second.
        assert_eq!(info.kbps, Some(128));
        assert_eq!(info.title, None);

        let mut src = Source::open(&path).unwrap();
        let out = read_all(&mut src);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(out.len(), 8000 * 2, "mono is doubled into stereo frames");
        assert_eq!(out[2 * 7], out[2 * 7 + 1]);
        assert!((out[2 * 7] - 700.0 / 32768.0).abs() < 1e-6);
        assert!(src.take_kbps().is_some());
    }

    /// A 16-bit big-endian AIFF of `frames` frames of mono at 8 kHz; frame `i` is `i * 10`.
    fn write_aiff(name: &str, frames: u32) -> PathBuf {
        let path = std::env::temp_dir().join(format!("neoamp-{}-{name}.aiff", std::process::id()));
        let data_len = frames * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"FORM");
        b.extend_from_slice(&(4 + 26 + 16 + data_len).to_be_bytes());
        b.extend_from_slice(b"AIFFCOMM");
        b.extend_from_slice(&18u32.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&frames.to_be_bytes());
        b.extend_from_slice(&16u16.to_be_bytes());
        // 8000 as an 80-bit extended float: 1.953125 x 2^12.
        b.extend_from_slice(&(16_383u16 + 12).to_be_bytes());
        b.extend_from_slice(&(8000u64 << (63 - 12)).to_be_bytes());
        b.extend_from_slice(b"SSND");
        b.extend_from_slice(&(8 + data_len).to_be_bytes());
        b.extend_from_slice(&[0; 8]);
        for i in 0..frames {
            b.extend_from_slice(&((i * 10) as i16).to_be_bytes());
        }
        std::fs::write(&path, b).unwrap();
        path
    }

    #[test]
    fn aiff_probes_and_decodes() {
        let path = write_aiff("aiff", 4000);
        let info = probe(&path).unwrap();
        let mut src = Source::open(&path).unwrap();
        let out = read_all(&mut src);
        std::fs::remove_file(&path).unwrap();
        assert_eq!((info.sample_rate, info.channels), (8000, 1));
        assert!((info.seconds.unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(out.len(), 4000 * 2);
        assert!((out[2 * 7] - 70.0 / 32768.0).abs() < 1e-6);
    }

    /// An ID3v2.3 tag holding `frames`: (frame id, text). Comments get a language and an
    /// empty description.
    fn id3v2(frames: &[(&str, &str)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (id, text) in frames {
            let mut data = vec![0u8]; // ISO-8859-1
            if *id == "COMM" {
                data.extend_from_slice(b"eng\0");
            }
            data.extend_from_slice(text.as_bytes());
            body.extend_from_slice(id.as_bytes());
            body.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(&data);
        }
        let n = u32::try_from(body.len()).unwrap();
        let mut tag = b"ID3\x03\x00\x00".to_vec();
        // The size in four 7-bit bytes.
        tag.extend((0..4).rev().map(|i| ((n >> (7 * i)) & 0x7F) as u8));
        tag.extend_from_slice(&body);
        tag
    }

    #[test]
    fn details_read_the_tags_and_the_stream() {
        let wav = write_wav("tagged", 8000, 2, 8000, |_, _| 0);
        let mut bytes = id3v2(&[
            ("TIT2", "Rolling Llama"),
            ("TPE1", "Justin F."),
            ("TALB", "  "),
            ("TYER", "1999"),
            ("TRCK", "3/9"),
            ("TCON", "Rock"),
            ("COMM", "Radio edit"),
        ]);
        bytes.extend_from_slice(&std::fs::read(&wav).unwrap());
        std::fs::remove_file(&wav).unwrap();
        let path = std::env::temp_dir().join(format!("neoamp-{}-tagged.wav", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let d = details(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let t = &d.tags;
        assert_eq!(t.title.as_deref(), Some("Rolling Llama"));
        assert_eq!(t.artist.as_deref(), Some("Justin F."));
        assert_eq!(t.album, None, "blank tags count as missing");
        assert_eq!(t.year.as_deref(), Some("1999"));
        assert_eq!(t.track.as_deref(), Some("3/9"));
        assert_eq!(t.genre.as_deref(), Some("Rock"));
        assert_eq!(t.comment.as_deref(), Some("Radio edit"));
        assert_eq!(d.info.title.as_deref(), Some("Justin F. - Rolling Llama"));
        assert_eq!((d.info.sample_rate, d.info.channels), (8000, 2));
        assert_eq!(d.bits_per_sample, Some(16));
        assert_eq!(d.size, Some(u64::try_from(bytes.len()).unwrap()));
        assert!(d.format.contains("Waveform"), "{}", d.format);
        assert!(d.codec.contains("PCM"), "{}", d.codec);
    }

    #[test]
    fn seeking_lands_on_the_exact_frame() {
        let path = write_wav("seek", 8000, 2, 16000, |i, c| {
            if c == 0 { (i % 8000) as i16 } else { -1 }
        });
        let mut src = Source::open(&path).unwrap();
        let at = src.seek(1.25).unwrap();
        let mut out = Vec::new();
        assert_eq!(src.read(&mut out).unwrap(), Read::Audio);
        let rest = read_all(&mut src);
        std::fs::remove_file(&path).unwrap();
        assert!((at - 1.25).abs() < 1e-9);
        // Frame 10000 holds 10000 % 8000 = 2000 on the left channel.
        let first = out[0] * 32768.0;
        assert!((first - 2000.0).abs() < 0.1, "got {first}");
        assert!((out[1] + 1.0 / 32768.0).abs() < 1e-6);
        assert_eq!((out.len() + rest.len()) / 2, 6000);
    }

    /// Two Ogg Vorbis songs of a quarter second each at 8 kHz, "Chain - One" and
    /// "Chain - Two", one after the other in one stream, as Icecast sends a station's songs.
    const CHAINED: &[u8] = include_bytes!("testdata/chained.ogg");

    #[test]
    fn a_chained_stream_moves_on_to_its_next_song_and_its_tags() {
        let media = symphonia::core::io::ReadOnlySource::new(std::io::Cursor::new(CHAINED));
        let (mut src, details) = Source::open_stream(Box::new(media), &Hint::new()).unwrap();
        assert_eq!(
            details.tags.playlist_title().as_deref(),
            Some("Chain - One")
        );
        let mut out = Vec::new();
        let mut read = Read::Audio;
        while read == Read::Audio {
            read = src.read(&mut out).unwrap();
        }
        assert_eq!(read, Read::NewStream);
        let first = out.len() / 2;
        let (mut src, details) = src.next_stream().unwrap();
        assert_eq!(
            details.tags.playlist_title().as_deref(),
            Some("Chain - Two")
        );
        assert_eq!((details.info.sample_rate, details.info.channels), (8000, 1));
        while src.read(&mut out).unwrap() == Read::Audio {}
        let second = out.len() / 2 - first;
        for frames in [first, second] {
            assert!((1500..=2500).contains(&frames), "{frames} frames");
        }
    }

    #[test]
    fn surround_keeps_the_front_pair() {
        let mut out = Vec::new();
        to_stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, &mut out);
        assert_eq!(out, [1.0, 2.0, 4.0, 5.0]);
        out.clear();
        to_stereo(&[0.5], 1, &mut out);
        assert_eq!(out, [0.5, 0.5]);
    }

    #[test]
    fn missing_and_bogus_files_fail_to_open() {
        assert!(Source::open(Path::new("/nonexistent/song.mp3")).is_err());
        let path = std::env::temp_dir().join(format!("neoamp-{}-bogus.mp3", std::process::id()));
        std::fs::write(&path, b"definitely not audio").unwrap();
        let err = Source::open(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(err.is_err());
    }
}
