use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};

use console::Style;

static COLORS: OnceLock<[Style; 5]> = OnceLock::new();

pub fn get_terminal_package_colors() -> &'static [Style; 5] {
    COLORS.get_or_init(|| {
        [
            Style::new().cyan(),
            Style::new().magenta(),
            Style::new().green(),
            Style::new().yellow(),
            Style::new().blue(),
        ]
    })
}

/// Selects colors for tasks and caches accordingly.
/// Shared between tasks so allows for concurrent access.
#[derive(Default)]
pub struct ColorSelector {
    idx: AtomicUsize,
    cache: HashMap<String, Style>,
}

impl ColorSelector {
    pub fn color_for_key(&mut self, key: &str) -> Style {
        if let Some(style) = self.cache.get(key) {
            return style.clone();
        }

        let colors = get_terminal_package_colors();

        let idx = self.idx.fetch_add(1, Ordering::Relaxed);
        let color = colors[idx % colors.len()].clone();
        self.cache.insert(key.to_string(), color.clone());

        color
    }

    pub fn prefix_with_color(&mut self, cache_key: &str, prefix: &str) -> String {
        if prefix.is_empty() {
            return "".into();
        }

        let style = self.color_for_key(cache_key);
        style.apply_to(format!("{}: ", prefix)).to_string()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_color_selector() {
        let mut selector = super::ColorSelector::default();
        let color1 = selector.color_for_key("key1");
        let color2 = selector.color_for_key("key2");
        let color3 = selector.color_for_key("key1");
        assert_eq!(color1, color3);
        assert_ne!(color1, color2);
    }
}
