// SPDX-License-Identifier: GPL-3.0-or-later
//! Sounds: decoded PCM and the WAV helpers (sheet 03 section 5.7).

use std::io::Cursor;
use std::path::Path;

use crate::error::PackError;

/// A decoded sound.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundAsset {
    /// Sample rate in Hz.
    pub rate: u32,
    /// 1 or 2.
    pub channels: u16,
    /// Interleaved samples in -1..1.
    pub samples: Vec<f32>,
}

impl SoundAsset {
    /// Number of sample frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    /// Duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        if self.rate == 0 {
            0.0
        } else {
            self.frames() as f32 / self.rate as f32
        }
    }
}

/// Converts a 16-bit sample to `f32` in -1..1 (the same mapping `load_wav` uses).
pub fn i16_to_f32(s: i16) -> f32 {
    f32::from(s) / 32768.0
}

/// Reads a WAV file. Accepts 8/16/24/32-bit integer and 32-bit float, mono or stereo.
pub fn load_wav(path: &Path) -> Result<SoundAsset, PackError> {
    let bytes =
        std::fs::read(path).map_err(|e| PackError::from_io(path, &path.to_string_lossy(), e))?;
    decode_wav(&bytes, &path.display().to_string())
}

/// Decodes WAV bytes; `label` names the source in errors.
pub fn decode_wav(bytes: &[u8], label: &str) -> Result<SoundAsset, PackError> {
    let reader = hound::WavReader::new(Cursor::new(bytes))
        .map_err(|e| PackError::format(format!("{label}: not a decodable WAV: {e}")))?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.channels > 2 {
        return Err(PackError::invalid(format!(
            "{label}: {} channels, only mono and stereo are supported",
            spec.channels
        )));
    }
    if spec.sample_rate == 0 {
        return Err(PackError::invalid(format!("{label}: sample rate is 0")));
    }
    let bad = |e: hound::Error| PackError::format(format!("{label}: {e}"));
    let samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => reader
            .into_samples::<f32>()
            .map(|s| s.map(|v| v.clamp(-1.0, 1.0)))
            .collect::<Result<_, _>>()
            .map_err(bad)?,
        (hound::SampleFormat::Int, bits @ (8 | 16 | 24 | 32)) => {
            let scale = 1.0 / (1u64 << (bits - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(bad)?
        }
        (format, bits) => {
            return Err(PackError::invalid(format!(
                "{label}: unsupported WAV sample format {format:?}/{bits} bits"
            )));
        }
    };
    if !samples.len().is_multiple_of(usize::from(spec.channels)) {
        return Err(PackError::invalid(format!(
            "{label}: sample count is not a multiple of the channel count"
        )));
    }
    Ok(SoundAsset {
        rate: spec.sample_rate,
        channels: spec.channels,
        samples,
    })
}

/// Encodes 16-bit PCM as a WAV file (interleaved for stereo).
pub fn encode_wav_i16(rate: u32, channels: u16, pcm: &[i16]) -> Result<Vec<u8>, PackError> {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)
            .map_err(|e| PackError::format(format!("wav encode: {e}")))?;
        for &s in pcm {
            writer
                .write_sample(s)
                .map_err(|e| PackError::format(format!("wav encode: {e}")))?;
        }
        writer
            .finalize()
            .map_err(|e| PackError::format(format!("wav encode: {e}")))?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;

    fn wav(
        spec: hound::WavSpec,
        write: impl FnOnce(&mut hound::WavWriter<&mut Cursor<Vec<u8>>>),
    ) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut cursor, spec).unwrap();
            write(&mut w);
            w.finalize().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn sixteen_bit_mono_round_trip() {
        let pcm = [0i16, 1, -1, i16::MAX, i16::MIN, 12345];
        let bytes = encode_wav_i16(22050, 1, &pcm).unwrap();
        let a = decode_wav(&bytes, "t").unwrap();
        assert_eq!((a.rate, a.channels), (22050, 1));
        assert_eq!(
            a.samples,
            pcm.iter().map(|&s| i16_to_f32(s)).collect::<Vec<_>>()
        );
        assert_eq!(a.samples[4], -1.0);
        assert_eq!(a.frames(), 6);
    }

    #[test]
    fn stereo_is_interleaved() {
        let bytes = encode_wav_i16(44100, 2, &[100, -100, 200, -200]).unwrap();
        let a = decode_wav(&bytes, "t").unwrap();
        assert_eq!(a.channels, 2);
        assert_eq!(a.frames(), 2);
        assert!(a.samples[0] > 0.0 && a.samples[1] < 0.0);
    }

    #[test]
    fn eight_bit_and_float() {
        let spec8 = hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 8,
            sample_format: hound::SampleFormat::Int,
        };
        let b = wav(spec8, |w| {
            for s in [-128i8, 0, 127] {
                w.write_sample(s).unwrap();
            }
        });
        let a = decode_wav(&b, "t").unwrap();
        assert_eq!(a.samples, vec![-1.0, 0.0, 127.0 / 128.0]);

        let specf = hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let b = wav(specf, |w| {
            for s in [0.5f32, -0.25, 2.0] {
                w.write_sample(s).unwrap();
            }
        });
        // Out of range float samples are clamped.
        assert_eq!(decode_wav(&b, "t").unwrap().samples, vec![0.5, -0.25, 1.0]);
    }

    #[test]
    fn too_many_channels_and_garbage_are_rejected() {
        let spec = hound::WavSpec {
            channels: 3,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let b = wav(spec, |w| {
            for s in [1i16, 2, 3] {
                w.write_sample(s).unwrap();
            }
        });
        assert!(matches!(decode_wav(&b, "t"), Err(PackError::Invalid(_))));
        assert!(matches!(
            decode_wav(b"RIFFxxxx", "t"),
            Err(PackError::Format(_))
        ));
    }
}
