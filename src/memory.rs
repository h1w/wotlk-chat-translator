use log::{debug, error, info, trace};
use std::io;

pub trait ProcessMemoryReader {
    fn attach(&mut self, pid: u32) -> io::Result<()>;
    fn read_memory(&self, address: usize, size: usize) -> io::Result<Vec<u8>>;
    fn detach(&mut self) -> io::Result<()>;
    fn is_attached(&self) -> bool;

    /// Scan process memory for a byte pattern. Returns addresses of all matches.
    fn scan_for_bytes(&self, _needle: &[u8]) -> io::Result<Vec<usize>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory scanning not supported on this platform",
        ))
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use log::warn;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::Memory::{MEM_COMMIT, MEMORY_BASIC_INFORMATION, VirtualQueryEx};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

    pub struct WindowsMemoryReader {
        handle: Option<HANDLE>,
    }

    impl WindowsMemoryReader {
        pub fn new() -> Self {
            Self { handle: None }
        }
    }

    impl ProcessMemoryReader for WindowsMemoryReader {
        fn attach(&mut self, pid: u32) -> io::Result<()> {
            self.detach()?;
            info!("Opening process PID={} with PROCESS_VM_READ", pid);
            let handle = unsafe { OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid) }
                .map_err(|e| {
                    error!("OpenProcess failed for PID={}: {}", pid, e);
                    io::Error::new(io::ErrorKind::PermissionDenied, e.to_string())
                })?;
            info!("Successfully opened process PID={}, handle={:?}", pid, handle);
            self.handle = Some(handle);
            Ok(())
        }

        fn read_memory(&self, address: usize, size: usize) -> io::Result<Vec<u8>> {
            let handle = self
                .handle
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "Not attached"))?;
            let mut buffer = vec![0u8; size];
            let mut bytes_read = 0usize;
            trace!("ReadProcessMemory addr=0x{:X} size={}", address, size);
            unsafe {
                ReadProcessMemory(
                    handle,
                    address as *const _,
                    buffer.as_mut_ptr() as *mut _,
                    size,
                    Some(&mut bytes_read),
                )
            }
            .map_err(|e| {
                debug!(
                    "ReadProcessMemory failed at 0x{:X} (size={}): {}",
                    address, size, e
                );
                io::Error::new(io::ErrorKind::Other, e.to_string())
            })?;
            trace!("ReadProcessMemory OK: {} of {} bytes read", bytes_read, size);
            buffer.truncate(bytes_read);
            Ok(buffer)
        }

        fn detach(&mut self) -> io::Result<()> {
            if let Some(handle) = self.handle.take() {
                info!("Closing process handle {:?}", handle);
                unsafe { CloseHandle(handle) }
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            }
            Ok(())
        }

        fn is_attached(&self) -> bool {
            self.handle.is_some()
        }

        fn scan_for_bytes(&self, needle: &[u8]) -> io::Result<Vec<usize>> {
            let handle = self
                .handle
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "Not attached"))?;
            if needle.is_empty() {
                return Ok(Vec::new());
            }

            let mut results = Vec::new();
            let mut address: usize = 0x10000; // Skip first 64 KB (null page area)
            let max_address: usize = 0x7FFF_0000;
            let max_results: usize = 1000;
            let mut regions_scanned: u32 = 0;
            let mut bytes_scanned: u64 = 0;

            info!("Scanning process memory for {} byte pattern...", needle.len());

            while address < max_address && results.len() < max_results {
                let mut mbi = MEMORY_BASIC_INFORMATION::default();
                let ret = unsafe {
                    VirtualQueryEx(
                        handle,
                        Some(address as *const _),
                        &mut mbi,
                        std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                    )
                };
                if ret == 0 {
                    break;
                }

                let base = mbi.BaseAddress as usize;
                let size = mbi.RegionSize;
                let next = base.wrapping_add(size);
                if next <= base {
                    break; // overflow
                }

                // Only scan committed, readable, non-guarded pages
                if mbi.State == MEM_COMMIT {
                    let p = mbi.Protect.0;
                    // p != 0, not PAGE_NOACCESS(0x01), not PAGE_GUARD(0x100)
                    if p != 0 && (p & 0x01) == 0 && (p & 0x100) == 0 {
                        const CHUNK: usize = 4 * 1024 * 1024;
                        let mut off = 0;
                        while off < size && results.len() < max_results {
                            let read_size = CHUNK.min(size - off);
                            let read_addr = base + off;
                            if let Ok(data) = self.read_memory(read_addr, read_size) {
                                if data.len() >= needle.len() {
                                    let mut i = 0;
                                    while i <= data.len() - needle.len() {
                                        if data[i] == needle[0]
                                            && data[i..i + needle.len()] == *needle
                                        {
                                            results.push(read_addr + i);
                                            if results.len() >= max_results {
                                                break;
                                            }
                                        }
                                        i += 1;
                                    }
                                }
                                bytes_scanned += data.len() as u64;
                            }
                            // Overlap at chunk boundaries to catch cross-boundary matches
                            if needle.len() > 1 && off + CHUNK < size {
                                off += CHUNK - (needle.len() - 1);
                            } else {
                                off += CHUNK;
                            }
                        }
                        regions_scanned += 1;
                    }
                }

                address = next;
            }

            if results.len() >= max_results {
                warn!("Scan capped at {} results", max_results);
            }

            info!(
                "Scan complete: {} regions, {:.1} MB scanned, {} matches",
                regions_scanned,
                bytes_scanned as f64 / (1024.0 * 1024.0),
                results.len(),
            );

            Ok(results)
        }
    }

    impl Drop for WindowsMemoryReader {
        fn drop(&mut self) {
            let _ = self.detach();
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_impl {
    use super::*;
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    pub struct LinuxMemoryReader {
        mem_file: Option<File>,
    }

    impl LinuxMemoryReader {
        pub fn new() -> Self {
            Self { mem_file: None }
        }
    }

    impl ProcessMemoryReader for LinuxMemoryReader {
        fn attach(&mut self, pid: u32) -> io::Result<()> {
            self.detach()?;
            let path = format!("/proc/{}/mem", pid);
            info!("Opening {} for memory reading", path);
            let file = File::open(&path).map_err(|e| {
                error!("Failed to open {}: {}", path, e);
                e
            })?;
            info!("Successfully opened {}", path);
            self.mem_file = Some(file);
            Ok(())
        }

        fn read_memory(&self, address: usize, size: usize) -> io::Result<Vec<u8>> {
            let file = self
                .mem_file
                .as_ref()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "Not attached"))?;
            let mut file = file.try_clone()?;
            trace!("Reading /proc mem at 0x{:X} size={}", address, size);
            file.seek(SeekFrom::Start(address as u64))?;
            let mut buffer = vec![0u8; size];
            let bytes_read = file.read(&mut buffer)?;
            trace!("Read {} of {} bytes from 0x{:X}", bytes_read, size, address);
            buffer.truncate(bytes_read);
            Ok(buffer)
        }

        fn detach(&mut self) -> io::Result<()> {
            if self.mem_file.take().is_some() {
                info!("Closed /proc/mem file");
            }
            Ok(())
        }

        fn is_attached(&self) -> bool {
            self.mem_file.is_some()
        }
    }
}

pub fn create_reader() -> Box<dyn ProcessMemoryReader> {
    #[cfg(windows)]
    {
        Box::new(windows_impl::WindowsMemoryReader::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux_impl::LinuxMemoryReader::new())
    }
}
