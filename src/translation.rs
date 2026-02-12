use log::{error, info, warn};
use std::sync::mpsc;
use std::thread;

use crate::chat::TextSegment;

// ─── Request / Response types ────────────────────────────────────────

pub struct TranslationRequest {
    pub message_id: u64,
    pub text: String,
    pub link_names: Vec<String>,
    pub source_lang: Option<String>,
    pub target_lang: Option<String>,
}

pub enum TranslationResponse {
    Success { message_id: u64, translated: String },
    Error { message_id: u64, error: String },
    Languages(Vec<(String, String)>),
    LanguagesError(String),
}

#[derive(Clone)]
pub enum TranslationEntry {
    Pending,
    Done(String),
    Error(String),
}

// ─── Shutdown sentinel ───────────────────────────────────────────────

enum WorkItem {
    Translate(TranslationRequest),
    FetchLanguages,
    Shutdown,
}

// ─── WoW link placeholder logic ─────────────────────────────────────

/// Build a translatable string from text segments, replacing WoW links
/// with numbered placeholders that DeepL will preserve.
///
/// Returns (text_with_placeholders, ordered_link_display_names).
pub fn prepare_for_translation(segments: &[TextSegment]) -> (String, Vec<String>) {
    let mut text = String::new();
    let mut link_names = Vec::new();

    for seg in segments {
        match seg {
            TextSegment::Plain(s) => text.push_str(s),
            TextSegment::WowLink { display_name, .. } => {
                link_names.push(display_name.clone());
                // Fullwidth angle brackets — DeepL treats these as non-translatable tokens
                text.push_str(&format!("\u{3008}{}\u{3009}", link_names.len()));
            }
        }
    }

    (text, link_names)
}

/// Restore WoW link names from numbered placeholders back into [Name] format.
fn restore_links(translated: &str, link_names: &[String]) -> String {
    let mut result = translated.to_string();
    for (i, name) in link_names.iter().enumerate() {
        let placeholder = format!("\u{3008}{}\u{3009}", i + 1);
        result = result.replace(&placeholder, &format!("[{}]", name));
    }
    result
}

// ─── Translation service ─────────────────────────────────────────────

pub struct TranslationService {
    work_tx: mpsc::Sender<WorkItem>,
    _handle: thread::JoinHandle<()>,
}

impl TranslationService {
    /// Start the background translation thread.
    /// Returns (service, response_receiver).
    pub fn start(
        api_key: String,
        target_lang: String,
    ) -> (Self, mpsc::Receiver<TranslationResponse>) {
        let (work_tx, work_rx) = mpsc::channel::<WorkItem>();
        let (resp_tx, resp_rx) = mpsc::channel::<TranslationResponse>();

        let handle = thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to create tokio runtime: {}", e);
                    return;
                }
            };

            let api = deepl::DeepLApi::with(&api_key).new();
            info!(
                "Translation service started (target: {})",
                target_lang
            );

            rt.block_on(async {
                while let Ok(item) = work_rx.recv() {
                    match item {
                        WorkItem::Shutdown => {
                            info!("Translation service shutting down");
                            break;
                        }
                        WorkItem::FetchLanguages => {
                            match api.languages(deepl::LangType::Target).await {
                                Ok(langs) => {
                                    let pairs: Vec<(String, String)> = langs
                                        .into_iter()
                                        .map(|l| (l.language, l.name))
                                        .collect();
                                    info!("Fetched {} target languages", pairs.len());
                                    let _ = resp_tx.send(TranslationResponse::Languages(pairs));
                                }
                                Err(e) => {
                                    let msg = format_deepl_error(&e);
                                    error!("Failed to fetch languages: {}", msg);
                                    let _ = resp_tx.send(TranslationResponse::LanguagesError(msg));
                                }
                            }
                        }
                        WorkItem::Translate(req) => {
                            let effective_target = req.target_lang.as_deref().unwrap_or(&target_lang);
                            let lang: deepl::Lang = match std::str::FromStr::from_str(effective_target) {
                                Ok(l) => l,
                                Err(_) => {
                                    let msg = format!(
                                        "Invalid target language code: {}",
                                        effective_target
                                    );
                                    warn!("{}", msg);
                                    let _ = resp_tx.send(TranslationResponse::Error {
                                        message_id: req.message_id,
                                        error: msg,
                                    });
                                    continue;
                                }
                            };

                            let source_lang: Option<deepl::Lang> = if let Some(ref src) = req.source_lang {
                                if src.is_empty() {
                                    None // empty = auto-detect
                                } else {
                                    match std::str::FromStr::from_str(src) {
                                        Ok(l) => Some(l),
                                        Err(_) => {
                                            let msg = format!("Invalid source language code: {}", src);
                                            warn!("{}", msg);
                                            let _ = resp_tx.send(TranslationResponse::Error {
                                                message_id: req.message_id,
                                                error: msg,
                                            });
                                            continue;
                                        }
                                    }
                                }
                            } else {
                                None
                            };

                            let mut builder = api.translate_text(req.text.as_str(), lang);
                            if let Some(src) = source_lang {
                                builder.source_lang(src);
                            }

                            match (&mut builder).await {
                                Ok(resp) => {
                                    if let Some(sentence) = resp.translations.first() {
                                        let translated = if req.link_names.is_empty() {
                                            sentence.text.clone()
                                        } else {
                                            restore_links(&sentence.text, &req.link_names)
                                        };
                                        let _ = resp_tx.send(TranslationResponse::Success {
                                            message_id: req.message_id,
                                            translated,
                                        });
                                    } else {
                                        let _ = resp_tx.send(TranslationResponse::Error {
                                            message_id: req.message_id,
                                            error: "No translation returned".into(),
                                        });
                                    }
                                }
                                Err(e) => {
                                    let msg = format_deepl_error(&e);
                                    error!(
                                        "Translation error for msg {}: {}",
                                        req.message_id, msg
                                    );
                                    let _ = resp_tx.send(TranslationResponse::Error {
                                        message_id: req.message_id,
                                        error: msg,
                                    });
                                }
                            }
                        }
                    }
                }
            });

            info!("Translation service thread exiting");
        });

        let service = TranslationService {
            work_tx: work_tx.clone(),
            _handle: handle,
        };

        (service, resp_rx)
    }

    /// Send a translation request.
    pub fn translate(&self, request: TranslationRequest) -> bool {
        self.work_tx.send(WorkItem::Translate(request)).is_ok()
    }

    /// Request the list of available target languages.
    pub fn fetch_languages(&self) -> bool {
        self.work_tx.send(WorkItem::FetchLanguages).is_ok()
    }

    /// Shut down the background thread.
    pub fn shutdown(&self) {
        let _ = self.work_tx.send(WorkItem::Shutdown);
    }
}

impl Drop for TranslationService {
    fn drop(&mut self) {
        let _ = self.work_tx.send(WorkItem::Shutdown);
    }
}

// ─── Error formatting ────────────────────────────────────────────────

fn format_deepl_error(e: &deepl::Error) -> String {
    let s = format!("{}", e);
    // Provide user-friendly messages for common HTTP errors
    if s.contains("403") {
        "Invalid API key".into()
    } else if s.contains("429") {
        "Rate limit exceeded, please wait".into()
    } else if s.contains("456") {
        "Translation quota exceeded".into()
    } else {
        s
    }
}
