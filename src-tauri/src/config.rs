use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use winreg::enums::*;
use winreg::RegKey;

const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE_NAME: &str = "MagicExplore";

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HotkeySpec {
    pub win: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub vk: u32,
}

impl Default for HotkeySpec {
    fn default() -> Self {
        Self {
            win: true,
            ctrl: false,
            alt: false,
            shift: false,
            vk: 0x20, // VK_SPACE
        }
    }
}

impl HotkeySpec {
    pub fn display_string(&self) -> String {
        let mut parts = Vec::new();
        if self.win {
            parts.push("Win");
        }
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(vk_to_label(self.vk));
        parts.join(" + ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub hotkey: HotkeySpec,
    pub hotkey_authorized: bool,
    pub consent_asked: bool,
    #[serde(default = "default_true")]
    pub autostart: bool,
    #[serde(default = "default_true")]
    pub show_admin_toast: bool,
    #[serde(default)]
    pub opaque_backdrop: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: HotkeySpec::default(),
            hotkey_authorized: false,
            consent_asked: false,
            autostart: true,
            show_admin_toast: true,
            opaque_backdrop: false,
        }
    }
}

pub fn app_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("Magic Explore");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn config_file_path() -> PathBuf {
    app_data_dir().join("config.json")
}

pub fn load_config() -> AppConfig {
    let path = config_file_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(mut cfg) = serde_json::from_str::<AppConfig>(&raw) {
            cfg.autostart = is_autostart_enabled();
            return cfg;
        }
    }
    let mut cfg = AppConfig::default();
    cfg.autostart = is_autostart_enabled();
    cfg
}

pub fn save_config(cfg: &AppConfig) -> Result<(), String> {
    let path = config_file_path();
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

pub fn is_autostart_enabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY_PATH, KEY_READ) {
        let val: Result<String, _> = key.get_value(RUN_VALUE_NAME);
        return val.map(|s| !s.trim().is_empty()).unwrap_or(false);
    }
    false
}

pub fn set_autostart_enabled(enabled: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(RUN_KEY_PATH)
        .map_err(|e| format!("Falha ao abrir registro: {e}"))?;

    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let quoted = format!("\"{}\"", exe.display());
        key.set_value(RUN_VALUE_NAME, &quoted)
            .map_err(|e| format!("Falha ao gravar autostart: {e}"))?;
    } else {
        let _ = key.delete_value(RUN_VALUE_NAME);
    }
    Ok(())
}

pub fn check_windows_conflict(hk: &HotkeySpec) -> (bool, Option<&'static str>) {
    if hk.win && !hk.ctrl && !hk.alt && !hk.shift && hk.vk == 0x4C {
        return (
            true,
            Some("Win + L bloqueia a tela do Windows no nível do sistema e não pode ser substituído."),
        );
    }
    if hk.ctrl && hk.alt && !hk.win && !hk.shift && hk.vk == 0x2E {
        return (
            true,
            Some("Ctrl + Alt + Del é protegido pelo kernel do Windows e não pode ser substituído."),
        );
    }

    if hk.win && !hk.ctrl && !hk.alt && !hk.shift {
        let reason = match hk.vk {
            0x20 => Some("Alternar idioma / layout do teclado"),
            0x53 => Some("Pesquisa nativa do Windows"),
            0x51 => Some("Pesquisa rápida do Windows"),
            0x52 => Some("Janela Executar (Win + R)"),
            0x45 => Some("Abrir Explorador de Arquivos"),
            0x44 => Some("Mostrar ou ocultar Área de Trabalho"),
            0x09 => Some("Visão de Tarefas (Task View)"),
            0x41 => Some("Central de Ações / Configurações Rápidas"),
            0x4E => Some("Central de Notificações"),
            0x57 => Some("Widgets do Windows"),
            0x43 => Some("Copilot / Assistente do Windows"),
            0x56 => Some("Histórico da Área de Transferência"),
            0x58 => Some("Menu rápido do botão Iniciar"),
            0x49 => Some("Configurações do Windows"),
            _ => None,
        };
        if reason.is_some() {
            return (false, reason);
        }
    }

    if !hk.win && !hk.ctrl && hk.alt && !hk.shift && hk.vk == 0x20 {
        return (false, Some("Menu de sistema da janela ativa (Mover/Fechar)"));
    }
    if !hk.win && hk.ctrl && !hk.alt && hk.shift && hk.vk == 0x1B {
        return (false, Some("Abrir Gerenciador de Tarefas"));
    }

    (false, None)
}

pub fn vk_to_label(vk: u32) -> &'static str {
    match vk {
        0x20 => "Space",
        0x09 => "Tab",
        0x0D => "Enter",
        0x08 => "Backspace",
        0x1B => "Esc",
        0x2E => "Delete",
        0x2D => "Insert",
        0x24 => "Home",
        0x23 => "End",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0xC0 => "`",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        0x30 => "0",
        0x31 => "1",
        0x32 => "2",
        0x33 => "3",
        0x34 => "4",
        0x35 => "5",
        0x36 => "6",
        0x37 => "7",
        0x38 => "8",
        0x39 => "9",
        0x41 => "A",
        0x42 => "B",
        0x43 => "C",
        0x44 => "D",
        0x45 => "E",
        0x46 => "F",
        0x47 => "G",
        0x48 => "H",
        0x49 => "I",
        0x4A => "J",
        0x4B => "K",
        0x4C => "L",
        0x4D => "M",
        0x4E => "N",
        0x4F => "O",
        0x50 => "P",
        0x51 => "Q",
        0x52 => "R",
        0x53 => "S",
        0x54 => "T",
        0x55 => "U",
        0x56 => "V",
        0x57 => "W",
        0x58 => "X",
        0x59 => "Y",
        0x5A => "Z",
        0x70 => "F1",
        0x71 => "F2",
        0x72 => "F3",
        0x73 => "F4",
        0x74 => "F5",
        0x75 => "F6",
        0x76 => "F7",
        0x77 => "F8",
        0x78 => "F9",
        0x79 => "F10",
        0x7A => "F11",
        0x7B => "F12",
        _ => "Key",
    }
}
