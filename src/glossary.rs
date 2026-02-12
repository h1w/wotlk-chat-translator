use log::{error, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config;

// ─── Data structures ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlossaryEntry {
    pub keys: Vec<String>,
    pub description_en: String,
    pub description_ru: String,
}

pub struct Glossary {
    pub entries: Vec<GlossaryEntry>,
    lookup: HashMap<String, usize>,
}

// ─── File path ──────────────────────────────────────────────────────

fn glossary_path() -> std::path::PathBuf {
    config::config_dir().join("glossary.json")
}

// ─── Implementation ─────────────────────────────────────────────────

impl Glossary {
    pub fn load() -> Self {
        let path = glossary_path();
        let entries: Vec<GlossaryEntry> = match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Vec<GlossaryEntry>>(&content) {
                Ok(entries) => {
                    info!("Loaded glossary from {} ({} entries)", path.display(), entries.len());
                    entries
                }
                Err(e) => {
                    error!("Failed to parse glossary JSON: {}", e);
                    load_bundled_glossary()
                }
            },
            Err(_) => {
                info!("No glossary file at {}, trying bundled assets", path.display());
                let entries = load_bundled_glossary();
                if !entries.is_empty() {
                    // Save bundled glossary to config dir for future edits
                    let g = Glossary {
                        lookup: HashMap::new(),
                        entries: entries.clone(),
                    };
                    g.save();
                }
                entries
            }
        };

        let mut g = Glossary {
            entries,
            lookup: HashMap::new(),
        };
        g.rebuild_lookup();
        g
    }

    pub fn save(&self) {
        let path = glossary_path();
        match serde_json::to_string_pretty(&self.entries) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    error!("Failed to write glossary: {}", e);
                }
            }
            Err(e) => error!("Failed to serialize glossary: {}", e),
        }
    }

    pub fn rebuild_lookup(&mut self) {
        self.lookup.clear();
        for (idx, entry) in self.entries.iter().enumerate() {
            for key in &entry.keys {
                self.lookup.insert(key.to_lowercase(), idx);
            }
        }
    }

    pub fn lookup_word(&self, word: &str, lang: &str) -> Option<&str> {
        self.lookup.get(&word.to_lowercase()).map(|&idx| {
            let entry = &self.entries[idx];
            match lang {
                "RU" if !entry.description_ru.is_empty() => entry.description_ru.as_str(),
                _ => entry.description_en.as_str(),
            }
        })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

const BUNDLED_GLOSSARY: &str = include_str!("../assets/glossary.json");

fn load_bundled_glossary() -> Vec<GlossaryEntry> {
    match serde_json::from_str::<Vec<GlossaryEntry>>(BUNDLED_GLOSSARY) {
        Ok(entries) => {
            info!("Loaded bundled glossary ({} entries)", entries.len());
            entries
        }
        Err(e) => {
            error!("Failed to parse bundled glossary: {}", e);
            Vec::new()
        }
    }
}

// ─── Tokenizer ──────────────────────────────────────────────────────

/// Split text into (token, is_word) pairs by word boundaries.
/// Words are sequences of alphanumeric/underscore characters.
/// Returns slices into the original string (zero-copy).
pub fn tokenize(text: &str) -> Vec<(&str, bool)> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some(&(start, ch)) = chars.peek() {
        if is_word_char(ch) {
            // Consume word characters
            let mut end = start + ch.len_utf8();
            chars.next();
            while let Some(&(_, c)) = chars.peek() {
                if is_word_char(c) {
                    end += c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push((&text[start..end], true));
        } else {
            // Consume non-word characters
            let mut end = start + ch.len_utf8();
            chars.next();
            while let Some(&(_, c)) = chars.peek() {
                if !is_word_char(c) {
                    end += c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push((&text[start..end], false));
        }
    }

    tokens
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
