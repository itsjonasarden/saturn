//! When Akyba names a Claude session after a ticket: `/start abc-224` makes the session
//! `ABC-224`, the same as typing `/rename ABC-224`. Only a slash command whose first word is a
//! ticket counts, so tickets mentioned in normal prompts never rename a session.

/// The ticket a prompt's slash command is about: `/start abc-224 now` → `ABC-224`.
pub fn ticket_from_command(prompt: &str) -> Option<String> {
    let mut words = prompt.split_whitespace();
    let command = words.next()?;
    if !command.starts_with('/') || command.len() < 2 {
        return None;
    }
    let ticket = words.next()?;
    let (prefix, number) = ticket.split_once('-')?;
    let valid = prefix.len() >= 2
        && prefix.chars().all(|c| c.is_ascii_alphanumeric())
        && prefix.starts_with(|c: char| c.is_ascii_alphabetic())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit());
    valid.then(|| ticket.to_ascii_uppercase())
}

/// The new session name for a prompt, if Akyba should rename the session.
///
/// - `current`: the session's custom name now (from the hook input), if it has one.
/// - `named_by_akyba`: the last name Akyba gave this session, if any.
///
/// A name the user gave (with `/rename`) is never overwritten; a name Akyba gave is replaced
/// when a command names a different ticket.
pub fn rename_for_prompt(prompt: &str, current: Option<&str>, named_by_akyba: Option<&str>) -> Option<String> {
    let ticket = ticket_from_command(prompt)?;
    match current {
        Some(name) if name == ticket => None,
        Some(name) if Some(name) != named_by_akyba => None,
        _ => Some(ticket),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_ticket_from_a_slash_command() {
        assert_eq!(ticket_from_command("/start abc-224").as_deref(), Some("ABC-224"));
        assert_eq!(ticket_from_command("/plan ABC-254 please").as_deref(), Some("ABC-254"));
        assert_eq!(ticket_from_command("/akyba:build wms-12").as_deref(), Some("WMS-12"));
    }

    #[test]
    fn ignores_prompts_that_are_not_a_ticket_command() {
        assert_eq!(ticket_from_command("review abc-223 and its PR"), None);
        assert_eq!(ticket_from_command("/start"), None);
        assert_eq!(ticket_from_command("/compact keep-it-short"), None);
        assert_eq!(ticket_from_command("/pr 221"), None);
    }

    #[test]
    fn any_ticket_shaped_first_word_counts() {
        // A known edge, accepted: telling `UTF-8` from `ABC-1` would need guessing.
        assert_eq!(ticket_from_command("/explain UTF-8").as_deref(), Some("UTF-8"));
    }

    #[test]
    fn names_an_unnamed_session() {
        assert_eq!(rename_for_prompt("/start abc-224", None, None).as_deref(), Some("ABC-224"));
    }

    #[test]
    fn never_overwrites_a_name_the_user_gave() {
        assert_eq!(rename_for_prompt("/start abc-224", Some("my-session"), None), None);
        assert_eq!(rename_for_prompt("/start abc-224", Some("my-session"), Some("ABC-100")), None);
    }

    #[test]
    fn replaces_its_own_name_for_a_new_ticket() {
        assert_eq!(rename_for_prompt("/start abc-300", Some("ABC-224"), Some("ABC-224")).as_deref(), Some("ABC-300"));
    }

    #[test]
    fn leaves_a_session_already_named_after_the_ticket() {
        assert_eq!(rename_for_prompt("/start abc-224", Some("ABC-224"), None), None);
    }
}
