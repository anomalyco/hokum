//! Live levels: a peak-detect capture stream per metered node, set up the way pavucontrol's
//! (through pipewire-pulse) are. `stream.monitor` + `resample.peaks` make the adapter hand us
//! per-channel peaks at a low rate instead of audio; `node.passive` keeps a meter from waking
//! or holding up a device; no `node.latency`, so the graph's quantum is left alone.

use std::cell::Cell;
use std::rc::Rc;

use pipewire as pw;
use pw::core::CoreRc;
use pw::properties::properties;
use pw::spa;
use pw::stream::{StreamFlags, StreamListener, StreamRc};

use crate::sound::model::Id;

use super::store::{METER_KEY, cubic};

/// Peaks per second the adapter delivers; above the ~30 Hz publish rate.
const RATE: u32 = 60;
/// How much of the last level is kept per frame when nothing louder arrived.
const FALLOFF: f32 = 0.7;

pub struct Meter {
    pub node: Id,
    _listener: StreamListener<()>,
    _stream: StreamRc,
    peak: Rc<Cell<f32>>,
    level: f32,
}

impl Meter {
    /// Meters `node` (by `object.serial`); `sink` captures what a sink plays.
    pub fn new(core: &CoreRc, node: Id, serial: &str, sink: bool) -> Result<Meter, String> {
        let mut props = properties! {
            "media.type" => "Audio",
            "media.category" => "Monitor",
            "media.name" => "Peak detect",
            "application.name" => "Hokum Sound",
            "application.id" => "hokum-sound",
            "node.name" => "hokum-sound-meter",
            "node.description" => "Hokum Sound (meter)",
            "target.object" => serial,
            "stream.monitor" => "true",
            "resample.peaks" => "true",
            "node.passive" => "true",
            "node.dont-reconnect" => "true",
            "node.dont-fallback" => "true",
            "node.dont-move" => "true",
            METER_KEY => "true",
        };
        if sink {
            props.insert("stream.capture.sink", "true");
        }
        let stream = StreamRc::new(core.clone(), "hokum-sound-meter", props).map_err(|e| e.to_string())?;
        let peak = Rc::new(Cell::new(0.0f32));
        let p = peak.clone();
        let listener = stream
            .add_local_listener_with_user_data(())
            .process(move |stream, _| {
                let Some(mut buffer) = stream.dequeue_buffer() else { return };
                let Some(data) = buffer.datas_mut().first_mut() else { return };
                let chunk = data.chunk().as_raw();
                let (offset, size) = (chunk.offset as usize, chunk.size as usize);
                let Some(bytes) = data.data() else { return };
                let end = (offset + size).min(bytes.len());
                let max = bytes.get(offset..end).unwrap_or_default().as_chunks::<4>().0.iter().map(|s| f32::from_le_bytes(*s).abs()).fold(0.0f32, f32::max);
                if max > p.get() {
                    p.set(max);
                }
            })
            .register()
            .map_err(|e| e.to_string())?;

        let mut info = spa::param::audio::AudioInfoRaw::new();
        info.set_format(spa::param::audio::AudioFormat::F32LE);
        info.set_rate(RATE);
        let obj = spa::pod::Object { type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(), id: spa::param::ParamType::EnumFormat.as_raw(), properties: info.into() };
        let bytes = super::params::serialize(&spa::pod::Value::Object(obj));
        let mut params = [spa::pod::Pod::from_bytes(&bytes).ok_or("bad format pod")?];
        stream.connect(spa::utils::Direction::Input, None, StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS, &mut params).map_err(|e| e.to_string())?;
        Ok(Meter { node, _listener: listener, _stream: stream, peak, level: 0.0 })
    }

    /// The level to show now, cubic: the loudest peak since the last call, falling off
    /// smoothly when it's quieter or nothing arrived.
    pub fn take_level(&mut self) -> f32 {
        let peak = cubic(self.peak.replace(0.0).min(1.0));
        self.level = peak.max(self.level * FALLOFF);
        if self.level < 0.001 {
            self.level = 0.0;
        }
        self.level
    }
}
