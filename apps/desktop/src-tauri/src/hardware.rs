//! Hardware detection (docs/Production.md §9.3), Windows APIs only, no child processes.
//! GPU memory comes from DXGI so dedicated VRAM and shared system memory are reported separately.
//! OpenVINO devices (CPU/GPU/NPU) are reported by the AI worker, not here.

use serde::Serialize;
use std::path::{Path, PathBuf};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIDevice, IDXGIFactory1};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::core::{HSTRING, Interface};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub gpus: Vec<GpuInfo>,
    pub data_disk: Option<DiskInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsInfo {
    pub name: String,
    pub edition: String,
    pub display_version: String,
    pub build: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuInfo {
    pub name: String,
    pub logical_processors: usize,
}

/// Bytes. `commit_*` is the Windows commit limit (RAM + pagefile) — the real ceiling for allocations.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInfo {
    pub total_physical: u64,
    pub available_physical: u64,
    pub commit_limit: u64,
    pub commit_available: u64,
    pub load_percent: u32,
}

/// Bytes. On integrated GPUs `shared_system_memory` is system RAM, not VRAM.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub name: String,
    pub vendor_id: u32,
    pub dedicated_video_memory: u64,
    pub shared_system_memory: u64,
    pub driver_version: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    pub path: PathBuf,
    pub total: u64,
    pub free: u64,
}

const NT_CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

pub fn detect(data_root: Option<&Path>) -> HardwareInfo {
    HardwareInfo { os: os(), cpu: cpu(), memory: memory(), gpus: gpus(), data_disk: data_root.and_then(disk) }
}

fn reg_string(subkey: &str, value: &str) -> Option<String> {
    let mut buf = [0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    }
    .ok()
    .ok()?;
    let chars = &buf[..(len as usize / 2)];
    let end = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
    Some(String::from_utf16_lossy(&chars[..end]).trim().to_string())
}

fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut len = 4u32;
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut len),
        )
    }
    .ok()
    .ok()?;
    Some(data)
}

fn os() -> OsInfo {
    let get = |v| reg_string(NT_CURRENT_VERSION, v).unwrap_or_default();
    let build = get("CurrentBuild");
    // ProductName still says "Windows 10" on Windows 11; the build number is authoritative.
    let name = if build.parse::<u32>().is_ok_and(|b| b >= 22000) { "Windows 11".to_string() } else { get("ProductName") };
    let ubr = reg_dword(NT_CURRENT_VERSION, "UBR").map(|u| format!(".{u}")).unwrap_or_default();
    OsInfo { name, edition: get("EditionID"), display_version: get("DisplayVersion"), build: format!("{build}{ubr}") }
}

fn cpu() -> CpuInfo {
    CpuInfo {
        name: reg_string(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString").unwrap_or_default(),
        logical_processors: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
    }
}

fn memory() -> MemoryInfo {
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_err() {
        return MemoryInfo::default();
    }
    MemoryInfo {
        total_physical: m.ullTotalPhys,
        available_physical: m.ullAvailPhys,
        commit_limit: m.ullTotalPageFile,
        commit_available: m.ullAvailPageFile,
        load_percent: m.dwMemoryLoad,
    }
}

fn gpus() -> Vec<GpuInfo> {
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return vec![] };
    let mut out = vec![];
    for i in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
        let Ok(d) = (unsafe { adapter.GetDesc1() }) else { continue };
        if d.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
            continue; // Microsoft Basic Render Driver
        }
        let end = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
        // UMD driver version packed as 4 × u16, e.g. 32.0.101.8860.
        let driver_version = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }.ok().map(|v| {
            let v = v as u64;
            format!("{}.{}.{}.{}", v >> 48, (v >> 32) & 0xffff, (v >> 16) & 0xffff, v & 0xffff)
        });
        out.push(GpuInfo {
            name: String::from_utf16_lossy(&d.Description[..end]),
            vendor_id: d.VendorId,
            dedicated_video_memory: d.DedicatedVideoMemory as u64,
            shared_system_memory: d.SharedSystemMemory as u64,
            driver_version,
        });
    }
    out
}

/// Free space of the drive holding `path` (or its nearest existing ancestor).
fn disk(path: &Path) -> Option<DiskInfo> {
    let existing = path.ancestors().find(|p| p.exists())?;
    let (mut free, mut total) = (0u64, 0u64);
    unsafe {
        GetDiskFreeSpaceExW(&HSTRING::from(existing.as_os_str()), Some(&mut free), Some(&mut total), None)
    }
    .ok()?;
    Some(DiskInfo { path: path.to_path_buf(), total, free })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_basic_hardware() {
        let h = detect(Some(&std::env::temp_dir().join("does-not-exist-yet")));
        assert!(!h.cpu.name.is_empty());
        assert!(h.cpu.logical_processors >= 1);
        assert!(h.memory.total_physical > 0);
        assert!(h.memory.commit_limit >= h.memory.total_physical);
        assert!(!h.os.build.is_empty());
        let disk = h.data_disk.as_ref().expect("disk of nearest existing ancestor");
        assert!(disk.total >= disk.free && disk.total > 0);
        println!("{h:#?}");
    }
}
