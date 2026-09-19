//! Per-sample signal evaluator. A Program is a flat list of ops; every op writes one f32.
//! `Row(r)` reads another node's value: current sample if it was computed earlier this
//! sample, previous sample otherwise (that's how feedback loops work).
use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)] pub enum Wave { Sin, Saw, Sqr, Tri, Pulse, Phase }
#[derive(Clone, Copy, Debug, PartialEq)] pub enum Filt { Lp, Hp, Bp }
#[derive(Clone, Copy, Debug, PartialEq)] pub enum Bin { Add, Sub, Mul, Div, Min, Max, Pow, Gt, Lt }
#[derive(Clone, Copy, Debug, PartialEq)] pub enum Un { Tanh, Clip, Abs, Sqrt, Neg }

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Param(usize), Freq, Gate, Vel, Note, Row(usize),
    Bin(Bin, usize, usize), Un(Un, usize),
    Osc(Wave, usize, usize, usize), Noise(usize), // ..., amp
    Env(usize, usize, usize, usize, usize),
    Filt(Filt, usize, usize, usize),
    Delay(usize, usize),
    Sh(usize, usize),
    /// in, write pos (0..1), read pos (0..1), record gate, size in seconds
    Buf(usize, usize, usize, usize, usize),
    /// gate, note, key: passes gate only while note == key
    Key(usize, usize, usize),
}

/// `ops` define structure (changing them resets voices); `params` are the constant
/// knob values and can change freely while playing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub ops: Vec<Op>,
    pub params: Vec<f32>,
    pub row_slot: Vec<usize>,  // row (node in eval order) -> op index holding its value
    pub row_node: Vec<usize>,  // row -> graph node index (for meters)
    pub out: Option<usize>,
}

#[derive(Clone, Debug)]
enum St { None, Phase(f32), Env { stage: u8, level: f32, gate: bool }, Svf(f32, f32), Delay(Vec<f32>, usize), Sh(f32, bool), Buf(Vec<f32>) }

pub struct Voice {
    vals: Vec<f32>,
    st: Vec<St>,
    pub freq: f32,
    pub gate: f32,
    pub vel: f32,
    pub pitch: f32,
    pub off_at: f64,
    pub born: u64,
    quiet: u32,
    rng: u32,
}

fn blep(t: f32, dt: f32) -> f32 {
    if t < dt { let t = t / dt; 2.0 * t - t * t - 1.0 }
    else if t > 1.0 - dt { let t = (t - 1.0) / dt; t * t + 2.0 * t + 1.0 }
    else { 0.0 }
}
fn square(t: f32, pw: f32, dt: f32) -> f32 {
    let v = if t < pw { 1.0 } else { -1.0 };
    v + blep(t, dt) - blep((t + 1.0 - pw).rem_euclid(1.0), dt)
}

impl Voice {
    pub fn new(p: &Program, sr: f32) -> Voice {
        let st = p.ops.iter().map(|o| match o {
            Op::Osc(..) => St::Phase(0.0),
            Op::Env(..) => St::Env { stage: 2, level: 0.0, gate: false },
            Op::Filt(..) => St::Svf(0.0, 0.0),
            Op::Delay(..) => St::Delay(vec![0.0; (sr * 2.0) as usize], 0),
            Op::Sh(..) => St::Sh(0.0, false),
            Op::Buf(..) => St::Buf(vec![0.0; (sr * 4.0) as usize]),
            _ => St::None,
        }).collect();
        Voice { vals: vec![0.0; p.ops.len()], st, freq: 440.0, gate: 0.0, vel: 1.0, pitch: 69.0, off_at: 0.0, born: 0, quiet: 1 << 30, rng: 0x9E37_79B9 }
    }
    pub fn idle(&self) -> bool { self.gate <= 0.0 && self.quiet > 4800 }
    pub fn row_val(&self, p: &Program, row: usize) -> f32 { self.vals[p.row_slot[row]] }
    /// make envelopes see a fresh rising edge (used when stealing a voice)
    pub fn retrigger(&mut self) {
        for s in &mut self.st { if let St::Env { gate, .. } = s { *gate = false } }
    }

    pub fn tick(&mut self, p: &Program, sr: f32) -> f32 {
        for i in 0..p.ops.len() {
            let v = match &p.ops[i] {
                Op::Param(k) => p.params[*k],
                Op::Freq => self.freq,
                Op::Gate => self.gate,
                Op::Vel => self.vel,
                Op::Note => self.pitch,
                Op::Row(r) => self.vals[p.row_slot[*r]],
                Op::Bin(b, x, y) => {
                    let (x, y) = (self.vals[*x], self.vals[*y]);
                    match b {
                        Bin::Add => x + y, Bin::Sub => x - y, Bin::Mul => x * y,
                        Bin::Div => if y.abs() < 1e-9 { 0.0 } else { x / y },
                        Bin::Min => x.min(y), Bin::Max => x.max(y), Bin::Pow => x.powf(y),
                        Bin::Gt => (x > y) as u8 as f32, Bin::Lt => (x < y) as u8 as f32,
                    }
                }
                Op::Un(u, x) => {
                    let x = self.vals[*x];
                    match u { Un::Neg => -x, Un::Tanh => x.tanh(), Un::Clip => x.clamp(-1.0, 1.0), Un::Abs => x.abs(), Un::Sqrt => x.abs().sqrt() }
                }
                Op::Osc(w, f, pw, amp) => {
                    let dt = (self.vals[*f] / sr).clamp(-0.5, 0.5);
                    let pw = self.vals[*pw].clamp(0.01, 0.99);
                    let amp = self.vals[*amp];
                    let St::Phase(ph) = &mut self.st[i] else { unreachable!() };
                    let t = *ph;
                    *ph = (t + dt).rem_euclid(1.0);
                    let dt = dt.abs().max(1e-6);
                    amp * match w {
                        Wave::Sin => (t * 2.0 * PI).sin(),
                        Wave::Saw => 2.0 * t - 1.0 - blep(t, dt),
                        Wave::Sqr => square(t, 0.5, dt),
                        Wave::Pulse => square(t, pw, dt),
                        Wave::Tri => 4.0 * (t - 0.5).abs() - 1.0,
                        Wave::Phase => t,
                    }
                }
                Op::Noise(amp) => {
                    self.rng ^= self.rng << 13; self.rng ^= self.rng >> 17; self.rng ^= self.rng << 5;
                    self.vals[*amp] * ((self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0)
                }
                Op::Env(g, a, d, s, r) => {
                    let g = self.vals[*g] > 0.5;
                    let (a, d, s, r) = (self.vals[*a].max(1e-4), self.vals[*d].max(1e-4), self.vals[*s].clamp(0.0, 1.0), self.vals[*r].max(1e-4));
                    let St::Env { stage, level, gate } = &mut self.st[i] else { unreachable!() };
                    if g && !*gate { *stage = 0 }
                    if !g && *gate { *stage = 2 }
                    *gate = g;
                    match *stage {
                        0 => { *level += 1.0 / (a * sr); if *level >= 1.0 { *level = 1.0; *stage = 1 } }
                        1 => { *level += (s - *level) / (d * sr); }
                        _ => { *level -= *level / (r * sr); if *level < 1e-5 { *level = 0.0 } }
                    }
                    *level
                }
                Op::Filt(m, x, c, q) => {
                    let x = self.vals[*x];
                    let g = (PI * self.vals[*c].clamp(10.0, sr * 0.45) / sr).tan();
                    let k = 1.0 / self.vals[*q].clamp(0.1, 50.0);
                    let St::Svf(ic1, ic2) = &mut self.st[i] else { unreachable!() };
                    let a1 = 1.0 / (1.0 + g * (g + k));
                    let (a2, a3) = (g * a1, g * g * a1);
                    let v3 = x - *ic2;
                    let v1 = a1 * *ic1 + a2 * v3;
                    let v2 = *ic2 + a2 * *ic1 + a3 * v3;
                    *ic1 = 2.0 * v1 - *ic1;
                    *ic2 = 2.0 * v2 - *ic2;
                    match m { Filt::Lp => v2, Filt::Bp => v1, Filt::Hp => x - k * v1 - v2 }
                }
                Op::Delay(x, t) => {
                    let x = self.vals[*x];
                    let d = self.vals[*t] * sr;
                    let St::Delay(buf, pos) = &mut self.st[i] else { unreachable!() };
                    let n = buf.len();
                    let d = d.clamp(1.0, (n - 2) as f32);
                    let rp = (*pos as f32 - d).rem_euclid(n as f32);
                    let i0 = rp as usize;
                    let fr = rp - i0 as f32;
                    let out = buf[i0 % n] * (1.0 - fr) + buf[(i0 + 1) % n] * fr;
                    buf[*pos] = x;
                    *pos = (*pos + 1) % n;
                    out
                }
                Op::Sh(x, t) => {
                    let (x, t) = (self.vals[*x], self.vals[*t] > 0.5);
                    let St::Sh(held, prev) = &mut self.st[i] else { unreachable!() };
                    if t && !*prev { *held = x }
                    *prev = t;
                    *held
                }
                Op::Buf(x, w, r, rec, size) => {
                    let (x, w, r, rec, size) = (self.vals[*x], self.vals[*w], self.vals[*r], self.vals[*rec] > 0.5, self.vals[*size]);
                    let St::Buf(buf) = &mut self.st[i] else { unreachable!() };
                    let n = ((size * sr) as usize).clamp(2, buf.len());
                    if rec { buf[(w.rem_euclid(1.0) * n as f32) as usize % n] = x }
                    let rp = r.rem_euclid(1.0) * n as f32;
                    let i0 = rp as usize % n;
                    let fr = rp - rp.floor();
                    buf[i0] * (1.0 - fr) + buf[(i0 + 1) % n] * fr
                }
                Op::Key(g, n, k) => if (self.vals[*n] - self.vals[*k]).abs() < 0.5 { self.vals[*g] } else { 0.0 },
            };
            self.vals[i] = if v.is_finite() { v } else { 0.0 };
        }
        let out = p.out.map_or(0.0, |o| self.vals[o]);
        if out.abs() < 1e-4 { self.quiet = self.quiet.saturating_add(1) } else { self.quiet = 0 }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demo_tracks_make_sound() {
        let song = crate::song::Song::demo();
        let ids: Vec<u32> = song.tracks.iter().map(|t| t.id).collect();
        let progs = song.graph.compile_all(&ids);
        for (t, p) in progs.iter().enumerate() {
            assert!(p.out.is_some(), "{} has no out", song.tracks[t].name);
            let mut v = Voice::new(p, 48000.0);
            v.freq = 261.6; v.gate = 1.0; v.vel = 1.0; v.pitch = 60.0;
            let mut peak = 0f32;
            for _ in 0..4800 { peak = peak.max(v.tick(p, 48000.0).abs()) }
            assert!(peak > 0.01 && peak.is_finite(), "{}: peak {peak}", song.tracks[t].name);
            v.gate = 0.0;
            for _ in 0..(48000 * 6) { v.tick(p, 48000.0); }
            assert!(v.idle(), "{} never went idle", song.tracks[t].name);
        }
    }
}
