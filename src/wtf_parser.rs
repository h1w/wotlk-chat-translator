use std::io;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};

use crate::chat::{ChatMessageType, ChatTab};

// ─── Types ───────────────────────────────────────────────────────────

pub struct CharacterConfig {
    pub account: String,
    pub realm: String,
    pub character: String,
    pub chat_cache_path: PathBuf,
}

impl CharacterConfig {
    pub fn display_label(&self) -> String {
        format!("{} - {}", self.realm, self.character)
    }
}

pub struct WtfChatWindow {
    pub name: String,
    pub message_types: Vec<ChatMessageType>,
}

// ─── Directory scanner ───────────────────────────────────────────────

/// Scan `<wow_path>/WTF/Account/*/Realm/Char/chat-cache.txt` for all characters.
pub fn find_character_configs(wow_path: &Path) -> io::Result<Vec<CharacterConfig>> {
    let wtf_account = wow_path.join("WTF").join("Account");
    if !wtf_account.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("WTF/Account directory not found at {}", wtf_account.display()),
        ));
    }

    let mut configs = Vec::new();

    for account_entry in std::fs::read_dir(&wtf_account)? {
        let account_entry = account_entry?;
        if !account_entry.file_type()?.is_dir() {
            continue;
        }
        let account_name = account_entry.file_name().to_string_lossy().into_owned();
        let account_path = account_entry.path();

        // Each subdirectory under account is a realm.
        for realm_entry in std::fs::read_dir(&account_path)? {
            let realm_entry = realm_entry?;
            if !realm_entry.file_type()?.is_dir() {
                continue;
            }
            let realm_name = realm_entry.file_name().to_string_lossy().into_owned();
            // Skip "SavedVariables" folder that lives alongside realm folders.
            if realm_name == "SavedVariables" {
                continue;
            }
            let realm_path = realm_entry.path();

            // Each subdirectory under realm is a character.
            for char_entry in std::fs::read_dir(&realm_path)? {
                let char_entry = char_entry?;
                if !char_entry.file_type()?.is_dir() {
                    continue;
                }
                let char_name = char_entry.file_name().to_string_lossy().into_owned();
                if char_name == "SavedVariables" {
                    continue;
                }
                let chat_cache = char_entry.path().join("chat-cache.txt");
                if chat_cache.is_file() {
                    debug!(
                        "Found chat-cache: {}/{}/{} at {}",
                        account_name, realm_name, char_name, chat_cache.display()
                    );
                    configs.push(CharacterConfig {
                        account: account_name.clone(),
                        realm: realm_name.clone(),
                        character: char_name.clone(),
                        chat_cache_path: chat_cache,
                    });
                }
            }
        }
    }

    info!("Found {} character configs", configs.len());
    Ok(configs)
}

// ─── chat-cache.txt parser ───────────────────────────────────────────

/// Parse a chat-cache.txt file into a list of WtfChatWindow definitions.
pub fn parse_chat_cache(path: &Path) -> io::Result<Vec<WtfChatWindow>> {
    let content = std::fs::read_to_string(path)?;
    let mut windows = Vec::new();

    #[derive(Debug)]
    enum State {
        Root,
        InWindow,
        InMessages,
        SkipSection, // CHANNELS, ZONECHANNELS, COLORS — skip until END
    }

    let mut state = State::Root;
    let mut current_name = String::new();
    let mut current_types: Vec<ChatMessageType> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match state {
            State::Root => {
                if line.starts_with("WINDOW ") {
                    state = State::InWindow;
                    current_name.clear();
                    current_types.clear();
                } else if line == "COLORS" {
                    state = State::SkipSection;
                }
                // Ignore VERSION, ADDEDVERSION, etc.
            }
            State::InWindow => {
                if line.starts_with("NAME ") {
                    current_name = line["NAME ".len()..].to_string();
                } else if line == "MESSAGES" {
                    state = State::InMessages;
                } else if line == "CHANNELS" || line == "ZONECHANNELS" {
                    state = State::SkipSection;
                } else if line.starts_with("WINDOW ") {
                    // Previous window ended implicitly — save it.
                    if !current_name.is_empty() {
                        windows.push(WtfChatWindow {
                            name: current_name.clone(),
                            message_types: current_types.clone(),
                        });
                    }
                    current_name.clear();
                    current_types.clear();
                }
                // Ignore SIZE, COLOR, LOCKED, etc.
            }
            State::InMessages => {
                if line == "END" {
                    state = State::InWindow;
                } else if let Some(msg_type) = wtf_type_to_chat_message_type(line) {
                    if !current_types.contains(&msg_type) {
                        current_types.push(msg_type);
                    }
                } else {
                    debug!("Unknown WTF message type: {}", line);
                }
            }
            State::SkipSection => {
                if line == "END" {
                    // If we were in a CHANNELS/ZONECHANNELS section inside a window,
                    // go back to InWindow. If we were in a root-level section (COLORS),
                    // go back to Root. We can detect this: if current_name is set or
                    // we've seen a WINDOW header, we're in a window context.
                    if current_name.is_empty() && windows.is_empty() && current_types.is_empty() {
                        state = State::Root;
                    } else {
                        state = State::InWindow;
                    }
                }
            }
        }
    }

    // Don't forget the last window.
    if !current_name.is_empty() {
        windows.push(WtfChatWindow {
            name: current_name,
            message_types: current_types,
        });
    }

    info!(
        "Parsed {} windows from {}",
        windows.len(),
        path.display()
    );
    for w in &windows {
        info!("  Window '{}': {} message types", w.name, w.message_types.len());
    }

    Ok(windows)
}

// ─── WTF type name → ChatMessageType mapping ────────────────────────

fn wtf_type_to_chat_message_type(name: &str) -> Option<ChatMessageType> {
    match name {
        "SAY" => Some(ChatMessageType::Say),
        "PARTY" | "PARTY_LEADER" => Some(ChatMessageType::Party),
        "RAID" | "RAID_LEADER" | "RAID_WARNING" => Some(ChatMessageType::Raid),
        "GUILD" => Some(ChatMessageType::Guild),
        "OFFICER" => Some(ChatMessageType::Officer),
        "YELL" => Some(ChatMessageType::Yell),
        "WHISPER" => Some(ChatMessageType::Whisper),
        "WHISPER_FOREIGN" => Some(ChatMessageType::WhisperMob),
        "WHISPER_INFORM" => Some(ChatMessageType::WhisperInform),
        "EMOTE" => Some(ChatMessageType::Emote),
        "TEXT_EMOTE" => Some(ChatMessageType::TextEmote),
        "MONSTER_SAY" => Some(ChatMessageType::MonsterSay),
        "MONSTER_PARTY" => Some(ChatMessageType::MonsterParty),
        "MONSTER_YELL" => Some(ChatMessageType::MonsterYell),
        "MONSTER_WHISPER" => Some(ChatMessageType::MonsterWhisper),
        "MONSTER_EMOTE" => Some(ChatMessageType::MonsterEmote),
        "CHANNEL" => Some(ChatMessageType::Channel),
        "CHANNEL_JOIN" => Some(ChatMessageType::ChannelJoin),
        "CHANNEL_LEAVE" => Some(ChatMessageType::ChannelLeave),
        "CHANNEL_LIST" => Some(ChatMessageType::ChannelList),
        "CHANNEL_NOTICE" => Some(ChatMessageType::ChannelNotice),
        "CHANNEL_NOTICE_USER" => Some(ChatMessageType::ChannelNoticeUser),
        "AFK" => Some(ChatMessageType::Afk),
        "DND" => Some(ChatMessageType::Dnd),
        "IGNORED" => Some(ChatMessageType::Ignored),
        "SKILL" => Some(ChatMessageType::Skill),
        "LOOT" => Some(ChatMessageType::Loot),
        "SYSTEM" | "SYSTEM_NOMENU" | "ERRORS" => Some(ChatMessageType::System),
        "BATTLEGROUND" | "BATTLEGROUND_LEADER" => Some(ChatMessageType::Raid),
        "MONSTER_BOSS_EMOTE" => Some(ChatMessageType::MonsterEmote),
        "MONSTER_BOSS_WHISPER" => Some(ChatMessageType::MonsterWhisper),
        "BG_HORDE" | "BG_ALLIANCE" | "BG_NEUTRAL"
        | "BG_SYSTEM_NEUTRAL" | "BG_SYSTEM_ALLIANCE" | "BG_SYSTEM_HORDE" => {
            Some(ChatMessageType::System)
        }
        "MONEY" => Some(ChatMessageType::Loot),
        "COMBAT_XP_GAIN" | "COMBAT_HONOR_GAIN" | "COMBAT_FACTION_CHANGE"
        | "COMBAT_MISC_INFO" => Some(ChatMessageType::System),
        "TRADESKILLS" | "OPENING" | "PET_INFO" => Some(ChatMessageType::Skill),
        // Types present in WTF with no meaningful mapping — skip silently.
        "ACHIEVEMENT" | "GUILD_ACHIEVEMENT" | "BN_WHISPER" | "BN_WHISPER_INFORM"
        | "BN_CONVERSATION" | "BN_CONVERSATION_NOTICE" | "BN_CONVERSATION_LIST"
        | "BN_INLINE_TOAST_ALERT" | "BN_INLINE_TOAST_BROADCAST"
        | "BN_INLINE_TOAST_BROADCAST_INFORM" | "BN_INLINE_TOAST_CONVERSATION"
        | "TARGETICONS" | "FILTERED" | "RESTRICTED" | "ADDON"
        | "RAID_BOSS_EMOTE" | "RAID_BOSS_WHISPER" | "BATTLENET"
        | "ARENA_POINTS" => None,
        _ => {
            warn!("Unrecognized WTF message type: {}", name);
            None
        }
    }
}

// ─── Convert parsed windows → ChatTab vec ────────────────────────────

/// Convert WTF-parsed windows into ChatTab structs, prepending an "All" tab.
pub fn to_chat_tabs(windows: &[WtfChatWindow]) -> Vec<ChatTab> {
    let mut tabs = vec![ChatTab {
        name: "All".into(),
        filter: None,
    }];

    for w in windows {
        if w.message_types.is_empty() {
            continue;
        }
        tabs.push(ChatTab {
            name: w.name.clone(),
            filter: Some(w.message_types.clone()),
        });
    }

    tabs
}
