// WoW 3.3.5a (Build 12340) — Memory Offsets
// See docs/offsets.md for full documentation.

// ── Chat Buffer (circular, 60 entries) ──────────────────────────────
pub const CHAT_BUFFER_START: usize = 0x00B75A60;
pub const CHAT_MESSAGE_STRIDE: usize = 0x17C0; // 6080 bytes per message
pub const CHAT_BUFFER_COUNT: usize = 0x00BCEFEC; // next-write index (u32) — may be wrong for some builds
pub const CHAT_BUFFER_COUNT_ALT: usize = 0x00B66EDC; // alternative count address
pub const CHAT_BUFFER_SIZE: usize = 60;

// ── Fields within a single chat message ─────────────────────────────
pub const MSG_SENDER_GUID: usize = 0x0000; // u64
pub const MSG_SENDER_NAME: usize = 0x0018; // char[49], may overlap — prefer parsing FormattedMessage
pub const MSG_FORMATTED: usize = 0x003C; // char[3000], with WoW color codes / links
pub const MSG_PLAIN_TEXT: usize = 0x0BF4; // char[3000], clean text
pub const MSG_TYPE: usize = 0x17AC; // u32, ChatMessageType
pub const MSG_CHANNEL_NUM: usize = 0x17B0; // u32
pub const MSG_SEQUENCE: usize = 0x17B4; // u32, per-message incrementing counter
pub const MSG_TIMESTAMP: usize = 0x17B8; // u32

pub const MSG_STRING_MAX_LEN: usize = 3000;

// ── Player info ─────────────────────────────────────────────────────
pub const PLAYER_NAME: usize = 0x00C79D18; // char[], null-terminated
pub const PLAYER_GUID: usize = 0x00CA1238; // u64
pub const REALM_NAME: usize = 0x00C79B9E; // char[], null-terminated

// ── Object Manager (for reading player descriptors) ─────────────────
pub const CLIENT_CONNECTION: usize = 0x00C79CE0; // ptr
pub const OBJECT_MANAGER_OFFSET: usize = 0x2ED0;
pub const FIRST_OBJECT_OFFSET: usize = 0xAC;
pub const LOCAL_GUID_OFFSET: usize = 0xC0;
pub const NEXT_OBJECT_OFFSET: usize = 0x3C;
pub const OBJECT_GUID_OFFSET: usize = 0x30;
pub const DESCRIPTOR_PTR_OFFSET: usize = 0x08;

// ── Unit/Player descriptor field offsets (build 12340 / 3.3.5a) ─────
pub const UNIT_FIELD_LEVEL: usize = 0xD8; // u32, descriptor index 0x36
pub const PLAYER_FIELD_COINAGE: usize = 0x1248; // u32 (copper), descriptor index 0x0492
