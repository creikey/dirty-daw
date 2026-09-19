mod docs;
mod dsp;
mod engine;
mod graph;
mod song;

use eframe::egui::{self, Align2, Color32, Event, FontId, Key, Modifiers, Pos2, Rect, Stroke, Vec2};
use engine::{Msg, PlayData, Shared, TrackPlay};
use graph::{Input, Kind};
use song::{note_name, Note, Pattern, Song};
use std::path::PathBuf;
use std::sync::{atomic::Ordering, mpsc::Sender, Arc};

#[derive(PartialEq, Clone, Copy)]
enum View { Arrange, Pattern, Synth }

#[derive(PartialEq, Clone, Copy)]
enum SMode { Node, Param, Connect(usize), Palette, Splice, OutPick, OutInput, Disconnect }

const HELP_A1: &str = "h/l step  H/L bar  j/k track  g/G start/end  enter open/create pattern  P pattern from selection  v select  x delete pattern  y/p yank/paste";
const HELP_A2: &str = "R repeat to end  E end repeat here  D detach copy  {/} move  ,/. resize  r random  n humanize  m mute  u undo  s seek  space play  tab synth  : cmd";
const HELP_P1: &str = "esc back  h/l step  H/L bar  j/k pitch  J/K octave  enter note  1-7 degree  c#/C# chord/7th  a arp  v select  x/X del/clear  y/p yank/paste";
const HELP_P2: &str = "r random  n humanize  (/) nudge 1/8 step  -/= semitone  </> length  _/+ velocity  ,/. insert length  u undo  space loop pattern  tab synth";
const HELP_NODE: &str = "h/j/k/l move  enter edit inputs  c this output goes to...  i an input reads...  d disconnect output  o send to OUT  a add  x delete  w variant  v mark  f focus  F12 manual";
const HELP_PARAM: &str = "j/k choose input  h/l adjust (H/L fine)  type number + enter  i this input reads...  c this output goes to...  x disconnect input  w variant  enter/esc done";
const HELP_CONNECT: &str = "INPUT ◂ pick the node this input will read (h/j/k/l)  enter connect  esc cancel";
const CMDS: &str = "type a command + enter:  voice  |  new  |  sampleproject  |  bpm 128  |  bars 8  |  beats 3  |  grid 3  |  swing 0.67  |  key F# minor  |  prog 1 5 6 4  |  add pluck  |  del  |  name X  |  help";
const PALETTE: [Color32; 6] = [
    Color32::from_rgb(90, 200, 255), Color32::from_rgb(255, 205, 80), Color32::from_rgb(255, 120, 180),
    Color32::from_rgb(120, 230, 140), Color32::from_rgb(190, 140, 255), Color32::from_rgb(255, 160, 90),
];
const NODE_W: f32 = 24.0;
const NODE_GAP: f32 = 7.0;

fn tcolor(t: usize) -> Color32 { PALETTE[t % PALETTE.len()] }
fn kind_color(k: Kind) -> Color32 {
    match k {
        Kind::Trk => Color32::from_rgb(200, 200, 200), Kind::Osc => Color32::from_rgb(90, 200, 255),
        Kind::Noise => Color32::from_rgb(255, 120, 180), Kind::Env => Color32::from_rgb(120, 230, 140),
        Kind::Filt => Color32::from_rgb(255, 205, 80), Kind::Delay => Color32::from_rgb(190, 140, 255),
        Kind::Math => Color32::from_rgb(220, 220, 220), Kind::Shape => Color32::from_rgb(255, 160, 90),
        Kind::Sh => Color32::from_rgb(90, 220, 210), Kind::Buf => Color32::from_rgb(230, 170, 255),
        Kind::Key => Color32::from_rgb(255, 230, 150), Kind::Out => Color32::from_rgb(255, 110, 110),
    }
}
fn fmt_num(v: f32) -> String {
    let a = v.abs();
    if a >= 100.0 { format!("{v:.0}") } else if a >= 10.0 { format!("{v:.1}") } else if a >= 1.0 { format!("{v:.2}") } else { format!("{v:.3}") }
}

struct App {
    song: Song,
    programs: Vec<dsp::Program>,
    undo: Vec<Song>,
    path: Option<PathBuf>,
    view: View,
    back: View,
    // arrange
    track: usize,
    cursor: usize,
    view_start: usize,
    pclip: Option<Pattern>,
    // pattern editor
    pat: Option<usize>,
    pcursor: usize,
    pview_start: usize,
    cpitch: i32,
    view_top: i32,
    ins_len: usize,
    sel: Option<usize>,
    pending: Option<char>,
    clip: Vec<Note>,
    // synth
    sel_node: Option<u32>,
    smode: SMode,
    param: usize,
    num: String,
    pick: Option<u32>,
    gscroll: Vec2,
    node_meters: Vec<f32>,
    track_meters: Vec<f32>,
    marks: Vec<u32>,
    focus: bool,
    splice_kind: Kind,
    splice_choices: Vec<(u32, usize)>,
    splice_idx: usize,
    // misc
    cmd: Option<String>,
    msg: String,
    playing: bool,
    tx: Sender<Msg>,
    shared: Arc<Shared>,
    _stream: cpal::Stream,
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 760.0]).with_title("dirty daw"),
        ..Default::default()
    };
    eframe::run_native("dirty daw", opts, Box::new(|_| Ok(Box::new(App::new()))))
}

impl App {
    fn new() -> App {
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared::default());
        let stream = engine::start(rx, shared.clone());
        let mut app = App {
            song: Song::empty(), programs: vec![], undo: vec![], path: None, view: View::Arrange, back: View::Arrange,
            track: 0, cursor: 0, view_start: 0, pclip: None,
            pat: None, pcursor: 0, pview_start: 0, cpitch: 72, view_top: 84, ins_len: 2, sel: None, pending: None, clip: vec![],
            sel_node: None, smode: SMode::Node, param: 0, num: String::new(), pick: None, gscroll: Vec2::ZERO, node_meters: vec![], track_meters: vec![], marks: vec![], focus: false, splice_kind: Kind::Math, splice_choices: vec![], splice_idx: 0,
            cmd: None, msg: "empty project. enter = create a pattern, tab = synth graph, :voice = give this track a playable synth, :sampleproject = demo song, F12 = manual".into(),
            playing: false, tx, shared, _stream: stream,
        };
        app.song.add_track("track1");
        app.sync();
        app
    }

    /// replace the whole project (used by :new, :sampleproject and opening a file)
    fn load(&mut self, song: Song, path: Option<PathBuf>) {
        self.edit();
        self.song = song;
        self.song.graph.normalize();
        self.path = path;
        self.cursor = 0; self.track = 0; self.sel = None; self.sel_node = None; self.pat = None;
        self.marks.clear(); self.focus = false; self.gscroll = Vec2::ZERO;
        self.view = View::Arrange; self.smode = SMode::Node;
        self.clamp();
        self.sync();
        self.seek(0.0);
    }

    // ---------- state plumbing ----------
    fn sync(&mut self) {
        self.programs = self.song.graph.compile_all(&self.song.track_ids());
        let pd = match self.pat {
            Some(i) if i < self.song.tracks[self.track].patterns.len() => PlayData {
                bpm: self.song.bpm,
                len: self.song.tracks[self.track].patterns[i].len as f64 * self.song.sb(),
                tracks: (0..self.song.tracks.len()).map(|t| TrackPlay { mute: false, notes: if t == self.track { self.song.pattern_solo(t, i) } else { vec![] } }).collect(),
            },
            _ => PlayData {
                bpm: self.song.bpm,
                len: self.song.len_beats(),
                tracks: (0..self.song.tracks.len()).map(|t| TrackPlay { mute: self.song.tracks[t].mute, notes: self.song.expanded_track(t) }).collect(),
            },
        };
        self.tx.send(Msg::Set(Arc::new(pd), Arc::new(self.programs.clone()))).ok();
    }
    fn edit(&mut self) {
        self.undo.push(self.song.clone());
        if self.undo.len() > 300 { self.undo.remove(0); }
    }
    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() { self.song = s; self.clamp(); self.sync(); self.msg = "undo".into(); }
    }
    fn clamp(&mut self) {
        if self.song.tracks.is_empty() { self.song.add_track("track1"); }
        self.track = self.track.min(self.song.tracks.len() - 1);
        self.cursor = self.cursor.min(self.song.steps().saturating_sub(1));
        if let Some(i) = self.pat {
            match self.song.tracks[self.track].patterns.get(i) {
                Some(p) => self.pcursor = self.pcursor.min(p.len - 1),
                None => { self.pat = None; if self.view == View::Pattern { self.view = View::Arrange } }
            }
        }
        let g = &self.song.graph;
        self.marks.retain(|&m| g.get(m).is_some());
        if self.sel_node.map_or(true, |id| g.get(id).is_none()) { self.sel_node = g.out_id(); }
        let vis = self.visible();
        if let Some(i) = self.sel_node.and_then(|id| g.idx(id)) { if !vis[i] { self.sel_node = vis.iter().position(|&v| v).map(|j| g.nodes[j].id) } }
        let np = self.sel_node.and_then(|id| g.get(id)).map_or(0, |n| n.inputs.len());
        self.param = self.param.min(np.saturating_sub(1));
        if np == 0 && !matches!(self.smode, SMode::Palette | SMode::Splice | SMode::OutPick | SMode::OutInput | SMode::Disconnect) { self.smode = SMode::Node }
    }
    fn seek(&self, beats: f64) { self.tx.send(Msg::Seek(beats)).ok(); }
    fn cursor_beats(&self) -> f64 {
        if self.pat.is_some() { self.pcursor as f64 * self.song.sb() } else { self.cursor as f64 * self.song.sb() }
    }
    fn toggle_play(&mut self) {
        self.playing = !self.playing;
        if self.playing { self.seek(self.cursor_beats()); }
        self.tx.send(Msg::Play(self.playing)).ok();
    }
    fn playhead(&self) -> f64 {
        if self.playing { f64::from_bits(self.shared.pos.load(Ordering::Relaxed)) } else { self.cursor_beats() }
    }
    fn poll_meters(&mut self) {
        if let Ok(mut m) = self.shared.meters.try_lock() {
            self.node_meters.resize(m.0.len(), 0.0);
            self.track_meters.resize(m.1.len(), 0.0);
            for (a, b) in self.node_meters.iter_mut().zip(m.0.iter_mut()) { *a = b.max(*a * 0.82); *b = 0.0 }
            for (a, b) in self.track_meters.iter_mut().zip(m.1.iter_mut()) { *a = b.max(*a * 0.82); *b = 0.0 }
        }
    }
    fn node_meter(&self, node_idx: usize) -> f32 { self.node_meters.get(node_idx).copied().unwrap_or(0.0).min(1.0) }

    fn open_pattern(&mut self, i: usize) {
        self.pat = Some(i);
        let p = self.song.tracks[self.track].patterns[i].clone();
        self.pcursor = self.cursor.saturating_sub(p.start) % p.len.max(1);
        self.sel = None;
        self.view = View::Pattern;
        self.sync();
        if self.playing { self.seek(0.0) }
        self.msg = format!("editing pattern {} of {} ({} steps, plays {}x). esc goes back", i + 1, self.song.tracks[self.track].name, p.len, p.reps);
    }
    fn close_pattern(&mut self) {
        if let Some(i) = self.pat { if let Some(p) = self.song.tracks[self.track].patterns.get(i) { self.cursor = p.start + self.pcursor } }
        self.pat = None;
        self.sel = None;
        self.view = View::Arrange;
        self.sync();
        if self.playing { self.seek(self.cursor_beats()) }
    }

    // ---------- files ----------
    fn save(&mut self, as_new: bool) {
        if self.path.is_none() || as_new {
            self.path = rfd::FileDialog::new().add_filter("dirty daw song", &["dd"]).set_file_name("song.dd").save_file();
        }
        if let Some(p) = self.path.clone() {
            let r = serde_json::to_string_pretty(&self.song).map_err(|e| e.to_string()).and_then(|s| std::fs::write(&p, s).map_err(|e| e.to_string()));
            self.msg = match r { Ok(()) => format!("saved {}", p.display()), Err(e) => format!("save failed: {e}") };
        }
    }
    fn open(&mut self) {
        let Some(p) = rfd::FileDialog::new().add_filter("dirty daw song", &["dd"]).pick_file() else { return };
        let r = std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str::<Song>(&s).map_err(|e| e.to_string()));
        match r {
            Ok(s) => { self.load(s, Some(p.clone())); self.msg = format!("opened {}", p.display()); }
            Err(e) => self.msg = format!("open failed: {e}"),
        }
    }

    // ---------- input ----------
    fn handle(&mut self, ctx: &egui::Context) {
        for e in ctx.input(|i| i.events.clone()) {
            match e {
                Event::Key { key, pressed: true, modifiers, .. } => self.key(key, modifiers),
                Event::Text(t) => for c in t.chars() { self.char(c) },
                _ => {}
            }
        }
    }

    fn key(&mut self, k: Key, m: Modifiers) {
        if m.command {
            match k { Key::S => self.save(m.shift), Key::O => self.open(), Key::Z => self.undo(), _ => {} }
            return;
        }
        if self.cmd.is_some() {
            match k {
                Key::Enter => { let c = self.cmd.take().unwrap(); self.run_cmd(&c) }
                Key::Escape => self.cmd = None,
                Key::Backspace => { self.cmd.as_mut().unwrap().pop(); }
                _ => {}
            }
            return;
        }
        match k {
            Key::F12 | Key::F1 => { self.msg = docs::open(); return }
            Key::Space => { self.toggle_play(); return }
            Key::Tab => {
                if self.view == View::Synth { self.view = self.back; }
                else {
                    self.back = self.view;
                    self.view = View::Synth;
                    let tid = self.song.tracks[self.track].id;
                    self.sel_node = self.song.graph.find_trk(tid, 1).or(self.sel_node);
                }
                self.smode = SMode::Node; self.num.clear(); self.clamp();
                return;
            }
            Key::Escape => {
                self.pending = None; self.num.clear();
                match self.view {
                    View::Synth => self.smode = match self.smode { SMode::Connect(_) => SMode::Param, _ => SMode::Node },
                    View::Pattern => { if self.sel.is_some() { self.sel = None } else { self.close_pattern() } }
                    View::Arrange => self.sel = None,
                }
                return;
            }
            _ => {}
        }
        match self.view { View::Arrange => self.key_arr(k), View::Pattern => self.key_pat(k), View::Synth => self.key_synth(k) }
    }

    fn char(&mut self, c: char) {
        if c.is_control() { return }
        if let Some(cmd) = &mut self.cmd { cmd.push(c); return }
        if c == ' ' { return }
        match self.view { View::Arrange => self.char_arr(c), View::Pattern => self.char_pat(c), View::Synth => self.char_synth(c) }
    }

    // ---------- arrange view ----------
    fn mv(&mut self, d: i64) { self.cursor = (self.cursor as i64 + d).clamp(0, self.song.steps() as i64 - 1) as usize }
    fn mv_track(&mut self, d: i64) { self.track = (self.track as i64 + d).clamp(0, self.song.tracks.len() as i64 - 1) as usize }
    fn ascope(&self) -> (usize, usize) {
        match self.sel {
            Some(s) => (s.min(self.cursor), s.max(self.cursor) + 1),
            None => { let bs = self.song.bar_steps(); let b = self.cursor / bs * bs; (b, b + bs) }
        }
    }

    fn key_arr(&mut self, k: Key) {
        match k {
            Key::Enter => {
                match self.song.pat_at(self.track, self.cursor) {
                    Some((i, _)) => self.open_pattern(i),
                    None => {
                        let (a, b) = self.ascope();
                        let b = self.song.tracks[self.track].patterns.iter().filter(|p| p.start > a).map(|p| p.start).min().unwrap_or(b).min(b);
                        self.edit();
                        match self.song.new_pattern(self.track, a, b) { Some(i) => self.open_pattern(i), None => self.msg = "no room for a pattern here".into() }
                    }
                }
            }
            Key::Delete | Key::Backspace => self.char_arr('x'),
            Key::ArrowLeft => self.mv(-1), Key::ArrowRight => self.mv(1),
            Key::ArrowUp => self.mv_track(-1), Key::ArrowDown => self.mv_track(1),
            _ => {}
        }
    }

    fn char_arr(&mut self, c: char) {
        let (t, s) = (self.track, self.cursor);
        let bs = self.song.bar_steps() as i64;
        let here = self.song.pat_at(t, s);
        match c {
            'h' => self.mv(-1), 'l' => self.mv(1), 'H' => self.mv(-bs), 'L' => self.mv(bs),
            'j' => self.mv_track(1), 'k' => self.mv_track(-1),
            'g' => self.cursor = 0, 'G' => self.cursor = self.song.steps() - 1,
            'v' => { self.sel = if self.sel.is_some() { None } else { Some(s) }; }
            'P' => {
                let (a, b) = self.ascope();
                self.edit();
                match self.song.new_pattern(t, a, b) { Some(_) => { self.sel = None; self.sync(); self.msg = format!("new pattern, {} steps", b - a) } None => self.msg = "overlaps an existing pattern".into() }
            }
            'x' => { if let Some((i, _)) = here { self.edit(); self.song.delete_pattern(t, i); self.sync(); self.msg = "pattern deleted".into() } }
            'y' => { if let Some((i, _)) = here { self.pclip = Some(self.song.tracks[t].patterns[i].clone()); self.msg = "pattern yanked".into() } }
            'p' => {
                if let Some(p) = self.pclip.clone() {
                    self.edit();
                    if self.song.free(t, s, s + p.len, None) && s + p.len <= self.song.steps() {
                        self.song.tracks[t].patterns.push(Pattern { start: s, reps: 1, ..p });
                        self.sync(); self.msg = "pasted".into()
                    } else { self.msg = "no room to paste here".into() }
                }
            }
            'R' => { if let Some((i, _)) = here { self.edit(); self.song.fill_repeat(t, i); self.sync(); self.msg = "repeating until the next pattern / end of song. E ends it at the cursor".into() } }
            'E' => { if let Some((i, _)) = here { self.edit(); self.song.end_at(t, i, s); self.sync(); self.msg = "repeat ends here".into() } }
            'D' => { self.edit(); let ok = self.song.detach_at(t, s); self.sync(); self.msg = if ok { "detached: this copy is its own pattern now".into() } else { "cursor is not on a repeated copy".into() } }
            '{' | '}' => { if let Some((i, _)) = here { self.edit(); if !self.song.move_pattern(t, i, if c == '{' { -1 } else { 1 }) { self.msg = "blocked".into() } self.sync() } }
            ',' | '.' => { if let Some((i, _)) = here { self.edit(); if !self.song.resize_pattern(t, i, if c == ',' { -1 } else { 1 }) { self.msg = "blocked".into() } self.sync() } }
            'r' => { if let Some((i, _)) = here { self.edit(); let l = self.song.tracks[t].patterns[i].len; self.song.random_range(t, i, 0, l, self.cpitch.div_euclid(12) - 1); self.sync(); self.msg = "new random melody in this pattern".into() } }
            'n' => { if let Some((i, _)) = here { self.edit(); let l = self.song.tracks[t].patterns[i].len; self.song.humanize(t, i, 0, l); self.sync(); self.msg = "humanized pattern".into() } }
            'm' => { self.edit(); self.song.tracks[t].mute = !self.song.tracks[t].mute; self.sync() }
            'u' => self.undo(),
            's' => self.seek(s as f64 * self.song.sb()),
            ':' => { self.cmd = Some(String::new()); self.msg = CMDS.into() }
            _ => {}
        }
    }

    // ---------- pattern editor ----------
    fn plen(&self) -> usize { self.pat.and_then(|i| self.song.tracks[self.track].patterns.get(i)).map_or(1, |p| p.len) }
    fn pmv(&mut self, d: i64) { self.pcursor = (self.pcursor as i64 + d).clamp(0, self.plen() as i64 - 1) as usize }
    fn octave(&self) -> i32 { self.cpitch.div_euclid(12) - 1 }
    /// selection if any, else the whole pattern
    fn pscope(&self) -> (usize, usize) {
        match self.sel { Some(s) => (s.min(self.pcursor), s.max(self.pcursor) + 1), None => (0, self.plen()) }
    }

    fn key_pat(&mut self, k: Key) {
        match k {
            Key::Enter => self.toggle_note(),
            Key::ArrowLeft => self.pmv(-1), Key::ArrowRight => self.pmv(1),
            Key::ArrowUp => self.cpitch = self.song.scale_step(self.cpitch, 1),
            Key::ArrowDown => self.cpitch = self.song.scale_step(self.cpitch, -1),
            _ => {}
        }
    }

    fn toggle_note(&mut self) {
        let Some(i) = self.pat else { return };
        let (t, s, p, sb) = (self.track, self.pcursor, self.cpitch, self.song.sb());
        self.edit();
        let notes = &mut self.song.tracks[t].patterns[i].notes;
        let before = notes.len();
        notes.retain(|n| !(n.step(sb) == s && n.pitch == p));
        if notes.len() == before { self.song.add_note(t, i, s, self.ins_len, p); self.pmv(self.ins_len as i64) }
        self.sync();
    }

    fn insert(&mut self, pitches: &[i32]) {
        let Some(i) = self.pat else { return };
        let (t, s, l, sb) = (self.track, self.pcursor, self.ins_len, self.song.sb());
        self.edit();
        self.song.tracks[t].patterns[i].notes.retain(|n| n.step(sb) != s);
        for &p in pitches { self.song.add_note(t, i, s, l, p) }
        if let Some(&p) = pitches.first() { self.cpitch = p }
        self.pmv(l as i64);
        self.sync();
    }

    fn char_pat(&mut self, c: char) {
        let Some(i) = self.pat else { return };
        if let Some(p) = self.pending.take() {
            if let Some(d) = c.to_digit(10).filter(|d| (1..=7).contains(d)) {
                let chord = self.song.chord(d as i32 - 1, self.octave(), if p == 'c' { 3 } else { 4 });
                self.insert(&chord);
            }
            return;
        }
        let (t, s, sb) = (self.track, self.pcursor, self.song.sb());
        let bs = self.song.bar_steps() as i64;
        let (a, b) = self.pscope();
        let (na, nb) = if self.sel.is_some() { (a, b) } else { (s, s + 1) }; // note-level ops: selection or cursor step
        match c {
            'h' => self.pmv(-1), 'l' => self.pmv(1), 'H' => self.pmv(-bs), 'L' => self.pmv(bs),
            'j' => self.cpitch = self.song.scale_step(self.cpitch, -1),
            'k' => self.cpitch = self.song.scale_step(self.cpitch, 1),
            'J' => self.cpitch = (self.cpitch - 12).max(0), 'K' => self.cpitch = (self.cpitch + 12).min(127),
            'g' => self.pcursor = 0, 'G' => self.pcursor = self.plen() - 1,
            '1'..='7' => { let p = self.song.degree_pitch(c as i32 - '1' as i32, self.octave()); self.insert(&[p]) }
            'c' | 'C' => { self.pending = Some(c); self.msg = format!("{c} + degree 1-7 for a {}", if c == 'c' { "triad" } else { "7th chord" }) }
            'v' => { self.sel = if self.sel.is_some() { None } else { Some(s) }; self.msg = if self.sel.is_some() { "selecting: move, then r/n/X/y/-/=/(/)/</>".into() } else { String::new() } }
            'a' => { self.edit(); self.song.arpeggiate(t, i, s, self.ins_len); self.sync(); self.msg = "arpeggiated".into() }
            'r' => { self.edit(); self.song.random_range(t, i, a, b, self.octave()); self.sync(); self.msg = "new random melody".into() }
            'x' => {
                self.edit();
                let notes = &mut self.song.tracks[t].patterns[i].notes;
                let p = self.cpitch;
                if notes.iter().any(|n| n.step(sb) == s && n.pitch == p) { notes.retain(|n| !(n.step(sb) == s && n.pitch == p)) } else { notes.retain(|n| n.step(sb) != s) }
                self.sync();
            }
            'X' => { self.edit(); self.song.clear_range(t, i, a, b); self.sel = None; self.sync(); self.msg = "cleared".into() }
            'y' => { self.clip = self.song.range_notes(t, i, a, b); self.sel = None; self.msg = format!("yanked {} notes", self.clip.len()) }
            'p' => {
                if self.clip.is_empty() { self.msg = "nothing yanked".into(); return }
                self.edit();
                let len = self.clip.iter().map(|n| n.step(sb) + 1).max().unwrap_or(0);
                self.song.clear_range(t, i, s, s + len);
                for n in &self.clip { let mut n = n.clone(); n.start += s as f64 * sb; self.song.tracks[t].patterns[i].notes.push(n) }
                self.sync();
            }
            'n' => { self.edit(); self.song.humanize(t, i, a, b); self.sync(); self.msg = "humanized".into() }
            'u' => self.undo(),
            '-' | '=' => { self.edit(); self.song.transpose(t, i, na, nb, if c == '-' { -1 } else { 1 }); self.sync() }
            '(' | ')' => { self.edit(); self.song.nudge(t, i, na, nb, if c == '(' { -sb / 8.0 } else { sb / 8.0 }); self.sync(); self.msg = "nudged 1/8 step".into() }
            '<' | '>' => { self.edit(); self.song.resize_notes(t, i, na, nb, if c == '<' { -1.0 } else { 1.0 }); self.sync() }
            '_' | '+' => { self.edit(); self.song.vel(t, i, na, nb, if c == '_' { -0.1 } else { 0.1 }); self.sync() }
            ',' => self.ins_len = (self.ins_len / 2).max(1),
            '.' => self.ins_len = (self.ins_len * 2).min(64),
            's' => self.seek(s as f64 * sb),
            ':' => { self.cmd = Some(String::new()); self.msg = CMDS.into() }
            _ => {}
        }
    }

    fn run_cmd(&mut self, c: &str) {
        let w: Vec<&str> = c.split_whitespace().collect();
        let num = |i: usize| w.get(i).and_then(|s| s.parse::<f64>().ok());
        self.msg = format!("ok: {c}");
        match w.first().copied() {
            Some("bpm") => match num(1) { Some(v) => { self.edit(); self.song.bpm = v.clamp(20.0, 400.0); self.sync() } None => self.msg = "bpm N".into() },
            Some("bars") => match num(1) { Some(v) => { self.edit(); self.song.bars = (v as usize).clamp(1, 256); self.clamp(); self.sync() } None => self.msg = "bars N".into() },
            Some("beats") => match num(1) { Some(v) => { self.edit(); self.song.beats = (v as u32).clamp(1, 16); self.clamp(); self.sync() } None => self.msg = "beats N (per bar)".into() },
            Some("grid") => match num(1) { Some(v) => { self.edit(); self.song.set_grid((v as u32).clamp(1, 32)); self.clamp(); self.sync() } None => self.msg = "grid N (steps per beat, 3 = triplets)".into() },
            Some("swing") => match num(1) { Some(v) => { self.edit(); self.song.swing = v.clamp(0.25, 0.75); self.sync() } None => self.msg = "swing 0.5 (straight) .. 0.67 (triplet)".into() },
            Some("key") => match w.get(1).and_then(|s| song::parse_root(s)) {
                Some(r) => { self.edit(); self.song.root = r; self.song.minor = w.get(2).map_or(w[1].ends_with('m'), |m| m.starts_with('m')); self.sync() }
                None => self.msg = "key C | key F# minor | key Bbm".into(),
            },
            Some("prog") => {
                let p: Vec<i32> = w[1..].iter().filter_map(|s| s.parse().ok()).filter(|d| (1..=7).contains(d)).collect();
                if p.is_empty() { self.msg = "prog 1 5 6 4".into() } else { self.edit(); self.song.prog = p; self.sync() }
            }
            Some("add") => {
                self.edit();
                let name = w.get(1).map(|s| s.to_string()).unwrap_or_else(|| format!("track{}", self.song.tracks.len() + 1));
                self.track = self.song.add_track(&name);
                self.sync();
                self.msg = format!("added {name}: its TRK nodes are in the synth graph (tab)");
            }
            Some("del") => {
                if self.song.tracks.len() > 1 { self.edit(); self.song.remove_track(self.track); self.pat = None; self.clamp(); self.sync() }
                else { self.msg = "can't delete the last track".into() }
            }
            Some("help") => self.msg = docs::open(),
            Some("voice") => { self.edit(); let last = self.song.add_voice(self.track); self.sel_node = Some(last); self.sync(); self.msg = format!("built a voice for {}: TRK freq → OSC → FILT → * ENV → * vel → * gain → OUT. tab to see it", self.song.tracks[self.track].name) }
            Some("new") => { let mut s = Song::empty(); s.add_track("track1"); self.load(s, None); self.msg = "new empty project (u undoes this)".into() }
            Some("sampleproject") | Some("sample") | Some("demo") => { self.load(Song::demo(), None); self.msg = "sample project loaded: 4 tracks, every node kind in the graph (u undoes this)".into(); if !self.playing { self.toggle_play() } }
            Some("name") => match w.get(1) { Some(n) => { self.edit(); self.song.tracks[self.track].name = n.to_string(); self.sync() } None => self.msg = "name X".into() },
            Some(_) => self.msg = format!("unknown command. {CMDS}"),
            None => self.msg.clear(),
        }
    }

    // ---------- synth view (node graph) ----------
    fn sel_id(&self) -> u32 { self.sel_node.or_else(|| self.song.graph.out_id()).unwrap_or(0) }

    /// which nodes are shown: all, or (in focus mode) the marks plus everything up/downstream of them
    fn visible(&self) -> Vec<bool> {
        let g = &self.song.graph;
        if self.focus && !self.marks.is_empty() { g.related(&self.marks) } else { vec![true; g.nodes.len()] }
    }
    fn layout(&self) -> Vec<(usize, usize)> { self.song.graph.layout_masked(&self.song.track_ids(), &self.visible()) }

    fn nav(&self, from: u32, dc: i32, dr: i32) -> Option<u32> {
        let g = &self.song.graph;
        let lay = self.layout();
        let i = g.idx(from)?;
        let (c, r) = lay[i];
        if c == usize::MAX { return lay.iter().position(|p| p.0 != usize::MAX).map(|j| g.nodes[j].id) }
        let ncols = lay.iter().filter(|p| p.0 != usize::MAX).map(|p| p.0).max().unwrap_or(0) as i32 + 1;
        if dc != 0 {
            let mut col = c as i32 + dc;
            while col >= 0 && col < ncols {
                let best = lay.iter().enumerate().filter(|(_, p)| p.0 == col as usize).min_by_key(|(_, p)| (p.1 as i32 - r as i32).abs());
                if let Some((j, _)) = best { return Some(g.nodes[j].id) }
                col += dc;
            }
            None
        } else {
            let want = (r as i32 + dr).max(0) as usize;
            lay.iter().enumerate().find(|(_, p)| p.0 == c && p.1 == want).map(|(j, _)| g.nodes[j].id)
        }
    }

    fn move_sel(&mut self, dc: i32, dr: i32) {
        let picking = matches!(self.smode, SMode::Connect(_) | SMode::OutPick);
        let cur = if picking { self.pick.unwrap_or(self.sel_id()) } else { self.sel_id() };
        if let Some(id) = self.nav(cur, dc, dr) {
            if picking { self.pick = Some(id) } else { self.sel_node = Some(id); self.param = 0; }
        }
    }

    fn set_input(&mut self, x: Input) {
        let (id, pi) = (self.sel_id(), self.param);
        self.edit();
        if let Some(n) = self.song.graph.get_mut(id) { if pi < n.inputs.len() { n.inputs[pi] = x } }
        self.sync();
    }
    fn cur_node(&self) -> Option<&graph::Node> { self.song.graph.get(self.sel_id()) }
    fn cur_input(&self) -> Option<Input> { self.cur_node().and_then(|n| n.inputs.get(self.param).copied()) }
    fn is_key_param(&self) -> bool { self.cur_node().map_or(false, |n| n.kind == Kind::Key && self.param == 2) }

    fn adjust(&mut self, up: bool, fine: bool) {
        let d = if up { 1 } else { -1 };
        if let Some(n) = self.cur_node() {
            if n.kind == Kind::Trk {
                let ids = self.song.track_ids();
                let cur = ids.iter().position(|&t| t == n.track).unwrap_or(0) as i32;
                let nt = ids[(cur + d).rem_euclid(ids.len() as i32) as usize];
                let id = n.id;
                self.edit();
                self.song.graph.get_mut(id).unwrap().track = nt;
                self.sync();
                return;
            }
        }
        let Some(Input::Const(v)) = self.cur_input() else { self.msg = "linked input: x disconnects it".into(); return };
        let nv = if self.is_key_param() { (v + d as f32).clamp(0.0, 127.0) } else {
            let f = if fine { 1.01 } else { 1.12 };
            if up { if v == 0.0 { 0.1 } else { v * f } } else { let n = v / f; if n.abs() < 0.001 { 0.0 } else { n } }
        };
        self.set_input(Input::Const(nv));
    }

    fn commit_num(&mut self) {
        if let Ok(v) = self.num.parse::<f32>() { if v.is_finite() { self.set_input(Input::Const(v)) } }
        else if let Some(p) = song::parse_root(&self.num) {
            let oct: i32 = self.num.chars().skip_while(|c| !c.is_ascii_digit() && *c != '-').collect::<String>().parse().unwrap_or(4);
            self.set_input(Input::Const((12 * (oct + 1) + p) as f32));
        }
        self.num.clear();
    }

    fn cycle_variant(&mut self, d: i32) {
        let id = self.sel_id();
        let Some(n) = self.song.graph.get(id) else { return };
        let nv = n.kind.variants().len() as i32;
        if nv == 0 { return }
        let var = (n.var as i32 + d).rem_euclid(nv) as usize;
        self.edit();
        self.song.graph.get_mut(id).unwrap().var = var;
        self.sync();
    }

    /// add a node after `after`; `targets` = which consumer inputs of `after` should now read the new node (None = all)
    fn add_node(&mut self, kind: Kind, after: Option<u32>, targets: Option<Vec<(u32, usize)>>) {
        self.edit();
        let tid = self.song.tracks[self.track].id;
        let id = match (after, targets) {
            (Some(a), Some(t)) => self.song.graph.insert_into(kind, a, &t, tid),
            _ => self.song.graph.insert(kind, after, tid),
        };
        let first = self.song.graph.get(id).and_then(|n| n.inputs.first().copied());
        let src = match first { Some(Input::Link(s)) => self.song.graph.get(s).map(|n| n.short()), _ => None };
        if self.focus { self.marks.push(id) }
        self.sel_node = Some(id);
        self.sync();
        self.msg = match src { Some(src) => format!("added {} reading {} on its first input", kind.name(), src), None => format!("added {}", kind.name()) };
    }

    fn confirm_splice(&mut self) {
        let after = self.sel_node;
        let n = self.splice_choices.len();
        let targets = match self.splice_idx {
            0 => self.splice_choices.clone(),
            i if i <= n => vec![self.splice_choices[i - 1]],
            _ => vec![],
        };
        self.smode = SMode::Node;
        self.add_node(self.splice_kind, after, Some(targets));
    }

    fn pick_inputs(&self) -> usize { self.pick.and_then(|p| self.song.graph.get(p)).map_or(0, |n| n.inputs.len()) }

    fn confirm_out_input(&mut self) {
        let (src, Some(t)) = (self.sel_id(), self.pick) else { return };
        let k = self.splice_idx;
        self.edit();
        if let Some(n) = self.song.graph.get_mut(t) { if k < n.inputs.len() { n.inputs[k] = Input::Link(src) } }
        self.sync();
        let name = self.song.graph.get(t).map_or(String::new(), |n| format!("{}.{}", n.short(), n.kind.params()[k]));
        self.msg = format!("connected: {} now reads {}", name, self.song.graph.get(src).map_or(String::new(), |n| n.short()));
        self.smode = SMode::Node;
    }

    fn confirm_disconnect(&mut self) {
        let n = self.splice_choices.len();
        let targets: Vec<(u32, usize)> = if self.splice_idx < n { vec![self.splice_choices[self.splice_idx]] } else { self.splice_choices.clone() };
        self.edit();
        for (t, k) in targets { self.song.graph.disconnect(t, k) }
        self.sync();
        self.msg = "disconnected".into();
        self.smode = SMode::Node;
    }

    fn key_synth(&mut self, k: Key) {
        match self.smode {
            SMode::Palette => {}
            SMode::OutPick => match k {
                Key::Enter => {
                    if self.pick == Some(self.sel_id()) { self.msg = "pick a different node".into() }
                    else if self.pick_inputs() == 0 { self.msg = "that node has no inputs".into() }
                    else if self.pick_inputs() == 1 { self.smode = SMode::OutInput; self.splice_idx = 0; self.confirm_out_input() }
                    else { self.smode = SMode::OutInput; self.splice_idx = 0; self.msg = "which input of that node should read this output? j/k or number, enter".into() }
                }
                Key::ArrowLeft => self.move_sel(-1, 0), Key::ArrowRight => self.move_sel(1, 0),
                Key::ArrowUp => self.move_sel(0, -1), Key::ArrowDown => self.move_sel(0, 1),
                _ => {}
            },
            SMode::OutInput => match k {
                Key::Enter => self.confirm_out_input(),
                Key::ArrowDown => self.splice_idx = (self.splice_idx + 1).min(self.pick_inputs().saturating_sub(1)),
                Key::ArrowUp => self.splice_idx = self.splice_idx.saturating_sub(1),
                _ => {}
            },
            SMode::Disconnect => match k {
                Key::Enter => self.confirm_disconnect(),
                Key::ArrowDown => self.splice_idx = (self.splice_idx + 1).min(self.splice_choices.len()),
                Key::ArrowUp => self.splice_idx = self.splice_idx.saturating_sub(1),
                _ => {}
            },
            SMode::Splice => match k {
                Key::Enter => self.confirm_splice(),
                Key::ArrowDown => self.splice_idx = (self.splice_idx + 1).min(self.splice_choices.len() + 1),
                Key::ArrowUp => self.splice_idx = self.splice_idx.saturating_sub(1),
                _ => {}
            },
            SMode::Connect(pi) => match k {
                Key::Enter => {
                    if let Some(p) = self.pick { if p != self.sel_id() { self.param = pi; self.set_input(Input::Link(p)); self.msg = "connected".into() } }
                    self.smode = SMode::Param;
                }
                Key::ArrowLeft => self.move_sel(-1, 0), Key::ArrowRight => self.move_sel(1, 0),
                Key::ArrowUp => self.move_sel(0, -1), Key::ArrowDown => self.move_sel(0, 1),
                _ => {}
            },
            SMode::Param => match k {
                Key::Enter => { if self.num.is_empty() { self.smode = SMode::Node } else { self.commit_num() } }
                Key::Backspace => { self.num.pop(); }
                Key::ArrowUp => self.param = self.param.saturating_sub(1),
                Key::ArrowDown => { let n = self.cur_node().map_or(0, |n| n.inputs.len()); self.param = (self.param + 1).min(n.saturating_sub(1)) }
                Key::ArrowLeft => self.adjust(false, false), Key::ArrowRight => self.adjust(true, false),
                _ => {}
            },
            SMode::Node => match k {
                Key::Enter => { if self.cur_node().map_or(0, |n| n.inputs.len()) > 0 { self.smode = SMode::Param; self.param = 0 } }
                Key::ArrowLeft => self.move_sel(-1, 0), Key::ArrowRight => self.move_sel(1, 0),
                Key::ArrowUp => self.move_sel(0, -1), Key::ArrowDown => self.move_sel(0, 1),
                Key::Delete | Key::Backspace => self.char_synth('x'),
                _ => {}
            },
        }
        self.clamp();
    }

    fn char_synth(&mut self, c: char) {
        match self.smode {
            SMode::Palette => {
                self.smode = SMode::Node;
                if c == 'v' {
                    self.edit();
                    let tid = self.song.tracks[self.track].id;
                    let vca = self.song.graph.voice(self.sel_node, tid);
                    if self.focus { self.marks.push(vca) }
                    self.sel_node = Some(vca);
                    self.sync();
                    self.msg = "voice built: ENV → OSC amp, OSC → × vel → × gain → OUT. play some notes on this track".into();
                    return;
                }
                if let Some((_, kind, _)) = graph::PALETTE.iter().find(|(k, _, _)| *k == c) {
                    let after = self.sel_node;
                    let cons = after.map(|a| self.song.graph.consumers(a)).unwrap_or_default();
                    if kind.chainable() && *kind != Kind::Key && cons.len() > 1 {
                        // ask which of the selected node's outputs the new node goes into
                        self.splice_kind = *kind;
                        self.splice_choices = cons;
                        self.splice_idx = 1;
                        self.smode = SMode::Splice;
                        self.msg = format!("{}: which connection of the selected node should it go into? j/k pick, enter confirms", kind.name());
                        return;
                    }
                    self.add_node(*kind, after, None);
                }
            }
            SMode::OutPick => match c {
                'h' => self.move_sel(-1, 0), 'l' => self.move_sel(1, 0), 'k' => self.move_sel(0, -1), 'j' => self.move_sel(0, 1),
                _ => {}
            },
            SMode::OutInput => match c {
                'j' => self.splice_idx = (self.splice_idx + 1).min(self.pick_inputs().saturating_sub(1)),
                'k' => self.splice_idx = self.splice_idx.saturating_sub(1),
                '1'..='9' => { let k = c as usize - '1' as usize; if k < self.pick_inputs() { self.splice_idx = k; self.confirm_out_input() } }
                _ => {}
            },
            SMode::Disconnect => match c {
                'j' => self.splice_idx = (self.splice_idx + 1).min(self.splice_choices.len()),
                'k' => self.splice_idx = self.splice_idx.saturating_sub(1),
                '1'..='9' => { let k = c as usize - '1' as usize; if k <= self.splice_choices.len() { self.splice_idx = k; self.confirm_disconnect() } }
                _ => {}
            },
            SMode::Splice => match c {
                'j' => self.splice_idx = (self.splice_idx + 1).min(self.splice_choices.len() + 1),
                'k' => self.splice_idx = self.splice_idx.saturating_sub(1),
                '1'..='9' => { let k = c as usize - '0' as usize; if k <= self.splice_choices.len() + 1 { self.splice_idx = k; self.confirm_splice() } }
                _ => {}
            },
            SMode::Connect(_) => match c {
                'h' => self.move_sel(-1, 0), 'l' => self.move_sel(1, 0), 'k' => self.move_sel(0, -1), 'j' => self.move_sel(0, 1),
                _ => {}
            },
            SMode::Param => match c {
                '0'..='9' | '.' | '-' | 'A'..='G' | '#' => self.num.push(c),
                'j' => { let n = self.cur_node().map_or(0, |n| n.inputs.len()); self.param = (self.param + 1).min(n.saturating_sub(1)) }
                'k' => self.param = self.param.saturating_sub(1),
                'h' => self.adjust(false, false), 'l' => self.adjust(true, false),
                'H' => self.adjust(false, true), 'L' => self.adjust(true, true),
                'x' => { let d = self.cur_node().map(|n| n.kind.defaults().get(self.param).copied().unwrap_or(0.0)).unwrap_or(0.0); self.set_input(Input::Const(d)); }
                'i' => { self.smode = SMode::Connect(self.param); self.pick = Some(self.sel_id()); self.msg = format!("input '{}' will READ the node you pick: h/j/k/l, enter", self.cur_node().map_or("", |n| n.kind.params()[self.param])) }
                'c' => { self.smode = SMode::OutPick; self.pick = Some(self.sel_id()); self.msg = "this node's OUTPUT goes to the node you pick: h/j/k/l, enter, then choose its input".into() }
                'w' => self.cycle_variant(1), 'W' => self.cycle_variant(-1),
                _ => {}
            },
            SMode::Node => match c {
                'h' => self.move_sel(-1, 0), 'l' => self.move_sel(1, 0), 'k' => self.move_sel(0, -1), 'j' => self.move_sel(0, 1),
                'a' => { self.smode = SMode::Palette; self.msg = format!("add node for track {}: press a letter", self.song.tracks[self.track].name) }
                'x' => {
                    let id = self.sel_id();
                    if self.cur_node().map_or(false, |n| n.kind == Kind::Out) { self.msg = "can't delete OUT".into(); return }
                    self.edit();
                    let next = self.nav(id, 1, 0).or_else(|| self.nav(id, -1, 0));
                    self.song.graph.remove(id);
                    self.sel_node = next;
                    self.clamp(); self.sync();
                }
                'w' => self.cycle_variant(1), 'W' => self.cycle_variant(-1),
                'v' => {
                    let id = self.sel_id();
                    if let Some(k) = self.marks.iter().position(|&m| m == id) { self.marks.remove(k); } else { self.marks.push(id) }
                    self.msg = format!("{} marked. f shows only marked nodes and what flows into/out of them", self.marks.len());
                }
                'f' => {
                    if self.marks.is_empty() { self.msg = "mark some nodes with v first".into() }
                    else { self.focus = !self.focus; self.msg = if self.focus { "focus on: showing marked nodes and their up/downstream".into() } else { "focus off".into() } }
                }
                'F' => { self.marks.clear(); self.focus = false; self.msg = "marks cleared".into() }
                'c' => { self.smode = SMode::OutPick; self.pick = Some(self.sel_id()); self.msg = "this node's OUTPUT goes to the node you pick: h/j/k/l, enter, then choose its input".into() }
                'i' => { if self.cur_node().map_or(0, |n| n.inputs.len()) > 0 { self.smode = SMode::Connect(0); self.param = 0; self.pick = Some(self.sel_id()); self.msg = "input mode: pick the node this input should read (h/j/k/l, enter). j/k first if you want a different input".into() } }
                'd' => {
                    let cons = self.song.graph.consumers(self.sel_id());
                    if cons.is_empty() { self.msg = "this node's output goes nowhere".into() }
                    else { self.splice_choices = cons; self.splice_idx = 0; self.smode = SMode::Disconnect; self.msg = "disconnect which connection? j/k or number, enter".into() }
                }
                'o' => {
                    let id = self.sel_id();
                    self.edit();
                    match self.song.graph.to_out(id) {
                        Some(n) if Some(n) == self.song.graph.out_id() => self.msg = "connected to OUT".into(),
                        Some(mix) => { if self.focus { self.marks.push(mix) } self.msg = "mixed into OUT through a new MATH +".into() }
                        None => self.msg = "that is OUT".into(),
                    }
                    self.sync();
                }
                '[' => self.mv_track(-1), ']' => self.mv_track(1),
                'u' => self.undo(),
                ':' => { self.cmd = Some(String::new()); self.msg = CMDS.into() }
                _ => {}
            },
        }
        self.clamp();
    }

    // ---------- drawing ----------
    fn draw_header(&self, g: &Grid, view: &str) {
        let pos = self.playhead();
        let file = self.path.as_ref().and_then(|p| p.file_name()).map_or("untitled".to_string(), |f| f.to_string_lossy().to_string());
        let bt = self.song.beats as f64;
        g.text(0.0, 0.0, &format!(
            "DIRTY DAW  {view}   {} bpm  {}/4  grid {}  swing {:.2}   {}   prog {}   {} bars   {} {}.{}   {}",
            self.song.bpm, self.song.beats, self.song.grid, self.song.swing, self.song.key_name(),
            self.song.prog.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(" "),
            self.song.bars, if self.playing { "▶" } else { "■" }, (pos / bt) as usize + 1, ((pos % bt) as usize) + 1, file,
        ), Color32::from_gray(220));
    }

    fn draw_arrange(&mut self, g: &Grid) {
        self.draw_header(g, "ARRANGE");
        let dim = Color32::from_gray(90);
        let steps = self.song.steps();
        let bs = self.song.bar_steps();
        let sw = g.cw;
        let left = g.pos(11.0, 0.0).x;
        let right = g.rect.right() - 10.0;
        let vis = (((right - left) / sw) as usize).max(4);
        if self.cursor < self.view_start { self.view_start = self.cursor }
        if self.cursor >= self.view_start + vis { self.view_start = self.cursor + 1 - vis }
        let vs = self.view_start;
        let end = (vs + vis).min(steps);
        let x_of = |s: f64| left + (s - vs as f64) as f32 * sw;
        let top = g.pos(0.0, 3.0).y;
        let rh = g.ch * 2.0;
        let ntr = self.song.tracks.len();
        let bottom = top + ntr as f32 * rh;
        g.text(0.0, 1.0, &format!("{} tracks. enter opens the pattern under the cursor (or creates one).", ntr), dim);
        for s in vs..end {
            let x = x_of(s as f64);
            if s % bs == 0 {
                g.c.line_segment([Pos2::new(x, top - 4.0), Pos2::new(x, bottom)], Stroke::new(1.0_f32, Color32::from_gray(70)));
                g.c.text(Pos2::new(x + 2.0, g.pos(0.0, 2.0).y), Align2::LEFT_TOP, format!("{}", s / bs + 1), g.font.clone(), dim);
            } else if s % self.song.grid as usize == 0 {
                g.c.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], Stroke::new(1.0_f32, Color32::from_gray(35)));
            }
        }
        let pos = self.playhead();
        for (t, tr) in self.song.tracks.iter().enumerate() {
            let y = top + t as f32 * rh;
            let color = tcolor(t);
            let row = Rect::from_min_max(Pos2::new(left, y), Pos2::new(x_of(end as f64), y + rh - 2.0));
            g.c.rect_filled(row, 0.0, if t == self.track { Color32::from_gray(28) } else { Color32::from_gray(22) });
            let name = format!("{}{}", tr.name.chars().take(8).collect::<String>(), if tr.mute { " m" } else { "" });
            g.c.text(Pos2::new(g.rect.left() + 10.0, y + 2.0), Align2::LEFT_TOP, name, g.font.clone(), if tr.mute { dim } else { color });
            let lvl = self.track_meters.get(t).copied().unwrap_or(0.0).min(1.0);
            g.c.rect_filled(Rect::from_min_size(Pos2::new(g.rect.left() + 10.0, y + g.ch + 4.0), Vec2::new(9.0 * g.cw * lvl, 6.0)), 0.0, color.gamma_multiply(0.8));
            for (i, p) in tr.patterns.iter().enumerate() {
                for k in 0..p.reps.max(1) {
                    let (a, b) = (p.start + k * p.len, p.start + (k + 1) * p.len);
                    if b <= vs || a >= end { continue }
                    let r = Rect::from_min_max(Pos2::new(x_of(a as f64) + 1.0, y + 2.0), Pos2::new(x_of(b as f64) - 1.0, y + rh - 4.0));
                    let hot = self.playing && self.pat.is_none() && pos >= a as f64 * self.song.sb() && pos < b as f64 * self.song.sb();
                    if k == 0 {
                        g.c.rect_filled(r, 3.0_f32, color.gamma_multiply(if tr.mute { 0.25 } else { 0.55 }));
                        g.c.text(r.min + Vec2::new(4.0, 2.0), Align2::LEFT_TOP, format!("{} ♪{}", i + 1, p.notes.len()), g.font.clone(), Color32::BLACK);
                    } else {
                        g.c.rect_filled(r, 3.0_f32, color.gamma_multiply(0.15));
                        g.c.rect_stroke(r, 3.0_f32, Stroke::new(1.0_f32, color.gamma_multiply(0.6)));
                        g.c.text(r.min + Vec2::new(4.0, 2.0), Align2::LEFT_TOP, format!("↻{}", k), g.font.clone(), color.gamma_multiply(0.8));
                    }
                    if hot { g.c.rect_stroke(r, 3.0_f32, Stroke::new(1.5_f32, Color32::WHITE)); }
                }
            }
            // note density strip inside the base block
            for p in &tr.patterns {
                let ys = y + g.ch + 4.0;
                for n in &p.notes {
                    let s = p.start as f64 + n.start / self.song.sb();
                    if s < vs as f64 || s >= end as f64 { continue }
                    let h = ((n.pitch - 36).clamp(0, 60) as f32 / 60.0) * (g.ch - 10.0);
                    g.c.rect_filled(Rect::from_min_size(Pos2::new(x_of(s), ys + (g.ch - 10.0) - h), Vec2::new((n.len / self.song.sb()) as f32 * sw - 1.0, 2.0)), 0.0, Color32::from_gray(10));
                }
            }
        }
        if let Some(s) = self.sel {
            let (a, b) = (s.min(self.cursor), s.max(self.cursor) + 1);
            g.c.rect_filled(Rect::from_min_max(Pos2::new(x_of(a as f64), top), Pos2::new(x_of(b as f64), bottom)), 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 22));
        }
        if self.pat.is_none() {
            let px = x_of(pos / self.song.sb());
            if px >= left && px <= x_of(end as f64) { g.c.line_segment([Pos2::new(px, top - 6.0), Pos2::new(px, bottom)], Stroke::new(2.0_f32, Color32::from_rgb(90, 230, 120))); }
        }
        let cr = Rect::from_min_size(Pos2::new(x_of(self.cursor as f64), top + self.track as f32 * rh), Vec2::new(sw, rh - 2.0));
        g.c.rect_stroke(cr, 1.0_f32, Stroke::new(1.5_f32, Color32::WHITE));
        let r = g.rows() as f32;
        g.text(0.0, r - 3.0, HELP_A1, dim);
        g.text(0.0, r - 2.0, HELP_A2, dim);
        self.draw_status(g);
    }

    fn draw_pattern(&mut self, g: &Grid) {
        self.draw_header(g, "PATTERN");
        let Some(pi) = self.pat else { return };
        let dim = Color32::from_gray(90);
        let tr = &self.song.tracks[self.track];
        let pat = &tr.patterns[pi];
        let sb = self.song.sb();
        let bs = self.song.bar_steps();
        let steps = pat.len;
        let sw = 2.0 * g.cw;
        let rh = 13.0;
        let left = g.pos(5.0, 0.0).x;
        let right = g.rect.right() - 10.0;
        let top = g.pos(0.0, 3.5).y;
        let bottom = g.pos(0.0, g.rows() as f32 - 3.0).y - 6.0;
        let vis = (((right - left) / sw) as usize).max(4);
        if self.pcursor < self.pview_start { self.pview_start = self.pcursor }
        if self.pcursor >= self.pview_start + vis { self.pview_start = self.pcursor + 1 - vis }
        let nrows = (((bottom - top) / rh) as usize).max(4) as i32;
        if self.cpitch > self.view_top { self.view_top = self.cpitch }
        if self.cpitch <= self.view_top - nrows { self.view_top = self.cpitch + nrows - 1 }
        let (vs, vt) = (self.pview_start, self.view_top);
        let end = (vs + vis).min(steps);
        let x_of = |s: f64| left + (s - vs as f64) as f32 * sw;
        let y_of = |p: i32| top + (vt - p) as f32 * rh;
        let color = tcolor(self.track);
        g.text(0.0, 1.0, &format!("pattern {} of {}   {} steps   plays {}x   looping this pattern only.  esc = back to arrangement", pi + 1, tr.name, pat.len, pat.reps), dim);
        for r in 0..nrows {
            let p = vt - r;
            if p < 0 { break }
            let y = y_of(p);
            let bg = if self.song.in_scale(p) { Color32::from_gray(30) } else { Color32::from_gray(20) };
            g.c.rect_filled(Rect::from_min_max(Pos2::new(left, y), Pos2::new(x_of(end as f64), y + rh - 1.0)), 0.0, bg);
            if p.rem_euclid(12) == self.song.root { g.c.line_segment([Pos2::new(left, y), Pos2::new(x_of(end as f64), y)], Stroke::new(1.0_f32, Color32::from_gray(60))); }
            let lc = if p == self.cpitch { Color32::WHITE } else if self.song.in_scale(p) { Color32::from_gray(140) } else { Color32::from_gray(70) };
            g.c.text(Pos2::new(g.rect.left() + 12.0, y), Align2::LEFT_TOP, note_name(p), FontId::monospace(10.0), lc);
        }
        for s in vs..end {
            let x = x_of(s as f64);
            let abs = pat.start + s;
            if abs % bs == 0 {
                g.c.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], Stroke::new(1.0_f32, Color32::from_gray(80)));
                g.c.text(Pos2::new(x + 2.0, g.pos(0.0, 2.0).y), Align2::LEFT_TOP, format!("bar {} (chord {})", abs / bs + 1, self.song.bar_chord(abs / bs) + 1), g.font.clone(), dim);
            } else if abs % self.song.grid as usize == 0 {
                g.c.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], Stroke::new(1.0_f32, Color32::from_gray(45)));
            }
        }
        if let Some(s) = self.sel {
            let (a, b) = (s.min(self.pcursor), s.max(self.pcursor) + 1);
            g.c.rect_filled(Rect::from_min_max(Pos2::new(x_of(a as f64), top), Pos2::new(x_of(b as f64), bottom)), 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 22));
        }
        let pos = self.playhead();
        for n in &pat.notes {
            let s = n.start / sb;
            if s + n.len / sb < vs as f64 || s > end as f64 || n.pitch > vt || n.pitch <= vt - nrows { continue }
            let r = Rect::from_min_size(Pos2::new(x_of(s), y_of(n.pitch) + 1.0), Vec2::new(((n.len / sb) as f32 * sw - 2.0).max(3.0), rh - 3.0));
            g.c.rect_filled(r, 2.0_f32, color.gamma_multiply(0.35 + 0.65 * n.vel));
            if self.playing && pos >= n.start && pos < n.start + n.len { g.c.rect_stroke(r.expand(1.0), 2.0_f32, Stroke::new(1.5_f32, Color32::WHITE)); }
        }
        let px = x_of(pos / sb);
        if px >= left && px <= x_of(end as f64) { g.c.line_segment([Pos2::new(px, top - 4.0), Pos2::new(px, bottom)], Stroke::new(2.0_f32, Color32::from_rgb(90, 230, 120))); }
        let cr = Rect::from_min_size(Pos2::new(x_of(self.pcursor as f64), y_of(self.cpitch)), Vec2::new(sw * self.ins_len as f32 - 1.0, rh - 1.0));
        g.c.rect_stroke(cr, 1.0_f32, Stroke::new(1.5_f32, Color32::WHITE));
        let r = g.rows() as f32;
        g.text(0.0, r - 3.0, HELP_P1, dim);
        g.text(0.0, r - 2.0, HELP_P2, dim);
        if let Some(p) = self.pending { g.text(0.0, r - 1.0, &format!("{p}? (degree 1-7)"), Color32::from_rgb(255, 205, 80)); return }
        self.draw_status(g);
    }

    fn draw_synth(&mut self, g: &Grid) {
        self.draw_header(g, "SYNTH");
        let dim = Color32::from_gray(90);
        let sel = self.sel_id();
        let gr = &self.song.graph;
        let ids = self.song.track_ids();
        let vis = self.visible();
        let lay = self.layout();
        let n = gr.nodes.len();
        let ncols = lay.iter().filter(|p| p.0 != usize::MAX).map(|p| p.0).max().map_or(1, |m| m + 1);
        let tname = |tid: u32| self.song.tracks.iter().position(|t| t.id == tid);
        let shown = vis.iter().filter(|&&v| v).count();
        let focus_txt = if self.focus && !self.marks.is_empty() { format!("FOCUS: {} of {} nodes (f to show all)", shown, n) } else if self.marks.is_empty() { "v marks nodes, f focuses on them".to_string() } else { format!("{} marked (f to focus)", self.marks.len()) };
        let _ = &ids;
        g.text(0.0, 1.0, &format!("track: {}  |  {}  |  F12 manual", self.song.tracks[self.track].name, focus_txt), dim);
        let mut pos = vec![(0.0f32, 0.0f32); n];
        let mut heights = vec![0.0f32; n];
        for c in 0..ncols {
            let mut y = 3.0;
            let mut col: Vec<usize> = (0..n).filter(|&i| lay[i].0 == c && vis[i]).collect();
            col.sort_by_key(|&i| lay[i].1);
            for i in col {
                let h = 2.0 + gr.nodes[i].inputs.len() as f32;
                pos[i] = (c as f32 * (NODE_W + NODE_GAP), y);
                heights[i] = h;
                y += h + 1.0;
            }
        }
        if let Some(si) = gr.idx(sel) {
            let (cols, rows) = (g.cols() as f32, g.rows() as f32 - 3.0);
            let (x, y) = (pos[si].0 - self.gscroll.x, pos[si].1 - self.gscroll.y);
            if x < 0.0 { self.gscroll.x = pos[si].0 }
            if x + NODE_W > cols { self.gscroll.x = pos[si].0 + NODE_W - cols }
            if y < 3.0 { self.gscroll.y = pos[si].1 - 3.0 }
            if y + heights[si] > rows { self.gscroll.y = pos[si].1 + heights[si] - rows }
        }
        let sc = self.gscroll;
        let bx = |i: usize| g.pos(pos[i].0 - sc.x, pos[i].1 - sc.y);
        let node_color = |nd: &graph::Node| if nd.kind == Kind::Trk { tname(nd.track).map_or(dim, tcolor) } else { kind_color(nd.kind) };
        let clip_top = g.pos(0.0, 2.0).y;
        let clip_bottom = g.pos(0.0, g.rows() as f32 - 2.0).y;
        for (i, node) in gr.nodes.iter().enumerate() {
            if !vis[i] { continue }
            for (pi, inp) in node.inputs.iter().enumerate() {
                let Input::Link(id) = inp else { continue };
                let Some(j) = gr.idx(*id) else { continue };
                if !vis[j] { continue }
                let from = bx(j) + Vec2::new(NODE_W * g.cw, (heights[j] - 0.5) * g.ch);
                let to = bx(i) + Vec2::new(0.0, (1.5 + pi as f32) * g.ch);
                if (from.y < clip_top && to.y < clip_top) || (from.y > clip_bottom && to.y > clip_bottom) { continue }
                let back = lay[j].0 >= lay[i].0;
                let lvl = self.node_meter(j);
                let color = if back { Color32::from_rgb(255, 150, 60) } else { node_color(&gr.nodes[j]) }.gamma_multiply(0.3 + 0.7 * lvl);
                let mut stroke = Stroke::new(1.0 + 2.0 * lvl, color);
                if self.smode == SMode::Splice && gr.nodes[j].id == sel {
                    let chosen = match self.splice_idx { 0 => true, k if k <= self.splice_choices.len() => self.splice_choices[k - 1] == (node.id, pi), _ => false };
                    stroke = if chosen { Stroke::new(3.0_f32, Color32::WHITE) } else { Stroke::new(1.0_f32, Color32::from_gray(60)) };
                }
                if back {
                    let dx = Vec2::new(g.cw * 1.5, 0.0);
                    let ty = bx(j).y.min(bx(i).y) - g.ch * 0.5;
                    let pts = [from, from + dx, Pos2::new(from.x + dx.x, ty), Pos2::new(to.x - dx.x, ty), to - dx, to];
                    for w in pts.windows(2) { g.c.line_segment([w[0], w[1]], stroke); }
                } else {
                    let mx = (from.x + to.x) * 0.5;
                    let pts = [from, Pos2::new(mx, from.y), Pos2::new(mx, to.y), to];
                    for w in pts.windows(2) { g.c.line_segment([w[0], w[1]], stroke); }
                }
            }
        }
        // preview of the connection being made
        if let (Some(si), Some(pi)) = (gr.idx(sel), self.pick.and_then(|p| gr.idx(p))) {
            if pi != si && vis[pi] {
                let (src, dst, row) = match self.smode {
                    SMode::OutPick => (si, pi, None),
                    SMode::OutInput => (si, pi, Some(self.splice_idx)),
                    SMode::Connect(k) => (pi, si, Some(k)),
                    _ => (si, si, None),
                };
                if src != dst {
                    let from = bx(src) + Vec2::new(NODE_W * g.cw, (heights[src] - 0.5) * g.ch);
                    let to = bx(dst) + Vec2::new(0.0, row.map_or(0.5, |k| 1.5 + k as f32) * g.ch);
                    let stroke = Stroke::new(2.5_f32, Color32::WHITE);
                    let mx = (from.x + to.x) * 0.5;
                    for w in [from, Pos2::new(mx, from.y), Pos2::new(mx, to.y), to].windows(2) { g.c.line_segment([w[0], w[1]], stroke); }
                    g.c.circle_filled(to, 5.0, Color32::WHITE);
                    let label = format!("{} ▸ {}{}", gr.nodes[src].short(), gr.nodes[dst].short(), row.map_or(String::new(), |k| format!(".{}", gr.nodes[dst].kind.params().get(k).copied().unwrap_or(""))));
                    g.c.text(Pos2::new(mx, (from.y + to.y) * 0.5 - g.ch), Align2::CENTER_CENTER, label, g.font.clone(), Color32::WHITE);
                }
            }
        }
        for (i, node) in gr.nodes.iter().enumerate() {
            if !vis[i] { continue }
            let r = Rect::from_min_size(bx(i), Vec2::new(NODE_W * g.cw, heights[i] * g.ch));
            if r.right() < g.rect.left() || r.left() > g.rect.right() || r.bottom() < clip_top || r.top() > clip_bottom { continue }
            let is_sel = node.id == sel;
            let is_pick = matches!(self.smode, SMode::Connect(_)) && self.pick == Some(node.id);
            g.c.rect_filled(r, 3.0_f32, if is_sel { Color32::from_gray(48) } else { Color32::from_gray(30) });
            let is_pick = is_pick || (matches!(self.smode, SMode::OutPick | SMode::OutInput) && self.pick == Some(node.id));
            let stroke = if is_pick { Stroke::new(2.0_f32, Color32::from_rgb(255, 205, 80)) } else if is_sel { Stroke::new(1.5_f32, Color32::WHITE) } else { Stroke::new(1.0_f32, Color32::from_gray(70)) };
            g.c.rect_stroke(r, 3.0_f32, stroke);
            let kc = node_color(node);
            let title = if node.kind == Kind::Trk {
                let tn = tname(node.track).map_or("?".to_string(), |t| self.song.tracks[t].name.clone());
                format!("TRK {} {}", tn.chars().take(6).collect::<String>(), node.kind.variants()[node.var % 4])
            } else { node.title() };
            g.c.text(r.min + Vec2::new(4.0, 2.0), Align2::LEFT_TOP, title, g.font.clone(), kc);
            if self.marks.contains(&node.id) { g.c.circle_filled(Pos2::new(r.max.x - 8.0, r.min.y + 9.0), 4.0, Color32::from_rgb(255, 205, 80)); }
            for (pi, inp) in node.inputs.iter().enumerate() {
                let y = r.min.y + (1.0 + pi as f32) * g.ch;
                let editing = (is_sel && matches!(self.smode, SMode::Param | SMode::Connect(_)) && self.param == pi)
                    || (self.smode == SMode::OutInput && self.pick == Some(node.id) && self.splice_idx == pi);
                g.c.circle_filled(Pos2::new(r.min.x, y + g.ch * 0.5), 3.5, if matches!(inp, Input::Link(_)) { Color32::from_gray(200) } else { Color32::from_gray(60) });
                if editing { g.c.rect_filled(Rect::from_min_size(Pos2::new(r.min.x + 2.0, y), Vec2::new(r.width() - 4.0, g.ch - 2.0)), 2.0_f32, Color32::from_gray(75)); }
                let name = node.kind.params()[pi];
                let val = match inp {
                    _ if editing && !self.num.is_empty() => format!("{}▏", self.num),
                    _ if node.kind == Kind::Trk => tname(node.track).map_or("?".into(), |t| self.song.tracks[t].name.clone()),
                    Input::Const(v) if node.kind == Kind::Key && pi == 2 => format!("{} ({})", note_name(*v as i32), fmt_num(*v)),
                    Input::Const(v) => fmt_num(*v),
                    Input::Link(id) => gr.get(*id).map_or("?".into(), |s| format!("{} {}", if gr.idx(*id).map_or(false, |j| vis[j]) { "◂" } else { "◂(hidden)" }, s.short())),
                };
                let vc = match inp { Input::Link(id) => gr.get(*id).map_or(dim, |s| node_color(s)), _ => Color32::from_gray(210) };
                g.c.text(Pos2::new(r.min.x + 4.0, y), Align2::LEFT_TOP, format!("{name:<5}"), g.font.clone(), dim);
                g.c.text(Pos2::new(r.min.x + 4.0 + 5.0 * g.cw, y), Align2::LEFT_TOP, val, g.font.clone(), vc);
            }
            // output row: where this node's signal goes, plus the output port and a level bar
            let oy = r.max.y - g.ch;
            let cons = gr.consumers(node.id);
            let dest = if cons.is_empty() { "nowhere".to_string() } else {
                cons.iter().take(3).map(|(t, k)| gr.get(*t).map_or("?".into(), |n| format!("{}.{}", n.short(), n.kind.params()[*k]))).collect::<Vec<_>>().join(" ")
                    + if cons.len() > 3 { " …" } else { "" }
            };
            let oc = if cons.is_empty() && node.kind != Kind::Out { Color32::from_rgb(255, 140, 80) } else { dim };
            g.c.text(Pos2::new(r.min.x + 4.0, oy), Align2::LEFT_TOP, if node.kind == Kind::Out { "▸ speakers" } else { "out ▸" }, g.font.clone(), oc);
            if node.kind != Kind::Out { g.c.text(Pos2::new(r.min.x + 4.0 + 6.0 * g.cw, oy), Align2::LEFT_TOP, dest, FontId::monospace(12.0), if cons.is_empty() { oc } else { Color32::from_gray(170) }); }
            g.c.circle_filled(Pos2::new(r.max.x, oy + g.ch * 0.5), 3.5, if cons.is_empty() { Color32::from_gray(60) } else { kc });
            let lvl = self.node_meter(i);
            g.c.rect_filled(Rect::from_min_size(Pos2::new(r.min.x + 2.0, r.max.y - 4.0), Vec2::new(r.width() - 4.0, 3.0)), 0.0, Color32::from_gray(22));
            g.c.rect_filled(Rect::from_min_size(Pos2::new(r.min.x + 2.0, r.max.y - 4.0), Vec2::new((r.width() - 4.0) * lvl, 3.0)), 0.0, kc.gamma_multiply(0.9));
        }
        if self.smode == SMode::Palette {
            let w = 60.0;
            let h = graph::PALETTE.len() as f32 + 2.0;
            let r = Rect::from_min_size(g.pos((g.cols() as f32 - w) * 0.5, 5.0), Vec2::new(w * g.cw, h * g.ch));
            g.c.rect_filled(r, 4.0_f32, Color32::from_gray(24));
            g.c.rect_stroke(r, 4.0_f32, Stroke::new(1.5_f32, Color32::from_gray(160)));
            g.c.text(r.min + Vec2::new(8.0, 4.0), Align2::LEFT_TOP, "add node (chainable nodes splice after the selected one; osc/env/key auto-wire to the current track)", g.font.clone(), Color32::from_gray(200));
            for (k, (key, kind, desc)) in graph::PALETTE.iter().enumerate() {
                let y = r.min.y + (1.5 + k as f32) * g.ch;
                g.c.text(Pos2::new(r.min.x + 8.0, y), Align2::LEFT_TOP, format!("{key}"), g.font.clone(), Color32::from_rgb(255, 205, 80));
                g.c.text(Pos2::new(r.min.x + 8.0 + 3.0 * g.cw, y), Align2::LEFT_TOP, format!("{:<6} {}", kind.name(), desc), g.font.clone(), kind_color(*kind));
            }
        }
        if self.smode == SMode::Splice {
            let w = 56.0;
            let h = self.splice_choices.len() as f32 + 4.0;
            let r = Rect::from_min_size(g.pos((g.cols() as f32 - w) * 0.5, 5.0), Vec2::new(w * g.cw, h * g.ch));
            g.c.rect_filled(r, 4.0_f32, Color32::from_gray(24));
            g.c.rect_stroke(r, 4.0_f32, Stroke::new(1.5_f32, Color32::from_gray(160)));
            let selname = gr.get(sel).map_or(String::new(), |n| n.short());
            g.c.text(r.min + Vec2::new(8.0, 4.0), Align2::LEFT_TOP, format!("insert {} into which output of {}?", self.splice_kind.name(), selname), g.font.clone(), Color32::from_gray(200));
            let mut rows: Vec<String> = vec![format!("all {} connections", self.splice_choices.len())];
            for (t, k) in &self.splice_choices {
                let nd = gr.get(*t);
                rows.push(format!("{} . {}", nd.map_or("?".into(), |n| n.title()), nd.map_or("?", |n| n.kind.params()[*k])));
            }
            rows.push("none: only connect the input, I'll wire the output myself".into());
            for (k, row) in rows.iter().enumerate() {
                let y = r.min.y + (1.5 + k as f32) * g.ch;
                let on = k == self.splice_idx;
                if on { g.c.rect_filled(Rect::from_min_size(Pos2::new(r.min.x + 4.0, y - 1.0), Vec2::new(r.width() - 8.0, g.ch)), 2.0_f32, Color32::from_gray(60)); }
                g.c.text(Pos2::new(r.min.x + 8.0, y), Align2::LEFT_TOP, format!("{}", k + 1), g.font.clone(), Color32::from_rgb(255, 205, 80));
                g.c.text(Pos2::new(r.min.x + 8.0 + 3.0 * g.cw, y), Align2::LEFT_TOP, row, g.font.clone(), if on { Color32::WHITE } else { Color32::from_gray(190) });
            }
        }
        if self.smode == SMode::OutInput {
            if let Some(t) = self.pick.and_then(|p| gr.get(p)) {
                let w = 56.0;
                let h = t.inputs.len() as f32 + 3.0;
                let r = Rect::from_min_size(g.pos((g.cols() as f32 - w) * 0.5, 5.0), Vec2::new(w * g.cw, h * g.ch));
                g.c.rect_filled(r, 4.0_f32, Color32::from_gray(24));
                g.c.rect_stroke(r, 4.0_f32, Stroke::new(1.5_f32, Color32::from_gray(160)));
                let selname = gr.get(sel).map_or(String::new(), |n| n.short());
                g.c.text(r.min + Vec2::new(8.0, 4.0), Align2::LEFT_TOP, format!("{} output ▸ which input of {}?", selname, t.title()), g.font.clone(), Color32::from_gray(200));
                for (k, inp) in t.inputs.iter().enumerate() {
                    let y = r.min.y + (1.5 + k as f32) * g.ch;
                    let on = k == self.splice_idx;
                    if on { g.c.rect_filled(Rect::from_min_size(Pos2::new(r.min.x + 4.0, y - 1.0), Vec2::new(r.width() - 8.0, g.ch)), 2.0_f32, Color32::from_gray(60)); }
                    let cur = match inp { Input::Const(v) => format!("currently {}", fmt_num(*v)), Input::Link(id) => format!("currently reads {}", gr.get(*id).map_or("?".into(), |n| n.short())) };
                    g.c.text(Pos2::new(r.min.x + 8.0, y), Align2::LEFT_TOP, format!("{}", k + 1), g.font.clone(), Color32::from_rgb(255, 205, 80));
                    g.c.text(Pos2::new(r.min.x + 8.0 + 3.0 * g.cw, y), Align2::LEFT_TOP, format!("{:<6} {}", t.kind.params()[k], cur), g.font.clone(), if on { Color32::WHITE } else { Color32::from_gray(190) });
                }
            }
        }
        if self.smode == SMode::Disconnect {
            let w = 56.0;
            let h = self.splice_choices.len() as f32 + 4.0;
            let r = Rect::from_min_size(g.pos((g.cols() as f32 - w) * 0.5, 5.0), Vec2::new(w * g.cw, h * g.ch));
            g.c.rect_filled(r, 4.0_f32, Color32::from_gray(24));
            g.c.rect_stroke(r, 4.0_f32, Stroke::new(1.5_f32, Color32::from_gray(160)));
            let selname = gr.get(sel).map_or(String::new(), |n| n.short());
            g.c.text(r.min + Vec2::new(8.0, 4.0), Align2::LEFT_TOP, format!("disconnect the output of {} from:", selname), g.font.clone(), Color32::from_gray(200));
            let mut rows: Vec<String> = self.splice_choices.iter().map(|(t, k)| gr.get(*t).map_or("?".into(), |n| format!("{} . {}", n.title(), n.kind.params()[*k]))).collect();
            rows.push("all of them".into());
            for (k, row) in rows.iter().enumerate() {
                let y = r.min.y + (1.5 + k as f32) * g.ch;
                let on = k == self.splice_idx;
                if on { g.c.rect_filled(Rect::from_min_size(Pos2::new(r.min.x + 4.0, y - 1.0), Vec2::new(r.width() - 8.0, g.ch)), 2.0_f32, Color32::from_gray(60)); }
                g.c.text(Pos2::new(r.min.x + 8.0, y), Align2::LEFT_TOP, format!("{}", k + 1), g.font.clone(), Color32::from_rgb(255, 205, 80));
                g.c.text(Pos2::new(r.min.x + 8.0 + 3.0 * g.cw, y), Align2::LEFT_TOP, row, g.font.clone(), if on { Color32::WHITE } else { Color32::from_gray(190) });
            }
        }
        let rr = g.rows() as f32;
        let help = match self.smode {
            SMode::Node | SMode::Palette => HELP_NODE, SMode::Param => HELP_PARAM, SMode::Connect(_) => HELP_CONNECT,
            SMode::Splice => "j/k or 1-9 choose the connection  enter confirm  esc cancel",
            SMode::OutPick => "OUTPUT ▸ move the yellow box onto the node that should receive this output (h/j/k/l)  enter  esc cancel",
            SMode::OutInput => "OUTPUT ▸ j/k or 1-9 choose which input of the yellow node reads this output  enter  esc cancel",
            SMode::Disconnect => "j/k or 1-9 choose which connection to cut  enter  esc cancel",
        };
        g.text(0.0, rr - 2.0, help, dim);
        self.draw_status(g);
    }

    fn draw_status(&self, g: &Grid) {
        let r = g.rows() as f32 - 1.0;
        let y = g.pos(0.0, r - 2.0).y - 3.0;
        g.p.line_segment([Pos2::new(g.rect.left(), y), Pos2::new(g.rect.right(), y)], Stroke::new(1.0_f32, Color32::from_gray(50)));
        let y = g.pos(0.0, 2.0).y - 2.0;
        g.p.line_segment([Pos2::new(g.rect.left(), y), Pos2::new(g.rect.right(), y)], Stroke::new(1.0_f32, Color32::from_gray(50)));
        match &self.cmd {
            Some(c) => g.text(0.0, r, &format!(":{c}▏     (enter runs, esc cancels)"), Color32::from_rgb(255, 205, 80)),
            None => g.text(0.0, r, &self.msg, Color32::from_gray(160)),
        }
    }
}

struct Grid { p: egui::Painter, c: egui::Painter, font: FontId, cw: f32, ch: f32, rect: Rect }

impl Grid {
    fn pos(&self, col: f32, row: f32) -> Pos2 { Pos2::new(self.rect.left() + 10.0 + col * self.cw, self.rect.top() + 8.0 + row * self.ch) }
    fn text(&self, col: f32, row: f32, s: &str, c: Color32) { self.p.text(self.pos(col, row), Align2::LEFT_TOP, s, self.font.clone(), c); }
    fn cols(&self) -> usize { ((self.rect.width() - 20.0) / self.cw) as usize }
    fn rows(&self) -> usize { ((self.rect.height() - 16.0) / self.ch) as usize }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.handle(ctx);
        self.poll_meters();
        let bg = Color32::from_rgb(16, 16, 20);
        egui::CentralPanel::default().frame(egui::Frame::none().fill(bg)).show(ctx, |ui| {
            let font = FontId::monospace(14.0);
            let cw = ctx.fonts(|f| f.glyph_width(&font, 'M'));
            let rect = ui.max_rect();
            let ch = 21.0;
            let rows = ((rect.height() - 16.0) / ch) as usize;
            let content = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 8.0 + 2.0 * ch), Pos2::new(rect.right(), rect.top() + 8.0 + (rows as f32 - 3.0) * ch - 2.0));
            let g = Grid { p: ui.painter().clone(), c: ui.painter().with_clip_rect(content), font, cw, ch, rect };
            match self.view { View::Arrange => self.draw_arrange(&g), View::Pattern => self.draw_pattern(&g), View::Synth => self.draw_synth(&g) }
        });
        ctx.request_repaint();
    }
}
