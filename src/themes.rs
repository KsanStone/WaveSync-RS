use catppuccin_egui::Theme;
use egui::Color32;

pub const DRACULA: Theme = Theme {
    rosewater: Color32::from_rgb(248, 248, 242),
    flamingo: Color32::from_rgb(255, 121, 198),
    pink: Color32::from_rgb(255, 121, 198),
    mauve: Color32::from_rgb(189, 147, 249),
    red: Color32::from_rgb(255, 85, 85),
    maroon: Color32::from_rgb(255, 110, 110),
    peach: Color32::from_rgb(255, 184, 108),
    yellow: Color32::from_rgb(241, 250, 140),
    green: Color32::from_rgb(80, 250, 123),
    teal: Color32::from_rgb(139, 233, 253),
    sky: Color32::from_rgb(164, 255, 255),
    sapphire: Color32::from_rgb(139, 233, 253),
    blue: Color32::from_rgb(189, 147, 249),
    lavender: Color32::from_rgb(214, 172, 255),
    text: Color32::from_rgb(248, 248, 242),
    subtext1: Color32::from_rgb(220, 220, 216),
    subtext0: Color32::from_rgb(190, 190, 188),
    overlay2: Color32::from_rgb(98, 114, 164),
    overlay1: Color32::from_rgb(82, 94, 130),
    overlay0: Color32::from_rgb(68, 71, 90),
    surface2: Color32::from_rgb(66, 68, 80),
    surface1: Color32::from_rgb(52, 55, 70),
    surface0: Color32::from_rgb(48, 50, 65),
    base: Color32::from_rgb(40, 42, 54),
    mantle: Color32::from_rgb(33, 34, 44),
    crust: Color32::from_rgb(25, 26, 33),
};

pub const NORD: Theme = Theme {
    rosewater: Color32::from_rgb(216, 222, 233),
    flamingo: Color32::from_rgb(191, 97, 106),
    pink: Color32::from_rgb(180, 142, 173),
    mauve: Color32::from_rgb(180, 142, 173),
    red: Color32::from_rgb(191, 97, 106),
    maroon: Color32::from_rgb(191, 97, 106),
    peach: Color32::from_rgb(208, 135, 112),
    yellow: Color32::from_rgb(235, 203, 139),
    green: Color32::from_rgb(163, 190, 140),
    teal: Color32::from_rgb(143, 188, 187),
    sky: Color32::from_rgb(136, 192, 208),
    sapphire: Color32::from_rgb(129, 161, 193),
    blue: Color32::from_rgb(136, 192, 208),
    lavender: Color32::from_rgb(180, 142, 173),
    text: Color32::from_rgb(236, 239, 244),
    subtext1: Color32::from_rgb(229, 233, 240),
    subtext0: Color32::from_rgb(216, 222, 233),
    overlay2: Color32::from_rgb(126, 140, 166),
    overlay1: Color32::from_rgb(94, 108, 135),
    overlay0: Color32::from_rgb(76, 86, 106),
    surface2: Color32::from_rgb(67, 76, 94),
    surface1: Color32::from_rgb(59, 66, 82),
    surface0: Color32::from_rgb(53, 60, 75),
    base: Color32::from_rgb(46, 52, 64),
    mantle: Color32::from_rgb(40, 46, 57),
    crust: Color32::from_rgb(35, 40, 50),
};

pub const TOKYO_NIGHT: Theme = Theme {
    rosewater: Color32::from_rgb(207, 201, 194),
    flamingo: Color32::from_rgb(247, 118, 142),
    pink: Color32::from_rgb(187, 154, 247),
    mauve: Color32::from_rgb(187, 154, 247),
    red: Color32::from_rgb(247, 118, 142),
    maroon: Color32::from_rgb(219, 75, 75),
    peach: Color32::from_rgb(255, 158, 100),
    yellow: Color32::from_rgb(224, 175, 104),
    green: Color32::from_rgb(158, 206, 106),
    teal: Color32::from_rgb(115, 218, 202),
    sky: Color32::from_rgb(125, 207, 255),
    sapphire: Color32::from_rgb(42, 195, 222),
    blue: Color32::from_rgb(122, 162, 247),
    lavender: Color32::from_rgb(187, 154, 247),
    text: Color32::from_rgb(192, 202, 245),
    subtext1: Color32::from_rgb(169, 177, 214),
    subtext0: Color32::from_rgb(154, 165, 206),
    overlay2: Color32::from_rgb(86, 95, 137),
    overlay1: Color32::from_rgb(65, 72, 104),
    overlay0: Color32::from_rgb(52, 59, 88),
    surface2: Color32::from_rgb(65, 72, 104),
    surface1: Color32::from_rgb(41, 46, 66),
    surface0: Color32::from_rgb(36, 40, 59),
    base: Color32::from_rgb(26, 27, 38),
    mantle: Color32::from_rgb(22, 22, 30),
    crust: Color32::from_rgb(18, 18, 24),
};

pub const GRUVBOX_DARK: Theme = Theme {
    rosewater: Color32::from_rgb(235, 219, 178),
    flamingo: Color32::from_rgb(251, 73, 52),
    pink: Color32::from_rgb(211, 134, 155),
    mauve: Color32::from_rgb(177, 98, 134),
    red: Color32::from_rgb(251, 73, 52),
    maroon: Color32::from_rgb(204, 36, 29),
    peach: Color32::from_rgb(254, 128, 25),
    yellow: Color32::from_rgb(250, 189, 47),
    green: Color32::from_rgb(184, 187, 38),
    teal: Color32::from_rgb(142, 192, 124),
    sky: Color32::from_rgb(131, 165, 152),
    sapphire: Color32::from_rgb(69, 133, 136),
    blue: Color32::from_rgb(131, 165, 152),
    lavender: Color32::from_rgb(211, 134, 155),
    text: Color32::from_rgb(235, 219, 178),
    subtext1: Color32::from_rgb(213, 196, 161),
    subtext0: Color32::from_rgb(189, 174, 147),
    overlay2: Color32::from_rgb(168, 153, 132),
    overlay1: Color32::from_rgb(146, 131, 116),
    overlay0: Color32::from_rgb(124, 111, 100),
    surface2: Color32::from_rgb(80, 73, 69),
    surface1: Color32::from_rgb(60, 56, 54),
    surface0: Color32::from_rgb(50, 48, 47),
    base: Color32::from_rgb(40, 40, 40),
    mantle: Color32::from_rgb(29, 32, 33),
    crust: Color32::from_rgb(24, 26, 27),
};

pub const GRAPHITE: Theme = Theme {
    rosewater: Color32::from_rgb(238, 238, 238),
    flamingo: Color32::from_rgb(218, 218, 218),
    pink: Color32::from_rgb(204, 204, 204),
    mauve: Color32::from_rgb(190, 190, 190),
    red: Color32::from_rgb(230, 230, 230),
    maroon: Color32::from_rgb(194, 194, 194),
    peach: Color32::from_rgb(210, 210, 210),
    yellow: Color32::from_rgb(224, 224, 224),
    green: Color32::from_rgb(200, 200, 200),
    teal: Color32::from_rgb(184, 184, 184),
    sky: Color32::from_rgb(214, 214, 214),
    sapphire: Color32::from_rgb(172, 172, 172),
    blue: Color32::from_rgb(224, 224, 224),
    lavender: Color32::from_rgb(198, 198, 198),
    text: Color32::from_rgb(232, 232, 232),
    subtext1: Color32::from_rgb(196, 196, 196),
    subtext0: Color32::from_rgb(164, 164, 164),
    overlay2: Color32::from_rgb(132, 132, 132),
    overlay1: Color32::from_rgb(108, 108, 108),
    overlay0: Color32::from_rgb(86, 86, 86),
    surface2: Color32::from_rgb(66, 66, 66),
    surface1: Color32::from_rgb(52, 52, 52),
    surface0: Color32::from_rgb(40, 40, 40),
    base: Color32::from_rgb(26, 26, 26),
    mantle: Color32::from_rgb(20, 20, 20),
    crust: Color32::from_rgb(14, 14, 14),
};

pub const THEMES: [(&str, Theme); 9] = [
    ("Mocha", catppuccin_egui::MOCHA),
    ("Frappe", catppuccin_egui::FRAPPE),
    ("Macchiato", catppuccin_egui::MACCHIATO),
    ("Latte", catppuccin_egui::LATTE),
    ("Dracula", DRACULA),
    ("Nord", NORD),
    ("Tokyo Night", TOKYO_NIGHT),
    ("Gruvbox Dark", GRUVBOX_DARK),
    ("Graphite", GRAPHITE),
];

pub fn theme_name(theme: Theme) -> &'static str {
    THEMES
        .iter()
        .find_map(|(name, candidate)| (*candidate == theme).then_some(*name))
        .unwrap_or("Mocha")
}

pub fn theme_from_name(name: &str) -> Theme {
    // Keep compatibility with the misspelling used by older saved settings.
    if name == "Machiato" {
        return catppuccin_egui::MACCHIATO;
    }

    THEMES
        .iter()
        .find_map(|(candidate_name, theme)| (*candidate_name == name).then_some(*theme))
        .unwrap_or(catppuccin_egui::MOCHA)
}

#[cfg(test)]
mod tests {
    use super::{THEMES, theme_from_name, theme_name};

    #[test]
    fn every_theme_round_trips_through_its_name() {
        for (name, theme) in THEMES {
            assert_eq!(theme_name(theme), name);
            assert_eq!(theme_from_name(name), theme);
        }
    }

    #[test]
    fn old_macchiato_spelling_still_loads() {
        assert_eq!(theme_from_name("Machiato"), catppuccin_egui::MACCHIATO);
    }
}
