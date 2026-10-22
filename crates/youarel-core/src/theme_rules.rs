//! The colors of the window: which hexadecimal colors a theme may write, and the full palette a theme's few declared colors resolve to.
//!
//! A theme from a file names some colors and leaves the rest, so every slot the window paints falls back to another slot or to the default of the theme's kind, and the fallbacks reach a default within [`SLOTS`] steps.

/// The mark that starts a hexadecimal color.
pub const HASH: u8 = b'#';
/// The digits of a color written short, `#rgb`, and with its alpha, `#rgba`.
pub const SHORT: usize = 3;
pub const SHORT_ALPHA: usize = 4;
/// The digits of a color written long, `#rrggbb`, and with its alpha, `#rrggbbaa`.
pub const LONG: usize = 6;
pub const LONG_ALPHA: usize = 8;
/// The alpha of an opaque color.
pub const OPAQUE: u8 = u8::MAX;
/// The digits of one channel in a short color and in a long one.
const SHORT_WIDTH: usize = 1;
const LONG_WIDTH: usize = 2;
/// The bits one hexadecimal digit holds.
const NIBBLE: u32 = 4;
/// The bits of a byte below its high digit.
const LOW_NIBBLE: u8 = 0x0f;
/// The value of the high one of two hexadecimal digits for each unit it counts.
const DIGIT_PLACE: u8 = 0x10;
/// What one hexadecimal digit written once stands for, as in `#abc`: the digit at both places.
const DOUBLED_DIGIT: u8 = 0x11;
/// The lowercase hexadecimal digits, each at the place of its value.
const DIGITS: &[u8; 16] = b"0123456789abcdef";
/// The value of the first letter digit.
const TEN: u8 = 10;
/// How many slots a palette holds.
pub const SLOTS: usize = 20;
/// The weights of red, green, and blue in a color's brightness, out of [`BRIGHTNESS_SCALE`], as ITU-R BT.601 weighs them.
const BRIGHTNESS_WEIGHTS: [u32; 3] = [299, 587, 114];
const BRIGHTNESS_SCALE: u32 = 1000;
/// The brightness below which a background is dark.
const DARK_BELOW: u32 = 128;

/// A color with its alpha, eight bits a channel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

/// The value of one hexadecimal digit, in either case.
const fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(byte.wrapping_sub(b'a').wrapping_add(TEN)),
        b'A'..=b'F' => Some(byte.wrapping_sub(b'A').wrapping_add(TEN)),
        _ => None,
    }
}

/// The channel two digits write, or one digit doubled.
fn channel(digits: &[u8]) -> Option<u8> {
    match digits {
        // The digits add up rather than combine their bits, so every operator here changes some channel.
        [single] => digit(*single).map(|value| value.wrapping_mul(DOUBLED_DIGIT)),
        [high, low] => Some(
            digit(*high)?
                .wrapping_mul(DIGIT_PLACE)
                .wrapping_add(digit(*low)?),
        ),
        _ => None,
    }
}

impl Color {
    /// The color `#rgb`, `#rgba`, `#rrggbb`, or `#rrggbbaa` writes, in either case, or `None` for any other text.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        Self::from_bytes(text.as_bytes())
    }

    /// The color the bytes of a text write, as [`Color::from_hex`] reads them.
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (&hash, digits) = bytes.split_first()?;
        if hash != HASH {
            return None;
        }
        let width = match digits.len() {
            SHORT | SHORT_ALPHA => SHORT_WIDTH,
            LONG | LONG_ALPHA => LONG_WIDTH,
            _ => return None,
        };
        let mut channels = digits.chunks(width).map(channel);
        let red = channels.next()??;
        let green = channels.next()??;
        let blue = channels.next()??;
        let alpha = channels.next().unwrap_or(Some(OPAQUE))?;
        Some(Self {
            red,
            green,
            blue,
            alpha,
        })
    }

    /// The color as `#rrggbbaa` in lowercase digits, which [`Color::from_hex`] reads back.
    #[must_use]
    pub fn hex(self) -> Hex {
        let digit = |value: u8| {
            DIGITS
                .get(usize::from(value & LOW_NIBBLE))
                .copied()
                .unwrap_or(HASH)
        };
        let high = |value: u8| digit(value.wrapping_shr(NIBBLE));
        let Self {
            red,
            green,
            blue,
            alpha,
        } = self;
        Hex([
            HASH,
            high(red),
            digit(red),
            high(green),
            digit(green),
            high(blue),
            digit(blue),
            high(alpha),
            digit(alpha),
        ])
    }

    /// The color's brightness out of 255, by the weights of its channels.
    #[must_use]
    pub fn brightness(self) -> u32 {
        let [red, green, blue] = BRIGHTNESS_WEIGHTS;
        let weighted = u32::from(self.red)
            .saturating_mul(red)
            .saturating_add(u32::from(self.green).saturating_mul(green))
            .saturating_add(u32::from(self.blue).saturating_mul(blue));
        weighted.checked_div(BRIGHTNESS_SCALE).unwrap_or_default()
    }
}

/// A color written as `#rrggbbaa`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Hex([u8; LONG_ALPHA + 1]);

impl Hex {
    /// The text's bytes, the mark and eight lowercase hexadecimal digits.
    #[must_use]
    pub const fn bytes(&self) -> &[u8; LONG_ALPHA + 1] {
        &self.0
    }
}

impl core::fmt::Display for Hex {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0
            .iter()
            .try_for_each(|byte| core::fmt::Write::write_char(formatter, char::from(*byte)))
    }
}

/// Whether a theme paints dark text on light ground or light text on dark ground.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Kind {
    #[default]
    Light,
    Dark,
}

/// The kind a background implies, for a theme that names none.
#[must_use]
pub fn kind_of(background: Color) -> Kind {
    if background.brightness() < DARK_BELOW {
        Kind::Dark
    } else {
        Kind::Light
    }
}

/// Every part of the window a theme colors.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Slot {
    Background,
    Sidebar,
    Surface,
    Raised,
    Border,
    Text,
    Muted,
    Accent,
    AccentText,
    Selection,
    Go,
    GoText,
    Danger,
    UrlScheme,
    UrlSub,
    UrlName,
    UrlSuffix,
    UrlPort,
    UrlRest,
    Thumbnail,
}

/// Every slot, in the order a palette holds them.
pub const ALL: [Slot; SLOTS] = [
    Slot::Background,
    Slot::Sidebar,
    Slot::Surface,
    Slot::Raised,
    Slot::Border,
    Slot::Text,
    Slot::Muted,
    Slot::Accent,
    Slot::AccentText,
    Slot::Selection,
    Slot::Go,
    Slot::GoText,
    Slot::Danger,
    Slot::UrlScheme,
    Slot::UrlSub,
    Slot::UrlName,
    Slot::UrlSuffix,
    Slot::UrlPort,
    Slot::UrlRest,
    Slot::Thumbnail,
];

impl Slot {
    /// The place of the slot in a palette.
    #[must_use]
    pub fn index(self) -> usize {
        ALL.iter()
            .position(|slot| *slot == self)
            .unwrap_or_default()
    }

    /// The slot a missing color takes its color from, or `None` for a slot that takes the default of its kind.
    #[must_use]
    pub const fn fallback(self) -> Option<Self> {
        match self {
            Self::Background
            | Self::Border
            | Self::Text
            | Self::Accent
            | Self::AccentText
            | Self::Danger => None,
            Self::Sidebar | Self::Raised => Some(Self::Background),
            Self::Surface => Some(Self::Sidebar),
            Self::Thumbnail => Some(Self::Surface),
            Self::Muted | Self::UrlRest => Some(Self::Text),
            Self::Selection | Self::Go | Self::UrlName | Self::UrlPort => Some(Self::Accent),
            Self::GoText => Some(Self::AccentText),
            Self::UrlSuffix => Some(Self::Danger),
            Self::UrlScheme | Self::UrlSub => Some(Self::Muted),
        }
    }
}

/// A color for each slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Palette([Color; SLOTS]);

impl Palette {
    /// The color of a slot.
    #[must_use]
    pub fn color(&self, slot: Slot) -> Color {
        self.0.get(slot.index()).copied().unwrap_or_default()
    }

    /// Every slot with its color, in order.
    pub fn colors(&self) -> impl Iterator<Item = (Slot, Color)> + '_ {
        ALL.into_iter().zip(self.0)
    }
}

/// An opaque color.
const fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color {
        red,
        green,
        blue,
        alpha: OPAQUE,
    }
}

/// The defaults of each kind, GitHub's light and dark palettes.
const WHITE: Color = rgb(0xff, 0xff, 0xff);
const DARK_BACKGROUND: Color = rgb(0x0d, 0x11, 0x17);
const LIGHT_BORDER: Color = rgb(0xd1, 0xd9, 0xe0);
const DARK_BORDER: Color = rgb(0x3d, 0x44, 0x4d);
const LIGHT_TEXT: Color = rgb(0x1f, 0x23, 0x28);
const DARK_TEXT: Color = rgb(0xf0, 0xf6, 0xfc);
const LIGHT_ACCENT: Color = rgb(0x09, 0x69, 0xda);
const DARK_ACCENT: Color = rgb(0x44, 0x93, 0xf8);
const LIGHT_DANGER: Color = rgb(0xcf, 0x22, 0x2e);
const DARK_DANGER: Color = rgb(0xf8, 0x51, 0x49);

/// The colors of a theme's kind for the slots that fall back to nothing.
#[must_use]
pub const fn default_color(kind: Kind, slot: Slot) -> Color {
    const fn pick(kind: Kind, light: Color, dark: Color) -> Color {
        match kind {
            Kind::Light => light,
            Kind::Dark => dark,
        }
    }
    match slot {
        Slot::Background => pick(kind, WHITE, DARK_BACKGROUND),
        Slot::Border => pick(kind, LIGHT_BORDER, DARK_BORDER),
        Slot::Text => pick(kind, LIGHT_TEXT, DARK_TEXT),
        Slot::Accent => pick(kind, LIGHT_ACCENT, DARK_ACCENT),
        Slot::Danger => pick(kind, LIGHT_DANGER, DARK_DANGER),
        // A slot with a fallback never reaches its own default, so these only fill a slot whose chain ends at no declared color.
        Slot::AccentText
        | Slot::Sidebar
        | Slot::Surface
        | Slot::Raised
        | Slot::Muted
        | Slot::Selection
        | Slot::Go
        | Slot::GoText
        | Slot::UrlScheme
        | Slot::UrlSub
        | Slot::UrlName
        | Slot::UrlSuffix
        | Slot::UrlPort
        | Slot::UrlRest
        | Slot::Thumbnail => WHITE,
    }
}

/// The palette of a theme of `kind` that declares the colors of `declared`, each missing one taken from its fallback, and a slot without a fallback from the kind's default.
#[must_use]
pub fn resolve(kind: Kind, declared: &[Option<Color>; SLOTS]) -> Palette {
    let mut colors = [Color::default(); SLOTS];
    for (place, slot) in ALL.into_iter().enumerate() {
        let mut current = slot;
        let mut found = None;
        // The fallbacks form chains shorter than the slots, so the walk meets a declared color or a slot without a fallback.
        for _ in 0..SLOTS {
            if let Some(color) = declared.get(current.index()).copied().flatten() {
                found = Some(color);
                break;
            }
            match current.fallback() {
                Some(next) => current = next,
                None => break,
            }
        }
        if let Some(target) = colors.get_mut(place) {
            *target = found.unwrap_or_else(|| default_color(kind, current));
        }
    }
    Palette(colors)
}

/// The properties Kani proves over every color, every short text, and every set of declared colors.
#[cfg(any(test, kani))]
mod proofs {
    use super::{ALL, Color, Kind, SLOTS, Slot, default_color, resolve};

    /// The longest text the parse harness draws: the mark and eight digits.
    #[cfg(kani)]
    const TEXT_BOUND: usize = 9;

    /// The color a slot resolves to, restated by following the fallbacks one step at a time.
    pub(super) fn expected(
        kind: Kind,
        declared: &[Option<Color>; SLOTS],
        slot: Slot,
        depth: usize,
    ) -> Option<Color> {
        if let Some(color) = declared.get(slot.index()).copied().flatten() {
            return Some(color);
        }
        match (slot.fallback(), depth.checked_sub(1)) {
            (Some(next), Some(deeper)) => expected(kind, declared, next, deeper),
            (Some(_), None) => None,
            (None, _) => Some(default_color(kind, slot)),
        }
    }

    /// Whether every slot reaches a slot without a fallback within the slots' count.
    pub(super) fn acyclic() -> bool {
        ALL.into_iter().all(|start| {
            let mut current = start;
            for _ in 0..SLOTS {
                match current.fallback() {
                    Some(next) => current = next,
                    None => return true,
                }
            }
            false
        })
    }

    /// What a resolution promises: each slot holds its declared color, or the color its fallbacks lead to.
    pub(super) fn resolves(kind: Kind, declared: &[Option<Color>; SLOTS]) -> bool {
        let palette = resolve(kind, declared);
        ALL.into_iter()
            .all(|slot| expected(kind, declared, slot, SLOTS) == Some(palette.color(slot)))
    }

    /// Every color written long reads back as itself.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(10)]
    fn every_color_reads_back_from_its_hexadecimal() {
        let color = Color {
            red: kani::any(),
            green: kani::any(),
            blue: kani::any(),
            alpha: kani::any(),
        };
        kani::assert(
            Color::from_bytes(color.hex().bytes()) == Some(color),
            "a color reads back",
        );
        kani::cover!(color.alpha == u8::MAX, "an opaque color");
    }

    /// Every text of up to nine bytes, each any byte.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(10)]
    fn only_hexadecimal_colors_of_four_lengths_are_read() {
        let bytes: [u8; TEXT_BOUND] = kani::any();
        let length: usize = kani::any();
        kani::assume(length <= TEXT_BOUND);
        let Some(prefix) = bytes.get(..length) else {
            return;
        };
        let read = Color::from_bytes(prefix);
        let well_formed = matches!(length, 4 | 5 | 7 | 9)
            && prefix.first() == Some(&b'#')
            && prefix.iter().skip(1).all(u8::is_ascii_hexdigit);
        kani::assert(
            read.is_some() == well_formed,
            "a color is read exactly from its forms",
        );
        if let (Some(color), 9) = (read, length) {
            kani::assert(
                color.hex().bytes().eq_ignore_ascii_case(prefix),
                "a long color reads as written",
            );
        }
        kani::cover!(read.is_some() && length == 4, "a short color");
        kani::cover!(read.is_some() && length == 9, "a long color with its alpha");
        kani::cover!(
            read.is_none() && length == 7,
            "a long text that is no color"
        );
    }

    /// Every set of declared colors over two colors and both kinds.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(22)]
    fn every_slot_resolves_to_its_declared_color_or_its_fallback() {
        kani::assert(acyclic(), "the fallbacks reach a default");
        let kind = if kani::any() { Kind::Dark } else { Kind::Light };
        let first = Color {
            red: kani::any(),
            green: 0,
            blue: 0,
            alpha: u8::MAX,
        };
        let second = Color {
            red: 0,
            green: kani::any(),
            blue: 0,
            alpha: u8::MAX,
        };
        let mut declared = [None; SLOTS];
        for slot in &mut declared {
            *slot = match kani::any::<u8>() % 3 {
                0 => None,
                1 => Some(first),
                _ => Some(second),
            };
        }
        kani::assert(
            resolves(kind, &declared),
            "each slot resolves to its declared color or its fallback",
        );
        kani::cover!(
            declared.iter().all(Option::is_none),
            "a theme that declares nothing"
        );
    }

    /// `#12` writes no color; this probe claims it reads one and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(10)]
    fn rejects_a_color_of_two_digits() {
        kani::assert(Color::from_hex("#12").is_some(), "NEGATIVE_THEME_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{acyclic, resolves};
    use super::{ALL, Color, Kind, OPAQUE, SLOTS, Slot, default_color, kind_of, resolve};

    const fn color(red: u8, green: u8, blue: u8, alpha: u8) -> Color {
        Color {
            red,
            green,
            blue,
            alpha,
        }
    }

    #[test]
    fn colors_read_in_every_form_and_case() {
        assert_eq!(
            Color::from_hex("#0969da"),
            Some(color(0x09, 0x69, 0xda, OPAQUE))
        );
        assert_eq!(
            Color::from_hex("#0969DA80"),
            Some(color(0x09, 0x69, 0xda, 0x80))
        );
        assert_eq!(
            Color::from_hex("#fA0"),
            Some(color(0xff, 0xaa, 0x00, OPAQUE))
        );
        assert_eq!(
            Color::from_hex("#fa08"),
            Some(color(0xff, 0xaa, 0x00, 0x88))
        );
        for refused in [
            "",
            "#",
            "#12",
            "#12345",
            "#1234567",
            "#123456789",
            "0969da",
            "#0969dg",
            "#０９６",
        ] {
            assert_eq!(Color::from_hex(refused), None, "{refused}");
        }
        assert_eq!(
            alloc::format!("{}", color(0x09, 0x69, 0xda, 0x80).hex()),
            "#0969da80"
        );
        assert_eq!(color(0x09, 0x69, 0xda, 0x80).hex().bytes(), b"#0969da80");
    }

    #[test]
    fn a_background_tells_its_kind() {
        assert_eq!(kind_of(color(0xff, 0xff, 0xff, OPAQUE)), Kind::Light);
        assert_eq!(kind_of(color(0x28, 0x2a, 0x36, OPAQUE)), Kind::Dark);
        assert_eq!(kind_of(color(0x80, 0x80, 0x80, OPAQUE)), Kind::Light);
        assert_eq!(kind_of(color(0x7f, 0x7f, 0x7f, OPAQUE)), Kind::Dark);
        assert_eq!(color(0xff, 0xff, 0xff, OPAQUE).brightness(), 255);
    }

    #[test]
    fn missing_colors_follow_their_fallbacks_to_the_kind_s_defaults() {
        assert!(acyclic());
        let mut declared = [None; SLOTS];
        let accent = color(0xbd, 0x93, 0xf9, OPAQUE);
        let background = color(0x28, 0x2a, 0x36, OPAQUE);
        declared[Slot::Accent.index()] = Some(accent);
        declared[Slot::Background.index()] = Some(background);
        let palette = resolve(Kind::Dark, &declared);
        assert_eq!(palette.color(Slot::UrlName), accent);
        assert_eq!(palette.color(Slot::Selection), accent);
        assert_eq!(palette.color(Slot::Thumbnail), background);
        assert_eq!(
            palette.color(Slot::Text),
            default_color(Kind::Dark, Slot::Text)
        );
        assert_eq!(
            palette.color(Slot::UrlScheme),
            default_color(Kind::Dark, Slot::Text)
        );
        assert_eq!(palette.colors().count(), SLOTS);
        assert!(resolves(Kind::Dark, &declared));
        assert!(resolves(Kind::Light, &[None; SLOTS]));
        for slot in ALL {
            assert_eq!(ALL[slot.index()], slot);
        }
        assert_ne!(
            default_color(Kind::Light, Slot::Background),
            default_color(Kind::Dark, Slot::Background)
        );
    }
}
