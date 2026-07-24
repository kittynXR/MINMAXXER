use crate::vr_overlay::VrOverlaySettings;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 49_321;
pub const OVERLAY_SETTINGS_SCHEMA_VERSION: u8 = 4;

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
    pub show_phase: bool,
    pub show_boss_number: bool,
    pub show_focus: bool,
    pub show_graph: bool,
    pub show_survival: bool,
    pub show_telemetry: bool,
    pub show_loadout: bool,
    pub anonymize_players: bool,
}

impl Default for OverlayProfile {
    fn default() -> Self {
        Self {
            id: "broadcast".to_owned(),
            name: "Broadcast HUD".to_owned(),
            layout: "landscape".to_owned(),
            theme: "void".to_owned(),
            accent: "cyan".to_owned(),
            background: "rgba(5, 7, 11, 0.94)".to_owned(),
            rows: 8,
            scale: 1.0,
            show_dps: true,
            show_damage: true,
            show_incoming: true,
            show_hits: false,
            show_recent_hits: true,
            recent_hit_rows: 5,
            show_encounter: true,
            show_phase: true,
            show_boss_number: true,
            show_focus: true,
            show_graph: true,
            show_survival: true,
            show_telemetry: false,
            show_loadout: false,
            anonymize_players: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub overlay_schema_version: u8,
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
            overlay_schema_version: OVERLAY_SETTINGS_SCHEMA_VERSION,
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
        let raw: serde_json::Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed parsing config {}", path.display()))?;
        let mut config: Self = serde_json::from_value(raw.clone())
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
        migrated |= config.migrate_overlay_schema(&raw);
        migrated |= config.inherit_legacy_run_context_visibility(&raw);
        migrated |= config.normalize_overlay_profiles();
        config.validate()?;
        if migrated {
            config.save(path)?;
        }
        Ok(config)
    }

    fn migrate_overlay_schema(&mut self, raw: &serde_json::Value) -> bool {
        let persisted_version = raw
            .get("overlay_schema_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        if persisted_version >= u64::from(OVERLAY_SETTINGS_SCHEMA_VERSION) {
            return false;
        }

        // v0.4 shipped these unavailable placeholders enabled by default. Treat every
        // pre-schema-4 profile as that old default exactly once; schema-4 profiles can
        // still opt back in explicitly from Overlay Studio.
        for profile in &mut self.overlay_profiles {
            profile.show_telemetry = false;
        }
        self.overlay_schema_version = OVERLAY_SETTINGS_SCHEMA_VERSION;
        true
    }

    fn inherit_legacy_run_context_visibility(&mut self, raw: &serde_json::Value) -> bool {
        let mut changed = false;
        if let Some(raw_profiles) = raw
            .get("overlay_profiles")
            .and_then(|value| value.as_array())
        {
            for (profile, raw_profile) in self.overlay_profiles.iter_mut().zip(raw_profiles) {
                let Some(raw_profile) = raw_profile.as_object() else {
                    continue;
                };
                if !raw_profile.contains_key("show_phase") {
                    profile.show_phase = profile.show_encounter;
                    changed = true;
                }
                if !raw_profile.contains_key("show_boss_number") {
                    profile.show_boss_number = profile.show_encounter;
                    changed = true;
                }
            }
        }

        if let Some(raw_vr) = raw.get("vr_overlay").and_then(|value| value.as_object()) {
            if !raw_vr.contains_key("show_phase") {
                self.vr_overlay.show_phase = self.vr_overlay.show_encounter;
                changed = true;
            }
            if !raw_vr.contains_key("show_boss_number") {
                self.vr_overlay.show_boss_number = self.vr_overlay.show_encounter;
                changed = true;
            }
        }
        changed
    }

    fn normalize_overlay_profiles(&mut self) -> bool {
        let mut changed = false;
        for profile in &mut self.overlay_profiles {
            if !matches!(profile.layout.as_str(), "portrait" | "landscape") {
                profile.layout = if profile.layout == "hits" {
                    "portrait".to_owned()
                } else {
                    "landscape".to_owned()
                };
                changed = true;
            }
            if !matches!(profile.theme.as_str(), "void" | "glass") {
                profile.theme = "void".to_owned();
                changed = true;
            }
            if profile.accent == "#8ff0cf" {
                profile.accent = "mint".to_owned();
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
            if !matches!(profile.layout.as_str(), "portrait" | "landscape") {
                anyhow::bail!(
                    "overlay profile `{}` has an invalid orientation",
                    profile.id
                );
            }
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
    fn legacy_overlay_profiles_inherit_run_context_without_enabling_loadout() {
        let profile: OverlayProfile =
            serde_json::from_value(serde_json::json!({ "id": "legacy" })).unwrap();

        assert!(profile.show_phase);
        assert!(profile.show_boss_number);
        assert!(profile.show_graph);
        assert!(profile.show_survival);
        assert!(!profile.show_telemetry);
        assert!(!profile.show_loadout);
    }

    #[test]
    fn pre_v4_overlay_profiles_disable_unavailable_telemetry_once() {
        let mut raw = serde_json::to_value(AppConfig::default()).unwrap();
        raw.as_object_mut()
            .unwrap()
            .remove("overlay_schema_version");
        raw["overlay_profiles"][0]["show_telemetry"] = serde_json::json!(true);

        let mut config: AppConfig = serde_json::from_value(raw.clone()).unwrap();
        assert!(config.overlay_profiles[0].show_telemetry);
        assert!(config.migrate_overlay_schema(&raw));
        assert_eq!(
            config.overlay_schema_version,
            OVERLAY_SETTINGS_SCHEMA_VERSION
        );
        assert!(!config.overlay_profiles[0].show_telemetry);

        let migrated_raw = serde_json::to_value(&config).unwrap();
        assert!(!config.migrate_overlay_schema(&migrated_raw));
    }

    #[test]
    fn schema_v4_preserves_explicit_unavailable_telemetry_opt_in() {
        let mut raw = serde_json::to_value(AppConfig::default()).unwrap();
        raw["overlay_profiles"][0]["show_telemetry"] = serde_json::json!(true);

        let mut config: AppConfig = serde_json::from_value(raw.clone()).unwrap();
        assert!(!config.migrate_overlay_schema(&raw));
        assert!(config.overlay_profiles[0].show_telemetry);
    }

    #[test]
    fn on_disk_legacy_visibility_inherits_the_encounter_choice() {
        let mut raw = serde_json::to_value(AppConfig::default()).unwrap();
        let profile = raw["overlay_profiles"][0].as_object_mut().unwrap();
        profile.insert("show_encounter".to_owned(), serde_json::json!(false));
        profile.remove("show_phase");
        profile.remove("show_boss_number");
        let vr = raw["vr_overlay"].as_object_mut().unwrap();
        vr.insert("show_encounter".to_owned(), serde_json::json!(false));
        vr.remove("show_phase");
        vr.remove("show_boss_number");

        let mut config: AppConfig = serde_json::from_value(raw.clone()).unwrap();
        assert!(config.overlay_profiles[0].show_phase);
        assert!(config.overlay_profiles[0].show_boss_number);
        assert!(config.vr_overlay.show_phase);
        assert!(config.vr_overlay.show_boss_number);
        assert!(config.inherit_legacy_run_context_visibility(&raw));
        assert!(!config.overlay_profiles[0].show_phase);
        assert!(!config.overlay_profiles[0].show_boss_number);
        assert!(!config.vr_overlay.show_phase);
        assert!(!config.vr_overlay.show_boss_number);
    }

    #[test]
    fn legacy_light_profiles_migrate_once_and_cannot_validate_again() {
        let mut config = AppConfig::default();
        config.overlay_profiles[0].theme = "light".to_owned();

        assert!(config.validate().is_err());
        assert!(config.normalize_overlay_profiles());
        assert_eq!(config.overlay_profiles[0].theme, "void");
        assert!(config.validate().is_ok());
        assert!(!config.normalize_overlay_profiles());
    }

    #[test]
    fn legacy_hex_accent_migrates_to_the_named_mint_swatch() {
        let mut config = AppConfig::default();
        config.overlay_profiles[0].accent = "#8ff0cf".to_owned();

        assert!(config.normalize_overlay_profiles());
        assert_eq!(config.overlay_profiles[0].accent, "mint");
        assert!(!config.normalize_overlay_profiles());
    }

    #[test]
    fn legacy_overlay_layouts_migrate_to_stream_orientations() {
        let mut config = AppConfig::default();
        config.overlay_profiles[0].layout = "hits".to_owned();

        assert!(config.normalize_overlay_profiles());
        assert_eq!(config.overlay_profiles[0].layout, "portrait");

        config.overlay_profiles[0].layout = "leaderboard".to_owned();
        assert!(config.normalize_overlay_profiles());
        assert_eq!(config.overlay_profiles[0].layout, "landscape");
        assert!(!config.normalize_overlay_profiles());
    }
}
