use crate::graph::{Graph, Input, Kind};
use serde::{Deserialize, Serialize};

pub const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Note {
    pub start: f64, // beats, relative to the pattern
    pub len: f64,   // beats
    pub pitch: i32, // midi
    pub vel: f32,
}

/// A block of notes placed at `start` (grid steps), `len` steps long, played `reps` times back to back.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Pattern { pub start: usize, pub len: usize, pub reps: usize, pub notes: Vec<Note> }

#[derive(Clone, Serialize, Deserialize)]
pub struct Track { pub id: u32, pub name: String, pub patterns: Vec<Pattern>, pub mute: bool }

fn d4() -> u32 { 4 }
fn dswing() -> f64 { 0.5 }

#[derive(Clone, Serialize, Deserialize)]
pub struct Song {
    pub bpm: f64,
    pub bars: usize,
    #[serde(default = "d4")]
    pub beats: u32, // beats per bar
    #[serde(default = "d4")]
    pub grid: u32, // steps per beat
    #[serde(default = "dswing")]
    pub swing: f64, // 0.5 = straight, 0.67 = triplet swing on off-beat 8ths
    pub root: i32,
    pub minor: bool,
    pub prog: Vec<i32>,
    pub next_track: u32,
    pub graph: Graph,
    pub tracks: Vec<Track>,
}

pub fn note_name(p: i32) -> String {
    format!("{}{}", NAMES[p.rem_euclid(12) as usize], p.div_euclid(12) - 1)
}

pub fn parse_root(s: &str) -> Option<i32> {
    let mut cs = s.chars();
    let base = match cs.next()?.to_ascii_uppercase() {
        'C' => 0, 'D' => 2, 'E' => 4, 'F' => 5, 'G' => 7, 'A' => 9, 'B' => 11, _ => return None,
    };
    let acc: i32 = match cs.next() { Some('#') => 1, Some('b') => -1, _ => 0 };
    Some((base + acc).rem_euclid(12))
}

impl Note {
    pub fn step(&self, sb: f64) -> usize { (self.start / sb).round().max(0.0) as usize }
}

impl Pattern {
    pub fn end(&self) -> usize { self.start + self.len * self.reps.max(1) }
    /// which repetition (0 = the editable original) covers this step
    pub fn rep_at(&self, step: usize) -> Option<usize> {
        if self.len == 0 || step < self.start || step >= self.end() { None } else { Some((step - self.start) / self.len) }
    }
}

/// graph builder used for the demo
struct B { g: Graph }
impl B {
    fn n(&mut self, kind: Kind, var: usize, inputs: &[Input]) -> u32 { self.g.add(kind, var, inputs.to_vec()) }
    fn add(&mut self, a: Input, b: Input) -> u32 { self.n(Kind::Math, 0, &[a, b]) }
    fn mul(&mut self, a: Input, b: Input) -> u32 { self.n(Kind::Math, 2, &[a, b]) }
    fn env(&mut self, gate: Input, a: f32, d: f32, s: f32, r: f32) -> u32 {
        self.n(Kind::Env, 0, &[gate, Input::Const(a), Input::Const(d), Input::Const(s), Input::Const(r)])
    }
    fn set(&mut self, id: u32, i: usize, x: Input) { if let Some(n) = self.g.get_mut(id) { n.inputs[i] = x } }
}

impl Song {
    pub fn empty() -> Song {
        let mut g = Graph::default();
        g.add(Kind::Out, 0, vec![]);
        Song { bpm: 120.0, bars: 4, beats: 4, grid: 4, swing: 0.5, root: 0, minor: false, prog: vec![1, 5, 6, 4], next_track: 1, graph: g, tracks: vec![] }
    }
    pub fn sb(&self) -> f64 { 1.0 / self.grid as f64 }
    pub fn bar_steps(&self) -> usize { (self.beats * self.grid) as usize }
    pub fn steps(&self) -> usize { self.bars * self.bar_steps() }
    pub fn len_beats(&self) -> f64 { (self.bars as u32 * self.beats) as f64 }
    pub fn track_ids(&self) -> Vec<u32> { self.tracks.iter().map(|t| t.id).collect() }
    pub fn key_name(&self) -> String {
        format!("{} {}", NAMES[self.root.rem_euclid(12) as usize], if self.minor { "minor" } else { "major" })
    }
    pub fn scale(&self) -> [i32; 7] { if self.minor { MINOR } else { MAJOR } }
    pub fn in_scale(&self, p: i32) -> bool { self.scale().contains(&(p - self.root).rem_euclid(12)) }
    pub fn scale_step(&self, p: i32, dir: i32) -> i32 {
        let mut q = p;
        for _ in 0..12 { q += dir; if self.in_scale(q) { break } }
        q.clamp(0, 127)
    }
    pub fn degree_pitch(&self, degree: i32, octave: i32) -> i32 {
        12 * (octave + 1) + self.root + self.scale()[degree.rem_euclid(7) as usize] + 12 * degree.div_euclid(7)
    }
    pub fn chord(&self, degree: i32, octave: i32, n: usize) -> Vec<i32> {
        (0..n as i32).map(|i| self.degree_pitch(degree + 2 * i, octave)).collect()
    }
    pub fn bar_chord(&self, bar: usize) -> i32 {
        if self.prog.is_empty() { 0 } else { self.prog[bar % self.prog.len()] - 1 }
    }
    /// change grid resolution, keeping patterns where they are in time
    pub fn set_grid(&mut self, grid: u32) {
        let (old, new) = (self.grid as usize, grid as usize);
        for t in &mut self.tracks { for p in &mut t.patterns { p.start = p.start * new / old; p.len = (p.len * new / old).max(1) } }
        self.grid = grid;
    }
    fn swung(&self, beats: f64) -> f64 {
        let f = beats.rem_euclid(1.0);
        if (f - 0.5).abs() < 0.02 { beats + (self.swing - 0.5) } else { beats }
    }

    // ---- tracks ----
    pub fn add_track(&mut self, name: &str) -> usize {
        let id = self.next_track;
        self.next_track += 1;
        for var in 0..4 { self.graph.add_trk(id, var); }
        self.tracks.push(Track { id, name: name.into(), patterns: vec![], mute: false });
        self.tracks.len() - 1
    }
    pub fn remove_track(&mut self, t: usize) {
        let id = self.tracks[t].id;
        let ids: Vec<u32> = self.graph.nodes.iter().filter(|n| n.kind == Kind::Trk && n.track == id).map(|n| n.id).collect();
        for n in ids { self.graph.remove(n) }
        self.tracks.remove(t);
    }

    /// a standard playable voice for track t: TRK freq -> OSC saw -> FILT lp -> * ENV(gate) -> * vel -> * 0.3 -> OUT
    pub fn add_voice(&mut self, t: usize) -> u32 {
        use Input::{Const as C, Link as L};
        let id = self.tracks[t].id;
        let g = &mut self.graph;
        let l = |x: Option<u32>| x.map_or(C(0.0), L);
        let (f, gt, v) = (l(g.find_trk(id, 0)), l(g.find_trk(id, 1)), l(g.find_trk(id, 2)));
        let env = g.add(Kind::Env, 0, vec![gt, C(0.01), C(0.2), C(0.6), C(0.2)]);
        let osc = g.add(Kind::Osc, 1, vec![f, C(0.5), L(env)]);
        let flt = g.add(Kind::Filt, 0, vec![L(osc), C(1500.0), C(0.7)]);
        let vel = g.add(Kind::Math, 2, vec![L(flt), v]);
        let gain = g.add(Kind::Math, 2, vec![L(vel), C(0.3)]);
        g.to_out(gain);
        gain
    }

    // ---- patterns ----
    pub fn pat_at(&self, t: usize, step: usize) -> Option<(usize, usize)> {
        self.tracks[t].patterns.iter().enumerate().find_map(|(i, p)| p.rep_at(step).map(|k| (i, k)))
    }
    pub fn free(&self, t: usize, a: usize, b: usize, ignore: Option<usize>) -> bool {
        self.tracks[t].patterns.iter().enumerate().all(|(i, p)| Some(i) == ignore || p.end() <= a || p.start >= b)
    }
    pub fn new_pattern(&mut self, t: usize, a: usize, b: usize) -> Option<usize> {
        if b <= a || !self.free(t, a, b, None) { return None }
        self.tracks[t].patterns.push(Pattern { start: a, len: b - a, reps: 1, notes: vec![] });
        Some(self.tracks[t].patterns.len() - 1)
    }
    pub fn delete_pattern(&mut self, t: usize, i: usize) { self.tracks[t].patterns.remove(i); }
    /// turn repetition k of the pattern at `step` into its own independent pattern
    pub fn detach_at(&mut self, t: usize, step: usize) -> bool {
        let Some((i, k)) = self.pat_at(t, step) else { return false };
        if k == 0 { return false }
        let p = self.tracks[t].patterns[i].clone();
        let copy = Pattern { start: p.start + k * p.len, len: p.len, reps: 1, notes: p.notes.clone() };
        let tail = Pattern { start: p.start + (k + 1) * p.len, len: p.len, reps: p.reps - k - 1, notes: p.notes.clone() };
        self.tracks[t].patterns[i].reps = k;
        self.tracks[t].patterns.push(copy);
        if tail.reps > 0 { self.tracks[t].patterns.push(tail) }
        true
    }
    /// repeat until the next pattern on the track (or the end of the song)
    pub fn fill_repeat(&mut self, t: usize, i: usize) {
        let p = &self.tracks[t].patterns[i];
        let limit = self.tracks[t].patterns.iter().enumerate().filter(|(j, q)| *j != i && q.start >= p.start + p.len).map(|(_, q)| q.start).min().unwrap_or(self.steps()).max(p.start + p.len);
        let reps = ((limit - p.start) / p.len).max(1);
        self.tracks[t].patterns[i].reps = reps;
    }
    pub fn end_at(&mut self, t: usize, i: usize, step: usize) {
        let p = &mut self.tracks[t].patterns[i];
        p.reps = ((step.saturating_sub(p.start) + p.len - 1) / p.len).max(1);
    }
    pub fn move_pattern(&mut self, t: usize, i: usize, d: i64) -> bool {
        let p = &self.tracks[t].patterns[i];
        let ns = p.start as i64 + d;
        if ns < 0 || ns as usize + p.len * p.reps > self.steps() { return false }
        let (a, b) = (ns as usize, ns as usize + p.len * p.reps);
        if !self.free(t, a, b, Some(i)) { return false }
        self.tracks[t].patterns[i].start = a;
        true
    }
    pub fn resize_pattern(&mut self, t: usize, i: usize, d: i64) -> bool {
        let p = &self.tracks[t].patterns[i];
        let nl = (p.len as i64 + d).max(1) as usize;
        if p.start + nl * p.reps > self.steps() || !self.free(t, p.start, p.start + nl * p.reps, Some(i)) { return false }
        self.tracks[t].patterns[i].len = nl;
        true
    }
    /// all notes of a track in absolute beats (with swing)
    pub fn expanded_track(&self, t: usize) -> Vec<Note> {
        let sb = self.sb();
        let mut out = vec![];
        for p in &self.tracks[t].patterns {
            for k in 0..p.reps.max(1) {
                let base = (p.start + k * p.len) as f64 * sb;
                for n in &p.notes { out.push(Note { start: self.swung(base + n.start), ..n.clone() }) }
            }
        }
        out
    }
    /// one pattern's notes as they'd play looping on their own
    pub fn pattern_solo(&self, t: usize, i: usize) -> Vec<Note> {
        let base = self.tracks[t].patterns[i].start as f64 * self.sb();
        self.tracks[t].patterns[i].notes.iter().map(|n| Note { start: self.swung(base + n.start) - base, ..n.clone() }).collect()
    }

    // ---- notes inside pattern (t, i); steps are relative to the pattern ----
    pub fn notes_at(&self, t: usize, i: usize, step: usize) -> Vec<usize> {
        let sb = self.sb();
        self.tracks[t].patterns[i].notes.iter().enumerate().filter(|(_, n)| n.step(sb) == step).map(|(k, _)| k).collect()
    }
    pub fn add_note(&mut self, t: usize, i: usize, step: usize, len_steps: usize, pitch: i32) {
        let sb = self.sb();
        self.tracks[t].patterns[i].notes.push(Note { start: step as f64 * sb, len: len_steps as f64 * sb, pitch, vel: 0.9 });
    }
    pub fn edit_range(&mut self, t: usize, i: usize, a: usize, b: usize, mut f: impl FnMut(&mut Note, f64)) {
        let sb = self.sb();
        for n in &mut self.tracks[t].patterns[i].notes { let s = n.step(sb); if s >= a && s < b { f(n, sb) } }
    }
    pub fn clear_range(&mut self, t: usize, i: usize, a: usize, b: usize) {
        let sb = self.sb();
        self.tracks[t].patterns[i].notes.retain(|n| { let s = n.step(sb); s < a || s >= b })
    }
    pub fn range_notes(&self, t: usize, i: usize, a: usize, b: usize) -> Vec<Note> {
        let sb = self.sb();
        self.tracks[t].patterns[i].notes.iter().filter(|n| { let s = n.step(sb); s >= a && s < b }).map(|n| Note { start: n.start - a as f64 * sb, ..n.clone() }).collect()
    }
    pub fn humanize(&mut self, t: usize, i: usize, a: usize, b: usize) {
        self.edit_range(t, i, a, b, |n, sb| {
            n.start = (n.step(sb) as f64 * sb + (fastrand::f64() - 0.5) * 0.07).max(0.0);
            n.vel = (n.vel * (0.8 + fastrand::f32() * 0.25)).clamp(0.2, 1.0);
        })
    }
    pub fn transpose(&mut self, t: usize, i: usize, a: usize, b: usize, semis: i32) {
        self.edit_range(t, i, a, b, |n, _| n.pitch = (n.pitch + semis).clamp(0, 127))
    }
    pub fn nudge(&mut self, t: usize, i: usize, a: usize, b: usize, beats: f64) {
        self.edit_range(t, i, a, b, |n, _| n.start = (n.start + beats).max(0.0))
    }
    pub fn vel(&mut self, t: usize, i: usize, a: usize, b: usize, d: f32) {
        self.edit_range(t, i, a, b, |n, _| n.vel = (n.vel + d).clamp(0.05, 1.0))
    }
    pub fn resize_notes(&mut self, t: usize, i: usize, a: usize, b: usize, steps: f64) {
        self.edit_range(t, i, a, b, |n, sb| n.len = (n.len + steps * sb).max(sb / 8.0))
    }
    pub fn arpeggiate(&mut self, t: usize, i: usize, step: usize, len_steps: usize) {
        let sb = self.sb();
        let mut idx = self.notes_at(t, i, step);
        idx.sort_by_key(|&k| self.tracks[t].patterns[i].notes[k].pitch);
        for (j, k) in idx.into_iter().enumerate() {
            let n = &mut self.tracks[t].patterns[i].notes[k];
            n.start = (step + j * len_steps) as f64 * sb;
            n.len = len_steps as f64 * sb;
        }
    }
    /// new random melody over relative steps [a,b) using the progression's chord for each bar
    pub fn random_range(&mut self, t: usize, i: usize, a: usize, b: usize, octave: i32) {
        self.clear_range(t, i, a, b);
        let (sb, bs, base) = (self.sb(), self.bar_steps(), self.tracks[t].patterns[i].start);
        let mut prev = self.chord(self.bar_chord((base + a) / bs), octave, 3)[0];
        let mut step = a;
        while step < b {
            let deg = self.bar_chord((base + step) / bs);
            let tones: Vec<i32> = (-1..=1).flat_map(|o| self.chord(deg, octave + o, 3)).collect();
            let scale: Vec<i32> = (-7..=14).map(|d| self.degree_pitch(d, octave)).collect();
            let len = [1, 1, 2, 2, 2, 4][fastrand::usize(0..6)].min(b - step);
            if fastrand::f32() < 0.2 { step += len; continue; }
            let pool = if fastrand::f32() < 0.7 { &tones } else { &scale };
            let near: Vec<i32> = pool.iter().copied().filter(|p| (p - prev).abs() <= 7 && *p != prev).collect();
            let pick = if near.is_empty() { pool[fastrand::usize(0..pool.len())] } else { near[fastrand::usize(0..near.len())] };
            self.tracks[t].patterns[i].notes.push(Note { start: step as f64 * sb, len: len as f64 * sb * 0.9, pitch: pick, vel: 0.6 + fastrand::f32() * 0.4 });
            prev = pick;
            step += len;
        }
    }

    pub fn demo() -> Song {
        use Input::{Const as C, Link as L};
        let mut s = Song::empty();
        let lead = s.add_track("lead");
        let bass = s.add_track("bass");
        let drums = s.add_track("drums");
        let pad = s.add_track("pad");
        let ids = s.track_ids();
        let mut b = B { g: std::mem::take(&mut s.graph) };
        let trk = |b: &B, t: usize, var: usize| L(b.g.find_trk(ids[t], var).unwrap());

        // lead: saw -> env-swept lowpass -> feedback delay
        let (f, g, v) = (trk(&b, lead, 0), trk(&b, lead, 1), trk(&b, lead, 2));
        let osc = b.n(Kind::Osc, 1, &[f]);
        let env = b.env(g, 0.01, 0.2, 0.5, 0.3);
        let cs = b.mul(L(env), C(3000.0));
        let cut = b.add(L(cs), C(300.0));
        let flt = b.n(Kind::Filt, 0, &[L(osc), L(cut), C(0.8)]);
        let amp = b.mul(L(flt), L(env));
        let dry = b.mul(L(amp), v);
        let fbin = b.add(L(dry), C(0.0));
        let dl = b.n(Kind::Delay, 0, &[L(fbin), C(0.375)]);
        let fb = b.mul(L(dl), C(0.4));
        b.set(fbin, 1, L(fb));
        let wet = b.mul(L(dl), C(0.3));
        let lmix = b.add(L(dry), L(wet));
        let lead_out = b.mul(L(lmix), C(0.25));

        // bass: square -> lowpass -> tanh drive
        let (f, g, v) = (trk(&b, bass, 0), trk(&b, bass, 1), trk(&b, bass, 2));
        let o = b.n(Kind::Osc, 2, &[f]);
        let a = b.env(g, 0.005, 0.15, 0.3, 0.1);
        let cs = b.mul(L(a), C(900.0));
        let cut = b.add(L(cs), C(100.0));
        let fl = b.n(Kind::Filt, 0, &[L(o), L(cut), C(1.2)]);
        let am = b.mul(L(fl), L(a));
        let drv = b.mul(L(am), C(2.0));
        let sat = b.n(Kind::Shape, 0, &[L(drv)]);
        let bv = b.mul(L(sat), v);
        let bass_out = b.mul(L(bv), C(0.35));

        // drums: one track, KEY nodes route C4 -> kick chain, D4 -> hat chain
        let (g, v, note) = (trk(&b, drums, 1), trk(&b, drums, 2), trk(&b, drums, 3));
        let kg = b.n(Kind::Key, 0, &[g, note, C(60.0)]);
        let pe = b.env(L(kg), 0.001, 0.05, 0.0, 0.05);
        let ps = b.mul(L(pe), C(250.0));
        let pitch = b.add(L(ps), C(50.0));
        let ko = b.n(Kind::Osc, 0, &[L(pitch)]);
        let ka = b.env(L(kg), 0.001, 0.2, 0.0, 0.1);
        let km = b.mul(L(ko), L(ka));
        let kd = b.mul(L(km), C(3.0));
        let ks = b.n(Kind::Shape, 0, &[L(kd)]);
        let kick_out = b.mul(L(ks), C(0.7));
        let hg = b.n(Kind::Key, 0, &[g, note, C(62.0)]);
        let nz = b.n(Kind::Noise, 0, &[]);
        let hf = b.n(Kind::Filt, 1, &[L(nz), C(6000.0), C(0.7)]);
        let ha = b.env(L(hg), 0.001, 0.05, 0.0, 0.02);
        let hm = b.mul(L(hf), L(ha));
        let hv = b.mul(L(hm), v);
        let hat_out = b.mul(L(hv), C(0.25));

        // pad: two triangles with sample&hold random vibrato, smeared through a buffer read at a different rate
        let (f, g, v) = (trk(&b, pad, 0), trk(&b, pad, 1), trk(&b, pad, 2));
        let o1 = b.n(Kind::Osc, 3, &[f]);
        let clk = b.n(Kind::Osc, 2, &[C(6.0)]);
        let nz2 = b.n(Kind::Noise, 0, &[]);
        let rnd = b.n(Kind::Sh, 0, &[L(nz2), L(clk)]);
        let vib = b.mul(L(rnd), C(3.0));
        let det = b.mul(f, C(1.01));
        let f2 = b.add(L(det), L(vib));
        let o2 = b.n(Kind::Osc, 3, &[L(f2)]);
        let pm = b.add(L(o1), L(o2));
        let pe = b.env(g, 0.3, 0.5, 0.7, 0.8);
        let pa = b.mul(L(pm), L(pe));
        let wph = b.n(Kind::Osc, 5, &[C(0.5)]);
        let rph = b.n(Kind::Osc, 5, &[C(0.53)]);
        let buf = b.n(Kind::Buf, 0, &[L(pa), L(wph), L(rph), C(1.0), C(2.0)]);
        let bw = b.mul(L(buf), C(0.5));
        let ps = b.add(L(pa), L(bw));
        let pf = b.n(Kind::Filt, 0, &[L(ps), C(1200.0), C(0.5)]);
        let pv = b.mul(L(pf), v);
        let pad_out = b.mul(L(pv), C(0.12));

        // master mix
        let m1 = b.add(L(lead_out), L(bass_out));
        let m2 = b.add(L(m1), L(kick_out));
        let m3 = b.add(L(m2), L(hat_out));
        let m4 = b.add(L(m3), L(pad_out));
        let out = b.g.out_id().unwrap();
        b.set(out, 0, L(m4));
        s.graph = b.g;

        let sb = s.sb();
        let mel: [[i32; 8]; 4] = [
            [72, 76, 79, 76, 84, 83, 79, 76],
            [74, 79, 83, 79, 86, 83, 79, 77],
            [72, 76, 81, 76, 84, 83, 81, 76],
            [69, 72, 77, 72, 81, 79, 77, 74],
        ];
        let mut notes = vec![];
        for (bar, m) in mel.iter().enumerate() {
            for (i, &p) in m.iter().enumerate() {
                notes.push(Note { start: bar as f64 * 4.0 + i as f64 * 0.5, len: 0.45, pitch: p, vel: if i % 4 == 0 { 1.0 } else { 0.75 } });
            }
        }
        s.tracks[lead].patterns.push(Pattern { start: 0, len: 64, reps: 1, notes });
        let notes = [0, 3, 6, 8, 11, 14].iter().enumerate()
            .map(|(k, st)| Note { start: *st as f64 * sb, len: 0.4, pitch: 36 + if k % 3 == 2 { 12 } else { 0 }, vel: 0.9 }).collect();
        s.tracks[bass].patterns.push(Pattern { start: 0, len: 16, reps: 4, notes });
        let mut notes: Vec<Note> = [0, 4, 8, 12].iter().map(|st| Note { start: *st as f64 * sb, len: 0.25, pitch: 60, vel: 1.0 }).collect();
        notes.extend([2, 6, 10, 14].iter().map(|st| Note { start: *st as f64 * sb, len: 0.2, pitch: 62, vel: 0.8 }));
        s.tracks[drums].patterns.push(Pattern { start: 0, len: 16, reps: 4, notes });
        for (bar, chord) in [[48, 52, 55], [47, 50, 55], [45, 48, 52], [45, 48, 53]].iter().enumerate() {
            let notes = chord.iter().map(|&p| Note { start: 0.0, len: 3.8, pitch: p, vel: 0.8 }).collect();
            s.tracks[pad].patterns.push(Pattern { start: bar * 16, len: 16, reps: 1, notes });
        }
        s
    }
}
