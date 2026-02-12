use log::{debug, info, trace, warn};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::memory::ProcessMemoryReader;
use crate::offsets;

static NEXT_MESSAGE_ID: AtomicU64 = AtomicU64::new(1);

// ─── Message Type ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatMessageType {
    Addon,
    Say,
    Party,
    Raid,
    Guild,
    Officer,
    Yell,
    Whisper,
    WhisperMob,
    WhisperInform,
    Emote,
    TextEmote,
    MonsterSay,
    MonsterParty,
    MonsterYell,
    MonsterWhisper,
    MonsterEmote,
    Channel,
    ChannelJoin,
    ChannelLeave,
    ChannelList,
    ChannelNotice,
    ChannelNoticeUser,
    Afk,
    Dnd,
    Ignored,
    Skill,
    Loot,
    System,
    Unknown(u32),
}

impl ChatMessageType {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Addon,
            1 => Self::Say,
            2 => Self::Party,
            3 => Self::Raid,
            4 => Self::Guild,
            5 => Self::Officer,
            6 => Self::Yell,
            7 => Self::Whisper,
            8 => Self::WhisperMob,
            9 => Self::WhisperInform,
            10 => Self::Emote,
            11 => Self::TextEmote,
            12 => Self::MonsterSay,
            13 => Self::MonsterParty,
            14 => Self::MonsterYell,
            15 => Self::MonsterWhisper,
            16 => Self::MonsterEmote,
            17 => Self::Channel,
            18 => Self::ChannelJoin,
            19 => Self::ChannelLeave,
            20 => Self::ChannelList,
            21 => Self::ChannelNotice,
            22 => Self::ChannelNoticeUser,
            23 => Self::Afk,
            24 => Self::Dnd,
            25 => Self::Ignored,
            26 => Self::Skill,
            27 => Self::Loot,
            28 => Self::System,
            other => Self::Unknown(other),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Addon => "Addon",
            Self::Say => "Say",
            Self::Party => "Party",
            Self::Raid => "Raid",
            Self::Guild => "Guild",
            Self::Officer => "Officer",
            Self::Yell => "Yell",
            Self::Whisper => "Whisper",
            Self::WhisperMob => "Whisper",
            Self::WhisperInform => "To",
            Self::Emote => "Emote",
            Self::TextEmote => "Emote",
            Self::MonsterSay => "Say",
            Self::MonsterParty => "Party",
            Self::MonsterYell => "Yell",
            Self::MonsterWhisper => "Whisper",
            Self::MonsterEmote => "Emote",
            Self::Channel => "Channel",
            Self::ChannelJoin => "Channel",
            Self::ChannelLeave => "Channel",
            Self::ChannelList => "Channel",
            Self::ChannelNotice => "Channel",
            Self::ChannelNoticeUser => "Channel",
            Self::Afk => "AFK",
            Self::Dnd => "DND",
            Self::Ignored => "Ignored",
            Self::Skill => "Skill",
            Self::Loot => "Loot",
            Self::System => "System",
            Self::Unknown(_) => "???",
        }
    }

    pub fn color(&self) -> [f32; 4] {
        match self {
            Self::Say | Self::MonsterSay => [1.0, 1.0, 1.0, 1.0],
            Self::Yell | Self::MonsterYell => [1.0, 0.25, 0.25, 1.0],
            Self::Party | Self::MonsterParty => [0.4, 0.6, 1.0, 1.0],
            Self::Raid => [1.0, 0.5, 0.0, 1.0],
            Self::Guild => [0.25, 1.0, 0.25, 1.0],
            Self::Officer => [0.25, 0.75, 0.25, 1.0],
            Self::Whisper | Self::WhisperMob | Self::WhisperInform => [1.0, 0.5, 1.0, 1.0],
            Self::MonsterWhisper => [1.0, 0.5, 1.0, 1.0],
            Self::Channel => [1.0, 0.75, 0.5, 1.0],
            Self::Emote | Self::TextEmote | Self::MonsterEmote => [1.0, 0.5, 0.25, 1.0],
            Self::System => [1.0, 1.0, 0.0, 1.0],
            Self::Loot => [0.0, 0.8, 0.0, 1.0],
            Self::Skill => [0.3, 0.3, 1.0, 1.0],
            Self::Afk | Self::Dnd => [1.0, 1.0, 0.0, 1.0],
            _ => [0.7, 0.7, 0.7, 1.0],
        }
    }
}

// ─── WoW Link / Rich Text Types ─────────────────────────────────────

#[derive(Debug, Clone)]
pub enum WowLinkType {
    Item(u32),
    Spell(u32),
    Achievement(u32),
    Quest(u32),
    Trade(u32),
    Other,
}

impl WowLinkType {
    /// Returns a Wowhead URL for this link. Known types with a valid ID get a direct link;
    /// unknown types or id=0 fall back to a search query using the display name.
    pub fn wowhead_url(&self, display_name: &str) -> String {
        match self {
            WowLinkType::Item(id) if *id > 0 => format!("https://www.wowhead.com/item={}", id),
            WowLinkType::Spell(id) if *id > 0 => format!("https://www.wowhead.com/spell={}", id),
            WowLinkType::Achievement(id) if *id > 0 => {
                format!("https://www.wowhead.com/achievement={}", id)
            }
            WowLinkType::Quest(id) if *id > 0 => {
                format!("https://www.wowhead.com/wotlk/quest={}", id)
            }
            WowLinkType::Trade(id) if *id > 0 => {
                format!("https://www.wowhead.com/wotlk/skill={}", id)
            }
            _ => format!("https://www.wowhead.com/search?q={}", url_encode(display_name)),
        }
    }
}

/// Percent-encode a string for use in a URL query parameter.
fn url_encode(s: &str) -> String {
    let mut result = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

#[derive(Debug, Clone)]
pub enum TextSegment {
    Plain(String),
    WowLink {
        link_type: WowLinkType,
        display_name: String,
        color: [f32; 4],
    },
}

// ─── Chat Message ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: u64,
    pub sender_guid: u64,
    pub sender_name: String,
    pub text: String,
    pub formatted: String,
    pub message_type: ChatMessageType,
    pub channel_number: u32,
    pub channel_name: String,
    pub timestamp: u32,
    pub segments: Vec<TextSegment>,
}

impl ChatMessage {
    pub fn from_raw_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < offsets::CHAT_MESSAGE_STRIDE {
            warn!(
                "from_raw_bytes: data too short ({} < {})",
                data.len(),
                offsets::CHAT_MESSAGE_STRIDE
            );
            return None;
        }

        let sender_guid = read_u64(data, offsets::MSG_SENDER_GUID);
        let formatted = read_cstring(data, offsets::MSG_FORMATTED, offsets::MSG_STRING_MAX_LEN);
        let raw_text = read_cstring(data, offsets::MSG_PLAIN_TEXT, offsets::MSG_STRING_MAX_LEN);
        let text = strip_wow_formatting(&raw_text);
        let segments = parse_text_segments(&raw_text);
        let msg_type_raw = read_u32(data, offsets::MSG_TYPE);
        let channel_number = read_u32(data, offsets::MSG_CHANNEL_NUM);
        let timestamp = read_u32(data, offsets::MSG_TIMESTAMP);

        trace!(
            "  raw fields: guid=0x{:X} type_raw={} ch={} ts={} formatted_len={} text_len={}",
            sender_guid,
            msg_type_raw,
            channel_number,
            timestamp,
            formatted.len(),
            text.len(),
        );

        // Skip empty messages (unused buffer slots)
        if formatted.is_empty() && text.is_empty() {
            debug!("  skipping empty slot (both formatted and text are empty)");
            return None;
        }

        let message_type = ChatMessageType::from_u32(msg_type_raw);
        let sender_name = extract_sender_name(&formatted);
        let channel_name = extract_channel_name(&formatted, channel_number);

        if message_type == ChatMessageType::Channel {
            debug!(
                "  channel msg: ch_num={} ch_name=\"{}\" formatted=\"{}\"",
                channel_number,
                channel_name,
                truncate_for_log(&formatted, 300),
            );
        }

        debug!(
            "  parsed: type={:?} sender=\"{}\" ch_name=\"{}\" text=\"{}\"",
            message_type,
            sender_name,
            channel_name,
            truncate_for_log(&text, 80),
        );
        trace!("  formatted: \"{}\"", truncate_for_log(&formatted, 200));

        Some(ChatMessage {
            id: NEXT_MESSAGE_ID.fetch_add(1, Ordering::Relaxed),
            sender_guid,
            sender_name,
            text,
            formatted,
            message_type,
            channel_number,
            channel_name,
            timestamp,
            segments,
        })
    }

    /// Type label including channel name for channel messages.
    pub fn type_label(&self) -> String {
        match self.message_type {
            ChatMessageType::Channel
            | ChatMessageType::ChannelJoin
            | ChatMessageType::ChannelLeave
            | ChatMessageType::ChannelList
            | ChatMessageType::ChannelNotice
            | ChatMessageType::ChannelNoticeUser => {
                if !self.channel_name.is_empty() {
                    format!("Channel: {}", self.channel_name)
                } else {
                    format!("Channel: {}", self.channel_number)
                }
            }
            _ => self.message_type.label().to_string(),
        }
    }

    /// Prefix for display: "[Type] Name: " or "[Type] "
    pub fn display_prefix(&self) -> String {
        let label = self.type_label();
        if self.sender_name.is_empty() {
            format!("[{}] ", label)
        } else {
            format!("[{}] {}: ", label, self.sender_name)
        }
    }

    /// Formatted line for display: "[Type] Name: text"
    pub fn display_line(&self) -> String {
        let prefix = self.display_prefix();
        format!("{}{}", prefix, self.text)
    }

    /// Whether any segment is a WowLink (contains clickable item/spell links).
    pub fn has_links(&self) -> bool {
        self.segments.iter().any(|s| matches!(s, TextSegment::WowLink { .. }))
    }
}

// ─── Helpers ────────────────────────────────────────────────────────

fn read_u32(data: &[u8], offset: usize) -> u32 {
    data.get(offset..offset + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    data.get(offset..offset + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0)
}

fn read_cstring(data: &[u8], offset: usize, max_len: usize) -> String {
    if offset >= data.len() {
        return String::new();
    }
    let end = (offset + max_len).min(data.len());
    let slice = &data[offset..end];
    let null_pos = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..null_pos]).into_owned()
}

fn truncate_for_log(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    // Walk back from max to find a valid UTF-8 char boundary.
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Extract player name from FormattedMessage.
///
/// Patterns tried in order:
///   1. `Player Name: [NAME]`  (server metadata format)
///   2. `|Hplayer:NAME|h`      (standard WoW hyperlink)
fn extract_sender_name(formatted: &str) -> String {
    // Pattern 1: Server metadata — "Player Name: [NAME]"
    if let Some(start) = formatted.find("Player Name: [") {
        let name_start = start + "Player Name: [".len();
        if let Some(end) = formatted[name_start..].find(']') {
            let name = &formatted[name_start..name_start + end];
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    // Pattern 2: Standard WoW hyperlink — |Hplayer:NAME|h
    if let Some(start) = formatted.find("|Hplayer:") {
        let name_start = start + "|Hplayer:".len();
        if let Some(end) = formatted[name_start..].find('|') {
            let name = &formatted[name_start..name_start + end];
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    String::new()
}

/// Extract channel display name from FormattedMessage.
///
/// Tries multiple strategies:
///   1. Server metadata: `Channel: [NAME]`  (Warmane-style format)
///   2. WoW hyperlink:   `|Hchannel:...|h[NUM. NAME]|h`
///   3. Fallback to raw channel_number field
fn extract_channel_name(formatted: &str, channel_number: u32) -> String {
    // Strategy 1: Server metadata format — "Channel: [NAME]"
    if let Some(start) = formatted.find("Channel: [") {
        let name_start = start + "Channel: [".len();
        if let Some(end) = formatted[name_start..].find(']') {
            let name = &formatted[name_start..name_start + end];
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }

    // Strategy 2: WoW channel hyperlink — |Hchannel:...|h[DISPLAY]|h
    if let Some(start) = formatted.find("|Hchannel:") {
        if let Some(bracket_rel) = formatted[start..].find("|h[") {
            let name_start = start + bracket_rel + 3; // skip "|h["
            if let Some(end) = formatted[name_start..].find(']') {
                let name = &formatted[name_start..name_start + end];
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
    }

    // Strategy 3: Fallback to raw channel_number field.
    if channel_number > 0 {
        return format!("{}", channel_number);
    }
    String::new()
}

/// Parse a WoW hyperlink type string like "item:49908:0:0:..." into a WowLinkType.
fn parse_wow_link_type(data: &str) -> WowLinkType {
    let (kind, rest) = data.split_once(':').unwrap_or((data, ""));
    let id: u32 = rest
        .split(':')
        .next()
        .unwrap_or("")
        .parse()
        .unwrap_or(0);
    match kind {
        "item" => WowLinkType::Item(id),
        "spell" | "enchant" => WowLinkType::Spell(id),
        "achievement" => WowLinkType::Achievement(id),
        "quest" => WowLinkType::Quest(id),
        "trade" => WowLinkType::Trade(id),
        _ => WowLinkType::Other,
    }
}

/// Parse WoW formatted text into rich TextSegments with colors and clickable links.
///
/// Handles: |cffRRGGBB (color), |r (reset), |H...|h (link start), |h (link end), |T...|t (texture skip).
fn parse_text_segments(raw: &str) -> Vec<TextSegment> {
    let mut segments: Vec<TextSegment> = Vec::new();
    let mut current_text = String::new();
    let mut current_color: Option<[f32; 4]> = None;
    let mut pending_link: Option<WowLinkType> = None;
    let mut link_color: Option<[f32; 4]> = None;

    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '|' {
            match chars.peek().copied() {
                Some('c') | Some('C') => {
                    // |cffRRGGBB — set color
                    chars.next(); // consume 'c'
                    let hex: String = (&mut chars).take(8).collect();
                    if hex.len() == 8 {
                        // First 2 chars are alpha (usually ff), next 6 are RRGGBB
                        let r = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
                        let g = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
                        let b = u8::from_str_radix(&hex[6..8], 16).unwrap_or(255);
                        current_color =
                            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]);
                    }
                }
                Some('r') | Some('R') => {
                    // |r — reset color
                    chars.next();
                    current_color = None;
                }
                Some('H') => {
                    // |H...|h — hyperlink data, extract link type
                    chars.next(); // consume 'H'
                    // Flush any pending plain text before the link
                    if !current_text.is_empty() {
                        segments.push(TextSegment::Plain(std::mem::take(&mut current_text)));
                    }
                    let mut link_data = String::new();
                    while let Some(c) = chars.next() {
                        if c == '|' && chars.peek() == Some(&'h') {
                            chars.next(); // consume 'h'
                            break;
                        }
                        link_data.push(c);
                    }
                    pending_link = Some(parse_wow_link_type(&link_data));
                    link_color = current_color;
                }
                Some('h') => {
                    // |h — end of link display name
                    chars.next();
                    if let Some(lt) = pending_link.take() {
                        let display_name = std::mem::take(&mut current_text);
                        segments.push(TextSegment::WowLink {
                            link_type: lt,
                            display_name,
                            color: link_color.unwrap_or([1.0, 1.0, 1.0, 1.0]),
                        });
                        link_color = None;
                    }
                }
                Some('T') => {
                    // |T...|t — texture, skip entirely
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '|' && chars.peek() == Some(&'t') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {
                    // Unknown escape, keep the pipe
                    current_text.push(ch);
                }
            }
        } else {
            current_text.push(ch);
        }
    }

    // Flush remaining text
    if !current_text.is_empty() {
        segments.push(TextSegment::Plain(current_text));
    }

    segments
}

/// Strip WoW color codes, hyperlinks, and texture tags for clean display.
fn strip_wow_formatting(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '|' {
            match chars.peek().copied() {
                Some('c') | Some('C') => {
                    chars.next(); // 'c'
                    for _ in 0..8 {
                        chars.next();
                    } // AARRGGBB
                }
                Some('r') | Some('R') => {
                    chars.next();
                }
                Some('H') => {
                    // Skip |H...|h (hyperlink data)
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '|' && chars.peek() == Some(&'h') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some('h') => {
                    chars.next();
                }
                Some('T') => {
                    // Skip |T...|t (texture)
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '|' && chars.peek() == Some(&'t') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {
                    result.push(ch);
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

// ─── Chat Reader (buffer-scanning approach) ─────────────────────────
//
// Instead of relying on a single "count" address (which may be wrong for
// some WoW builds), we read the entire 60-slot buffer each poll and detect
// which slots changed by comparing a fingerprint (first 16 bytes of each
// entry).  New/changed slots are parsed and returned as new messages.

const FINGERPRINT_LEN: usize = 16;
const TOTAL_BUFFER_SIZE: usize = offsets::CHAT_BUFFER_SIZE * offsets::CHAT_MESSAGE_STRIDE;

pub struct ChatReader {
    fingerprints: [[u8; FINGERPRINT_LEN]; offsets::CHAT_BUFFER_SIZE],
    initialized: bool,
}

impl ChatReader {
    pub fn new() -> Self {
        Self {
            fingerprints: [[0u8; FINGERPRINT_LEN]; offsets::CHAT_BUFFER_SIZE],
            initialized: false,
        }
    }

    pub fn reset(&mut self) {
        info!("ChatReader reset");
        self.fingerprints = [[0u8; FINGERPRINT_LEN]; offsets::CHAT_BUFFER_SIZE];
        self.initialized = false;
    }

    /// Poll the chat buffer for new messages by scanning all 60 slots.
    pub fn poll(&mut self, reader: &dyn ProcessMemoryReader) -> io::Result<Vec<ChatMessage>> {
        // Read entire buffer in one syscall (~360 KB).
        let buffer = reader.read_memory(offsets::CHAT_BUFFER_START, TOTAL_BUFFER_SIZE)?;
        if buffer.len() < TOTAL_BUFFER_SIZE {
            warn!(
                "poll: buffer read returned {} bytes, expected {}",
                buffer.len(),
                TOTAL_BUFFER_SIZE,
            );
            return Ok(Vec::new());
        }

        // First poll — parse all existing messages instead of skipping.
        let is_first = !self.initialized;

        let mut new_messages: Vec<(u32, ChatMessage)> = Vec::new();

        for i in 0..offsets::CHAT_BUFFER_SIZE {
            let offset = i * offsets::CHAT_MESSAGE_STRIDE;
            let fp: [u8; FINGERPRINT_LEN] = buffer[offset..offset + FINGERPRINT_LEN]
                .try_into()
                .unwrap();

            if !is_first && fp == self.fingerprints[i] {
                continue; // Slot unchanged.
            }

            // Skip all-zero fingerprints (slot was cleared, not a real message).
            if fp == [0u8; FINGERPRINT_LEN] {
                self.fingerprints[i] = fp;
                continue;
            }

            debug!("  slot {} changed, fp={:02X?}", i, &fp);

            let slot_data = &buffer[offset..offset + offsets::CHAT_MESSAGE_STRIDE];
            match ChatMessage::from_raw_bytes(slot_data) {
                Some(msg) => {
                    let order_key = if msg.timestamp > 0 {
                        msg.timestamp
                    } else {
                        i as u32
                    };
                    new_messages.push((order_key, msg));
                }
                None => debug!("  slot {}: parsed as None (empty/invalid)", i),
            }

            self.fingerprints[i] = fp;
        }

        if is_first {
            self.initialized = true;
            info!(
                "poll: first sync — loaded {} existing messages from buffer",
                new_messages.len(),
            );
        }

        // Sort by timestamp so messages appear in chronological order.
        new_messages.sort_by_key(|(key, _)| *key);

        if !new_messages.is_empty() && is_first {
            info!("poll: first sync — loaded {} existing messages from buffer", new_messages.len());
        }

        Ok(new_messages.into_iter().map(|(_, msg)| msg).collect())
    }
}

// ─── Debug Scan ─────────────────────────────────────────────────────

/// Read diagnostic info from the chat buffer and log it.
/// Call this manually (via UI button) to diagnose offset issues.
pub fn debug_scan(reader: &dyn ProcessMemoryReader) {
    info!("=== DEBUG SCAN START ===");

    // 1. Try count addresses.
    for (label, addr) in [
        ("CHAT_BUFFER_COUNT", offsets::CHAT_BUFFER_COUNT),
        ("CHAT_BUFFER_COUNT_ALT", offsets::CHAT_BUFFER_COUNT_ALT),
    ] {
        match reader.read_memory(addr, 4) {
            Ok(data) if data.len() >= 4 => {
                let val = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                info!("  {} @ 0x{:X} = {} (0x{:X})", label, addr, val, val);
            }
            Ok(data) => warn!("  {} @ 0x{:X}: only {} bytes", label, addr, data.len()),
            Err(e) => warn!("  {} @ 0x{:X}: FAILED: {}", label, addr, e),
        }
    }

    // 2. Read entire buffer and summarize each slot.
    info!("  Reading entire buffer ({} bytes)...", TOTAL_BUFFER_SIZE);
    let buffer = match reader.read_memory(offsets::CHAT_BUFFER_START, TOTAL_BUFFER_SIZE) {
        Ok(b) if b.len() >= TOTAL_BUFFER_SIZE => b,
        Ok(b) => {
            warn!(
                "  Buffer read: only {} / {} bytes — buffer start 0x{:X} may be WRONG",
                b.len(),
                TOTAL_BUFFER_SIZE,
                offsets::CHAT_BUFFER_START,
            );
            info!("=== DEBUG SCAN END ===");
            return;
        }
        Err(e) => {
            warn!(
                "  Buffer read FAILED at 0x{:X}: {} — buffer start is WRONG",
                offsets::CHAT_BUFFER_START,
                e,
            );
            info!("=== DEBUG SCAN END ===");
            return;
        }
    };

    info!("  Buffer read OK. Scanning {} slots...", offsets::CHAT_BUFFER_SIZE);

    let mut populated = 0;
    for i in 0..offsets::CHAT_BUFFER_SIZE {
        let base = i * offsets::CHAT_MESSAGE_STRIDE;

        let guid = read_u64(&buffer, base + offsets::MSG_SENDER_GUID);
        let msg_type = read_u32(&buffer, base + offsets::MSG_TYPE);
        let channel = read_u32(&buffer, base + offsets::MSG_CHANNEL_NUM);
        let sequence = read_u32(&buffer, base + offsets::MSG_SEQUENCE);
        let timestamp = read_u32(&buffer, base + offsets::MSG_TIMESTAMP);
        let formatted = read_cstring(&buffer, base + offsets::MSG_FORMATTED, 80);
        let text = read_cstring(&buffer, base + offsets::MSG_PLAIN_TEXT, 80);

        let is_empty = guid == 0 && formatted.is_empty() && text.is_empty();

        if !is_empty {
            populated += 1;
            info!(
                "  slot {:2}: guid=0x{:X} type={} ch={} seq={} ts={} fmt=\"{}\" txt=\"{}\"",
                i,
                guid,
                msg_type,
                channel,
                sequence,
                timestamp,
                truncate_for_log(&formatted, 50),
                truncate_for_log(&text, 50),
            );
        }
    }
    info!(
        "  Summary: {}/{} slots populated",
        populated,
        offsets::CHAT_BUFFER_SIZE,
    );
    info!("=== DEBUG SCAN END ===");
}

// ─── Scan Analysis ───────────────────────────────────────────────────

/// Analyze addresses where a search string was found, looking for chat buffer patterns.
pub fn analyze_found_addresses(addresses: &[usize]) {
    info!("=== SCAN ANALYSIS ===");
    info!("{} matches found", addresses.len());

    // Log first 50 matches
    for (i, &addr) in addresses.iter().enumerate() {
        if i >= 50 {
            info!("  ... ({} more not shown)", addresses.len() - 50);
            break;
        }
        info!("  match {}: 0x{:08X}", i, addr);
    }

    // Check for pairs separated by the expected message stride (0x17C0)
    info!(
        "  Checking for stride-aligned pairs (stride=0x{:X}):",
        offsets::CHAT_MESSAGE_STRIDE,
    );
    let check_limit = addresses.len().min(200);
    let mut stride_pairs = 0;
    for i in 0..check_limit {
        for j in (i + 1)..check_limit {
            let diff = addresses[j].saturating_sub(addresses[i]);
            if diff > 0 && diff % offsets::CHAT_MESSAGE_STRIDE == 0 {
                let slots = diff / offsets::CHAT_MESSAGE_STRIDE;
                if slots <= offsets::CHAT_BUFFER_SIZE {
                    info!(
                        "    0x{:08X} -> 0x{:08X}: {} slots apart — STRIDE MATCH",
                        addresses[i], addresses[j], slots,
                    );
                    stride_pairs += 1;
                }
            }
        }
    }
    if stride_pairs == 0 {
        info!("  No stride-aligned pairs found among first {} results", check_limit);
    }

    // For small result sets, show possible buffer origins
    if addresses.len() <= 30 {
        info!(
            "  Possible origins (if in PlainText @ offset 0x{:X}):",
            offsets::MSG_PLAIN_TEXT,
        );
        for &addr in addresses {
            let msg_base = addr.wrapping_sub(offsets::MSG_PLAIN_TEXT);
            info!("    0x{:08X} -> msg_base=0x{:08X}", addr, msg_base);
        }
        info!(
            "  Possible origins (if in FormattedMsg @ offset 0x{:X}):",
            offsets::MSG_FORMATTED,
        );
        for &addr in addresses {
            let msg_base = addr.wrapping_sub(offsets::MSG_FORMATTED);
            info!("    0x{:08X} -> msg_base=0x{:08X}", addr, msg_base);
        }
    }

    info!("=== SCAN ANALYSIS END ===");
}

// ─── Chat Tabs (message-type filter groups) ─────────────────────────

pub struct ChatTab {
    pub name: String,
    /// None = show all messages (the "All" tab).
    pub filter: Option<Vec<ChatMessageType>>,
}

impl ChatTab {
    pub fn matches(&self, msg_type: ChatMessageType) -> bool {
        match &self.filter {
            None => true,
            Some(types) => types.contains(&msg_type),
        }
    }
}

/// Default filter tabs.
///
/// NOTE: These are NOT parsed from the WoW client.  WoW stores chat window
/// configuration in its Lua state / WTF config files, which are not
/// practically readable via external memory reading.  These are reasonable
/// defaults that mirror typical WoW chat tab layout.
pub fn default_tabs() -> Vec<ChatTab> {
    vec![
        ChatTab {
            name: "All".into(),
            filter: None,
        },
        ChatTab {
            name: "General".into(),
            filter: Some(vec![
                ChatMessageType::Say,
                ChatMessageType::Yell,
                ChatMessageType::Emote,
                ChatMessageType::TextEmote,
                ChatMessageType::Whisper,
                ChatMessageType::WhisperMob,
                ChatMessageType::WhisperInform,
                ChatMessageType::Channel,
                ChatMessageType::Guild,
                ChatMessageType::Officer,
                ChatMessageType::MonsterSay,
                ChatMessageType::MonsterYell,
                ChatMessageType::MonsterWhisper,
                ChatMessageType::MonsterEmote,
                ChatMessageType::System,
                ChatMessageType::Afk,
                ChatMessageType::Dnd,
            ]),
        },
        ChatTab {
            name: "Combat Log".into(),
            filter: Some(vec![
                ChatMessageType::Skill,
                ChatMessageType::Loot,
                ChatMessageType::System,
            ]),
        },
        ChatTab {
            name: "Group".into(),
            filter: Some(vec![
                ChatMessageType::Party,
                ChatMessageType::Raid,
                ChatMessageType::MonsterParty,
            ]),
        },
    ]
}
