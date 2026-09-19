use crate::dsp::{Program, Voice};
use crate::song::Note;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc::Receiver, Arc, Mutex};

pub struct TrackPlay { pub mute: bool, pub notes: Vec<Note> }
pub struct PlayData { pub bpm: f64, pub len: f64, pub tracks: Vec<TrackPlay> }

pub enum Msg { Set(Arc<PlayData>, Arc<Vec<Program>>), Play(bool), Seek(f64) }

/// pos: playhead in beats (f64 bits). meters: (per graph node, per track) peak |value| since last read.
#[derive(Default)]
pub struct Shared { pub pos: AtomicU64, pub meters: Mutex<(Vec<f32>, Vec<f32>)> }

const POLY: usize = 8;

struct Engine {
    data: Arc<PlayData>,
    progs: Arc<Vec<Program>>,
    voices: Vec<Vec<Voice>>,
    lead: Vec<Option<usize>>,
    node_peaks: Vec<f32>,
    track_peaks: Vec<f32>,
    pos: f64,
    playing: bool,
    sr: f32,
    counter: u64,
    rx: Receiver<Msg>,
    shared: Arc<Shared>,
}

pub fn start(rx: Receiver<Msg>, shared: Arc<Shared>) -> cpal::Stream {
    let dev = cpal::default_host().default_output_device().expect("no audio output device");
    let cfg = dev.default_output_config().expect("no output config");
    let sr = cfg.sample_rate().0 as f32;
    let ch = cfg.channels() as usize;
    let mut e = Engine {
        data: Arc::new(PlayData { bpm: 120.0, len: 4.0, tracks: vec![] }), progs: Arc::new(vec![]), voices: vec![], lead: vec![], node_peaks: vec![], track_peaks: vec![],
        pos: 0.0, playing: false, sr, counter: 0, rx, shared,
    };
    let stream = dev
        .build_output_stream(&cfg.into(), move |out: &mut [f32], _| e.process(out, ch), |err| eprintln!("audio error: {err}"), None)
        .expect("failed to build audio stream");
    stream.play().expect("failed to start audio");
    stream
}

impl Engine {
    fn all_off(&mut self) { for vs in &mut self.voices { for v in vs { v.gate = 0.0 } } }

    fn handle_msgs(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Set(d, p) => {
                    let mut voices = std::mem::take(&mut self.voices);
                    voices.resize_with(p.len(), Vec::new);
                    self.lead.resize(p.len(), None);
                    self.track_peaks.resize(p.len(), 0.0);
                    let nn = p.iter().flat_map(|q| q.row_node.iter().copied()).max().map_or(0, |m| m + 1);
                    self.node_peaks.resize(nn, 0.0);
                    for t in 0..p.len() {
                        if self.progs.get(t).map(|q| &q.ops) != Some(&p[t].ops) || voices[t].len() != POLY {
                            voices[t] = (0..POLY).map(|_| Voice::new(&p[t], self.sr)).collect();
                            self.lead[t] = None;
                        }
                    }
                    self.voices = voices;
                    self.data = d;
                    self.progs = p;
                }
                Msg::Play(b) => { self.playing = b; if !b { self.all_off() } }
                Msg::Seek(x) => { self.pos = x; self.all_off() }
            }
        }
    }

    fn process(&mut self, out: &mut [f32], ch: usize) {
        self.handle_msgs();
        let frames = out.len() / ch;
        let mut f = 0;
        while f < frames {
            let n = (frames - f).min(32);
            self.advance(n);
            for k in 0..n {
                let s = self.sample();
                for c in 0..ch { out[(f + k) * ch + c] = s }
            }
            f += n;
        }
        self.shared.pos.store(self.pos.to_bits(), Ordering::Relaxed);
        if let Ok(mut m) = self.shared.meters.try_lock() {
            if m.0.len() != self.node_peaks.len() { m.0 = self.node_peaks.clone() } else { for (a, b) in m.0.iter_mut().zip(&self.node_peaks) { *a = a.max(*b) } }
            if m.1.len() != self.track_peaks.len() { m.1 = self.track_peaks.clone() } else { for (a, b) in m.1.iter_mut().zip(&self.track_peaks) { *a = a.max(*b) } }
            self.node_peaks.fill(0.0);
            self.track_peaks.fill(0.0);
        }
    }

    fn advance(&mut self, n: usize) {
        if !self.playing { return }
        let len = self.data.len.max(0.25);
        let dt = n as f64 * self.data.bpm / 60.0 / self.sr as f64;
        let (a, b) = (self.pos, self.pos + dt);
        if b < len { self.window(a, b); self.pos = b; }
        else if a >= len { self.pos = 0.0; }
        else {
            self.window(a, len);
            for vs in &mut self.voices { for v in vs { v.off_at -= len } }
            let b = (b - len).rem_euclid(len);
            self.window(0.0, b);
            self.pos = b;
        }
    }

    fn window(&mut self, a: f64, b: f64) {
        let data = self.data.clone();
        for (t, tr) in data.tracks.iter().enumerate() {
            if t >= self.voices.len() { break }
            for v in &mut self.voices[t] {
                if v.gate > 0.0 && v.off_at >= a && v.off_at < b { v.gate = 0.0 }
            }
            if tr.mute { continue }
            for n in &tr.notes {
                if n.start >= a && n.start < b { self.note_on(t, n.pitch, n.vel, n.start + n.len) }
            }
        }
    }

    fn note_on(&mut self, t: usize, pitch: i32, vel: f32, off_at: f64) {
        if self.voices[t].is_empty() { return }
        let vs = &mut self.voices[t];
        let idx = vs.iter().position(|v| v.idle())
            .or_else(|| vs.iter().position(|v| v.gate <= 0.0))
            .unwrap_or_else(|| (0..vs.len()).min_by_key(|&i| vs[i].born).unwrap_or(0));
        let v = &mut vs[idx];
        if v.gate > 0.0 { v.retrigger() }
        v.freq = 440.0 * 2f32.powf((pitch as f32 - 69.0) / 12.0);
        v.gate = 1.0;
        v.vel = vel;
        v.pitch = pitch as f32;
        v.off_at = off_at;
        v.born = self.counter;
        self.counter += 1;
        self.lead[t] = Some(idx);
    }

    fn sample(&mut self) -> f32 {
        let Engine { voices, progs, lead, node_peaks, track_peaks, sr, .. } = self;
        let mut mix = 0.0;
        for (t, vs) in voices.iter_mut().enumerate() {
            let p = &progs[t];
            let mut tm = 0.0;
            for (vi, v) in vs.iter_mut().enumerate() {
                if v.idle() { continue }
                tm += v.tick(p, *sr);
                if lead[t] == Some(vi) {
                    for r in 0..p.row_slot.len() {
                        let a = v.row_val(p, r).abs();
                        let node = p.row_node[r];
                        if a > node_peaks[node] { node_peaks[node] = a }
                    }
                }
            }
            if tm.abs() > track_peaks[t] { track_peaks[t] = tm.abs() }
            mix += tm;
        }
        (mix * 0.7).tanh()
    }
}
