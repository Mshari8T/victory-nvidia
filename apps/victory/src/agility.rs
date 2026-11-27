//! The D3D12 Agility SDK on Windows 10. Its own D3D12 stops at shader model
//! 6.5 and refuses root signatures with the directly indexed heap flags the
//! title uses, so a newer `D3D12Core.dll` is loaded at run time and made the
//! runtime of the whole process before any device exists.
//!
//! The DLL is looked for in `VICTORY_D3D12_DIR`, then in `D3D12\` beside
//! victory.exe. Without one nothing changes (Windows 11 needs none).

use std::path::{Path, PathBuf};

use victory_pe::ImageInfo;
use windows::Win32::Graphics::Direct3D12::{
    CLSID_D3D12SDKConfiguration, D3D12_DEVICE_FACTORY_FLAG_ALLOW_RETURNING_EXISTING_DEVICE,
    D3D12GetInterface, ID3D12DeviceFactory, ID3D12SDKConfiguration1,
};
use windows::core::PCSTR;

pub fn load() {
    let Some(dir) = sdk_dir() else { return };
    let version = match sdk_version(&dir.join("D3D12Core.dll")) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("VIctory: Agility SDK in {} not used: {e}", dir.display());
            return;
        }
    };
    let mut path = dir.to_string_lossy().into_owned();
    if !path.ends_with('\\') {
        path.push('\\');
    }
    path.push('\0');
    match apply(version, &path) {
        Ok(()) => tracing::info!(version, dir = %dir.display(), "D3D12 Agility SDK loaded"),
        Err(e) => eprintln!("VIctory: Agility SDK {version} in {} not used: {e}", dir.display()),
    }
}

fn sdk_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("VICTORY_D3D12_DIR").filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => std::env::current_exe().ok()?.parent()?.join("D3D12"),
    };
    let dir = std::path::absolute(&dir).unwrap_or(dir);
    dir.join("D3D12Core.dll").is_file().then_some(dir)
}

fn sdk_version(dll: &Path) -> Result<u32, String> {
    let data = std::fs::read(dll).map_err(|e| e.to_string())?;
    let image = ImageInfo::parse(&data).map_err(|e| e.to_string())?;
    let rva = image
        .exports
        .iter()
        .find(|e| e.name.as_deref() == Some("D3D12SDKVersion"))
        .and_then(|e| e.rva)
        .ok_or("no D3D12SDKVersion export")?;
    let at = image.rva_to_file_offset(rva).ok_or("D3D12SDKVersion outside the file")?;
    victory_pe::layout::read_u32(&data, at as usize).ok_or_else(|| "D3D12SDKVersion unreadable".into())
}

fn apply(version: u32, path: &str) -> windows::core::Result<()> {
    // SAFETY: `path` ends in a NUL; the factory is kept for the life of the
    // process, since the global state it applied refers to it.
    unsafe {
        let mut config: Option<ID3D12SDKConfiguration1> = None;
        D3D12GetInterface(&CLSID_D3D12SDKConfiguration, &mut config)?;
        let config = config.ok_or_else(windows::core::Error::empty)?;
        let factory: ID3D12DeviceFactory =
            config.CreateDeviceFactory(version, PCSTR(path.as_ptr()))?;
        factory.SetFlags(D3D12_DEVICE_FACTORY_FLAG_ALLOW_RETURNING_EXISTING_DEVICE)?;
        factory.ApplyToGlobalState()?;
        std::mem::forget(factory);
    }
    Ok(())
}
