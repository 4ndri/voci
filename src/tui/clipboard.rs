//! Keep native clipboard ownership alive for the entire TUI session.

pub trait Clipboard {
    fn copy(&mut self, text: String) -> Result<(), String>;
    fn paste(&mut self) -> Result<String, String>;
}
#[derive(Default)]
pub struct DesktopClipboard {
    inner: Option<arboard::Clipboard>,
}
impl DesktopClipboard {
    fn get(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.inner.is_none() {
            self.inner = Some(arboard::Clipboard::new().map_err(|_| {
                "Desktop clipboard unavailable in this terminal environment.".to_string()
            })?);
        }
        Ok(self.inner.as_mut().unwrap())
    }
}
impl Clipboard for DesktopClipboard {
    fn copy(&mut self, text: String) -> Result<(), String> {
        self.get()?
            .set_text(text)
            .map_err(|_| "Cannot write to the desktop clipboard. Try again.".to_string())
    }
    fn paste(&mut self) -> Result<String, String> {
        self.get()?.get_text().map_err(|_| {
            "Cannot read text from the desktop clipboard. Copy some text and try again.".to_string()
        })
    }
}
