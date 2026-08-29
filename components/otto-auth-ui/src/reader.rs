//! Reading a fingerprint module's messages.
//!
//! `pam_fprintd` phrases its own side of a fingerprint login, and both clients
//! show what it says. Two of its messages should not be shown as they arrive:
//! its request for a finger, and its report of one it did not recognise.
//!
//! Both are said in the module's process locale, which is not the panel's — a
//! lock screen's stack and a greeter's greetd both run with an environment of
//! their own, and greetd's is usually bare. An Italian card reading "Place
//! your right index finger on Elan Fingerprint Sensor" is the reader talking
//! past the person in front of it.
//!
//! What those two messages mean is a choice from a fixed table rather than
//! free text, so they can be recognised and said again from Otto's catalogues.
//! Everything else the module volunteers — a swipe too short, a finger left on
//! the reader — is guidance written for the moment it happens, and keeps the
//! module's own words.
//!
//! This lives beside the panel because both clients need it and neither owns
//! it: the crate draws what the two of them share, and what the reader is
//! asking for is part of that.

/// One of the ten fingers a reader can ask for by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerName {
    LeftThumb,
    LeftIndex,
    LeftMiddle,
    LeftRing,
    LeftLittle,
    RightThumb,
    RightIndex,
    RightMiddle,
    RightRing,
    RightLittle,
}

impl FingerName {
    /// The catalogue key naming this finger. Shared by both clients: a finger
    /// is a finger whether it is unlocking or logging in.
    fn key(self) -> &'static str {
        match self {
            Self::LeftThumb => "auth-finger-left-thumb",
            Self::LeftIndex => "auth-finger-left-index",
            Self::LeftMiddle => "auth-finger-left-middle",
            Self::LeftRing => "auth-finger-left-ring",
            Self::LeftLittle => "auth-finger-left-little",
            Self::RightThumb => "auth-finger-right-thumb",
            Self::RightIndex => "auth-finger-right-index",
            Self::RightMiddle => "auth-finger-right-middle",
            Self::RightRing => "auth-finger-right-ring",
            Self::RightLittle => "auth-finger-right-little",
        }
    }
}

/// What the reader is asking for: a touch or a swipe, and which finger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FingerRequest {
    /// Swipe readers say "swipe"; the common press readers say "place".
    pub swipe: bool,
    /// `None` when the module asked for a finger without naming one.
    pub finger: Option<FingerName>,
}

/// Whether a message is about a fingerprint reader at all.
///
/// The wording varies with locale and reader, so this matches loosely — it
/// only decides whether the Touch ID mark is shown, and being wrong costs a
/// mark, not a login.
pub fn mentions_fingerprint(message: &str) -> bool {
    let message = message.to_lowercase();
    ["finger", "fprint", "biometric"]
        .iter()
        .any(|needle| message.contains(needle))
}

/// Recognise a request for a finger, so the panel can say it in the user's
/// language.
pub fn finger_request(message: &str) -> Option<FingerRequest> {
    let message = message.to_lowercase();
    // "Scan" is what the module says for readers that are neither.
    let swipe = message.contains("swipe");
    if !swipe && !message.contains("place") && !message.contains("scan") {
        return None;
    }
    if !mentions_fingerprint(&message) {
        return None;
    }

    // Longest first: "left index finger" must not be read as a bare "index".
    const NAMES: [(&str, FingerName); 10] = [
        ("left thumb", FingerName::LeftThumb),
        ("left index", FingerName::LeftIndex),
        ("left middle", FingerName::LeftMiddle),
        ("left ring", FingerName::LeftRing),
        ("left little", FingerName::LeftLittle),
        ("right thumb", FingerName::RightThumb),
        ("right index", FingerName::RightIndex),
        ("right middle", FingerName::RightMiddle),
        ("right ring", FingerName::RightRing),
        ("right little", FingerName::RightLittle),
    ];
    let finger = NAMES
        .iter()
        .find(|(needle, _)| message.contains(needle))
        .map(|(_, finger)| *finger);

    Some(FingerRequest { swipe, finger })
}

/// Whether an error is the reader saying it did not recognise the finger.
///
/// This one is worth replacing: it is the commonest thing a reader ever says,
/// and it carries nothing the user could act on that the catalogue cannot say
/// better. A swipe too short is a different matter, and is left alone.
pub fn is_no_match(message: &str) -> bool {
    let message = message.to_lowercase();
    mentions_fingerprint(&message)
        && [
            "match",
            "not recognized",
            "not recognised",
            "verification failed",
        ]
        .iter()
        .any(|needle| message.contains(needle))
}

/// The request for a finger, in the panel's language.
///
/// `client` names the catalogue's own family of status lines — `"lock"` or
/// `"greeter"`. The two say the same thing in the same words today, but they
/// are separate keys because the two screens are separate places, and a
/// translator should be free to phrase locking and logging in differently.
///
/// The reader's model name is dropped. It is hardware's name, it is never
/// translated, and the card has one clipped line to say this in.
pub fn request_line(request: FingerRequest, client: &str) -> String {
    let Some(finger) = request.finger else {
        let key = match request.swipe {
            true => format!("{client}-status-swipe-finger"),
            false => format!("{client}-status-place-finger"),
        };
        return otto_kit::i18n::lookup(&key, None).into_owned();
    };

    let finger = otto_kit::t!(finger.key());
    let key = match request.swipe {
        true => format!("{client}-status-swipe-named-finger"),
        false => format!("{client}-status-place-named-finger"),
    };
    let args =
        otto_kit::i18n::args_from(vec![("finger", otto_kit::i18n::FluentValue::from(finger))]);
    otto_kit::i18n::lookup(&key, Some(&args)).into_owned()
}

/// The reader's report of a finger it did not recognise, in the panel's
/// language. `client` as in [`request_line`].
pub fn no_match_line(client: &str) -> String {
    otto_kit::i18n::lookup(&format!("{client}-status-no-match"), None).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_hints_are_recognised_however_they_are_worded() {
        assert!(mentions_fingerprint("Place your finger on the reader"));
        assert!(mentions_fingerprint("Scan your fingerprint"));
        assert!(mentions_fingerprint("pam_fprintd: swipe"));
        assert!(!mentions_fingerprint("Password:"));
    }

    /// A missed finger is replaced by our own words; anything else the reader
    /// says is guidance, and keeps the module's.
    #[test]
    fn a_missed_finger_is_told_apart_from_the_reader_s_advice() {
        assert!(is_no_match("Failed to match fingerprint"));
        assert!(is_no_match("Fingerprint verification failed"));
        assert!(!is_no_match("Swipe was too short, try again"));
        assert!(!is_no_match("Authentication failure"));
    }

    /// The module names the finger and the hardware; the panel wants the
    /// finger and its own words for the rest.
    #[test]
    fn a_request_for_a_finger_is_read_out_of_the_module_s_wording() {
        assert_eq!(
            finger_request("Place your right index finger on Elan Fingerprint Sensor"),
            Some(FingerRequest {
                swipe: false,
                finger: Some(FingerName::RightIndex),
            })
        );
        assert_eq!(
            finger_request("Swipe your left thumb across the fingerprint reader"),
            Some(FingerRequest {
                swipe: true,
                finger: Some(FingerName::LeftThumb),
            })
        );
        assert_eq!(
            finger_request("Scan your finger on the fingerprint reader"),
            Some(FingerRequest {
                swipe: false,
                finger: None,
            })
        );
        assert!(finger_request("Failed to match fingerprint").is_none());
        assert!(finger_request("Enter your password").is_none());
    }

    /// Both screens must resolve every line, or a reader would put a raw key
    /// on the card.
    #[test]
    fn both_clients_can_say_every_request() {
        for client in ["lock", "greeter"] {
            for swipe in [true, false] {
                let named = request_line(
                    FingerRequest {
                        swipe,
                        finger: Some(FingerName::RightIndex),
                    },
                    client,
                );
                let bare = request_line(
                    FingerRequest {
                        swipe,
                        finger: None,
                    },
                    client,
                );
                for line in [named, bare, no_match_line(client)] {
                    assert!(!line.is_empty());
                    assert!(!line.contains(client), "unresolved key on the card: {line}");
                }
            }
        }
    }
}
