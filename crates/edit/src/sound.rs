//! Sound files for the 1.12 client: a WAV or MP3 file decoded to samples, and
//! samples written as a 16-bit PCM WAV.
//!
//! All 6,845 WAVs the archives ship are uncompressed 16-bit PCM: 6,665 mono at
//! 22,050 Hz, and the other 180 stereo, at 44,100 Hz, or both. All 194 MP3s
//! are MPEG-1 Layer III at 44,100 Hz, and they are the music. A file from
//! another tool can be 24-bit, floating point, at 48,000 Hz, or have an
//! extensible header. [`decode`] reads any of those with rodio, the decoder
//! `vale_client::sound::mixer` plays through, and [`wav`] writes the samples
//! as 16-bit PCM at the rate and channel count of the [`Pcm`] it is given.

use crate::EditError;

/// The two sample rates the shipped WAVs are at.
pub const RATES: [u32; 2] = [22_050, 44_100];

/// The longest sound [`decode`] reads, in seconds: 20 minutes. A longer file
/// is refused, so an hour-long recording picked by mistake does not fill
/// memory with samples.
pub const LONGEST: f32 = 20.0 * 60.0;

/// A decoded sound: its sample rate in Hz, its channel count, and its
/// samples.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    /// Interleaved by channel, each from -1 to 1.
    pub samples: Vec<f32>,
}

impl Pcm {
    /// The number of samples in each channel.
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.rate.max(1) as f32
    }

    /// The sound mixed to one channel: each output sample is the mean of one
    /// frame's channels.
    pub fn mono(&self) -> Pcm {
        let channels = usize::from(self.channels.max(1));
        let samples = self
            .samples
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect();
        Pcm { rate: self.rate, channels: 1, samples }
    }

    /// The sound resampled to `rate`. Lowering the rate averages the source
    /// samples each output sample spans, which removes most of the frequencies
    /// the lower rate cannot represent, so they do not alias. Raising the rate
    /// interpolates linearly between neighbouring samples.
    pub fn at_rate(&self, rate: u32) -> Pcm {
        let frames = self.frames();
        if rate == self.rate || frames == 0 || rate == 0 {
            return Pcm { rate: rate.max(1), ..self.clone() };
        }
        let channels = usize::from(self.channels.max(1));
        let step = f64::from(self.rate) / f64::from(rate);
        let written = ((frames as f64 / step).round() as usize).max(1);
        let at = |frame: usize, channel: usize| self.samples[frame.min(frames - 1) * channels + channel];
        let mut samples = Vec::with_capacity(written * channels);
        for out in 0..written {
            let start = out as f64 * step;
            let first = start.floor() as usize;
            for channel in 0..channels {
                let value = match step > 1.0 {
                    true => {
                        let end = ((start + step).ceil() as usize).clamp(first + 1, frames.max(first + 1));
                        (first..end).map(|frame| at(frame, channel)).sum::<f32>() / (end - first) as f32
                    }
                    false => {
                        let along = (start - first as f64) as f32;
                        let (a, b) = (at(first, channel), at(first + 1, channel));
                        a + (b - a) * along
                    }
                };
                samples.push(value);
            }
        }
        Pcm { rate, channels: self.channels, samples }
    }

    /// The largest absolute sample in each of `bins` equal spans of the sound,
    /// across every channel, clamped to 0..=1. A waveform is drawn from these.
    pub fn peaks(&self, bins: usize) -> Vec<f32> {
        let frames = self.frames();
        let channels = usize::from(self.channels.max(1));
        if bins == 0 || frames == 0 {
            return vec![0.0; bins];
        }
        (0..bins)
            .map(|bin| {
                let first = bin * frames / bins;
                let end = ((bin + 1) * frames / bins).max(first + 1).min(frames);
                self.samples[first * channels..end * channels]
                    .iter()
                    .fold(0.0f32, |loudest, sample| loudest.max(sample.abs()))
                    .min(1.0)
            })
            .collect()
    }
}

/// A sound file's encoding, read from its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A WAV: its format tag (1 is integer PCM), channels, rate and bits per
    /// sample.
    Wav { tag: u16, channels: u16, rate: u32, bits: u16 },
    /// An MP3, from its first frame's header.
    Mp3 { channels: u16, rate: u32, kbps: u32 },
}

impl Format {
    /// The format of a WAV or MP3 file's bytes, or `None` for neither.
    pub fn of(bytes: &[u8]) -> Option<Format> {
        match bytes.get(0..4) {
            Some(b"RIFF") => wav_format(bytes),
            _ => mp3_format(bytes),
        }
    }

    /// Whether it is a WAV in the encoding every shipped WAV has: 16-bit
    /// integer PCM at one of [`RATES`], in one or two channels.
    pub fn is_shipped_wav(&self) -> bool {
        matches!(*self, Format::Wav { tag: 1, channels: 1 | 2, rate, bits: 16 } if RATES.contains(&rate))
    }

    /// The format in words: `WAV, 16-bit PCM, mono, 22,050 Hz` or
    /// `MP3, 128 kbps, stereo, 44,100 Hz`.
    pub fn words(&self) -> String {
        let channels = |count: u16| match count {
            1 => "mono".to_string(),
            2 => "stereo".to_string(),
            n => format!("{n} channels"),
        };
        match *self {
            Format::Wav { tag, channels: count, rate, bits } => {
                let coding = match tag {
                    1 => format!("{bits}-bit PCM"),
                    3 => format!("{bits}-bit floating point"),
                    0xFFFE => format!("{bits}-bit, extensible header"),
                    other => format!("format {other}"),
                };
                format!("WAV, {coding}, {}, {} Hz", channels(count), thousands(rate))
            }
            Format::Mp3 { channels: count, rate, kbps } => {
                format!("MP3, {kbps} kbps, {}, {} Hz", channels(count), thousands(rate))
            }
        }
    }
}

/// A number with a comma between each three digits.
fn thousands(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A WAV's `fmt ` chunk, found by walking the chunks after the RIFF header.
fn wav_format(bytes: &[u8]) -> Option<Format> {
    if bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let word = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let long = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = long(at + 4)? as usize;
        if &bytes[at..at + 4] == b"fmt " {
            return Some(Format::Wav {
                tag: word(at + 8)?,
                channels: word(at + 10)?,
                rate: long(at + 12)?,
                bits: word(at + 22)?,
            });
        }
        // A chunk of odd size is followed by a byte of padding.
        at = at.checked_add(8 + size + (size & 1))?;
    }
    None
}

/// A WAV or MP3 file's length in seconds, read from its headers without
/// decoding. A WAV's is its `data` chunk's size over its byte rate. An MP3's
/// is the file's size over its first frame's bit rate, which is exact for a
/// constant-bit-rate file. `None` for a file that is neither, or a header
/// that gives no rate.
pub fn seconds_of(bytes: &[u8]) -> Option<f32> {
    match Format::of(bytes)? {
        Format::Wav { channels, rate, bits, .. } => {
            let per_second = rate as f32 * f32::from(channels) * f32::from(bits) / 8.0;
            let long = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
            let mut at = 12;
            while at + 8 <= bytes.len() {
                let size = long(at + 4)? as usize;
                if &bytes[at..at + 4] == b"data" {
                    // A `data` size larger than the rest of the file is cut
                    // to the file's end, which is where a decoder stops.
                    let size = size.min(bytes.len() - at - 8);
                    return (per_second > 0.0).then(|| size as f32 / per_second);
                }
                at = at.checked_add(8 + size + (size & 1))?;
            }
            None
        }
        Format::Mp3 { kbps, .. } => (kbps > 0).then(|| bytes.len() as f32 * 8.0 / (kbps as f32 * 1000.0)),
    }
}

/// An MP3's first frame header, after an ID3v2 tag if there is one.
fn mp3_format(bytes: &[u8]) -> Option<Format> {
    let mut at = 0;
    if bytes.get(0..3) == Some(b"ID3") {
        let size = bytes.get(6..10)?.iter().fold(0usize, |size, byte| (size << 7) | usize::from(byte & 0x7F));
        at = 10 + size;
    }
    // The frame sync is eleven set bits. Only the first 8,192 bytes after
    // the tag are searched, so a file that is not an MP3 is not read to its
    // end.
    let end = bytes.len().min(at + 8192);
    let start = (at..end.saturating_sub(3)).find(|&i| bytes[i] == 0xFF && bytes[i + 1] & 0xE0 == 0xE0)?;
    let header = &bytes[start..start + 4];
    let version = (header[1] >> 3) & 3;
    let layer = (header[1] >> 1) & 3;
    if layer != 1 || version == 1 {
        return None;
    }
    // Layer III bit rates, MPEG-1 and then MPEG-2 and 2.5.
    const MPEG1: [u32; 16] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
    const MPEG2: [u32; 16] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0];
    let rates: [u32; 3] = match version {
        3 => [44_100, 48_000, 32_000],
        2 => [22_050, 24_000, 16_000],
        _ => [11_025, 12_000, 8_000],
    };
    let kbps = match version {
        3 => MPEG1[usize::from(header[2] >> 4)],
        _ => MPEG2[usize::from(header[2] >> 4)],
    };
    let rate = *rates.get(usize::from((header[2] >> 2) & 3))?;
    let channels = match header[3] >> 6 {
        3 => 1,
        _ => 2,
    };
    Some(Format::Mp3 { channels, rate, kbps })
}

/// A WAV or MP3 file's samples, read with rodio, the decoder
/// `vale_client::sound::mixer` plays through. The client crate cannot play a
/// file rodio refuses, so such a file is refused here and never written. A
/// file longer than [`LONGEST`] is refused too.
pub fn decode(bytes: &[u8]) -> Result<Pcm, EditError> {
    use rodio::Source;
    let decoder = rodio::Decoder::new(std::io::Cursor::new(bytes.to_vec()))
        .map_err(|e| EditError::malformed("sound", e.to_string()))?;
    let channels = decoder.channels().get();
    let rate = decoder.sample_rate().get();
    let most = (LONGEST * rate as f32) as usize * usize::from(channels);
    let samples: Vec<f32> = decoder.take(most + 1).collect();
    if samples.len() > most {
        return Err(EditError::Refused(format!("the sound is longer than {} minutes", LONGEST / 60.0)));
    }
    if samples.is_empty() {
        return Err(EditError::malformed("sound", "it holds no samples"));
    }
    Ok(Pcm { rate, channels, samples })
}

/// The samples as a WAV file: a 44-byte header and 16-bit integer PCM.
pub fn wav(pcm: &Pcm) -> Vec<u8> {
    let channels = pcm.channels.max(1);
    let data = pcm.samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&pcm.rate.to_le_bytes());
    out.extend_from_slice(&(pcm.rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    for sample in &pcm.samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second of a tone at `rate`, in `channels` channels.
    fn tone(rate: u32, channels: u16) -> Pcm {
        let samples = (0..rate)
            .flat_map(|i| {
                let value = (i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.5;
                std::iter::repeat_n(value, usize::from(channels))
            })
            .collect();
        Pcm { rate, channels, samples }
    }

    /// A written WAV is 16-bit PCM mono at 22,050 Hz, its length is read from
    /// its header, and rodio reads back the same samples.
    #[test]
    fn a_written_wav_reads_back() {
        let pcm = tone(22_050, 1);
        let file = wav(&pcm);
        let format = Format::of(&file).expect("a format");
        assert_eq!(format, Format::Wav { tag: 1, channels: 1, rate: 22_050, bits: 16 });
        assert!(format.is_shipped_wav());
        assert_eq!(format.words(), "WAV, 16-bit PCM, mono, 22,050 Hz");
        assert_eq!(seconds_of(&file), Some(1.0));
        let back = decode(&file).expect("decoded");
        assert_eq!((back.rate, back.channels, back.frames()), (22_050, 1, 22_050));
        assert!(back.samples.iter().zip(&pcm.samples).all(|(a, b)| (a - b).abs() < 1.0 / 16_000.0));
    }

    /// Mixing stereo to mono halves the sample count, and resampling keeps
    /// the length in seconds and the peak level.
    #[test]
    fn mono_and_a_new_rate() {
        let stereo = tone(48_000, 2);
        let mono = stereo.mono();
        assert_eq!((mono.channels, mono.frames()), (1, 48_000));
        let lower = mono.at_rate(44_100);
        assert_eq!(lower.frames(), 44_100);
        assert!((lower.seconds() - 1.0).abs() < 1e-3);
        let loudest = lower.samples.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!((loudest - 0.5).abs() < 0.02, "{loudest}");
        let higher = tone(11_025, 1).at_rate(22_050);
        assert_eq!(higher.frames(), 22_050);
        let peaks = lower.peaks(10);
        assert_eq!(peaks.len(), 10);
        assert!(peaks.iter().all(|peak| (peak - 0.5).abs() < 0.02));
    }

    /// An MP3's first frame header, after an ID3 tag, gives its sample rate,
    /// bit rate and channel count. Bytes that are neither WAV nor MP3 give no
    /// format and do not decode.
    #[test]
    fn an_mp3_header_is_read() {
        let mut file = b"ID3\x03\x00\x00\x00\x00\x00\x02xx".to_vec();
        file.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x40]);
        file.extend_from_slice(&[0; 64]);
        assert_eq!(Format::of(&file), Some(Format::Mp3 { channels: 2, rate: 44_100, kbps: 128 }));
        assert_eq!(seconds_of(&file), Some(file.len() as f32 * 8.0 / 128_000.0));
        assert_eq!(Format::of(b"not a sound at all"), None);
        assert!(decode(b"not a sound at all").is_err());
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
