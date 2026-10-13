//! The models NexaOS knows how to fetch. Each is pinned by size and SHA-256:
//! a download that doesn't match exactly is thrown away. Only permissively
//! licensed models. Measured speeds are in docs/NEXA_MIND.md.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    Chat,
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
}

pub const MODELS: &[Model] = &[Model {
    name: "qwen2.5-1.5b",
    title: "Qwen2.5 1.5B Instruct (fast, for 8 GB laptops)",
    file: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
    url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
    size: 1_117_320_736,
    sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
    licence: "Apache-2.0",
    role: Role::Chat,
    ram_mb: 1900,
}];

pub const DEFAULT_CHAT: &str = "qwen2.5-1.5b";

pub fn find(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_well_formed() {
        assert!(find(DEFAULT_CHAT).is_some());
        for m in MODELS {
            assert_eq!(m.sha256.len(), 64, "{}", m.name);
            assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(m.url.starts_with("https://"), "{}: downloads must be https", m.name);
            assert!(m.file.ends_with(".gguf") && !m.file.contains('/'));
            assert!(["Apache-2.0", "MIT"].contains(&m.licence), "{}: permissive licences only", m.name);
        }
    }
}
