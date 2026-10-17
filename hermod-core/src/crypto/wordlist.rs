//! EFF 短词表 1（1296 词）+ 传输码生成/解析。
//!
//! 对应 Go 版 `hermod-go/internal/crypto/crypto.go` 的：
//! - `effShortWordlist`（第 458 行）        → [`EFF_SHORT_WORDLIST`]
//! - `randomWordIndex`（第 684 行）         → [`random_word_index`]
//! - `GenerateTransferCode`（第 703 行）    → [`generate_transfer_code`]
//! - `ParseTransferCode`（第 730 行）       → [`parse_transfer_code`]
//!
//! 词表是 EFF Short Wordlist 1（1296 词，全部唯一），从 Go 源码逐字提取，
//! 保证逐字节一致（M1.6 会用向量对拍）。3 词口令熵 log₂(1296³) ≈ 31.9 bits。

use rand_core::Rng;

/// EFF 短词表 1：完整 1296 词，每词唯一。索引 0..1295 与 Go 数组完全一致。
#[allow(clippy::large_const_arrays)] // 刻意做成大常量：静态只读数据、须与 Go 逐字节一致，拆散反而增加出错面
pub const EFF_SHORT_WORDLIST: [&str; 1296] = [
    "acid", "acorn", "acre", "acts", "afar", "affix", "aged", "agent",
    "agile", "aging", "agony", "ahead", "aide", "aids", "aim", "ajar",
    "alarm", "alias", "alibi", "alien", "alike", "alive", "aloe", "aloft",
    "aloha", "alone", "amend", "amino", "ample", "amuse", "angel", "anger",
    "angle", "ankle", "apple", "april", "apron", "aqua", "area", "arena",
    "argue", "arise", "armed", "armor", "army", "aroma", "array", "arson",
    "art", "ashen", "ashes", "atlas", "atom", "attic", "audio", "avert",
    "avoid", "awake", "award", "awoke", "axis", "bacon", "badge", "bagel",
    "baggy", "baked", "baker", "balmy", "banjo", "barge", "barn", "bash",
    "basil", "bask", "batch", "bath", "baton", "bats", "blade", "blank",
    "blast", "blaze", "bleak", "blend", "bless", "blimp", "blink", "bloat",
    "blob", "blog", "blot", "blunt", "blurt", "blush", "boast", "boat",
    "body", "boil", "bok", "bolt", "boned", "boney", "bonus", "bony",
    "book", "booth", "boots", "boss", "botch", "both", "boxer", "breed",
    "bribe", "brick", "bride", "brim", "bring", "brink", "brisk", "broad",
    "broil", "broke", "brook", "broom", "brush", "buck", "bud", "buggy",
    "bulge", "bulk", "bully", "bunch", "bunny", "bunt", "bush", "bust",
    "busy", "buzz", "cable", "cache", "cadet", "cage", "cake", "calm",
    "cameo", "canal", "candy", "cane", "canon", "cape", "card", "cargo",
    "carol", "carry", "carve", "case", "cash", "cause", "cedar", "chain",
    "chair", "chant", "chaos", "charm", "chase", "cheek", "cheer", "chef",
    "chess", "chest", "chew", "chief", "chili", "chill", "chip", "chomp",
    "chop", "chow", "chuck", "chump", "chunk", "churn", "chute", "cider",
    "cinch", "city", "civic", "civil", "clad", "claim", "clamp", "clap",
    "clash", "clasp", "class", "claw", "clay", "clean", "clear", "cleat",
    "cleft", "clerk", "click", "cling", "clink", "clip", "cloak", "clock",
    "clone", "cloth", "cloud", "clump", "coach", "coast", "coat", "cod",
    "coil", "coke", "cola", "cold", "colt", "coma", "come", "comic",
    "comma", "cone", "cope", "copy", "coral", "cork", "cost", "cot",
    "couch", "cough", "cover", "cozy", "craft", "cramp", "crane", "crank",
    "crate", "crave", "crawl", "crazy", "creme", "crepe", "crept", "crib",
    "cried", "crisp", "crook", "crop", "cross", "crowd", "crown", "crumb",
    "crush", "crust", "cub", "cult", "cupid", "cure", "curl", "curry",
    "curse", "curve", "curvy", "cushy", "cut", "cycle", "dab", "dad",
    "daily", "dairy", "daisy", "dance", "dandy", "darn", "dart", "dash",
    "data", "date", "dawn", "deaf", "deal", "dean", "debit", "debt",
    "debug", "decaf", "decal", "decay", "deck", "decor", "decoy", "deed",
    "delay", "denim", "dense", "dent", "depth", "derby", "desk", "dial",
    "diary", "dice", "dig", "dill", "dime", "dimly", "diner", "dingy",
    "disco", "dish", "disk", "ditch", "ditzy", "dizzy", "dock", "dodge",
    "doing", "doll", "dome", "donor", "donut", "dose", "dot", "dove",
    "down", "dowry", "doze", "drab", "drama", "drank", "draw", "dress",
    "dried", "drift", "drill", "drive", "drone", "droop", "drove", "drown",
    "drum", "dry", "duck", "duct", "dude", "dug", "duke", "duo",
    "dusk", "dust", "duty", "dwarf", "dwell", "eagle", "early", "earth",
    "easel", "east", "eaten", "eats", "ebay", "ebony", "ebook", "echo",
    "edge", "eel", "eject", "elbow", "elder", "elf", "elk", "elm",
    "elope", "elude", "elves", "email", "emit", "empty", "emu", "enter",
    "entry", "envoy", "equal", "erase", "error", "erupt", "essay", "etch",
    "evade", "even", "evict", "evil", "evoke", "exact", "exit", "fable",
    "faced", "fact", "fade", "fall", "false", "fancy", "fang", "fax",
    "feast", "feed", "femur", "fence", "fend", "ferry", "fetal", "fetch",
    "fever", "fiber", "fifth", "fifty", "film", "filth", "final", "finch",
    "fit", "five", "flag", "flaky", "flame", "flap", "flask", "fled",
    "flick", "fling", "flint", "flip", "flirt", "float", "flock", "flop",
    "floss", "flyer", "foam", "foe", "fog", "foil", "folic", "folk",
    "food", "fool", "found", "fox", "foyer", "frail", "frame", "fray",
    "fresh", "fried", "frill", "frisk", "from", "front", "frost", "froth",
    "frown", "froze", "fruit", "gag", "gains", "gala", "game", "gap",
    "gas", "gave", "gear", "gecko", "geek", "gem", "genre", "gift",
    "gig", "gills", "given", "giver", "glad", "glass", "glide", "gloss",
    "glove", "glow", "glue", "goal", "going", "golf", "gong", "good",
    "gooey", "goofy", "gore", "gown", "grab", "grain", "grant", "grape",
    "graph", "grasp", "grass", "grave", "gravy", "gray", "green", "greet",
    "grew", "grid", "grief", "grill", "grip", "grit", "groom", "grope",
    "growl", "grub", "grunt", "guide", "gulf", "gulp", "gummy", "guru",
    "gush", "gut", "guy", "habit", "half", "halo", "halt", "happy",
    "harm", "hash", "hasty", "hatch", "hate", "haven", "hazel", "hazy",
    "heap", "heat", "heave", "hedge", "hefty", "help", "herbs", "hers",
    "hub", "hug", "hula", "hull", "human", "humid", "hump", "hung",
    "hunk", "hunt", "hurry", "hurt", "hush", "hut", "ice", "icing",
    "icon", "icy", "igloo", "image", "ion", "iron", "islam", "issue",
    "item", "ivory", "ivy", "jab", "jam", "jaws", "jazz", "jeep",
    "jelly", "jet", "jiffy", "job", "jog", "jolly", "jolt", "jot",
    "joy", "judge", "juice", "juicy", "july", "jumbo", "jump", "junky",
    "juror", "jury", "keep", "keg", "kept", "kick", "kilt", "king",
    "kite", "kitty", "kiwi", "knee", "knelt", "koala", "kung", "ladle",
    "lady", "lair", "lake", "lance", "land", "lapel", "large", "lash",
    "lasso", "last", "latch", "late", "lazy", "left", "legal", "lemon",
    "lend", "lens", "lent", "level", "lever", "lid", "life", "lift",
    "lilac", "lily", "limb", "limes", "line", "lint", "lion", "lip",
    "list", "lived", "liver", "lunar", "lunch", "lung", "lurch", "lure",
    "lurk", "lying", "lyric", "mace", "maker", "malt", "mama", "mango",
    "manor", "many", "map", "march", "mardi", "marry", "mash", "match",
    "mate", "math", "moan", "mocha", "moist", "mold", "mom", "moody",
    "mop", "morse", "most", "motor", "motto", "mount", "mouse", "mousy",
    "mouth", "move", "movie", "mower", "mud", "mug", "mulch", "mule",
    "mull", "mumbo", "mummy", "mural", "muse", "music", "musky", "mute",
    "nacho", "nag", "nail", "name", "nanny", "nap", "navy", "near",
    "neat", "neon", "nerd", "nest", "net", "next", "niece", "ninth",
    "nutty", "oak", "oasis", "oat", "ocean", "oil", "old", "olive",
    "omen", "onion", "only", "ooze", "opal", "open", "opera", "opt",
    "otter", "ouch", "ounce", "outer", "oval", "oven", "owl", "ozone",
    "pace", "pagan", "pager", "palm", "panda", "panic", "pants", "panty",
    "paper", "park", "party", "pasta", "patch", "path", "patio", "payer",
    "pecan", "penny", "pep", "perch", "perky", "perm", "pest", "petal",
    "petri", "petty", "photo", "plank", "plant", "plaza", "plead", "plot",
    "plow", "pluck", "plug", "plus", "poach", "pod", "poem", "poet",
    "pogo", "point", "poise", "poker", "polar", "polio", "polka", "polo",
    "pond", "pony", "poppy", "pork", "poser", "pouch", "pound", "pout",
    "power", "prank", "press", "print", "prior", "prism", "prize", "probe",
    "prong", "proof", "props", "prude", "prune", "pry", "pug", "pull",
    "pulp", "pulse", "puma", "punch", "punk", "pupil", "puppy", "purr",
    "purse", "push", "putt", "quack", "quake", "query", "quiet", "quill",
    "quilt", "quit", "quota", "quote", "rabid", "race", "rack", "radar",
    "radio", "raft", "rage", "raid", "rail", "rake", "rally", "ramp",
    "ranch", "range", "rank", "rant", "rash", "raven", "reach", "react",
    "ream", "rebel", "recap", "relax", "relay", "relic", "remix", "repay",
    "repel", "reply", "rerun", "reset", "rhyme", "rice", "rich", "ride",
    "rigid", "rigor", "rinse", "riot", "ripen", "rise", "risk", "ritzy",
    "rival", "river", "roast", "robe", "robin", "rock", "rogue", "roman",
    "romp", "rope", "rover", "royal", "ruby", "rug", "ruin", "rule",
    "runny", "rush", "rust", "rut", "sadly", "sage", "said", "saint",
    "salad", "salon", "salsa", "salt", "same", "sandy", "santa", "satin",
    "sauna", "saved", "savor", "sax", "say", "scale", "scam", "scan",
    "scare", "scarf", "scary", "scoff", "scold", "scoop", "scoot", "scope",
    "score", "scorn", "scout", "scowl", "scrap", "scrub", "scuba", "scuff",
    "sect", "sedan", "self", "send", "sepia", "serve", "set", "seven",
    "shack", "shade", "shady", "shaft", "shaky", "sham", "shape", "share",
    "sharp", "shed", "sheep", "sheet", "shelf", "shell", "shine", "shiny",
    "ship", "shirt", "shock", "shop", "shore", "shout", "shove", "shown",
    "showy", "shred", "shrug", "shun", "shush", "shut", "shy", "sift",
    "silk", "silly", "silo", "sip", "siren", "sixth", "size", "skate",
    "skew", "skid", "skier", "skies", "skip", "skirt", "skit", "sky",
    "slab", "slack", "slain", "slam", "slang", "slash", "slate", "slaw",
    "sled", "sleek", "sleep", "sleet", "slept", "slice", "slick", "slimy",
    "sling", "slip", "slit", "slob", "slot", "slug", "slum", "slurp",
    "slush", "small", "smash", "smell", "smile", "smirk", "smog", "snack",
    "snap", "snare", "snarl", "sneak", "sneer", "sniff", "snore", "snort",
    "snout", "snowy", "snub", "snuff", "speak", "speed", "spend", "spent",
    "spew", "spied", "spill", "spiny", "spoil", "spoke", "spoof", "spool",
    "spoon", "sport", "spot", "spout", "spray", "spree", "spur", "squad",
    "squat", "squid", "stack", "staff", "stage", "stain", "stall", "stamp",
    "stand", "stank", "stark", "start", "stash", "state", "stays", "steam",
    "steep", "stem", "step", "stew", "stick", "sting", "stir", "stock",
    "stole", "stomp", "stony", "stood", "stool", "stoop", "stop", "storm",
    "stout", "stove", "straw", "stray", "strut", "stuck", "stud", "stuff",
    "stump", "stung", "stunt", "suds", "sugar", "sulk", "surf", "sushi",
    "swab", "swan", "swarm", "sway", "swear", "sweat", "sweep", "swell",
    "swept", "swim", "swing", "swipe", "swirl", "swoop", "swore", "syrup",
    "tacky", "taco", "tag", "take", "tall", "talon", "tamer", "tank",
    "taper", "taps", "tarot", "tart", "task", "taste", "tasty", "taunt",
    "thank", "thaw", "theft", "theme", "thigh", "thing", "think", "thong",
    "thorn", "those", "throb", "thud", "thumb", "thump", "thus", "tiara",
    "tidal", "tidy", "tiger", "tile", "tilt", "tint", "tiny", "trace",
    "track", "trade", "train", "trait", "trap", "trash", "tray", "treat",
    "tree", "trek", "trend", "trial", "tribe", "trick", "trio", "trout",
    "truce", "truck", "trump", "trunk", "try", "tug", "tulip", "tummy",
    "turf", "tusk", "tutor", "tutu", "tux", "tweak", "tweet", "twice",
    "twine", "twins", "twirl", "twist", "uncle", "uncut", "undo", "unify",
    "union", "unit", "untie", "upon", "upper", "urban", "used", "user",
    "usher", "utter", "value", "vapor", "vegan", "venue", "verse", "vest",
    "veto", "vice", "video", "view", "viral", "virus", "visa", "visor",
    "vixen", "vocal", "voice", "void", "volt", "voter", "vowel", "wad",
    "wafer", "wager", "wages", "wagon", "wake", "walk", "wand", "wasp",
    "watch", "water", "wavy", "wheat", "whiff", "whole", "whoop", "wick",
    "widen", "widow", "width", "wife", "wifi", "wilt", "wimp", "wind",
    "wing", "wink", "wipe", "wired", "wiry", "wise", "wish", "wispy",
    "wok", "wolf", "womb", "wool", "woozy", "word", "work", "worry",
    "wound", "woven", "wrath", "wreck", "wrist", "xerox", "yahoo", "yam",
    "yard", "year", "yeast", "yelp", "yield", "yo-yo", "yodel", "yoga",
    "yoyo", "yummy", "zebra", "zero", "zesty", "zippy", "zone", "zoom",
];

/// 词表长度（1296）。
pub const WORD_COUNT: usize = EFF_SHORT_WORDLIST.len();

/// 把随机 uint16 映射为词索引 [0, WORD_COUNT)，用拒绝采样消除取模偏差。
///
/// 对应 Go `randomWordIndex` 的接受逻辑：`limit = floor(65536/1296)*1296 = 64800`，
/// 只有 `v < 64800` 的值被接受（`v % 1296` 均匀），否则返回 `None` 表示"重抽"。
fn word_index_from_u16(v: u16) -> Option<usize> {
    const LIMIT: usize = (65536 / WORD_COUNT) * WORD_COUNT; // 50 * 1296 = 64800
    (usize::from(v) < LIMIT).then_some(usize::from(v) % WORD_COUNT)
}

/// 均匀随机返回一个词索引 [0, WORD_COUNT)，拒绝采样（对应 Go `randomWordIndex`）。
pub fn random_word_index(rng: &mut impl Rng) -> usize {
    loop {
        let mut buf = [0u8; 2];
        rng.fill_bytes(&mut buf);
        if let Some(idx) = word_index_from_u16(u16::from_be_bytes(buf)) {
            return idx;
        }
    }
}

/// 生成随机传输码：`<channelID 十进制>-word-word-...`（对应 Go `GenerateTransferCode`）。
///
/// - `num_words < 3` 时按 3 处理（与 Go 一致）。
/// - channelID 是随机 uint16（大端读取，与 Go `binary.BigEndian.Uint16` 一致）。
/// - 每个词独立拒绝采样，无模偏差。
///
/// 返回 (channelID, 传输码字符串)。
pub fn generate_transfer_code(rng: &mut impl Rng, num_words: usize) -> (u16, String) {
    let num_words = num_words.max(3);

    let mut id_buf = [0u8; 2];
    rng.fill_bytes(&mut id_buf);
    let channel_id = u16::from_be_bytes(id_buf);

    let mut words = Vec::with_capacity(num_words);
    for _ in 0..num_words {
        words.push(EFF_SHORT_WORDLIST[random_word_index(rng)]);
    }

    let code = format!("{channel_id}-{}", words.join("-"));
    (channel_id, code)
}

/// 解析传输码为 (channelID, 词列表)。对应 Go `ParseTransferCode`。
///
/// - 格式错误（没有 `-`）        → [`WordlistError::InvalidFormat`]
/// - channelID 不是合法 uint16    → [`WordlistError::InvalidChannelId`]
/// - 词数 < 3                    → [`WordlistError::TooFewWords`]
pub fn parse_transfer_code(code: &str) -> Result<(u16, Vec<&str>), WordlistError> {
    let (id_part, rest) = code
        .split_once('-')
        .ok_or(WordlistError::InvalidFormat)?;
    let channel_id = id_part
        .parse::<u16>()
        .map_err(|_| WordlistError::InvalidChannelId)?;
    let words: Vec<&str> = rest.split('-').collect();
    if words.len() < 3 {
        return Err(WordlistError::TooFewWords);
    }
    Ok((channel_id, words))
}

/// 传输码解析错误。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WordlistError {
    #[error("invalid transfer code format")]
    InvalidFormat,
    #[error("invalid channel ID in transfer code")]
    InvalidChannelId,
    #[error("transfer code must contain at least 3 words")]
    TooFewWords,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 词表完整性与 Go 逐字节一致：1296 词、全部唯一。
    #[test]
    fn wordlist_matches_go() {
        assert_eq!(EFF_SHORT_WORDLIST.len(), 1296);
        assert_eq!(EFF_SHORT_WORDLIST[0], "acid"); // 首词
        assert_eq!(EFF_SHORT_WORDLIST[1295], "zoom"); // 末词
        let mut sorted = EFF_SHORT_WORDLIST.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 1296, "词表有重复词");
    }

    /// 拒绝采样边界：v < 64800 接受，>= 64800 拒绝（对应 Go 的 limit）。
    #[test]
    fn word_index_rejection_boundaries() {
        assert_eq!(word_index_from_u16(0), Some(0)); // "acid"
        assert_eq!(word_index_from_u16(1295), Some(1295)); // "zoom"
        assert_eq!(word_index_from_u16(1296), Some(0)); // 1296 % 1296 = 0，取模均匀
        assert_eq!(word_index_from_u16(64799), Some(64799 % 1296));
        assert_eq!(word_index_from_u16(64800), None); // 恰好等于 limit → 拒绝
        assert_eq!(word_index_from_u16(65535), None); // 最大值 → 拒绝
    }

    /// 固定随机源下的确定性向量（与手算一致，可作 Go 对拍基准）。
    #[test]
    fn deterministic_generate_and_parse() {
        // 字节流：channelID(2) | 每词 2 字节；各值均 < 64800 直接命中
        let mut rng = TestRng::new(&[
            0x12, 0x34, // channelID = 0x1234 = 4660
            0x00, 0x00, // 词索引 0  → "acid"
            0x00, 0x01, // 词索引 1  → "acorn"
            0x00, 0x02, // 词索引 2  → "acre"
            0x00, 0x03, // 词索引 3  → "acts"
        ]);
        let (channel_id, code) = generate_transfer_code(&mut rng, 4);
        assert_eq!(channel_id, 4660);
        assert_eq!(code, "4660-acid-acorn-acre-acts");

        // 解析回来必须还原（round-trip）
        let (parsed_id, words) = parse_transfer_code(&code).unwrap();
        assert_eq!(parsed_id, channel_id);
        assert_eq!(words, vec!["acid", "acorn", "acre", "acts"]);
    }

    /// 少于 3 个词：生成时按 3 个词处理（与 Go 一致）。
    #[test]
    fn generate_min_words() {
        let mut rng = TestRng::new(&[
            0x00, 0x2a, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02,
        ]);
        let (id, code) = generate_transfer_code(&mut rng, 1); // 请求 1 个 → 实际 3 个
        assert_eq!(id, 42);
        assert_eq!(code, "42-acid-acorn-acre");
    }

    /// 非法传输码必须报错（对应 Go TestParseTransferCodeInvalid）。
    #[test]
    fn parse_invalid() {
        // 完全没有连字符 → 格式错误（Go：SplitN 只得到 1 段）
        assert_eq!(
            parse_transfer_code("acorn"),
            Err(WordlistError::InvalidFormat)
        );
        // 有连字符但前缀不是 uint16（Go："no" 无法 Sscanf 进 uint16）
        assert_eq!(
            parse_transfer_code("no-dash-here"),
            Err(WordlistError::InvalidChannelId)
        );
        assert_eq!(
            parse_transfer_code("65536-acid-acorn-acre"), // uint16 溢出
            Err(WordlistError::InvalidChannelId)
        );
        assert_eq!(
            parse_transfer_code("42-acid-acorn"), // 只有 2 个词
            Err(WordlistError::TooFewWords)
        );
    }

    // ------------------------------------------------------------------
    // 测试用确定性 RNG：按固定字节流喂数据。
    // 实现 rand_core::TryRng（Error=Infallible），自动获得 Rng 实现。
    // ------------------------------------------------------------------
    struct TestRng {
        bytes: Vec<u8>,
        pos: usize,
    }

    impl TestRng {
        fn new(bytes: &[u8]) -> Self {
            Self { bytes: bytes.to_vec(), pos: 0 }
        }
    }

    impl rand_core::TryRng for TestRng {
        type Error = std::convert::Infallible;

        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            let mut b = [0u8; 4];
            self.try_fill_bytes(&mut b)?;
            Ok(u32::from_be_bytes(b))
        }

        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            let mut b = [0u8; 8];
            self.try_fill_bytes(&mut b)?;
            Ok(u64::from_be_bytes(b))
        }

        fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
            for b in dst.iter_mut() {
                *b = self.bytes[self.pos % self.bytes.len()];
                self.pos += 1;
            }
            Ok(())
        }
    }
}
