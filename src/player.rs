use log::debug;
use std::io;

use crate::memory::ProcessMemoryReader;
use crate::offsets;

/// WoW 3.3.5a is 32-bit; valid userspace pointers are in this range.
const MIN_VALID_PTR: usize = 0x10000;
const MAX_VALID_PTR: usize = 0x7FFF_0000;

fn is_valid_ptr(addr: usize) -> bool {
    addr > MIN_VALID_PTR && addr < MAX_VALID_PTR
}

pub struct PlayerInfo {
    pub name: String,
    pub realm: String,
    pub level: u32,
    pub copper: u32,
}

impl PlayerInfo {
    pub fn gold(&self) -> u32 {
        self.copper / 10000
    }
    pub fn silver(&self) -> u32 {
        (self.copper % 10000) / 100
    }
    pub fn copper_rem(&self) -> u32 {
        self.copper % 100
    }
}

fn read_u32_mem(reader: &dyn ProcessMemoryReader, addr: usize) -> io::Result<u32> {
    let data = reader.read_memory(addr, 4)?;
    if data.len() < 4 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read"));
    }
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

fn read_u64_mem(reader: &dyn ProcessMemoryReader, addr: usize) -> io::Result<u64> {
    let data = reader.read_memory(addr, 8)?;
    if data.len() < 8 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read"));
    }
    Ok(u64::from_le_bytes(data[..8].try_into().unwrap()))
}

fn read_cstring_mem(
    reader: &dyn ProcessMemoryReader,
    addr: usize,
    max_len: usize,
) -> io::Result<String> {
    let data = reader.read_memory(addr, max_len)?;
    let null_pos = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    Ok(String::from_utf8_lossy(&data[..null_pos]).into_owned())
}

/// Read a u32 pointer and validate it's in 32-bit userspace.
fn read_ptr(reader: &dyn ProcessMemoryReader, addr: usize) -> io::Result<usize> {
    let ptr = read_u32_mem(reader, addr)? as usize;
    if !is_valid_ptr(ptr) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid pointer"));
    }
    Ok(ptr)
}

/// Find the local player's object base address by traversing the Object Manager linked list.
fn find_local_player_base(reader: &dyn ProcessMemoryReader) -> io::Result<usize> {
    let client_conn = read_ptr(reader, offsets::CLIENT_CONNECTION)?;
    let obj_mgr = read_ptr(reader, client_conn + offsets::OBJECT_MANAGER_OFFSET)?;

    let local_guid = read_u64_mem(reader, obj_mgr + offsets::LOCAL_GUID_OFFSET)?;
    if local_guid == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "local guid is 0",
        ));
    }

    let mut current = read_ptr(reader, obj_mgr + offsets::FIRST_OBJECT_OFFSET)?;
    let mut iterations = 0u32;
    const MAX_ITERATIONS: u32 = 500;

    while is_valid_ptr(current) && iterations < MAX_ITERATIONS {
        let guid = read_u64_mem(reader, current + offsets::OBJECT_GUID_OFFSET)?;
        if guid == local_guid {
            return Ok(current);
        }
        current = read_u32_mem(reader, current + offsets::NEXT_OBJECT_OFFSET)? as usize;
        iterations += 1;
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "local player object not found",
    ))
}

/// Read current player info from process memory.
/// Returns None if the player is not logged in or data is unavailable.
pub fn read_player_info(reader: &dyn ProcessMemoryReader) -> Option<PlayerInfo> {
    let name = read_cstring_mem(reader, offsets::PLAYER_NAME, 50).unwrap_or_default();
    let realm = read_cstring_mem(reader, offsets::REALM_NAME, 50).unwrap_or_default();

    let (level, copper) = match find_local_player_base(reader) {
        Ok(player_base) => {
            match read_ptr(reader, player_base + offsets::DESCRIPTOR_PTR_OFFSET) {
                Ok(descriptor_ptr) => {
                    let level = read_u32_mem(reader, descriptor_ptr + offsets::UNIT_FIELD_LEVEL)
                        .unwrap_or(0);
                    let copper =
                        read_u32_mem(reader, descriptor_ptr + offsets::PLAYER_FIELD_COINAGE)
                            .unwrap_or(0);
                    // Sanity: level 1-80 for WotLK, money < ~214k gold (u32 max)
                    let level = if level > 0 && level <= 80 { level } else { 0 };
                    (level, copper)
                }
                Err(_) => (0, 0),
            }
        }
        Err(e) => {
            debug!("Could not find player object: {}", e);
            (0, 0)
        }
    };

    if name.is_empty() && realm.is_empty() && level == 0 {
        return None;
    }

    Some(PlayerInfo {
        name,
        realm,
        level,
        copper,
    })
}
