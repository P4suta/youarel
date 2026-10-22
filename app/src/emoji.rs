//! The emoji the mark picker offers, each under the name its shortcode writes, such as `books` for `:books:`, in the category the picker shows it in.
//!
//! The names follow the shortcodes chat and code hosts share, so a person who types `:rocket:` elsewhere types it here too.
//! An emoji that needs a variation selector writes it escaped, since the repository keeps invisible characters out of its source.

use serde::Serialize;

/// An emoji and its name, as the picker shows them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmojiChoice {
    pub emoji: String,
    pub name: String,
}

/// A heading of the picker with the emoji under it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmojiGroup {
    pub label: String,
    pub choices: Vec<EmojiChoice>,
}

/// The mark that opens and closes a shortcode.
const COLON: char = ':';
/// The most emoji a search shows.
const MOST_FOUND: usize = 64;
/// The heading of what a search finds.
const FOUND: &str = "Found";

const OBJECTS: &[(&str, &str)] = &[
    ("📚", "books"),
    ("📖", "book"),
    ("📕", "closed_book"),
    ("📗", "green_book"),
    ("📘", "blue_book"),
    ("📙", "orange_book"),
    ("📓", "notebook"),
    ("📒", "ledger"),
    ("📝", "memo"),
    ("📄", "page_facing_up"),
    ("📃", "page_with_curl"),
    ("📑", "bookmark_tabs"),
    ("🔖", "bookmark"),
    ("📁", "file_folder"),
    ("📂", "open_file_folder"),
    ("📋", "clipboard"),
    ("📌", "pushpin"),
    ("📍", "round_pushpin"),
    ("📎", "paperclip"),
    ("🔍", "mag"),
    ("🔎", "mag_right"),
    ("🔒", "lock"),
    ("🔓", "unlock"),
    ("🔑", "key"),
    ("🔨", "hammer"),
    ("🔧", "wrench"),
    ("🔩", "nut_and_bolt"),
    ("\u{2699}\u{fe0f}", "gear"),
    ("💡", "bulb"),
    ("🔦", "flashlight"),
    ("🧰", "toolbox"),
    ("🧲", "magnet"),
    ("🧪", "test_tube"),
    ("🔬", "microscope"),
    ("🔭", "telescope"),
    ("💻", "computer"),
    ("📱", "iphone"),
    ("📞", "telephone_receiver"),
    ("📷", "camera"),
    ("📸", "camera_flash"),
    ("🎥", "movie_camera"),
    ("📺", "tv"),
    ("📻", "radio"),
    ("🎧", "headphones"),
    ("💾", "floppy_disk"),
    ("💿", "cd"),
    ("📀", "dvd"),
    ("📦", "package"),
    ("📫", "mailbox"),
    ("📮", "postbox"),
    ("📧", "email"),
    ("📨", "incoming_envelope"),
    ("📤", "outbox_tray"),
    ("📥", "inbox_tray"),
    ("📅", "date"),
    ("📆", "calendar"),
    ("📈", "chart_with_upwards_trend"),
    ("📉", "chart_with_downwards_trend"),
    ("📊", "bar_chart"),
    ("💰", "moneybag"),
    ("💳", "credit_card"),
    ("💵", "dollar"),
    ("💴", "yen"),
    ("🧾", "receipt"),
    ("🛒", "shopping_cart"),
    ("🎁", "gift"),
    ("🔔", "bell"),
    ("📣", "mega"),
    ("📢", "loudspeaker"),
    ("🧭", "compass"),
    ("⏰", "alarm_clock"),
    ("⌛", "hourglass"),
    ("⏳", "hourglass_flowing_sand"),
    ("🔋", "battery"),
    ("🔌", "electric_plug"),
    ("🧮", "abacus"),
    ("🧩", "jigsaw"),
    ("🎨", "art"),
    ("🧵", "thread"),
    ("🧶", "yarn"),
];

const SYMBOLS: &[(&str, &str)] = &[
    ("🚀", "rocket"),
    ("🐛", "bug"),
    ("🔥", "fire"),
    ("⚡", "zap"),
    ("✨", "sparkles"),
    ("⭐", "star"),
    ("🌟", "star2"),
    ("💎", "gem"),
    ("🎯", "dart"),
    ("🏆", "trophy"),
    ("🥇", "1st_place_medal"),
    ("🏅", "medal_sports"),
    ("🎓", "mortar_board"),
    ("🧠", "brain"),
    ("💬", "speech_balloon"),
    ("💭", "thought_balloon"),
    ("\u{2764}\u{fe0f}", "heart"),
    ("🧡", "orange_heart"),
    ("💛", "yellow_heart"),
    ("💚", "green_heart"),
    ("💙", "blue_heart"),
    ("💜", "purple_heart"),
    ("🖤", "black_heart"),
    ("🤍", "white_heart"),
    ("💯", "100"),
    ("✅", "white_check_mark"),
    ("❌", "x"),
    ("❗", "exclamation"),
    ("❓", "question"),
    ("\u{26a0}\u{fe0f}", "warning"),
    ("🚧", "construction"),
    ("🔴", "red_circle"),
    ("🟠", "orange_circle"),
    ("🟡", "yellow_circle"),
    ("🟢", "green_circle"),
    ("🔵", "large_blue_circle"),
    ("🟣", "purple_circle"),
    ("⚫", "black_circle"),
    ("⚪", "white_circle"),
    ("🟥", "red_square"),
    ("🟧", "orange_square"),
    ("🟨", "yellow_square"),
    ("🟩", "green_square"),
    ("🟦", "blue_square"),
    ("🟪", "purple_square"),
    ("⬛", "black_large_square"),
    ("⬜", "white_large_square"),
    ("🔶", "large_orange_diamond"),
    ("🔷", "large_blue_diamond"),
    ("🔺", "small_red_triangle"),
    ("\u{267b}\u{fe0f}", "recycle"),
    ("🆕", "new"),
    ("🆗", "ok"),
    ("🆒", "cool"),
    ("🆓", "free"),
    ("🔗", "link"),
    ("🌐", "globe_with_meridians"),
    ("🏁", "checkered_flag"),
    ("🚩", "triangular_flag_on_post"),
    ("🔰", "beginner"),
    ("💤", "zzz"),
    ("🎉", "tada"),
    ("🎊", "confetti_ball"),
    ("🎈", "balloon"),
];

const NATURE: &[(&str, &str)] = &[
    ("🌱", "seedling"),
    ("🌿", "herb"),
    ("🍀", "four_leaf_clover"),
    ("🌳", "deciduous_tree"),
    ("🌲", "evergreen_tree"),
    ("🌵", "cactus"),
    ("🌸", "cherry_blossom"),
    ("🌻", "sunflower"),
    ("🌹", "rose"),
    ("🌷", "tulip"),
    ("🍁", "maple_leaf"),
    ("🍂", "fallen_leaf"),
    ("🌊", "ocean"),
    ("🌈", "rainbow"),
    ("\u{2600}\u{fe0f}", "sunny"),
    ("🌙", "crescent_moon"),
    ("🌍", "earth_africa"),
    ("🌎", "earth_americas"),
    ("🌏", "earth_asia"),
    ("🌋", "volcano"),
    ("\u{2744}\u{fe0f}", "snowflake"),
    ("\u{2601}\u{fe0f}", "cloud"),
    ("🐶", "dog"),
    ("🐱", "cat"),
    ("🐭", "mouse"),
    ("🐹", "hamster"),
    ("🐰", "rabbit"),
    ("🦊", "fox_face"),
    ("🐻", "bear"),
    ("🐼", "panda_face"),
    ("🐨", "koala"),
    ("🐯", "tiger"),
    ("🦁", "lion"),
    ("🐮", "cow"),
    ("🐷", "pig"),
    ("🐸", "frog"),
    ("🐵", "monkey_face"),
    ("🐔", "chicken"),
    ("🐧", "penguin"),
    ("🐦", "bird"),
    ("🦉", "owl"),
    ("🦄", "unicorn"),
    ("🐝", "bee"),
    ("🦋", "butterfly"),
    ("🐢", "turtle"),
    ("🐍", "snake"),
    ("🐙", "octopus"),
    ("🐳", "whale"),
    ("🐬", "dolphin"),
    ("🐟", "fish"),
    ("🦀", "crab"),
    ("🐘", "elephant"),
    ("🦒", "giraffe"),
    ("🦔", "hedgehog"),
    ("🐾", "feet"),
];

const FOOD: &[(&str, &str)] = &[
    ("🍎", "apple"),
    ("🍊", "tangerine"),
    ("🍋", "lemon"),
    ("🍌", "banana"),
    ("🍉", "watermelon"),
    ("🍇", "grapes"),
    ("🍓", "strawberry"),
    ("🍑", "peach"),
    ("🍒", "cherries"),
    ("🥑", "avocado"),
    ("🍅", "tomato"),
    ("🥕", "carrot"),
    ("🌽", "corn"),
    ("🍞", "bread"),
    ("🧀", "cheese"),
    ("🍳", "fried_egg"),
    ("🍔", "hamburger"),
    ("🍕", "pizza"),
    ("🌮", "taco"),
    ("🍣", "sushi"),
    ("🍙", "rice_ball"),
    ("🍜", "ramen"),
    ("🍱", "bento"),
    ("🍰", "cake"),
    ("🍩", "doughnut"),
    ("🍪", "cookie"),
    ("🍫", "chocolate_bar"),
    ("🍦", "icecream"),
    ("☕", "coffee"),
    ("🍵", "tea"),
    ("🍺", "beer"),
    ("🍷", "wine_glass"),
    ("🥤", "cup_with_straw"),
];

const PLACES: &[(&str, &str)] = &[
    ("🏠", "house"),
    ("🏡", "house_with_garden"),
    ("🏢", "office"),
    ("🏫", "school"),
    ("🏥", "hospital"),
    ("🏦", "bank"),
    ("🏪", "convenience_store"),
    ("🏬", "department_store"),
    ("🏭", "factory"),
    ("🏰", "european_castle"),
    ("🗼", "tokyo_tower"),
    ("🗽", "statue_of_liberty"),
    ("🗻", "mount_fuji"),
    ("🚗", "car"),
    ("🚕", "taxi"),
    ("🚌", "bus"),
    ("🚃", "railway_car"),
    ("🚄", "bullettrain_side"),
    ("\u{2708}\u{fe0f}", "airplane"),
    ("🚲", "bike"),
    ("🚢", "ship"),
    ("⛵", "boat"),
    ("🧳", "luggage"),
    ("🎡", "ferris_wheel"),
    ("⛺", "tent"),
];

const ACTIVITIES: &[(&str, &str)] = &[
    ("⚽", "soccer"),
    ("🏀", "basketball"),
    ("🏈", "football"),
    ("⚾", "baseball"),
    ("🎾", "tennis"),
    ("🏐", "volleyball"),
    ("🏓", "ping_pong"),
    ("🏸", "badminton"),
    ("🥊", "boxing_glove"),
    ("🎮", "video_game"),
    ("🎲", "game_die"),
    ("🎸", "guitar"),
    ("🎹", "musical_keyboard"),
    ("🎺", "trumpet"),
    ("🎻", "violin"),
    ("🥁", "drum"),
    ("🎵", "musical_note"),
    ("🎶", "notes"),
    ("🎬", "clapper"),
    ("🎭", "performing_arts"),
    ("🎤", "microphone"),
    ("🏃", "runner"),
    ("🚴", "bicyclist"),
    ("🏊", "swimmer"),
    ("🧘", "lotus_position"),
    ("🎿", "ski"),
];

const PEOPLE: &[(&str, &str)] = &[
    ("😀", "grinning"),
    ("😃", "smiley"),
    ("😄", "smile"),
    ("😁", "grin"),
    ("😆", "laughing"),
    ("😅", "sweat_smile"),
    ("😂", "joy"),
    ("🙂", "slightly_smiling_face"),
    ("😉", "wink"),
    ("😊", "blush"),
    ("😍", "heart_eyes"),
    ("🤩", "star_struck"),
    ("😎", "sunglasses"),
    ("🤓", "nerd_face"),
    ("🧐", "monocle_face"),
    ("🤔", "thinking"),
    ("😴", "sleeping"),
    ("😇", "innocent"),
    ("🥳", "partying_face"),
    ("😭", "sob"),
    ("😱", "scream"),
    ("😡", "rage"),
    ("🤯", "exploding_head"),
    ("🥺", "pleading_face"),
    ("👍", "thumbsup"),
    ("👎", "thumbsdown"),
    ("👏", "clap"),
    ("🙌", "raised_hands"),
    ("🙏", "pray"),
    ("💪", "muscle"),
    ("👀", "eyes"),
    ("👋", "wave"),
    ("🤝", "handshake"),
    ("👶", "baby"),
    ("👨", "man"),
    ("👩", "woman"),
    ("🧒", "child"),
    ("👪", "family"),
    ("🤖", "robot"),
    ("👻", "ghost"),
    ("💀", "skull"),
    ("👽", "alien"),
    ("🎃", "jack_o_lantern"),
    ("🎅", "santa"),
];

const FLAGS: &[(&str, &str)] = &[
    ("🇯🇵", "jp"),
    ("🇺🇸", "us"),
    ("🇬🇧", "gb"),
    ("🇫🇷", "fr"),
    ("🇩🇪", "de"),
    ("🇨🇳", "cn"),
    ("🇰🇷", "kr"),
    ("🇮🇹", "it"),
    ("🇪🇸", "es"),
    ("🇨🇦", "ca"),
    ("🇦🇺", "au"),
    ("🇮🇳", "in"),
    ("🇧🇷", "br"),
    ("🏴", "black_flag"),
];

/// The picker's categories in the order it shows them.
const CATEGORIES: [(&str, &[(&str, &str)]); 8] = [
    ("Objects", OBJECTS),
    ("Symbols", SYMBOLS),
    ("Nature", NATURE),
    ("Food", FOOD),
    ("Places", PLACES),
    ("Activities", ACTIVITIES),
    ("People", PEOPLE),
    ("Flags", FLAGS),
];

fn choice((emoji, name): (&str, &str)) -> EmojiChoice {
    EmojiChoice {
        emoji: emoji.to_owned(),
        name: name.to_owned(),
    }
}

/// Every emoji with its name, in the picker's order.
fn every() -> impl Iterator<Item = (&'static str, &'static str)> {
    CATEGORIES
        .iter()
        .flat_map(|(_, emoji)| emoji.iter().copied())
}

/// A search as the names write it: without its colons, in lowercase, with spaces and hyphens as underscores.
fn normalized(query: &str) -> String {
    query
        .trim()
        .trim_matches(COLON)
        .to_lowercase()
        .replace([' ', '-'], "_")
}

/// What the picker shows: every category for an empty search, or the emoji whose names hold the search, those that start with it first.
#[must_use]
pub fn pick(query: &str) -> Vec<EmojiGroup> {
    let wanted = normalized(query);
    if wanted.is_empty() {
        return CATEGORIES
            .iter()
            .map(|(label, emoji)| EmojiGroup {
                label: (*label).to_owned(),
                choices: emoji.iter().copied().map(choice).collect(),
            })
            .collect();
    }
    let mut found: Vec<(&str, &str)> = every()
        .filter(|(_, name)| name.contains(wanted.as_str()))
        .collect();
    // A name that starts with the search ranks first, then a shorter name, as a closer match.
    found.sort_by_key(|(_, name)| (!name.starts_with(wanted.as_str()), name.len()));
    found.truncate(MOST_FOUND);
    vec![EmojiGroup {
        label: FOUND.to_owned(),
        choices: found.into_iter().map(choice).collect(),
    }]
}

/// The emoji a name writes, such as 📚 for `books`.
fn named(name: &str) -> Option<&'static str> {
    every()
        .find(|(_, known)| *known == name)
        .map(|(emoji, _)| emoji)
}

/// A text with each `:name:` the picker knows written as its emoji, so `:books:` reads as 📚; anything else stays as typed.
#[must_use]
pub fn expand(text: &str) -> String {
    let mut written = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(COLON) {
        let (before, from) = rest.split_at(start);
        written.push_str(before);
        let after = from.get(COLON.len_utf8()..).unwrap_or_default();
        let shortcode = after
            .find(COLON)
            .and_then(|end| after.get(..end).map(|name| (name, end)));
        if let Some((emoji, end)) =
            shortcode.and_then(|(name, end)| named(name).map(|emoji| (emoji, end)))
        {
            written.push_str(emoji);
            rest = after
                .get(end.saturating_add(COLON.len_utf8())..)
                .unwrap_or_default();
        } else {
            // A colon that opens no known name stays, and the next one may open one.
            written.push(COLON);
            rest = after;
        }
    }
    written.push_str(rest);
    written
}

#[cfg(test)]
mod tests {
    use super::{CATEGORIES, EmojiChoice, every, expand, pick};
    use std::collections::BTreeSet;

    #[test]
    fn every_name_is_its_own_and_every_emoji_shows_once() {
        let names: BTreeSet<&str> = every().map(|(_, name)| name).collect();
        let emoji: BTreeSet<&str> = every().map(|(emoji, _)| emoji).collect();
        let count = every().count();
        assert_eq!((names.len(), emoji.len()), (count, count));
        assert!(
            every().all(|(_, name)| name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')),
            "a name writes as a shortcode"
        );
    }

    #[test]
    fn an_empty_search_shows_every_category_and_a_search_its_closest_names_first() {
        assert_eq!(pick("").len(), CATEGORIES.len());
        assert_eq!(pick("  ::  ").len(), CATEGORIES.len());
        let found = pick(":boo");
        let names: Vec<&str> = found
            .iter()
            .flat_map(|group| group.choices.iter().map(|choice| choice.name.as_str()))
            .collect();
        assert_eq!(names.first(), Some(&"book"), "{names:?}");
        assert!(names.contains(&"bookmark") && names.contains(&"closed_book"));
        assert_eq!(
            pick("File Folder")
                .first()
                .and_then(|group| group.choices.first()),
            Some(&EmojiChoice {
                emoji: "📁".to_owned(),
                name: "file_folder".to_owned()
            })
        );
        assert_eq!(
            pick("nothing like it")
                .first()
                .map(|group| group.choices.len()),
            Some(0)
        );
    }

    #[test]
    fn a_shortcode_writes_its_emoji_and_anything_else_stays() {
        assert_eq!(expand(":books:"), "📚");
        assert_eq!(expand(" :rocket::fire: "), " 🚀🔥 ");
        assert_eq!(expand(":nope: :books:"), ":nope: 📚");
        assert_eq!(expand("a:b"), "a:b");
        assert_eq!(expand("::books:"), ":📚");
        assert_eq!(expand(":books"), ":books");
        assert_eq!(expand("📚"), "📚");
    }
}
