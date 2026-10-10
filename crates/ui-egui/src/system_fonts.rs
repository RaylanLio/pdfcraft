//! The last-resort interface fonts: faces already installed on this machine.
//!
//! The embedded faces (Inter, egui's defaults, craft-fonts) come first in every family; these
//! only draw characters none of them has: Chinese, Japanese, Korean and Telugu text, or an
//! Arabic file name, in a build without craft-fonts. They are read at runtime and never embedded
//! or shipped (AGENTS.md §1.4), and `PDFCRAFT_SYSTEM_FONTS=0` turns them off (published
//! screenshots do).
//!
//! A collection such as Noto Sans CJK holds one face per region; [`HanForms`] makes the Han
//! group prefer the interface language's face (`SC` for Simplified Chinese, `TC` for Traditional
//! Chinese) over the first one that fits.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use egui::FontData;
use skrifa::MetadataProvider as _;

/// Every entry this module adds to the font families starts with this.
pub const PREFIX: &str = "system-fallback";

/// Larger files are not read: a font path is still untrusted input. (Noto Sans CJK's `.ttc` is
/// about 20 MiB.)
const MAX_BYTES: u64 = 32 << 20;
/// Faces tried in a collection (`.ttc`).
const MAX_FACES: u32 = 16;

/// One installed face, the last fallback of every font family.
pub struct Fallback {
    pub name: &'static str,
    pub data: Arc<FontData>,
}

/// The Han glyph forms the interface language asks for.
///
/// A collection such as Noto Sans CJK holds one face per region, mapping the same characters to
/// that region's forms; the preferred face is chosen by family name (see [`face_with`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HanForms {
    /// The first face that fits; every language other than Chinese.
    Default,
    /// The `SC` faces (Simplified Chinese).
    Simplified,
    /// The `TC` faces (Traditional Chinese).
    Traditional,
}

impl HanForms {
    /// The forms for an interface language code (`zh-hans`, `zh-hant`, …).
    pub fn from_code(code: &str) -> Self {
        match code {
            "zh-hans" => Self::Simplified,
            "zh-hant" => Self::Traditional,
            _ => Self::Default,
        }
    }

    /// The family-name markers of the regional faces; empty means "the first face that fits".
    /// YaHei and JhengHei are the Simplified and Traditional faces on Windows, each in a file
    /// of its own.
    fn markers(self) -> &'static [&'static str] {
        match self {
            Self::Default => &[],
            Self::Simplified => &[" SC", "YaHei"],
            Self::Traditional => &[" TC", "JhengHei"],
        }
    }
}

/// One group of characters the embedded faces may lack; the first well-known font that has every
/// probe provides the group's face.
struct Group {
    /// The font families entry: `system-fallback-<script>`.
    entry: &'static str,
    /// Characters the face must map. One character is enough: egui falls back per character, so
    /// a face only helps for what it has.
    probes: &'static [char],
    /// Well-known locations, best first.
    paths: Vec<PathBuf>,
    /// The group is Han text: prefer the interface language's regional face over the first that
    /// fits (kana, Hangul and Arabic have no such variants).
    regional: bool,
}

/// The installed fallback faces for one interface mode, read once per mode.
///
/// Each script group gets the first well-known font that maps its probes, in group order. A
/// group whose characters an earlier face already maps adds nothing, so a face that spans
/// several scripts (Noto Sans CJK, Droid Sans Fallback) is loaded once, not once per script.
/// `forms` picks the regional face of a collection for Han text (Simplified Chinese prefers
/// `SC`, Traditional Chinese `TC`).
pub fn fallbacks(forms: HanForms) -> &'static [Fallback] {
    static DEFAULT: OnceLock<Vec<Fallback>> = OnceLock::new();
    static SIMPLIFIED: OnceLock<Vec<Fallback>> = OnceLock::new();
    static TRADITIONAL: OnceLock<Vec<Fallback>> = OnceLock::new();
    let cache = match forms {
        HanForms::Default => &DEFAULT,
        HanForms::Simplified => &SIMPLIFIED,
        HanForms::Traditional => &TRADITIONAL,
    };
    cache.get_or_init(|| load(forms))
}

fn load(forms: HanForms) -> Vec<Fallback> {
    if std::env::var_os("PDFCRAFT_SYSTEM_FONTS").is_some_and(|v| v == "0") {
        return Vec::new();
    }
    let mut faces: Vec<Fallback> = Vec::new();
    let mut used: Vec<PathBuf> = Vec::new();
    for group in groups() {
        if faces.iter().any(|face| maps_all(&face.data, group.probes)) {
            continue;
        }
        let group_forms = if group.regional { forms } else { HanForms::Default };
        // A regional face is worth reading past files that only carry other regions' forms; the
        // first file that fits at all is the fallback when no file carries the region.
        let mut chosen = None;
        let mut fallback = None;
        for path in &group.paths {
            if used.contains(path) {
                continue;
            }
            let Some((data, preferred)) = read(path, group.probes, group_forms) else {
                continue;
            };
            if preferred {
                chosen = Some((path.clone(), data));
                break;
            }
            fallback.get_or_insert((path.clone(), data));
        }
        if let Some((path, data)) = chosen.or(fallback) {
            log::info!("interface font fallback: {} ({})", path.display(), group.entry);
            used.push(path);
            faces.push(Fallback { name: group.entry, data });
        }
    }
    faces
}

/// Well-known installed font locations per script group, best first.
fn groups() -> Vec<Group> {
    if cfg!(windows) {
        let dir = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")).map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        let fonts = |files: &[&str]| files.iter().map(|f| dir.join("Fonts").join(f)).collect::<Vec<PathBuf>>();
        vec![
            // Microsoft YaHei first; JhengHei and SimSun are the older Chinese defaults.
            Group {
                entry: "system-fallback-han",
                probes: &['漢'],
                paths: fonts(&["msyh.ttc", "msyh.ttf", "msjh.ttc", "simsun.ttc"]),
                regional: true,
            },
            // Meiryo and MS Gothic are the Japanese defaults; YaHei has kana in a pinch.
            Group {
                entry: "system-fallback-kana",
                probes: &['あ'],
                paths: fonts(&["meiryo.ttc", "YuGothM.ttc", "msgothic.ttc", "msyh.ttc"]),
                regional: false,
            },
            Group { entry: "system-fallback-hangul", probes: &['한'], paths: fonts(&["malgun.ttf", "gulim.ttc", "batang.ttc"]), regional: false },
            Group { entry: "system-fallback-arabic", probes: &['ا'], paths: fonts(&["segoeui.ttf", "tahoma.ttf", "arial.ttf"]), regional: false },
            // The full-coverage Indic face Windows 8 and later ships.
            Group { entry: "system-fallback-telugu", probes: &['త'], paths: fonts(&["Nirmala.ttf"]), regional: false },
        ]
    } else if cfg!(target_os = "macos") {
        let paths = |files: &[&str]| files.iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
        vec![
            // PingFang is the system Chinese face; Hiragino Sans GB and Songti cover older setups.
            Group {
                entry: "system-fallback-han",
                probes: &['漢'],
                paths: paths(&[
                    "/System/Library/Fonts/PingFang.ttc",
                    "/System/Library/Fonts/Supplemental/Hiragino Sans GB.ttc",
                    "/System/Library/Fonts/Supplemental/Songti.ttc",
                ]),
                regional: true,
            },
            Group {
                entry: "system-fallback-kana",
                probes: &['あ'],
                paths: paths(&["/System/Library/Fonts/PingFang.ttc", "/System/Library/Fonts/Supplemental/Hiragino Sans GB.ttc"]),
                regional: false,
            },
            Group {
                entry: "system-fallback-hangul",
                probes: &['한'],
                paths: paths(&["/System/Library/Fonts/AppleSDGothicNeo.ttc"]),
                regional: false,
            },
            Group {
                entry: "system-fallback-arabic",
                probes: &['ا'],
                paths: paths(&[
                    "/System/Library/Fonts/SFArabic.ttf",
                    "/System/Library/Fonts/GeezaPro.ttc",
                    "/System/Library/Fonts/Supplemental/Arial.ttf",
                ]),
                regional: false,
            },
            Group {
                entry: "system-fallback-telugu",
                probes: &['త'],
                paths: paths(&["/System/Library/Fonts/Supplemental/Kohinoor Telugu.ttc", "/System/Library/Fonts/Supplemental/Telugu MN.ttc"]),
                regional: false,
            },
        ]
    } else {
        let roots = ["/usr/share/fonts", "/usr/local/share/fonts"];
        let share = |files: &[&str]| roots.iter().flat_map(|root| files.iter().map(move |f| Path::new(root).join(f))).collect::<Vec<PathBuf>>();
        // One collection serves Han and kana alike; its faces differ only in the forms they
        // prefer, and any of them maps the probes.
        let cjk = [
            "opentype/noto/NotoSansCJK-Regular.ttc",   // Debian, Ubuntu
            "noto-cjk/NotoSansCJK-Regular.ttc",        // Arch
            "google-noto-cjk/NotoSansCJK-Regular.ttc", // Fedora
            "truetype/noto/NotoSansCJK-Regular.ttc",
            // Older Ubuntu, one region per file.
            "opentype/noto/NotoSansCJKjp-Regular.otf",
            "opentype/noto/NotoSansCJKtc-Regular.otf",
            "opentype/noto/NotoSansCJKsc-Regular.otf",
            "truetype/droid/DroidSansFallbackFull.ttf", // Debian, Ubuntu
            "truetype/droid/DroidSansFallback.ttf",
            "truetype/wqy/wqy-zenhei.ttc",
            "truetype/wqy/wqy-microhei.ttc",
            "truetype/arphic/uming.ttc",
        ];
        let korean = [
            "truetype/noto/NotoSansKR-Regular.ttf",
            "opentype/noto/NotoSansKR-Regular.otf",
            "truetype/nanum/NanumGothic.ttf",
            "truetype/unfonts-core/UnDotum.ttf",
        ];
        let arabic = [
            "truetype/noto/NotoSansArabic-Regular.ttf",
            "noto/NotoSansArabic-Regular.ttf",
            "google-noto/NotoSansArabic-Regular.ttf",
            "truetype/dejavu/DejaVuSans.ttf",
            "TTF/DejaVuSans.ttf",
            "dejavu/DejaVuSans.ttf",
            "dejavu-sans-fonts/DejaVuSans.ttf",
        ];
        let telugu = [
            "truetype/noto/NotoSansTelugu-Regular.ttf",
            "noto/NotoSansTelugu-Regular.ttf",
            "google-noto/NotoSansTelugu-Regular.ttf",
            "truetype/lohit-telugu/Lohit-Telugu.ttf",
            "truetype/samyak/Samyak-Telugu.ttf",
        ];
        vec![
            Group { entry: "system-fallback-han", probes: &['漢'], paths: share(&cjk), regional: true },
            Group { entry: "system-fallback-kana", probes: &['あ'], paths: share(&cjk), regional: false },
            Group { entry: "system-fallback-hangul", probes: &['한'], paths: share(&korean), regional: false },
            Group { entry: "system-fallback-arabic", probes: &['ا'], paths: share(&arabic), regional: false },
            Group { entry: "system-fallback-telugu", probes: &['త'], paths: share(&telugu), regional: false },
        ]
    }
}

/// Reads the file and picks the face for `probes`: `Some((data, preferred))`, where `preferred`
/// is false when only a face of another region fits (the caller may keep looking).
fn read(path: &Path, probes: &[char], forms: HanForms) -> Option<(Arc<FontData>, bool)> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let (index, preferred) = face_with(&bytes, probes, forms)?;
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    Some((Arc::new(data), preferred))
}

/// The first face of the file that parses and maps every probe, and whether it is one of
/// `forms`' regional faces. A face whose family name carries a marker wins over an earlier fit:
/// Noto Sans CJK holds JP, KR, SC, TC and HK faces that map the same characters to forms of
/// different regions, and Simplified or Traditional Chinese prefers its own.
fn face_with(bytes: &[u8], probes: &[char], forms: HanForms) -> Option<(u32, bool)> {
    let mut first = None;
    let markers = forms.markers();
    for index in 0..MAX_FACES {
        let Ok(font) = skrifa::FontRef::from_index(bytes, index) else {
            break;
        };
        let charmap = font.charmap();
        if !probes.iter().all(|c| charmap.map(*c).is_some()) {
            continue;
        }
        if markers.is_empty() {
            return Some((index, true));
        }
        if family_name(&font).is_some_and(|name| markers.iter().any(|marker| name.contains(marker))) {
            return Some((index, true));
        }
        first.get_or_insert(index);
    }
    first.map(|index| (index, false))
}

/// The English family name of a face (`Noto Sans CJK SC`).
fn family_name(font: &skrifa::FontRef<'_>) -> Option<String> {
    use skrifa::string::StringId;
    font.localized_strings(StringId::FAMILY_NAME).english_or_first().map(|name| name.to_string())
}

/// Whether the face's chosen collection index maps every probe.
fn maps_all(data: &FontData, probes: &[char]) -> bool {
    skrifa::FontRef::from_index(&data.font, data.index).is_ok_and(|font| {
        let charmap = font.charmap();
        probes.iter().all(|c| charmap.map(*c).is_some())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_and_missing_files_are_skipped() {
        assert_eq!(face_with(b"", &['A'], HanForms::Default), None);
        assert_eq!(face_with(b"not a font at all", &['A'], HanForms::Default), None);
        assert_eq!(face_with(&[0u8; 4096], &['A'], HanForms::Default), None);
        assert!(read(Path::new("definitely/not/here.ttf"), &['A'], HanForms::Default).is_none());
        // A directory is not a font.
        assert!(read(&std::env::temp_dir(), &['A'], HanForms::Default).is_none());
    }

    #[test]
    fn a_face_without_the_probes_is_rejected() {
        // Inter is Latin, Greek and Cyrillic only.
        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        assert_eq!(face_with(inter, &['ا'], HanForms::Default), None);
        assert_eq!(face_with(inter, &['漢'], HanForms::Default), None);
        assert_eq!(face_with(inter, &['A'], HanForms::Default), Some((0, true)));
    }

    #[test]
    fn regional_markers_fall_back_to_the_first_fit() {
        // Inter carries no regional marker, so a Chinese preference keeps the first fit but
        // reports it unpreferred; the default preference reports the first fit as chosen.
        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        assert_eq!(face_with(inter, &['A'], HanForms::Simplified), Some((0, false)));
        assert_eq!(face_with(inter, &['A'], HanForms::Traditional), Some((0, false)));
        assert_eq!(face_with(inter, &['A'], HanForms::Default), Some((0, true)));
    }

    #[test]
    fn language_codes_map_to_forms() {
        assert_eq!(HanForms::from_code("zh-hans"), HanForms::Simplified);
        assert_eq!(HanForms::from_code("zh-hant"), HanForms::Traditional);
        assert_eq!(HanForms::from_code("ja"), HanForms::Default);
        assert_eq!(HanForms::from_code("en"), HanForms::Default);
    }

    #[test]
    fn maps_all_reads_the_chosen_face() {
        let data = FontData::from_static(include_bytes!("../../../assets/fonts/Inter-Regular.ttf"));
        assert!(maps_all(&data, &['A']));
        assert!(!maps_all(&data, &['漢']));
    }

    #[test]
    fn groups_are_absolute_font_files_under_the_prefix() {
        let groups = groups();
        assert_eq!(groups.len(), 5);
        assert_eq!(groups.iter().filter(|group| group.regional).count(), 1);
        for group in groups {
            assert!(group.entry.starts_with(PREFIX), "{}", group.entry);
            assert!(!group.paths.is_empty(), "{}", group.entry);
            assert!(
                group.paths.iter().all(|p| p.is_absolute() && p.extension().is_some_and(|e| e == "ttf" || e == "ttc" || e == "otf")),
                "{}",
                group.entry
            );
            assert!(group.probes.iter().all(|c| !c.is_ascii()), "{}", group.entry);
        }
    }
}
