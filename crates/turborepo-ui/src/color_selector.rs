use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
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

        let idx = self.idx.load(Ordering::Relaxed) + 1;
        self.idx.store(idx, Ordering::Relaxed);
        let color = colors[idx % colors.len()].clone();
        self.cache.insert(key.to_string(), color.clone());

        color
    }

    pub fn prefix_with_color(&mut self, cache_key: &str, prefix: &str) -> String {
        if prefix == "" {
            return "".into();
        }

        let style = self.color_for_key(cache_key);
        style.apply_to(format!("{}: ", prefix)).to_string()
    }
}
