use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::Path,
};

pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub dark: bool,
    pub background: u32,
    pub surface: u32,
    pub panel: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub accent_text: u32,
    pub accent_hover: u32,
    pub accent_active: u32,
    pub accent_soft: u32,
    pub accent_ink: u32,
    pub success: u32,
    pub success_soft: u32,
    pub success_mark: u32,
    pub error: u32,
    pub error_soft: u32,
    pub error_mark: u32,
    pub warning: u32,
}

const PORCELAIN: Theme = Theme {
    id: "porcelain",
    name: "Porzellan & Kobaltblau",
    dark: false,
    background: 0xEDF2F8,
    surface: 0xFFFFFF,
    panel: 0xE6ECF5,
    border: 0xC9D5E5,
    text: 0x13243F,
    muted: 0x455B75,
    accent: 0x2456E6,
    accent_text: 0xFFFFFF,
    accent_hover: 0x1B48CE,
    accent_active: 0x183AB0,
    accent_soft: 0xE0EAFD,
    accent_ink: 0x1943BA,
    success: 0x145C38,
    success_soft: 0xDDF1E3,
    success_mark: 0x239653,
    error: 0xA52228,
    error_soft: 0xFBE6E8,
    error_mark: 0xD63C46,
    warning: 0x815111,
};

const GRAPHITE: Theme = Theme {
    id: "graphite",
    name: "Graphit & Bernstein",
    dark: true,
    background: 0x191B20,
    surface: 0x23262C,
    panel: 0x2C3037,
    border: 0x40454E,
    text: 0xF2EDE3,
    muted: 0xB6B8BE,
    accent: 0xF2B544,
    accent_text: 0x221809,
    accent_hover: 0xFFC760,
    accent_active: 0xDBA036,
    accent_soft: 0x382E21,
    accent_ink: 0xFFD17B,
    success: 0xB5D8AB,
    success_soft: 0x2D3C32,
    success_mark: 0x92C786,
    error: 0xF99588,
    error_soft: 0x473030,
    error_mark: 0xEB7F71,
    warning: 0xF7C775,
};

pub const THEMES: &[&Theme] = &[&PORCELAIN, &GRAPHITE];
pub const DEFAULT_THEME: &Theme = &GRAPHITE;

#[derive(Clone, Copy, Default, Deserialize, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerLayout {
    #[default]
    Matrix,
    Cards,
}

pub struct Appearance {
    pub theme: &'static Theme,
    pub server_layout: ServerLayout,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: DEFAULT_THEME,
            server_layout: ServerLayout::default(),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct StoredAppearance {
    theme: String,
    #[serde(default)]
    server_layout: ServerLayout,
}

pub fn load(path: &Path) -> Result<Appearance> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Appearance::default()),
        Err(error) => {
            return Err(error).context("Design-Einstellungen konnten nicht gelesen werden");
        }
    };
    let settings: StoredAppearance =
        toml::from_str(&contents).context("Design-Einstellungen sind ungültig")?;
    Ok(Appearance {
        theme: THEMES
            .iter()
            .copied()
            .find(|theme| theme.id == settings.theme)
            .unwrap_or(DEFAULT_THEME),
        server_layout: settings.server_layout,
    })
}

pub fn save(path: &Path, theme: &Theme, server_layout: ServerLayout) -> Result<()> {
    let directory = path
        .parent()
        .context("Verzeichnis für Design-Einstellungen fehlt")?;
    fs::create_dir_all(directory)?;
    let contents = toml::to_string(&StoredAppearance {
        theme: theme.id.into(),
        server_layout,
    })?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(contents.as_bytes())?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_each_theme_across_reloads_without_touching_health_checks() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("appearance.toml");
        let config = directory.path().join("config.toml");
        fs::write(&config, "interval_seconds = 60")?;
        assert_eq!(load(&path)?.theme.id, DEFAULT_THEME.id);

        for theme in THEMES {
            save(&path, theme, ServerLayout::Cards)?;
            assert_eq!(load(&path)?.server_layout, ServerLayout::Cards);
            assert_eq!(load(&path)?.theme.id, theme.id);
        }

        assert_eq!(fs::read_to_string(config)?, "interval_seconds = 60");
        fs::write(&path, "theme = 'porcelain'")?;
        let legacy = load(&path)?;
        assert_eq!(legacy.theme.id, "porcelain");
        assert_eq!(legacy.server_layout, ServerLayout::Matrix);
        save(&path, legacy.theme, ServerLayout::Matrix)?;
        assert_eq!(load(&path)?.server_layout, ServerLayout::Matrix);
        fs::write(&path, "theme = 'removed-theme'")?;
        assert_eq!(load(&path)?.theme.id, DEFAULT_THEME.id);
        Ok(())
    }
}
