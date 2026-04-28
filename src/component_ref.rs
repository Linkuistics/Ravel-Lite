//! Globally-unique component reference: `(repo_slug, component_id)`.
//!
//! `repo_slug` is a key in `<context>/repos.yaml`. `component_id` is the
//! `id` field of a `ComponentEntry` inside that repo's
//! `.atlas/components.yaml`. Together they identify a single component
//! across the entire ravel context — used by targets, edges in
//! `commits.yaml`, memory attribution, and `this-cycle-focus.yaml`.
//!
//! The on-the-wire and human-facing form is `<repo_slug>:<component_id>`
//! (e.g. `atlas:atlas-core`, `ravel-lite:phase-loop`). Neither half may
//! be empty; neither half may contain `:` (one colon delimits cleanly,
//! and component ids are kebab-case in the architecture-next examples).

use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComponentRef {
    pub repo_slug: String,
    pub component_id: String,
}

impl ComponentRef {
    pub fn new(repo_slug: impl Into<String>, component_id: impl Into<String>) -> Self {
        ComponentRef {
            repo_slug: repo_slug.into(),
            component_id: component_id.into(),
        }
    }
}

impl fmt::Display for ComponentRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.repo_slug, self.component_id)
    }
}

impl FromStr for ComponentRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (repo, id) = s.split_once(':').ok_or_else(|| {
            anyhow!(
                "ComponentRef '{s}' missing ':' separator; expected '<repo_slug>:<component_id>'"
            )
        })?;
        if repo.is_empty() {
            return Err(anyhow!(
                "ComponentRef '{s}' has empty repo_slug; expected '<repo_slug>:<component_id>'"
            ));
        }
        if id.is_empty() {
            return Err(anyhow!(
                "ComponentRef '{s}' has empty component_id; expected '<repo_slug>:<component_id>'"
            ));
        }
        if id.contains(':') {
            return Err(anyhow!(
                "ComponentRef '{s}' contains more than one ':' separator"
            ));
        }
        Ok(ComponentRef {
            repo_slug: repo.to_string(),
            component_id: id.to_string(),
        })
    }
}

impl Serialize for ComponentRef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ComponentRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_form() {
        let r: ComponentRef = "atlas:atlas-core".parse().unwrap();
        assert_eq!(r.repo_slug, "atlas");
        assert_eq!(r.component_id, "atlas-core");
    }

    #[test]
    fn round_trips_through_display() {
        let r = ComponentRef::new("ravel-lite", "phase-loop");
        let parsed: ComponentRef = r.to_string().parse().unwrap();
        assert_eq!(r, parsed);
    }

    #[test]
    fn rejects_missing_separator() {
        let err = "atlas-core".parse::<ComponentRef>().unwrap_err();
        assert!(format!("{err:#}").contains("missing ':'"));
    }

    #[test]
    fn rejects_empty_repo_slug() {
        let err = ":atlas-core".parse::<ComponentRef>().unwrap_err();
        assert!(format!("{err:#}").contains("empty repo_slug"));
    }

    #[test]
    fn rejects_empty_component_id() {
        let err = "atlas:".parse::<ComponentRef>().unwrap_err();
        assert!(format!("{err:#}").contains("empty component_id"));
    }

    #[test]
    fn rejects_extra_colons() {
        let err = "atlas:foo:bar".parse::<ComponentRef>().unwrap_err();
        assert!(format!("{err:#}").contains("more than one ':'"));
    }

    #[test]
    fn yaml_round_trip_uses_string_form() {
        let r = ComponentRef::new("atlas", "atlas-core");
        let yaml = serde_yaml::to_string(&r).unwrap();
        assert!(
            yaml.contains("atlas:atlas-core"),
            "expected stringified form, got {yaml}"
        );
        let back: ComponentRef = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(r, back);
    }
}
