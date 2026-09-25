//! Naming helpers behind [`Resource::slug`](crate::resource::Resource::slug)
//! and [`Resource::navigation_label`](crate::resource::Resource::navigation_label).

use crate::schema::capitalize;

/// The last segment of a type's full path, e.g.
/// `tablo_core::resource::tests::UserResource` → `UserResource`.
pub(crate) fn type_short_name<T: ?Sized>() -> &'static str {
    let name = std::any::type_name::<T>();
    name.rsplit("::").next().unwrap_or(name)
}

/// Pluralize a capitalized English word with a compact ruleset (Filament
/// pluralizes via Laravel's `Str::plural`; this is the admin-grade subset):
/// a small irregular table (`person` → `people`, …), consonant-`y` → `ies`
/// (`Category` → `Categories`), sibilant endings → `es` (`Box` → `Boxes`),
/// `f`/`fe` → `ves` (`Knife` → `Knives`) with a few `+s` exceptions, and the
/// default `+s`.
pub(crate) fn pluralize(word: &str) -> String {
    if word.is_empty() {
        return word.to_string();
    }
    let lower = word.to_lowercase();
    const IRREGULAR: &[(&str, &str)] = &[
        ("person", "people"),
        ("man", "men"),
        ("woman", "women"),
        ("child", "children"),
        ("mouse", "mice"),
        ("goose", "geese"),
        ("foot", "feet"),
        ("tooth", "teeth"),
        ("datum", "data"),
        ("criterion", "criteria"),
        ("index", "indices"),
        ("matrix", "matrices"),
        ("vertex", "vertices"),
        ("axis", "axes"),
        ("crisis", "crises"),
        ("analysis", "analyses"),
    ];
    if let Some((_, plural)) = IRREGULAR.iter().find(|(singular, _)| *singular == lower) {
        return match word.chars().next() {
            Some(first) if first.is_uppercase() => capitalize(plural),
            _ => (*plural).to_string(),
        };
    }
    // `f`/`fe` → `ves`, except the words that simply take `s`.
    const F_EXCEPTIONS: &[&str] = &["roof", "chief", "belief", "chef", "cliff", "cuff"];
    const UNCOUNTABLE: &[&str] = &[
        "fish",
        "sheep",
        "deer",
        "moose",
        "series",
        "species",
        "news",
        "equipment",
        "information",
        "rice",
    ];
    if UNCOUNTABLE.contains(&lower.as_str()) {
        return word.to_string();
    }
    if F_EXCEPTIONS.contains(&lower.as_str()) {
        format!("{word}s")
    } else if lower.ends_with('f') {
        format!("{}ves", &word[..word.len() - 1])
    } else if lower.ends_with("fe") {
        format!("{}ves", &word[..word.len() - 2])
    } else if lower.ends_with('y')
        && word.len() > 1
        && !"aeiou".contains(word.chars().nth(word.len() - 2).unwrap_or(' '))
    {
        format!("{}ies", &word[..word.len() - 1])
    } else if ["s", "ss", "sh", "ch", "x", "z"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        format!("{word}es")
    } else {
        format!("{word}s")
    }
}

/// Convert a CamelCase identifier to kebab-case: `BlogPost` → `blog-post`,
/// `APIKey` → `api-key`.
///
/// Delegates to `heck::ToKebabCase`: digits split words
/// (`User2FA` → `user2-fa`) and so do underscores (`Audit_Log` → `audit-log`).
/// Name resources without underscores or override
/// [`Resource::slug`](crate::Resource::slug).
pub(crate) fn kebab_case(name: &str) -> String {
    use heck::ToKebabCase;
    name.to_kebab_case()
}

#[cfg(test)]
mod tests {
    #[test]
    fn pluralize_and_kebab_follow_english_rules() {
        use super::{kebab_case, pluralize};
        // rules
        assert_eq!(pluralize("User"), "Users");
        assert_eq!(pluralize("Category"), "Categories");
        assert_eq!(pluralize("Dummy"), "Dummies");
        assert_eq!(pluralize("Day"), "Days");
        assert_eq!(pluralize("Box"), "Boxes");
        assert_eq!(pluralize("Bus"), "Buses");
        assert_eq!(pluralize("Church"), "Churches");
        assert_eq!(pluralize("Knife"), "Knives");
        assert_eq!(pluralize("Roof"), "Roofs");
        // irregulars (case preserved)
        assert_eq!(pluralize("Person"), "People");
        assert_eq!(pluralize("Child"), "Children");
        assert_eq!(pluralize("Index"), "Indices");
        // kebab
        assert_eq!(kebab_case("Users"), "users");
        assert_eq!(kebab_case("BlogPost"), "blog-post");
        assert_eq!(kebab_case("APIKey"), "api-key");
        // The heck delegate: digit boundaries split (`User2FA` →
        // `user2-fa`) and underscores split words — pinned so a heck upgrade
        // cannot silently change slugs.
        assert_eq!(kebab_case("User2FA"), "user2-fa");
        assert_eq!(kebab_case("Blog_Post"), "blog-post");
    }

    #[test]
    fn naming_invariants_hold() {
        // GH #136 §5 property candidates: kebab is lowercase + hyphen-only,
        // pluralize never empties.
        use super::{kebab_case, pluralize};
        for word in [
            "User", "BlogPost", "APIKey", "Category", "Box", "Person", "",
        ] {
            let kebab = kebab_case(word);
            assert!(
                kebab
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                    || kebab.is_empty(),
                "kebab must be lower-hyphen, got {kebab:?} from {word:?}"
            );
            let plural = pluralize(word);
            assert!(
                word.is_empty() || !plural.is_empty(),
                "plural must not empty {word:?}"
            );
        }
        // kebab round-trips through slug vocabulary (no underscores).
        assert!(!kebab_case("Audit_Log").contains('_'));
    }
}
