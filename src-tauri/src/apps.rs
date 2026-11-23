use crate::config::app_data_dir;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{SIZE, S_OK};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, ShellExecuteW, SIIGBF_ICONONLY,
    SIIGBF_RESIZETOFIT,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub source: String,
    pub launch_target: String,
    pub icon_data_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StartAppItem {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "AppID")]
    app_id: Option<String>,
}

fn cache_file_path() -> PathBuf {
    app_data_dir().join("apps_cache.json")
}

pub fn load_cached_apps() -> Vec<AppEntry> {
    if let Ok(raw) = fs::read_to_string(cache_file_path()) {
        if let Ok(items) = serde_json::from_str::<Vec<AppEntry>>(&raw) {
            return items;
        }
    }
    Vec::new()
}

pub fn save_cached_apps(apps: &[AppEntry]) {
    if let Ok(json) = serde_json::to_string(apps) {
        let _ = fs::write(cache_file_path(), json);
    }
}

fn should_skip_shortcut(name_lower: &str) -> bool {
    name_lower.contains("uninstall")
        || name_lower.contains("desinstalar")
        || name_lower.starts_with("readme")
        || name_lower.starts_with("leia-me")
}

fn collect_shortcuts_recursive(dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_shortcuts_recursive(&path, out);
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if ext != "lnk" && ext != "url" {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let clean = stem.trim();
        if clean.is_empty() || should_skip_shortcut(&clean.to_lowercase()) {
            continue;
        }
        out.push((clean.to_string(), path));
    }
}

fn scan_shortcuts_fast() -> Vec<AppEntry> {
    let mut roots = Vec::new();
    for var in ["ProgramData", "APPDATA"] {
        if let Ok(val) = std::env::var(var) {
            roots.push(PathBuf::from(val).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
    }
    if let Ok(val) = std::env::var("PUBLIC") {
        roots.push(PathBuf::from(val).join("Desktop"));
    }
    if let Ok(val) = std::env::var("USERPROFILE") {
        roots.push(PathBuf::from(val).join("Desktop"));
    }

    let mut raw = Vec::new();
    for root in roots {
        collect_shortcuts_recursive(&root, &mut raw);
    }

    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for (name, path) in raw {
        let key = name.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let target = path.to_string_lossy().to_string();
        apps.push(AppEntry {
            id: format!("lnk:{}", target),
            name,
            source: "Aplicativo".to_string(),
            launch_target: target,
            icon_data_url: None,
        });
    }
    apps
}

fn fetch_start_apps() -> Vec<AppEntry> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Get-StartApps | ConvertTo-Json -Compress",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let Ok(out) = output else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let items: Vec<StartAppItem> = if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).unwrap_or_default()
    } else if let Ok(single) = serde_json::from_str::<StartAppItem>(trimmed) {
        vec![single]
    } else {
        Vec::new()
    };

    let mut result = Vec::new();
    for item in items {
        let Some(name) = item.name.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some(app_id) = item.app_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
            continue;
        };
        if should_skip_shortcut(&name.to_lowercase()) {
            continue;
        }
        let is_store = app_id.contains('!');
        result.push(AppEntry {
            id: format!("uwp:{}", app_id),
            name,
            source: if is_store {
                "Microsoft Store".to_string()
            } else {
                "Aplicativo".to_string()
            },
            launch_target: format!("shell:AppsFolder\\{}", app_id),
            icon_data_url: None,
        });
    }
    result
}

pub fn index_all_apps(previous_icons: &HashMap<String, String>) -> Vec<AppEntry> {
    let shortcuts = scan_shortcuts_fast();
    let start_apps = fetch_start_apps();

    let mut by_name: HashMap<String, AppEntry> = HashMap::new();
    for app in shortcuts {
        by_name.entry(app.name.to_lowercase()).or_insert(app);
    }
    for app in start_apps {
        by_name.entry(app.name.to_lowercase()).or_insert(app);
    }

    let mut merged: Vec<AppEntry> = by_name.into_values().collect();
    merged.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    // Extract missing icons with COM initialized on this worker thread
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let com_inited = hr == S_OK || hr.0 == 1; // S_OK or S_FALSE

        for app in &mut merged {
            if let Some(cached) = previous_icons.get(&app.launch_target) {
                app.icon_data_url = Some(cached.clone());
            } else {
                app.icon_data_url = extract_icon_data_url(&app.launch_target);
            }
        }

        if com_inited {
            CoUninitialize();
        }
    }

    save_cached_apps(&merged);
    merged
}

pub fn index_shortcuts_only(previous_icons: &HashMap<String, String>) -> Vec<AppEntry> {
    let mut apps = scan_shortcuts_fast();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    for app in &mut apps {
        if let Some(cached) = previous_icons.get(&app.launch_target) {
            app.icon_data_url = Some(cached.clone());
        }
    }
    apps
}

unsafe fn extract_icon_data_url(parsing_name: &str) -> Option<String> {
    let wide = HSTRING::from(parsing_name);
    let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&wide, None).ok()?;

    let size = SIZE { cx: 32, cy: 32 };
    let hbmp: HBITMAP = factory
        .GetImage(size, SIIGBF_ICONONLY | SIIGBF_RESIZETOFIT)
        .or_else(|_| factory.GetImage(size, SIIGBF_RESIZETOFIT))
        .ok()?;

    let result = hbitmap_to_png_data_url(hbmp);
    let _ = DeleteObject(HGDIOBJ(hbmp.0));
    result
}

unsafe fn hbitmap_to_png_data_url(hbmp: HBITMAP) -> Option<String> {
    let mut bmp = BITMAP::default();
    if GetObjectW(
        HGDIOBJ(hbmp.0),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bmp as *mut _ as *mut _),
    ) == 0
    {
        return None;
    }

    let w = bmp.bmWidth;
    let h = bmp.bmHeight.abs();
    if w <= 0 || h <= 0 || w > 256 || h > 256 {
        return None;
    }

    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h, // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };

    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let hdc = GetDC(None);
    let lines = GetDIBits(
        hdc,
        hbmp,
        0,
        h as u32,
        Some(pixels.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    ReleaseDC(None, hdc);

    if lines == 0 {
        return None;
    }

    // BGRA -> RGBA and fix legacy icons where alpha is 0 everywhere
    let mut has_alpha = false;
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2);
        if chunk[3] != 0 {
            has_alpha = true;
        }
    }
    if !has_alpha {
        for chunk in pixels.chunks_exact_mut(4) {
            if chunk[0] != 0 || chunk[1] != 0 || chunk[2] != 0 {
                chunk[3] = 255;
            }
        }
    }

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&pixels).ok()?;
    }

    Some(format!("data:image/png;base64,{}", BASE64.encode(&png_bytes)))
}

pub fn shell_open(target: &str) -> Result<(), String> {
    let verb = HSTRING::from("open");
    let file = HSTRING::from(target);
    let res = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (res.0 as isize) <= 32 {
        return Err(format!("Falha ao abrir \"{}\" (código {})", target, res.0 as isize));
    }
    Ok(())
}

pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for byte in input.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            b' ' => out.push_str("%20"),
            b => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}

pub fn open_explorer_search(query: &str) -> Result<(), String> {
    let enc = url_encode(query.trim());
    // CLSID {20D04FE0-3AEA-1069-A2D8-08002B30309D} = Este Computador / This PC
    let uri = format!(
        "search-ms:displayname=Pesquisa%20Magic%20Explore&crumb=System.Generic.String%3A{}&crumb=location:%3A%3A%7B20D04FE0-3AEA-1069-A2D8-08002B30309D%7D",
        enc
    );
    shell_open(&uri)
}

pub fn open_web_search(query: &str) -> Result<(), String> {
    let enc = url_encode(query.trim());
    let url = format!("https://www.google.com/search?q={}", enc);
    shell_open(&url)
}
