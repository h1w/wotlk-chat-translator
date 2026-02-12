use log::error;

pub struct ClipboardHelper {
    clipboard: arboard::Clipboard,
}

impl ClipboardHelper {
    pub fn new() -> Option<Self> {
        match arboard::Clipboard::new() {
            Ok(clipboard) => Some(Self { clipboard }),
            Err(e) => {
                error!("Failed to initialize clipboard: {}", e);
                None
            }
        }
    }

    pub fn copy(&mut self, text: &str) {
        if let Err(e) = self.clipboard.set_text(text) {
            error!("Failed to copy to clipboard: {}", e);
        }
    }
}

/// imgui clipboard backend backed by arboard.
/// Enables Ctrl+V paste in all imgui input fields.
pub struct ImguiClipboardBackend {
    clipboard: arboard::Clipboard,
}

impl ImguiClipboardBackend {
    pub fn new() -> Option<Self> {
        arboard::Clipboard::new()
            .ok()
            .map(|clipboard| Self { clipboard })
    }
}

impl imgui::ClipboardBackend for ImguiClipboardBackend {
    fn get(&mut self) -> Option<String> {
        self.clipboard.get_text().ok()
    }

    fn set(&mut self, value: &str) {
        let _ = self.clipboard.set_text(value);
    }
}
