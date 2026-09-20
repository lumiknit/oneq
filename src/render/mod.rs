pub mod color;
pub mod format;

pub use color::*;
pub use format::*;

#[derive(Clone)]
pub struct Options {
    pub out: FormatOptions,
    pub theme: Option<ColorTheme>,
}

impl Options {
    pub fn new(out: FormatOptions) -> Self {
        Self { out, theme: None }
    }

    pub fn with_theme(&mut self, theme: ColorTheme) -> &mut Self {
        self.theme = Some(theme);
        self
    }

    pub fn style_ansi(&self, idx: ThemeIdx) -> Option<TextStyle> {
        self.theme.as_ref()?.styles[idx as usize].clone().into()
    }

    pub fn style_reset(&self) -> Option<&'static str> {
        self.theme.as_ref().map(|_| RESET)
    }

    /// prints a string with color if the theme is set.
    pub fn styled<P>(&self, s: &str, idx: ThemeIdx, mut printer: P)
    where
        P: FnMut(&str),
    {
        if let Some(ansi) = self.style_ansi(idx) {
            printer(ansi.to_string().as_str());
            printer(s);
            printer(RESET);
        } else {
            printer(s);
        }
    }
}
