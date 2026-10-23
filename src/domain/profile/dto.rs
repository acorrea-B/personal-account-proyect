//! The request DTOs of the profile (T018): the rules verbatim of
//! data-model.md — `full_name` 1..200 after the trim, headline/location
//! ≤120, bio ≤2000, links ≤10 (the type ≤40, the url http(s) ≤2048),
//! skills ≤30 (the strings ≤40, no duplicates). The PUT travels complete:
//! the fields omitted arrive as `None` and are EMPTIED (the contract).

use serde::{Deserialize, Serialize};
use validator::Validate;

/// One link of the profile (the contract `http-profile.md`): the JSON
/// field is `type`.
#[derive(Debug, Serialize, Deserialize, Validate, Clone)]
pub struct ProfileLink {
    #[validate(length(max = 40))]
    #[serde(rename = "type")]
    pub link_type: String,
    #[validate(url)]
    pub url: String,
}

/// The full put of the profile (FR-014) — the FULL_NAME required; the rest
/// optional and emptied when absent. The full_name trims in the handler
/// (1..200 AFTER the trim).
#[derive(Debug, Serialize, Deserialize, Validate)]
pub struct UpdateProfileRequest {
    #[validate(length(min = 1, max = 200))]
    pub full_name: String,
    #[validate(length(max = 120))]
    pub headline: Option<String>,
    #[validate(length(max = 2000))]
    pub bio: Option<String>,
    #[validate(length(max = 120))]
    pub location: Option<String>,
    #[validate(nested, length(max = 10))]
    pub links: Option<Vec<ProfileLink>>,
    /// The limit of the list + the strings ≤40 + no duplicates: the custom
    /// policy (the validator checks the lengths; the duplicates live here).
    #[validate(length(max = 30), custom(function = "skills_policy"))]
    pub skills: Option<Vec<String>>,
}

/// R4's cousin of the skills (data-model): ≤40 chars each and no
/// duplicates in the list.
pub fn skills_policy(skills: &[String]) -> Result<(), validator::ValidationError> {
    if skills.iter().any(|skill| skill.chars().count() > 40) {
        return Err(validator::ValidationError::new("skills_too_long"));
    }
    let mut seen = std::collections::HashSet::new();
    if skills
        .iter()
        .any(|skill| !seen.insert(skill.to_lowercase()))
    {
        return Err(validator::ValidationError::new("skills_duplicated"));
    }
    Ok(())
}

/// The links' rule of the contract: each url http(s) — the validator's
/// `url` check accepts also the others' schemes; this policy tightens it.
pub fn links_policy_of(links: &Option<Vec<ProfileLink>>) -> bool {
    links.as_ref().is_none_or(|links| {
        links
            .iter()
            .all(|link| link.url.starts_with("http://") || link.url.starts_with("https://"))
    })
}
