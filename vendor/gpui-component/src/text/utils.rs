use gpui::{ListOffset, ListState, Pixels, px};

pub(super) fn scroll_reading_list(list: &ListState, key: &str, page: Pixels) -> bool {
    match key {
        "up" => list.scroll_by(-px(24.)),
        "down" => list.scroll_by(px(24.)),
        "pageup" => list.scroll_by(-page),
        "pagedown" | "space" => list.scroll_by(page),
        "home" => list.scroll_to(ListOffset::default()),
        "end" => list.scroll_to(ListOffset {
            item_ix: list.item_count(),
            offset_in_item: px(0.),
        }),
        _ => return false,
    }
    true
}

const NUMBERED_PREFIXES_1: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const NUMBERED_PREFIXES_2: &str = "abcdefghijklmnopqrstuvwxyz";

const BULLETS: [&str; 5] = ["▪", "•", "◦", "‣", "⁃"];

/// Returns the prefix for a list item.
pub(super) fn list_item_prefix(ix: usize, start: Option<u64>, depth: usize) -> String {
    if let Some(start) = start {
        if depth == 0 || start != 1 {
            return format!("{}. ", start + ix as u64);
        }

        if depth == 1 {
            return format!(
                "{}. ",
                NUMBERED_PREFIXES_1
                    .chars()
                    .nth(ix % NUMBERED_PREFIXES_1.len())
                    .unwrap()
            );
        } else {
            return format!(
                "{}. ",
                NUMBERED_PREFIXES_2
                    .chars()
                    .nth(ix % NUMBERED_PREFIXES_2.len())
                    .unwrap()
            );
        }
    } else {
        let depth = depth.min(BULLETS.len() - 1);
        let bullet = BULLETS[depth];
        return format!("{} ", bullet);
    }
}

#[cfg(test)]
mod tests {
    use crate::text::utils::list_item_prefix;

    #[test]
    fn test_list_item_prefix() {
        assert_eq!(list_item_prefix(0, Some(1), 0), "1. ");
        assert_eq!(list_item_prefix(1, Some(1), 0), "2. ");
        assert_eq!(list_item_prefix(2, Some(1), 0), "3. ");
        assert_eq!(list_item_prefix(10, Some(1), 0), "11. ");
        assert_eq!(list_item_prefix(0, Some(7), 0), "7. ");
        assert_eq!(list_item_prefix(1, Some(7), 0), "8. ");
        assert_eq!(list_item_prefix(0, Some(0), 1), "0. ");
        assert_eq!(list_item_prefix(1, Some(7), 2), "8. ");
        assert_eq!(list_item_prefix(0, Some(1), 1), "A. ");
        assert_eq!(list_item_prefix(1, Some(1), 1), "B. ");
        assert_eq!(list_item_prefix(2, Some(1), 1), "C. ");
        assert_eq!(list_item_prefix(0, Some(1), 2), "a. ");
        assert_eq!(list_item_prefix(1, Some(1), 2), "b. ");
        assert_eq!(list_item_prefix(6, Some(1), 2), "g. ");
        assert_eq!(list_item_prefix(0, Some(1), 1), "A. ");
        assert_eq!(list_item_prefix(0, Some(1), 2), "a. ");
        assert_eq!(list_item_prefix(0, None, 0), "▪ ");
        assert_eq!(list_item_prefix(0, None, 1), "• ");
        assert_eq!(list_item_prefix(0, None, 2), "◦ ");
        assert_eq!(list_item_prefix(0, None, 3), "‣ ");
        assert_eq!(list_item_prefix(0, None, 4), "⁃ ");
    }
}
