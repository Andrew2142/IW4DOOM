//! Aggregator for the audio backend submodules (sfx + music).
//!
//! The Rust audio backend replaces the SDL2 mixer that the original
//! `vendor/doomgeneric/i_sound.c` and `i_oplmusic.c` drive.  It is built on
//! top of `rodio`: each Doom logical channel becomes a rodio `Player`
//! connected to a single shared output device, and music decoding lives in
//! the `music` submodule.
//!
//! State is kept in a `thread_local!` `AUDIO` cell.  Doom's audio API is
//! synchronous and only ever called from the main thread, so a thread-local
//! is both safe and the simplest place to hold the `MixerDeviceSink` without
//! introducing any global lock.

pub(crate) mod music;
pub(crate) mod sfx;

use std::cell::RefCell;
use std::sync::Arc;

use rodio::buffer::SamplesBuffer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player, Source};

pub(crate) use sfx::{decode_doom_sfx, PannedSource};
use sfx::{ArcSamples, PanState, SampleCtl, StereoGain};

thread_local! {
    /// Thread-local home for the singleton [`AudioState`].
    ///
    /// `None` until [`AudioState::new`] succeeds, after which callers in
    /// `doom::i_sound` and `doom::s_sound` borrow it mutably to start/stop
    /// sounds.  Keeping it thread-local avoids needing a `Mutex` because Doom
    /// only ever drives audio from the main loop thread.
    pub(crate) static AUDIO: RefCell<Option<AudioState>> = const { RefCell::new(None) };
}

/// Singleton audio backend state held in the [`AUDIO`] thread-local.
///
/// Owns the output device sink, the shared mixer that all players feed into,
/// the eight Doom SFX channels, and the music state.
pub(crate) struct AudioState {
    /// The device-bound sink.  Held only to keep the audio device open for
    /// the lifetime of the engine; the mixer below is what we actually push
    /// samples through.
    pub(crate) _device_sink: MixerDeviceSink,
    /// Cloned handle to the device sink's mixer.  Cheap to clone (it is an
    /// `Arc` internally) and shared across all per-channel `Player`s and the
    /// music player.
    pub(crate) mixer: rodio::mixer::Mixer,
    /// Per-channel state for Doom's eight logical SFX channels.  `Box`ed so
    /// the `AudioState` itself stays small and movable.
    channels: Box<[sfx::ChannelState; 8]>,
    /// Music decoder/player state (MUS conversion, MIDI sequencing, volume).
    pub(crate) music: music::MusicState,
    /// DOOMMW2's own channel pool (sheet 12 section 5.6), separate from Doom's eight so MW2
    /// gunfire never cuts monster sounds. Sized on first use from `[audio] channels`.
    dmw_channels: Vec<DmwChannel>,
    /// Start counter used to find the oldest DOOMMW2 channel.
    dmw_seq: u64,
}

/// One DOOMMW2 mixer channel.
struct DmwChannel {
    /// The live player, `None` when idle.
    player: Option<Player>,
    /// Start order (larger = newer).
    seq: u64,
    /// Alias of the sound on the channel; a replay of the same alias shortens it.
    alias: String,
    /// Playback window of the live source.
    ctl: Option<Arc<SampleCtl>>,
    /// Interleaved samples per 20 ms of the live source.
    cut_samples: usize,
}

impl DmwChannel {
    fn idle() -> Self {
        Self {
            player: None,
            seq: 0,
            alias: String::new(),
            ctl: None,
            cut_samples: 0,
        }
    }

    fn busy(&self) -> bool {
        self.player.as_ref().is_some_and(|p| !p.empty())
    }
}

/// Milliseconds of a replaced weapon sound that keep playing (iw4L `AUDIO.md`).
const REPLACE_KEEP_MS: u64 = 20;

/// Construction and channel-control entry points used by the engine's
/// `S_sound` shims.
impl AudioState {
    /// Open the default audio device and build a fresh [`AudioState`].
    ///
    /// Returns an error if the platform refuses to open a default output
    /// device (e.g. no audio hardware, audio daemon not running).  On success
    /// the eight channel slots are empty (`player == None`) and music
    /// playback is idle.
    pub(crate) fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let device_sink = DeviceSinkBuilder::open_default_sink()?;
        let mixer = device_sink.mixer().clone();
        let channels = Box::new(std::array::from_fn(|_| sfx::ChannelState::new()));
        Ok(Self {
            _device_sink: device_sink,
            mixer,
            channels,
            music: music::MusicState::new(),
            dmw_channels: Vec::new(),
            dmw_seq: 0,
        })
    }

    /// Start playing a Doom DMX SFX blob on the given logical channel.
    ///
    /// `data` is the raw `DSxxxxx` lump bytes; `vol` is the Doom 0-127 volume
    /// scale; `sep` is the 0-255 stereo separation (`128` is centred);
    /// `channel` must be in `0..8`.
    ///
    /// Returns `true` on success, `false` if the channel index is out of
    /// range or if the SFX lump fails to decode.  Any previously playing
    /// sound on the same channel is replaced (its `Player` is dropped, which
    /// stops playback immediately).
    pub(crate) fn start_sound(&mut self, data: &[u8], vol: i32, sep: i32, channel: usize) -> bool {
        if channel >= 8 {
            return false;
        }
        let Some((sample_rate, samples)) = decode_doom_sfx(data) else {
            return false;
        };
        let pan = Arc::clone(&self.channels[channel].pan);
        pan.update(vol, sep);
        let buf = SamplesBuffer::new(
            std::num::NonZero::new(1u16).unwrap(),
            std::num::NonZero::new(sample_rate).unwrap(),
            samples,
        );
        let source = PannedSource::new(buf, pan);
        let player = Player::connect_new(&self.mixer);
        player.append(source);
        self.channels[channel].player = Some(player);
        true
    }

    /// Immediately stop whatever is playing on the given channel.
    ///
    /// Out-of-range `channel` values are silently ignored, matching the
    /// permissive behaviour of the SDL backend.
    pub(crate) fn stop_sound(&mut self, channel: usize) {
        if channel >= 8 {
            return;
        }
        self.channels[channel].player = None;
    }

    /// Update the volume and stereo separation of an in-flight sound without
    /// restarting it.
    ///
    /// Doom calls this every tic while a sound is active so the apparent
    /// position tracks the emitting `mobj`.  Out-of-range `channel` values
    /// are silently ignored.
    pub(crate) fn update_sound_params(&self, channel: usize, vol: i32, sep: i32) {
        if channel >= 8 {
            return;
        }
        self.channels[channel].pan.update(vol, sep);
    }

    /// Return `true` if the channel still has unconsumed samples.
    ///
    /// Out-of-range `channel` values report `false`.
    pub(crate) fn is_playing(&self, channel: usize) -> bool {
        if channel >= 8 {
            return false;
        }
        self.channels[channel]
            .player
            .as_ref()
            .is_some_and(|p| !p.empty())
    }

    /// Plays a DOOMMW2 sound on the MW2 channel pool (sheet 12 section 5.6).
    ///
    /// `vol` is Doom's 0-127 scale, `sep` the 0-255 separation (128 centred), `speed` the
    /// playback rate multiplier, `pool` the pool size (`[audio] channels`). A free channel is
    /// used, else the oldest is stolen. Playing `alias` again while it still plays on another
    /// channel lets the old one run for 20 ms more. Returns `false` for sounds with more than
    /// two channels.
    pub(crate) fn start_dmw_sound(
        &mut self,
        asset: Arc<dmw_format::SoundAsset>,
        alias: &str,
        vol: i32,
        sep: i32,
        speed: f32,
        pool: usize,
    ) -> bool {
        let pool = pool.max(1);
        while self.dmw_channels.len() < pool {
            self.dmw_channels.push(DmwChannel::idle());
        }
        self.dmw_channels.truncate(pool);

        let (channels, rate) = (usize::from(asset.channels), asset.rate);
        if !(1..=2).contains(&channels) {
            return false;
        }
        // The previous sound of this alias keeps its first 20 ms.
        for c in self
            .dmw_channels
            .iter()
            .filter(|c| c.busy() && c.alias == alias)
        {
            if let Some(ctl) = &c.ctl {
                ctl.cut_after(c.cut_samples);
            }
        }
        let slot = self
            .dmw_channels
            .iter()
            .position(|c| !c.busy())
            .or_else(|| {
                self.dmw_channels
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, c)| c.seq)
                    .map(|(i, _)| i)
            })
            .unwrap_or(0);

        let ctl = Arc::new(SampleCtl::new());
        let pan = Arc::new(PanState::new(vol, sep));
        let source = ArcSamples::new(Arc::clone(&asset), Arc::clone(&ctl)).speed(speed);
        let player = Player::connect_new(&self.mixer);
        if channels == 1 {
            player.append(PannedSource::new(source, pan));
        } else {
            player.append(StereoGain::new(source, pan));
        }
        self.dmw_seq += 1;
        let ch = &mut self.dmw_channels[slot];
        ch.player = Some(player);
        ch.seq = self.dmw_seq;
        ch.alias = alias.to_owned();
        ch.ctl = Some(ctl);
        ch.cut_samples = (u64::from(rate) * REPLACE_KEEP_MS / 1000) as usize * channels;
        true
    }
}
