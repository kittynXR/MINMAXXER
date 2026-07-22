use crate::vr_overlay::VrOverlaySettings;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 49_321;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopOverlaySettings {
    pub enabled: bool,
    pub width: f64,
    pub height: f64,
    pub offset_x: i32,
    pub offset_y: i32,
    pub corner: String,
    pub opacity: f64,
    pub click_through: bool,
    pub only_when_vrchat_foreground: bool,
    pub show_when_vr_active: bool,
    pub profile: String,
}

impl Default for DesktopOverlaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 520.0,
            height: 300.0,
            offset_x: 24,
            offset_y: 24,
            corner: "top-right".to_owned(),
            opacity: 0.94,
            click_through: true,
            only_when_vrchat_foreground: true,
            show_when_vr_active: false,
            profile: "broadcast".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayProfile {
    pub id: String,
    pub name: String,
    pub layout: String,
    pub theme: String,
    pub accent: String,
    pub background: String,
    pub rows: u8,
    pub scale: f32,
    pub show_dps: bool,
    pub show_damage: bool,
    pub show_incoming: bool,
    pub show_hits: bool,
    pub show_recent_hits: bool,
    pub recent_hit_rows: u8,
    pub show_encounter: bool,
    pub show_focus: bool,
    pub anonymize_players: bool,
}

impl Default for OverlayProfile {
    fn default() -> Self {
        Self {
            id: "broadcast".to_owned(),
            name: "Broadcast HUD".to_owned(),
            layout: "leaderboard".to_owned(),
            theme: "void".to_owned(),
            accent: "cyan".to_owned(),
            background: "rgba(8, 12, 24, 0.82)".to_owned(),
            rows: 8,
            scale: 1.0,
            show_dps: true,
            show_damage: true,
            show_incoming: true,
            show_hits: false,
            show_recent_hits: true,
            recent_hit_rows: 5,
            show_encounter: true,
            show_focus: true,
            anonymize_players: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub port: u16,
    pub log_directory: PathBuf,
    pub auto_import_recent_logs: bool,
    pub import_days: u32,
    pub launch_minimized: bool,
    pub minimize_to_tray: bool,
    pub stream_token: String,
    pub desktop_overlay: DesktopOverlaySettings,
    pub vr_overlay: VrOverlaySettings,
    pub overlay_profiles: Vec<OverlayProfile>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            log_directory: default_vrchat_log_directory(),
            auto_import_recent_logs: true,
            import_days: 3,
            launch_minimized: false,
            minimize_to_tray: true,
            stream_token: generate_install_token(),
            desktop_overlay: DesktopOverlaySettings::default(),
            vr_overlay: VrOverlaySettings::default(),
            overlay_profiles: vec![OverlayProfile::default()],
        }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            let config = Self::default();
            config.save(path)?;
            return Ok(config);
        }
        let bytes =
            fs::read(path).with_context(|| format!("failed reading config {}", path.display()))?;
        let mut config: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed parsing config {}", path.display()))?;
        let mut migrated = false;
        if config.stream_token.is_empty() {
            config.stream_token = generate_install_token();
            migrated = true;
        }
        if config.overlay_profiles.is_empty() {
            config.overlay_profiles.push(OverlayProfile::default());
            migrated = true;
        }
        migrated |= config.normalize_dark_overlay_themes();
        config.validate()?;
        if migrated {
            config.save(path)?;
        }
        Ok(config)
    }

    fn normalize_dark_overlay_themes(&mut self) -> bool {
        let mut changed = false;
        for profile in &mut self.overlay_profiles {
            if !matches!(profile.theme.as_str(), "void" | "glass") {
                profile.theme = "void".to_owned();
                changed = true;
            }
        }
        changed
    }

    pub fn validate(&self) -> Result<()> {
        if self.port == 0 {
            anyhow::bail!("port 0 is not supported because OBS needs a stable local URL");
        }
        if self.log_directory.as_os_str().is_empty() {
            anyhow::bail!("VRChat log directory cannot be empty");
        }
        if self.stream_token.len() < 32
            || !self
                .stream_token
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            anyhow::bail!("local API token is invalid");
        }
        if !(260.0..=3840.0).contains(&self.desktop_overlay.width)
            || !(120.0..=2160.0).contains(&self.desktop_overlay.height)
        {
            anyhow::bail!("desktop overlay size is outside the supported range");
        }
        if !(0.0..=1.0).contains(&self.desktop_overlay.opacity) {
            anyhow::bail!("desktop overlay opacity must be between 0 and 1");
        }
        if !matches!(
            self.desktop_overlay.corner.as_str(),
            "top-left" | "top-right" | "bottom-left" | "bottom-right"
        ) {
            anyhow::bail!("desktop overlay corner is invalid");
        }
        if self.vr_overlay.sanitized() != self.vr_overlay {
            anyhow::bail!("VR overlay placement or row settings are outside supported ranges");
        }
        for profile in &self.overlay_profiles {
            if !matches!(profile.theme.as_str(), "void" | "glass") {
                anyhow::bail!(
                    "overlay profile `{}` requests an unsupported theme; MINMAXXER is dark-only",
                    profile.id
                );
            }
            if !(1..=12).contains(&profile.rows)
                || !(1..=12).contains(&profile.recent_hit_rows)
                || !profile.scale.is_finite()
                || !(0.5..=2.0).contains(&profile.scale)
            {
                anyhow::bail!("overlay profile `{}` has invalid rows or scale", profile.id);
            }
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed creating {}", parent.display()))?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("failed writing {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("failed replacing {}", path.display()))?;
        Ok(())
    }
}

pub fn app_data_directory() -> PathBuf {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data).join("MINMAXXER");
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".minmaxxer")
}

pub fn default_vrchat_log_directory() -> PathBuf {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile)
            .join("AppData")
            .join("LocalLow")
            .join("VRChat")
            .join("VRChat");
    }
    PathBuf::from(".")
}

fn generate_install_token() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system did not provide secure randomness");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_light_profiles_migrate_once_and_cannot_validate_again() {
        let mut config = AppConfig::default();
        config.overlay_profiles[0].theme = "light".to_owned();

        assert!(config.validate().is_err());
        assert!(config.normalize_dark_overlay_themes());
        assert_eq!(config.overlay_profiles[0].theme, "void");
        assert!(config.validate().is_ok());
        assert!(!config.normalize_dark_overlay_themes());
    }
}
