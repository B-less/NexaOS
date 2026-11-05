//! nexa-intent-train: make the sentences, train the model, measure it,
//! write model/nexa-intent.bin.
//!
//!   nexa-intent-train            train and write the model
//!   nexa-intent-train --check    train and compare with the file (the
//!                                same seed must give the same bytes)
//!   nexa-intent-train --eval     only measure the model in the file
//!   nexa-intent-train --ask TEXT what the model in the file makes of TEXT
//!
//! Measured on three sets: a slice of generated sentences it didn't train
//! on (seed 2), and tests/heldout.tsv, written by hand.

use nexa_intent::data::{generate, INTENTS};
use nexa_intent::model::{takes_arg, Model, SURE};
use std::path::PathBuf;
use std::time::Instant;

const PER_INTENT: usize = 900;
const EPOCHS: usize = 10;
const SEED: u64 = 1;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// (intent, sentence, name) from a hand-written set.
fn hand(file: &str) -> Vec<(String, String, Option<String>)> {
    std::fs::read_to_string(root().join("tests").join(file)).unwrap_or_default().lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some((f.next()?.to_string(), f.next()?.to_string(), f.next().map(str::to_string)))
        })
        .collect()
}

fn measure(m: &Model) -> bool {
    let val = generate(60, 2, true);
    let right = val.iter().filter(|e| m.predict(&e.text).intent == INTENTS[e.intent].0).count();
    println!("generated, not trained on: {right} of {} right ({:.1} %)", val.len(), 100.0 * right as f64 / val.len() as f64);
    let mut all_good = true;
    for file in ["heldout.tsv", "fresh.tsv", "outside.tsv"] {
        all_good &= measure_set(m, file);
    }
    all_good
}

fn measure_set(m: &Model, file: &str) -> bool {
    let h = hand(file);
    let (mut ok, mut named, mut named_ok, mut wrong) = (0, 0, 0, vec![]);
    let t0 = Instant::now();
    for (want, text, name) in &h {
        let p = m.predict(text);
        if &p.intent == want {
            ok += 1;
        } else {
            wrong.push(format!("  \"{text}\": {} ({:.2}), wanted {want}", p.intent, p.p));
        }
        if let Some(n) = name {
            named += 1;
            if p.arg.as_deref() == Some(n.as_str()) {
                named_ok += 1;
            } else if takes_arg(want) {
                wrong.push(format!("  \"{text}\": name {:?}, wanted {n:?}", p.arg));
            }
        }
    }
    let per = t0.elapsed().as_secs_f64() * 1e6 / h.len().max(1) as f64;
    println!("{file}: {ok} of {} intents right ({:.1} %), {named_ok} of {named} names", h.len(), 100.0 * ok as f64 / h.len().max(1) as f64);
    println!("time per command: {per:.0} µs");
    for w in &wrong {
        println!("{w}");
    }
    // Never mistaken for an action: a delete request is always refused, and
    // chat never does anything (a name lookup that finds nothing is harmless:
    // "find my keys" looks for a file called keys).
    let lookup = |i: &str| matches!(i, "launch" | "find_file" | "workspace_go");
    let bad = h.iter().filter(|(want, text, _)| {
        let got = m.predict(text).intent;
        (want == "destroy" && got != "destroy" && got != "unknown") || (want == "unknown" && got != "unknown" && got != "destroy" && !lookup(&got))
    }).count();
    println!("delete/chat sentences taken for an action: {bad}");
    ok * 100 >= h.len() * 90 && bad == 0
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    let path = root().join("model/nexa-intent.bin");
    if arg == "--ask" {
        let m = Model::from_bytes(&std::fs::read(&path).expect("no model file")).expect("damaged model file");
        for q in std::env::args().skip(2) {
            let p = m.predict(&q);
            println!("{q}: {} (raw {} {:.2}, then {} {:.2}) name {:?}", p.intent, p.raw, p.p, p.second, p.p2, p.arg);
        }
        return;
    }
    if arg == "--eval" {
        let m = Model::from_bytes(&std::fs::read(&path).expect("no model file")).expect("damaged model file");
        std::process::exit(if measure(&m) { 0 } else { 1 });
    }
    let t0 = Instant::now();
    let data = generate(PER_INTENT, SEED, true);
    let n = data.len();
    let m = Model::train(&data, EPOCHS, SEED);
    let bytes = m.to_bytes();
    println!("trained on {n} sentences ({} intents) in {:.1} s; model {} bytes", INTENTS.len(), t0.elapsed().as_secs_f64(), bytes.len());
    // Measure the model as stored (8-bit), not the training copy.
    let stored = Model::from_bytes(&bytes).expect("the model reads back");
    let good = measure(&stored);
    if arg == "--check" {
        let same = std::fs::read(&path).map(|b| b == bytes).unwrap_or(false);
        println!("{}", if same { "the file matches a fresh training" } else { "the file differs from a fresh training" });
        std::process::exit(if same && good { 0 } else { 1 });
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    println!("wrote {}", path.display());
    std::process::exit(if good { 0 } else { 1 });
}
