//! Fuzzy matching for the palettes: the typed letters in order, anywhere in the text, scored so
//! that matches in a file's name, at the start of words and in runs come first.

/// How well `query` matches `text`, higher is better, or `None` when its letters aren't all in
/// `text` in order. Case is ignored; spaces in the query are ignored too.
pub fn score(query: &str, text: &str) -> Option<i64> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    // Where the last part of a path starts: matches there count most.
    let name_start = chars.iter().rposition(|c| *c == '/').map_or(0, |at| at + 1);

    // Match each letter at its best place greedily, from the end, so a file's name wins over the
    // same letters in a folder; then score the positions.
    let mut positions = Vec::with_capacity(query.len());
    let mut at = lower.len();
    for wanted in query.iter().rev() {
        let found = lower[..at].iter().rposition(|c| c == wanted)?;
        positions.push(found);
        at = found;
    }
    positions.reverse();

    let starts_word = |at: usize| {
        at == 0
            || matches!(chars[at - 1], '/' | '_' | '-' | '.' | ' ')
            || (chars[at].is_uppercase() && chars[at - 1].is_lowercase())
    };
    let mut total: i64 = 0;
    for (index, &at) in positions.iter().enumerate() {
        total += 1;
        if at >= name_start {
            total += 4;
        }
        if starts_word(at) {
            total += 6;
        }
        if index > 0 && positions[index - 1] + 1 == at {
            total += 5;
        } else if index > 0 {
            // A jump: letters scattered across a path say little.
            total -= 3;
        }
    }
    if positions.first() == Some(&name_start) {
        total += 8;
    }
    // Shorter texts first, among equals.
    total -= i64::try_from(chars.len()).unwrap_or(i64::MAX) / 8;
    Some(total)
}

/// The indices of `texts` that match `query`, best first, at most `limit`. Matches scoring under
/// half the best are left out: they only share scattered letters.
pub fn rank<'a>(query: &str, texts: impl IntoIterator<Item = &'a str>, limit: usize) -> Vec<usize> {
    let mut scored: Vec<(i64, usize)> = texts
        .into_iter()
        .enumerate()
        .filter_map(|(index, text)| score(query, text).map(|score| (score, index)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let floor = scored.first().map_or(0, |(best, _)| if *best > 0 { best / 2 } else { i64::MIN });
    scored.into_iter().take_while(|(score, _)| *score >= floor).take(limit).map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_every_letter_in_order() {
        assert!(score("tax", "packages/money/src/tax.ts").is_some());
        assert!(score("xat", "packages/money/src/tax.ts").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn a_files_name_beats_its_folders() {
        let files = [
            "apps/web/src/cart/Cart.tsx",
            "apps/web/src/cart/total.ts",
            "packages/money/src/tax.ts",
            "apps/api/src/orders.ts",
            "apps/web/src/checkout/Checkout.tsx",
        ];
        let best = |query: &str| files[rank(query, files, 1)[0]];
        assert_eq!(best("tax"), "packages/money/src/tax.ts");
        assert_eq!(best("cart"), "apps/web/src/cart/Cart.tsx");
        assert_eq!(best("chk"), "apps/web/src/checkout/Checkout.tsx");
        assert_eq!(best("api ord"), "apps/api/src/orders.ts");
    }

    #[test]
    fn scattered_letters_are_left_out() {
        let files = [
            "apps/web/src/cart/Cart.tsx",
            "apps/web/src/cart/total.ts",
            "packages/catalog/src/index.ts",
            "packages/money/src/format.ts",
        ];
        let found: Vec<&str> = rank("cart", files, 10).into_iter().map(|index| files[index]).collect();
        assert_eq!(found, ["apps/web/src/cart/Cart.tsx", "apps/web/src/cart/total.ts"]);
    }

    #[test]
    fn word_starts_find_commands() {
        let commands = ["Split right", "Show history", "Toggle sidebar", "Settings"];
        assert_eq!(commands[rank("sh", commands, 1)[0]], "Show history");
        assert_eq!(commands[rank("tsid", commands, 1)[0]], "Toggle sidebar");
    }
}
