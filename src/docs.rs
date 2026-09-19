//! Built-in manual: a tiny HTTP server on localhost that serves one HTML page.
//! The page is generated from the same tables the synth uses, so it can't drift from the code.
use crate::graph::{Kind, ALL, PALETTE};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::OnceLock;

static PORT: OnceLock<u16> = OnceLock::new();

pub fn open() -> String {
    let port = *PORT.get_or_init(start);
    let url = format!("http://127.0.0.1:{port}/");
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(target_os = "linux")]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn();
    match r { Ok(_) => format!("manual: {url}"), Err(e) => format!("couldn't open a browser ({e}); manual is at {url}") }
}

fn start() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind docs server");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let body = html();
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(body.as_bytes());
        }
    });
    port
}

fn node_doc(k: Kind) -> (&'static str, &'static str) {
    match k {
        Kind::Trk => ("The notes of one track, as signals. Pick which track with h/l on its <b>track</b> param. Variants: <b>freq</b> = pitch in Hz, <b>gate</b> = 1 while the note is held (0 after), <b>vel</b> = velocity 0..1, <b>note</b> = MIDI note number (C4 = 60). Every voice of that track evaluates the graph with its own values here.",
            "A track is silent until something wires its TRK nodes toward OUT. Tab from the arrangement jumps to the current track's gate node."),
        Kind::Osc => ("Oscillator. <b>freq</b> in Hz (wire it to TRK freq, or type a constant for an LFO), <b>pw</b> only matters for <b>pulse</b>, <b>amp</b> scales the output: put an ENV there and the note has a volume envelope. Variants: sin, saw, sqr, tri, pulse, and <b>phase</b>, a raw 0..1 ramp for driving BUF, S&amp;H or your own waveshaping. saw/sqr/pulse are anti-aliased.",
            "FM: wire an OSC (times a MATH *) into another OSC's freq. Detune: MATH * freq by 1.01. Sub osc: MATH * freq by 0.5. Vibrato: add a slow sin*3 to freq."),
        Kind::Noise => ("White noise, -1..1 every sample, times <b>amp</b> (put an ENV there for a hit).", "Hi-hat: NOISE → FILT hp 6000 → * ENV. Snare: NOISE + sine, short envelope. Random modulation: NOISE → S&amp;H."),
        Kind::Env => ("ADSR envelope, 0..1. <b>gate</b> starts it on a rising edge and releases when it drops. <b>att</b>/<b>dec</b>/<b>rel</b> are seconds, <b>sus</b> is a level 0..1.",
            "Percussive: sus 0. Pitch envelope for a kick: ENV → MATH * 250 → MATH + 50 → OSC sin freq. Filter sweep: ENV → MATH * 3000 → MATH + 300 → FILT cut."),
        Kind::Filt => ("State-variable filter. <b>in</b> audio, <b>cut</b> cutoff in Hz (modulate it with anything), <b>q</b> resonance 0.1..50. Variants: lp lowpass, hp highpass, bp bandpass.",
            "Subtractive synth: OSC saw → FILT lp with an envelope on cut. Wah: cut from a slow OSC sin. Self-oscillating: q 30+."),
        Kind::Delay => ("Delay line up to 2 seconds. <b>in</b> audio, <b>time</b> seconds (modulating it slowly gives chorus/flanger). Output is the delayed signal only, so mix it back with MATH +.",
            "Echo with feedback: feed the DELAY output through MATH * 0.4 back into the MATH + that feeds the DELAY (cycles are allowed; the loop reads the previous sample). Karplus-Strong string: NOISE burst → DELAY with time = 1/freq (MATH / : 1 divided by TRK freq) and high feedback through a FILT lp. Reverb: several delays with different times, mixed."),
        Kind::Math => ("Two-input arithmetic. Variants: + - * / min max pow, and comparisons &gt; and &lt; which output 1 or 0. Divide by zero gives 0.",
            "Gain: MATH * by a constant. Mix: MATH +. Ring mod: MATH * two oscillators. Gates from any signal: MATH &gt; 0.5. Rectify: max(x, 0)."),
        Kind::Shape => ("Waveshapers on one input. tanh = soft clip / drive (scale the input up first with MATH *), clip = hard clip to -1..1, abs = full-wave rectify (octave up), sqrt (of |x|), neg = invert.",
            "Distortion: MATH * 5 → SHAPE tanh → MATH * 0.3. Octaver: SHAPE abs then a FILT lp to smooth."),
        Kind::Sh => ("Sample &amp; hold. Captures <b>x</b> whenever <b>trig</b> goes above 0.5 and holds it until the next rising edge.",
            "Random stepped modulation: NOISE into x, OSC sqr at 6 Hz into trig. Arpeggio-like pitch steps: sample a slow OSC saw with a fast clock."),
        Kind::Buf => ("A recordable buffer up to 4 seconds: the primitive that makes wavetables, loopers, granular and tape effects possible. <b>in</b> is written at position <b>wpos</b> (0..1 across <b>size</b> seconds) while <b>rec</b> &gt; 0.5; the output is read (interpolated) at <b>rpos</b> (0..1, wraps). Both positions are signals, so drive them with OSC <b>phase</b> ramps, S&amp;H, envelopes or constants.",
            "Wavetable: write one cycle (rec 1, wpos from OSC phase at 1/size Hz... or just record any sound) then read with an OSC phase at the note frequency times size. Tape/pitch shift: write with phase at 0.5 Hz, read with phase at 0.53 Hz (the demo pad). Reverse: rpos = MATH 1 - phase. Freeze: rec from TRK gate so it only records while a note is held, then keep reading. Granular: rpos = S&amp;H(NOISE) + a small fast phase ramp."),
        Kind::Key => ("Note router. Outputs <b>gate</b> only while <b>note</b> equals <b>key</b> (MIDI number, shown as a note name; type 62 or D4). Zero otherwise.",
            "Drums on one track: KEY C4 → kick chain, KEY D4 → hat chain, KEY E4 → snare chain, all summed with MATH + into the mix. Adding a KEY node auto-wires the current track's gate and note."),
        Kind::Out => ("The master output. Everything you hear goes through its <b>in</b>. There is exactly one; it can't be deleted.",
            "Sum tracks into it with a chain of MATH + nodes. A MATH * before OUT is your master volume. A SHAPE tanh before OUT is a soft limiter."),
    }
}

fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

pub fn html() -> String {
    let mut nodes = String::new();
    for k in ALL {
        let (what, tips) = node_doc(k);
        let key = PALETTE.iter().find(|(_, kk, _)| *kk == k).map_or("(auto)".to_string(), |(c, _, _)| format!("<kbd>a</kbd> then <kbd>{c}</kbd>"));
        let params: Vec<String> = k.params().iter().zip(k.defaults()).map(|(p, d)| format!("<b>{p}</b> = {d}")).collect();
        let vars = if k.variants().is_empty() { String::new() } else { format!("<div><span class=l>variants</span> {} <span class=dim>(w / W cycles)</span></div>", k.variants().iter().map(|v| esc(v)).collect::<Vec<_>>().join(", ")) };
        nodes.push_str(&format!(
            "<section id=\"{n}\"><h3>{n}</h3><div><span class=l>add</span> {key}</div><div><span class=l>inputs</span> {p}</div>{vars}<p>{what}</p><p class=tip><b>Recipes.</b> {tips}</p></section>",
            n = k.name(), p = if params.is_empty() { "none".into() } else { params.join(" · ") }));
    }
    format!(r##"<!doctype html><html><head><meta charset="utf-8"><title>dirty daw manual</title>
<style>
body{{background:#101014;color:#ddd;font:15px/1.5 -apple-system,Helvetica,Arial,sans-serif;max-width:900px;margin:0 auto;padding:24px}}
h1{{color:#5ac8ff}} h2{{color:#ffcd50;margin-top:40px;border-bottom:1px solid #333}} h3{{color:#78e68c;margin:24px 0 4px}}
kbd{{background:#2a2a30;border:1px solid #555;border-radius:4px;padding:0 5px;font-family:monospace;color:#fff}}
code{{background:#22222a;padding:1px 5px;border-radius:3px;font-family:monospace}}
table{{border-collapse:collapse;width:100%}} td{{padding:3px 8px;border-bottom:1px solid #222;vertical-align:top}} td:first-child{{white-space:nowrap;color:#fff}}
.l{{color:#888;display:inline-block;width:70px}} .dim{{color:#777}} .tip{{color:#bbb;background:#17171c;padding:8px 10px;border-left:3px solid #ffcd50}}
section{{margin-bottom:10px}} nav a{{margin-right:12px;color:#5ac8ff}}
</style></head><body>
<h1>dirty daw</h1>
<p>Keyboard-only, pure-synthesis DAW. No samples, no plugins: one signal graph you patch by hand, and an arrangement of patterns that plays into it. This page is served by the running app (<kbd>F12</kbd> or <code>:help</code>).</p>
<nav><a href="#concepts">concepts</a><a href="#arrange">arrangement</a><a href="#pattern">pattern editor</a><a href="#synth">synth graph</a><a href="#commands">commands</a><a href="#nodes">nodes</a><a href="#files">files</a></nav>

<h2 id="concepts">Concepts</h2>
<p><b>Everything is a signal.</b> The synth is one graph shared by the whole song. Each node computes one number per sample (48000 times a second) from its inputs. An input is either a constant you type or a link to another node's output. Any output can drive any input: audio, pitch, cutoff, envelope times, buffer positions.</p>
<p><b>Inputs and outputs.</b> A node has exactly <em>one output</em> (the number it computes) and a handful of named <em>inputs</em> (the rows inside the box). Each input holds either a constant you type or a link that reads another node's output. That is the only kind of connection there is: "connect A to B" means "some input of B reads A". A node's output can feed any number of inputs; the <code>out ▸</code> row lists where it goes, and the dots on the box edges are the ports (bright = linked). Connect from either side, with two different keys so the direction is never ambiguous: <kbd>c</kbd> sends <em>this node's output</em> into an input of a node you pick (works in node and param mode); <kbd>i</kbd> makes <em>this input</em> read a node you pick (param mode, or node mode for the first input). While picking, a white preview wire with a label like <code>env5 ▸ *7.b</code> shows exactly what will be connected. <kbd>d</kbd> cuts one of a node's output connections, <kbd>x</kbd> in param mode cuts an input.</p>
<p><b>Envelopes don't trigger oscillators, they control volume.</b> Oscillators run all the time. An ENV is a 0..1 curve that starts when its <b>gate</b> rises (the note starts) and fades when it drops. Connect the ENV's output to the oscillator's <b>amp</b> input and the note has a shape: select the ENV, <kbd>c</kbd>, move onto the OSC, <kbd>Enter</kbd>, pick <b>amp</b>. (Adding an OSC while an ENV is selected, or an ENV while an OSC is selected, wires this automatically.) The standard voice is TRK freq → OSC (amp ◂ ENV from TRK gate) → OUT; <code>:voice</code> or <kbd>a</kbd> <kbd>v</kbd> builds it.</p>
<p><b>Fastest way to hear something:</b> select any node, press <kbd>a</kbd> then <kbd>v</kbd>. That builds OSC × ENV → OUT for the current track, reusing the selected node if it already is an OSC or an ENV. Then put notes in a pattern on that track.</p>
<p><b>Tracks are sources.</b> A track shows up in the graph as <code>TRK</code> nodes carrying the freq / gate / vel / note of whatever note the track is playing. A chain of nodes from a track's TRK nodes to <code>OUT</code> is that track's instrument. Nothing is hard-wired: a track with no path to OUT is silent.</p>
<p><b>Voices.</b> Each track has 8 voices. A voice is a private copy of the graph, pruned to the nodes that track can influence (nodes that depend only on other tracks are skipped), running with that voice's note. Voices are summed at OUT. Consequence: effects are per-voice. That's perfect for anything linear (delay, filters, mixing) and slightly different from a DAW's shared bus for nonlinear things like a master <code>tanh</code>.</p>
<p><b>Feedback is allowed.</b> The graph is evaluated from OUT backwards each sample; a link that points "back" reads the previous sample's value. That one-sample delay is what makes feedback loops stable. Feedback links draw in orange.</p>
<p><b>Live view.</b> Every node has a level bar, links glow with their signal level, and track rows show meters. Constants change instantly while playing; changing the graph's structure restarts that track's voices.</p>

<h2 id="arrange">Arrangement view</h2>
<p>All tracks at once. Each solid block is a pattern; outlined <code>↻n</code> blocks are its repeats. The cursor is a white box: <kbd>j</kbd>/<kbd>k</kbd> track, <kbd>h</kbd>/<kbd>l</kbd> step, <kbd>H</kbd>/<kbd>L</kbd> bar, <kbd>g</kbd>/<kbd>G</kbd> start/end.</p>
<table>
<tr><td><kbd>Enter</kbd></td><td>open the pattern under the cursor in the editor. On empty space: create a bar-long pattern there and open it.</td></tr>
<tr><td><kbd>v</kbd> then move, then <kbd>P</kbd></td><td>select a range of steps and create an empty pattern exactly that long.</td></tr>
<tr><td><kbd>R</kbd></td><td>repeat the pattern until the next pattern on the track (or the end of the song).</td></tr>
<tr><td><kbd>E</kbd></td><td>end the repeats at the cursor.</td></tr>
<tr><td><kbd>D</kbd></td><td>detach the repeat copy under the cursor into an independent pattern, so you can change it without changing the others.</td></tr>
<tr><td><kbd>x</kbd> / <kbd>Delete</kbd></td><td>delete the pattern.</td></tr>
<tr><td><kbd>y</kbd> / <kbd>p</kbd></td><td>yank the pattern / paste a copy at the cursor.</td></tr>
<tr><td><kbd>{{</kbd> / <kbd>}}</kbd></td><td>move the pattern one step earlier / later.</td></tr>
<tr><td><kbd>,</kbd> / <kbd>.</kbd></td><td>shorten / lengthen the pattern by one step.</td></tr>
<tr><td><kbd>r</kbd> / <kbd>n</kbd></td><td>random melody in the pattern (follows the chord progression) / humanize it.</td></tr>
<tr><td><kbd>m</kbd></td><td>mute the track.</td></tr>
<tr><td><kbd>Space</kbd></td><td>play / pause (from the cursor). <kbd>s</kbd> moves the playhead to the cursor while playing.</td></tr>
<tr><td><kbd>u</kbd> / <kbd>Cmd+Z</kbd></td><td>undo (everything, including graph edits).</td></tr>
<tr><td><kbd>Tab</kbd></td><td>synth graph, landing on this track's TRK gate node.</td></tr>
<tr><td><kbd>:</kbd></td><td>command line (below).</td></tr>
</table>

<h2 id="pattern">Pattern editor</h2>
<p>A piano roll of one pattern. While it is open, <kbd>Space</kbd> loops only this pattern, at its own length, so you can hear edits immediately. <kbd>Esc</kbd> returns to the arrangement and the whole song plays again. Rows are pitches (in-key rows are lighter); the cursor's octave is the octave that degree keys use.</p>
<table>
<tr><td><kbd>h</kbd>/<kbd>l</kbd>, <kbd>H</kbd>/<kbd>L</kbd></td><td>step / bar. <kbd>j</kbd>/<kbd>k</kbd> move by scale tone, <kbd>J</kbd>/<kbd>K</kbd> by octave.</td></tr>
<tr><td><kbd>Enter</kbd></td><td>put a note at the cursor pitch (or remove the one that is there), then advance by the insert length.</td></tr>
<tr><td><kbd>1</kbd>…<kbd>7</kbd></td><td>scale degree in the cursor's octave, replacing whatever is at that step, then advance.</td></tr>
<tr><td><kbd>c</kbd> + degree, <kbd>C</kbd> + degree</td><td>diatonic triad / seventh chord on that degree. <kbd>a</kbd> arpeggiates the chord under the cursor.</td></tr>
<tr><td><kbd>,</kbd> / <kbd>.</kbd></td><td>halve / double the insert length (shown in the header as ins n/grid).</td></tr>
<tr><td><kbd>v</kbd></td><td>start a step selection. Range operations use the selection, otherwise the whole pattern (<kbd>r</kbd> <kbd>n</kbd> <kbd>X</kbd> <kbd>y</kbd>) or the cursor step (<kbd>-</kbd> <kbd>=</kbd> <kbd>(</kbd> <kbd>)</kbd> <kbd>&lt;</kbd> <kbd>&gt;</kbd> <kbd>_</kbd> <kbd>+</kbd>).</td></tr>
<tr><td><kbd>r</kbd></td><td>new random melody using each bar's chord from the progression (<code>:prog</code>).</td></tr>
<tr><td><kbd>n</kbd></td><td>humanize: small random timing and velocity changes.</td></tr>
<tr><td><kbd>(</kbd> / <kbd>)</kbd></td><td>nudge notes earlier / later by 1/8 of a step. Times are stored as floats, so nothing is ever re-quantized.</td></tr>
<tr><td><kbd>-</kbd> / <kbd>=</kbd></td><td>transpose by a semitone.</td></tr>
<tr><td><kbd>&lt;</kbd> / <kbd>&gt;</kbd></td><td>shorten / lengthen notes by a step.</td></tr>
<tr><td><kbd>_</kbd> / <kbd>+</kbd></td><td>velocity down / up.</td></tr>
<tr><td><kbd>x</kbd> / <kbd>X</kbd></td><td>delete the note at the cursor (or all notes at that step) / clear the range.</td></tr>
<tr><td><kbd>y</kbd> / <kbd>p</kbd></td><td>yank notes in the range / paste them at the cursor.</td></tr>
</table>

<h2 id="synth">Synth graph</h2>
<p>Nodes are laid out automatically: track inputs on the left, OUT on the right, each column one step further along the signal flow. The selected node has a white border. There are three modes; the help line at the bottom always shows the current one.</p>
<p><b>Silent?</b> The second header line turns orange with a diagnosis: a track with no path to OUT, nothing connected to OUT, or an oscillator whose freq is driven by a 0..1 signal (an envelope or gate) and therefore runs at 0..1 Hz. A typical voice is TRK freq → OSC → MATH * (other input: ENV from TRK gate) → OUT.</p>
<h3>Node mode</h3>
<table>
<tr><td><kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd></td><td>move between nodes (columns / rows of the layout).</td></tr>
<tr><td><kbd>Enter</kbd></td><td>edit the node's inputs (param mode).</td></tr>
<tr><td><kbd>c</kbd></td><td>connect this node's <em>output</em>: move the yellow box onto the target node, <kbd>Enter</kbd>, then pick which of its inputs should read this node (<kbd>j</kbd>/<kbd>k</kbd> or a number), <kbd>Enter</kbd>. A white preview wire shows the connection before you confirm.</td></tr>
<tr><td><kbd>i</kbd></td><td>the other direction: pick a node that this node's first input should read.</td></tr>
<tr><td><kbd>d</kbd></td><td>disconnect this node's output from one (or all) of the inputs it feeds.</td></tr>
<tr><td><kbd>a</kbd></td><td>add a node: a palette opens, press its letter. Chainable nodes (filter, delay, math, shaper, S&amp;H, buffer) are spliced right after the selected node. If the selected node feeds several places you are asked which connection the new node goes into (<kbd>j</kbd>/<kbd>k</kbd> or a number, the candidate wire turns white, <kbd>Enter</kbd>); "all" splices every connection, "none" only wires the input. Every new node's first input (OSC freq, ENV gate, KEY gate, FILT in, …) reads the selected node when that makes sense: an OSC takes the selection only if it is a TRK freq, MATH or S&amp;H node; an ENV or KEY only if it is a TRK gate, KEY or a MATH comparison. Otherwise they fall back to the current track's TRK nodes. So select a KEY and add an ENV to get an envelope triggered by that key.</td></tr>
<tr><td><kbd>o</kbd></td><td>send the selected node to OUT. If OUT already has an input, a MATH + is inserted so both are mixed. This is how you make a new track audible: build its chain, then <kbd>o</kbd> on the last node.</td></tr>
<tr><td><kbd>x</kbd></td><td>delete the node; its consumers are rewired to its main input.</td></tr>
<tr><td><kbd>w</kbd> / <kbd>W</kbd></td><td>cycle the node's variant (saw → sqr, lp → hp, + → *, …).</td></tr>
<tr><td><kbd>v</kbd></td><td>mark / unmark the selected node (small dot). Marks are the seeds for focus.</td></tr>
<tr><td><kbd>f</kbd></td><td>toggle <b>focus</b>: show only the marked nodes plus everything upstream or downstream of them. Nodes you add while focused are marked automatically so they stay visible. <kbd>F</kbd> clears the marks.</td></tr>
<tr><td><kbd>[</kbd> / <kbd>]</kbd></td><td>change the current track (affects auto-wiring and where Tab lands).</td></tr>
</table>
<h3>Param mode</h3>
<table>
<tr><td><kbd>j</kbd> / <kbd>k</kbd></td><td>choose the parameter.</td></tr>
<tr><td><kbd>h</kbd> / <kbd>l</kbd></td><td>decrease / increase by about 12% (<kbd>H</kbd> / <kbd>L</kbd>: 1%). On a TRK node this cycles the track; on a KEY node's key it steps a semitone.</td></tr>
<tr><td>digits then <kbd>Enter</kbd></td><td>type an exact value: <code>0.25</code>, <code>-3</code>, <code>6000</code>. For a KEY node's key you can also type a note name like <code>D4</code>. The value shows in the row as you type; <kbd>Backspace</kbd> edits, <kbd>Enter</kbd> commits.</td></tr>
<tr><td><kbd>i</kbd></td><td>this input reads…: pick the source node with <kbd>h j k l</kbd> (yellow border) and press <kbd>Enter</kbd>. The input now follows that node's output.</td></tr>
<tr><td><kbd>c</kbd></td><td>this node's output goes to…: same as <kbd>c</kbd> in node mode.</td></tr>
<tr><td><kbd>x</kbd></td><td>disconnect: back to the default constant.</td></tr>
<tr><td><kbd>Enter</kbd> / <kbd>Esc</kbd></td><td>back to node mode.</td></tr>
</table>

<h2 id="commands">Command line</h2>
<p>Press <kbd>:</kbd>, type the command with its arguments separated by spaces, press <kbd>Enter</kbd>. For example <code>:bpm 128</code> is three keystrokes plus the number: <kbd>:</kbd>, then <code>bpm 128</code>, then <kbd>Enter</kbd>. <kbd>Esc</kbd> cancels.</p>
<table>
<tr><td><code>bpm 128</code></td><td>tempo.</td></tr>
<tr><td><code>bars 8</code></td><td>song length in bars.</td></tr>
<tr><td><code>beats 3</code></td><td>beats per bar (3 = 3/4).</td></tr>
<tr><td><code>grid 3</code></td><td>steps per beat: 4 = sixteenths, 3 = triplets, 8 = 32nds. Patterns keep their place in time.</td></tr>
<tr><td><code>swing 0.67</code></td><td>delay the off-beat eighths: 0.5 straight, 0.67 triplet feel.</td></tr>
<tr><td><code>key F# minor</code></td><td>key and mode (also <code>key Bbm</code>, <code>key C</code>). Affects degree entry, chords, random melodies and the piano-roll shading.</td></tr>
<tr><td><code>prog 1 5 6 4</code></td><td>chord progression by scale degree, one per bar, repeating. Random melodies follow it.</td></tr>
<tr><td><code>add pluck</code></td><td>new track named pluck. Its TRK nodes appear in the graph; wire them up.</td></tr>
<tr><td><code>del</code> · <code>name X</code></td><td>delete / rename the current track.</td></tr>
<tr><td><code>voice</code></td><td>build a playable synth for the current track: TRK freq → OSC saw → FILT lp → × ENV(gate) → × vel → × 0.3, mixed into OUT. Then tweak it.</td></tr>
<tr><td><code>new</code></td><td>empty project: one track, an empty graph with just OUT and the track's TRK nodes. The app starts like this.</td></tr>
<tr><td><code>sampleproject</code></td><td>replace the project with the demo song (lead, bass, drums routed with KEY nodes, pad) whose graph uses every node kind. <kbd>u</kbd> undoes it.</td></tr>
<tr><td><code>help</code></td><td>this page.</td></tr>
</table>

<h2 id="nodes">Nodes</h2>
{nodes}

<h2 id="files">Files</h2>
<p><kbd>Cmd+S</kbd> saves (Shift for save-as), <kbd>Cmd+O</kbd> opens. A song is a single JSON file (<code>.dd</code>) holding the graph, the tracks and their patterns, so it diffs and versions well.</p>
</body></html>"##)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_serves_every_node() {
        let port = start();
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut body = String::new();
        s.read_to_string(&mut body).unwrap();
        assert!(body.starts_with("HTTP/1.1 200"));
        for k in ALL { assert!(body.contains(&format!("<h3>{}</h3>", k.name())), "{} missing", k.name()) }
        assert!(body.contains("bpm 128"));
    }
}
