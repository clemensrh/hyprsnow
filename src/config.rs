use serde::{Deserialize, Serialize};
use std::{env, fs, fs::OpenOptions, io::Write, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnowOption {
    Enabled(bool),
    Accumulation(bool),
    Density(f32),
    WindStrength(f32),
    ParticleSize(f32),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub general: GeneralSettings,
    pub snow: SnowSettings,
    pub windows: WindowSettings,
    pub physics: PhysicsSettings,
    pub interaction: InteractionSettings,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct GeneralSettings {
    pub enabled: bool,
    pub fps_limit: u32,
    pub pause_on_fullscreen: bool,
    pub start_with_session: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct SnowSettings {
    pub enabled: bool,
    pub density: f32,
    pub wind_strength: f32,
    pub particle_size: f32,
    pub accumulation: bool,
    pub accumulation_speed: f32,
    pub max_snow_depth: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct WindowSettings {
    pub enabled: bool,
    pub accumulate_on_windows: bool,
    pub max_snow_depth: f32,
    pub track_floating: bool,
    pub track_tiled: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct PhysicsSettings {
    pub enabled: bool,
    pub gravity: f32,
    pub snow_compaction: f32,
    pub sliding: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct InteractionSettings {
    pub snowman_building: bool,
    pub interaction_key: String,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            fps_limit: 60,
            pause_on_fullscreen: true,
            start_with_session: true,
        }
    }
}

impl Default for SnowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            density: 0.7,
            wind_strength: 0.25,
            particle_size: 1.0,
            accumulation: true,
            accumulation_speed: 1.0,
            max_snow_depth: 120.0,
        }
    }
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            accumulate_on_windows: true,
            max_snow_depth: 32.0,
            track_floating: true,
            track_tiled: true,
        }
    }
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            gravity: 9.81,
            snow_compaction: 0.4,
            sliding: true,
        }
    }
}

impl Default for InteractionSettings {
    fn default() -> Self {
        Self {
            snowman_building: true,
            interaction_key: "SUPER SHIFT S".to_owned(),
        }
    }
}

impl Settings {
    pub fn load() -> Result<Self, String> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let settings: Self = toml::from_str(&contents)
            .map_err(|error| format!("invalid TOML in {}: {error}", path.display()))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=240).contains(&self.general.fps_limit) {
            return Err("general.fps_limit must be between 1 and 240".to_owned());
        }
        check_range("snow.density", self.snow.density, 0.0, 1.0)?;
        check_range("snow.wind_strength", self.snow.wind_strength, 0.0, 1.0)?;
        check_range("snow.particle_size", self.snow.particle_size, 0.25, 4.0)?;
        check_range(
            "snow.accumulation_speed",
            self.snow.accumulation_speed,
            0.0,
            4.0,
        )?;
        check_range("snow.max_snow_depth", self.snow.max_snow_depth, 0.0, 500.0)?;
        check_range(
            "windows.max_snow_depth",
            self.windows.max_snow_depth,
            0.0,
            500.0,
        )?;
        check_range("physics.gravity", self.physics.gravity, 0.0, 50.0)?;
        check_range(
            "physics.snow_compaction",
            self.physics.snow_compaction,
            0.0,
            1.0,
        )?;
        if self.interaction.interaction_key.trim().is_empty() {
            return Err("interaction.interaction_key cannot be empty".to_owned());
        }
        Ok(())
    }
}

pub fn update_snow_option(option: SnowOption) -> Result<Settings, String> {
    let path = config_path()?;
    let mut document = if path.exists() {
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        contents
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| format!("invalid TOML in {}: {error}", path.display()))?
    } else {
        toml::to_string_pretty(&Settings::default())
            .map_err(|error| error.to_string())?
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| error.to_string())?
    };

    update_document_snow_option(&mut document, option)?;
    let contents = document.to_string();
    let settings: Settings = toml::from_str(&contents)
        .map_err(|error| format!("invalid TOML in {}: {error}", path.display()))?;
    settings.validate()?;
    write_atomic(&path, contents.as_bytes())?;
    Ok(settings)
}

fn update_document_snow_option(
    document: &mut toml_edit::DocumentMut,
    option: SnowOption,
) -> Result<(), String> {
    if !document.contains_key("snow") {
        document["snow"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    if matches!(option, SnowOption::Enabled(_)) && !document.contains_key("general") {
        document["general"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    match option {
        SnowOption::Enabled(value) => {
            snow_table_mut(document)?["enabled"] = toml_edit::value(value);
            document
                .get_mut("general")
                .and_then(toml_edit::Item::as_table_mut)
                .ok_or("general settings must be a TOML table")?["enabled"] =
                toml_edit::value(value);
        }
        SnowOption::Accumulation(value) => {
            snow_table_mut(document)?["accumulation"] = toml_edit::value(value);
        }
        SnowOption::Density(value) => {
            snow_table_mut(document)?["density"] = float_value(value)?;
        }
        SnowOption::WindStrength(value) => {
            snow_table_mut(document)?["wind_strength"] = float_value(value)?;
        }
        SnowOption::ParticleSize(value) => {
            snow_table_mut(document)?["particle_size"] = float_value(value)?;
        }
    }
    Ok(())
}

fn snow_table_mut(document: &mut toml_edit::DocumentMut) -> Result<&mut toml_edit::Table, String> {
    document
        .get_mut("snow")
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| "snow settings must be a TOML table".to_owned())
}

fn float_value(value: f32) -> Result<toml_edit::Item, String> {
    let value = value
        .to_string()
        .parse::<f64>()
        .map_err(|error| format!("cannot serialize snow setting: {error}"))?;
    Ok(toml_edit::value(value))
}

pub fn ensure_config_file() -> Result<PathBuf, String> {
    let path = config_path()?;
    if !path.exists() {
        let contents =
            toml::to_string_pretty(&Settings::default()).map_err(|error| error.to_string())?;
        write_atomic(&path, contents.as_bytes())?;
    }
    Ok(path)
}

fn write_atomic(path: &PathBuf, contents: &[u8]) -> Result<(), String> {
    let path = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::canonicalize(path).map_err(|error| {
                format!(
                    "cannot resolve configuration symlink {}: {error}",
                    path.display()
                )
            })?
        }
        Ok(_) => path.clone(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.clone(),
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    };
    let parent = path
        .parent()
        .ok_or_else(|| format!("configuration path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        std::process::id()
    ));
    let permissions = match fs::metadata(&path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("cannot create {}: {error}", temporary.display()))?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions).map_err(|error| {
            format!("cannot set permissions on {}: {error}", temporary.display())
        })?;
    }
    file.write_all(contents)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    file.sync_all()
        .map_err(|error| format!("cannot sync {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))?;
    Ok(())
}

pub fn config_path() -> Result<PathBuf, String> {
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home)
            .join("christmas-snow")
            .join("config.toml"));
    }
    let home = env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("christmas-snow")
        .join("config.toml"))
}

fn check_range(name: &str, value: f32, min: f32, max: f32) -> Result<(), String> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_configuration() {
        let settings = Settings::default();
        assert_eq!(settings.general.fps_limit, 60);
        assert_eq!(settings.snow.density, 0.7);
        assert_eq!(settings.snow.max_snow_depth, 120.0);
        assert_eq!(settings.windows.max_snow_depth, 32.0);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn validation_rejects_non_finite_physics_values() {
        let mut settings = Settings::default();
        settings.snow.wind_strength = f32::NAN;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn updating_a_snow_option_preserves_comments_and_other_settings() {
        let mut document = r#"
[snow]
# Keep this user note.
enabled = true
density = 0.7
wind_strength = 0.25
particle_size = 1.0
accumulation = true
accumulation_speed = 1.0
max_snow_depth = 120

[windows]
enabled = false
"#
        .parse::<toml_edit::DocumentMut>()
        .unwrap();

        update_document_snow_option(&mut document, SnowOption::Density(0.5)).unwrap();

        let contents = document.to_string();
        assert!(contents.contains("# Keep this user note."));
        assert!(contents.contains("density = 0.5"));
        assert!(contents.contains("wind_strength = 0.25"));
        assert!(contents.contains("enabled = false"));
    }

    #[test]
    fn updating_a_snow_option_adds_a_missing_snow_table() {
        let mut document = "[general]\nenabled = true\n"
            .parse::<toml_edit::DocumentMut>()
            .unwrap();

        update_document_snow_option(&mut document, SnowOption::Accumulation(false)).unwrap();

        let settings: Settings = toml::from_str(&document.to_string()).unwrap();
        assert!(!settings.snow.accumulation);
        assert!(settings.general.enabled);
    }

    #[test]
    fn snowfall_toggle_updates_global_and_snow_enabled_flags() {
        let mut document = r#"
[general]
enabled = false

[snow]
enabled = false
"#
        .parse::<toml_edit::DocumentMut>()
        .unwrap();

        update_document_snow_option(&mut document, SnowOption::Enabled(true)).unwrap();

        let settings: Settings = toml::from_str(&document.to_string()).unwrap();
        assert!(settings.general.enabled);
        assert!(settings.snow.enabled);
    }
}
