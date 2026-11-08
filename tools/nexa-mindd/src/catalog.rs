//! The models NexaOS knows how to fetch. Each file is pinned by size and
//! SHA-256: a download that doesn't match exactly is thrown away. Only
//! permissively licensed models. Measured speeds are in docs/NEXA_MIND.md.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    /// Answers questions (llama-server).
    Chat,
    /// Hears you: speech to text (whisper-cli).
    Listen,
    /// Speaks: text to speech (nexa-speak).
    Voice,
}

/// A small file that comes with a model (Kokoro's voices), checked the
/// same way.
#[derive(Debug, Clone, Copy)]
pub struct Extra {
    pub file: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct Model {
    pub name: &'static str,
    pub title: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub licence: &'static str,
    pub role: Role,
    /// Memory the model needs loaded (weights + an 8k context), roughly.
    pub ram_mb: u64,
    pub extras: &'static [Extra],
}

/// A file of Kokoro-82M v1.0 (ONNX export), at a fixed revision.
macro_rules! concat_kokoro {
    ($path:literal) => {
        concat!("https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/1939ad2a8e416c0acfeecc08a694d14ef25f2231/", $path)
    };
}

/// Kokoro's voices: <name>.bin, 510 x 256 floats. The first is the default.
pub const VOICES: &[Extra] = &[
    Extra { file: "kokoro-af_heart.bin", url: concat_kokoro!("voices/af_heart.bin"), size: 522_240,
            sha256: "d583ccff3cdca2f7fae535cb998ac07e9fcb90f09737b9a41fa2734ec44a8f0b" },
    Extra { file: "kokoro-af_bella.bin", url: concat_kokoro!("voices/af_bella.bin"), size: 522_240,
            sha256: "f69d836209b78eb8c66e75e3cda491e26ea838a3674257e9d4e5703cbaf55c8b" },
    Extra { file: "kokoro-am_michael.bin", url: concat_kokoro!("voices/am_michael.bin"), size: 522_240,
            sha256: "1d1f21dd8da39c30705cd4c75d039d265e9bc4a2a93ed09bc9e1b1225eb95ba1" },
    Extra { file: "kokoro-bf_emma.bin", url: concat_kokoro!("voices/bf_emma.bin"), size: 522_240,
            sha256: "669fe0647f9dd04fcab92f1439a40eeb4c8b4ab1f82e4996fe3d918ce4a63b73" },
];

pub const MODELS: &[Model] = &[
    Model {
        name: "qwen2.5-1.5b",
        title: "Qwen2.5 1.5B Instruct (fast, for 8 GB laptops)",
        file: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        size: 1_117_320_736,
        sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
        licence: "Apache-2.0",
        role: Role::Chat,
        ram_mb: 1900,
        extras: &[],
    },
    Model {
        name: "whisper-base.en",
        title: "Whisper base (English): hears what you say",
        file: "ggml-base.en-q8_0.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.en-q8_0.bin",
        size: 81_781_811,
        sha256: "a4d4a0768075e13cfd7e19df3ae2dbc4a68d37d36a7dad45e8410c9a34f8c87e",
        licence: "MIT",
        role: Role::Listen,
        ram_mb: 250,
        extras: &[],
    },
    Model {
        name: "kokoro-82m",
        title: "Kokoro 82M: Nexa's voice (four voices)",
        file: "kokoro-82m-v1.0.onnx",
        url: concat_kokoro!("onnx/model.onnx"),
        size: 325_532_232,
        sha256: "8fbea51ea711f2af382e88c833d9e288c6dc82ce5e98421ea61c058ce21a34cb",
        licence: "Apache-2.0",
        role: Role::Voice,
        ram_mb: 450,
        extras: VOICES,
    },
];

pub const DEFAULT_CHAT: &str = "qwen2.5-1.5b";
pub const DEFAULT_LISTEN: &str = "whisper-base.en";
pub const DEFAULT_VOICE: &str = "kokoro-82m";
/// The voice used unless you pick another (Settings > Nexa Mind).
pub const DEFAULT_VOICE_NAME: &str = "af_heart";

pub fn find(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

/// A voice by its name (af_heart), if the catalogue has it.
pub fn voice(name: &str) -> Option<&'static Extra> {
    VOICES.iter().find(|v| v.file.strip_prefix("kokoro-").and_then(|f| f.strip_suffix(".bin")) == Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned(file: &str, url: &str, sha: &str) {
        assert_eq!(sha.len(), 64, "{file}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(url.starts_with("https://"), "{file}: downloads must be https");
        assert!(!url.contains("/resolve/main/") || file.ends_with(".gguf"), "{file}: pinned to a revision");
        assert!(!file.contains('/') && !file.starts_with('.'));
    }

    #[test]
    fn catalogue_is_well_formed() {
        assert!(voice(DEFAULT_VOICE_NAME).is_some());
        for name in [DEFAULT_CHAT, DEFAULT_LISTEN, DEFAULT_VOICE] {
            assert!(find(name).is_some(), "{name}");
        }
        for m in MODELS {
            pinned(m.file, m.url, m.sha256);
            for e in m.extras {
                pinned(e.file, e.url, e.sha256);
            }
            let ext = match m.role {
                Role::Chat => ".gguf",
                Role::Listen => ".bin",
                Role::Voice => ".onnx",
            };
            assert!(m.file.ends_with(ext), "{}", m.name);
            assert!(["Apache-2.0", "MIT"].contains(&m.licence), "{}: permissive licences only", m.name);
        }
        let mut files: Vec<&str> = MODELS.iter().map(|m| m.file).chain(VOICES.iter().map(|v| v.file)).collect();
        files.sort();
        files.dedup();
        assert_eq!(files.len(), MODELS.len() + VOICES.len(), "every file name is its own");
    }

    #[test]
    fn voices_by_name() {
        assert_eq!(voice("af_heart").unwrap().file, "kokoro-af_heart.bin");
        assert!(voice("af").is_none() && voice("../x").is_none() && voice("").is_none());
    }
}
