//! One synth graph shared by the whole song. Tracks appear as TRK source nodes
//! (freq/gate/vel/note of the notes on that track). Any topology is allowed, including
//! cycles (feedback). Each track gets its own pruned program: nodes that depend only on
//! other tracks are left out, so a voice of track A never computes track B's chain.
use crate::dsp::{self, Op, Program};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
pub enum Kind { Trk, Osc, Noise, Env, Filt, Delay, Math, Shape, Sh, Buf, Key, Out }

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
pub enum Input { Const(f32), Link(u32) }

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Node {
    pub id: u32,
    pub kind: Kind,
    pub var: usize,
    pub inputs: Vec<Input>,
    #[serde(default)]
    pub track: u32, // TRK only: which track feeds this node
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct Graph { pub nodes: Vec<Node>, pub next_id: u32 }

pub const ALL: [Kind; 12] = [Kind::Trk, Kind::Osc, Kind::Noise, Kind::Env, Kind::Filt, Kind::Delay, Kind::Math, Kind::Shape, Kind::Sh, Kind::Buf, Kind::Key, Kind::Out];

pub const PALETTE: &[(char, Kind, &str)] = &[
    ('t', Kind::Trk, "track input  freq/gate/vel/note of the current track"),
    ('o', Kind::Osc, "oscillator  sin/saw/sqr/tri/pulse/phase  (amp input = volume, put an ENV there)"),
    ('n', Kind::Noise, "noise"),
    ('e', Kind::Env, "envelope  gate a d s r"),
    ('f', Kind::Filt, "filter  lp/hp/bp"),
    ('d', Kind::Delay, "delay line (feedback ok)"),
    ('m', Kind::Math, "math  + - * / min max pow > <"),
    ('s', Kind::Shape, "shaper  tanh/clip/abs/sqrt/neg"),
    ('h', Kind::Sh, "sample & hold"),
    ('b', Kind::Buf, "buffer  write/read at any position (wavetable, granular, looper)"),
    ('k', Kind::Key, "key gate  passes gate only when note == key (drum routing)"),
    ('v', Kind::Out, "VOICE recipe: OSC × ENV → OUT for this track (reuses the selected OSC or ENV)"),
];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Trk => "TRK", Kind::Osc => "OSC", Kind::Noise => "NOISE", Kind::Env => "ENV", Kind::Filt => "FILT", Kind::Delay => "DELAY",
            Kind::Math => "MATH", Kind::Shape => "SHAPE", Kind::Sh => "S&H", Kind::Buf => "BUF", Kind::Key => "KEY", Kind::Out => "OUT",
        }
    }
    pub fn params(self) -> &'static [&'static str] {
        match self {
            Kind::Trk => &["track"], Kind::Osc => &["freq", "pw", "amp"], Kind::Noise => &["amp"], Kind::Env => &["gate", "att", "dec", "sus", "rel"],
            Kind::Filt => &["in", "cut", "q"], Kind::Delay => &["in", "time"], Kind::Math => &["a", "b"],
            Kind::Shape => &["x"], Kind::Sh => &["x", "trig"], Kind::Buf => &["in", "wpos", "rpos", "rec", "size"],
            Kind::Key => &["gate", "note", "key"], Kind::Out => &["in"],
        }
    }
    pub fn defaults(self) -> &'static [f32] {
        match self {
            Kind::Trk => &[0.0], Kind::Osc => &[440.0, 0.5, 1.0], Kind::Noise => &[1.0], Kind::Env => &[0.0, 0.01, 0.2, 0.6, 0.2], Kind::Filt => &[0.0, 1000.0, 0.7],
            Kind::Delay => &[0.0, 0.25], Kind::Math => &[0.0, 1.0], Kind::Shape => &[0.0], Kind::Sh => &[0.0, 0.0],
            Kind::Buf => &[0.0, 0.0, 0.0, 1.0, 1.0], Kind::Key => &[0.0, 0.0, 60.0], Kind::Out => &[0.0],
        }
    }
    pub fn variants(self) -> &'static [&'static str] {
        match self {
            Kind::Trk => &["freq", "gate", "vel", "note"], Kind::Osc => &["sin", "saw", "sqr", "tri", "pulse", "phase"],
            Kind::Filt => &["lp", "hp", "bp"], Kind::Math => &["+", "-", "*", "/", "min", "max", "pow", ">", "<"],
            Kind::Shape => &["tanh", "clip", "abs", "sqrt", "neg"], _ => &[],
        }
    }
    /// nodes with a primary audio input at index 0 get spliced into a chain when added
    pub fn chainable(self) -> bool { matches!(self, Kind::Filt | Kind::Delay | Kind::Math | Kind::Shape | Kind::Sh | Kind::Buf | Kind::Key) }
}

impl Node {
    pub fn title(&self) -> String {
        let v = self.kind.variants();
        if v.is_empty() { format!("{} #{}", self.kind.name(), self.id) } else { format!("{} {} #{}", self.kind.name(), v[self.var % v.len()], self.id) }
    }
    pub fn short(&self) -> String {
        let v = self.kind.variants();
        if v.is_empty() { format!("{}{}", self.kind.name().to_lowercase(), self.id) } else { format!("{}{}", v[self.var % v.len()], self.id) }
    }
}

impl Graph {
    pub fn idx(&self, id: u32) -> Option<usize> { self.nodes.iter().position(|n| n.id == id) }
    pub fn get(&self, id: u32) -> Option<&Node> { self.nodes.iter().find(|n| n.id == id) }
    pub fn get_mut(&mut self, id: u32) -> Option<&mut Node> { self.nodes.iter_mut().find(|n| n.id == id) }
    pub fn out_id(&self) -> Option<u32> { self.nodes.iter().find(|n| n.kind == Kind::Out).map(|n| n.id) }
    pub fn find_trk(&self, track: u32, var: usize) -> Option<u32> {
        self.nodes.iter().find(|n| n.kind == Kind::Trk && n.track == track && n.var == var).map(|n| n.id)
    }

    pub fn add(&mut self, kind: Kind, var: usize, inputs: Vec<Input>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let mut inp: Vec<Input> = kind.defaults().iter().map(|&v| Input::Const(v)).collect();
        for (i, x) in inputs.into_iter().enumerate() { if i < inp.len() { inp[i] = x } }
        self.nodes.push(Node { id, kind, var, inputs: inp, track: 0 });
        id
    }
    pub fn add_trk(&mut self, track: u32, var: usize) -> u32 {
        let id = self.add(Kind::Trk, var, vec![]);
        self.nodes.last_mut().unwrap().track = track;
        id
    }

    /// Add a node wired to the selection: its first input reads `after` (unless `after` is OUT),
    /// OSC/ENV/KEY otherwise follow the track's TRK nodes, and chainable nodes are spliced
    /// after `after` (everything that read `after` now reads the new node).
    pub fn insert(&mut self, kind: Kind, after: Option<u32>, track: u32) -> u32 {
        let after = after.filter(|&a| self.get(a).map_or(false, |n| n.kind != Kind::Out));
        let l = |x: Option<u32>| x.map_or(Input::Const(0.0), Input::Link);
        let mut inputs = match kind {
            Kind::Osc => vec![l(self.find_trk(track, 0))],
            Kind::Env => vec![l(self.find_trk(track, 1))],
            Kind::Key => vec![l(self.find_trk(track, 1)), l(self.find_trk(track, 3))],
            _ => vec![],
        };
        // the selected node feeds the new node's first input, unless that would obviously be
        // the wrong kind of signal (an envelope as a frequency, audio as a gate, ...)
        if let Some(a) = after {
            let ak = self.get(a).map(|n| (n.kind, n.var));
            let sensible = match kind {
                Kind::Osc => matches!(ak, Some((Kind::Trk, 0)) | Some((Kind::Math, _)) | Some((Kind::Sh, _))),
                Kind::Env | Kind::Key => matches!(ak, Some((Kind::Trk, 1)) | Some((Kind::Key, _)) | Some((Kind::Math, 7)) | Some((Kind::Math, 8))),
                Kind::Trk | Kind::Noise => false,
                _ => true,
            };
            if sensible { if inputs.is_empty() { inputs.push(Input::Link(a)) } else { inputs[0] = Input::Link(a) } }
        }
        let var = if kind == Kind::Math { 2 } else { 0 }; // MATH defaults to *, the VCA case
        let ak = after.and_then(|a| self.get(a)).map(|n| n.kind);
        // an envelope and an oscillator want each other on the oscillator's amp
        if kind == Kind::Osc && ak == Some(Kind::Env) { while inputs.len() < 3 { inputs.push(Input::Const(kind.defaults()[inputs.len()])) } inputs[2] = Input::Link(after.unwrap()) }
        if kind == Kind::Noise && ak == Some(Kind::Env) { inputs = vec![Input::Link(after.unwrap())] }
        let id = self.add(kind, var, inputs);
        if kind == Kind::Env && matches!(ak, Some(Kind::Osc) | Some(Kind::Noise)) {
            let a = after.unwrap();
            let k = if ak == Some(Kind::Osc) { 2 } else { 0 };
            if let Some(n) = self.get_mut(a) { if matches!(n.inputs[k], Input::Const(_)) { n.inputs[k] = Input::Link(id) } }
        }
        if kind == Kind::Trk { self.nodes.last_mut().unwrap().track = track }
        if let (Some(a), true) = (after, kind.chainable() && kind != Kind::Key) {
            for n in &mut self.nodes {
                if n.id == id { continue }
                for i in &mut n.inputs { if *i == Input::Link(a) { *i = Input::Link(id) } }
            }
        }
        id
    }

    /// the recipe behind `a` `v`: OSC (freq from the track) × ENV (gate from the track) → OUT,
    /// reusing `sel` if it already is an OSC or ENV. Returns the MATH * node.
    pub fn voice(&mut self, sel: Option<u32>, track: u32) -> u32 {
        let selk = sel.and_then(|id| self.get(id)).map(|n| n.kind);
        let l = |x: Option<u32>| x.map_or(Input::Const(0.0), Input::Link);
        let env = if selk == Some(Kind::Env) { sel.unwrap() } else { let g = l(self.find_trk(track, 1)); self.add(Kind::Env, 0, vec![g, Input::Const(0.01), Input::Const(0.2), Input::Const(0.6), Input::Const(0.2)]) };
        let osc = if selk == Some(Kind::Osc) { sel.unwrap() } else { let f = l(self.find_trk(track, 0)); self.add(Kind::Osc, 1, vec![f]) };
        if let Some(n) = self.get_mut(osc) { n.inputs[2] = Input::Link(env) }
        let vel = l(self.find_trk(track, 2));
        let vv = self.add(Kind::Math, 2, vec![Input::Link(osc), vel]);
        let gain = self.add(Kind::Math, 2, vec![Input::Link(vv), Input::Const(0.3)]);
        self.to_out(gain);
        osc
    }

    /// send `id` to OUT: directly if OUT is free, otherwise through a new MATH + that mixes it with what's already there
    pub fn to_out(&mut self, id: u32) -> Option<u32> {
        let out = self.out_id()?;
        if id == out { return None }
        let cur = self.get(out)?.inputs[0];
        match cur {
            Input::Link(x) if x == id => Some(out),
            Input::Link(x) => {
                let mix = self.add(Kind::Math, 0, vec![Input::Link(x), Input::Link(id)]);
                self.get_mut(out).unwrap().inputs[0] = Input::Link(mix);
                Some(mix)
            }
            Input::Const(_) => { self.get_mut(out).unwrap().inputs[0] = Input::Link(id); Some(out) }
        }
    }

    #[allow(dead_code)]
    /// human-readable reasons a track might be silent
    pub fn diagnose(&self, track_ids: &[u32], names: &[String]) -> Vec<String> {
        let mut out = vec![];
        let deps = self.deps(track_ids);
        let Some(o) = self.out_id().and_then(|id| self.idx(id)) else { return vec!["no OUT node".into()] };
        for (t, name) in names.iter().enumerate() {
            let has_trk = self.nodes.iter().any(|n| n.kind == Kind::Trk && n.track == track_ids[t]);
            if has_trk && !deps[o][t] { out.push(format!("track {name} has no path to OUT: build a chain from its TRK nodes and press o on the last node (or :voice)")) }
        }
        if let Some(n) = self.get(self.out_id().unwrap()) { if matches!(n.inputs[0], Input::Const(_)) { out.push("OUT has nothing connected (press o on a node)".into()) } }
        for n in &self.nodes {
            if n.kind == Kind::Osc { if let Input::Link(src) = n.inputs[0] {
                if let Some(sn) = self.get(src) {
                    if sn.kind == Kind::Env || sn.kind == Kind::Key || (sn.kind == Kind::Trk && sn.var != 0) {
                        out.push(format!("{} freq is driven by {} (a 0..1 signal, so 0..1 Hz): wire TRK freq into freq, or scale it with MATH * first", n.short(), sn.short()));
                    }
                }
            } }
        }
        out
    }

    /// disconnect input `k` of node `t` (back to its default constant)
    pub fn disconnect(&mut self, t: u32, k: usize) {
        if let Some(n) = self.get_mut(t) { if k < n.inputs.len() { let d = n.kind.defaults()[k]; n.inputs[k] = Input::Const(d) } }
    }

    /// every (node id, input index) that reads `id`
    pub fn consumers(&self, id: u32) -> Vec<(u32, usize)> {
        let mut v = vec![];
        for n in &self.nodes { for (k, inp) in n.inputs.iter().enumerate() { if *inp == Input::Link(id) { v.push((n.id, k)) } } }
        v
    }

    /// like `insert`, but the new chainable node only replaces the given consumer inputs of `after`
    pub fn insert_into(&mut self, kind: Kind, after: u32, targets: &[(u32, usize)], track: u32) -> u32 {
        let id = self.insert(kind, None, track);
        if let Some(n) = self.get_mut(id) { if !n.inputs.is_empty() { n.inputs[0] = Input::Link(after) } }
        for &(t, k) in targets { if let Some(n) = self.get_mut(t) { if k < n.inputs.len() { n.inputs[k] = Input::Link(id) } } }
        id
    }

    /// remove a node; consumers get rewired to its primary input (or a constant)
    pub fn remove(&mut self, id: u32) {
        let Some(i) = self.idx(id) else { return };
        if self.nodes[i].kind == Kind::Out { return }
        let repl = self.nodes[i].inputs.iter().find(|x| matches!(x, Input::Link(_))).copied();
        self.nodes.remove(i);
        for n in &mut self.nodes {
            let d = n.kind.defaults();
            for (k, inp) in n.inputs.iter_mut().enumerate() {
                if *inp == Input::Link(id) { *inp = repl.unwrap_or(Input::Const(d.get(k).copied().unwrap_or(0.0))) }
            }
        }
    }

    /// fix up inputs lengths (e.g. after loading a file)
    pub fn normalize(&mut self) {
        for n in &mut self.nodes {
            let d = n.kind.defaults();
            for k in n.inputs.len()..d.len() { n.inputs.push(Input::Const(d[k])) }
            n.inputs.truncate(d.len());
            for (k, inp) in n.inputs.iter_mut().enumerate() { if let Input::Const(v) = inp { if !v.is_finite() { *v = d[k] } } }
        }
        if self.out_id().is_none() { self.add(Kind::Out, 0, vec![]); }
        self.next_id = self.nodes.iter().map(|n| n.id + 1).max().unwrap_or(0).max(self.next_id);
    }

    /// which tracks each node (transitively) depends on
    pub fn deps(&self, track_ids: &[u32]) -> Vec<Vec<bool>> {
        let n = self.nodes.len();
        let mut deps = vec![vec![false; track_ids.len()]; n];
        for (i, node) in self.nodes.iter().enumerate() {
            if node.kind == Kind::Trk { if let Some(t) = track_ids.iter().position(|&id| id == node.track) { deps[i][t] = true } }
        }
        for _ in 0..n {
            let mut changed = false;
            for i in 0..n {
                for k in 0..self.nodes[i].inputs.len() {
                    let Input::Link(id) = self.nodes[i].inputs[k] else { continue };
                    let Some(j) = self.idx(id) else { continue };
                    for t in 0..track_ids.len() { if deps[j][t] && !deps[i][t] { deps[i][t] = true; changed = true } }
                }
            }
            if !changed { break }
        }
        deps
    }

    /// one program per track, each containing only the nodes that track can influence
    pub fn compile_all(&self, track_ids: &[u32]) -> Vec<Program> {
        let deps = self.deps(track_ids);
        (0..track_ids.len()).map(|t| {
            let keep: Vec<bool> = deps.iter().map(|d| d[t] || d.iter().all(|&x| !x)).collect();
            self.compile_subset(&keep, track_ids[t])
        }).collect()
    }

    /// evaluation order: depth-first from OUT so forward signal flow has no latency;
    /// only back-edges (feedback) read the previous sample.
    fn order(&self, keep: &[bool]) -> Vec<usize> {
        fn dfs(g: &Graph, i: usize, keep: &[bool], state: &mut Vec<u8>, order: &mut Vec<usize>) {
            if state[i] != 0 || !keep[i] { return }
            state[i] = 1;
            for inp in &g.nodes[i].inputs {
                if let Input::Link(id) = inp { if let Some(j) = g.idx(*id) { dfs(g, j, keep, state, order) } }
            }
            state[i] = 2;
            order.push(i);
        }
        let mut state = vec![0u8; self.nodes.len()];
        let mut order = vec![];
        if let Some(o) = self.out_id().and_then(|id| self.idx(id)) { dfs(self, o, keep, &mut state, &mut order) }
        for i in 0..self.nodes.len() { dfs(self, i, keep, &mut state, &mut order) }
        order
    }

    fn compile_subset(&self, keep: &[bool], track: u32) -> Program {
        let order = self.order(keep);
        let mut row_of = vec![usize::MAX; self.nodes.len()];
        for (r, &i) in order.iter().enumerate() { row_of[i] = r }
        let mut p = Program { row_slot: vec![0; order.len()], row_node: order.clone(), ..Default::default() };
        for (r, &i) in order.iter().enumerate() {
            let node = &self.nodes[i];
            let ins: Vec<usize> = node.inputs.iter().map(|inp| {
                match inp {
                    Input::Link(id) if self.idx(*id).map_or(false, |j| row_of[j] != usize::MAX) => p.ops.push(Op::Row(row_of[self.idx(*id).unwrap()])),
                    Input::Link(_) => { p.params.push(0.0); p.ops.push(Op::Param(p.params.len() - 1)) }
                    Input::Const(v) => { p.params.push(*v); p.ops.push(Op::Param(p.params.len() - 1)) }
                }
                p.ops.len() - 1
            }).collect();
            let v = node.var;
            let op = match node.kind {
                Kind::Trk => if node.track == track { [Op::Freq, Op::Gate, Op::Vel, Op::Note][v % 4].clone() } else { p.params.push(0.0); Op::Param(p.params.len() - 1) },
                Kind::Osc => Op::Osc([dsp::Wave::Sin, dsp::Wave::Saw, dsp::Wave::Sqr, dsp::Wave::Tri, dsp::Wave::Pulse, dsp::Wave::Phase][v % 6], ins[0], ins[1], ins[2]),
                Kind::Noise => Op::Noise(ins[0]),
                Kind::Env => Op::Env(ins[0], ins[1], ins[2], ins[3], ins[4]),
                Kind::Filt => Op::Filt([dsp::Filt::Lp, dsp::Filt::Hp, dsp::Filt::Bp][v % 3], ins[0], ins[1], ins[2]),
                Kind::Delay => Op::Delay(ins[0], ins[1]),
                Kind::Math => Op::Bin([dsp::Bin::Add, dsp::Bin::Sub, dsp::Bin::Mul, dsp::Bin::Div, dsp::Bin::Min, dsp::Bin::Max, dsp::Bin::Pow, dsp::Bin::Gt, dsp::Bin::Lt][v % 9], ins[0], ins[1]),
                Kind::Shape => Op::Un([dsp::Un::Tanh, dsp::Un::Clip, dsp::Un::Abs, dsp::Un::Sqrt, dsp::Un::Neg][v % 5], ins[0]),
                Kind::Sh => Op::Sh(ins[0], ins[1]),
                Kind::Buf => Op::Buf(ins[0], ins[1], ins[2], ins[3], ins[4]),
                Kind::Key => Op::Key(ins[0], ins[1], ins[2]),
                Kind::Out => { p.row_slot[r] = ins[0]; if p.out.is_none() { p.out = Some(ins[0]) } continue }
            };
            p.ops.push(op);
            p.row_slot[r] = p.ops.len() - 1;
        }
        p
    }

    /// nodes upstream or downstream of any marked node (plus the marks themselves)
    pub fn related(&self, marks: &[u32]) -> Vec<bool> {
        let n = self.nodes.len();
        let starts: Vec<usize> = marks.iter().filter_map(|&m| self.idx(m)).collect();
        let mut anc = vec![false; n];
        let mut stack = starts.clone();
        while let Some(i) = stack.pop() {
            if anc[i] { continue }
            anc[i] = true;
            for inp in &self.nodes[i].inputs { if let Input::Link(id) = inp { if let Some(j) = self.idx(*id) { stack.push(j) } } }
        }
        let mut desc = vec![false; n];
        let mut stack = starts;
        while let Some(i) = stack.pop() {
            if desc[i] { continue }
            desc[i] = true;
            let id = self.nodes[i].id;
            for (j, nd) in self.nodes.iter().enumerate() { if nd.inputs.contains(&Input::Link(id)) { stack.push(j) } }
        }
        (0..n).map(|i| anc[i] || desc[i]).collect()
    }

    #[allow(dead_code)]
    pub fn layout(&self, track_ids: &[u32]) -> Vec<(usize, usize)> {
        self.layout_masked(track_ids, &vec![true; self.nodes.len()])
    }

    /// (column, row) per node for display: track inputs on the left, OUT on the right.
    /// Rows within a column are grouped by the first track the node depends on.
    /// Nodes with keep=false are hidden (usize::MAX) and ignored for spacing.
    pub fn layout_masked(&self, track_ids: &[u32], keep: &[bool]) -> Vec<(usize, usize)> {
        fn height(g: &Graph, i: usize, keep: &[bool], memo: &mut Vec<Option<usize>>, visiting: &mut Vec<bool>) -> usize {
            if let Some(h) = memo[i] { return h }
            if visiting[i] { return 0 }
            visiting[i] = true;
            let mut h = 0;
            for inp in &g.nodes[i].inputs {
                if let Input::Link(id) = inp { if let Some(j) = g.idx(*id) { if keep[j] { h = h.max(height(g, j, keep, memo, visiting) + 1) } } }
            }
            visiting[i] = false;
            memo[i] = Some(h);
            h
        }
        let n = self.nodes.len();
        let (mut memo, mut visiting) = (vec![None; n], vec![false; n]);
        if let Some(o) = self.out_id().and_then(|id| self.idx(id)) { if keep[o] { height(self, o, keep, &mut memo, &mut visiting); } }
        let cols: Vec<usize> = (0..n).map(|i| if keep[i] { height(self, i, keep, &mut memo, &mut visiting) } else { usize::MAX }).collect();
        let deps = self.deps(track_ids);
        let group = |i: usize| deps[i].iter().position(|&d| d).unwrap_or(track_ids.len());
        let mut rows = vec![usize::MAX; n];
        let maxc = cols.iter().filter(|&&c| c != usize::MAX).max().copied().unwrap_or(0);
        for c in 0..=maxc {
            let mut ids: Vec<usize> = (0..n).filter(|&i| cols[i] == c).collect();
            ids.sort_by_key(|&i| (group(i), self.nodes[i].id));
            for (r, i) in ids.into_iter().enumerate() { rows[i] = r }
        }
        cols.into_iter().zip(rows).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pruning_and_splicing() {
        let song = crate::song::Song::demo();
        let ids: Vec<u32> = song.tracks.iter().map(|t| t.id).collect();
        let mut g = song.graph.clone();
        let progs = g.compile_all(&ids);
        // every track program is smaller than the whole graph but has an output
        for p in &progs { assert!(p.out.is_some()); assert!(p.row_node.len() < g.nodes.len()); }
        let osc = g.nodes.iter().find(|n| n.kind == Kind::Osc).unwrap().id;
        let f = g.insert(Kind::Filt, Some(osc), ids[0]);
        assert_eq!(g.get(f).unwrap().inputs[0], Input::Link(osc));
        assert!(g.nodes.iter().filter(|n| n.id != f).all(|n| !n.inputs.contains(&Input::Link(osc))));
        g.remove(f);
        assert!(g.nodes.iter().any(|n| n.inputs.contains(&Input::Link(osc))));
        let lay = g.layout(&ids);
        let out = g.idx(g.out_id().unwrap()).unwrap();
        assert_eq!(lay[out].0, lay.iter().map(|p| p.0).max().unwrap());
    }
}
