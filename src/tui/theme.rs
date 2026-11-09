//! Los tokens de color de Claude Code que usa esta pantalla. Ningún widget usa un
//! `Color` literal: todo pasa por aquí.

use ratatui::style::{Color, Modifier, Style};

/// (token, hex, índice xterm-256 más cercano): los colores de Claude Code.
/// `claude` es el naranja de la marca; `claude-soft`, su versión clara.
const TOKENS: [(&str, &str, u8); 9] = [
    ("subtle", "#505050", 239),
    ("claude", "#d77757", 173),
    ("claude-soft", "#eb9f7f", 216),
    ("ink", "#faf9f5", 255),
    ("ink-muted", "#999999", 246),
    ("alert", "#ff6b80", 204),
    ("on-claude", "#141413", 233),
    // ámbar de aviso: el contexto casi lleno
    ("warning", "#e5b567", 179),
    // el verde de éxito de Claude Code: la actualización instalada
    ("success", "#4eba65", 71),
];

pub struct Theme {
    pub subtle: Color,
    pub claude: Color,
    pub claude_soft: Color,
    pub ink: Color,
    pub ink_muted: Color,
    pub alert: Color,
    pub on_claude: Color,
    pub warning: Color,
    pub success: Color,
}

pub fn parse_hex(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

impl Theme {
    fn build(color: impl Fn(usize) -> Color) -> Self {
        Self {
            subtle: color(0),
            claude: color(1),
            claude_soft: color(2),
            ink: color(3),
            ink_muted: color(4),
            alert: color(5),
            on_claude: color(6),
            warning: color(7),
            success: color(8),
        }
    }

    /// Paleta de Claude Code en truecolor.
    pub fn claude() -> Self {
        Self::build(|i| parse_hex(TOKENS[i].1).expect("hex válido en TOKENS"))
    }

    /// Aproximación a 256 colores para terminales sin truecolor.
    pub fn claude_256() -> Self {
        Self::build(|i| Color::Indexed(TOKENS[i].2))
    }

    /// La paleta completa si el terminal anuncia truecolor; si no, la de 256.
    pub fn detect() -> Self {
        match std::env::var("COLORTERM").as_deref() {
            Ok("truecolor") | Ok("24bit") => Self::claude(),
            _ => Self::claude_256(),
        }
    }

    pub fn text(&self) -> Style {
        Style::new().fg(self.ink)
    }
    /// Cifras de la ficha: `ink` en negrita.
    pub fn text_bold(&self) -> Style {
        Style::new().fg(self.ink).add_modifier(Modifier::BOLD)
    }
    pub fn muted(&self) -> Style {
        Style::new().fg(self.ink_muted)
    }
    pub fn accent(&self) -> Style {
        Style::new().fg(self.claude)
    }
    pub fn accent_bold(&self) -> Style {
        Style::new().fg(self.claude).add_modifier(Modifier::BOLD)
    }
    pub fn soft(&self) -> Style {
        Style::new().fg(self.claude_soft)
    }
    pub fn soft_bold(&self) -> Style {
        Style::new()
            .fg(self.claude_soft)
            .add_modifier(Modifier::BOLD)
    }
    /// Error del hilo de lectura.
    pub fn alert(&self) -> Style {
        Style::new().fg(self.alert)
    }
    /// Aviso: el contexto desde el 85 %.
    pub fn warn(&self) -> Style {
        Style::new().fg(self.warning)
    }
    /// Lo que salió bien: `✓` de `claude-agents update`.
    pub fn ok(&self) -> Style {
        Style::new().fg(self.success)
    }
    pub fn line(&self) -> Style {
        Style::new().fg(self.subtle)
    }
    /// La pestaña abierta: fondo `claude`, texto `on-claude`. Lo único de la
    /// pantalla que pinta fondo, para que el ojo vaya ahí.
    pub fn selected(&self) -> Style {
        Style::new().fg(self.on_claude).bg(self.claude)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    #[test]
    fn hex_parses_six_digit_colors_only() {
        assert_eq!(parse_hex("#8fb8ff"), Some(Color::Rgb(0x8f, 0xb8, 0xff)));
        assert_eq!(parse_hex("8fb8ff"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
    }

    #[test]
    fn truecolor_palette_uses_rgb() {
        assert_eq!(Theme::claude().claude, Color::Rgb(0xd7, 0x77, 0x57));
    }

    #[test]
    fn fallback_palette_uses_indexed_colors() {
        let t = Theme::claude_256();
        assert_eq!(t.claude, Color::Indexed(173));
        assert_eq!(t.alert, Color::Indexed(204));
        assert_eq!(t.on_claude, Color::Indexed(233));
    }

    #[test]
    fn figures_are_ink_in_bold() {
        let t = Theme::claude();
        assert_eq!(t.text_bold().fg, Some(Color::Rgb(0xfa, 0xf9, 0xf5)));
        assert!(t.text_bold().add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn warning_is_amber_in_both_palettes() {
        assert_eq!(Theme::claude().warning, Color::Rgb(0xe5, 0xb5, 0x67));
        assert_eq!(Theme::claude_256().warning, Color::Indexed(179));
        assert_eq!(
            Theme::claude().warn().fg,
            Some(Color::Rgb(0xe5, 0xb5, 0x67))
        );
    }

    #[test]
    fn success_is_green_in_both_palettes() {
        assert_eq!(Theme::claude().success, Color::Rgb(0x4e, 0xba, 0x65));
        assert_eq!(Theme::claude_256().success, Color::Indexed(71));
        assert_eq!(Theme::claude().ok().fg, Some(Color::Rgb(0x4e, 0xba, 0x65)));
    }

    #[test]
    fn selected_is_on_claude_over_claude() {
        let t = Theme::claude();
        assert_eq!(t.selected().bg, Some(Color::Rgb(0xd7, 0x77, 0x57)));
        assert_eq!(t.selected().fg, Some(Color::Rgb(0x14, 0x14, 0x13)));
    }
}
