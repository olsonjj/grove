//! The THEME dropdown's data: the SYSTEM group's Follow System plus the registry's
//! themes, grouped SYSTEM/LIGHT/DARK, searched case-insensitively, with Grove-drawn
//! rows for the VoiceOver labels (spec story 14) and Grove-styled section headers
//! the component's generic rendering doesn't carry.

use gpui_kit::component::IndexPath;
use gpui_kit::component::searchable_list::{SearchableListDelegate, SearchableListItem};
use gpui_kit::component::theme::{ThemeMode, ThemeRegistry};
use gpui_kit::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Task, TestSupportExt, Window, div, px,
};

use grove_core::ThemeChoice;

use crate::{icons, theme};

/// The SYSTEM group's single fixed entry: Follow System. It is also the row's
/// stored value — it is no registry name, so a pick can never read as a theme, and
/// themes are searchable with it (typing "system" finds it).
pub(crate) const FOLLOW_SYSTEM: &str = "Follow System";

/// The row's kind: Follow System, or a theme by its registry name.
pub(crate) fn choice_for(value: &str) -> ThemeChoice {
    if value == FOLLOW_SYSTEM {
        ThemeChoice::FollowSystem
    } else {
        ThemeChoice::Name(value.to_string())
    }
}

/// One theme row in the picker: the registry name — what the setting stores for a
/// theme — plus the mode the LIGHT/DARK grouping hangs off. `None` is Follow
/// System's row, which has no mode of its own: what it looks like tracks macOS
/// (spec story 13), so claiming light or dark would go stale when the Mac switches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeOffer {
    name: SharedString,
    mode: Option<ThemeMode>,
}

impl SearchableListItem for ThemeOffer {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.name
    }
}

/// One dropdown section: a header title and its themes, in display order.
#[derive(Debug, Clone)]
pub struct ThemeSection {
    title: &'static str,
    items: Vec<ThemeOffer>,
}

impl ThemeSection {
    fn new(title: &'static str, items: Vec<ThemeOffer>) -> Self {
        Self { title, items }
    }
}

/// The dropdown's sections: SYSTEM first with its single fixed entry (Follow
/// System), then LIGHT and DARK populated from the registry in `sorted_themes()`
/// order (defaults first, light before dark, case-insensitive by name), which the
/// groups preserve. The ticket-34 pinned contract — the SYSTEM group shows as soon
/// as it is filled — now holds everywhere, and it is: first.
pub fn sections(cx: &App) -> Vec<ThemeSection> {
    let system = ThemeSection::new(
        "SYSTEM",
        vec![ThemeOffer {
            name: FOLLOW_SYSTEM.into(),
            mode: None,
        }],
    );
    let mut light = ThemeSection::new("LIGHT", Vec::new());
    let mut dark = ThemeSection::new("DARK", Vec::new());
    for config in ThemeRegistry::global(cx).sorted_themes() {
        let offer = ThemeOffer {
            name: config.name.clone(),
            mode: Some(config.mode),
        };
        if config.mode.is_dark() {
            dark.items.push(offer);
        } else {
            light.items.push(offer);
        }
    }
    vec![system, light, dark]
}

/// The rows of one kept section: the section's index and its matched rows.
#[derive(Debug, Clone)]
struct Matched {
    section: usize,
    rows: Vec<usize>,
}

/// The picker's delegate over [`sections`]: the full list plus the matched view
/// the search query rebuilds, in the component's searchable-list protocol.
#[derive(Debug, Clone)]
pub struct ThemeMenu {
    sections: Vec<ThemeSection>,
    matched: Vec<Matched>,
}

impl ThemeMenu {
    pub fn new(sections: Vec<ThemeSection>) -> Self {
        let mut menu = Self {
            sections,
            matched: Vec::new(),
        };
        menu.refresh("");
        menu
    }

    /// Rebuilds the matched view. Sections keep their item order. A section the
    /// query filters to zero rows is hidden outright — the component's row cache
    /// hides any zero-row section, title match or not (its `rows_cache` simply
    /// emits no entries for empty counts), so keeping one here would only put
    /// index paths behind what actually renders. The same rule drops the SYSTEM
    /// group on a query that doesn't match Follow System.
    fn refresh(&mut self, query: &str) {
        let query = query.to_lowercase();
        self.matched = self
            .sections
            .iter()
            .enumerate()
            .filter_map(|(ix, section)| {
                if section.items.is_empty() {
                    return None;
                }
                let rows: Vec<usize> = section
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.matches(&query))
                    .map(|(row, _)| row)
                    .collect();
                (!rows.is_empty()).then_some(Matched { section: ix, rows })
            })
            .collect();
    }

    /// The row index counted across every kept section, so row test ids stay
    /// unique per render (the component's own row ids repeat per section).
    fn flat_row(&self, ix: IndexPath) -> usize {
        self.matched[..ix.section]
            .iter()
            .map(|m| m.rows.len())
            .sum::<usize>()
            + ix.row
    }
}

impl SearchableListDelegate for ThemeMenu {
    type Item = ThemeOffer;

    fn sections_count(&self, _: &App) -> usize {
        self.matched.len()
    }

    fn items_count(&self, section: usize) -> usize {
        self.matched.get(section).map_or(0, |m| m.rows.len())
    }

    fn item(&self, ix: IndexPath) -> Option<&ThemeOffer> {
        let matched = self.matched.get(ix.section)?;
        let row = matched.rows.get(ix.row)?;
        self.sections.get(matched.section)?.items.get(*row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        for (section, matched) in self.matched.iter().enumerate() {
            for (row, item_row) in matched.rows.iter().enumerate() {
                if self.sections[matched.section].items[*item_row].value() == value {
                    return Some(IndexPath::default().section(section).row(row));
                }
            }
        }
        None
    }

    fn perform_search(&mut self, query: &str, _: &mut Window, _: &mut App) -> Task<()> {
        self.refresh(query);
        Task::ready(())
    }

    fn render_item(
        &self,
        ix: IndexPath,
        item: &ThemeOffer,
        checked: bool,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let palette = theme::palette(cx);
        // The row stands on its own for VoiceOver (spec story 14): name, mode, and
        // "selected" when it's the committed choice — the settings choices' pattern.
        let label = match item.mode {
            // A named theme claims its light|dark.
            Some(mode) => format!(
                "{}, {}{}",
                item.name,
                if mode.is_dark() { "dark" } else { "light" },
                if checked { ", selected" } else { "" }
            ),
            // Follow System has no light|dark of its own — what the row looks like
            // at this instant would go stale when the Mac switches — so the label
            // says what the row does, keeping VoiceOver's value stable either way.
            None => format!(
                "{}, follows system appearance{}",
                item.name,
                if checked { ", selected" } else { "" }
            ),
        };
        let flat = self.flat_row(ix);
        Some(
            div()
                .id(("theme-row", flat))
                .test_support()
                .aria_label(label)
                .flex()
                .w_full()
                .min_w_0()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .child(item.name.clone()),
                )
                .children(checked.then(|| {
                    div()
                        .id(("theme-check", flat))
                        .flex_none()
                        .child(icons::icon(icons::CHECK, 12.).text_color(palette.text()))
                }))
                .into_any_element(),
        )
    }

    fn render_section_header(
        &self,
        section: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let matched = self.matched.get(section)?;
        let title = self.sections.get(matched.section)?.title;
        let palette = theme::palette(cx);
        Some(
            div()
                .id(("theme-section", section))
                .test_support()
                .aria_label(title)
                // The section headers in the settings body: the label() pattern.
                .text_size(px(11.))
                .text_color(palette.faint())
                .child(title)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(name: &str, mode: ThemeMode) -> ThemeOffer {
        ThemeOffer {
            name: name.into(),
            mode: Some(mode),
        }
    }

    fn follow() -> ThemeOffer {
        ThemeOffer {
            name: FOLLOW_SYSTEM.into(),
            mode: None,
        }
    }

    /// SYSTEM (Follow System, fixed first entry), then a light and a dark section.
    fn menu() -> ThemeMenu {
        ThemeMenu::new(vec![
            ThemeSection::new("SYSTEM", vec![follow()]),
            ThemeSection::new(
                "LIGHT",
                vec![
                    offer("Ayu Light", ThemeMode::Light),
                    offer("Zune Light", ThemeMode::Light),
                ],
            ),
            ThemeSection::new(
                "DARK",
                vec![
                    offer("Default Dark", ThemeMode::Dark),
                    offer("Grove Dark", ThemeMode::Dark),
                ],
            ),
        ])
    }

    /// Names in the matched view, row by row, section by section.
    fn rows(menu: &ThemeMenu) -> Vec<String> {
        (0..menu.matched.len())
            .flat_map(|section| {
                (0..menu.items_count(section)).map(move |row| {
                    menu.item(IndexPath::default().section(section).row(row))
                        .expect("matched row")
                        .name
                        .to_string()
                })
            })
            .collect()
    }

    #[test]
    fn the_search_drops_a_section_whose_rows_match_nothing() {
        // A query that matches nothing in SYSTEM keeps the group out of the list;
        // the component hides any zero-row section, so the row count and index
        // paths behind what renders stay in step.
        let mut menu = menu();
        menu.refresh("gro");
        assert_eq!(rows(&menu), ["Grove Dark"]);
        menu.refresh("light");
        assert_eq!(rows(&menu), ["Ayu Light", "Zune Light"]);
    }

    #[test]
    fn a_filled_system_group_shows_first() {
        let menu = ThemeMenu::new(vec![
            ThemeSection::new("SYSTEM", vec![follow()]),
            ThemeSection::new("DARK", vec![offer("Grove Dark", ThemeMode::Dark)]),
        ]);
        assert_eq!(rows(&menu), ["Follow System", "Grove Dark"]);
        assert_eq!(
            menu.position(&SharedString::from(FOLLOW_SYSTEM)),
            Some(IndexPath::default())
        );
    }

    #[test]
    fn the_query_filters_names_case_insensitively() {
        let mut menu = menu();
        menu.refresh("gro");
        assert_eq!(rows(&menu), ["Grove Dark"]);
        assert_eq!(
            menu.position(&SharedString::from("Grove Dark")),
            Some(IndexPath::default())
        );
        menu.refresh("AYU");
        assert_eq!(rows(&menu), ["Ayu Light"]);
        // Follow System is searchable with the themes.
        menu.refresh("system");
        assert_eq!(rows(&menu), [FOLLOW_SYSTEM]);
        assert_eq!(
            menu.position(&SharedString::from(FOLLOW_SYSTEM)),
            Some(IndexPath::default())
        );
    }

    #[test]
    fn positions_answer_for_the_matched_view() {
        let mut menu = menu();
        menu.refresh("light");
        assert_eq!(
            menu.position(&SharedString::from("Zune Light")),
            Some(IndexPath::default().section(0).row(1))
        );
        // A name the view filtered out has no position, like a hidden row.
        assert_eq!(menu.position(&SharedString::from("Grove Dark")), None);
    }
}

/// The registry grouping under the real init path: SYSTEM first with its Follow
/// System row, then LIGHT before DARK, each group exactly in `sorted_themes()`
/// order — the ticket-34 pinned contract, filled.
#[gpui_kit::test]
fn registry_sections_keep_sorted_themes_order(cx: &mut gpui_kit::TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        let menu = ThemeMenu::new(sections(cx));
        assert_eq!(menu.sections_count(cx), 3, "SYSTEM is first and filled");
        assert_eq!(menu.items_count(0), 1);
        assert!(menu.items_count(1) > 0 && menu.items_count(2) > 0);
        let names = |section: usize| {
            (0..menu.items_count(section))
                .map(|row| {
                    menu.item(IndexPath::default().section(section).row(row))
                        .unwrap()
                        .name
                        .clone()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(names(0), [FOLLOW_SYSTEM], "SYSTEM holds only Follow System");
        let registry = ThemeRegistry::global(cx);
        let expect = |dark: bool| {
            registry
                .sorted_themes()
                .iter()
                .filter(|config| config.mode.is_dark() == dark)
                .map(|config| config.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(1), expect(false), "LIGHT keeps sorted_themes order");
        assert_eq!(names(2), expect(true), "DARK keeps sorted_themes order");
        assert!(names(2).iter().any(|name| name == "Grove Dark"));
    });
}
