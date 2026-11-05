//! nexa-intent: NexaOS's tiny command model. It reads a short command
//! ("make the screen darker", "where did I put my CV", "open fierfox")
//! and says which of about 60 intents it is, how sure it is, and which
//! words are the name in it (an app, a file, a workspace). About 1 MB, a
//! fraction of a millisecond per command, no download: trained from
//! sentences made by `data.rs` (`nexa-intent-train`), measured on
//! hand-written ones it never saw (tests/heldout.tsv).
//!
//! It only understands; MicroBrain (ZeroSearch) turns the intent into one
//! of the desktop's typed actions, and refuses anything that destroys.

pub mod data;
pub mod features;
pub mod model;
pub mod rng;
pub mod vocab;

pub use model::{Model, Prediction};

/// The trained model, built into the program.
#[cfg(feature = "builtin")]
pub fn builtin() -> &'static Model {
    use std::sync::OnceLock;
    static M: OnceLock<Model> = OnceLock::new();
    M.get_or_init(|| Model::from_bytes(include_bytes!("../model/nexa-intent.bin")).expect("the built-in command model is damaged"))
}

/// A number in the words: digits, or a few number words ("fifty", "half").
pub fn number(ws: &[String]) -> Option<u32> {
    const W: &[(&str, u32)] = &[("zero", 0), ("one", 1), ("two", 2), ("three", 3), ("four", 4), ("five", 5), ("six", 6), ("seven", 7),
                                ("eight", 8), ("nine", 9), ("ten", 10), ("twenty", 20), ("quarter", 25), ("thirty", 30), ("forty", 40),
                                ("fifty", 50), ("half", 50), ("sixty", 60), ("seventy", 70), ("eighty", 80), ("ninety", 90),
                                ("hundred", 100), ("full", 100), ("max", 100)];
    ws.iter().find_map(|w| w.parse::<u32>().ok().or_else(|| W.iter().find(|(n, _)| n == w).map(|(_, v)| *v)))
}
