//! The themes that ship with the window, each written in the colors of its own published palette.
//!
//! Each row names the slots in the order of [`youarel_core::theme_rules::ALL`]: background, sidebar, surface, raised, border, text, muted, accent, accent text, selection, go, go text, danger, the scheme, the subdomains, the name, the suffix, the port, and the rest of a link, and the thumbnail.

use youarel_core::theme_rules::{Kind, SLOTS};

/// One bundled theme: its identifier, its name, its kind, and its colors.
pub type Row = (&'static str, &'static str, Kind, [&'static str; SLOTS]);

/// The bundled themes, the light ones first.
pub const THEMES: [Row; 16] = [
    (
        "github-light",
        "GitHub Light",
        Kind::Light,
        [
            "#ffffff", "#f6f8fa", "#f6f8fa", "#ffffff", "#d1d9e0", "#1f2328", "#59636e", "#0969da",
            "#ffffff", "#ddf4ff", "#1f883d", "#ffffff", "#cf222e", "#59636e", "#59636e", "#8250df",
            "#cf222e", "#0550ae", "#0a3069", "#f6f8fa",
        ],
    ),
    (
        "ayu-light",
        "Ayu Light",
        Kind::Light,
        [
            "#fcfcfc", "#f8f9fa", "#f3f4f5", "#ffffff", "#e7eaed", "#5c6166", "#8a9199", "#fa8d3e",
            "#fcfcfc", "#ffe1c2", "#6cbf43", "#fcfcfc", "#e65050", "#adaeb1", "#8a9199", "#fa8d3e",
            "#a37acc", "#399ee6", "#86b300", "#f3f4f5",
        ],
    ),
    (
        "solarized-light",
        "Solarized Light",
        Kind::Light,
        [
            "#fdf6e3", "#eee8d5", "#eee8d5", "#fdf6e3", "#ddd6c1", "#586e75", "#839496", "#268bd2",
            "#fdf6e3", "#eee8d5", "#859900", "#fdf6e3", "#dc322f", "#93a1a1", "#93a1a1", "#b58900",
            "#cb4b16", "#d33682", "#2aa198", "#eee8d5",
        ],
    ),
    (
        "one-light",
        "One Light",
        Kind::Light,
        [
            "#fafafa", "#f0f0f1", "#eaeaeb", "#ffffff", "#dbdbdc", "#383a42", "#696c77", "#4078f2",
            "#ffffff", "#e5e5e6", "#50a14f", "#ffffff", "#e45649", "#a0a1a7", "#a0a1a7", "#a626a4",
            "#e45649", "#986801", "#50a14f", "#f0f0f1",
        ],
    ),
    (
        "catppuccin-latte",
        "Catppuccin Latte",
        Kind::Light,
        [
            "#eff1f5", "#e6e9ef", "#e6e9ef", "#eff1f5", "#ccd0da", "#4c4f69", "#6c6f85", "#8839ef",
            "#eff1f5", "#ccd0da", "#40a02b", "#eff1f5", "#d20f39", "#9ca0b0", "#8c8fa1", "#8839ef",
            "#fe640b", "#179299", "#40a02b", "#e6e9ef",
        ],
    ),
    (
        "rose-pine-dawn",
        "Rosé Pine Dawn",
        Kind::Light,
        [
            "#faf4ed", "#f2e9e1", "#fffaf3", "#fffaf3", "#dfdad9", "#575279", "#797593", "#d7827e",
            "#faf4ed", "#dfdad9", "#286983", "#faf4ed", "#b4637a", "#9893a5", "#9893a5", "#907aa9",
            "#d7827e", "#ea9d34", "#286983", "#f2e9e1",
        ],
    ),
    (
        "gruvbox-light",
        "Gruvbox Light",
        Kind::Light,
        [
            "#fbf1c7", "#f2e5bc", "#ebdbb2", "#fbf1c7", "#d5c4a1", "#3c3836", "#7c6f64", "#076678",
            "#fbf1c7", "#ebdbb2", "#79740e", "#fbf1c7", "#9d0006", "#928374", "#928374", "#8f3f71",
            "#af3a03", "#b57614", "#427b58", "#ebdbb2",
        ],
    ),
    (
        "vitesse-light",
        "Vitesse Light",
        Kind::Light,
        [
            "#ffffff", "#f7f7f7", "#f7f7f7", "#ffffff", "#e7e7e7", "#393a34", "#7d7e78", "#1c6b48",
            "#ffffff", "#eeeeee", "#1c6b48", "#ffffff", "#ab5959", "#a0ada0", "#a0ada0", "#1e754f",
            "#b56959", "#2f798a", "#59873a", "#f7f7f7",
        ],
    ),
    (
        "github-dark",
        "GitHub Dark",
        Kind::Dark,
        [
            "#0d1117", "#010409", "#151b23", "#151b23", "#3d444d", "#f0f6fc", "#9198a1", "#4493f8",
            "#ffffff", "#1f2d3d", "#238636", "#ffffff", "#f85149", "#9198a1", "#9198a1", "#d2a8ff",
            "#ff7b72", "#79c0ff", "#a5d6ff", "#151b23",
        ],
    ),
    (
        "ayu-mirage",
        "Ayu Mirage",
        Kind::Dark,
        [
            "#242936", "#1f2430", "#1c212b", "#2a3040", "#363c4a", "#cccac2", "#8a919c", "#ffcc66",
            "#1f2430", "#34455a", "#87d96c", "#1f2430", "#ff6666", "#707a8c", "#707a8c", "#ffd173",
            "#ffad66", "#dfbfff", "#d5ff80", "#1c212b",
        ],
    ),
    (
        "ayu-dark",
        "Ayu Dark",
        Kind::Dark,
        [
            "#0d1017", "#0b0e14", "#131721", "#131721", "#2d323b", "#bfbdb6", "#6c7380", "#e6b450",
            "#0b0e14", "#273747", "#7fd962", "#0b0e14", "#d95757", "#6c7380", "#6c7380", "#ffb454",
            "#ff8f40", "#d2a6ff", "#aad94c", "#131721",
        ],
    ),
    (
        "dracula",
        "Dracula",
        Kind::Dark,
        [
            "#282a36", "#21222c", "#343746", "#343746", "#44475a", "#f8f8f2", "#a0a7c7", "#bd93f9",
            "#282a36", "#44475a", "#50fa7b", "#282a36", "#ff5555", "#6272a4", "#6272a4", "#ff79c6",
            "#8be9fd", "#bd93f9", "#f1fa8c", "#21222c",
        ],
    ),
    (
        "nord",
        "Nord",
        Kind::Dark,
        [
            "#2e3440", "#292e39", "#3b4252", "#3b4252", "#434c5e", "#d8dee9", "#9aa5b8", "#88c0d0",
            "#2e3440", "#434c5e", "#a3be8c", "#2e3440", "#bf616a", "#7b88a1", "#7b88a1", "#88c0d0",
            "#81a1c1", "#b48ead", "#a3be8c", "#3b4252",
        ],
    ),
    (
        "one-dark-pro",
        "One Dark Pro",
        Kind::Dark,
        [
            "#282c34", "#21252b", "#2c313a", "#2c313a", "#3e4451", "#abb2bf", "#7f848e", "#61afef",
            "#282c34", "#3e4451", "#98c379", "#282c34", "#e06c75", "#7f848e", "#7f848e", "#c678dd",
            "#e06c75", "#d19a66", "#98c379", "#21252b",
        ],
    ),
    (
        "tokyo-night",
        "Tokyo Night",
        Kind::Dark,
        [
            "#1a1b26", "#16161e", "#1f2335", "#1f2335", "#292e42", "#c0caf5", "#8f96bf", "#7aa2f7",
            "#1a1b26", "#283457", "#9ece6a", "#1a1b26", "#f7768e", "#565f89", "#737aa2", "#bb9af7",
            "#ff9e64", "#7dcfff", "#9ece6a", "#16161e",
        ],
    ),
    (
        "catppuccin-mocha",
        "Catppuccin Mocha",
        Kind::Dark,
        [
            "#1e1e2e", "#181825", "#313244", "#1e1e2e", "#45475a", "#cdd6f4", "#a6adc8", "#cba6f7",
            "#1e1e2e", "#313244", "#a6e3a1", "#1e1e2e", "#f38ba8", "#6c7086", "#7f849c", "#cba6f7",
            "#fab387", "#94e2d5", "#a6e3a1", "#181825",
        ],
    ),
];
