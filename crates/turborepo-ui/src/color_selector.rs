use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, RwLock},
};

use console::{Style, StyledObject};

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
    inner: Arc<RwLock<ColorSelectorInner>>,
}

#[derive(Default)]
struct ColorSelectorInner {
    idx: usize,
    cache: HashMap<String, &'static Style>,
}

impl ColorSelector {
    pub fn color_for_key(&self, key: &str) -> &'static Style {
        if let Some(style) = self.inner.read().expect("lock poisoned").color(key) {
            return style;
        }

        let color = {
            self.inner
                .write()
                .expect("lock poisoned")
                .insert_color(key.to_string())
        };

        color
    }

    pub fn prefix_with_color(&self, cache_key: &str, prefix: &str) -> StyledObject<String> {
        if prefix.is_empty() {
            return Style::new().apply_to(String::new());
        }

        let style = self.color_for_key(cache_key);
        style.apply_to(format!("{}: ", prefix))
    }
}

impl ColorSelectorInner {
    fn color(&self, key: &str) -> Option<&'static Style> {
        self.cache.get(key).copied()
    }

    fn insert_color(&mut self, key: String) -> &'static Style {
        let colors = get_terminal_package_colors();
        let chosen_color = &colors[self.idx % colors.len()];
        // A color might have been chosen by the time we get to inserting
        self.cache.entry(key).or_insert_with(|| {
            // If a color hasn't been chosen, then we increment the index
            self.idx += 1;
            chosen_color
        })
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    #[test]
    fn test_color_selector() {
        let selector = super::ColorSelector::default();
        let color1 = selector.color_for_key("key1");
        let color2 = selector.color_for_key("key2");
        let color3 = selector.color_for_key("key1");
        assert_eq!(color1, color3);
        assert_ne!(color1, color2);
    }

    #[test]
    fn test_multithreaded_selector() {
        let selector = super::ColorSelector::default();
        thread::scope(|s| {
            s.spawn(|| {
                let color = selector.color_for_key("key1");
                assert_eq!(color, selector.color_for_key("key1"));
            });
            s.spawn(|| {
                let color = selector.color_for_key("key2");
                assert_eq!(color, selector.color_for_key("key2"));
            });
            s.spawn(|| {
                let color1 = selector.color_for_key("key1");
                let color2 = selector.color_for_key("key2");
                assert_eq!(color1, selector.color_for_key("key1"));
                assert_eq!(color2, selector.color_for_key("key2"));
                assert_ne!(color1, color2);
            });
        });
        // We only inserted 2 keys so next index should be 2
        assert_eq!(selector.inner.read().unwrap().idx, 2);
    }

    #[test]
    fn test_color_selector_wraps_around() {
        let selector = super::ColorSelector::default();
        for key in &["1", "2", "3", "4", "5", "6"] {
            selector.color_for_key(key);
        }
        assert_eq!(selector.color_for_key("1"), selector.color_for_key("6"));
    }
}
