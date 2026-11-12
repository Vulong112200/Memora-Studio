//! Request box (owner decision 2026-11-08: offline and keyword based — no language model, no cloud).
//! Reads a plain Vietnamese or English sentence ("video dọc 30 giây, nhiều cây cổ thụ, bỏ selfie, tông ấm,
//! tiêu đề: Đà Lạt") into a style, dates and editing Wishes, and says back what it understood so the user
//! can check before anything is made. Words it does not know are simply ignored.

use crate::editing::{LANDSCAPES, Wishes};
use crate::tagging::TagName;
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// Editing style id when the sentence names one (or asks for vertical video).
    pub style: Option<String>,
    /// Recording dates, inclusive YYYY-MM-DD (a month ends at "-31": dates compare as text).
    pub from: Option<String>,
    pub to: Option<String>,
    pub wishes: Wishes,
    /// What was understood, one short line each, for the user to check.
    pub understood: Vec<String>,
}

/// Lower-case Vietnamese without diacritics; one char in → one char out, so positions stay aligned.
fn fold_char(c: char) -> char {
    const GROUPS: &[(&str, char)] = &[
        ("àáạảãâầấậẩẫăằắặẳẵ", 'a'),
        ("èéẹẻẽêềếệểễ", 'e'),
        ("ìíịỉĩ", 'i'),
        ("òóọỏõôồốộổỗơờớợởỡ", 'o'),
        ("ùúụủũưừứựửữ", 'u'),
        ("ỳýỵỷỹ", 'y'),
        ("đ", 'd'),
    ];
    GROUPS.iter().find(|(set, _)| set.contains(c)).map_or(c, |&(_, base)| base)
}

fn fold(s: &str) -> String {
    s.to_lowercase().chars().map(fold_char).collect()
}

/// The sentence twice, aligned char by char: lower case with diacritics, and folded. Matched spans are
/// blanked so a word is used once (a title's words never become concepts; "bãi biển" is not also "biển").
struct Text {
    raw: Vec<char>,
    plain: Vec<char>,
}

const CLAUSE_END: &[char] = &[',', '.', ';', '!', '?', '\n'];

impl Text {
    fn new(s: &str) -> Self {
        // to_lowercase can change the char count for a few scripts; fold per lowered char keeps alignment.
        let raw: Vec<char> = s.to_lowercase().chars().collect();
        let plain = raw.iter().map(|&c| fold_char(c)).collect();
        Text { raw, plain }
    }

    /// Next whole-word match at or after `from`. Needles with diacritics match exactly; needles without
    /// them also match text typed with diacritics ("tieu de" finds "tiêu đề").
    fn find(&self, needle: &str, from: usize) -> Option<usize> {
        let n: Vec<char> = needle.to_lowercase().chars().collect();
        let exact = needle.to_lowercase() != fold(needle);
        let hay = if exact { &self.raw } else { &self.plain };
        let word = |i: usize| hay.get(i).is_some_and(|c| c.is_alphanumeric());
        if n.is_empty() || n.len() > hay.len() {
            return None;
        }
        (from..=hay.len() - n.len())
            .find(|&i| hay[i..i + n.len()] == n[..] && (i == 0 || !word(i - 1)) && !word(i + n.len()))
    }

    fn any(&self, needles: &[&str]) -> Option<(usize, usize)> {
        needles.iter().filter_map(|n| self.find(n, 0).map(|i| (i, n.chars().count()))).min()
    }

    fn blank(&mut self, at: usize, len: usize) {
        for i in at..(at + len).min(self.raw.len()) {
            self.raw[i] = ' ';
            self.plain[i] = ' ';
        }
    }

    /// Start of the clause containing `at`.
    fn clause_start(&self, at: usize) -> usize {
        self.raw[..at].iter().rposition(|c| CLAUSE_END.contains(c)).map_or(0, |i| i + 1)
    }

    fn clause_end(&self, at: usize) -> usize {
        self.raw[at..].iter().position(|c| CLAUSE_END.contains(c)).map_or(self.raw.len(), |i| at + i)
    }

    /// Text after a keyword: quoted ("…", “…”, '…') or the rest of the clause; the span is blanked.
    fn take_text_after(&mut self, keywords: &[&str], original: &[char]) -> Option<String> {
        let (at, len) = self.any(keywords)?;
        let mut i = at + len;
        while i < self.raw.len() && (self.raw[i] == ' ' || self.raw[i] == ':') {
            i += 1;
        }
        for filler in ["là ", "la "] {
            let f: Vec<char> = filler.chars().collect();
            if self.raw.get(i..i + f.len()) == Some(&f[..]) {
                i += f.len();
            }
        }
        let (start, end) = match self.raw.get(i) {
            Some(&q @ ('"' | '“' | '\'')) => {
                let close = if q == '“' { '”' } else { q };
                let end = self.raw[i + 1..].iter().position(|&c| c == close).map_or(self.raw.len(), |p| i + 1 + p);
                (i + 1, end)
            }
            _ => (i, self.clause_end(i)),
        };
        let text: String = original.get(start..end)?.iter().collect::<String>().trim().to_string();
        self.blank(at, end + 1 - at);
        (!text.is_empty()).then_some(text)
    }
}

const AVOID_WORDS: &[&str] = &["bỏ", "bo", "không", "khong", "tránh", "trừ", "loại", "except", "without", "no", "avoid", "skip"];
const LESS_WORDS: &[&str] = &["bớt", "ít", "less", "fewer", "fewer of"];

const STYLES: &[(&str, &[&str])] = &[
    ("journey_recap", &["tổng hợp", "tong hop", "hành trình", "hanh trinh", "cả chuyến đi", "ca chuyen di", "recap", "journey"]),
    ("travel_cinematic", &["cinematic", "điện ảnh", "dien anh"]),
    ("memory_nostalgia", &["hoài niệm", "hoai niem", "kỷ niệm", "ky niem", "nostalgia", "nostalgic", "memories"]),
    ("travel_documentary", &["tài liệu", "tai lieu", "documentary", "vlog", "kể chuyện", "ke chuyen"]),
    ("clean_minimal", &["tối giản", "toi gian", "minimal", "đơn giản", "don gian"]),
    ("social_story", &["sôi động", "soi dong", "năng động", "nang dong"]),
];
const VERTICAL: &[&str] = &["dọc", "video doc", "reels", "reel", "tiktok", "story", "shorts", "9:16", "vertical", "đứng"];
const HORIZONTAL: &[&str] = &["ngang", "landscape", "16:9", "youtube", "horizontal"];
const COLORS: &[(&str, &[&str])] = &[
    ("bw", &["đen trắng", "den trang", "trắng đen", "trang den", "black and white", "black & white"]),
    ("warm", &["ấm", "tông ấm", "tong am", "màu ấm", "mau am", "warm"]),
    ("cool", &["lạnh", "mát", "tông lạnh", "tong lanh", "cool", "cold"]),
    ("vivid", &["rực rỡ", "ruc ro", "tươi", "sặc sỡ", "vivid", "colourful", "colorful"]),
    ("vintage", &["cổ điển", "co dien", "hoài cổ", "hoai co", "vintage", "retro"]),
];
const MUSIC: &[(&str, &[&str])] = &[
    ("none", &["không nhạc", "khong nhac", "không cần nhạc", "khong can nhac", "bỏ nhạc", "bo nhac", "no music", "without music"]),
    ("low", &["nhạc nhẹ", "nhac nhe", "nhẹ nhàng", "nhe nhang", "du dương", "du duong", "calm music", "soft music", "gentle music"]),
    ("high", &["nhạc sôi động", "nhac soi dong", "nhạc mạnh", "nhac manh", "upbeat", "energetic music"]),
    ("medium", &["nhạc vui", "nhac vui", "vui tươi", "vui tuoi", "happy music"]),
    ("any", &["có nhạc", "co nhac", "nhạc nền", "nhac nen", "nhạc", "music"]),
];
/// Extra words for a tag (`en` name) besides its English / Vietnamese names.
/// Photos in the plan (mixed timelines). "ảnh tự sướng" (selfie) is a concept, not these.
const PHOTOS: &[(&str, &[&str])] = &[
    ("none", &["không dùng ảnh", "khong dung anh", "không có ảnh", "khong co anh", "không ảnh", "khong anh", "bỏ ảnh", "bo anh",
        "chỉ video", "chi video", "chỉ dùng video", "chi dung video", "no photos", "without photos", "videos only", "only videos"]),
    ("more", &["nhiều ảnh", "nhieu anh", "thêm nhiều ảnh", "them nhieu anh", "more photos"]),
    ("fewer", &["ít ảnh", "it anh", "bớt ảnh", "bot anh", "fewer photos"]),
    ("yes", &["thêm ảnh", "them anh", "có ảnh", "co anh", "kèm ảnh", "kem anh", "dùng ảnh", "dung anh", "cả ảnh", "ca anh",
        "with photos", "add photos", "photos too"]),
];
const PHOTO_EACH: &[&str] = &["mỗi ảnh", "moi anh", "mỗi tấm", "moi tam", "each photo", "per photo"];

const ALIASES: &[(&str, &[&str])] = &[
    ("bicycle", &["đạp xe", "dap xe", "cycling", "biking"]),
    ("hiking trail", &["đi bộ trong rừng", "di bo trong rung", "trekking", "hiking"]),
    // Scene cues (OpenCLIP model v4), owner's words 2026-11-11.
    ("forest trail", &["đường rừng", "duong rung", "đường nhỏ giữa rừng", "đường mòn trong rừng", "forest path"]),
    ("small bridge", &["qua cầu", "qua cau", "cầu nhỏ", "cau nho", "bridge"]),
    ("slope", &["lên dốc", "len doc", "xuống dốc", "xuong doc", "đường dốc", "duong doc", "uphill", "downhill"]),
];
const NATURE: &[&str] =&["thiên nhiên", "thien nhien", "phong cảnh", "phong canh", "nature", "scenery", "landscapes"];

/// Concept class from the words before it in its clause: "bỏ selfie" → avoid, "bớt xe đạp" → less.
fn class(t: &Text, at: usize) -> &'static str {
    let clause = Text { raw: t.raw[t.clause_start(at)..at].to_vec(), plain: t.plain[t.clause_start(at)..at].to_vec() };
    if LESS_WORDS.iter().any(|w| clause.find(w, 0).is_some()) {
        "less"
    } else if AVOID_WORDS.iter().any(|w| clause.find(w, 0).is_some()) {
        "avoid"
    } else {
        "prefer"
    }
}

/// Digits at `i` (with one optional decimal separator) → (value, next index).
fn number(c: &[char], i: usize) -> Option<(f64, usize)> {
    let mut j = i;
    while j < c.len() && (c[j].is_ascii_digit() || ((c[j] == '.' || c[j] == ',') && j > i && c.get(j + 1).is_some_and(|d| d.is_ascii_digit()))) {
        j += 1;
    }
    if j == i {
        return None;
    }
    c[i..j].iter().collect::<String>().replace(',', ".").parse().ok().map(|v| (v, j))
}

/// "30 giây", "1 phút 30 giây", "45s", "2 min" → milliseconds.
fn duration(t: &Text) -> Option<i64> {
    let c = &t.plain;
    let (mut total, mut i) = (0.0, 0);
    while i < c.len() {
        let starts = c[i].is_ascii_digit() && (i == 0 || !c[i - 1].is_alphanumeric());
        let Some((v, j)) = starts.then(|| number(c, i)).flatten() else {
            i += 1;
            continue;
        };
        let mut k = j;
        while k < c.len() && c[k] == ' ' {
            k += 1;
        }
        let unit: String = c[k..].iter().take_while(|c| c.is_alphabetic()).collect();
        total += v * match unit.as_str() {
            "s" | "sec" | "secs" | "second" | "seconds" | "giay" => 1000.0,
            "p" | "ph" | "phut" | "m" | "min" | "mins" | "minute" | "minutes" => 60_000.0,
            _ => 0.0,
        };
        i = k + unit.chars().count().max(1);
    }
    (total >= 1000.0).then_some(total as i64)
}

/// Recording dates: "từ 01/02/2026 đến 15/02/2026", "tháng 2/2026", "tháng 2 năm 2026", "02/2026", "năm 2025".
fn dates(t: &Text) -> Option<(String, String)> {
    let s: String = t.plain.iter().collect();
    let tokens: Vec<&str> = s.split(|c: char| c.is_whitespace() || (CLAUSE_END.contains(&c) && c != '.')).filter(|w| !w.is_empty()).collect();
    let year = |w: &str| w.parse::<u32>().ok().filter(|y| (1900..=2100).contains(y));
    let month = |w: &str| w.parse::<u32>().ok().filter(|m| (1..=12).contains(m));
    let parts = |w: &str| w.trim_end_matches('.').split(['/', '-']).map(String::from).collect::<Vec<_>>();
    let days: Vec<String> = tokens
        .iter()
        .filter_map(|w| match parts(w).as_slice() {
            [d, m, y] => Some(format!("{:04}-{:02}-{:02}", year(y)?, month(m)?, d.parse::<u32>().ok().filter(|d| (1..=31).contains(d))?)),
            _ => None,
        })
        .collect();
    match days.as_slice() {
        [a, b, ..] => return Some((a.clone().min(b.clone()), a.clone().max(b.clone()))),
        [a] => return Some((a.clone(), a.clone())),
        [] => {}
    }
    for (i, w) in tokens.iter().enumerate() {
        let next = |k: usize| tokens.get(i + k).copied().unwrap_or("");
        let month_year = match (*w, parts(w).as_slice()) {
            ("thang" | "month", _) => match parts(next(1)).as_slice() {
                [m, y] => month(m).zip(year(y)),
                [m] if next(2) == "nam" || next(2) == "year" => month(m).zip(year(next(3))),
                _ => None,
            },
            (_, [m, y]) => month(m).zip(year(y)),
            _ => None,
        };
        if let Some((m, y)) = month_year {
            return Some((format!("{y:04}-{m:02}-01"), format!("{y:04}-{m:02}-31")));
        }
        if (*w == "nam" || *w == "year") && let Some(y) = year(next(1)) {
            return Some((format!("{y:04}-01-01"), format!("{y:04}-12-31")));
        }
    }
    None
}

/// Reads `text`. `vocab` = the tag names the library was tagged with (empty when untagged).
pub fn parse(text: &str, vocab: &[TagName]) -> Request {
    let original: Vec<char> = text.chars().collect();
    let mut t = Text::new(text);
    let mut r = Request::default();
    let w = &mut r.wishes;
    w.request = text.trim().to_string();

    // Free text first, so its words are not read as wishes.
    w.title = t.take_text_after(&["tiêu đề", "tieu de", "title", "tên video", "ten video"], &original);
    w.ending = t.take_text_after(&["lời kết", "loi ket", "kết thúc bằng", "ket thuc bang", "câu kết", "cau ket", "ending"], &original);
    if let Some((from, to)) = dates(&t) {
        r.understood.push(if from == to { format!("recorded on {from}") } else { format!("recorded {from} – {to}") });
        (r.from, r.to) = (Some(from), Some(to.clone()));
    }
    // Photos before the length: "mỗi ảnh 4 giây" is not the video length.
    if let Some((at, len)) = t.any(PHOTO_EACH) {
        let end = t.clause_end(at);
        let mut k = at + len;
        while k < end && !t.plain[k].is_ascii_digit() {
            k += 1;
        }
        if let Some((v, _)) = number(&t.plain, k) {
            w.photo_ms = Some((v * 1000.0) as i64);
            t.blank(at, end - at);
        }
    }
    for (kind, needles) in PHOTOS {
        let selfie = |t: &Text, at: usize, len: usize| t.plain[at + len..].iter().collect::<String>().trim_start().starts_with("tu suong");
        if let Some((at, len)) = t.any(needles).filter(|&(at, len)| !selfie(&t, at, len)) {
            w.photos = Some((*kind).into());
            t.blank(at, len);
            break;
        }
    }
    if w.photo_ms.is_some() && w.photos.is_none() {
        w.photos = Some("yes".into());
    }
    w.target_ms = duration(&t);

    // Music before colours and concepts ("nhạc sôi động" is music, not the social style).
    if let Some((kind, (at, len))) = MUSIC.iter().find_map(|(k, n)| t.any(n).map(|m| (*k, m))) {
        w.music = Some(kind.into());
        t.blank(at, len);
    }
    if let Some(&(c, _)) = COLORS.iter().find(|(_, n)| t.any(n).is_some()) {
        w.color = Some(c.into());
    }
    for (style, names) in STYLES {
        if t.any(names).is_some() {
            r.style = Some((*style).into());
            break;
        }
    }
    if t.any(VERTICAL).is_some() {
        w.preset = Some("reels".into());
        r.style.get_or_insert_with(|| "social_story".into());
    } else if t.any(HORIZONTAL).is_some() {
        w.preset = Some("landscape".into());
    }
    if let Some((at, len)) = t.any(&["không phụ đề", "khong phu de", "no subtitles", "without subtitles"]) {
        w.captions = Some(false);
        t.blank(at, len);
    } else if t.any(&["phụ đề", "phu de", "subtitles", "captions"]).is_some() {
        w.captions = Some(true);
    }

    // Concepts, longest names first ("bãi biển" before "biển"); English or Vietnamese, with or without diacritics.
    let mut names: Vec<(String, Vec<String>)> = vocab
        .iter()
        .filter(|n| !n.negative)
        .map(|n| {
            let mut needles = vec![n.en.to_lowercase(), n.vi.to_lowercase()];
            // Without diacritics only when unambiguous: several words, or one long word ("thuyen" = thuyền),
            // never short ones ("bien" could be biến, "rung" rung, "song" song).
            if n.vi.contains(' ') || n.vi.chars().count() >= 5 {
                needles.push(fold(&n.vi));
            }
            // Verbs people use for the activity (owner's "chuyến đi đạp xe" was not read as bicycle).
            needles.extend(ALIASES.iter().filter(|(en, _)| *en == n.en).flat_map(|(_, a)| a.iter().map(|s| s.to_string())));
            (n.en.clone(), needles)
        })
        .collect();
    names.sort_by_key(|(_, n)| std::cmp::Reverse(n.iter().map(|s| s.chars().count()).max().unwrap_or(0)));
    let mut found: Vec<(usize, &'static str, String)> = vec![];
    for (en, needles) in &names {
        for needle in needles {
            while let Some(at) = t.find(needle, 0) {
                if !found.iter().any(|(_, _, e)| e == en) {
                    found.push((at, class(&t, at), en.clone()));
                }
                t.blank(at, needle.chars().count());
            }
        }
    }
    if let Some((at, _)) = t.any(NATURE) {
        let k = class(&t, at);
        for en in LANDSCAPES.iter().filter(|e| vocab.iter().any(|n| n.en == **e)) {
            if !found.iter().any(|(_, _, x)| x == en) {
                found.push((at, k, (*en).into()));
            }
        }
    }
    found.sort_by_key(|f| f.0); // read back in the order the user wrote them
    for (_, k, en) in found {
        match k {
            "avoid" => w.avoid.push(en),
            "less" => w.less.push(en),
            _ => w.prefer.push(en),
        }
    }

    // Read back.
    let vi = |list: &[String]| {
        list.iter().map(|e| vocab.iter().find(|n| &n.en == e).map_or(e.clone(), |n| n.vi.clone())).collect::<Vec<_>>().join(", ")
    };
    let w = &r.wishes;
    let mut u = vec![];
    if let Some(s) = &r.style {
        u.push(format!("style {s}"));
    }
    match w.preset.as_deref() {
        Some("reels") => u.push("vertical 9:16".into()),
        Some(_) => u.push("horizontal 16:9".into()),
        None => {}
    }
    if let Some(ms) = w.target_ms {
        u.push(format!("length {} s", ms / 1000));
    }
    for (label, list) in [("more", &w.prefer), ("less", &w.less), ("leave out", &w.avoid)] {
        if !list.is_empty() {
            u.push(format!("{label}: {}", vi(list)));
        }
    }
    if let Some(c) = &w.color {
        u.push(format!("colour {c}"));
    }
    match w.music.as_deref() {
        Some("none") => u.push("no music".into()),
        Some("any") => u.push("library music (best match)".into()),
        Some(e) => u.push(format!("library music, {e} energy")),
        None => {}
    }
    if let Some(c) = w.captions {
        u.push(format!("speech subtitles {}", if c { "on" } else { "off" }));
    }
    match w.photos.as_deref() {
        Some("none") => u.push("video only, no photos".into()),
        Some(p) => u.push(format!("photos: {p}")),
        None => {}
    }
    if let Some(ms) = w.photo_ms {
        u.push(format!("each photo {} s", ms as f64 / 1000.0));
    }
    if let Some(x) = &w.title {
        u.push(format!("title \"{x}\""));
    }
    if let Some(x) = &w.ending {
        u.push(format!("ending \"{x}\""));
    }
    r.understood.splice(0..0, u);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vec<TagName> {
        [
            ("big tree", "cây cổ thụ"), ("hiking trail", "đường mòn"), ("selfie", "ảnh tự sướng"), ("bicycle", "xe đạp"),
            ("sea", "biển"), ("beach", "bãi biển"), ("mountain", "núi"), ("forest", "rừng"), ("people", "con người"), ("boat", "thuyền"),
        ]
        .iter()
        .map(|(e, v)| TagName { en: (*e).into(), vi: (*v).into(), negative: false })
        .chain(std::iter::once(TagName { en: "blurry shot".into(), vi: "cảnh bị mờ".into(), negative: true }))
        .collect()
    }

    #[test]
    fn trail_scenes_by_the_owners_words() {
        let mut v = vocab();
        v.extend([("forest trail", "đường mòn trong rừng"), ("small bridge", "qua cầu nhỏ"), ("slope", "đường dốc trong rừng")]
            .iter().map(|(e, vi)| TagName { en: (*e).into(), vi: (*vi).into(), negative: false }));
        let r = parse("tổng hợp chuyến đi, nhiều cảnh đường rừng, lên dốc, qua cầu", &v);
        assert_eq!(r.wishes.prefer, ["forest trail", "slope", "small bridge"], "{:?}", r.understood);
        assert_eq!(r.wishes.target_ms, None, "no length named: the recap picks one from the footage");
    }

    #[test]
    fn photos_in_the_request() {
        let r = parse("video 5 phút có cả ảnh, mỗi ảnh 3 giây, bỏ ảnh tự sướng", &vocab());
        let w = &r.wishes;
        assert_eq!((w.photos.as_deref(), w.photo_ms, w.target_ms), (Some("yes"), Some(3_000), Some(300_000)), "{:?}", r.understood);
        assert_eq!(w.avoid, ["selfie"], "\"bỏ ảnh tự sướng\" is the selfie concept, not \"no photos\"");
        assert_eq!(parse("chỉ video, không nhạc", &vocab()).wishes.photos.as_deref(), Some("none"));
        assert_eq!(parse("tổng hợp chuyến đi, ít ảnh thôi", &vocab()).wishes.photos.as_deref(), Some("fewer"));
        assert_eq!(parse("tổng hợp chuyến đi", &vocab()).wishes.photos, None);
    }

    #[test]
    fn owner_style_request_in_vietnamese() {
        let r = parse("Video dọc 30 giây, nhiều cảnh cây cổ thụ và đường mòn, bỏ selfie, bớt xe đạp, tông ấm, nhạc nhẹ, tiêu đề: \"Đà Lạt 2026\"", &vocab());
        let w = &r.wishes;
        assert_eq!((r.style.as_deref(), w.preset.as_deref(), w.target_ms), (Some("social_story"), Some("reels"), Some(30_000)));
        assert_eq!(w.prefer, ["big tree", "hiking trail"]);
        assert_eq!((w.avoid.as_slice(), w.less.as_slice()), (&["selfie".to_string()][..], &["bicycle".to_string()][..]));
        assert_eq!((w.color.as_deref(), w.music.as_deref(), w.title.as_deref()), (Some("warm"), Some("low"), Some("Đà Lạt 2026")));
        assert!(r.understood.iter().any(|l| l == "more: cây cổ thụ, đường mòn") && r.understood.iter().any(|l| l == "leave out: ảnh tự sướng"), "{:?}", r.understood);
    }

    #[test]
    fn without_diacritics_and_dates() {
        let r = parse("video ngang 1 phut 30 giay, thien nhien, khong nhac, co phu de, thang 2/2026", &vocab());
        let w = &r.wishes;
        assert_eq!((w.preset.as_deref(), w.target_ms, w.music.as_deref(), w.captions), (Some("landscape"), Some(90_000), Some("none"), Some(true)));
        assert_eq!(w.prefer, ["mountain", "sea", "beach", "forest"], "nature = the landscape concepts the library knows");
        assert_eq!((r.from.as_deref(), r.to.as_deref()), (Some("2026-02-01"), Some("2026-02-31")));
        assert_eq!(r.style, None, "no style word: the selected style is kept");
        assert_eq!(parse("video tổng hợp cả chuyến đi, 4 phút", &vocab()).style.as_deref(), Some("journey_recap"));
        assert_eq!(parse("hanh trinh dap xe", &vocab()).style.as_deref(), Some("journey_recap"));
        assert_eq!(parse("tổng hợp chuyến đi đạp xe", &vocab()).wishes.prefer, ["bicycle"], "the activity verb names the concept");
        assert_eq!(parse("bớt đạp xe", &vocab()).wishes.less, ["bicycle"]);
    }

    #[test]
    fn free_text_is_not_read_as_wishes() {
        let r = parse("Hoài niệm năm 2025, tiêu đề Biển xanh, kết thúc bằng: Cảm ơn mọi người", &vocab());
        assert_eq!((r.style.as_deref(), r.from.as_deref(), r.to.as_deref()), (Some("memory_nostalgia"), Some("2025-01-01"), Some("2025-12-31")));
        assert_eq!((r.wishes.title.as_deref(), r.wishes.ending.as_deref()), (Some("Biển xanh"), Some("Cảm ơn mọi người")));
        assert!(r.wishes.prefer.is_empty(), "\"Biển\" in the title is not a wish: {:?}", r.wishes.prefer);
    }

    #[test]
    fn longer_names_win_and_clauses_scope_negation() {
        let r = parse("nhiều bãi biển, không selfie và xe đạp, thêm rừng", &vocab());
        assert_eq!(r.wishes.prefer, ["beach", "forest"], "bãi biển is beach, not also sea");
        assert_eq!(r.wishes.avoid, ["selfie", "bicycle"], "negation covers its clause only");
        let r = parse("from 01/02/2026 to 15/02/2026, less people, upbeat, cinematic, black and white", &vocab());
        assert_eq!((r.from.as_deref(), r.to.as_deref()), (Some("2026-02-01"), Some("2026-02-15")));
        assert_eq!((r.wishes.less.as_slice(), r.wishes.music.as_deref()), (&["people".to_string()][..], Some("high")));
        assert_eq!((r.style.as_deref(), r.wishes.color.as_deref()), (Some("travel_cinematic"), Some("bw")));
    }

    #[test]
    fn nothing_known_means_nothing_changed() {
        let r = parse("làm cho thật cảm động nha", &vocab());
        assert!(r.understood.is_empty() && r.style.is_none());
        assert_eq!(Wishes { request: String::new(), ..r.wishes }, Wishes::default());
        assert!(parse("", &[]).understood.is_empty());
        assert_eq!(parse("ấm áp", &vocab()).wishes.color.as_deref(), Some("warm"));
        assert_eq!(parse("âm nhạc", &vocab()).wishes.color, None, "âm (sound) is not ấm (warm)");
        assert_eq!(parse("nhieu thuyen", &vocab()).wishes.prefer, ["boat"], "long single word without diacritics");
        assert!(parse("bien doi mau", &vocab()).wishes.prefer.is_empty(), "short word without diacritics is ambiguous (biến ≠ biển)");
    }
}
