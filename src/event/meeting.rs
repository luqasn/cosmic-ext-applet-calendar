// SPDX-License-Identifier: GPL-3.0-only

//! Recognizes online meeting links that can be joined from the popup.
//!
//! Only links to the services in [`MEETING_PROVIDERS`] are treated as joinable meetings.
//! To support another service, add an entry to that list.

use crate::event::is_safe_web_url;

/// A video conferencing service whose links can be joined from the popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeetingProvider {
    pub name: &'static str,
    /// Exact host name of the service's meeting links, compared case-insensitively.
    pub host: &'static str,
}

/// Whitelist of services whose meeting links get a join button.
pub const MEETING_PROVIDERS: &[MeetingProvider] = &[MeetingProvider {
    name: "Google Meet",
    host: "meet.google.com",
}];

/// Returns the provider for `raw_url` if it is an HTTPS link to a whitelisted
/// service that points at a specific meeting (not just the service's home page).
pub fn meeting_provider(raw_url: &str) -> Option<&'static MeetingProvider> {
    if !is_safe_web_url(raw_url) {
        return None;
    }
    let parsed = url::Url::parse(raw_url.trim()).ok()?;
    if parsed.scheme() != "https" || parsed.path().trim_matches('/').is_empty() {
        return None;
    }
    let host = parsed.host_str()?;
    MEETING_PROVIDERS
        .iter()
        .find(|provider| host.eq_ignore_ascii_case(provider.host))
}

/// Finds the first whitelisted meeting link in free text such as an event description.
/// Handles links wrapped in punctuation, quotes or HTML markup.
pub fn find_meeting_url(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || "\"'<>()[]{},;".contains(c))
        .map(|word| word.trim_end_matches(['.', '!', '?', ':']))
        .find(|word| meeting_provider(word).is_some())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google_meet_is_whitelisted() {
        let provider = meeting_provider("https://meet.google.com/abc-defg-hij").unwrap();
        assert_eq!(provider.name, "Google Meet");
        assert!(meeting_provider("https://MEET.google.com/abc-defg-hij?authuser=0").is_some());
    }

    #[test]
    fn test_non_meeting_links_are_rejected() {
        // Home page without a meeting code.
        assert!(meeting_provider("https://meet.google.com/").is_none());
        // Not HTTPS.
        assert!(meeting_provider("http://meet.google.com/abc-defg-hij").is_none());
        // Look-alike hosts.
        assert!(meeting_provider("https://meet.google.com.evil.example/abc").is_none());
        assert!(meeting_provider("https://evil.example/meet.google.com/abc").is_none());
        // Services not (yet) on the whitelist.
        assert!(meeting_provider("https://zoom.us/j/123456789").is_none());
        assert!(meeting_provider("https://teams.microsoft.com/l/meetup-join/123").is_none());
    }

    #[test]
    fn test_find_meeting_url_in_text() {
        assert_eq!(
            find_meeting_url("Join at https://meet.google.com/abc-defg-hij. See you!").as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(
            find_meeting_url(
                "<a href=\"https://meet.google.com/abc-defg-hij\">Join with Google Meet</a>"
            )
            .as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(
            find_meeting_url("Docs: https://example.com/doc, call: https://zoom.us/j/1"),
            None
        );
    }
}
